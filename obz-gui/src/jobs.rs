use obz_core::{engine::{JobControl, Progress}, operations::{self, OperationResponse, Request}};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, atomic::{AtomicU64, Ordering}}};
use std::{io::{Read, Seek, SeekFrom}, fs::File, process::{Command, Stdio}, time::{Duration, Instant}};

struct Job { control: JobControl, state: Mutex<JobState>, finished: std::sync::Condvar }
struct JobState { status: String, result: Option<Arc<OperationResponse>>, error: Option<String> }
#[derive(Serialize)]
pub struct Snapshot { id: u64, status: String, progress: Progress, result: Option<Arc<OperationResponse>>, error: Option<String> }
static JOBS: OnceLock<Mutex<BTreeMap<u64, Arc<Job>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn registry() -> &'static Mutex<BTreeMap<u64, Arc<Job>>> { JOBS.get_or_init(Default::default) }

// Integration tests share the real two-slot registry. Hold this across the
// entire test (including worker completion), not inside start_operation: the
// drag test intentionally runs two jobs together to verify isolated cancellation.
#[cfg(test)]
pub(crate) fn test_registry_guard() -> std::sync::MutexGuard<'static, ()> {
    static TEST_REGISTRY: Mutex<()> = Mutex::new(());
    TEST_REGISTRY.lock().unwrap()
}

pub fn running() -> bool { registry().lock().map(|jobs|jobs.values().any(|j|j.state.lock().map(|s|s.status=="running").unwrap_or(true))).unwrap_or(true) }
pub fn cancel_all() { if let Ok(jobs)=registry().lock(){for job in jobs.values(){
    let phase=job.control.progress.lock().unwrap_or_else(|e|e.into_inner());
    if phase.phase!="recycling" {job.control.cancel();}
}} crate::verification::revoke(); }

fn check_cancel(control: &JobControl) -> Result<(), String> {
    if control.cancelled.load(Ordering::SeqCst) {Err("operation cancelled".into())} else {Ok(())}
}
fn validate_recycle_request(request: &Request) -> Result<(), String> {
    if request.operation != "extract" || !request.options.selected.is_empty()
        || request.options.overwrite == "skip" {
        return Err("自动回收仅支持全部文件成功解压，不能使用选定范围或跳过策略".into());
    }
    Ok(())
}
fn validate_archive_header(header: &str) -> Result<(), String> {
    let mut types = Vec::new();
    for line in header.lines() {
        let Some((key,value)) = line.split_once(" = ") else {continue};
        let key=key.trim().to_ascii_lowercase(); let value=value.trim().to_ascii_lowercase();
        if key=="type" {types.push(value.clone());}
        if (key=="multivolume" && value!="-")
            || (key=="volumes" && value!="1")
            || ((key=="volume index" || key=="volumeindex") && value!="0")
            || (key=="type" && value=="split") {
            return Err("引擎检测到分卷压缩包，不能自动回收".into());
        }
    }
    if types.is_empty() || types.iter().any(|t| !["zip","7z","tar","gzip","bzip2","xz"].contains(&t.as_str())) {
        return Err("引擎未确认受支持的单卷格式，请使用普通解压".into());
    }
    Ok(())
}
fn capture_prefix(mut reader: impl Read, maximum: usize) -> Vec<u8> {
    let mut result=Vec::new(); let mut buffer=[0u8;8192];
    loop {match reader.read(&mut buffer) {Ok(0)|Err(_)=>break,Ok(n)=>{
        let keep=n.min(maximum.saturating_sub(result.len()));result.extend_from_slice(&buffer[..keep]);
    }}}
    result
}
pub(crate) fn confirm_single_volume(request: &Request, control: &JobControl) -> Result<(), String> {
    let _policy = obz_core::machine_policy::scope();
    use std::os::windows::process::CommandExt;
    check_cancel(control)?;
    if let Some(password)=&request.options.password {obz_core::engine_password::validate(password)?;}
    let mut command=Command::new(obz_core::engine::executable()?);
    command.args(["l","-slt","-sccUTF-8","-y","-spd"]).creation_flags(0x08000000)
        .stdin(if request.options.password.is_some(){Stdio::piped()}else{Stdio::null()})
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cp)=request.options.codepage {command.arg(format!("-mcp={cp}"));}
    let mut child=command.arg("--").arg(&request.path).spawn().map_err(|e|e.to_string())?;
    let password_input=match request.options.password.as_deref() {
        Some(password)=>match obz_core::engine_password::PasswordInput::spawn(&mut child,password) {
            Ok(input)=>Some(input),
            Err(error)=>{let _=child.kill();let _=child.wait();return Err(error);}
        },
        None=>None,
    };
    let stdout=child.stdout.take().ok_or("无法读取归档信息")?;
    let stderr=child.stderr.take().ok_or("无法读取引擎状态")?;
    let out=std::thread::spawn(move||capture_prefix(stdout,65536));
    let err=std::thread::spawn(move||capture_prefix(stderr,4096));
    let start=Instant::now();
    let status=loop {
        if control.cancelled.load(Ordering::SeqCst) || start.elapsed()>Duration::from_secs(60) {
            let _=child.kill();let _=child.wait();let _=out.join();let _=err.join();
            if let Some(input)=password_input {let _=input.finish();}
            check_cancel(control)?;
            return Err("确认单卷状态超时，原压缩包保持不变".into());
        }
        match child.try_wait() {
            Ok(Some(status))=>break status,
            Ok(None)=>if let Err(e)=obz_core::engine::wait_for_process(&child,Duration::from_millis(40)) {
                let _=child.kill();let _=child.wait();let _=out.join();let _=err.join();
                if let Some(input)=password_input {let _=input.finish();}
                return Err(e.to_string());
            },
            Err(e)=>{let _=child.kill();let _=child.wait();let _=out.join();let _=err.join();
                if let Some(input)=password_input {let _=input.finish();}
                return Err(e.to_string());}
        }
    };
    if let Some(input)=password_input {input.finish()?;}
    let bytes=out.join().map_err(|_|"归档信息读取失败")?;let _=err.join();
    if !status.success(){return Err("无法确认压缩包完整且为单卷，未解压或回收，请使用普通解压查看原因".into());}
    let text=String::from_utf8_lossy(&bytes);
    let (header,_)=text.split_once("----------").ok_or("归档信息不完整，不能自动回收")?;
    validate_archive_header(header)?;
    if header.lines().any(|line| line.trim().eq_ignore_ascii_case("Type = zip"))
        && pinned_zip_entries(&File::open(&request.path).map_err(|error| error.to_string())?)?.is_none()
    {
        return Err("无法在原压缩包上验证 ZIP 单卷结束记录，原压缩包已保留".into());
    }
    check_cancel(control)
}

