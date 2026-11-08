//! Queue archive arguments before notifying the WebView, including startup races.
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{Emitter, Manager};
static PENDING_FOLDERS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PENDING_FILE_GROUPS: Mutex<Vec<Result<Vec<String>, String>>> = Mutex::new(Vec::new());
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static PENDING_EXTRACTIONS: Mutex<Vec<ShellExtraction>> = Mutex::new(Vec::new());
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ShellExtraction {
    path: String,
    dest: String,
    smart: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum StartupMode { Open, CompressFolders, CompressFiles, SmartExtract, ExtractHere }
fn startup_mode(args: &[String]) -> StartupMode {
    match args.get(1).map(String::as_str) {
        Some("--compress-folder") => StartupMode::CompressFolders,
        Some("--compress-files") => StartupMode::CompressFiles,
        Some("--smart-extract") => StartupMode::SmartExtract,
        Some("--extract-here") => StartupMode::ExtractHere,
        _ => StartupMode::Open,
    }
}
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
    if startup_mode(&std::env::args().collect::<Vec<_>>()) != StartupMode::Open {return Vec::new();}
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
    queue_arguments(args, &std::env::current_dir().unwrap_or_default());
}
pub fn receive(app: &tauri::AppHandle, args: Vec<String>, cwd: String) {
    queue_arguments(args, Path::new(&cwd));
    // The event is a wakeup; the frontend drains the queue once after listener setup.
    let _ = app.emit("open-archives", ());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
fn extraction_requests(paths: Vec<String>, smart: bool) -> Vec<ShellExtraction> {
    paths.into_iter().take(128).filter_map(|path| {
        // Reuse the same canonicalization and ancestor reparse checks as tabs.
        let path = canonical_archive_path(path).ok()?;
        let dest = Path::new(&path).parent()?.to_string_lossy().into_owned();
        Some(ShellExtraction { path, dest, smart })
    }).collect()
}
fn queue_arguments(args: Vec<String>, cwd: &Path) {
    let mode = startup_mode(&args);
    match mode {
        StartupMode::Open => enqueue(&paths(args, cwd, false)),
        StartupMode::CompressFolders => queue_folders(paths(args, cwd, true)),
        StartupMode::CompressFiles => {
            let selection = compression_files(&args[2..], cwd);
            let mut pending = PENDING_FILE_GROUPS.lock().unwrap_or_else(|e| e.into_inner());
            if pending.len() < 128 {
                pending.push(selection);
            } else if pending.len() == 128 {
                pending.push(Err("待处理的压缩任务已满，请稍后重试".into()));
            }
        }
        StartupMode::SmartExtract | StartupMode::ExtractHere => {
            let paths = paths(args, cwd, false);
            let mut pending = PENDING_EXTRACTIONS.lock().unwrap_or_else(|e| e.into_inner());
            for request in extraction_requests(paths, mode == StartupMode::SmartExtract) {
                if pending.len() < 128 && !pending.iter().any(|p| p.smart == request.smart && p.path.eq_ignore_ascii_case(&request.path)) {
                    pending.push(request);
                }
            }
        }
    }
}
fn compression_files(args: &[String], cwd: &Path) -> Result<Vec<String>, String> {
    if !(1..=128).contains(&args.len()) {
        return Err("每次可选择 1 至 128 个文件压缩".into());
    }
    let mut files = Vec::new();
    for name in args {
        if name.is_empty() || name.contains(['\0', '\r', '\n']) || name.contains("://") {
            return Err("压缩来源包含无效文件路径".into());
        }
        let path = cwd.join(name);
        let path = canonical_archive_path(path.to_str().ok_or("无效的文件路径")?.to_owned())?;
        if !files.contains(&path) { files.push(path); }
    }
    Ok(files)
}
#[tauri::command]
pub fn take_pending_compression_files() -> Vec<Result<Vec<String>, String>> {
    std::mem::take(&mut *PENDING_FILE_GROUPS.lock().unwrap_or_else(|e| e.into_inner()))
}
#[tauri::command]
pub fn take_pending_shell_extractions() -> Vec<ShellExtraction> {
    std::mem::take(&mut *PENDING_EXTRACTIONS.lock().unwrap_or_else(|e| e.into_inner()))
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
    fn extraction_switches_do_not_open_tabs_or_compress_folders() {
        for (arg, expected) in [("--smart-extract", StartupMode::SmartExtract), ("--extract-here", StartupMode::ExtractHere), ("--compress-folder", StartupMode::CompressFolders)] {
            assert_eq!(startup_mode(&["app.exe".into(), arg.into(), "a.zip".into()]), expected);
        }
        assert_eq!(startup_mode(&["app.exe".into(), "a.zip".into()]), StartupMode::Open);
    }
    #[test]
    fn shell_extraction_queue_retains_mode_destination_and_deduplicates() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("archive.zip");
        std::fs::write(&file, b"data").unwrap();
        let _ = take_pending_shell_extractions();
        for switch in ["--smart-extract", "--smart-extract", "--extract-here"] {
            queue_arguments(["app.exe", switch, "archive.zip", "missing.zip", "."].map(str::to_string).to_vec(), root.path());
        }
        let pending = take_pending_shell_extractions();
        assert_eq!(pending.len(), 2);
        assert!(pending[0].smart);
        assert!(!pending[1].smart);
        assert_eq!(pending[0].path, canonical_archive_path(file.to_string_lossy().into_owned()).unwrap());
        assert_eq!(Path::new(&pending[0].path).parent().unwrap(), Path::new(&pending[0].dest));
        assert!(take_pending_shell_extractions().is_empty());
        assert!(take_pending_folders().is_empty());
    }
    #[test]
    fn extraction_requests_reject_directories_and_missing_paths() {
        let root = tempfile::tempdir().unwrap();
        assert!(extraction_requests(vec![root.path().display().to_string(), root.path().join("missing.zip").display().to_string()], true).is_empty());
    }
    #[test]
    fn compression_arguments_accept_folders_only(){
        let root=tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("中文 folder")).unwrap();
        std::fs::write(root.path().join("file.zip"),b"data").unwrap();
        let found=paths(["app.exe","--compress-folder","中文 folder","file.zip","missing"].map(str::to_string),root.path(),true);
        assert_eq!(found.len(),1);assert!(found[0].ends_with("中文 folder"));
    }
    #[test]
    fn file_compression_requests_keep_each_selection_together() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["one.txt", "文件 二.txt"] { std::fs::write(temp.path().join(name), b"x").unwrap(); }
        let submit = |items: &[&str]| {
            let args = ["qing-zip.exe", "--compress-files"].into_iter()
                .chain(items.iter().copied()).map(str::to_string).collect();
            queue_arguments(args, temp.path());
        };
        submit(&["one.txt", "文件 二.txt", "one.txt"]);
        submit(&["one.txt", "missing.txt"]);
        submit(&["文件 二.txt"]);
        let groups = take_pending_compression_files();
        assert_eq!(groups.len(), 3);
        let expected: Vec<_> = ["one.txt", "文件 二.txt"].iter().map(|name|
            canonical_archive_path(temp.path().join(name).display().to_string()).unwrap()).collect();
        assert_eq!(groups[0].as_ref().unwrap(), &expected);
        assert!(groups[1].is_err());
        assert_eq!(groups[2].as_ref().unwrap(), &expected[1..]);
        assert!(take_pending_compression_files().is_empty());
        assert!(compression_files(&vec!["one.txt".into();129], temp.path()).is_err());
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
