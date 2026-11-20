//! GUI-owned deep sessions survive ordinary job eviction and password dialogs.
use crate::{
    desktop::ArchiveRecycleGuard,
    jobs::{commit_recycle, confirm_single_volume},
};
use obz_core::{
    engine::{deep_extract::Session, JobControl},
    operations::{OperationResponse, Request},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
struct DesktopSession {
    engine: Session,
    recycle: Option<Result<BTreeMap<PathBuf, ArchiveRecycleGuard>, String>>,
    recycle_inputs: Vec<String>,
    codepage: Option<u32>,
    closed: bool,
}
type Sessions = Mutex<BTreeMap<u64, Arc<Mutex<DesktopSession>>>>;
static SESSIONS: OnceLock<Sessions> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn sessions() -> &'static Sessions {
    SESSIONS.get_or_init(Default::default)
}

fn acquire_sources(request: &Request) -> Result<BTreeMap<PathBuf, ArchiveRecycleGuard>, String> {
    if !request.options.selected.is_empty() || request.options.overwrite == "skip" {
        return Err("自动回收要求完整解压且没有跳过项目".into());
    }
    let mut guards = BTreeMap::new();
    for input in &request.inputs {
        crate::nested::validate_request(
            &Request {
                path: input.clone(),
                ..Request::default()
            },
            true,
        )?;
        let mut key = PathBuf::from(input);
        if !key.is_absolute() {
            key = std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(key);
        }
        let guard =
            ArchiveRecycleGuard::acquire_deep(input).map_err(|e| format!("{input}：{e}"))?;
        if guards.insert(key, guard).is_some() {
            return Err("输入包含重复的压缩包路径，原压缩包已保留".into());
        }
    }
    Ok(guards)
}

fn recycle_completed(
    engine: &Session,
    guards: BTreeMap<PathBuf, ArchiveRecycleGuard>,
    codepage: Option<u32>,
    control: &JobControl,
) -> Result<(Vec<String>, Vec<String>, Option<String>), String> {
    let confirm = engine.for_each_recyclable_root(|path, passwords| {
        let file = path.to_string_lossy();
        let guard = guards.get(path).ok_or("无法确认解压来源与原压缩包一致")?;
        guard.unchanged().map_err(|e| format!("{file}：{e}"))?;
        let mut request = Request {
            path: file.to_string(),
            ..Request::default()
        };
        crate::nested::validate_request(&request, true)?;
        request.options.codepage = codepage;
        let result = if passwords.is_empty() {
            confirm_single_volume(&request, control)
        } else {
            let mut result = Err("引擎未确认此压缩包为单卷".into());
            for password in passwords {
                if control.cancelled.load(Ordering::SeqCst) {
                    return Err("任务已取消，原压缩包已保留".into());
                }
                request.options.password = Some(password.clone());
                result = confirm_single_volume(&request, control);
                if result.is_ok() {
                    break;
                }
            }
            result
        };
        result.map_err(|e| format!("{file}：{e}"))
    });
    confirm?;
    for (path, guard) in &guards {
        guard
            .unchanged()
            .map_err(|e| format!("{}：{e}", path.display()))?;
    }
    commit_recycle(control)?;
    let mut recycled = Vec::new();
    let mut retained = Vec::new();
    let mut warnings = Vec::new();
    for (path, guard) in guards {
        match guard.recycle() {
            Ok(()) => recycled.push(path.to_string_lossy().into_owned()),
            Err(error) => {
                warnings.push(format!("{}：{error}", path.display()));
                retained.push(path.to_string_lossy().into_owned());
            }
        }
    }
    Ok((
        recycled,
        retained,
        (!warnings.is_empty()).then(|| warnings.join("\n")),
    ))
}

pub fn execute(
    mut request: Request,
    recycle: bool,
    control: &JobControl,
) -> Result<OperationResponse, String> {
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
        let recycle = recycle.then(|| acquire_sources(&request));
        let engine = Session::start(
            request.inputs.clone(),
            &request.dest,
            request.smart,
            request.options.clone(),
            request.deep.clone().unwrap_or_default(),
        )?;
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let session = Arc::new(Mutex::new(DesktopSession {
            engine,
            recycle,
            recycle_inputs: request.inputs,
            codepage: request.options.codepage,
            closed: false,
        }));
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
    if state.closed {
        return Err("此会话已结束".into());
    }
    if request.operation == "deep-discard" {
        state.closed = true;
        state.recycle.take();
        let mut result = serde_json::to_value(state.engine.report()).map_err(|e| e.to_string())?;
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
            state.engine.resume(
                node,
                request.deep_skip,
                request.options.password.take(),
                request.deep,
            )?;
        }
        state.engine.retry_publication(request.options.overwrite)?;
    }
    let mut report = match state.engine.drive(control) {
        Ok(report) => report,
        Err(error) => {
            let mut report = state.engine.report();
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
    let finished = report.finished;
    let cancelled = control.cancelled.load(Ordering::SeqCst);
    if state.recycle.is_some() {
        if cancelled {
            state.recycle = Some(Err("任务曾取消，原压缩包已保留".into()));
        } else if report.partial && !finished && !report.outputs.is_empty() {
            // Source locks do not protect output files after a partial publication
            // returns control to the user. Keep the originals across later retries.
            state.recycle = Some(Err("此前已部分保存结果，原压缩包已保留".into()));
        }
    }
    let mut recycled = Vec::new();
    let mut retained = Vec::new();
    let mut recycle_error = None;
    if finished {
        state.closed = true;
        if let Some(recycle) = state.recycle.take() {
            match match recycle {
                Ok(guards) if !report.partial && !cancelled => {
                    recycle_completed(&state.engine, guards, state.codepage, control)
                }
                Ok(_) => Err("任务未完整解压，原压缩包已保留".into()),
                Err(error) => Err(error),
            } {
                Ok(outcome) => (recycled, retained, recycle_error) = outcome,
                Err(error) => {
                    retained = std::mem::take(&mut state.recycle_inputs);
                    recycle_error = Some(error);
                }
            };
            if !recycled.is_empty() {
                report.summary = format!(
                    "深度解压已保存 {} 个文件，{} 个原压缩包已移入回收站。",
                    report.files,
                    recycled.len()
                );
            }
            if let Some(error) = recycle_error.as_deref() {
                report
                    .summary
                    .push_str(&format!("\n{} 个原压缩包已保留：{error}", retained.len()));
            }
        }
    }
    if !report.protected_archives.is_empty() {
        report.summary.push_str(&format!(
            " {} 个应用或文档容器已保持完整。",
            report.protected_archives.len()
        ));
    }
    let mut result = serde_json::to_value(report).map_err(|e| e.to_string())?;
    result["deep_session"] = Value::from(id);
    result["cancelled"] = control.cancelled.load(Ordering::SeqCst).into();
    result["recycled_sources"] = recycled.len().into();
    result["recycled_originals"] = serde_json::to_value(recycled).map_err(|e| e.to_string())?;
    result["retained_originals"] = serde_json::to_value(retained).map_err(|e| e.to_string())?;
    if let Some(error) = recycle_error {
        result["recycle_error"] = error.into();
    }
    drop(state);
    if finished {
        sessions()
            .lock()
            .map_err(|_| "深度解压会话不可用")?
            .remove(&id);
    }
    Ok(OperationResponse::Json(result))
}