// Read the end record from the same pinned file held through publication.
pub(crate) fn pinned_zip_entries(mut file: &File) -> Result<Option<usize>, String> {
    let mut signature = [0; 4];
    file.seek(SeekFrom::Start(0)).and_then(|_| file.read_exact(&mut signature))
        .map_err(|e| e.to_string())?;
    if !matches!(&signature, b"PK\x03\x04" | b"PK\x05\x06") {
        return Ok(None);
    }
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    // Include the ZIP64 locator even with a maximum-length classic comment.
    let tail_length = length.min(65_577);
    file.seek(SeekFrom::End(-(tail_length as i64))).map_err(|e| e.to_string())?;
    let mut bytes = vec![0; tail_length as usize];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let invalid = || "ZIP 单卷结束记录无效，原压缩包已保留".to_owned();
    let u16_at = |bytes: &[u8], offset| u16::from_le_bytes(bytes[offset..offset+2].try_into().unwrap());
    let u32_at = |bytes: &[u8], offset| u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap());
    let u64_at = |bytes: &[u8], offset| u64::from_le_bytes(bytes[offset..offset+8].try_into().unwrap());
    let mut ends = bytes.windows(22).enumerate().rev().filter_map(|(position, record)| {
        (record.starts_with(b"PK\x05\x06")
            && position + 22 + usize::from(u16_at(record, 20)) == bytes.len())
            .then_some(position)
    });
    let pos = ends.next().ok_or_else(invalid)?;
    if ends.next().is_some() {
        return Err(invalid());
    }
    let classic = &bytes[pos..];
    let count = u16_at(classic, 10);
    if u16_at(classic, 4) != 0 || u16_at(classic, 6) != 0 || u16_at(classic, 8) != count {
        return Err(invalid());
    }
    let classic_offset = length - tail_length + pos as u64;
    let size = u32_at(classic, 12);
    let offset = u32_at(classic, 16);
    if pos >= 20 && bytes[pos-20..pos].starts_with(b"PK\x06\x07") {
        let locator = &bytes[pos-20..pos];
        let record_offset = u64_at(locator, 8);
        let locator_offset = classic_offset.checked_sub(20).ok_or_else(invalid)?;
        if u32_at(locator, 4) != 0 || u32_at(locator, 16) != 1
            || record_offset.checked_add(56).is_none_or(|end| end > locator_offset)
        {
            return Err(invalid());
        }
        let mut record = [0; 56];
        file.seek(SeekFrom::Start(record_offset)).and_then(|_| file.read_exact(&mut record))
            .map_err(|e| e.to_string())?;
        let total = u64_at(&record, 32);
        let zip64_size = u64_at(&record, 40);
        let zip64_offset = u64_at(&record, 48);
        if !record.starts_with(b"PK\x06\x06")
            || u64_at(&record, 4) < 44
            || record_offset.checked_add(12).and_then(|start| start.checked_add(u64_at(&record, 4))) != Some(locator_offset)
            || u32_at(&record, 16) != 0 || u32_at(&record, 20) != 0
            || u64_at(&record, 24) != total
            || zip64_offset.checked_add(zip64_size) != Some(record_offset)
            || (count != u16::MAX && u64::from(count) != total)
            || (size != u32::MAX && u64::from(size) != zip64_size)
            || (offset != u32::MAX && u64::from(offset) != zip64_offset)
        {
            return Err(invalid());
        }
        return usize::try_from(total).map(Some).map_err(|_| invalid());
    }
    if count == u16::MAX || u64::from(offset) + u64::from(size) != classic_offset {
        return Err(invalid());
    }
    Ok(Some(usize::from(count)))
}
pub(crate) fn commit_recycle(control: &JobControl) -> Result<(), String> {
    let mut phase=control.progress.lock().unwrap_or_else(|e|e.into_inner());
    check_cancel(control)?;
    *phase=Progress{phase:"recycling".into(),percent:None,message:"解压已完成，正在将原压缩包移入回收站（此阶段不可取消）".into(),..Progress::default()};
    Ok(())
}
fn verify_completed_files(result: &Value, expected: u64, control: &JobControl) -> Result<(),String> {
    check_cancel(control)?;
    if expected == 0 || result.get("files").and_then(Value::as_u64)!=Some(expected) {
        return Err("实际解压文件数不匹配，原压缩包保持不变".into());
    }
    Ok(())
}
fn execute_job(mut request: Request, recycle_after: bool, control: &JobControl) -> Result<OperationResponse,String> {
    if let Some(id)=request.password_id.take() {
        if request.options.password.is_some(){return Err("choose password or password_id, not both".into());}
        request.options.password=Some(obz_core::services::Vault::open(operations::vault_path()?)?.password_book_password(&id)?);
    }
    if request.operation == "external-edit" {return crate::nested::edit_job(request.path,request.name,request.options,request.label);}
    if matches!(request.operation.as_str(),"deep-extract"|"deep-resume"|"deep-discard"){return crate::deep_extract::execute(request,recycle_after,control);}
    if request.operation == "external-save" {return crate::nested::external_save_job(&request.path,control);}
    if request.operation == "nested-open" { return crate::nested::open_job(request.path,request.name,request.options); }
    if request.operation == "nested-save" { return crate::nested::save_job(&request.path, control); }
    crate::nested::validate_request(&request, recycle_after)?;
    let edit_path = request.path.clone();
    let receipt = crate::nested::track(&mut request);
    let password_use = request.options.extraction_password_use.clone()
        .and_then(|evidence| request.options.password.clone().map(|password| (evidence, password)));
    let mut result = execute_archive_job(request, recycle_after, control);
    if let (Some((evidence, password)), Ok(OperationResponse::Json(output))) = (password_use, &mut result) {
        let published = output.get("files").and_then(Value::as_u64).unwrap_or(0) as usize;
        if !control.cancelled.load(Ordering::SeqCst) && evidence.verified(published) {
            if let Err(error) = crate::password_book::remember([(edit_path.as_str(), password.as_str())]) {
                output["password_book_error"] = Value::String(error);
            }
        }
    }
    if let Some(revision) = receipt.and_then(|r|r.take()) { crate::nested::committed(&edit_path, &revision); }
    result
}

