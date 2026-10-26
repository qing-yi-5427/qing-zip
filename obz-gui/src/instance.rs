//! Queue archive arguments before notifying the WebView, including startup races.
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, Manager};
static PENDING_FOLDERS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
#[tauri::command]
pub fn canonical_archive_path(path: String) -> Result<String, String> {
    if path.is_empty()
        || path.contains('\0')
        || path.starts_with(r"\\.\")
        || path.starts_with(r"\\?\")
    {
        return Err("无效的归档路径".into());
    }
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    use std::os::windows::fs::MetadataExt;
    for parent in path.ancestors() {
        let meta = std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("归档路径不能包含链接或重解析点".into());
        }
    }
    if !path.is_file() {
        return Err("请选择普通压缩文件".into());
    }
    let path = path
        .canonicalize()
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        Ok(format!(r"\\{unc}"))
    } else {
        Ok(path.strip_prefix(r"\\?\").unwrap_or(&path).to_string())
    }
}
fn paths(args: impl IntoIterator<Item = String>, cwd: &Path, folders: bool) -> Vec<String> {
    let mut result = Vec::new();
    for arg in args.into_iter().skip(1).take(1024) {
        if arg.is_empty()
            || arg.starts_with('-')
            || arg.contains(['\0', '\r', '\n'])
            || arg.contains("://")
            || arg.starts_with(r"\\.\")
            || arg.starts_with(r"\\?\")
        {
            continue;
        }
        let path = PathBuf::from(&arg);
        let path = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        let Ok(path) = std::path::absolute(path) else {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        use std::os::windows::fs::MetadataExt;
        if (if folders {!meta.is_dir() || path.file_name().is_none()} else {!meta.is_file()}) || meta.file_attributes() & 0x400 != 0 {
            continue;
        }
        if folders && path.ancestors().any(|p|std::fs::symlink_metadata(p).map(|m|m.file_attributes() & 0x400 != 0).unwrap_or(true)){continue;}
        let path = path.to_string_lossy().into_owned();
        if !result
            .iter()
            .any(|p: &String| p.eq_ignore_ascii_case(&path))
        {
            result.push(path);
        }
    }
    result
}
pub fn startup_paths() -> Vec<String> {
    if std::env::args().nth(1).as_deref()==Some("--compress-folder"){return Vec::new();}
    paths(
        std::env::args(),
        &std::env::current_dir().unwrap_or_default(), false,
    )
}
fn enqueue(paths: &[String]) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    for path in paths {
        if !pending.iter().any(|p| p.eq_ignore_ascii_case(path)) {
            pending.push(path.clone());
        }
    }
}
pub fn queue_startup() {
    let args:Vec<String>=std::env::args().collect();
    if args.get(1).map(String::as_str)==Some("--compress-folder"){queue_folders(paths(args,&std::env::current_dir().unwrap_or_default(),true));}else{enqueue(&startup_paths());}
}
pub fn receive(app: &tauri::AppHandle, args: Vec<String>, cwd: String) {
    let folders=args.get(1).map(String::as_str)==Some("--compress-folder");
    let paths = paths(args, Path::new(&cwd),folders);
    if folders {queue_folders(paths.clone());}else{enqueue(&paths);}
    // The event is a wakeup; the frontend drains the queue once after listener setup.
    let _ = app.emit("open-archives", paths);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
#[tauri::command]
pub fn take_pending_archives() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
}
fn queue_folders(paths:Vec<String>){
    let mut pending=PENDING_FOLDERS.lock().unwrap_or_else(|e|e.into_inner());
    for path in paths {if pending.len()<128 && !pending.iter().any(|p|p.eq_ignore_ascii_case(&path)){pending.push(path);}}
}
#[tauri::command]
pub fn take_pending_folders()->Vec<String>{std::mem::take(&mut *PENDING_FOLDERS.lock().unwrap_or_else(|e|e.into_inner()))}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compression_arguments_accept_folders_only(){
        let root=tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("中文 folder")).unwrap();
        std::fs::write(root.path().join("file.zip"),b"data").unwrap();
        let found=paths(["app.exe","--compress-folder","中文 folder","file.zip","missing"].map(str::to_string),root.path(),true);
        assert_eq!(found.len(),1);assert!(found[0].ends_with("中文 folder"));
    }
    #[test]
    fn arguments_only_select_existing_regular_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.zip"), b"test").unwrap();
        let result = paths(
            [
                "app.exe",
                "a.zip",
                "A.zip",
                "--help",
                "https://example.invalid/x.zip",
                "missing.zip",
                ".",
            ]
            .map(str::to_string),
            root.path(), false,
        );
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("a.zip"));
    }
    #[test]
    fn pending_arguments_survive_until_drained_once() {
        let _ = take_pending_archives();
        enqueue(&["C:\\one.zip".into()]);
        enqueue(&["C:\\ONE.zip".into(), "C:\\two.7z".into()]);
        assert_eq!(take_pending_archives().len(), 2);
        assert!(take_pending_archives().is_empty());
    }
    #[test]
    fn canonical_paths_normalize_aliases_but_reject_nonfiles() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("sample.zip");
        std::fs::write(&file, b"test").unwrap();
        let alias = root.path().join(".").join("sample.zip");
        assert_eq!(
            canonical_archive_path(file.to_string_lossy().into_owned()).unwrap(),
            canonical_archive_path(alias.to_string_lossy().into_owned()).unwrap()
        );
        assert!(canonical_archive_path(root.path().to_string_lossy().into_owned()).is_err());
        assert!(canonical_archive_path("https://example.invalid/archive.zip".into()).is_err());
    }
}
