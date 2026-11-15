//! Persistent recovery index, independent of temporary preview caches. Never stores passwords.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
static INDEX: Mutex<()> = Mutex::new(());
#[derive(Clone, Serialize, Deserialize)]
pub struct CopyRecord {
    pub path: String,
    pub parent: String,
    pub member: String,
}
fn no_links(path: &Path) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        let m = fs::symlink_metadata(part).map_err(|e| e.to_string())?;
        if m.file_attributes() & 0x400 != 0 {
            return Err("编辑副本路径不能包含链接".into());
        }
    }
    Ok(())
}
pub fn root() -> Result<PathBuf, String> {
    let path = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?)
        .join("qing-zip")
        .join("EditedFiles");
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    no_links(&path)?;
    Ok(path)
}
fn read(root: &Path) -> Result<Vec<CopyRecord>, String> {
    match fs::read(root.join("index.json")) {
        Ok(data) => serde_json::from_slice(&data).map_err(|e| format!("编辑副本索引无法读取：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}
fn write(root: &Path, records: &[CopyRecord]) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(records).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(root.join("index.json"))
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn remember(record: CopyRecord) -> Result<(), String> {
    let _lock = INDEX.lock().map_err(|_| "编辑副本索引不可用")?;
    let root = root()?;
    let mut records = read(&root)?;
    records.retain(|r| r.path != record.path);
    records.push(record);
    write(&root, &records)
}
pub fn forget(path: &str) -> Result<(), String> {
    let _lock = INDEX.lock().map_err(|_| "编辑副本索引不可用")?;
    let root = root()?;
    let mut records = read(&root)?;
    records.retain(|r| r.path != path);
    write(&root, &records)
}
#[tauri::command]
pub async fn retained_edits() -> Result<Vec<CopyRecord>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _lock = INDEX.lock().map_err(|_| "编辑副本索引不可用")?;
        let root = root()?;
        let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let records = read(&root)?
            .into_iter()
            .filter(|r| {
                let path = Path::new(&r.path);
                path.is_file()
                    && no_links(path).is_ok()
                    && fs::canonicalize(path).is_ok_and(|p| p.starts_with(&root))
            })
            .collect();
        Ok(records)
    })
    .await
    .map_err(|e| e.to_string())?
}
