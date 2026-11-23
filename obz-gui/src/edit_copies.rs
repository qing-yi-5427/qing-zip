//! Persistent recovery index, independent of temporary preview caches. Never stores passwords.
use obz_core::engine::nested::RecoveryCopy;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
static INDEX: Mutex<()> = Mutex::new(());
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyRecord {
    pub path: String,
    pub parent: String,
    pub member: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested: Option<NestedCopy>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NestedCopy {
    pub outer: String,
    pub members: Vec<String>,
    pub proof: RecoveryCopy,
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
    #[cfg(test)]
    let path = {
        static TEST_ROOT: std::sync::OnceLock<Result<tempfile::TempDir, String>> =
            std::sync::OnceLock::new();
        TEST_ROOT.get_or_init(|| tempfile::tempdir().map_err(|e| e.to_string()))
            .as_ref().map_err(Clone::clone)?.path().to_owned()
    };
    #[cfg(not(test))]
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
    forget_many(&[path])
}
pub fn forget_many(paths: &[&str]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let _lock = INDEX.lock().map_err(|_| "编辑副本索引不可用")?;
    let root = root()?;
    let mut records = read(&root)?;
    let previous = records.len();
    records.retain(|r| !paths.contains(&r.path.as_str()));
    if records.len() != previous {
        write(&root, &records)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn retained_edits() -> Result<Vec<RetainedView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        retained_copy_records()?.into_iter().map(|record| {
            // Windows file IDs and timestamps cannot round-trip as JS numbers.
            let selection = serde_json::to_string(&record).map_err(|e| e.to_string())?;
            let label = match &record.nested {
                Some(copy) => std::iter::once(copy.outer.as_str()).chain(copy.members.iter().map(String::as_str)).collect::<Vec<_>>().join(" › "),
                None => format!("{} — {}", record.member, record.path),
            };
            Ok(RetainedView { path: record.path, label, nested: record.nested.is_some(), selection })
        }).collect()
    }).await.map_err(|e| e.to_string())?
}
#[derive(Serialize)]
pub struct RetainedView {
    path: String,
    label: String,
    nested: bool,
    selection: String,
}

fn external_path(path: &Path, root: &Path) -> bool {
    path.is_file() && no_links(path).is_ok()
        && fs::canonicalize(path).is_ok_and(|p| p.starts_with(root))
}

fn indexed() -> Result<(PathBuf, Vec<CopyRecord>), String> {
    let _lock = INDEX.lock().map_err(|_| "编辑副本索引不可用")?;
    let root = fs::canonicalize(root()?).map_err(|e| e.to_string())?;
    let records = read(&root)?;
    Ok((root, records))
}

pub(crate) fn retained_copy_records() -> Result<Vec<CopyRecord>, String> {
    let active = crate::nested::active_copy_paths();
    let (root, records) = indexed()?;
    Ok(records.into_iter().filter(|record| {
        let path = Path::new(&record.path);
        match &record.nested {
            Some(copy) => !active.contains(&record.path)
                && copy.proof.pin_changed(path, &record.member, false).is_ok_and(|file| file.is_some()),
            None => external_path(path, &root),
        }
    }).collect())
}

#[tauri::command]
pub async fn save_retained_edit(selection: String, dest: String) -> Result<RetainedSave, String> {
    tauri::async_runtime::spawn_blocking(move || save_copy(&selection, &dest))
        .await.map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct RetainedSave {
    path: String,
    cleanup_warning: bool,
}

pub(crate) fn save_copy(selection: &str, dest: &str) -> Result<RetainedSave, String> {
    let record: CopyRecord = serde_json::from_str(selection).map_err(|_| "编辑副本选择无效")?;
    let (root, records) = indexed()?;
    if !records.contains(&record) {
        return Err("编辑副本登记已变化，请重新打开恢复列表".into());
    }
    let path = Path::new(&record.path);
    if let Some(copy) = &record.nested {
        if crate::nested::active_copy_paths().contains(&record.path) {
            return Err("该工作副本仍在当前标签中打开".into());
        }
        let _source = copy.proof.pin_changed(path, &record.member, true)?
            .ok_or("临时工作副本已清理或发生变化，未复制任何文件")?;
        let path = crate::desktop::copy_inner(&record.path, dest)?;
        Ok(RetainedSave { path, cleanup_warning: forget(&record.path).is_err() })
    } else if external_path(path, &root) {
        let path = crate::desktop::copy_inner(&record.path, dest)?;
        Ok(RetainedSave { path, cleanup_warning: false })
    } else {
        Err("编辑副本不再可用".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_external_record_still_lists_and_copies_without_consuming_it() {
        let _registry = crate::jobs::test_registry_guard();
        let index_root = root().unwrap();
        let fixture = tempfile::tempdir_in(index_root).unwrap();
        let copy = fixture.path().join("old-working.txt");
        let bytes = b"older external editor working copy";
        fs::write(&copy, bytes).unwrap();
        let original_record = serde_json::json!({
            "path": copy.to_str().unwrap(), "parent": "outer.zip", "member": "old-working.txt"
        });
        let record: CopyRecord = serde_json::from_value(original_record.clone()).unwrap();
        assert!(record.nested.is_none());
        assert_eq!(serde_json::to_value(&record).unwrap(), original_record);
        remember(record.clone()).unwrap();
        assert!(retained_copy_records().unwrap().contains(&record));
        let dest = fixture.path().join("exported.txt");
        assert!(!save_copy(&original_record.to_string(), dest.to_str().unwrap()).unwrap().cleanup_warning);
        assert_eq!(fs::read(dest).unwrap(), bytes);
        assert_eq!(fs::read(copy).unwrap(), bytes);
        assert!(retained_copy_records().unwrap().contains(&record));
        forget(&record.path).unwrap();
    }
}
