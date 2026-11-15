//! Owns private nested archive workspaces for the lifetime of their tabs.
use obz_core::{
    engine::{
        nested::{ArchiveRevision, EditReceipt, NestedArchive},
        EngineOptions, JobControl,
    },
    operations::{OperationResponse, Request},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

struct Session {
    archive: NestedArchive,
    // Filesystem spelling, not the TEMP alias used to create the private copy.
    // Cache before editing: atomic replacement must not change graph identity.
    path_key: String,
    parent: String,
    member: String,
    options: EngineOptions,
    dirty: bool,
    conflict: bool,
    editor: Option<crate::editor_process::EditorProcess>,
}
#[derive(Serialize)]
pub struct View {
    id: u64,
    path: String,
    parent: String,
    member: String,
    dirty: bool,
    conflict: bool,
}
static SESSIONS: OnceLock<Mutex<BTreeMap<u64, Session>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
static DIRTY: AtomicBool = AtomicBool::new(false);
fn registry() -> &'static Mutex<BTreeMap<u64, Session>> {
    SESSIONS.get_or_init(Default::default)
}
fn key(path: &str) -> String {
    path.replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .to_owned()
}
fn request_key(path: &str) -> String {
    key(&crate::instance::canonical_archive_path(path.to_owned())
        .unwrap_or_else(|_| path.to_owned()))
}
fn view(id: u64, s: &Session) -> View {
    View {
        id,
        path: s.archive.path().to_string_lossy().into_owned(),
        parent: s.parent.clone(),
        member: s.member.clone(),
        dirty: s.dirty,
        conflict: s.conflict,
    }
}
fn update_dirty(sessions: &BTreeMap<u64, Session>) {
    DIRTY.store(
        sessions.values().any(|s| s.dirty || s.editor.is_some()),
        Ordering::SeqCst,
    );
}
pub fn dirty() -> bool {
    DIRTY.load(Ordering::SeqCst)
}
pub fn cleanup() {
    registry().lock().unwrap_or_else(|e| e.into_inner()).clear();
    DIRTY.store(false, Ordering::SeqCst);
}

