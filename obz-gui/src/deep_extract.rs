//! GUI-owned deep sessions survive ordinary job eviction and password dialogs.
use obz_core::{
    engine::{deep_extract::Session, JobControl},
    operations::{OperationResponse, Request},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
static SESSIONS: OnceLock<Mutex<BTreeMap<u64, Arc<Mutex<Session>>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn sessions() -> &'static Mutex<BTreeMap<u64, Arc<Mutex<Session>>>> {
    SESSIONS.get_or_init(Default::default)
}
pub fn execute(
    mut request: Request,
    recycle: bool,
    control: &JobControl,
) -> Result<OperationResponse, String> {
    if recycle {
        return Err("深度解压会保留原压缩包，不能同时自动移入回收站".into());
    }
    if request.options.preserve_input_atime
        || request.options.update_mode != obz_core::engine::UpdateMode::Add
    {
        return Err("深度解压不支持这些添加文件选项".into());
    }
    let (id, session) = if request.operation == "deep-extract" {
        let mut registry = sessions().lock().map_err(|_| "深度解压会话不可用")?;
        if registry.len() >= 4 {
            return Err("已有四个待处理的深度解压会话，请先完成或结束其中一个".into());
        }
        if request.inputs.is_empty() && !request.path.is_empty() {
            request.inputs.push(request.path.clone());
        }
        let session = Session::start(
            request.inputs.clone(),
            &request.dest,
            request.smart,
            request.options.clone(),
            request.deep.clone().unwrap_or_default(),
        )?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let session = Arc::new(Mutex::new(session));
        registry.insert(id, session.clone());
        (id, session)
    } else {
        let id = request.deep_session.ok_or("缺少深度解压会话")?;
        let session = sessions()
            .lock()
            .map_err(|_| "深度解压会话不可用")?
            .get(&id)
            .cloned()
            .ok_or("此会话已结束；已保留的恢复目录仍可直接打开")?;
        (id, session)
    };
    let mut state = session.lock().map_err(|_| "深度解压会话不可用")?;
    if request.operation == "deep-discard" {
        let mut result = serde_json::to_value(state.report()).map_err(|e| e.to_string())?;
        result["summary"] = "会话已结束；原压缩包和恢复目录已保留。".into();
        result["session_closed"] = true.into();
        result["deep_session"] = id.into();
        drop(state);
        sessions()
            .lock()
            .map_err(|_| "深度解压会话不可用")?
            .remove(&id);
        return Ok(OperationResponse::Json(result));
    }
    if request.operation == "deep-resume" {
        if !["error", "overwrite", "skip", "rename"].contains(&request.options.overwrite.as_str()) {
            return Err("同名文件处理方式无效".into());
        }
        if let Some(node) = request.deep_node {
            state.resume(
                node,
                request.deep_skip,
                request.options.password.take(),
                request.deep,
            )?;
        }
        state.retry_publication(request.options.overwrite)?;
    }
    let mut report = match state.drive(control) {
        Ok(report) => report,
        Err(error) => {
            let mut report = state.report();
            report.partial = true;
            report.summary = format!(
                "{}；原压缩包和已校验结果已保留，可从恢复目录继续。",
                if control.cancelled.load(Ordering::SeqCst) {
                    "深度解压已取消".to_owned()
                } else {
                    error
                }
            );
            report
        }
    };
    if !report.protected_archives.is_empty() {
        report.summary.push_str(&format!(
            " {} 个应用或文档容器已保持完整。",
            report.protected_archives.len()
        ));
    }
    let finished = report.finished;
    let mut result = serde_json::to_value(report).map_err(|e| e.to_string())?;
    result["deep_session"] = Value::from(id);
    result["cancelled"] = control.cancelled.load(Ordering::SeqCst).into();
    drop(state);
    if finished {
        sessions()
            .lock()
            .map_err(|_| "深度解压会话不可用")?
            .remove(&id);
    }
    Ok(OperationResponse::Json(result))
}