fn execute_archive_job(request: Request, recycle_after: bool, control: &JobControl) -> Result<OperationResponse,String> {
    execute_archive_job_with_recycler(request, recycle_after, control, |guard| {
        guard.recycle(|| commit_recycle(control))
    })
}
fn execute_archive_job_with_recycler(
    mut request: Request, recycle_after: bool, control: &JobControl,
    recycle_source: impl FnOnce(crate::desktop::ArchiveRecycleGuard) -> Result<(), String>,
) -> Result<OperationResponse,String> {
    if !recycle_after {return operations::execute_request(request);}
    check_cancel(control)?;
    if request.operation != "extract" {
        return operations::execute_request(request);
    }
    let inventory = Arc::new(OnceLock::new());
    request.options.extraction_inventory = Some(inventory.clone());
    let proof = (|| {
        validate_recycle_request(&request)?;
        let guard = crate::desktop::ArchiveRecycleGuard::acquire(&request.path)?;
        let zip_entries = pinned_zip_entries(guard.file())?;
        if zip_entries.is_none() && !obz_core::engine::standalone_7z_source(guard.file())? {
            confirm_single_volume(&request, control)?;
        }
        Ok::<_, String>((guard, zip_entries))
    })();
    check_cancel(control)?;
    let mut result = operations::execute(request)?;
    let recycle = (|| {
        let (guard, zip_entries) = proof?;
        let completed = inventory.get().ok_or("未取得完整解压的文件清单，原压缩包已保留")?;
        if zip_entries.is_some_and(|total| completed.entries != total) {
            return Err("ZIP 目录与结束记录的项目数不匹配，原压缩包已保留".into());
        }
        verify_completed_files(&result, completed.files as u64, control)?;
        recycle_source(guard)
    })();
    result["recycled"]=Value::Bool(recycle.is_ok());
    if let Err(error)=recycle {result["recycle_error"]=Value::String(error);}
    Ok(OperationResponse::Json(result))
}