pub fn open_job(
    parent: String,
    member: String,
    options: EngineOptions,
) -> Result<OperationResponse, String> {
    open_impl(parent, member, options, None)
}
pub fn edit_job(
    parent: String,
    member: String,
    options: EngineOptions,
    editor: String,
) -> Result<OperationResponse, String> {
    open_impl(parent, member, options, Some(editor))
}
fn open_impl(
    parent: String,
    member: String,
    mut options: EngineOptions,
    editor: Option<String>,
) -> Result<OperationResponse, String> {
    let parent = crate::instance::canonical_archive_path(parent)?;
    options.selected.clear();
    options.edit_receipt = None;
    let mut sessions = registry().lock().map_err(|_| "内层归档会话不可用")?;
    if editor.is_some() && sessions.values().any(|s| s.editor.is_some()) {
        return Err("请先完成现有的外部编辑会话（编辑菜单 → 外部编辑会话）".into());
    }
    if let Some((&id, s)) = sessions
        .iter()
        .find(|(_, s)| key(&s.parent) == key(&parent) && s.member == member)
    {
        if editor.is_some() || s.editor.is_some() {
            return Err("该成员已在另一会话中打开，请先完成并关闭它".into());
        }
        return Ok(OperationResponse::Json(
            serde_json::to_value(view(id, s)).map_err(|e| e.to_string())?,
        ));
    }
    let mut ancestor = parent.clone();
    let mut depth = 0;
    while let Some(s) = sessions
        .values()
        .find(|s| s.path_key == key(&ancestor))
    {
        depth += 1;
        if depth >= 32 {
            return Err("嵌套层数过深，请先解压后打开".into());
        }
        ancestor = s.parent.clone();
    }
    let archive = if editor.is_some() {
        NestedArchive::open_external(&parent, &member, &options, &crate::edit_copies::root()?)?
    } else {
        NestedArchive::open(&parent, &member, &options)?
    };
    let path_key = key(&crate::desktop::canonical_spelling(archive.path(), false)?);
    let process = match editor {
        Some(editor) => {
            if options
                .control
                .as_ref()
                .is_some_and(|c| c.cancelled.load(Ordering::SeqCst))
            {
                return Err("operation cancelled".into());
            }
            crate::edit_copies::remember(crate::edit_copies::CopyRecord {
                path: archive.path().to_string_lossy().into_owned(),
                parent: parent.clone(),
                member: member.clone(),
            })?;
            let process =
                match crate::editor_process::EditorProcess::launch(&editor, archive.path()) {
                    Ok(process) => process,
                    Err(error) => {
                        let _ = crate::edit_copies::forget(&archive.path().to_string_lossy());
                        return Err(error);
                    }
                };
            archive.retain_working_copy(true);
            Some(process)
        }
        None => None,
    };
    options.control = None;
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let session = Session {
        archive,
        path_key,
        parent,
        member,
        options,
        dirty: false,
        conflict: false,
        editor: process,
    };
    let result = serde_json::to_value(view(id, &session)).map_err(|e| e.to_string())?;
    sessions.insert(id, session);
    update_dirty(&sessions);
    Ok(OperationResponse::Json(result))
}
fn session_views() -> Vec<View> {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(&id, s)| view(id, s))
        .collect()
}
#[tauri::command]
pub async fn nested_sessions() -> Result<Vec<View>, String> {
    tauri::async_runtime::spawn_blocking(session_views)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn close_nested(ids: Vec<u64>, discard: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || close_sessions(ids, discard))
        .await
        .map_err(|e| e.to_string())?
}
fn close_sessions(ids: Vec<u64>, discard: bool) -> Result<(), String> {
    if crate::jobs::running() {
        return Err("请等待当前任务完成".into());
    }
    let mut sessions = registry().lock().map_err(|_| "内层归档会话不可用")?;
    let paths = ids
        .iter()
        .filter_map(|id| sessions.get(id))
        .map(|s| s.path_key.clone())
        .collect::<Vec<_>>();
    if sessions
        .iter()
        .any(|(id, s)| !ids.contains(id) && paths.contains(&key(&s.parent)))
    {
        return Err("请先关闭更深一层的压缩包标签".into());
    }
    if !discard
        && ids
            .iter()
            .filter_map(|id| sessions.get(id))
            .any(|s| s.dirty)
    {
        return Err("内层压缩包尚未写回，请保存或明确放弃更改".into());
    }
    if ids
        .iter()
        .any(|id| sessions.get(id).is_some_and(|s| s.editor.is_some()))
    {
        return Err("请通过外部编辑会话确认写回或结束编辑".into());
    }
    for id in ids {
        sessions.remove(&id);
    }
    update_dirty(&sessions);
    Ok(())
}
pub fn validate_request(request: &Request, recycle: bool) -> Result<(), String> {
    if !recycle {
        return Ok(());
    }
    let path = request_key(&request.path);
    if recycle
        && registry()
            .lock()
            .map_err(|_| "内层归档会话不可用")?
            .values()
            .any(|s| s.path_key == path || key(&s.parent) == path)
    {
        return Err("请先保存并关闭内层标签，再回收压缩包".into());
    }
    Ok(())
}
fn acknowledge(
    sessions: &mut BTreeMap<u64, Session>,
    path: &str,
    revision: &ArchiveRevision,
    skip: Option<u64>,
) {
    for (&id, session) in sessions.iter_mut() {
        if skip == Some(id) {
            continue;
        }
        if session.path_key == key(path) {
            session.dirty = true;
        }
        if key(&session.parent) == key(path) {
            match session
                .archive
                .rebind_after_commit(revision, &session.options)
            {
                Ok(true) => session.conflict = false,
                _ => session.conflict = true,
            }
        }
    }
    update_dirty(sessions);
}
pub fn committed(path: &str, revision: &ArchiveRevision) {
    let path = request_key(path);
    let mut sessions = registry().lock().unwrap_or_else(|e| e.into_inner());
    acknowledge(&mut sessions, &path, revision, None);
}
pub fn save_job(path: &str, control: &JobControl) -> Result<OperationResponse, String> {
    save_impl(path, control, false)
}
fn save_impl(
    path: &str,
    control: &JobControl,
    external_confirmed: bool,
) -> Result<OperationResponse, String> {
    let path = request_key(path);
    let mut sessions = registry().lock().map_err(|_| "内层归档会话不可用")?;
    let id = sessions
        .iter()
        .find_map(|(&id, s)| (s.path_key == path).then_some(id))
        .ok_or("内层归档会话已关闭")?;
    // Save descendants first, then this archive and each enclosing archive.
    let mut pending = vec![id];
    let mut index = 0;
    while index < pending.len() {
        let path = sessions[&pending[index]].path_key.clone();
        let children = sessions
            .iter()
            .filter_map(|(&id, s)| (key(&s.parent) == path).then_some(id))
            .collect::<Vec<_>>();
        pending.extend(children);
        index += 1;
    }
    pending.reverse();
    let mut ancestor = sessions[&id].parent.clone();
    while let Some((&id, s)) = sessions
        .iter()
        .find(|(_, s)| s.path_key == key(&ancestor))
    {
        pending.push(id);
        ancestor = s.parent.clone();
    }
    if (sessions[&id].editor.is_some() && !external_confirmed)
        || pending
            .iter()
            .any(|other| *other != id && sessions[other].editor.is_some())
    {
        return Err("请通过外部编辑会话确认写回包内文件".into());
    }
    for id in pending {
        let (parent, receipt) = {
            let session = sessions.get_mut(&id).ok_or("内层归档会话已关闭")?;
            if !session.dirty {
                continue;
            }
            if control.cancelled.load(Ordering::SeqCst) {
                return Err("operation cancelled".into());
            }
            let mut options = session.options.clone();
            options.control = Some(control.clone());
            let receipt = EditReceipt::capture(&mut options);
            session
                .archive
                .save(&options)
                .map_err(|e| format!("写回失败，内层工作副本已保留，可重试或另存为：{e}"))?;
            session.dirty = false;
            session.conflict = false;
            (session.parent.clone(), receipt.take())
        };
        if let Some(receipt) = receipt {
            acknowledge(&mut sessions, &parent, &receipt, Some(id));
        }
        update_dirty(&sessions);
    }
    Ok(OperationResponse::Json(serde_json::json!({"saved":true})))
}

