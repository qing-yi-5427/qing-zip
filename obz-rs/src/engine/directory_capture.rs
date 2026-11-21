//! Read the ordinary engine's technical listing while it is being written.
//! The private spool remains authoritative for errors and unusual records.
use super::{append_listing_lines, Entry, JobControl, OUTPUT_LIMIT};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(windows)]
pub(super) fn pipe() -> std::io::Result<(File, File)> {
    use std::{ffi::c_void, os::windows::io::FromRawHandle, ptr::null_mut};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(
            read: *mut *mut c_void,
            write: *mut *mut c_void,
            attributes: *mut c_void,
            size: u32,
        ) -> i32;
    }
    let (mut read, mut write) = (null_mut(), null_mut());
    if unsafe { CreatePipe(&mut read, &mut write, null_mut(), 256 * 1024) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (File::from_raw_handle(read), File::from_raw_handle(write)) })
}
#[cfg(not(windows))]
pub(super) fn pipe() -> std::io::Result<(File, File)> {
    Err(std::io::ErrorKind::Unsupported.into())
}

pub(super) struct DirectoryCapture {
    stopped: Arc<AtomicBool>,
    task: Option<JoinHandle<Result<Option<Vec<Entry>>, String>>>,
}

impl DirectoryCapture {
    pub(super) fn start(pipe: File, spool: File, ctl: JobControl) -> Result<Self, String> {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        let task = thread::Builder::new()
            .name("archive-directory".into())
            .spawn(move || capture(pipe, spool, &ctl, &flag))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            stopped,
            task: Some(task),
        })
    }

    pub(super) fn ended(&self) -> bool {
        self.task.as_ref().is_some_and(JoinHandle::is_finished)
    }

    pub(super) fn finish(&mut self) -> Result<Option<Vec<Entry>>, String> {
        self.stopped.store(true, Ordering::Release);
        self.task
            .take()
            .expect("directory capture running")
            .join()
            .map_err(|_| "archive directory reader failed".to_string())?
    }
}

impl Drop for DirectoryCapture {
    fn drop(&mut self) {
        if self.task.is_some() {
            let _ = self.finish();
        }
    }
}

#[cfg(windows)]
fn available(pipe: &File) -> Result<usize, String> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn PeekNamedPipe(
            pipe: *mut std::ffi::c_void,
            buf: *mut u8,
            size: u32,
            read: *mut u32,
            available: *mut u32,
            remaining: *mut u32,
        ) -> i32;
    }
    let mut n = 0;
    let ok = unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut n,
            std::ptr::null_mut(),
        )
    };
    if ok != 0 {
        return Ok(n as usize);
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(109) {
        return Ok(0);
    }
    Err(err.to_string())
}
#[cfg(not(windows))]
fn available(_: &File) -> Result<usize, String> {
    Err("unsupported pipe platform".into())
}

// This allowance applies only to a bare listing that is being reduced to
// validated entries. Keep the legacy replay and diagnostic spool at its
// original limit and never spool the excess.
#[derive(Clone, Copy)]
struct DirectoryLimits {
    spool: u64,
    wire: u64,
    entries: usize,
    text: usize,
    record: usize,
}
const DIRECTORY_LIMITS: DirectoryLimits = DirectoryLimits {
    spool: OUTPUT_LIMIT,
    wire: 256 * 1024 * 1024,
    entries: 512_000,
    text: 128 * 1024 * 1024,
    record: 1024 * 1024,
};

fn capture(
    pipe: File,
    spool: File,
    ctl: &JobControl,
    stopped: &AtomicBool,
) -> Result<Option<Vec<Entry>>, String> {
    capture_with_limits(pipe, spool, ctl, stopped, DIRECTORY_LIMITS)
}