pub fn start_operation(request: Request, recycle_after: Option<bool>) -> Result<u64, String> {
    start_controlled(request, recycle_after.unwrap_or(false), JobControl::new(), |_| {})
}

pub(super) fn start_controlled(mut request: Request, recycle_after: bool, control: JobControl,
    finished: impl FnOnce(bool) + Send + 'static) -> Result<u64, String> {
    let mut jobs = registry().lock().map_err(|_|"job registry unavailable")?;
    let running = jobs.values().filter(|j|j.state.lock().map(|s|s.status=="running").unwrap_or(true)).count();
    if running >= 2 { return Err("已有两个任务运行，请等待完成后重试".into()); }
    while jobs.len() >= 12 {
        let id = jobs.iter().find_map(|(id,j)|j.state.lock().ok().filter(|s|s.status!="running").map(|_|*id));
        if let Some(id)=id {jobs.remove(&id);} else {break;}
    }
    request.options.control=Some(control.clone());
    let verification=crate::verification::prepare(&mut request,recycle_after);
    let job=Arc::new(Job{control,state:Mutex::new(JobState{status:"running".into(),result:None,error:None}),finished:std::sync::Condvar::new()});
    let id=NEXT.fetch_add(1,Ordering::Relaxed); jobs.insert(id,job.clone());
    let worker = std::thread::Builder::new().spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||execute_job(request,recycle_after,&job.control)))
            .unwrap_or_else(|_|Err("归档任务异常结束".into()));
        if let Some(pending)=verification {pending.finish(result.is_ok() && !job.control.cancelled.load(Ordering::SeqCst));}
        finished(result.is_ok());
        let mut state=job.state.lock().unwrap_or_else(|e|e.into_inner());
        match result {
            Ok(value)=>{state.status=if job.control.cancelled.load(Ordering::SeqCst) && matches!(&value,OperationResponse::Json(v) if v.get("deep_session").is_some()){"cancelled"}else{"complete"}.into();state.result=Some(Arc::new(value));},
            Err(e)=>{state.status=if job.control.cancelled.load(Ordering::Relaxed){"cancelled"}else{"failed"}.into();state.error=Some(e);}
        }
        job.finished.notify_all();
    });
    if let Err(error) = worker { jobs.remove(&id); return Err(error.to_string()); }
    Ok(id)
}

#[tauri::command]
pub fn operation_status(id:u64)->Result<Snapshot,String>{
    let job=registry().lock().map_err(|_|"job registry unavailable")?.get(&id).cloned().ok_or("任务不存在或已释放")?;
    let state=job.state.lock().map_err(|_|"job state unavailable")?;
    Ok(Snapshot{id,status:state.status.clone(),progress:job.control.snapshot(),result:state.result.clone(),error:state.error.clone()})
}

fn wait_snapshot(job: &Job, id: u64, timeout: Duration) -> Result<Snapshot,String> {
    let state=job.state.lock().map_err(|_|"job state unavailable")?;
    // Predicate and notification share the state lock: completion before or
    // during registration is never lost. Timeout only refreshes progress.
    let (state,_)=job.finished.wait_timeout_while(state,timeout,|s|s.status=="running")
        .map_err(|_|"job state unavailable")?;
    Ok(Snapshot{id,status:state.status.clone(),progress:job.control.snapshot(),result:state.result.clone(),error:state.error.clone()})
}

#[tauri::command]
pub async fn wait_operation(id: u64, defer_listing: Option<bool>) -> Result<Snapshot, String> {
    let job=registry().lock().map_err(|_|"job registry unavailable")?.get(&id).cloned().ok_or("任务不存在或已释放")?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut snapshot = wait_snapshot(&job, id, Duration::from_millis(150))?;
        if defer_listing == Some(true)
            && matches!(snapshot.result.as_deref(), Some(OperationResponse::Listing { .. }))
        {
            snapshot.result = None;
        }
        Ok(snapshot)
    })
    .await.map_err(|e|e.to_string())?
}

