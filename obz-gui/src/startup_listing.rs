use std::{
    fs::File,
    os::windows::io::AsRawHandle,
    path::Path,
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};

use obz_core::{
    engine::{pin_archive_for_listing, standalone_7z_source, JobControl},
    operations::Request,
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        FileBasicInfo, GetDriveTypeW, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
    },
};

struct Prepared {
    path: String,
    revision: Revision,
    started: Instant,
    id: u64,
}

#[derive(PartialEq, Eq)]
struct Revision {
    volume: u32,
    file_index: (u32, u32),
    creation: i64,
    modified: i64,
    changed: i64,
    size: u64,
}

static PREPARED: Mutex<Option<Prepared>> = Mutex::new(None);
static CLAIMED: Condvar = Condvar::new();
const MAX_AGE: Duration = Duration::from_secs(10);
// Bound speculative ZIP metadata by directory bytes and entry count.
const MAX_ZIP_DIRECTORY_BYTES: u64 = 32 * 1024 * 1024;
const MAX_7Z_BYTES: u64 = 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 128_000;

fn fixed_local_path(path: &str) -> bool {
    // A mapped/network/device path must not delay creation of the main window.
    let bytes = path.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) || bytes.get(1..3) != Some(b":\\") {
        return false;
    }
    let root = [u16::from(bytes[0]), b':' as u16, b'\\' as u16, 0];
    const DRIVE_FIXED: u32 = 3;
    (unsafe { GetDriveTypeW(windows::core::PCWSTR(root.as_ptr())) }) == DRIVE_FIXED
}