fn capture_with_limits(
    mut pipe: File,
    mut spool: File,
    ctl: &JobControl,
    stopped: &AtomicBool,
    limits: DirectoryLimits,
) -> Result<Option<Vec<Entry>>, String> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    let mut block = vec![0; 128 * 1024];
    let mut total = 0_u64;
    let mut text_bytes = 0;
    let mut ordinary = true;
    loop {
        ctl.check()?;
        let finished = stopped.load(Ordering::Acquire);
        let n = available(&pipe)?.min(block.len());
        if n == 0 {
            if finished {
                break;
            }
            thread::sleep(Duration::from_millis(1));
            continue;
        }
        let count = pipe.read(&mut block[..n]).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        let spool_bytes = limits.spool.saturating_sub(total).min(count as u64) as usize;
        spool
            .write_all(&block[..spool_bytes])
            .map_err(|e| e.to_string())?;
        total += count as u64;
        if total > limits.wire {
            return Err("engine listing exceeds supported limit".into());
        }
        if ordinary {
            let start = pending.len();
            pending.extend_from_slice(&block[..count]);
            if let Some(end) = boundary(&pending, start) {
                ordinary = extend_directory(
                    &pending[..end],
                    &mut entries,
                    &mut seen,
                    &mut text_bytes,
                    limits,
                )?;
                pending.drain(..end);
            }
            if pending.len() > limits.record {
                return Err("engine listing record exceeds supported limit".into());
            }
            if !ordinary {
                entries = Vec::new();
                seen = HashSet::new();
                pending = Vec::new();
            }
        }
        if !ordinary && total > limits.spool {
            return Err("engine listing exceeds supported limit".into());
        }
    }
    let valid = ordinary
        && extend_directory(&pending, &mut entries, &mut seen, &mut text_bytes, limits)?
        && !entries.is_empty();
    if valid {
        Ok(Some(entries))
    } else if total > limits.spool {
        Err("engine listing exceeds supported limit".into())
    } else {
        Ok(None)
    }
}

fn extend_directory(
    block: &[u8],
    entries: &mut Vec<Entry>,
    seen: &mut HashSet<String>,
    text_bytes: &mut usize,
    limits: DirectoryLimits,
) -> Result<bool, String> {
    if block.len() > limits.record {
        let mut start = 0;
        for i in 0..block.len() {
            if is_record_end(block, i) {
                if i + 1 - start > limits.record {
                    return Err("engine listing record exceeds supported limit".into());
                }
                start = i + 1;
            }
        }
        if block.len() - start > limits.record {
            return Err("engine listing record exceeds supported limit".into());
        }
    }
    let old_count = entries.len();
    let valid = parse(block, entries, seen);
    if entries.len() > limits.entries {
        return Err("engine listing exceeds supported entry limit".into());
    }
    *text_bytes += entries[old_count..]
        .iter()
        .map(|entry| {
            // Account for the name and separately retained collision key.
            entry.name.len() * 2
                + entry.created.len()
                + entry.accessed.len()
                + entry.crc.len()
                + entry.modified.len()
                + entry.method.len()
                + entry.comment.len()
                + entry.attributes.len()
        })
        .sum::<usize>();
    if *text_bytes > limits.text {
        return Err("engine listing exceeds supported metadata limit".into());
    }
    Ok(valid)
}

fn is_record_end(buf: &[u8], i: usize) -> bool {
    buf[i] == b'\n' && (buf[..i].ends_with(b"\n") || buf[..i].ends_with(b"\n\r"))
}

fn boundary(buf: &[u8], start: usize) -> Option<usize> {
    (start..buf.len())
        .rev()
        .find(|&i| is_record_end(buf, i))
        .map(|i| i + 1)
}

