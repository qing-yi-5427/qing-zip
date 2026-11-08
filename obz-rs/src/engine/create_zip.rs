use super::*;

extern "C" {
    pub(super) fn qing_spawn_create_worker(
        exe: *const u16,
        process: *mut Handle,
        input: *mut Handle,
        output: *mut Handle,
    ) -> u32;
    fn qz_create_zip(
        runtime: *const u16,
        owner: Handle,
        directory: Handle,
        root_name: *const u16,
        output: Handle,
        level: u32,
        threads: u32,
        explicit: u32,
        error: *mut u8,
        capacity: usize,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationByHandleW(
        file: Handle,
        name: *mut u16,
        n: u32,
        serial: *mut u32,
        component: *mut u32,
        flags: *mut u32,
        fs: *mut u16,
        size: u32,
    ) -> i32;
}

fn local_disk(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(p)) = path.components().next() else {
        return false;
    };
    let Prefix::Disk(letter) = p.kind() else {
        return false;
    };
    let root = [u16::from(letter), 58, 92, 0];
    attribute_query_path(path).is_some() && unsafe { GetDriveTypeW(root.as_ptr()) } == 3
}
fn ordinary_ntfs(file: &File) -> bool {
    let mut fs = [0u16; 16];
    unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            16,
        ) != 0
            && fs[..5] == [78, 84, 70, 83, 0]
    }
}

pub(in crate::engine) fn eligible(format: &str, names: &[String], opts: &EngineOptions) -> bool {
    format == "zip"
        && names.len() == 1
        && opts
            .method
            .as_deref()
            .is_none_or(|m| m.trim().eq_ignore_ascii_case("deflate"))
        && opts.level.is_none_or(|v| (1..=9).contains(&v))
        && opts.threads.is_none_or(|v| (1..=64).contains(&v))
        && opts.password.is_none()
        && opts.codepage.is_none()
        && opts.encryption.is_none()
        && opts.dictionary_bytes.is_none()
        && opts.solid.is_none()
        && opts.header_encryption.is_none()
        && opts.volume_bytes.is_none()
        && opts.selected.is_empty()
        && opts.overwrite == "error"
        && opts.zone_identifier.is_none()
}

// Routing samples a limited number of directory listings. The actual native
// manifest or the ordinary source preflight always rechecks the tree independently.
const MIN_WORKER_FILES: usize = 2048;
const PROBE_DIRECTORIES: usize = 32;
const PROBE_ENTRIES: usize = MIN_WORKER_FILES + 256;

