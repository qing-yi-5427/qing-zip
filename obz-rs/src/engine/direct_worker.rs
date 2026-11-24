//! The worker decodes through one checked final-name handle at a time.
//! It receives neither output paths nor permission to open files itself.
use super::*;

const ROLLBACK_FILES: usize = 4096;

#[repr(C)]
#[derive(Default)]
struct WorkerIoCounters {
    read_operations: u64,
    write_operations: u64,
    other_operations: u64,
    read_bytes: u64,
    write_bytes: u64,
    other_bytes: u64,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetProcessIoCounters(process: std::os::windows::io::RawHandle, counters: *mut WorkerIoCounters) -> i32;
}

pub(in crate::engine) struct Pending {
    session: Session,
    _runtime: Vec<crate::engine_stream::PinnedSource>,
    _image: crate::engine_stream::PinnedSource,
    pub(in crate::engine) volumes: rar_listing::NamedPins,
    nonce: [u8; 16],
    mode: u32,
}

pub(in crate::engine) fn prepare(
    path: &Path,
    source: &mut File,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<Option<(Vec<Entry>, Pending)>, String> {
    let probe = PhaseTimer::new("direct-extract-probe");
    if opts.codepage.is_some()
        || crate::machine_policy::scope().restricted()
        || !using_process_token()
    {
        return Ok(None);
    }
    let (mode, volumes) =
        if let Some((format, volumes)) = rar_listing::direct_volumes(path, source, ctl)? {
            (if format == 5 { 3u32 } else { 4u32 }, volumes)
        } else if !zip_volume_view::positioned_zip_hint(path) {
            if zip_input_probe::signature(source)? == Some("7z") {
                (1, Vec::new())
            } else {
                let mut magic = [0; 4];
                if source.read_exact(&mut magic).is_ok() && magic == *b"PK\x03\x04" {
                    let Some(layout) = zip_input_probe::list_layout(source, ctl)? else {
                        return Ok(None);
                    };
                    if layout.first_local != 0 || layout.count == 0 || layout.count > 512_000 {
                        return Ok(None);
                    }
                    (2, Vec::new())
                } else {
                    source.rewind().map_err(|e| e.to_string())?;
                    return Ok(None);
                }
            }
        } else {
            return Ok(None);
        };
    let Some(runtime) = runtime_dir()? else {
        return Ok(None);
    };
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("obz.exe");
    #[cfg(test)]
    let exe = if exe.is_file() {
        exe
    } else {
        exe.parent().unwrap().parent().unwrap().join("obz.exe")
    };
    let helper = exe.with_file_name("obz-worker.exe");
    let exe = if helper.is_file() { helper } else { exe };
    if !exe.is_file() {
        return Ok(None);
    }
    base("x", opts)?;
    let image = crate::engine_stream::pin(&exe)?;
    let mut nonce = [0; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 16, 2) } < 0 {
        return Err("direct extraction worker nonce failed".into());
    }
    drop(probe);
    let _timer = PhaseTimer::new("direct-extract-manifest");
    let mut pending = Pending {
        session: Session::start_reader(&exe, 3, true)?,
        _runtime: Vec::new(),
        _image: image,
        volumes,
        nonce,
        mode,
    };
    pending._runtime = pin_runtime(&runtime, ctl)?;
    let leaf = match mode {
        1 => OsStr::new("input.7z"),
        2 => OsStr::new("input.zip"),
        _ => path.file_name().ok_or("RAR source has no filename")?,
    };
    let mut request = nonce.to_vec();
    wide(&mut request, runtime.as_os_str())?;
    let child = &pending.session.child;
    request.extend_from_slice(
        &duplicate(child, unsafe { GetCurrentProcess() }, 0x100000)?.to_le_bytes(),
    );
    request.extend_from_slice(&((pending.volumes.len() + 1) as u32).to_le_bytes());
    for (name, file) in std::iter::once((leaf, &*source)).chain(
        pending
            .volumes
            .iter()
            .map(|(name, file)| (OsStr::new(name), &**file)),
    ) {
        request.extend_from_slice(&duplicate(child, file.as_raw_handle(), 0x100081)?.to_le_bytes());
        request.extend_from_slice(
            &file
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
                .to_le_bytes(),
        );
        wide(&mut request, name)?;
    }
    let password = opts.password.as_deref().unwrap_or_default();
    let units = password.encode_utf16().count();
    if units > MAX_PASSWORD_UNITS {
        return Err("password input length exceeds limit".into());
    }
    request.extend_from_slice(&(units as u32).to_le_bytes());
    request.extend_from_slice(&(mode + 9).to_le_bytes());
    let threads = opts.threads;
    request.extend_from_slice(&threads.unwrap_or(0).to_le_bytes());
    ctl.set("listing", None, "Reading archive directory");
    pending.session.send(11, request, ctl)?;
    if units != 0 {
        pending.session.send(6, SecretFrame::new(password), ctl)?;
    }
    let mut thread_ack = threads.is_none();
    let mut password_requested = false;
    while let Some((kind, body)) = pending.session.next(ctl)? {
        let mut data = Fields(&body);
        if data.take(16)? != nonce {
            return Err("direct extraction worker identity mismatch".into());
        }
        match kind {
            7 => {
                data.end()?;
                password_requested = true;
            }
            8 if !thread_ack => {
                if Some(data.u32()?) != threads {
                    return Err("worker thread limit mismatch".into());
                }
                data.end()?;
                thread_ack = true;
            }
            26 if thread_ack => {
                let _timer = PhaseTimer::new("direct-manifest-parse");
                let entries = full_worker::parse_manifest(&body, &nonce, false, true)?;
                if password_requested && units != 0 {
                    if let Some(evidence) = &opts.extraction_password_use {
                        evidence.confirm();
                    }
                }
                return Ok(Some((entries, pending)));
            }
            // No output paths or handles have been granted. Reuse the legacy
            // route for unsupported layouts and its format/password diagnostics.
            5 => {
                if PhaseTimer::enabled() {
                    eprintln!(
                        "OA_DIRECT_EXTRACT manifest_declined mode={mode} {}",
                        terminal_description(data)?
                    );
                }
                ctl.check()?;
                return Ok(None);
            }
            _ => return Err("unexpected direct extraction manifest response".into()),
        }
    }
    Err("direct extraction worker exited before manifest".into())
}