fn parse(block: &[u8], entries: &mut Vec<Entry>, seen: &mut HashSet<String>) -> bool {
    let Ok(text) = std::str::from_utf8(block) else {
        return false;
    };
    if text.contains("\r\r\n") {
        return false;
    }
    let lines = text.lines().map(|line| (!line.is_empty()).then_some(line));
    append_listing_lines(lines.chain(std::iter::once(None)), entries, seen).is_ok()
}
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Seek;

    #[test]
    fn finished_writer_with_another_open_handle_drains_without_waiting_for_eof() {
        let (reader, mut writer) = pipe().unwrap();
        let bytes = b"Path = first\r\nSize = 4\r\n\r\nPath = last\nSize = 6";
        writer.write_all(bytes).unwrap();
        let mut spool = tempfile::tempfile().unwrap();
        let result = capture(
            reader,
            spool.try_clone().unwrap(),
            &JobControl::new(),
            &AtomicBool::new(true),
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!((result[0].size, result[1].size), (4, 6));
        spool.rewind().unwrap();
        let mut original = Vec::new();
        spool.read_to_end(&mut original).unwrap();
        assert_eq!(original, bytes);
    }

    #[test]
    fn rejected_live_records_preserve_the_exact_private_spool() {
        let (reader, mut writer) = pipe().unwrap();
        let bytes = b"Path = benign\nSize = 1\n\nPath = ../bad\nSize = 2\n\n\xff";
        writer.write_all(bytes).unwrap();
        let mut spool = tempfile::tempfile().unwrap();
        assert!(capture(
            reader,
            spool.try_clone().unwrap(),
            &JobControl::new(),
            &AtomicBool::new(true)
        )
        .unwrap()
        .is_none());
        spool.rewind().unwrap();
        let mut original = Vec::new();
        spool.read_to_end(&mut original).unwrap();
        assert_eq!(original, bytes);
        assert!(super::super::parse_listing(&original)
            .unwrap_err()
            .contains("UTF-8"));
    }
    fn through_pipe(text: Vec<u8>) -> Option<Vec<Entry>> {
        let (reader, mut writer) = pipe().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let writer_done = Arc::clone(&done);
        let producer = thread::spawn(move || {
            writer.write_all(&text).unwrap();
            drop(writer);
            writer_done.store(true, Ordering::Release);
        });
        let entries = capture(
            reader,
            tempfile::tempfile().unwrap(),
            &JobControl::new(),
            &done,
        )
        .unwrap();
        producer.join().unwrap();
        entries
    }

    #[test]
    fn streaming_keeps_one_collision_set_and_all_variable_length_properties() {
        let mut text = String::new();
        for i in 0..17_000 {
            use std::fmt::Write;
            writeln!(text, "Path = data/{i}\nSize = 3\nComment = alpha\n").unwrap();
        }
        let entries = through_pipe(text.as_bytes().to_vec()).unwrap();
        assert_eq!(entries.len(), 17_000);
        assert_eq!(entries.last().unwrap().name, "data/16999");
        text.push_str("Path = DATA/0\nSize = 0");
        assert!(through_pipe(text.into_bytes()).is_none());
        let wide = format!(
            "Path = before\n\nPath = wide\nComment = {}\n\nPath = after",
            "y".repeat(350_000)
        );
        let entries = through_pipe(wide.into_bytes()).unwrap();
        assert_eq!(entries[1].comment.len(), 350_000);
        assert_eq!(entries.last().unwrap().name, "after");
    }

    #[test]
    fn ordinary_directory_can_exceed_diagnostic_spool_without_losing_entries() {
        let text = "Path = first\nSize = 4\n\nPath = second\nSize = 6\n".as_bytes();
        let (reader, mut writer) = pipe().unwrap();
        writer.write_all(text).unwrap();
        let mut spool = tempfile::tempfile().unwrap();
        let entries = capture_with_limits(
            reader,
            spool.try_clone().unwrap(),
            &JobControl::new(),
            &AtomicBool::new(true),
            DirectoryLimits {
                spool: 24,
                ..DIRECTORY_LIMITS
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        spool.rewind().unwrap();
        let mut captured = Vec::new();
        spool.read_to_end(&mut captured).unwrap();
        assert_eq!(&captured, &text[..24]);
    }

    #[test]
    fn directory_budgets_are_terminal_even_if_legacy_replay_fits() {
        let text = b"Path = first\nSize = 4\n\nPath = second\nSize = 6\n\n";
        for (limits, category) in [
            (
                DirectoryLimits {
                    record: 20,
                    ..DIRECTORY_LIMITS
                },
                "record",
            ),
            (
                DirectoryLimits {
                    entries: 1,
                    ..DIRECTORY_LIMITS
                },
                "entry",
            ),
            (
                DirectoryLimits {
                    text: 4,
                    ..DIRECTORY_LIMITS
                },
                "metadata",
            ),
        ] {
            let (reader, mut writer) = pipe().unwrap();
            writer.write_all(text).unwrap();
            let error = capture_with_limits(
                reader,
                tempfile::tempfile().unwrap(),
                &JobControl::new(),
                &AtomicBool::new(true),
                limits,
            )
            .unwrap_err();
            assert!(error.contains(category), "{error}");
        }
    }

    #[test]
    fn cancelling_a_live_capture_interrupts_an_idle_writer() {
        let (reader, _writer) = pipe().unwrap();
        let ctl = JobControl::new();
        let mut capture =
            DirectoryCapture::start(reader, tempfile::tempfile().unwrap(), ctl.clone()).unwrap();
        ctl.cancel();
        assert_eq!(capture.finish().unwrap_err(), "operation cancelled");
    }

    #[test]
    fn cancelling_normal_list_does_not_publish_partial_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("many.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for i in 0..9000 {
            zip.start_file(format!("entry-{i}.txt"), options).unwrap();
        }
        drop(zip.finish().unwrap());
        let control = JobControl::new();
        let list_control = control.clone();
        let path = archive.clone();
        let task = thread::spawn(move || {
            super::super::list_with_control(
                path.to_str().unwrap(),
                &super::super::EngineOptions::default(),
                &list_control,
            )
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while control.snapshot().phase != "listing" {
            assert!(!task.is_finished() && std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        control.cancel();
        assert_eq!(task.join().unwrap().unwrap_err(), "operation cancelled");
        std::fs::remove_file(archive).unwrap();
    }
}
