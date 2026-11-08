//! Read the ordinary engine's technical listing while it is being written.
//! The private spool remains authoritative for errors and unusual records.
use super::{append_listing_lines, zip_u16, Entry, JobControl, OUTPUT_LIMIT};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
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

pub(super) fn large_zip_hint(file: &mut File) -> bool {
    // The terminal classic count is a routing hint only. ZIP64, comments and
    // other layouts always retain the original path and all validations.
    let Ok(position) = file.stream_position() else {
        return false;
    };
    let mut tail = [0; 22];
    let found = file
        .seek(SeekFrom::End(-22))
        .and_then(|_| file.read_exact(&mut tail))
        .is_ok()
        && tail.starts_with(b"PK\x05\x06")
        && tail[4..8] == [0; 4]
        && tail[20..] == [0; 2]
        && tail[8..10] == tail[10..12]
        && (2048..u16::MAX).contains(&zip_u16(&tail, 10));
    file.seek(SeekFrom::Start(position)).is_ok() && found
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

fn capture(
    mut pipe: File,
    mut spool: File,
    ctl: &JobControl,
    stopped: &AtomicBool,
) -> Result<Option<Vec<Entry>>, String> {
    const LIVE_ENTRIES: usize = 16_384;
    const LIVE_TEXT_BYTES: u64 = 8 * 1024 * 1024;
    const LARGE_RECORD: usize = 256 * 1024;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    let mut block = vec![0; 128 * 1024];
    let mut total = 0_u64;
    let mut accepted = 0_u64;
    let mut deferred = false;
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
        total += count as u64;
        if total > OUTPUT_LIMIT {
            return Err("engine listing exceeds supported limit".into());
        }
        spool
            .write_all(&block[..count])
            .map_err(|e| e.to_string())?;
        if ordinary && !deferred {
            if total > LIVE_TEXT_BYTES {
                deferred = true;
            } else {
                let start = pending.len();
                pending.extend_from_slice(&block[..count]);
                if let Some(end) = boundary(&pending, start) {
                    // Bound how many entries a batch can add before retaining it.
                    let upper = pending[..end].len() / 6 + 1;
                    if entries.len().saturating_add(upper) > LIVE_ENTRIES
                        && entries.len().saturating_add(
                            pending[..end]
                                .split(|b| *b == b'\n')
                                .filter(|l| l.is_empty() || *l == b"\r")
                                .count()
                                + 1,
                        ) > LIVE_ENTRIES
                    {
                        deferred = true;
                    } else {
                        ordinary = parse(&pending[..end], &mut entries, &mut seen);
                        accepted += end as u64;
                        pending.drain(..end);
                    }
                }
            }
            if pending.len() > LARGE_RECORD {
                deferred = true;
            }
            if deferred {
                pending = Vec::new();
            }
        }
    }
    if ordinary && deferred {
        pending.clear();
        spool
            .seek(SeekFrom::Start(accepted))
            .map_err(|e| e.to_string())?;
        while accepted < total {
            ctl.check()?;
            let length = (total - accepted).min(block.len() as u64) as usize;
            spool
                .read_exact(&mut block[..length])
                .map_err(|e| e.to_string())?;
            let start = pending.len();
            pending.extend_from_slice(&block[..length]);
            accepted += length as u64;
            if let Some(end) = boundary(&pending, start) {
                ordinary = parse(&pending[..end], &mut entries, &mut seen);
                if !ordinary {
                    break;
                }
                pending.drain(..end);
            }
        }
    }
    Ok(
        (ordinary && parse(&pending, &mut entries, &mut seen) && !entries.is_empty())
            .then_some(entries),
    )
}

fn boundary(buf: &[u8], start: usize) -> Option<usize> {
    (start..buf.len())
        .rev()
        .find(|&i| buf[i] == b'\n' && (buf[..i].ends_with(b"\n") || buf[..i].ends_with(b"\n\r")))
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
    fn deferred_spool_keeps_one_collision_set_and_all_variable_length_properties() {
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
