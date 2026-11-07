//! Copy-only native Explorer drag. Sources are retained for asynchronous shell consumers.
use windows::{core::{implement, BOOL, HRESULT, PCWSTR}, Win32::{
    Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK},
    System::{Com::{CoTaskMemFree, IDataObject}, Ole::{DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, OleUninitialize, DROPEFFECT, DROPEFFECT_COPY}, SystemServices::{MODIFIERKEYS_FLAGS, MK_LBUTTON}},
    UI::{WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON}, Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON, VK_RBUTTON}, Shell::{SHParseDisplayName, SHCreateShellItemArrayFromIDLists, BHID_DataObject, Common::ITEMIDLIST}},
}};
#[implement(IDropSource)]
struct Source;
impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() { DRAGDROP_S_CANCEL } else if keys.0 & MK_LBUTTON.0 == 0 { DRAGDROP_S_DROP } else { S_OK }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
}
struct Ole;
impl Drop for Ole {fn drop(&mut self){unsafe{OleUninitialize()}}}
struct Pidls(Vec<*mut ITEMIDLIST>);
impl Drop for Pidls {fn drop(&mut self){for p in &self.0 {unsafe{CoTaskMemFree(Some((*p).cast()))}}}}
fn data_object(paths:Vec<String>) -> Result<IDataObject,String> {
    unsafe {
        let mut pidls=Pidls(Vec::new());
        for path in paths {
            let wide:Vec<u16>=path.encode_utf16().chain(Some(0)).collect();let mut pidl=std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(wide.as_ptr()),None,&mut pidl,0,None).map_err(|e|e.to_string())?;
            pidls.0.push(pidl);
        }
        let pointers:Vec<*const ITEMIDLIST>=pidls.0.iter().map(|p|*p as *const _).collect();
        let items=SHCreateShellItemArrayFromIDLists(&pointers).map_err(|e|e.to_string())?;
        items.BindToHandler(None,&BHID_DataObject).map_err(|e|e.to_string())
    }
}
use obz_core::{
    engine::{JobControl, ShellDragDirectory},
    operations::Request,
};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU8, Ordering},
    Arc, Mutex,
};

const PREPARING: u8 = 0;
const CANCELLED: u8 = 1;
const SHELL: u8 = 2;
const CANCEL_MESSAGE: &str = "已取消拖放";
struct Preparation {
    token: String,
    control: JobControl,
    phase: AtomicU8,
    ready: AtomicBool,
    directory: ShellDragDirectory,
    paths: Vec<String>,
}
static ACTIVE: Mutex<Option<Arc<Preparation>>> = Mutex::new(None);

impl Preparation {
    fn cancel(&self) {
        if self
            .phase
            .compare_exchange(PREPARING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.control.cancel();
        }
    }