fn large_directory_hint(root: &Path, ctl: &JobControl) -> Result<bool, String> {
    use std::collections::VecDeque;
    let mut pending = VecDeque::from([root.to_owned()]);
    let mut probe_pins = SourceValidationGuards::new();
    let (mut visited, mut entries, mut files) = (0, 0, 0);
    while let Some(dir) = pending.pop_front() {
        // Root and its ancestors are pinned by the caller. Each additional
        // directory is opened without following reparse points; listed types
        // select work only and cannot authorize a path or file.
        if dir != root && !matches!(probe_pins.pin_directory(&dir), Ok(true)) {
            return Ok(false);
        }
        visited += 1;
        let Ok(iter) = fs::read_dir(dir) else {
            return Ok(false);
        };
        for entry in iter {
            ctl.check()?;
            if entries == PROBE_ENTRIES {
                return Ok(false);
            }
            entries += 1;
            let Ok(entry) = entry else { return Ok(false) };
            let Ok(kind) = entry.file_type() else {
                return Ok(false);
            };
            if kind.is_symlink() {
                return Ok(false);
            }
            if kind.is_file() {
                files += 1;
                if files == MIN_WORKER_FILES {
                    return Ok(true);
                }
            } else if kind.is_dir() {
                if visited + pending.len() < PROBE_DIRECTORIES {
                    pending.push_back(entry.path());
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(false)
}

/// false is possible only before UpdateItems starts; the caller must then redo
/// the complete ordinary source validation. Security, cancellation, protocol and
/// publication errors never turn into a second encode.
pub(in crate::engine) fn try_create(
    out: &str,
    names: &[String],
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<bool, String> {
    let policy = crate::machine_policy::scope();
    ctl.check()?;
    if policy.restricted() {
        return Ok(false);
    }
    let src = absolute(&names[0])?;
    let target = absolute(out)?;
    if !local_disk(&src)
        || !local_disk(&target)
        || !target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        return Ok(false);
    }
    let parent = target.parent().ok_or("output has no parent")?;
    if !parent.is_dir() || !src.is_dir() {
        return Ok(false);
    }
    let mut source_guards = SourceValidationGuards::new();
    for a in src.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if !source_guards.pin_directory(a)? {
            return Ok(false);
        }
    }
    let large = {
        let _timer = PhaseTimer::new("zip-create-size-probe");
        large_directory_hint(&src, ctl)?
    };
    if !large {
        return Ok(false);
    }
    let inputs = checked_input_roots(names)?;
    no_links(&target)?;
    reject_output_in_inputs(parent, &inputs)?;
    if target.exists() {
        return Err("output archive already exists; use update to modify it".into());
    }
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("obz.exe");
    if !exe.is_file() {
        return Ok(false);
    }
    let Some(runtime) = runtime_dir()? else {
        return Ok(false);
    };
    let mut guards = CheckedDirectories::default();
    guards.check(parent)?;
    let Some((destination, _)) = guards.held.get(parent) else {
        return Ok(false);
    };
    if !ordinary_ntfs(destination) || !ordinary_ntfs(&source_guards.held[&src]) {
        return Ok(false);
    }
    let mut dest_ids = HashSet::new();
    for a in parent.ancestors() {
        let Some((f, _)) = guards.held.get(a) else {
            return Ok(false);
        };
        dest_ids.insert(directory_identity(f)?);
    }
    if dest_ids.contains(&directory_identity(&source_guards.held[&src])?) {
        return Err("target inside source directory".into());
    }
    let shared = source_guards
        .held
        .iter()
        .map(|(p, f)| Ok((p.clone(), dest_ids.contains(&directory_identity(f)?))))
        .collect::<Result<Vec<_>, String>>()?;
    let _runtime = pin_runtime(&runtime, ctl)?;
    let _image = crate::engine_stream::pin(&exe)?;
    let parent_pin = match File::options()
        .access_mode(0x200a0)
        .share_mode(3)
        .custom_flags(0x02200000)
        .open(parent)
    {
        Ok(file) => file,
        Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if directory_identity(&parent_pin)? != directory_identity(destination)?
        || directory_attributes(&parent_pin)? & 0x410 != 0x10
    {
        return Err("ZIP destination directory identity changed".into());
    }
    let Some(mut output) = crate::private_stage::ArchiveOutput::try_new(parent, &parent_pin)?
    else {
        return Ok(false);
    };
    let encode = encode(
        &exe,
        &src,
        &source_guards.held[&src],
        output.file(),
        opts,
        ctl,
    );
    for (p, overlap) in shared {
        if overlap {
            source_guards.held.remove(&p);
        }
    }
    let result = encode.and_then(|encoded| {
        ctl.check()?;
        policy.unchanged()?;
        if !encoded {
            return Ok(false);
        }
        guards.check_prepared(parent)?;
        reject_output_in_inputs(parent, &inputs)?;
        output.file().sync_all().map_err(|e| e.to_string())?;
        ctl.check()?;
        policy.unchanged()?;
        output.publish(target.file_name().ok_or("missing output name")?, || {
            guards.check_prepared(parent)?;
            policy.unchanged()?;
            ctl.check()
        })?;
        Ok(true)
    });
    match (result, output.cleanup()) {
        (Err(error), Err(cleanup)) => Err(format!("{error}; staging cleanup failed: {cleanup}")),
        (Ok(true), Err(cleanup)) => Err(format!(
            "archive was committed but staging cleanup failed: {cleanup}"
        )),
        (Ok(false), Err(cleanup)) => Err(cleanup),
        (result, Ok(())) => result,
    }
}

fn encode(
    exe: &Path,
    src: &Path,
    root: &File,
    output: &File,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<bool, String> {
    let mut session = Session::start(exe, true)?;
    let mut data = Vec::new();
    for (handle, access) in [
        (unsafe { GetCurrentProcess() }, 0x100000),
        (root.as_raw_handle(), 0x100081),
        (output.as_raw_handle(), 0x100082),
    ] {
        data.extend_from_slice(&duplicate(&session.child, handle, access)?.to_le_bytes());
    }
    wide(&mut data, src.file_name().ok_or("source name missing")?)?;
    data.extend_from_slice(&opts.level.map(u32::from).unwrap_or(u32::MAX).to_le_bytes());
    data.extend_from_slice(&opts.threads.unwrap_or(0).to_le_bytes());
    data.extend_from_slice(&u32::from(opts.method.is_some()).to_le_bytes());
    session.send(42, data, ctl)?;
    let mut began = false;
    let encoded = loop {
        let (kind, body) = session
            .next(ctl)?
            .ok_or("ZIP worker exited without terminal receipt")?;
        match (kind, body.as_slice()) {
            (47, []) if !began => {
                began = true;
                if PhaseTimer::enabled() {
                    eprintln!("OA_CREATE_WORKER status=encoding");
                }
                ctl.set("compressing", Some(0), "Creating archive");
            }
            (46, [percent]) if began && *percent <= 99 => {
                ctl.set("compressing", Some(*percent), "Creating archive")
            }
            (41, []) if !began => break false,
            (43, []) if began => break true,
            (43, msg) if !msg.is_empty() => return Err(String::from_utf8_lossy(msg).into_owned()),
            _ => return Err("invalid ZIP worker receipt".into()),
        }
    };
    if session.next(ctl)?.is_some() {
        return Err("trailing ZIP worker receipt".into());
    }
    loop {
        match session.exit_code()? {
            Some(0) if encoded => break,
            Some(3) if !encoded => break,
            Some(_) => return Err("ZIP worker status differs from its receipt".into()),
            None => {
                ctl.check()?;
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    Ok(encoded)
}

fn run_worker() -> Result<bool, String> {
    let runtime = runtime_dir()?.ok_or("missing ZIP runtime")?;
    let runtime: Vec<_> = runtime.as_os_str().encode_wide().chain(Some(0)).collect();
    let (kind, bytes) = read_frame(&mut std::io::stdin().lock(), 1024)?.ok_or("no ZIP request")?;
    if kind != 42 {
        return Err("invalid ZIP request".into());
    }
    let mut frame = Fields(&bytes);
    let owner = frame.u64()? as usize as Handle;
    let root = frame.u64()? as usize as Handle;
    let output = frame.u64()? as usize as Handle;
    let name: Vec<_> = frame.wide()?.encode_utf16().chain(Some(0)).collect();
    let level = frame.u32()?;
    let threads = frame.u32()?;
    let explicit = frame.bit()?;
    frame.end()?;
    let mut error = [0u8; 256];
    let status = unsafe {
        qz_create_zip(
            runtime.as_ptr(),
            owner,
            root,
            name.as_ptr(),
            output,
            level,
            threads,
            u32::from(explicit),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if status == 1 {
        return Ok(false);
    }
    if status != 0 {
        let n = error.iter().position(|b| *b == 0).unwrap_or(error.len());
        return Err(format!(
            "ZIP worker failed {status:#x}: {}",
            String::from_utf8_lossy(&error[..n])
        ));
    }
    Ok(true)
}

pub fn worker() -> u8 {
    let r = run_worker();
    let err = r.as_ref().err().map(String::as_bytes).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    let unsupported = matches!(r, Ok(false));
    if out
        .write_all(&[if unsupported { 41 } else { 43 }])
        .and_then(|_| out.write_all(&(err.len() as u32).to_le_bytes()))
        .and_then(|_| out.write_all(err))
        .is_err()
    {
        return 2;
    }
    if unsupported {
        3
    } else {
        u8::from(r.is_err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_matching_zip_create_options_use_the_worker() {
        let inputs = ["source".to_owned()];
        let normal = EngineOptions::default();
        assert!(eligible("zip", &inputs, &normal));
        assert!(!eligible("7z", &inputs, &normal));
        assert!(!eligible("zip", &[], &normal));
        for options in [
            EngineOptions {
                password: Some("secret".into()),
                ..normal.clone()
            },
            EngineOptions {
                method: Some("store".into()),
                ..normal.clone()
            },
            EngineOptions {
                level: Some(0),
                ..normal.clone()
            },
            EngineOptions {
                volume_bytes: Some(1234),
                ..normal.clone()
            },
            EngineOptions {
                threads: Some(0),
                ..normal.clone()
            },
            EngineOptions {
                threads: Some(65),
                ..normal.clone()
            },
            EngineOptions {
                codepage: Some(932),
                ..normal.clone()
            },
            EngineOptions {
                encryption: Some("aes256".into()),
                ..normal.clone()
            },
            EngineOptions {
                dictionary_bytes: Some(4096),
                ..normal.clone()
            },
            EngineOptions {
                header_encryption: Some(false),
                ..normal.clone()
            },
            EngineOptions {
                solid: Some(false),
                ..normal.clone()
            },
        ] {
            assert!(!eligible("zip", &inputs, &options));
        }
        crate::machine_policy::with_rules([&[], &[], &[], &["blocked codec"]], || {
            assert!(!try_create("missing.zip", &inputs, &normal, &JobControl::new()).unwrap());
        });
    }
}
