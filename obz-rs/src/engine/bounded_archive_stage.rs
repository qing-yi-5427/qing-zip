//! Stream qualified small ZIP and single-folder 7z members into an already private same-volume
//! transaction. Only the producer touches the ZIP reader; writers receive
//! independently CRC-verified small buffers. A larger member streams directly
//! to a private file with a fixed scratch buffer. Nothing is visible yet.
use super::*;
use std::sync::{mpsc, Mutex};

const QUEUED_MEMBER_LIMIT: u64 = 256 * 1024;

fn eligible(entries: &[Entry]) -> bool {
    entries.len() <= MEMORY_ZIP_ENTRY_LIMIT
        && entries.iter().filter(|e| !e.is_dir).count() >= 128
        && entries.iter().all(|e| {
            e.size_known
                && !e.encrypted
                && e.size <= MEMORY_ZIP_FILE_LIMIT
                && (!e.is_dir || e.size == 0)
        })
        && entries
            .iter()
            .try_fold(0u64, |n, e| n.checked_add(e.size))
            .is_some_and(|n| n <= MEMORY_ZIP_LIMIT)
}

pub(super) struct ZipPlan {
    archive: zip::ZipArchive<std::io::BufReader<File>>,
    order: Vec<usize>,
}

pub(super) struct SevenStreamPlan {
    archive: PathBuf,
    backend: crate::machine_policy::Backend,
    parsed: crate::sevenz::MemoryArchive,
    order: Vec<usize>,
}

pub(super) enum Plan {
    Zip(ZipPlan),
    Seven(SevenStreamPlan),
}

pub(super) fn plan_transaction(
    path: &str,
    dest: &Path,
    source: Option<&mut File>,
    entries: &[Entry],
    ctl: &JobControl,
    output: &mut CheckedDirectories,
) -> Result<Option<(Plan, DirectoryTransaction)>, String> {
    let Some(source) = source else {
        return Ok(None);
    };
    let ext = Path::new(path).extension();
    let zip = ext.is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if !eligible(entries)
        || (!zip
            && (!cfg!(nana_selected_worker)
                || !ext.is_some_and(|e| e.eq_ignore_ascii_case("7z"))))
    {
        return Ok(None);
    }
    let Some(transaction) = new_directory_stage(path, dest, true, output, entries, true)? else {
        return Ok(None);
    };
    // try_clone shares the underlying cursor. The original reader is idle during
    // qualification and is rewound if the compatibility decoder will use it.
    let reader = source.try_clone().map_err(|e| e.to_string())?;
    let plan = if zip {
        prepare(reader, entries, ctl)?.map(Plan::Zip)
    } else {
        let selected = executable()?;
        if selected
            .file_name()
            .is_none_or(|n| n != "NanaZip.Universal.Console.exe")
        {
            return Ok(None);
        }
        let manifest = prepare_memory_sevenz(
            Some(reader.try_clone().map_err(|e| e.to_string())?),
            entries,
            ctl,
        )?;
        if let Some(SevenMemoryPlan {
            data,
            parsed,
            order,
        }) = manifest
        {
            parsed.validate_stream(&data, &|| ctl.check())?;
            // Qualification and extraction share this read-only, deny-write/delete pin.
            // Release compressed bytes before the child allocates its dictionary.
            drop(data);
            Some(Plan::Seven(SevenStreamPlan {
                archive: absolute(path)?,
                backend: crate::machine_policy::verify_backend_at(
                    selected.parent().ok_or("missing decoder directory")?,
                )?,
                parsed,
                order,
            }))
        } else {
            None
        }
    };
    match plan {
        Some(plan) => Ok(Some((plan, transaction))),
        None => {
            source.rewind().map_err(|e| e.to_string())?;
            Ok(None)
        }
    }
}