struct Terminal {
    result: u32,
    outcome: u32,
    count: usize,
    completed: usize,
    errors: [u32; 2],
    open: u32,
    decoded: u64,
    written: u64,
    code: i32,
    reason: String,
}
fn terminal(mut data: Fields<'_>) -> Result<Terminal, String> {
    let result = data.u32()?;
    let outcome = data.u32()?;
    let count = data.u32()? as usize;
    let completed = data.u32()? as usize;
    let errors = [data.u32()?, data.u32()?];
    let open = data.u32()?;
    let decoded = data.u64()?;
    let written = data.u64()?;
    let code = data.u32()? as i32;
    let length = data.u32()? as usize;
    if length > 192 {
        return Err("invalid direct extraction receipt".into());
    }
    let reason = std::str::from_utf8(data.take(length)?)
        .map_err(|_| "invalid direct extraction receipt")?
        .to_owned();
    data.end()?;
    Ok(Terminal {
        result,
        outcome,
        count,
        completed,
        errors,
        open,
        decoded,
        written,
        code,
        reason,
    })
}
fn terminal_description(data: Fields<'_>) -> Result<String, String> {
    let receipt = terminal(data)?;
    Ok(format!("({:#010x}): {}", receipt.result, receipt.reason))
}

/// A regular existing destination stays shared. Every newly opened child is
/// created relative to a pinned, non-reparse parent with NTFS FILE_CREATE.
fn checked_parent(parent: &Path, guards: &mut CheckedDirectories) -> Result<File, String> {
    guards.check(parent)?;
    let file = File::options()
        .access_mode(0x200a0)
        .share_mode(3)
        .custom_flags(0x02200000)
        .open(parent)
        .map_err(|e| e.to_string())?;
    validate_transaction_root(&file, parent)?;
    Ok(file)
}

pub(in crate::engine) enum Destination {
    Existing,
    Claim(PathBuf),
}