fn revision(file: &File) -> Option<Revision> {
    let handle = HANDLE(file.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let mut basic = FILE_BASIC_INFO::default();
    unsafe {
        GetFileInformationByHandle(handle, &mut info).ok()?;
        GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .ok()?;
    }
    if info.nNumberOfLinks != 1 || info.dwFileAttributes & (0x400 | 0x10 | 0x1000) != 0 {
        return None;
    }
    Some(Revision {
        volume: info.dwVolumeSerialNumber,
        file_index: (info.nFileIndexHigh, info.nFileIndexLow),
        creation: basic.CreationTime,
        modified: basic.LastWriteTime,
        changed: basic.ChangeTime,
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
    })
}

fn listing_request(path: String) -> Request {
    Request {
        operation: "list".into(),
        path,
        detect_header_encryption: true,
        ..Request::default()
    }
}

/// Called after the single-instance plugin and before creation of the WebView.
pub(crate) fn prepare(path: String) {
    let Some(extension) = Path::new(&path)
        .extension()
        .and_then(|value| value.to_str())
    else {
        return;
    };
    let zip = extension.eq_ignore_ascii_case("zip");
    if !zip && !extension.eq_ignore_ascii_case("7z") {
        return;
    }
    if !fixed_local_path(&path) {
        return;
    }
    let Ok(file) = pin_archive_for_listing(Path::new(&path)) else {
        return;
    };
    let Some(expected) = revision(&file) else {
        return;
    };
    let standalone = if zip {
        crate::jobs::pinned_zip_directory(&file)
            .ok()
            .flatten()
            .is_some_and(|directory| {
                directory.bytes <= MAX_ZIP_DIRECTORY_BYTES && directory.entries <= MAX_ZIP_ENTRIES
            })
    } else {
        expected.size <= MAX_7Z_BYTES && standalone_7z_source(&file).unwrap_or(false)
    };
    if !standalone {
        return;
    }

    // Pin the directory chain and the same leaf throughout the speculative read.
    let Ok(id) = crate::jobs::start_controlled(
        listing_request(path.clone()),
        false,
        JobControl::new(),
        move |_| drop(file),
    ) else {
        return;
    };
    let prepared = Prepared {
        path,
        revision: expected,
        started: Instant::now(),
        id,
    };
    *PREPARED.lock().unwrap_or_else(|e| e.into_inner()) = Some(prepared);
    // An unused prepared listing must not retain an entry vector indefinitely.
    if std::thread::Builder::new()
        .name("startup-listing-expiry".into())
        .spawn(move || expire(id, MAX_AGE))
        .is_err()
    {
        discard_unclaimed();
    }
}

fn expire(id: u64, age: Duration) {
    let state = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    let (mut state, _) = CLAIMED
        .wait_timeout_while(state, age, |state| {
            state.as_ref().is_some_and(|prepared| prepared.id == id)
        })
        .unwrap_or_else(|e| e.into_inner());
    let prepared = state.take_if(|prepared| prepared.id == id);
    drop(state);
    if let Some(prepared) = prepared {
        crate::jobs::discard_prepared_listing(prepared.id);
    }
}

pub(crate) fn discard_unclaimed() {
    let prepared = PREPARED.lock().unwrap_or_else(|e| e.into_inner()).take();
    CLAIMED.notify_all();
    if let Some(prepared) = prepared {
        crate::jobs::discard_prepared_listing(prepared.id);
    }
}

fn claim(path: &str) -> Option<u64> {
    let mut state = PREPARED.lock().unwrap_or_else(|e| e.into_inner());
    let prepared = state.take_if(|prepared| prepared.path == path)?;
    drop(state);
    CLAIMED.notify_all();
    if prepared.started.elapsed() <= MAX_AGE
        && pin_archive_for_listing(Path::new(path))
            .ok()
            .and_then(|file| revision(&file))
            .is_some_and(|revision| revision == prepared.revision)
    {
        return Some(prepared.id);
    }
    crate::jobs::discard_prepared_listing(prepared.id);
    None
}

#[tauri::command]
pub fn start_queued_archive_listing(path: String) -> Result<u64, String> {
    if let Some(id) = claim(&path) {
        // The bounded job registry can evict a completed speculative job.
        if crate::jobs::operation_status(id).is_ok() {
            return Ok(id);
        }
    }
    crate::jobs::start_operation(listing_request(path), Some(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_listing_claims_only_the_same_revision_once() {
        let _registry = crate::jobs::test_registry_guard();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.zip");
        let mut empty_zip = [0; 22];
        empty_zip[..4].copy_from_slice(b"PK\x05\x06");
        std::fs::write(&path, empty_zip).unwrap();
        let path =
            crate::instance::canonical_archive_path(path.to_string_lossy().into_owned()).unwrap();

        prepare(path.clone());
        assert_eq!(claim("C:\\unrelated.zip"), None);
        let id = claim(&path).expect("the first queued archive should claim its own listing");
        assert_eq!(claim(&path), None);
        wait_for_listing(id);
        crate::jobs::forget_operation(id).unwrap();

        prepare(path.clone());
        let stale = PREPARED.lock().unwrap().as_ref().unwrap().id;
        wait_for_listing(stale);
        let replaced = dir.path().join("old.zip");
        std::fs::rename(&path, replaced).unwrap();
        std::fs::write(&path, empty_zip).unwrap();
        assert_eq!(claim(&path), None);
        assert!(crate::jobs::operation_status(stale).is_err());

        let busy = File::options().write(true).open(&path).unwrap();
        prepare(path.clone());
        assert!(PREPARED.lock().unwrap().is_none());
        drop(busy);

        prepare(path.clone());
        let unclaimed = PREPARED.lock().unwrap().as_ref().unwrap().id;
        expire(unclaimed, Duration::ZERO);
        CLAIMED.notify_all();
        assert_eq!(claim(&path), None);
        assert!(crate::jobs::operation_status(unclaimed).is_err());
    }

    #[test]
    fn cold_zip_listing_accepts_large_payload_with_small_directory() {
        let _registry = crate::jobs::test_registry_guard();
        let scratch = tempfile::tempdir().unwrap();
        let payload = scratch.path().join("ordinary.bin");
        let payload_bytes = 33 * 1024 * 1024;
        File::create(&payload)
            .unwrap()
            .set_len(payload_bytes)
            .unwrap();
        let archive = scratch.path().join("ordinary.zip");
        let options = obz_core::engine::EngineOptions {
            method: Some("store".into()),
            ..Default::default()
        };
        obz_core::engine::create(
            archive.to_str().unwrap(),
            &[payload.to_str().unwrap().to_owned()],
            "zip",
            &options,
        )
        .unwrap();
        assert!(archive.metadata().unwrap().len() > payload_bytes);
        let path =
            crate::instance::canonical_archive_path(archive.to_str().unwrap().to_owned()).unwrap();

        prepare(path.clone());
        let id = claim(&path).expect("a small directory should prefetch despite a large ZIP payload");
        wait_for_listing(id);
        crate::jobs::forget_operation(id).unwrap();
    }

    fn wait_for_listing(id: u64) {
        tauri::async_runtime::block_on(async {
            loop {
                let snapshot = crate::jobs::wait_operation(id, Some(true)).await.unwrap();
                let snapshot = serde_json::to_value(snapshot).unwrap();
                match snapshot["status"].as_str() {
                    Some("running") => continue,
                    Some("complete") => break,
                    _ => panic!("{snapshot:?}"),
                }
            }
        })
    }
}