fn prepare(
    mut source: File,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<Option<ZipPlan>, String> {
    if !ordinary_zip_file_layout(&mut source, entries, ctl)? {
        return Ok(None);
    }
    source.rewind().map_err(|e| e.to_string())?;
    let Ok(mut archive) =
        zip::ZipArchive::new(std::io::BufReader::with_capacity(64 * 1024, source))
    else {
        return Ok(None);
    };
    let Some(order) = verified_zip_order(&mut archive, entries, ctl)? else {
        return Ok(None);
    };
    Ok(Some(ZipPlan { archive, order }))
}

#[cfg(nana_selected_worker)]
mod solid_process {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
        },
    };
    type Handle = *mut c_void;
    extern "C" {
        fn qing_spawn_solid_reader(
            exe: *const u16,
            archive: *const u16,
            threads: u32,
            process: *mut Handle,
            output: *mut Handle,
        ) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(handle: Handle, code: *mut u32) -> i32;
        fn TerminateProcess(handle: Handle, code: u32) -> i32;
    }
    struct Process(OwnedHandle);
    impl Drop for Process {
        fn drop(&mut self) {
            unsafe {
                let h = self.0.as_raw_handle();
                if WaitForSingleObject(h, 0) != 0 {
                    TerminateProcess(h, 2);
                    WaitForSingleObject(h, u32::MAX);
                }
            }
        }
    }
    impl Process {
        fn wait_slice(&self) -> Result<Option<u32>, String> {
            let h = self.0.as_raw_handle();
            match unsafe { WaitForSingleObject(h, 10) } {
                258 => Ok(None),
                0 => {
                    let mut exit = 0;
                    if unsafe { GetExitCodeProcess(h, &mut exit) } == 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                    Ok(Some(exit))
                }
                _ => Err(std::io::Error::last_os_error().to_string()),
            }
        }
    }

    /// Restrict inherited handles to standard I/O and forbid descendants.
    /// The monitor releases blocked reads by killing the child on cancellation.
    pub(super) fn consume(
        program: &Path,
        archive: &Path,
        ctl: &JobControl,
        threads: Option<u32>,
        consume: impl FnOnce(&mut dyn Read) -> Result<(), String>,
    ) -> Result<(), String> {
        ctl.check()?;
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let exe = wide(program);
        let path = wide(archive);
        let (mut proc, mut out) = (std::ptr::null_mut(), std::ptr::null_mut());
        let error = unsafe {
            qing_spawn_solid_reader(
                exe.as_ptr(),
                path.as_ptr(),
                threads.unwrap_or(0),
                &mut proc,
                &mut out,
            )
        };
        if error != 0 {
            return Err(format!(
                "start solid decoder: {}",
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
        let process = Process(unsafe { OwnedHandle::from_raw_handle(proc) });
        let stdout = unsafe { File::from_raw_handle(out) };
        thread::scope(|scope| {
            let (stop, rx) = mpsc::channel();
            let monitor = thread::Builder::new()
                .name("solid-decoder-monitor".into())
                .spawn_scoped(scope, move || loop {
                    ctl.check()?;
                    if let Some(code) = process.wait_slice()? {
                        return if code == 0 {
                            Ok(())
                        } else {
                            Err(format!("solid decoder exited unsuccessfully ({code})"))
                        };
                    }
                    match rx.try_recv() {
                        Err(mpsc::TryRecvError::Empty) => (),
                        Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                            return Err("solid decode interrupted".into())
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            let mut buffered = std::io::BufReader::with_capacity(64 * 1024, stdout);
            let result = consume(&mut buffered);
            drop(buffered);
            if result.is_err() {
                let _ = stop.send(());
            }
            let status = monitor
                .join()
                .map_err(|_| "solid decoder monitor failed".to_owned())?;
            ctl.check()?;
            result.and(status)
        })
    }
}
#[cfg(not(nana_selected_worker))]
mod solid_process {
    use super::*;
    pub(super) fn consume(
        _: &Path,
        _: &Path,
        _: &JobControl,
        _: Option<u32>,
        _: impl FnOnce(&mut dyn Read) -> Result<(), String>,
    ) -> Result<(), String> {
        Err("isolated solid reader unavailable".into())
    }
}

impl Plan {
    fn read_members(
        self,
        ctl: &JobControl,
        threads: Option<u32>,
        mut visit: impl FnMut(usize, usize, &mut dyn Read, Option<u32>, bool) -> Result<(), String>,
    ) -> Result<(), String> {
        match self {
            Self::Zip(ZipPlan { mut archive, order }) => {
                for (i, index) in order.into_iter().enumerate() {
                    ctl.check()?;
                    let mut member = archive
                        .by_index(i)
                        .map_err(|e| format!("ZIP decode: {e}"))?;
                    let crc = member.crc32();
                    visit(i, index, &mut member, Some(crc), true)?;
                }
                Ok(())
            }
            Self::Seven(SevenStreamPlan {
                archive,
                backend,
                parsed,
                order,
            }) => {
                solid_process::consume(&backend.exe, &archive, ctl, threads, |stdout| {
                    parsed.verify_output(stdout, &|| ctl.check(), |folder| {
                        for (i, (&index, member)) in
                            order.iter().zip(&parsed.archive.entries).enumerate()
                        {
                            ctl.check()?;
                            // Empty streams have no folder association or CRC.
                            if member.folder_index.is_none() {
                                visit(i, index, &mut std::io::empty(), member.crc, true)?;
                            } else {
                                visit(i, index, folder, member.crc, false)?;
                            }
                        }
                        Ok(())
                    })
                })
            }
        }
    }
    pub(super) fn decode(
        self,
        entries: &[Entry],
        root: &Path,
        guards: &mut CheckedDirectories,
        ctl: &JobControl,
        thread_limit: Option<u32>,
    ) -> Result<(), String> {
        let timer = match self {
            Self::Zip(_) => "bounded-zip-private-stage",
            Self::Seven(_) => "bounded-7z-private-stage",
        };
        let _timer = PhaseTimer::new(timer);
        let mut targets = Vec::with_capacity(entries.len());
        let mut parents = HashSet::new();
        for entry in entries {
            ctl.check()?;
            let target = root.join(validate_name(&entry.name)?);
            let parent = if entry.is_dir {
                target.as_path()
            } else {
                target.parent().ok_or("missing private ZIP parent")?
            };
            if !parents.contains(parent) {
                guards.create_dirs(parent)?;
                parents.insert(parent.to_owned());
            }
            targets.push(target);
        }
        drop(parents);
        let guards: &CheckedDirectories = guards;
        let workers = thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(thread_limit.unwrap_or(4).clamp(1, 4) as usize);
        let (send, receive) = mpsc::sync_channel::<(usize, Vec<u8>)>(workers);
        let receive = Arc::new(Mutex::new(receive));
        let (recycle, reused) = mpsc::channel::<Vec<u8>>();
        ctl.set("decoding", Some(0), "Verifying archive and staging files");
        let stopped = AtomicBool::new(false);
        thread::scope(|scope| {
            let mut handles = Vec::with_capacity(workers);
            for _ in 0..workers {
                let receive = Arc::clone(&receive);
                let recycle = recycle.clone();
                let targets = &targets;
                let stopped = &stopped;
                handles.push(
                    thread::Builder::new()
                        .spawn_scoped(scope, move || -> Result<(), String> {
                            loop {
                                let next = receive
                                    .lock()
                                    .map_err(|_| "archive staging queue poisoned")?
                                    .recv();
                                let Ok((index, bytes)) = next else {
                                    return Ok(());
                                };
                                if stopped.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                if let Err(e) = write_verified_bytes(
                                    &entries[index],
                                    &targets[index],
                                    &bytes,
                                    guards,
                                    ctl,
                                ) {
                                    stopped.store(true, Ordering::Release);
                                    return Err(e);
                                }
                                let _ = recycle.send(bytes);
                            }
                        })
                        .map_err(|e| e.to_string())?,
                );
            }
            // No unused receiver may keep a full queue alive if its writers fail.
            drop(receive);
            drop(recycle);
            let decode = (|| -> Result<(), String> {
                let mut scratch = [0u8; 64 * 1024];
                self.read_members(
                    ctl,
                    thread_limit,
                    |i, index, member, expected_crc, check_eof| {
                        ctl.check()?;
                        if stopped.load(Ordering::Acquire) {
                            return Err("private archive writer stopped".into());
                        }
                        let entry = &entries[index];
                        if entry.size > QUEUED_MEMBER_LIMIT {
                            use std::io::Write;
                            let mut file = open_prepared_stage_output(&targets[index], guards)?;
                            let mut remaining = entry.size;
                            let mut crc = crc32fast::Hasher::new();
                            while remaining != 0 {
                                ctl.check()?;
                                let n = remaining.min(scratch.len() as u64) as usize;
                                member
                                    .read_exact(&mut scratch[..n])
                                    .map_err(|e| format!("archive integrity: {e}"))?;
                                crc.update(&scratch[..n]);
                                file.write_all(&scratch[..n]).map_err(|e| e.to_string())?;
                                remaining -= n as u64;
                            }
                            if (check_eof
                                && member
                                    .read(&mut [0; 1])
                                    .map_err(|e| format!("archive integrity: {e}"))?
                                    != 0)
                                || expected_crc.is_some_and(|want| crc.finalize() != want)
                            {
                                return Err("archive size or CRC mismatch".into());
                            }
                            restore_open_file_metadata(&file, entry)?;
                        } else {
                            let mut bytes = reused.try_recv().unwrap_or_default();
                            let length = entry.size as usize;
                            bytes
                                .try_reserve_exact(length.saturating_sub(bytes.len()))
                                .map_err(|e| e.to_string())?;
                            bytes.resize(length, 0);
                            for chunk in bytes.chunks_mut(64 * 1024) {
                                ctl.check()?;
                                member
                                    .read_exact(chunk)
                                    .map_err(|e| format!("archive integrity: {e}"))?;
                            }
                            if (check_eof
                                && member
                                    .read(&mut [0; 1])
                                    .map_err(|e| format!("archive integrity: {e}"))?
                                    != 0)
                                || expected_crc.is_some_and(|want| crc32fast::hash(&bytes) != want)
                            {
                                return Err("archive size or CRC mismatch".into());
                            }
                            if !entry.is_dir {
                                send.send((index, bytes))
                                    .map_err(|_| "private archive writers stopped")?;
                            }
                        }
                        if i % 64 == 0 {
                            ctl.set(
                                "decoding",
                                Some((i * 100 / entries.len()) as u8),
                                "Verifying archive and staging files",
                            );
                        }
                        Ok(())
                    },
                )
            })();
            if decode.is_err() {
                stopped.store(true, Ordering::Release);
            }
            drop(send);
            let mut write = Ok(());
            for handle in handles {
                write = write.and(
                    handle
                        .join()
                        .map_err(|_| "private archive writer panicked".to_owned())
                        .and_then(|r| r),
                );
            }
            write.and(decode)
        })?;
        ctl.check()
    }
}
