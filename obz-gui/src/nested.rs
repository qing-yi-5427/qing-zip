//! Owns private nested archive workspaces for the lifetime of their tabs.
use obz_core::{
    engine::{
        nested::{ordinary_recovery_zip, ArchiveRevision, EditReceipt, NestedArchive},
        EngineOptions, JobControl,
    },
    operations::{OperationResponse, Request},
};
use serde::Serialize;
use crate::edit_copies::{self, CopyRecord, NestedCopy};
use std::{
    collections::{BTreeMap, HashSet},
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
    recovery: Recovery,
    recovery_unavailable: bool,
}
enum Recovery {
    Unchecked,
    Disabled,
    Armed(NestedCopy),
}
impl Session {
    fn recovered_path(&self) -> Option<&str> {
        matches!(self.recovery, Recovery::Armed(_)).then(|| self.archive.path().to_str()).flatten()
    }
    fn register_recovery(&self, copy: NestedCopy) -> Result<(), String> {
        edit_copies::remember(CopyRecord {
            path: self.archive.path().to_string_lossy().into_owned(),
            parent: self.parent.clone(),
            member: self.member.clone(),
            nested: Some(copy),
        })
    }
    fn clear_recovery(&mut self) -> Result<(), String> {
        if let Some(path) = self.recovered_path() {
            edit_copies::forget(path)?;
            self.recovery = Recovery::Unchecked;
        }
        Ok(())
    }
    fn publish_recovery(&mut self, revision: &ArchiveRevision) -> Result<(), String> {
        if self.recovery_unavailable {
            return Err("无法绑定临时工作副本".into());
        }
        if let Recovery::Armed(copy) = &self.recovery {
            let mut copy = copy.clone();
            if copy.proof.record_commit(self.archive.path(), revision)? {
                self.register_recovery(copy.clone())?;
                self.recovery = Recovery::Armed(copy);
            }
        }
        Ok(())
    }
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
static GENERATION: AtomicU64 = AtomicU64::new(0);
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
pub fn cleanup_if_idle() -> Option<bool> {
    // Dropping workspaces can touch disk. Only the registry decision and
    // detachment need to happen under the job-admission lock.
    let removed = crate::jobs::if_idle(take_clean_sessions)?;
    if let Some(sessions) = &removed {
        let paths = sessions.values().filter_map(Session::recovered_path).collect::<Vec<_>>();
        let _ = crate::edit_copies::forget_many(&paths);
    }
    Some(removed.is_some())
}
fn take_clean_sessions() -> Option<BTreeMap<u64, Session>> {
    if dirty() {
        return None;
    }
    let mut sessions = registry().lock().ok()?;
    if sessions.values().any(|session| session.dirty || session.editor.is_some()) {
        return None;
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
    DIRTY.store(false, Ordering::SeqCst);
    Some(std::mem::take(&mut *sessions))
}
#[cfg(test)]
fn cleanup() {
    let mut sessions = registry().lock().unwrap_or_else(|e| e.into_inner());
    GENERATION.fetch_add(1, Ordering::Relaxed);
    sessions.clear();
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
    const MAX_OPEN_WORKSPACES: usize = 256;
    let editing = editor.is_some();
    let member_key = key(&member).to_lowercase();
    let existing = |sessions: &BTreeMap<u64, Session>| -> Result<Option<OperationResponse>, String> {
        if let Some((&id, session)) = sessions.iter().find(|(_, session)| {
            key(&session.parent) == key(&parent) && key(&session.member).to_lowercase() == member_key
        }) {
            if editing || session.editor.is_some() {
                return Err("该成员已在另一会话中打开，请先完成并关闭它".into());
            }
            return Ok(Some(OperationResponse::Json(
                serde_json::to_value(view(id, session)).map_err(|e| e.to_string())?,
            )));
        }
        if sessions.len() >= MAX_OPEN_WORKSPACES {
            return Err("打开的内层归档和编辑工作区过多，请关闭暂不用的会话".into());
        }
        Ok(None)
    };
    let generation = {
        let sessions = registry().lock().map_err(|_| "内层归档会话不可用")?;
        if let Some(response) = existing(&sessions)? {
            return Ok(response);
        }
        GENERATION.load(Ordering::Relaxed)
    };
    // Extract outside the shared registry lock. The engine pins the outer
    // archive during extraction, and each work copy owns a private directory.
    let archive = if editor.is_some() {
        NestedArchive::open_external(&parent, &member, &options, &crate::edit_copies::root()?)?
    } else {
        NestedArchive::open_with_recovery(&parent, &member, &options)?
    };
    let path_key = key(&crate::desktop::canonical_spelling(archive.path(), false)?);
    let mut sessions = registry().lock().map_err(|_| "内层归档会话不可用")?;
    if GENERATION.load(Ordering::Relaxed) != generation {
        return Err("内层归档会话已关闭".into());
    }
    if let Some(response) = existing(&sessions)? {
        return Ok(response);
    }
    archive.check_parent_current()?;

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
                nested: None,
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
    let recovery = if process.is_none() && options.password.is_none() {
        Recovery::Unchecked
    } else {
        Recovery::Disabled
    };
    let session = Session {
        archive,
        path_key,
        parent,
        member,
        options,
        dirty: false,
        conflict: false,
        editor: process,
        recovery,
        recovery_unavailable: false,
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
pub(crate) fn active_copy_paths() -> HashSet<String> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).values()
        .map(|session| session.archive.path().to_string_lossy().into_owned()).collect()
}
#[tauri::command]
pub async fn nested_sessions() -> Result<Vec<View>, String> {
    tauri::async_runtime::spawn_blocking(session_views)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn close_nested(ids: Vec<u64>, discard: bool) -> Result<(), String> {
    crate::verification::revoke();
    tauri::async_runtime::spawn_blocking(move || close_sessions(ids, discard))
        .await
        .map_err(|e| e.to_string())?
}
fn close_sessions(ids: Vec<u64>, discard: bool) -> Result<(), String> {
    crate::jobs::if_idle(|| close_idle_sessions(ids, discard))
        .unwrap_or_else(|| Err("请等待当前任务完成".into()))?;
    Ok(())
}
fn close_idle_sessions(ids: Vec<u64>, discard: bool) -> Result<Vec<Session>, String> {
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
    let records = ids.iter().filter_map(|id| sessions.get(id).and_then(Session::recovered_path)).collect::<Vec<_>>();
    // Removing the owned workspace also invalidates any stale index entry.
    // An unavailable index must not veto an explicit discard or clean close.
    let _ = edit_copies::forget_many(&records);
    let removed = ids.into_iter().filter_map(|id| sessions.remove(&id)).collect();
    update_dirty(&sessions);
    Ok(removed)
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
) -> Option<String> {
    let mut warning = None;
    for (&id, session) in sessions.iter_mut() {
        if skip == Some(id) {
            continue;
        }
        if session.path_key == key(path) {
            session.dirty = true;
            if let Err(error) = session.publish_recovery(revision) {
                warning = Some(format!("内层更改已保存在当前标签，跨会话恢复登记未完成。关闭前请保存或另存为：{error}"));
            }
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
    warning
}
pub fn committed(path: &str, revision: &ArchiveRevision) -> Option<String> {
    let path = request_key(path);
    let mut sessions = registry().lock().unwrap_or_else(|e| e.into_inner());
    acknowledge(&mut sessions, &path, revision, None)
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
    let mut warning = None;
    for id in pending {
        let session = sessions.get(&id).ok_or("内层归档会话已关闭")?;
        if !session.dirty {
            continue;
        }
        if control.cancelled.load(Ordering::SeqCst) {
            return Err("operation cancelled".into());
        }
        let sensitive = matches!(session.recovery, Recovery::Disabled);
        let parent = key(&session.parent);
        if let Some((&parent_id, _)) = sessions.iter().find(|(_, s)| s.path_key == parent) {
            arm_recovery(&mut sessions, parent_id, sensitive)?;
        }
        let (parent, receipt) = {
            let session = sessions.get_mut(&id).ok_or("内层归档会话已关闭")?;
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
        update_dirty(&sessions);
        if let Some(receipt) = receipt {
            if let Some(message) = acknowledge(&mut sessions, &parent, &receipt, Some(id)) {
                warning = Some(message);
                // The earlier child is still useful if the new parent could
                // not be registered and saving the outer archive later fails.
                continue;
            }
        }
        if let Err(error) = sessions.get_mut(&id).ok_or("内层归档会话已关闭")?.clear_recovery() {
            warning = Some(format!("内层更改已写回，但恢复登记未能清除：{error}"));
        }
    }
    Ok(OperationResponse::Json(serde_json::json!({"saved":true,"recovery_warning":warning})))
}

fn plain_recovery_chain(sessions: &BTreeMap<u64, Session>, id: u64) -> Option<(String, Vec<String>)> {
    let mut session = &sessions[&id];
    if !ordinary_recovery_zip(session.archive.path()) {
        return None;
    }
    let mut members = Vec::new();
    for _ in 0..sessions.len() {
        if matches!(session.recovery, Recovery::Disabled)
            || !session.archive.has_ordinary_source()
        {
            return None;
        }
        members.push(session.member.clone());
        let parent = key(&session.parent);
        match sessions.values().find(|candidate| candidate.path_key == parent) {
            Some(ancestor) => session = ancestor,
            None => {
                members.reverse();
                return Some((session.parent.clone(), members));
            }
        }
    }
    None
}

fn arm_recovery(sessions: &mut BTreeMap<u64, Session>, id: u64, sensitive: bool) -> Result<(), String> {
    let state = &sessions[&id].recovery;
    if matches!(state, Recovery::Disabled) {
        return Ok(());
    }
    let origin = (!sensitive && matches!(state, Recovery::Unchecked))
        .then(|| plain_recovery_chain(sessions, id)).flatten();
    let session = sessions.get_mut(&id).ok_or("内层归档会话已关闭")?;
    if sensitive {
        if matches!(&session.recovery, Recovery::Armed(copy) if copy.proof.has_commit()) {
            session.clear_recovery().map_err(|error| format!("无法在处理密码数据前撤销旧恢复登记：{error}"))?;
        }
        session.recovery = Recovery::Disabled;
        session.recovery_unavailable = false;
    } else if matches!(session.recovery, Recovery::Unchecked) {
        if let Some((outer, members)) = origin {
            match session.archive.recovery_copy() {
                Ok(proof) => {
                    let copy = NestedCopy { outer, members, proof };
                    // If the index was briefly unavailable, the completed
                    // receipt can still be recorded after the edit succeeds.
                    let _ = session.register_recovery(copy.clone());
                    session.recovery = Recovery::Armed(copy);
                    session.recovery_unavailable = false;
                }
                Err(_) => session.recovery_unavailable = true,
            }
        } else {
            session.recovery = Recovery::Disabled;
            session.recovery_unavailable = false;
        }
    }
    Ok(())
}

pub fn track(request: &mut Request) -> Result<Option<EditReceipt>, String> {
    if !matches!(
        request.operation.as_str(),
        "add" | "archive-add" | "archive-move" | "archive-copy" | "delete" | "rename" | "comment-set"
    ) {
        return Ok(None);
    }
    let mut sessions = registry().lock().map_err(|_| "内层归档会话不可用")?;
    if sessions.is_empty() {
        return Ok(None);
    }
    let path = request_key(&request.path);
    if let Some((&id, _)) = sessions.iter().find(|(_, session)| session.path_key == path) {
        // Projection from an archive can contain data decoded using another
        // password context, even if the destination itself has no password.
        let sensitive = request.options.password.is_some() || request.source_archive.is_some();
        arm_recovery(&mut sessions, id, sensitive)?;
    }
    if sessions
        .values()
        .any(|s| key(&s.parent) == path || s.path_key == path)
    {
        Ok(Some(EditReceipt::capture(&mut request.options)))
    } else {
        Ok(None)
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
    crate::verification::revoke();
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
        let receipt = track(&mut request).unwrap().unwrap();
        operations::execute_request(request).unwrap();
        assert!(committed(path, &receipt.take().unwrap()).is_none());
    }
    fn names(path: &str) -> Vec<String> {
        operations::list_archive(path, &Default::default())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    #[test]
    fn nested_edit_and_clean_discard_work_when_the_recovery_index_is_unreadable() {
        let _registry = crate::jobs::test_registry_guard();
        cleanup();
        let fixture = tempfile::tempdir().unwrap();
        let text = fixture.path().join("before.txt");
        let inner = fixture.path().join("inner.zip");
        let outer = fixture.path().join("outer.zip");
        std::fs::write(&text, "unchanged text").unwrap();
        for (input, output) in [(&text, &inner), (&inner, &outer)] {
            engine::create(output.to_str().unwrap(), &[input.display().to_string()], "zip", &Default::default()).unwrap();
        }
        let index = edit_copies::root().unwrap().join("index.json");
        let prior = std::fs::read(&index).ok();
        std::fs::write(&index, "bad index").unwrap();
        let tab = open(outer.to_str().unwrap(), "inner.zip");
        let mut request = Request {
            operation: "rename".into(),
            path: tab.path.clone(),
            inputs: vec!["before.txt".into(), "after.txt".into()],
            ..Default::default()
        };
        let receipt = track(&mut request).unwrap().unwrap();
        operations::execute_request(request).unwrap();
        let warning = committed(&tab.path, &receipt.take().unwrap()).unwrap();
        assert!(warning.contains("跨会话恢复登记未完成"));
        assert_eq!(names(&tab.path), vec!["after.txt"]);
        assert!(close_sessions(vec![tab.id], false).is_err());
        close_sessions(vec![tab.id], true).unwrap();
        assert!(!std::path::Path::new(&tab.path).exists());
        assert_eq!(std::fs::read(&index).unwrap(), b"bad index");
        if let Some(previous) = prior {
            std::fs::write(index, previous).unwrap();
        } else {
            std::fs::remove_file(index).unwrap();
        }
    }

    #[test]
    fn nested_recovery_exports_the_latest_detached_parent_without_touching_the_outer() {
        let _registry = crate::jobs::test_registry_guard();
        cleanup();
        let temp = tempfile::tempdir().unwrap();
        let text = temp.path().join("before.txt");
        let inner = temp.path().join("inner.zip");
        let middle = temp.path().join("middle.zip");
        let outer = temp.path().join("outer.zip");
        std::fs::write(&text, "my text").unwrap();
        for (input, output) in [(&text, &inner), (&inner, &middle), (&middle, &outer)] {
            engine::create(output.to_str().unwrap(), &[input.display().to_string()], "zip", &Default::default()).unwrap();
        }
        let middle = open(outer.to_str().unwrap(), "middle.zip");
        let inner = open(&middle.path, "inner.zip");
        rename(&inner.path, "before.txt", "after.txt");
        assert!(!edit_copies::retained_copy_records().unwrap().iter().any(|r| r.path == inner.path));
        // A separate ordinary writer changes the outer file between the edit
        // and explicit save; the inner is copied into the middle before the conflict.
        std::fs::write(&outer, "independent outer contents").unwrap();
        assert!(save_job(&inner.path, &JobControl::new()).is_err());
        assert!(!session_views().iter().find(|s| s.id == inner.id).unwrap().dirty);
        assert!(session_views().iter().find(|s| s.id == middle.id).unwrap().dirty);
        let detached = {
            let mut sessions = registry().lock().unwrap();
            let detached = std::mem::take(&mut *sessions);
            update_dirty(&sessions);
            detached
        };
        let recovered = edit_copies::retained_copy_records().unwrap().into_iter()
            .filter(|record| record.path == middle.path || record.path == inner.path).collect::<Vec<_>>();
        assert_eq!(recovered.len(), 1);
        let record = &recovered[0];
        assert_eq!(record.path, middle.path);
        let nested = record.nested.as_ref().unwrap();
        assert_eq!(key(&nested.outer), key(outer.to_str().unwrap()));
        assert_eq!(nested.members, vec!["middle.zip"]);
        let picked = serde_json::to_string(record).unwrap();
        assert!(edit_copies::save_copy(&picked, outer.to_str().unwrap()).is_err());
        let destination = temp.path().join("saved.zip");
        edit_copies::save_copy(&picked, destination.to_str().unwrap()).unwrap();
        assert!(edit_copies::save_copy(&picked, temp.path().join("other.zip").to_str().unwrap()).is_err());
        let readback = NestedArchive::open(destination.to_str().unwrap(), "inner.zip", &Default::default()).unwrap();
        assert_eq!(names(readback.path().to_str().unwrap()), vec!["after.txt"]);
        assert_eq!(std::fs::read(&outer).unwrap(), b"independent outer contents");
        assert_eq!(std::fs::read(&text).unwrap(), b"my text");
        drop(detached);
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
        let _registry = crate::jobs::test_registry_guard();
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
        assert_eq!(cleanup_if_idle(), Some(false));
        assert!(std::path::Path::new(&a.path).exists());
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
