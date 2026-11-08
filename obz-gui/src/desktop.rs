//! Native desktop integration. Never executes selected file contents.
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Foundation::{HANDLE, HGLOBAL, HWND},
        Storage::FileSystem::{
            GetDriveTypeW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        },
        System::{Com::*, DataExchange::*, Memory::*},
        UI::Shell::*,
    },
};
const MAX_PATHS: usize = 512;
const MAX_CLIPBOARD: usize = 2 * 1024 * 1024;
fn wide(s: &str) -> Result<Vec<u16>, String> {
    if s.contains('\0') {
        return Err("路径包含无效字符".into());
    }
    let v: Vec<_> = s.encode_utf16().chain(Some(0)).collect();
    if v.len() > 32768 {
        return Err("路径过长".into());
    }
    Ok(v)
}
fn no_links(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                use std::os::windows::fs::MetadataExt;
                if m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 {
                    return Err("不支持符号链接或重解析点".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
fn absolute(path: &str) -> Result<PathBuf, String> {
    wide(path)?;
    if path.starts_with(r"\\.\") || path.starts_with(r"\\?\") {
        return Err("不支持设备命名空间路径".into());
    }
    if path.is_empty() {
        return Err("请选择路径".into());
    }
    let p = std::path::absolute(path).map_err(|e| e.to_string())?;
    no_links(&p)?;
    Ok(p)
}
// Preserve the filesystem spelling of distinct names in case-sensitive directories.
pub(crate) fn canonical_spelling(path: &Path, directory: bool) -> Result<String, String> {
    let path = obz_core::engine::canonical_input_path(path, directory).map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidData => "路径不能包含链接或重解析点".to_string(),
        std::io::ErrorKind::InvalidInput if directory => "请选择普通文件夹".to_string(),
        std::io::ErrorKind::InvalidInput => "请选择普通压缩文件".to_string(),
        _ => error.to_string(),
    })?
        .to_string_lossy()
        .into_owned();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        Ok(format!(r"\\{unc}"))
    } else {
        Ok(path.strip_prefix(r"\\?\").unwrap_or(&path).to_string())
    }
}
fn existing(path: &str) -> Result<PathBuf, String> {
    let p = absolute(path)?;
    if !p.exists() {
        return Err("路径不存在".into());
    }
    Ok(p)
}
#[derive(Serialize)]
pub struct KnownFolders {
    pub desktop: String,
    pub documents: String,
    pub pictures: String,
    pub downloads: String,
    pub home: String,
}
fn known(id: &GUID) -> Result<String, String> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None).map_err(|e| e.to_string())?;
        let text = p.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(p.0.cast()));
        text
    }
}
#[tauri::command]
pub fn known_folders() -> Result<KnownFolders, String> {
    Ok(KnownFolders {
        desktop: known(&FOLDERID_Desktop)?,
        documents: known(&FOLDERID_Documents)?,
        pictures: known(&FOLDERID_Pictures)?,
        downloads: known(&FOLDERID_Downloads)?,
        home: known(&FOLDERID_Profile)?,
    })
}
#[tauri::command]
pub fn reveal_path(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let p = existing(&path)?;
    let explorer =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("explorer.exe");
    let mut c = std::process::Command::new(explorer);
    if p.is_file() {
        c.arg("/select,");
    }
    c.arg(p)
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn copy_inner(path: &str, dest: &str) -> Result<String, String> {
    let src = existing(path)?;
    if !src.is_file() {
        return Err("只能复制归档文件".into());
    }
    let mut dst = absolute(dest)?;
    if dst.is_dir() {
        dst = dst.join(src.file_name().ok_or("源文件名无效")?);
    }
    no_links(&dst)?;
    if dst.exists() {
        return Err("目标已存在，未覆盖任何文件".into());
    }
    let parent = dst.parent().ok_or("目标目录无效")?;
    if !parent.is_dir() {
        return Err("目标目录不存在".into());
    }
    no_links(parent)?;
    obz_core::engine::copy_archive_file(&src, &dst)?;
    Ok(dst.to_string_lossy().into_owned())
}
#[tauri::command]
pub async fn copy_archive(path: String, dest: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || copy_inner(&path, &dest))
        .await
        .map_err(|e| e.to_string())?
}
#[derive(Debug, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
}
#[tauri::command]
pub fn list_directory(path: String) -> Result<Vec<DirectoryEntry>, String> {
    let p = existing(&path)?;
    if !p.is_dir() {
        return Err("请选择文件夹".into());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(p).map_err(|e| e.to_string())? {
        let e = entry.map_err(|e| e.to_string())?;
        let m = fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        use std::os::windows::fs::MetadataExt;
        if m.is_dir() && !m.file_type().is_symlink() && m.file_attributes() & 0x400 == 0 {
            out.push(DirectoryEntry {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path().to_string_lossy().into_owned(),
            });
            if out.len() > 10000 {
                return Err("子文件夹数量超过显示限制".into());
            }
        }
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    Ok(out)
}
type ClipboardExport = obz_core::engine::ShellDragDirectory;
static CLIPBOARD_EXPORTS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, ClipboardExport>>,
> = std::sync::LazyLock::new(Default::default);

#[tauri::command]
pub fn clipboard_directory() -> Result<String, String> {
    let directory = ClipboardExport::create()?;
    let path = directory.path().to_str().ok_or("临时目录编码无效")?.to_owned();
    CLIPBOARD_EXPORTS.lock().map_err(|_| "剪贴板导出暂不可用")?
        .insert(path.clone(), directory);
    Ok(path)
}

fn take_clipboard_export(dest: &str) -> Result<Option<ClipboardExport>, String> {
    Ok(CLIPBOARD_EXPORTS.lock().map_err(|_| "剪贴板导出暂不可用")?.remove(dest))
}

#[tauri::command]
pub fn discard_clipboard_export(dest: String) -> Result<(), String> {
    drop(take_clipboard_export(&dest)?);
    Ok(())
}

#[tauri::command]
pub fn publish_clipboard_export(
    window: tauri::Window, dest: String, paths: Vec<String>,
) -> Result<(), String> {
    let export = take_clipboard_export(&dest)?.ok_or("剪贴板导出已结束")?;
    let _pin = export.pin_for_read()?;
    let paths = checked_paths(paths)?;
    let root = canonical_spelling(export.path(), true)?;
    if !paths.iter().all(|p| Path::new(p).starts_with(&root) && p != &root) {
        return Err("所选文件不在本次导出目录中".into());
    }
    let owner = window.hwnd().map_err(|e| e.to_string())?;
    publish_paths(HWND(owner.0), &paths, Some(&export))
}
pub(crate) fn checked_paths(paths: Vec<String>) -> Result<Vec<String>, String> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        return Err("请选择1至512个文件或文件夹".into());
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for p in paths {
        if !Path::new(&p).is_absolute() {
            return Err("文件列表必须使用完整路径".into());
        }
        let path = existing(&p)?;
        let p = canonical_spelling(&path, path.is_dir())?;
        if seen.insert(p.clone()) {
            out.push(p)
        }
    }
    Ok(out)
}
fn drop_payload(paths: &[String]) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; 20];
    out[..4].copy_from_slice(&20u32.to_le_bytes());
    out[16..20].copy_from_slice(&1u32.to_le_bytes());
    for p in paths {
        for unit in wide(p)? {
            out.extend_from_slice(&unit.to_le_bytes())
        }
    }
    out.extend_from_slice(&[0, 0]);
    if out.len() > MAX_CLIPBOARD {
        return Err("剪贴板文件列表过大".into());
    }
    Ok(out)
}
fn validate_drop_payload(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 22 || bytes.len() > MAX_CLIPBOARD {
        return Err("剪贴板文件列表大小无效".into());
    }
    let offset = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let is_wide = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) != 0;
    if offset < 20 || offset >= bytes.len() {
        return Err("剪贴板文件列表偏移无效".into());
    }
    let data = &bytes[offset..];
    if is_wide {
        if offset % 2 != 0
            || data.len() % 2 != 0
            || !data
                .chunks_exact(2)
                .map(|x| u16::from_le_bytes([x[0], x[1]]))
                .collect::<Vec<_>>()
                .windows(2)
                .any(|w| w == [0, 0])
        {
            return Err("剪贴板文件列表未完整终止".into());
        }
    } else if !data.windows(2).any(|w| w == [0, 0]) {
        return Err("剪贴板文件列表未完整终止".into());
    }
    Ok(())
}
struct Clipboard;
impl Clipboard {
    fn open(owner: HWND) -> Result<Self, String> {
        for _ in 0..20 {
            if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
                return Ok(Self);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("剪贴板正在被其他程序使用".into())
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
struct Memory(HGLOBAL);
impl Memory {
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        unsafe {
            let m = Self(GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?);
            let p = GlobalLock(m.0);
            if p.is_null() {
                return Err("无法写入剪贴板内存".into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(m.0);
            Ok(m)
        }
    }
    fn publish(self, format: u32) -> Result<(), String> {
        unsafe {
            SetClipboardData(format, Some(HANDLE(self.0 .0))).map_err(|e| e.to_string())?;
        }
        std::mem::forget(self);
        Ok(())
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::GlobalFree(Some(self.0));
        }
    }
}
#[tauri::command]
pub fn clipboard_paths(
    window: tauri::Window,
    paths: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let owner = window.hwnd().map_err(|e| e.to_string())?;
    clipboard_inner(HWND(owner.0), paths)
}
fn publish_paths(owner: HWND, paths: &[String], export: Option<&ClipboardExport>) -> Result<(), String> {
    let data = Memory::from_bytes(&drop_payload(paths)?)?;
    let effect = Memory::from_bytes(&1u32.to_le_bytes())?;
    let format =
        unsafe { RegisterClipboardFormatW(PCWSTR(wide("Preferred DropEffect")?.as_ptr())) };
    if format == 0 {
        return Err("无法注册剪贴板格式".into());
    }
    let _clip = Clipboard::open(owner)?;
    unsafe {
        EmptyClipboard().map_err(|e| e.to_string())?;
    }
    data.publish(15)?;
    // Once CF_HDROP is visible another process may use these paths after we exit.
    if let Some(export) = export {
        export.retain_for_shell();
    }
    effect.publish(format)?;
    Ok(())
}
fn clipboard_inner(owner: HWND, paths: Option<Vec<String>>) -> Result<Vec<String>, String> {
    if let Some(paths) = paths {
        let paths = checked_paths(paths)?;
        publish_paths(owner, &paths, None)?;
        return Ok(paths);
    }
    let _clip = Clipboard::open(owner)?;
    unsafe {
        if IsClipboardFormatAvailable(15).is_err() {
            return Ok(Vec::new());
        }
        let h = GetClipboardData(15).map_err(|e| e.to_string())?;
        let mem = HGLOBAL(h.0);
        let size = GlobalSize(mem);
        if size > MAX_CLIPBOARD || size < 22 {
            return Err("剪贴板文件列表过大或已损坏".into());
        }
        let ptr = GlobalLock(mem);
        if ptr.is_null() {
            return Err("无法读取剪贴板内存".into());
        }
        let valid = validate_drop_payload(std::slice::from_raw_parts(ptr.cast::<u8>(), size));
        let _ = GlobalUnlock(mem);
        valid?;
        let drop = HDROP(h.0);
        let count = DragQueryFileW(drop, u32::MAX, None) as usize;
        if count > MAX_PATHS {
            return Err("剪贴板文件数量超过512".into());
        }
        let mut out = Vec::new();
        for i in 0..count {
            let len = DragQueryFileW(drop, i as u32, None) as usize;
            if len == 0 || len > 32767 {
                return Err("剪贴板路径长度无效".into());
            }
            let mut text = vec![0u16; len + 1];
            if DragQueryFileW(drop, i as u32, Some(&mut text)) as usize != len {
                return Err("读取剪贴板路径失败".into());
            }
            out.push(String::from_utf16(&text[..len]).map_err(|e| e.to_string())?);
        }
        if out.is_empty() {
            Ok(out)
        } else {
            checked_paths(out)
        }
    }
}
fn recycle_candidates(paths: Vec<String>) -> Result<Vec<String>, String> {
    let paths = checked_paths(paths)?;
    for path in &paths {
        let p = Path::new(path);
        if p.parent().is_none() || p.file_name().is_none() || path.starts_with("\\\\") {
            return Err("不能将磁盘根目录或网络路径移到回收站".into());
        }
        let root = p.ancestors().last().ok_or("无效磁盘路径")?;
        let root = wide(&root.to_string_lossy())?;
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != 3 {
            return Err("只支持本地固定磁盘的回收站操作".into());
        }
    }
    Ok(paths)
}
fn recycle_inner(paths: Vec<String>) -> Result<usize, String> {
    let paths = recycle_candidates(paths)?;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
    }
    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _com = Com;
    unsafe {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        op.SetOperationFlags(
            FOFX_RECYCLEONDELETE
                | FOFX_ADDUNDORECORD
                | FOF_ALLOWUNDO
                | FOF_NOERRORUI
                | FOFX_EARLYFAILURE
                | FOF_SILENT
                | FOF_NOCONFIRMATION
                | FOF_NO_CONNECTED_ELEMENTS,
        )
        .map_err(|e| e.to_string())?;
        for path in &paths {
            no_links(Path::new(path))?;
            let p = wide(path)?;
            let item: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(p.as_ptr()), None).map_err(|e| e.to_string())?;
            op.DeleteItem(&item, None).map_err(|e| e.to_string())?;
        }
        op.PerformOperations().map_err(|e| e.to_string())?;
        if op
            .GetAnyOperationsAborted()
            .map_err(|e| e.to_string())?
            .as_bool()
        {
            return Err("回收站操作未全部完成，没有尝试永久删除".into());
        }
        Ok(paths.len())
    }
}
#[tauri::command]
pub async fn recycle_paths(paths: Vec<String>) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::spawn(move || recycle_inner(paths))
            .join()
            .map_err(|_| "回收站操作线程异常".to_string())?
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, PartialEq, Eq)]
struct ArchiveIdentity {
    volume: u32,
    index: u64,
    size: u64,
    modified: u64,
}
fn archive_identity(file: &File) -> Result<ArchiveIdentity, String> {
    use std::os::windows::io::AsRawHandle;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
            .map_err(|e| e.to_string())?;
    }
    if info.nNumberOfLinks != 1 || info.dwFileAttributes & (0x400 | 0x10) != 0 {
        return Err("自动回收不支持链接、重解析点或目录".into());
    }
    Ok(ArchiveIdentity {
        volume: info.dwVolumeSerialNumber,
        index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        modified: ((info.ftLastWriteTime.dwHighDateTime as u64) << 32)
            | info.ftLastWriteTime.dwLowDateTime as u64,
    })
}
fn readonly_archive(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    no_links(path)?;
    OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
        .open(path).map_err(|e| format!("无法锁定原压缩包，未解压或回收：{e}"))
}
fn volume_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    (ext.len() >= 3 && ext.bytes().all(|c| c.is_ascii_digit()))
        || (ext.len() >= 3 && matches!(ext.as_bytes()[0], b'r' | b'z')
            && ext.as_bytes()[1..].iter().all(u8::is_ascii_digit))
        || lower.rsplit_once(".part").is_some_and(|(_, tail)| {
            tail.strip_suffix(".rar").is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        })
}
fn single_archive_name(path: &Path) -> Result<(), String> {
    let name = path.file_name().and_then(|v| v.to_str()).ok_or("无效压缩包名称")?.to_ascii_lowercase();
    if volume_name(&name) { return Err("分卷压缩包暂不支持解压后自动回收，请使用普通解压".into()); }
    let ext = path.extension().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["zip", "zipx", "7z", "tar", "gz", "gzip", "bz2", "bzip2", "xz", "tgz", "tbz", "tbz2", "txz"].contains(&ext.as_str()) {
        return Err("此格式尚未验证单卷自动回收，请使用普通解压".into());
    }
    let stem = path.file_stem().and_then(|v|v.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut count = 0;
    for entry in fs::read_dir(path.parent().ok_or("无效压缩包路径")?).map_err(|e|e.to_string())? {
        count += 1;
        if count > 100000 { return Err("目录过大，不能确认分卷状态，请使用普通解压".into()); }
        let entry = entry.map_err(|e|e.to_string())?;
        let candidate = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if candidate != name && (candidate.starts_with(&(name.clone()+"."))
            || candidate.starts_with(&(stem.clone()+"."))) && volume_name(&candidate) {
            return Err("检测到关联分卷，保留所有压缩包，请使用普通解压".into());
        }
    }
    Ok(())
}
/// Held for the entire decode and publication. Windows denies writes and renames
/// while the guard is live; no completion claim is accepted from the frontend.
pub(crate) struct ArchiveRecycleGuard {
    file: File,
    path: PathBuf,
    identity: ArchiveIdentity,
}
impl ArchiveRecycleGuard {
    pub(crate) fn acquire(path: &str) -> Result<Self, String> {
        let path = recycle_candidates(vec![path.into()])?.remove(0);
        let path = existing(&path)?;
        single_archive_name(&path)?;
        let file = readonly_archive(&path)?;
        let identity = archive_identity(&file)?;
        Ok(Self { file, path, identity })
    }
    fn unchanged(&self) -> Result<(), String> {
        single_archive_name(&self.path)?;
        let current = readonly_archive(&self.path)?;
        if archive_identity(&self.file)? != self.identity || archive_identity(&current)? != self.identity {
            return Err("原压缩包发生改变，解压结果保留，原包未回收".into());
        }
        Ok(())
    }
    pub(crate) fn recycle(self) -> Result<(), String> {
        self.unchanged()?;
        let path = self.path.to_string_lossy().into_owned();
        // IFileOperation needs DELETE access. Release the write/delete-denying
        // source handle only at the final, non-cancellable commit boundary.
        drop(self);
        recycle_inner(vec![path.clone()])?;
        if Path::new(&path).exists() {
            return Err("回收站未移走原压缩包，未尝试永久删除".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!(
                "open-archive-desktop-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn shell_file_lists_resolve_pinned_drag_exports() {
        let export = obz_core::engine::ShellDragDirectory::create().unwrap();
        let file = export.path().join("drag.txt");
        fs::write(&file, b"payload").unwrap();
        let _pin = export.pin_for_read().unwrap();
        let paths = checked_paths(vec![file.display().to_string()]).unwrap();
        assert_eq!(fs::read(&paths[0]).unwrap(), b"payload");
    }
    #[test]
    fn shell_file_lists_keep_distinct_names_and_fold_aliases() {
        let temp = Temp::new();
        let lower = temp.0.join("sample.txt");
        let upper = temp.0.join("SAMPLE.txt");
        fs::write(&lower, b"lower").unwrap();
        let canonical_parent = canonical_spelling(&temp.0, true).unwrap();
        let expected = |name| Path::new(&canonical_parent).join(name).display().to_string();
        let path_list = || [&lower, &upper, &lower].map(|p| p.display().to_string()).to_vec();
        if upper.exists() {
            assert_eq!(checked_paths(path_list()).unwrap(), vec![expected("sample.txt")]);
        }
        fs::remove_file(&lower).unwrap();
        let enabled = std::process::Command::new("fsutil")
            .args(["file", "setCaseSensitiveInfo"]).arg(&temp.0).arg("enable").output().unwrap();
        if enabled.status.success() {
            fs::write(&lower, b"lower").unwrap();
            File::create_new(&upper).unwrap();
            assert_eq!(checked_paths(path_list()).unwrap(),
                vec![expected("sample.txt"), expected("SAMPLE.txt")]);
        }
    }
    #[test]
    fn automatic_recycle_locks_source_and_checks_identity() {
        let temp=Temp::new();let path=temp.0.join("sample.zip");fs::write(&path,b"test").unwrap();
        let mut guard=ArchiveRecycleGuard::acquire(path.to_str().unwrap()).unwrap();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::remove_file(&path).is_err());
        assert!(fs::rename(&path,temp.0.join("moved.zip")).is_err());
        assert!(guard.unchanged().is_ok());
        guard.identity.size+=1;
        assert!(guard.unchanged().is_err());
        drop(guard);assert_eq!(fs::read(path).unwrap(),b"test");
    }
    #[test]
    fn automatic_recycle_rejects_volumes_companions_and_hardlinks() {
        let temp=Temp::new();
        for name in ["a.zip.001","a.7z.001","a.z01","a.r00","a.part01.rar","a.rar","a.exe"] {
            let path=temp.0.join(name);fs::write(&path,b"test").unwrap();
            assert!(ArchiveRecycleGuard::acquire(path.to_str().unwrap()).is_err(),"{name}");
        }
        let path=temp.0.join("a.zip");fs::write(&path,b"test").unwrap();
        assert!(ArchiveRecycleGuard::acquire(path.to_str().unwrap()).is_err());
        let hard=temp.0.join("hard.zip");fs::write(&hard,b"test").unwrap();
        fs::hard_link(&hard,temp.0.join("hard-copy.zip")).unwrap();
        assert!(ArchiveRecycleGuard::acquire(hard.to_str().unwrap()).is_err());
    }
    #[test]
    fn copy_never_overwrites_or_copies_itself() {
        let t = Temp::new();
        let source = t.0.join("archive.zip");
        fs::write(&source, b"archive bytes").unwrap();
        let dest = t.0.join("copy.zip");
        copy_inner(source.to_str().unwrap(), dest.to_str().unwrap()).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"archive bytes");
        assert!(copy_inner(source.to_str().unwrap(), dest.to_str().unwrap()).is_err());
        assert!(copy_inner(source.to_str().unwrap(), source.to_str().unwrap()).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"archive bytes");
        let alias = t.0.join("alias.zip");
        fs::hard_link(&source, &alias).unwrap();
        assert!(copy_inner(
            alias.to_str().unwrap(),
            t.0.join("other.zip").to_str().unwrap()
        )
        .is_err());
    }
    #[test]
    fn archive_copy_preserves_internet_zone_and_refuses_malformed_marker() {
        let t = Temp::new();
        let source = t.0.join("archive.zip");
        let copy = t.0.join("copy.zip");
        let marker = |p: &Path| PathBuf::from(format!("{}:Zone.Identifier", p.display()));
        let bytes = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.invalid/archive.zip\r\n";
        fs::write(&source, b"archive data").unwrap();
        fs::write(marker(&source), bytes).unwrap();
        copy_inner(source.to_str().unwrap(), copy.to_str().unwrap()).unwrap();
        assert_eq!(fs::read(marker(&copy)).unwrap(), bytes);
        fs::write(marker(&source), b"not a zone").unwrap();
        let other = t.0.join("other.zip");
        assert!(copy_inner(source.to_str().unwrap(), other.to_str().unwrap()).is_err());
        assert!(!other.exists());
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 2);
    }
    #[test]
    fn clipboard_directories_are_distinct_and_empty() {
        let first = PathBuf::from(clipboard_directory().unwrap());
        let second = PathBuf::from(clipboard_directory().unwrap());
        assert_ne!(first, second);
        for path in [first, second] {
            assert!(path.starts_with(std::env::temp_dir()));
            assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
            fs::write(path.join("partial"), b"partial extraction").unwrap();
            discard_clipboard_export(path.display().to_string()).unwrap();
            assert!(!path.exists());
        }
    }
    #[test]
    fn directory_tree_contains_only_subdirectories() {
        let t = Temp::new();
        fs::create_dir(t.0.join("child")).unwrap();
        fs::write(t.0.join("file.txt"), b"text").unwrap();
        let rows = list_directory(t.0.to_string_lossy().into_owned()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "child");
    }
    #[test]
    fn clipboard_framing_is_bounded_without_touching_system_clipboard() {
        let b = drop_payload(&["C:\\temporary\\中文.zip".into()]).unwrap();
        assert!(validate_drop_payload(&b).is_ok());
        assert!(validate_drop_payload(&b[..b.len() - 4]).is_err());
        let mut bad = b.clone();
        bad[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(validate_drop_payload(&bad).is_err());
        assert!(drop_payload(&["bad\0name".into()]).is_err());
    }
    #[test]
    fn known_folder_ids_return_absolute_paths_without_listing_contents() {
        let folders = known_folders().unwrap();
        for p in [
            folders.desktop,
            folders.documents,
            folders.pictures,
            folders.downloads,
            folders.home,
        ] {
            assert!(Path::new(&p).is_absolute());
        }
    }
}