    fn handoff(&self, held: bool) -> Result<bool, String> {
        if !held || self.control.cancelled.load(Ordering::Acquire) {
            self.cancel();
        }
        if !self.ready.load(Ordering::Acquire) {
            return Err("拖出文件尚未准备完成".into());
        }
        if self
            .phase
            .compare_exchange(PREPARING, SHELL, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(false);
        }
        // A shell extension may keep the data object even after DoDragDrop returns.
        self.directory.retain_for_shell();
        Ok(true)
    }
}
fn active(token: &str) -> Option<Arc<Preparation>> {
    ACTIVE
        .lock()
        .ok()?
        .as_ref()
        .filter(|p| p.token == token)
        .cloned()
}
fn remove(token: &str) {
    if let Some(p) = active(token) {
        p.cancel();
        unregister(&p);
    }
}
fn unregister(preparation: &Arc<Preparation>) {
    let removed = ACTIVE.lock().ok().and_then(|mut current| {
        if current.as_ref().is_some_and(|p| Arc::ptr_eq(p, preparation)) {
            current.take()
        } else {
            None
        }
    });
    drop(removed);
}
fn button_held() -> bool {
    unsafe {
        let button = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            VK_RBUTTON
        } else {
            VK_LBUTTON
        };
        GetAsyncKeyState(button.0 as i32) < 0 && GetAsyncKeyState(VK_ESCAPE.0 as i32) >= 0
    }
}
fn monitor(preparation: std::sync::Weak<Preparation>, held: impl Fn() -> bool) {
    while let Some(p) = preparation.upgrade() {
        match p.phase.load(Ordering::Acquire) {
            CANCELLED => {
                unregister(&p);
                break;
            }
            SHELL => break,
            _ => (),
        }
        if !held() {
            p.cancel();
            unregister(&p);
            break;
        }
        drop(p);
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

#[tauri::command]
pub fn start_drag_preparation(token: String, request: Request) -> Result<u64, String> {
    start_with_input(token, request, button_held)
}
fn start_with_input(
    token: String,
    mut request: Request,
    held: impl Fn() -> bool + Send + 'static,
) -> Result<u64, String> {
    if token.len() != 36
        || !token.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        || request.operation != "extract"
        || request.options.selected.is_empty()
        || request.options.selected.len() > 512
    {
        return Err("拖出文件选择无效".into());
    }
    if !held() {
        return Err(CANCEL_MESSAGE.into());
    }
    let relative = request
        .options
        .selected
        .iter()
        .map(|name| obz_core::engine::validate_name(name))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    let directory = ShellDragDirectory::create()?;
    request.dest = directory
        .path()
        .to_str()
        .ok_or("临时目录编码无效")?
        .to_owned();
    request.smart = false;
    request.options.overwrite = "error".into();
    let paths = relative
        .iter()
        .map(|name| directory.path().join(name).to_string_lossy().into_owned())
        .collect();
    let p = Arc::new(Preparation {
        token,
        control: JobControl::new(),
        phase: AtomicU8::new(PREPARING),
        ready: AtomicBool::new(false),
        directory,
        paths,
    });
    {
        let mut current = ACTIVE.lock().map_err(|_| "拖放不可用")?;
        if current.is_some() {
            return Err("已有拖放操作".into());
        }
        *current = Some(p.clone());
    }
    let weak = Arc::downgrade(&p);
    if let Err(error) = std::thread::Builder::new()
        .name("drag-input".into())
        .spawn(move || monitor(weak, held))
    {
        remove(&p.token);
        return Err(error.to_string());
    }
    let done = p.clone();
    let result = crate::jobs::start_controlled(request, false, p.control.clone(), move |success| {
        done.ready.store(success, Ordering::Release);
        if !success {
            unregister(&done);
        }
    });
    if result.is_err() {
        remove(&p.token);
    }
    result
}

#[tauri::command]
pub fn cancel_drag_preparation(token: String) {
    if let Some(p) = active(&token) {
        p.cancel();
    }
}
#[tauri::command]
pub async fn discard_drag(token: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || remove(&token))
        .await
        .map_err(|e| e.to_string())
}
fn native_drag(p: &Preparation) -> Result<bool, String> {
    if !button_held() || p.phase.load(Ordering::Acquire) != PREPARING {
        return Ok(false);
    }
    let _pin = p.directory.pin_for_read()?;
    let paths = crate::desktop::checked_paths(p.paths.clone())?;
    unsafe {
        OleInitialize(None).map_err(|e| e.to_string())?;
        let _ole = Ole;
        let data = data_object(paths)?;
        let source: IDropSource = Source.into();
        let mut effect = DROPEFFECT(0);
        if !p.handoff(button_held())? {
            return Ok(false);
        }
        let hr = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
        hr.ok().map_err(|e| e.to_string())?;
        Ok(hr == DRAGDROP_S_DROP && effect.0 & DROPEFFECT_COPY.0 != 0)
    }
}
#[tauri::command]
pub async fn finish_drag(app: tauri::AppHandle, token: String) -> Result<bool, String> {
    let Some(p) = active(&token) else {
        return Ok(false);
    };
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(native_drag(&p));
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait_job(id: u64) -> serde_json::Value {
        let began = std::time::Instant::now();
        loop {
            let snapshot = serde_json::to_value(crate::jobs::operation_status(id).unwrap()).unwrap();
            if snapshot["status"] != "running" {
                return snapshot;
            }
            assert!(began.elapsed() < std::time::Duration::from_secs(15));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn preparation_cancels_its_own_job_and_keeps_shell_exports() {
        use std::{fs, sync::atomic::AtomicBool};
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("items.zip");
        let file = temp.path().join("data.txt");
        fs::write(&file, b"selected from archive").unwrap();
        obz_core::zipfmt::create_zip(
            archive.to_str().unwrap(),
            &[file.display().to_string()],
            true,
        )
        .unwrap();
        let request = || Request {
            operation: "extract".into(),
            path: archive.display().to_string(),
            options: obz_core::engine::EngineOptions {
                selected: vec!["data.txt".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let token = "12345678-0000-4000-8000-123456789012";
        let held = Arc::new(AtomicBool::new(true));
        let input = held.clone();
        let first = start_with_input(token.into(), request(), move || {
            input.load(Ordering::Acquire)
        })
        .unwrap();
        let p = active(token).unwrap();
        let exported = PathBuf::from(&p.paths[0]);
        cancel_drag_preparation("unrelated-gesture".into());
        assert!(!p.control.cancelled.load(Ordering::Acquire));
        let snapshot = wait_job(first);
        assert_eq!(snapshot["status"], "complete", "{snapshot}");
        assert_eq!(fs::read(&exported).unwrap(), b"selected from archive");
        // Release on the desktop after extraction but before OLE handoff.
        held.store(false, Ordering::Release);
        let start = std::time::Instant::now();
        while !p.control.cancelled.load(Ordering::Acquire) {
            assert!(start.elapsed() < std::time::Duration::from_secs(3));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!p.handoff(true).unwrap());
        drop(p);
        let folder = exported.parent().unwrap();
        while folder.exists() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(3),
                "abandoned export was not removed"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        crate::jobs::forget_operation(first).unwrap();

        let second = start_with_input(token.into(), request(), || true).unwrap();
        let p = active(token).unwrap();
        assert_eq!(wait_job(second)["status"], "complete");
        let handed = PathBuf::from(&p.paths[0]);
        assert!(p.handoff(true).unwrap());
        cancel_drag_preparation(token.into());
        assert!(!p.control.cancelled.load(Ordering::Acquire));
        remove(token);
        drop(p);
        assert_eq!(fs::read(&handed).unwrap(), b"selected from archive");
        fs::remove_file(&handed).unwrap();
        fs::remove_dir(handed.parent().unwrap()).unwrap();
        crate::jobs::forget_operation(second).unwrap();

        // Substitute only the physical input sample. The job registry,
        // actual archive engine, cancellation flag and cleanup all run normally.
        let large = temp.path().join("large.bin");
        fs::File::create(&large).unwrap().set_len(64 * 1024 * 1024).unwrap();
        let big_zip = temp.path().join("large.zip");
        let nana = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..").join(obz_core::engine::BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
        let created = std::process::Command::new(nana).current_dir(temp.path())
            .args(["a", "-tzip", "-mx1"]).arg(&big_zip).arg("large.bin").output().unwrap();
        assert!(created.status.success());
        let mut large_request = request();
        large_request.path = big_zip.display().to_string();
        large_request.options.selected = vec!["large.bin".into()];
        let other_control = JobControl::new();
        let (entered, started) = std::sync::mpsc::channel();
        let (release, held_job) = std::sync::mpsc::channel();
        let other = crate::jobs::start_controlled(Request {
            operation: "test".into(), path: archive.display().to_string(), ..Default::default()
        }, false, other_control.clone(), move |ok| {
            entered.send(ok).unwrap();
            let _ = held_job.recv_timeout(std::time::Duration::from_secs(20));
        }).unwrap();
        assert!(started.recv_timeout(std::time::Duration::from_secs(15)).unwrap());
        let first_sample = AtomicBool::new(true);
        let observed = Arc::new(Mutex::new(None));
        let recording = observed.clone();
        let interrupted = start_with_input(token.into(), large_request, move || {
            if first_sample.swap(false, Ordering::AcqRel) {
                return true;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while let Some(p) = active(token) {
                let phase = p.control.snapshot().phase;
                if phase == "decoding" {
                    *recording.lock().unwrap() = Some(p.directory.path().to_owned());
                    return false;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::yield_now();
            }
            false
        })
        .unwrap();
        assert_eq!(wait_job(interrupted)["status"], "cancelled");
        let abandoned = observed
            .lock()
            .unwrap()
            .take()
            .expect("release during real archive work");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while abandoned.exists() {
            assert!(std::time::Instant::now() < deadline, "cancelled export still exists");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!other_control.cancelled.load(Ordering::Acquire));
        release.send(()).unwrap();
        assert_eq!(wait_job(other)["status"], "complete");
        crate::jobs::forget_operation(other).unwrap();
        crate::jobs::forget_operation(interrupted).unwrap();

        let mut bad = request();
        bad.path = temp.path().join("missing.7z").display().to_string();
        let failed = start_with_input(token.into(), bad, || true).unwrap();
        assert_eq!(wait_job(failed)["status"], "failed");
        assert!(active(token).is_none());
        crate::jobs::forget_operation(failed).unwrap();
        assert!(start_with_input(token.into(), request(), || false).is_err());
    }

    #[test]
    fn abandoned_export_only_removes_its_private_tree_including_readonly_children() {
        use std::fs;
        let other = PathBuf::from(crate::desktop::clipboard_directory().unwrap());
        let other_file = other.join("clipboard-example.txt");
        fs::write(&other_file, b"not a drag output").unwrap();
        let export = ShellDragDirectory::create().unwrap();
        let path = export.path().to_owned();
        let alternate = path.with_file_name(format!("{}.moved", path.file_name().unwrap().to_string_lossy()));
        assert!(fs::rename(&path, alternate).is_err());
        let junction = path.join("external");
        let link = std::process::Command::new("cmd.exe").args(["/D", "/C", "mklink", "/J"])
            .arg(&junction).arg(&other).output().unwrap();
        assert!(link.status.success());
        fs::create_dir(path.join("nested")).unwrap();
        let leaf = path.join("nested/readonly.txt");
        fs::write(&leaf, b"partial output").unwrap();
        let mut perms = fs::metadata(&leaf).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&leaf, perms).unwrap();
        drop(export);
        assert!(!path.exists());
        assert_eq!(fs::read(&other_file).unwrap(), b"not a drag output");
        fs::remove_file(other_file).unwrap();
        fs::remove_dir(other).unwrap();
    }

    #[test] fn drop_source_requires_held_button_and_honors_escape(){
        let source:IDropSource=Source.into();unsafe {
            assert_eq!(source.QueryContinueDrag(true, MK_LBUTTON),DRAGDROP_S_CANCEL);
            assert_eq!(source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(0)),DRAGDROP_S_DROP);
            assert_eq!(source.QueryContinueDrag(false, MK_LBUTTON),S_OK);
            assert_eq!(source.GiveFeedback(DROPEFFECT_COPY),DRAGDROP_S_USEDEFAULTCURSORS);
        }
    }
    #[test] fn shell_drag_payload_contains_unicode_files_and_directories(){
        use windows::Win32::{System::{Com::{FORMATETC,DVASPECT_CONTENT,TYMED_HGLOBAL},Ole::ReleaseStgMedium},UI::Shell::{DragQueryFileW,HDROP}};
        let temp=tempfile::tempdir().unwrap();let file=temp.path().join("中文 file.txt");let folder=temp.path().join("folder");
        std::fs::write(&file,b"unchanged").unwrap();std::fs::create_dir(&folder).unwrap();
        let paths=crate::desktop::checked_paths(vec![file.to_string_lossy().into_owned(),folder.to_string_lossy().into_owned()]).unwrap();
        unsafe {
            OleInitialize(None).unwrap();let _ole=Ole;
            let data=data_object(paths.clone()).unwrap();
            let format=FORMATETC{cfFormat:15,ptd:std::ptr::null_mut(),dwAspect:DVASPECT_CONTENT.0,lindex:-1,tymed:TYMED_HGLOBAL.0 as u32};
            let mut medium=data.GetData(&format).unwrap();let drop=HDROP(medium.u.hGlobal.0);
            let count=DragQueryFileW(drop,u32::MAX,None);let mut actual=Vec::new();
            for i in 0..count {let len=DragQueryFileW(drop,i,None);let mut buf=vec![0u16;len as usize+1];DragQueryFileW(drop,i,Some(&mut buf));actual.push(String::from_utf16_lossy(&buf[..len as usize]));}
            ReleaseStgMedium(&mut medium);
            // Shell PIDLs may expand DOS 8.3 aliases (for example RUNNER~1 in
            // CI's TEMP). Require the same ordered filesystem objects while
            // retaining the exact Unicode filenames and source-content check.
            let canonical = |values: &[String]| values.iter()
                .map(|path| std::fs::canonicalize(path).unwrap()).collect::<Vec<_>>();
            assert_eq!(canonical(&actual),canonical(&paths));
            assert_eq!(std::fs::read(file).unwrap(),b"unchanged");
        }
    }

}