pub fn track(request: &mut Request) -> Option<EditReceipt> {
    if !matches!(
        request.operation.as_str(),
        "add" | "delete" | "rename" | "comment-set"
    ) {
        return None;
    }
    let sessions = registry().lock().unwrap_or_else(|e| e.into_inner());
    if sessions.is_empty() {
        return None;
    }
    let path = request_key(&request.path);
    if sessions
        .values()
        .any(|s| key(&s.parent) == path || s.path_key == path)
    {
        Some(EditReceipt::capture(&mut request.options))
    } else {
        None
    }
}

#[derive(Serialize)]
pub struct ExternalView {
    id: u64,
    path: String,
    parent: String,
    member: String,
    state: String,
    changed: Option<bool>,
    error: Option<String>,
}
#[tauri::command]
pub async fn external_edits() -> Result<Vec<ExternalView>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let sessions = registry().lock().map_err(|_| "编辑会话不可用")?;
        Ok(sessions
            .iter()
            .filter_map(|(&id, s)| {
                s.editor.as_ref().map(|editor| {
                    let state = editor.state().unwrap_or("handoff").to_owned();
                    let changed = s.archive.has_changes(&s.options);
                    ExternalView {
                        id,
                        path: s.archive.path().to_string_lossy().into_owned(),
                        parent: s.parent.clone(),
                        member: s.member.clone(),
                        state,
                        changed: changed.as_ref().ok().copied(),
                        error: changed.err(),
                    }
                })
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}
pub fn external_save_job(path: &str, control: &JobControl) -> Result<OperationResponse, String> {
    let path_key = request_key(path);
    {
        let mut sessions = registry().lock().map_err(|_| "编辑会话不可用")?;
        let session = sessions
            .values_mut()
            .find(|s| s.path_key == path_key && s.editor.is_some())
            .ok_or("编辑会话已结束")?;
        // The explicit save command is the only operation that authorizes these bytes.
        // save() pins the working file again for the complete transactional write-back.
        session.dirty = session.archive.has_changes(&session.options)?;
        if !session.dirty {
            return Ok(OperationResponse::Json(
                serde_json::json!({"saved":false,"unchanged":true}),
            ));
        }
    }
    save_impl(path, control, true)
}
#[tauri::command]
pub async fn finish_external_edit(id: u64, discard: bool) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if crate::jobs::running() {
            return Err("请等待当前任务完成".into());
        }
        let mut sessions = registry().lock().map_err(|_| "编辑会话不可用")?;
        let session = sessions.get(&id).ok_or("编辑会话已结束")?;
        if session.editor.is_none() {
            return Err("不是外部编辑会话".into());
        }
        if !discard && session.archive.has_changes(&session.options)? {
            return Err("文件仍有未写回的更改，请重新确认或选择不写回".into());
        }
        // Even a terminated launcher can have handed this document to an
        // already-running host outside our job. Never delete external copies
        // on the strength of process timing or executable name.
        session.archive.retain_working_copy(true);
        let retained = Some(session.archive.path().to_string_lossy().into_owned());
        sessions.remove(&id);
        update_dirty(&sessions);
        Ok(retained)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use obz_core::{engine, operations};
    fn open(parent: &str, member: &str) -> View {
        let OperationResponse::Json(value) =
            open_job(parent.into(), member.into(), EngineOptions::default()).unwrap()
        else {
            panic!()
        };
        let sessions = session_views();
        sessions
            .into_iter()
            .find(|s| Some(s.id) == value["id"].as_u64())
            .unwrap()
    }
    fn rename(path: &str, from: &str, to: &str) {
        let mut request = Request {
            operation: "rename".into(),
            path: path.into(),
            inputs: vec![from.into(), to.into()],
            ..Default::default()
        };
        let receipt = track(&mut request).unwrap();
        operations::execute_request(request).unwrap();
        committed(path, &receipt.take().unwrap());
    }
    fn names(path: &str) -> Vec<String> {
        operations::list_archive(path, &Default::default())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }
    #[test]
    fn session_graph_uses_filesystem_spelling_for_temp_aliases() {
        // A separate process avoids mutating TEMP while unrelated tests run.
        // Hosted Windows runners can supply a short-name TEMP; an alternate
        // spelling on case-insensitive NTFS reproduces the same key mismatch.
        let temp = tempfile::Builder::new().prefix("NestedMixedCaseTemp-").tempdir().unwrap();
        let actual = crate::desktop::canonical_spelling(temp.path(), true).unwrap();
        let alias = temp.path().with_file_name(temp.path().file_name().unwrap().to_string_lossy().to_uppercase());
        if crate::desktop::canonical_spelling(&alias, true).ok().as_deref() != Some(actual.as_str()) {
            // This volume is case-sensitive: do not reinterpret a distinct path.
            return;
        }
        assert_ne!(key(&alias.to_string_lossy()), key(&actual));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "nested::tests::session_graph_saves_siblings_and_descendants_and_retains_external_conflicts", "--nocapture"])
            .env("TEMP", &alias).env("TMP", &alias).output().unwrap();
        assert!(output.status.success(), "nested graph with aliased TEMP failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn session_graph_saves_siblings_and_descendants_and_retains_external_conflicts() {
        cleanup();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let text = root.join("content.txt");
        std::fs::write(&text, "original").unwrap();
        let deep = root.join("deep.zip");
        engine::create(
            deep.to_str().unwrap(),
            &[text.display().to_string()],
            "zip",
            &Default::default(),
        )
        .unwrap();
        let a = root.join("a.zip");
        let b = root.join("b.zip");
        engine::create(
            a.to_str().unwrap(),
            &[deep.display().to_string(), text.display().to_string()],
            "zip",
            &Default::default(),
        )
        .unwrap();
        engine::create(
            b.to_str().unwrap(),
            &[text.display().to_string()],
            "zip",
            &Default::default(),
        )
        .unwrap();
        let outer = root.join("outer.zip");
        engine::create(
            outer.to_str().unwrap(),
            &[a.display().to_string(), b.display().to_string()],
            "zip",
            &Default::default(),
        )
        .unwrap();
        let a = open(outer.to_str().unwrap(), "a.zip");
        let b = open(outer.to_str().unwrap(), "b.zip");
        let deep = open(&a.path, "deep.zip");
        rename(&deep.path, "content.txt", "deep-edited.txt");
        assert!(dirty());
        save_job(&deep.path, &JobControl::new()).unwrap();
        assert!(!dirty());
        assert!(session_views().iter().all(|s| !s.conflict));
        let check =
            NestedArchive::open(outer.to_str().unwrap(), "a.zip", &Default::default()).unwrap();
        let check_deep = NestedArchive::open(
            check.path().to_str().unwrap(),
            "deep.zip",
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            names(check_deep.path().to_str().unwrap()),
            vec!["deep-edited.txt"]
        );
        assert_eq!(
            open(&outer.to_string_lossy().to_lowercase(), "b.zip").id,
            b.id
        );
        assert!(validate_request(
            &Request {
                path: outer.to_string_lossy().to_lowercase(),
                operation: "extract".into(),
                ..Default::default()
            },
            true
        )
        .is_err());
        rename(&b.path.to_lowercase(), "content.txt", "b-edited.txt");
        save_job(&b.path, &JobControl::new()).unwrap();
        assert!(!dirty());
        assert!(session_views().iter().all(|s| !s.conflict));
        rename(&a.path, "content.txt", "a-edited.txt");
        assert!(close_sessions(vec![a.id], false).is_err());
        // A genuine outside writer changes the same object. No receipt can authorize it.
        std::fs::write(&outer, "outside version").unwrap();
        assert!(save_job(&a.path, &JobControl::new()).is_err());
        assert_eq!(std::fs::read(&outer).unwrap(), b"outside version");
        assert!(std::path::Path::new(&a.path).exists());
        assert!(dirty());
        assert!(close_sessions(vec![a.id, b.id, deep.id], false).is_err());
        close_sessions(vec![a.id, b.id, deep.id], true).unwrap();
        assert!(!dirty());
        assert!(session_views().is_empty());
        assert!(!std::path::Path::new(&a.path).exists());
    }
}