#[tauri::command]
pub async fn take_archive_listing(id: u64) -> Result<tauri::ipc::Response, String> {
    #[derive(Serialize)]
    struct Payload<'a> {
        rows: crate::archive_listing::Listing<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        header_encrypted: Option<bool>,
    }

    let job=registry().lock().map_err(|_|"job registry unavailable")?.get(&id).cloned().ok_or("任务不存在或已释放")?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = {
            let mut state = job.state.lock().map_err(|_| "job state unavailable")?;
            if !matches!(state.result.as_deref(), Some(OperationResponse::Listing { .. })) {
                return Err("压缩包目录尚未就绪或已经读取".into());
            }
            state.result.take().unwrap()
        };
        match result.as_ref() {
            OperationResponse::Listing { entries, header_encrypted } => {
                crate::archive_listing::response(Payload { rows: crate::archive_listing::Listing(entries), header_encrypted: *header_encrypted })
            }
            OperationResponse::Json(_) => unreachable!(),
        }
    })
    .await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cancel_operation(id:u64)->Result<(),String>{
    let job=registry().lock().map_err(|_|"job registry unavailable")?.get(&id).cloned().ok_or("任务不存在")?;
    let phase=job.control.progress.lock().unwrap_or_else(|e|e.into_inner());
    if phase.phase=="recycling" {return Err("原包正在移入回收站，此阶段不可取消".into());}
    job.control.cancel(); crate::verification::revoke(); Ok(())
}

