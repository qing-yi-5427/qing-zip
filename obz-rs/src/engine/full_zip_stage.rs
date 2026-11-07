//! Stream ordinary small ZIP members into an already private same-volume
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

pub(super) struct Plan {
    archive: zip::ZipArchive<std::io::BufReader<File>>,
    order: Vec<usize>,
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
    if !Path::new(path)
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
        || !eligible(entries)
    {
        return Ok(None);
    }
    let Some(transaction) = new_directory_stage(path, dest, true, output, entries, true)? else {
        return Ok(None);
    };
    // try_clone shares the underlying cursor. The original reader is idle during
    // qualification and is rewound if the compatibility decoder will use it.
    let reader = source.try_clone().map_err(|e| e.to_string())?;
    match prepare(reader, entries, ctl)? {
        Some(plan) => Ok(Some((plan, transaction))),
        None => {
            source.rewind().map_err(|e| e.to_string())?;
            Ok(None)
        }
    }
}

fn prepare(mut source: File, entries: &[Entry], ctl: &JobControl) -> Result<Option<Plan>, String> {
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
    Ok(Some(Plan { archive, order }))
}

impl Plan {
    pub(super) fn decode(
        mut self,
        entries: &[Entry],
        root: &Path,
        guards: &mut CheckedDirectories,
        ctl: &JobControl,
        thread_limit: Option<u32>,
    ) -> Result<(), String> {
        let _timer = PhaseTimer::new("bounded-zip-private-stage");
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
                                    .map_err(|_| "ZIP staging queue poisoned")?
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
                for (i, index) in self.order.into_iter().enumerate() {
                    ctl.check()?;
                    if stopped.load(Ordering::Acquire) {
                        return Err("private ZIP writer stopped".into());
                    }
                    let entry = &entries[index];
                    let mut member = self
                        .archive
                        .by_index(i)
                        .map_err(|e| format!("ZIP decode: {e}"))?;
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
                                .map_err(|e| format!("ZIP integrity: {e}"))?;
                            crc.update(&scratch[..n]);
                            file.write_all(&scratch[..n]).map_err(|e| e.to_string())?;
                            remaining -= n as u64;
                        }
                        if member
                            .read(&mut [0; 1])
                            .map_err(|e| format!("ZIP integrity: {e}"))?
                            != 0
                            || crc.finalize() != member.crc32()
                        {
                            return Err("ZIP size or CRC mismatch".into());
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
                                .map_err(|e| format!("ZIP integrity: {e}"))?;
                        }
                        if member
                            .read(&mut [0; 1])
                            .map_err(|e| format!("ZIP integrity: {e}"))?
                            != 0
                            || crc32fast::hash(&bytes) != member.crc32()
                        {
                            return Err("ZIP size or CRC mismatch".into());
                        }
                        if !entry.is_dir {
                            send.send((index, bytes))
                                .map_err(|_| "private ZIP writers stopped")?;
                        }
                    }
                    if i % 64 == 0 {
                        ctl.set(
                            "decoding",
                            Some((i * 100 / entries.len()) as u8),
                            "Verifying archive and staging files",
                        );
                    }
                }
                Ok(())
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
                        .map_err(|_| "private ZIP writer panicked".to_owned())
                        .and_then(|r| r),
                );
            }
            write.and(decode)
        })?;
        ctl.check()
    }
}
