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
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    let mut block = vec![0; 128 * 1024];
    let mut total = 0_u64;
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
        if ordinary {
            let prev = pending.len();
            pending.extend_from_slice(&block[..count]);
            if let Some(i) = (prev..pending.len()).rev().find(|&i| {
                pending[i] == b'\n'
                    && (pending[..i].ends_with(b"\n") || pending[..i].ends_with(b"\n\r"))
            }) {
                ordinary = parse(&pending[..=i], &mut entries, &mut seen);
                pending.drain(..=i);
            }
        }
    }
    Ok(
        (ordinary && parse(&pending, &mut entries, &mut seen) && !entries.is_empty())
            .then_some(entries),
    )
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
}