#[tauri::command]
pub fn forget_operation(id:u64)->Result<(),String>{
    let mut jobs=registry().lock().map_err(|_|"job registry unavailable")?;
    if let Some(job)=jobs.get(&id) {
        if job.state.lock().map_err(|_|"job state unavailable")?.status=="running" {return Err("运行中的任务不能释放".into());}
    }
    jobs.remove(&id);Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test]
    fn zip_creation_job_returns_the_new_archive_and_completed_progress() {
        let _registry = test_registry_guard();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("notes.txt"), b"desktop create").unwrap();
        for n in 0..2048 {
            std::fs::write(source.join(format!("entry-{n:04}.txt")), b"desktop source").unwrap();
        }
        let archive = temp.path().join("result.zip").display().to_string();
        let request = Request {
            operation: "create".into(), format: "zip".into(), path: archive.clone(),
            inputs: vec![source.display().to_string()], ..Default::default()
        };
        let id = start_operation(request, None).unwrap();
        let job = registry().lock().unwrap().get(&id).cloned().unwrap();
        let state = wait_snapshot(&job, id, Duration::from_secs(30)).unwrap();
        assert_eq!(state.status, "complete", "{:?}", state.error);
        assert_eq!(state.progress.percent, Some(100));
        assert_eq!(serde_json::to_value(&state).unwrap()["result"]["outputs"], serde_json::json!([archive]));
        let entry = obz_core::engine::preview(&archive, "source/notes.txt", &Default::default()).unwrap();
        assert_eq!(entry,b"desktop create");
        forget_operation(id).unwrap();
    }
    #[test] fn real_local_link_job_uses_typed_request_and_returns_created_path(){
        let _registry=test_registry_guard();let root=tempfile::tempdir().unwrap();let source=root.path().join("source");std::fs::write(&source,b"keep").unwrap();
        let request:Request=serde_json::from_value(serde_json::json!({"operation":"fs-create-link","path":source,"dest":root.path(),"name":"link","link_kind":"hard","options":{"overwrite":"error"}})).unwrap();
        let id=start_operation(request,None).unwrap();let job=registry().lock().unwrap().get(&id).cloned().unwrap();
        let state=wait_snapshot(&job,id,Duration::from_secs(10)).unwrap();assert_eq!(state.status,"complete","{:?}",state.error);
        assert_eq!(serde_json::to_value(state.result).unwrap()["path"],root.path().join("link").display().to_string());
        assert_eq!(std::fs::read(source).unwrap(),b"keep");forget_operation(id).unwrap();
    }
    #[test] fn real_ads_export_jobs_preserve_partial_cancellation_details(){
        let _registry=test_registry_guard();let root=tempfile::tempdir().unwrap();let source=root.path().join("source");let dest=root.path().join("out");std::fs::write(&source,b"base untouched").unwrap();std::fs::create_dir(&dest).unwrap();
        std::fs::write(format!("{}:first",source.display()),b"first").unwrap();let large=File::create(format!("{}:large",source.display())).unwrap();large.set_len(256*1024*1024).unwrap();drop(large);
        let listed=obz_core::operations::execute(serde_json::from_value(serde_json::json!({"operation":"fs-streams","path":source})).unwrap()).unwrap();
        let request:Request=serde_json::from_value(serde_json::json!({"operation":"fs-export-streams","path":source,"dest":dest,"stream_revision":listed["revision"],"stream_exports":[{"name":"first","size":5,"output_name":"first.txt"},{"name":"large","size":256*1024*1024,"output_name":"large.bin"}]})).unwrap();
        let control=JobControl::new();let observer=control.clone();let id=start_controlled(request,false,control,|_|{}).unwrap();let job=registry().lock().unwrap().get(&id).cloned().unwrap();
        let deadline=Instant::now()+Duration::from_secs(15);loop{let progress=observer.snapshot();if progress.message.contains("2 / 2"){observer.cancel();break;}assert!(Instant::now()<deadline,"second ADS never started");std::thread::sleep(Duration::from_millis(1));}
        let state=wait_snapshot(&job,id,Duration::from_secs(15)).unwrap();assert_eq!(state.status,"cancelled");let error=state.error.unwrap();assert!(error.contains("已导出 1 / 2"),"{error}");assert!(error.contains("first.txt"),"{error}");assert!(error.contains("operation cancelled"));
        assert_eq!(std::fs::read(dest.join("first.txt")).unwrap(),b"first");assert!(!dest.join("large.bin").exists());assert_eq!(std::fs::read_dir(&dest).unwrap().count(),1);assert_eq!(std::fs::read(&source).unwrap(),b"base untouched");forget_operation(id).unwrap();
    }
    #[test] fn real_existing_link_jobs_inspect_edit_and_clear_without_sync_bridge(){
        let _registry=test_registry_guard();let root=tempfile::tempdir().unwrap();let path=root.path().join("wsl-link");
        let run=|body:serde_json::Value|{
            let request:Request=serde_json::from_value(body).unwrap();let id=start_operation(request,None).unwrap();let job=registry().lock().unwrap().get(&id).cloned().unwrap();
            let state=wait_snapshot(&job,id,Duration::from_secs(10)).unwrap();assert_eq!(state.status,"complete","{:?}",state.error);let result=serde_json::to_value(state.result).unwrap();forget_operation(id).unwrap();result
        };
        run(serde_json::json!({"operation":"fs-create-link","dest":root.path(),"name":"wsl-link","link_kind":"wsl","wsl_target":"../old"}));
        let before=run(serde_json::json!({"operation":"fs-inspect-link","path":path}));assert_eq!(before["target"],"../old");assert!(before["receipt"]["file_id"].is_string());
        run(serde_json::json!({"operation":"fs-edit-link","path":path,"link_receipt":before["receipt"],"link_target":"/new/用户"}));
        let after=run(serde_json::json!({"operation":"fs-inspect-link","path":path}));assert_eq!(after["target"],"/new/用户");
        assert_eq!(run(serde_json::json!({"operation":"fs-clear-link-data","path":path,"link_receipt":after["receipt"]}))["removed"],true);
        assert!(std::fs::symlink_metadata(&path).unwrap().is_file());
    }
    #[test]
    fn real_listing_job_serializes_the_shared_entries_in_each_snapshot() {
        let _registry = test_registry_guard();
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("example.zip").display().to_string();
        let input = root.path().join("note.txt");
        std::fs::write(&input, b"contents").unwrap();
        obz_core::engine::create(&archive, &[input.display().to_string()], "zip", &Default::default()).unwrap();
        let request = || Request { operation: "list".into(), path: archive.clone(), ..Default::default() };
        let expected = operations::execute(request()).unwrap();
        let id = start_operation(request(), None).unwrap();
        let job = registry().lock().unwrap().get(&id).cloned().unwrap();
        let first = wait_snapshot(&job, id, Duration::from_secs(30)).unwrap();
        assert_eq!(first.status, "complete", "{:?}", first.error);
        assert!(matches!(first.result.as_deref(), Some(OperationResponse::Listing { entries, .. }) if entries.len() == 1));
        let second = operation_status(id).unwrap();
        assert!(Arc::ptr_eq(first.result.as_ref().unwrap(), second.result.as_ref().unwrap()));
        forget_operation(id).unwrap();
        assert_eq!(serde_json::to_vec(&first.result).unwrap(), serde_json::to_vec(&expected).unwrap());
        assert_eq!(serde_json::to_vec(&second.result).unwrap(), serde_json::to_vec(&expected).unwrap());
    }
    #[test]
    fn smart_zip_job_keeps_selected_layout_and_full_output() {
        let _registry = test_registry_guard();
        let root = tempfile::tempdir().unwrap();
        let wrapped = root.path().join("bundle");
        std::fs::create_dir(&wrapped).unwrap();
        std::fs::write(wrapped.join("selected.txt"), b"first").unwrap();
        std::fs::write(wrapped.join("other.txt"), b"second").unwrap();
        let archive = root.path().join("bundle.zip").display().to_string();
        obz_core::engine::create(&archive, &[wrapped.display().to_string()], "zip", &Default::default()).unwrap();
        for (name, selected, count) in [
            ("selected", vec!["bundle/selected.txt".to_owned()], 1),
            ("full", Vec::new(), 2),
        ] {
            let dest = root.path().join(name);
            let id = start_operation(Request {
                operation: "extract".into(),
                path: archive.clone(),
                dest: dest.display().to_string(),
                smart: true,
                options: obz_core::engine::EngineOptions { selected, ..Default::default() },
                ..Default::default()
            }, None).unwrap();
            let job = registry().lock().unwrap().get(&id).cloned().unwrap();
            let snapshot = wait_snapshot(&job, id, Duration::from_secs(30)).unwrap();
            assert_eq!(snapshot.status, "complete", "{:?}", snapshot.error);
            let Some(OperationResponse::Json(result)) = snapshot.result.as_deref() else { panic!("missing extraction result") };
            assert_eq!(result["files"].as_u64(), Some(count));
            assert_eq!(result["destination"].as_str(), dest.to_str());
            assert_eq!(std::fs::read(dest.join("bundle").join("selected.txt")).unwrap(), b"first");
            assert_eq!(dest.join("bundle").join("other.txt").exists(), count == 2);
            forget_operation(id).unwrap();
        }
    }

    #[test]
    fn completion_wait_handles_finished_and_inflight_jobs_without_lost_wakeup() {
        let job=Arc::new(Job{control:JobControl::new(),state:Mutex::new(JobState{status:"running".into(),result:None,error:None}),finished:std::sync::Condvar::new()});
        assert_eq!(wait_snapshot(&job,1,Duration::from_millis(1)).unwrap().status,"running");
        let worker=job.clone();
        let thread=std::thread::spawn(move||{
            std::thread::sleep(Duration::from_millis(20));
            let mut state=worker.state.lock().unwrap();state.status="complete".into();state.result=Some(Arc::new(OperationResponse::Json(serde_json::json!({"valid":true}))));
            worker.finished.notify_all();
        });
        let start=Instant::now();
        let state=wait_snapshot(&job,1,Duration::from_secs(5)).unwrap();
        assert_eq!(state.status,"complete");
        assert_eq!(serde_json::to_value(&state).unwrap()["result"],serde_json::json!({"valid":true}));
        assert!(start.elapsed()<Duration::from_secs(2));thread.join().unwrap();
        let start=Instant::now();
        let next=wait_snapshot(&job,1,Duration::from_secs(5)).unwrap();
        assert_eq!(next.status,"complete");
        assert!(Arc::ptr_eq(state.result.as_ref().unwrap(), next.result.as_ref().unwrap()));
        assert!(start.elapsed()<Duration::from_secs(2));
    }
    #[test] fn unknown_job_fails(){assert!(operation_status(u64::MAX).is_err());assert!(cancel_operation(u64::MAX).is_err());}
    #[test] fn background_failure_is_reported_and_released(){
        let _registry = test_registry_guard();
        let id=start_operation(Request{operation:"unknown".into(),..Default::default()},None).unwrap();
        for _ in 0..100 {let s=operation_status(id).unwrap();if s.status!="running"{assert_eq!(s.status,"failed");assert!(s.error.is_some());forget_operation(id).unwrap();return;}std::thread::sleep(std::time::Duration::from_millis(10));}
        panic!("job did not finish");
    }
    #[test] fn recycle_requires_complete_extraction_and_non_skip_policy() {
        let mut request=Request{operation:"extract".into(),..Default::default()};
        assert!(validate_recycle_request(&request).is_ok());
        request.options.selected.push("only-one.txt".into());
        assert!(validate_recycle_request(&request).is_err());
        request.options.selected.clear();request.options.overwrite="skip".into();
        assert!(validate_recycle_request(&request).is_err());
        request.options.overwrite="overwrite".into();request.operation="test".into();
        assert!(validate_recycle_request(&request).is_err());
    }
    #[test] fn recycle_checks_real_archive_header_instead_of_extension() {
        for header in ["Type = 7z\n", "Type = zip\n", "Type = gzip\nType = tar\n", "Type = xz\n"] {
            assert!(validate_archive_header(header).is_ok());
        }
        for header in ["", "Type = Split\nType = 7z\n", "Type = Rar\n", "Type = zip\nVolumes = 2\n", "Type = zip\nMultivolume = +\n", "Type = zip\nVolumeIndex = 1\n"] {
            assert!(validate_archive_header(header).is_err(),"{header}");
        }
    }
    #[test] fn recycle_header_check_uses_password_stdin_and_keeps_split_safety() {
        use obz_core::engine::{self,EngineOptions};
        let temp=tempfile::tempdir().unwrap();let root=temp.path();
        let input=root.join("input.txt");std::fs::write(&input,b"synthetic recycle header fixture").unwrap();
        let archive=root.join("encrypted.7z").display().to_string();
        let options=EngineOptions{password:Some(" header 中文 🔐 ".into()),..Default::default()};
        engine::create(&archive,&[input.display().to_string()],"7z",&options).unwrap();
        let mut request=Request{operation:"extract".into(),path:archive,options:options.clone(),..Default::default()};
        confirm_single_volume(&request,&JobControl::new()).unwrap();
        request.options.password=Some("wrong".into());
        assert!(confirm_single_volume(&request,&JobControl::new()).is_err());
        request.options.password=None;
        assert!(confirm_single_volume(&request,&JobControl::new()).is_err());
        let cancelled=JobControl::new();cancelled.cancel();
        assert_eq!(confirm_single_volume(&request,&cancelled).unwrap_err(),"operation cancelled");
        let split=root.join("split.7z").display().to_string();
        let split_options=EngineOptions{volume_bytes:Some(128),..options.clone()};
        let volumes=engine::create(&split,&[input.display().to_string()],"7z",&split_options).unwrap();
        request.path=volumes[0].clone();request.options=options;
        assert!(confirm_single_volume(&request,&JobControl::new()).is_err());
    }
    #[test] fn recycle_credits_only_complete_proven_outputs() {
        use obz_core::engine::{self, EngineOptions};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let input = root.join("member.txt");
        std::fs::write(&input, b"recycle completion test").unwrap();
        let path = root.join("archive.zip").display().to_string();
        engine::create(&path, &[input.display().to_string()], "zip", &EngineOptions::default()).unwrap();
        let original = std::fs::read(&path).unwrap();
        let run = |destination: &str, selected: Vec<String>, overwrite: &str, expected_recycle| {
            let control = JobControl::new();
            let request = Request {
                operation: "extract".into(), path: path.clone(), dest: root.join(destination).display().to_string(),
                options: EngineOptions { selected, overwrite: overwrite.into(), control: Some(control.clone()), ..Default::default() },
                ..Default::default()
            };
            let actual = std::cell::Cell::new(false);
            let result = execute_archive_job_with_recycler(request, true, &control, |guard| {
                guard.unchanged()?;
                actual.set(true);
                Err("test does not use the system recycle bin".into())
            }).unwrap();
            let OperationResponse::Json(result) = result else { panic!("extraction has no JSON report"); };
            assert_eq!(actual.get(), expected_recycle);
            assert_eq!(result["recycled"], false);
            assert!(result["recycle_error"].is_string());
            assert_eq!(std::fs::read(root.join(destination).join("member.txt")).unwrap(), b"recycle completion test");
            assert_eq!(std::fs::read(&path).unwrap(), original);
        };
        run("all", vec![], "error", true);
        run("selected", vec!["member.txt".into()], "error", false);
        run("skip", vec![], "skip", false);
        std::fs::write(root.join("archive.z01"), b"not a real companion").unwrap();
        run("companion", vec![], "error", false);
    }
    #[test] fn pinned_zip_proof_rejects_ambiguous_and_split_end_records() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("end.zip");
        let mut classic = [0; 22];
        classic[..4].copy_from_slice(b"PK\x05\x06");
        std::fs::write(&path, classic).unwrap();
        assert_eq!(pinned_zip_entries(&File::open(&path).unwrap()).unwrap(), Some(0));
        let mut ambiguous = classic.to_vec();
        ambiguous[20] = 22;
        ambiguous.extend_from_slice(&classic);
        std::fs::write(&path, ambiguous).unwrap();
        assert!(pinned_zip_entries(&File::open(&path).unwrap()).is_err());
        classic[4] = 1;
        std::fs::write(&path, classic).unwrap();
        assert!(pinned_zip_entries(&File::open(&path).unwrap()).is_err());
    }
    #[test] fn cancelled_or_incomplete_extraction_cannot_commit_recycle() {
        let control=JobControl::new();
        assert!(verify_completed_files(&serde_json::json!({"files":2}),3,&control).is_err());
        assert!(verify_completed_files(&serde_json::json!({"files":3}),3,&control).is_ok());
        control.cancel();
        assert!(verify_completed_files(&serde_json::json!({"files":3}),3,&control).is_err());
        assert!(commit_recycle(&control).is_err());
        assert_ne!(control.snapshot().phase,"recycling");
        let request=Request{operation:"extract".into(),path:"does-not-exist.zip".into(),..Default::default()};
        assert_eq!(execute_job(request,true,&control).unwrap_err(),"operation cancelled");
    }
    #[test] fn failed_archive_preflight_preserves_source() {
        let path=std::env::temp_dir().join(format!("open-archive-recycle-failure-{}.zip",std::process::id()));
        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        use std::io::Write;file.write_all(b"not a ZIP archive").unwrap();drop(file);
        let control=JobControl::new();
        let result=execute_job(Request{operation:"extract".into(),path:path.to_string_lossy().into_owned(),dest:path.with_extension("out").to_string_lossy().into_owned(),..Default::default()},true,&control);
        assert!(result.is_err());assert_eq!(std::fs::read(&path).unwrap(),b"not a ZIP archive");
        std::fs::remove_file(path).unwrap();
    }
}
