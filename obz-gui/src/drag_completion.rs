use obz_core::engine::ShellDragDirectory;
use std::{
    sync::{mpsc, Arc, OnceLock},
    time::Duration,
};
use windows::{
    core::{AgileReference, Interface},
    Win32::{
        System::{
            Com::{
                CoInitializeEx, CoUninitialize, IDataObject, COINIT_MULTITHREADED,
                DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::{ReleaseStgMedium, DROPEFFECT_COPY},
        },
        UI::Shell::{IDataObjectAsyncCapability, CFSTR_PERFORMEDDROPEFFECT},
    },
};

/// Preserve the Shell's native data object and its other COM interfaces.
pub(super) fn enable(data: &IDataObject) -> Option<IDataObjectAsyncCapability> {
    let capability = data.cast::<IDataObjectAsyncCapability>().ok()?;
    unsafe {
        capability.SetAsyncMode(true).ok()?;
    }
    Some(capability)
}

// This format is written by a cooperating drop target to report the operation
// it actually performed. A returned cursor effect is only a proposed operation.
fn copy_reported(data: &IDataObject) -> bool {
    unsafe {
        let id = RegisterClipboardFormatW(CFSTR_PERFORMEDDROPEFFECT);
        if id == 0 {
            return false;
        }
        let format = FORMATETC {
            cfFormat: id as u16,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        if data.QueryGetData(&format) != windows::Win32::Foundation::S_OK {
            return false;
        }
        let Ok(mut medium) = data.GetData(&format) else {
            return false;
        };
        let mut copied = false;
        if medium.tymed == TYMED_HGLOBAL.0 as u32 {
            let handle = medium.u.hGlobal;
            if !handle.is_invalid() && GlobalSize(handle) >= std::mem::size_of::<u32>() {
                let ptr = GlobalLock(handle).cast::<u32>();
                if !ptr.is_null() {
                    copied = ptr.read_unaligned() == DROPEFFECT_COPY.0;
                    let _ = GlobalUnlock(handle);
                }
            }
        }
        ReleaseStgMedium(&mut medium);
        copied
    }
}

struct Pending {
    directory: Arc<ShellDragDirectory>,
    operation: AgileReference<IDataObjectAsyncCapability>,
}

// All operations share one worker. It sleeps without polling when idle, and a
// target that never calls EndOperation cannot accumulate a thread per drag.
fn monitor(receiver: mpsc::Receiver<Pending>) {
    if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_err() {
        return;
    }
    let mut active = Vec::new();
    while let Ok(first) = receiver.recv() {
        let mut next = Some(first);
        loop {
            for job in next.take().into_iter().chain(receiver.try_iter()) {
                if let Ok(operation) = job.operation.resolve() {
                    active.push((operation, job.directory));
                }
            }
            active.retain(
                |(operation, directory)| match unsafe { operation.InOperation() } {
                    Ok(state) if state.as_bool() => true,
                    Ok(_) => {
                        directory.shell_read_finished();
                        false
                    }
                    Err(_) => false, // No reliable completion. Retain the files on disk.
                },
            );
            if active.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    drop(active);
    unsafe {
        CoUninitialize();
    }
}

fn watch(job: Pending) {
    static MONITOR: OnceLock<Option<mpsc::Sender<Pending>>> = OnceLock::new();
    if let Some(sender) = MONITOR.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("shell-drop-completion".into())
            .spawn(move || monitor(receiver))
            .ok()
            .map(|_| sender)
    }) {
        let _ = sender.send(job);
    }
}

pub(super) fn after_copy(
    directory: &Arc<ShellDragDirectory>,
    data: &IDataObject,
    capability: Option<&IDataObjectAsyncCapability>,
) {
    let Some(capability) = capability else { return };
    match unsafe { capability.InOperation() } {
        Ok(active) if active.as_bool() => {
            // The target has called StartOperation. A subsequent successful
            // false is its EndOperation, including completion with an error.
            let Ok(reference) = AgileReference::new(capability) else {
                return;
            };
            watch(Pending {
                directory: Arc::clone(directory),
                operation: reference,
            });
        }
        Ok(_) if copy_reported(data) => directory.shell_read_finished(),
        // Some targets keep just the file paths and never acknowledge reading.
        // In that case release all process handles, but retain the private files.
        _ => (),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::drag::{data_object, Ole};
    use std::time::Instant;
    use windows::Win32::{
        Foundation::S_OK,
        System::Ole::OleInitialize,
        UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        },
    };

    fn pump() {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    #[test]
    fn delayed_shell_reader_keeps_source_until_end_operation() {
        unsafe { OleInitialize(None).unwrap() };
        let _ole = Ole;
        let directory = Arc::new(ShellDragDirectory::create().unwrap());
        let path = directory.path().join("late.txt");
        let root = directory.path().to_owned();
        std::fs::write(&path, b"copy after Drop returned").unwrap();
        let data = data_object(vec![path.to_string_lossy().into_owned()]).unwrap();
        let capability = enable(&data).unwrap();
        assert!(!copy_reported(&data));
        unsafe { capability.StartOperation(None).unwrap() };
        directory.retain_for_shell();
        after_copy(&directory, &data, Some(&capability));
        drop(directory);
        let wait = Instant::now();
        while wait.elapsed() < Duration::from_millis(450) {
            pump();
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"copy after Drop returned");
        unsafe {
            capability
                .EndOperation(S_OK, None, DROPEFFECT_COPY.0)
                .unwrap()
        };
        let wait = Instant::now();
        while root.exists() && wait.elapsed() < Duration::from_secs(8) {
            pump();
        }
        assert!(
            !root.exists(),
            "EndOperation must reclaim the pinned export"
        );
    }

    #[test]
    fn synchronous_target_reports_copy_only_after_reading() {
        use windows::Win32::System::{
            Com::{STGMEDIUM, STGMEDIUM_0},
            Memory::{GlobalAlloc, GMEM_MOVEABLE},
        };
        unsafe { OleInitialize(None).unwrap() };
        let _ole = Ole;
        let directory = Arc::new(ShellDragDirectory::create().unwrap());
        let root = directory.path().to_owned();
        let source = root.join("sync.txt");
        std::fs::write(&source, b"synchronous copy").unwrap();
        let data = data_object(vec![source.to_string_lossy().into_owned()]).unwrap();
        let capability = enable(&data).unwrap();
        directory.retain_for_shell();
        assert!(!copy_reported(&data));
        assert_eq!(std::fs::read(&source).unwrap(), b"synchronous copy");
        unsafe {
            let value = GlobalAlloc(GMEM_MOVEABLE, 4).unwrap();
            let p = GlobalLock(value).cast::<u32>();
            assert!(!p.is_null());
            p.write(DROPEFFECT_COPY.0);
            let _ = GlobalUnlock(value);
            let medium = STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: value },
                pUnkForRelease: Default::default(),
            };
            let format = FORMATETC {
                cfFormat: RegisterClipboardFormatW(CFSTR_PERFORMEDDROPEFFECT) as u16,
                ptd: std::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
            };
            data.SetData(&format, &medium, true).unwrap();
        }
        assert!(copy_reported(&data));
        after_copy(&directory, &data, Some(&capability));
        drop(directory);
        assert!(!root.exists());
    }

    #[test]
    fn target_that_never_acknowledges_copy_can_read_after_releasing_com() {
        unsafe { OleInitialize(None).unwrap() };
        let _ole = Ole;
        let directory = Arc::new(ShellDragDirectory::create().unwrap());
        let path = directory.path().join("unreported.txt");
        let root = directory.path().to_owned();
        std::fs::write(&path, b"read retained paths").unwrap();
        let data = data_object(vec![path.to_string_lossy().into_owned()]).unwrap();
        let capability = enable(&data).unwrap();
        assert!(!unsafe { capability.InOperation().unwrap() }.as_bool());
        directory.retain_for_shell();
        after_copy(&directory, &data, Some(&capability));
        drop((capability, data, directory));
        assert_eq!(std::fs::read(&path).unwrap(), b"read retained paths");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