pub(in crate::engine) fn eligible_destination(
    dest: &Path,
    entries: &[&Entry],
    guards: &mut CheckedDirectories,
    ctl: &JobControl,
) -> Result<Option<Destination>, String> {
    // Keep path forms that do not admit the ordinary, pinned-disk check on the
    // compatibility path. No direct output has been granted at this point.
    if attribute_query_path(dest).is_none() {
        return Ok(None);
    }
    let mut claim = None;
    let volume_dir = loop {
        let current = claim
            .map_or(Some(dest), Path::parent)
            .ok_or("missing output parent")?;
        match fs::symlink_metadata(current) {
            Ok(meta) if meta.is_dir() => break current,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => claim = Some(current),
            Err(error) => return Err(error.to_string()),
        }
    };
    let Ok(volume) = checked_parent(volume_dir, guards) else {
        return Ok(None);
    };
    if !crate::private_stage::direct_output_supported(&volume) {
        return Ok(None);
    }
    if let Some(claim) = claim {
        return Ok(Some(Destination::Claim(claim.to_owned())));
    }
    let mut parents = HashSet::new();
    for entry in entries {
        ctl.check()?;
        let target = dest.join(validate_name(&entry.name)?);
        let parent = target.parent().ok_or("missing direct output parent")?;
        if parents.insert(parent.to_owned()) {
            guards.check(parent)?;
        }
        match fs::symlink_metadata(&target) {
            Ok(meta) if entry.is_dir && meta.is_dir() => guards.check(&target)?,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(Some(Destination::Existing))
}

impl Pending {
    pub(in crate::engine) fn extract(
        mut self,
        entries: &[Entry],
        dest: &Path,
        destination: Destination,
        opts: &EngineOptions,
        mut guards: CheckedDirectories,
        ctl: &JobControl,
    ) -> Result<usize, String> {
        let _timer = PhaseTimer::new("direct-extract");
        let setup = PhaseTimer::new("direct-prepare-decode");
        let policy = crate::machine_policy::scope();
        let selection = Selection::new(&opts.selected);
        let chosen = entries
            .iter()
            .map(|e| selection.contains(&e.name))
            .collect::<Vec<_>>();
        let file_count = entries
            .iter()
            .zip(&chosen)
            .filter(|(e, wanted)| !e.is_dir && **wanted)
            .count();
        let (total, selected) =
            entries
                .iter()
                .zip(&chosen)
                .fold((0u64, 0u64), |(total, selected), (entry, wanted)| {
                    (
                        total + entry.size,
                        selected + if *wanted { entry.size } else { 0 },
                    )
                });
        let journal = PublicationJournal::default();
        let overlap = self.mode == 2
            && opts.password.is_none()
            && opts.selected.is_empty()
            && matches!(&destination, Destination::Claim(_))
            && entries.iter().all(|entry| !entry.encrypted)
            && file_count >= 128;
        let mut active = None;
        let mut completed = None;
        let mut released = 0usize;
        let result = (|| -> Result<usize, String> {
            ctl.check()?;
            policy.validate_output()?;
            guards.selected_output_root = Some(dest.to_owned());
            if let Destination::Claim(root) = &destination {
                let path = root.parent().ok_or("missing output parent")?;
                guards.allow_publication(path)?;
                let parent = checked_parent(path, &mut guards)?;
                let leaf = root.file_name().ok_or("missing destination leaf")?;
                let file = crate::private_stage::create_journal_directory(&parent, leaf)?;
                let proof = created_directories::Identity::owned(&file)?;
                guards.exclusive_claim = true;
                guards.record_created_output_dir(root, &file, proof)?;
                guards.selected_output_root = Some(root.clone());
                guards.create_dirs(dest)?;
            }
            let mut ack = self.nonce.to_vec();
            ack.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for wanted in &chosen {
                ack.push(u8::from(*wanted));
                ack.extend_from_slice(&0u64.to_le_bytes());
            }
            self.session.send(3, ack, ctl)?;
            let mut writer = worker_ack::AckWriter::new(&self.session)?;
            let mut parents = HashMap::<PathBuf, File>::new();
            let mut verified_files = vec![false; entries.len()];
            let mut count = 0usize;
            let mut written = 0u64;
            let mut decoded = 0u64;
            let mut receipt = None;
            ctl.report_decoding("", count, file_count, decoded, total);
            drop(setup);
            let decode = PhaseTimer::new("direct-decode-and-publish");
            while let Some((kind, body)) = self.session.next(ctl)? {
                let mut data = Fields(&body);
                if data.take(16)? != self.nonce || receipt.is_some() {
                    return Err("invalid direct extraction response".into());
                }
                match kind {
                    27 if active.is_none() => {
                        let _timer = PhaseTimer::new("direct-open-output");
                        let index = data.u32()? as usize;
                        data.end()?;
                        let entry = entries.get(index).ok_or("invalid direct output index")?;
                        if entry.is_dir
                            || !chosen[index]
                            || verified_files[index]
                            || completed
                                .as_ref()
                                .is_some_and(|(prior, _, _)| *prior == index)
                        {
                            return Err("unexpected direct output request".into());
                        }
                        let target = dest.join(validate_name(&entry.name)?);
                        let path = target.parent().ok_or("missing direct output parent")?;
                        if !parents.contains_key(path) {
                            guards.create_dirs(path)?;
                            guards.allow_publication(path)?;
                            let file = checked_parent(path, &mut guards)?;
                            if parents.len() == 256 {
                                parents.clear();
                            }
                            parents.insert(path.to_owned(), file);
                        }
                        // The directory handle cannot be renamed while retained.
                        // Relative creation also fails if a foreign leaf appeared.
                        let leaf = target.file_name().ok_or("missing direct output name")?;
                        let output =
                            crate::private_stage::create_direct_output(path, &parents[path], leaf)?;
                        let handle = duplicate(
                            &self.session.child,
                            output.as_file().as_raw_handle(),
                            0x100082,
                        )?;
                        active = Some((index, target, output));
                        let mut reply = self.nonce.to_vec();
                        reply.extend_from_slice(&(index as u32).to_le_bytes());
                        reply.extend_from_slice(&handle.to_le_bytes());
                        writer.send(28, reply, ctl)?;
                    }
                    29 => {
                        let index = data.u32()? as usize;
                        data.end()?;
                        let (active_index, target, output) =
                            active.take().ok_or("unexpected direct output completion")?;
                        if index != active_index || completed.is_some() {
                            return Err("direct output completion index mismatch".into());
                        }
                        completed = Some((index, target, output));
                        if overlap {
                            continue;
                        }
                    }
                    4 => {
                        let current = data.u64()?;
                        if data.u64()? != total || current > total {
                            return Err("invalid direct extraction progress".into());
                        }
                        data.end()?;
                        if current > decoded {
                            decoded = current;
                            ctl.report_decoding("", count, file_count, decoded, total);
                        }
                    }
                    5 => receipt = Some(terminal(data)?),
                    _ => return Err("unexpected direct extraction frame".into()),
                }
                // The worker can decode into the next capability after its
                // ACK while the verified previous file is finalized. At most
                // two unpublished outputs are held by the parent.
                if let Some((index, target, mut output)) = completed.take() {
                    let _timer = PhaseTimer::new("direct-commit-output");
                    let entry = &entries[index];
                    if output
                        .as_file()
                        .metadata()
                        .map_err(|e| e.to_string())?
                        .len()
                        != entry.size
                    {
                        return Err(format!("decoded size differs for {}", entry.name));
                    }
                    crate::motw::apply_to_file(output.as_file(), opts.zone_identifier.as_ref())?;
                    restore_open_file_metadata(output.as_file(), entry)?;
                    ctl.check()?;
                    if count % 256 == 0 {
                        policy.validate_output()?;
                    }
                    drop(output.commit_owned(&target, &journal)?);
                    confirm_encrypted_output(entry, opts.extraction_password_use.as_deref());
                    released += usize::from(journal.retain_latest(ROLLBACK_FILES));
                    verified_files[index] = true;
                    count += 1;
                    written += entry.size;
                    decoded = decoded.max(written);
                    if count == 1 || count % 64 == 0 || count == file_count {
                        ctl.report_decoding(&entry.name, count, file_count, decoded, total);
                    }
                }
            }
            drop(decode);
            let exit = {
                let _timer = PhaseTimer::new("direct-worker-exit");
                self.session.wait_exit(ctl)?
            };
            let _finalize = PhaseTimer::new("direct-finalize");
            let receipt =
                receipt.ok_or("direct extraction worker exited without verification receipt")?;
            if exit != 0
                || receipt.result != 0
                || receipt.outcome != 2
                || receipt.count != entries.len()
                || receipt.completed != entries.len()
                || receipt.errors != [0, 0]
                || receipt.open != 0
                || receipt.decoded != total
                || receipt.written != selected
                || receipt.code != -1
                || count != file_count
                || active.is_some()
                || completed.is_some()
            {
                if receipt.code == 9
                    || (opts.password.is_some()
                        && entries.iter().any(|entry| entry.encrypted)
                        && (matches!(receipt.code, 2 | 3)
                            || (receipt.code == -1
                                && receipt.reason
                                    == "sticky source callback failure or cancellation")))
                {
                    return Err("incorrect password or damaged encrypted archive".into());
                }
                if receipt.code == 3 {
                    return Err("archive CRC verification failed".into());
                }
                return Err(format!(
                    "direct extraction worker failed ({:#010x}): {}",
                    receipt.result, receipt.reason
                ));
            }
            ctl.check()?;
            policy.validate_output()?;
            // Empty directories and folder timestamps are finalized only after
            // the decoder confirms success for the complete archive.
            let mut directories = entries
                .iter()
                .zip(&chosen)
                .filter(|(e, yes)| e.is_dir && **yes)
                .map(|(entry, _)| Ok((dest.join(validate_name(&entry.name)?), entry)))
                .collect::<Result<Vec<_>, String>>()?;
            for (path, _) in &directories {
                ctl.check()?;
                guards.create_dirs(path)?;
            }
            directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
            let owned = guards
                .created_output_dirs
                .iter()
                .enumerate()
                .map(|(i, (path, _))| (path.clone(), i))
                .collect::<HashMap<_, _>>();
            for (path, entry) in directories {
                ctl.check()?;
                if guards.exclusive_claim {
                    guards.restore_claimed_directory_metadata(&path, entry)?;
                    continue;
                }
                if let Some(&index) = owned.get(&path) {
                    match &mut guards.created_output_dirs[index].1 {
                        proof @ created_directories::Identity::Owned(_, _) => {
                            proof.restore_owned_metadata(entry)?
                        }
                        _ => {
                            guards.check(&path)?;
                            restore_metadata(&path, entry)?;
                        }
                    }
                }
            }
            policy.validate_output()?;
            ctl.check()?;
            guards.created_output_dirs.clear();
            Ok(count)
        })();
        let mode = self.mode;
        if mode == 2 && PhaseTimer::enabled() {
            let mut io = WorkerIoCounters::default();
            if unsafe { GetProcessIoCounters(self.session.child.as_raw_handle(), &mut io) } != 0 {
                eprintln!("OA_DIRECT_ZIP_READ_OPERATIONS {}", io.read_operations);
            }
        }
        drop(self); // Reap the decoder before abandoning the last writable handle or rolling back.
        drop(active);
        drop(completed);
        match result {
            Ok(count) => {
                if PhaseTimer::enabled() {
                    eprintln!("OA_DIRECT_EXTRACT files={count} mode={mode}");
                }
                ctl.set("complete", Some(100), "Extraction complete");
                Ok(count)
            }
            Err(mut reason) => {
                let errors = journal.rollback();
                drop(guards);
                if ctl.check().is_err() && (released != 0 || !errors.is_empty()) {
                    reason = "任务已取消".into();
                }
                if released != 0 {
                    reason.push_str(&format!(
                        "。已保留 {released} 个解压完成并通过完整性检查的文件，位置：{}",
                        dest.display()
                    ));
                }
                if !errors.is_empty() {
                    reason.push_str(&format!(
                        "。另有 {} 个文件无法安全清理，请检查：{}",
                        errors.len(),
                        dest.display()
                    ));
                    for error in errors.iter().take(3) {
                        reason.push_str(&format!("。{error}"));
                    }
                }
                Err(reason)
            }
        }
    }
}
