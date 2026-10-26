use obz_core::{engine::{JobControl, Progress}, operations::{self, Request}};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, atomic::{AtomicU64, Ordering}}};
use std::{io::{Read, Seek, SeekFrom}, fs::File, process::{Command, Stdio}, time::{Duration, Instant}};

struct Job { control: JobControl, state: Mutex<JobState>, finished: std::sync::Condvar }
struct JobState { status: String, result: Option<Value>, error: Option<String> }
#[derive(Serialize)]
pub struct Snapshot { id: u64, status: String, progress: Progress, result: Option<Value>, error: Option<String> }
static JOBS: OnceLock<Mutex<BTreeMap<u64, Arc<Job>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn registry() -> &'static Mutex<BTreeMap<u64, Arc<Job>>> { JOBS.get_or_init(Default::default) }

pub fn running() -> bool { registry().lock().map(|jobs|jobs.values().any(|j|j.state.lock().map(|s|s.status=="running").unwrap_or(true))).unwrap_or(true) }
pub fn cancel_all() { if let Ok(jobs)=registry().lock(){for job in jobs.values(){
    let phase=job.control.progress.lock().unwrap_or_else(|e|e.into_inner());
    if phase.phase!="recycling" {job.control.cancel();}
}} }

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
fn confirm_single_volume(request: &Request, control: &JobControl) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    check_cancel(control)?;
    let mut command=Command::new(obz_core::engine::executable()?);
    command.args(["l","-slt","-sccUTF-8","-y","-spd"]).creation_flags(0x08000000)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(password)=&request.options.password {
        if password.is_empty() || password.contains(['\0','\n','\r']) {return Err("无效密码".into());}
        command.arg(format!("-p{password}"));
    }
    if let Some(cp)=request.options.codepage {command.arg(format!("-mcp={cp}"));}
    let mut child=command.arg("--").arg(&request.path).spawn().map_err(|e|e.to_string())?;
    let stdout=child.stdout.take().ok_or("无法读取归档信息")?;
    let stderr=child.stderr.take().ok_or("无法读取引擎状态")?;
    let out=std::thread::spawn(move||capture_prefix(stdout,65536));
    let err=std::thread::spawn(move||capture_prefix(stderr,4096));
    let start=Instant::now();
    let status=loop {
        if control.cancelled.load(Ordering::SeqCst) || start.elapsed()>Duration::from_secs(60) {
            let _=child.kill();let _=child.wait();let _=out.join();let _=err.join();
            check_cancel(control)?;
            return Err("确认单卷状态超时，原压缩包保持不变".into());
        }
        match child.try_wait() {
            Ok(Some(status))=>break status,
            Ok(None)=>if let Err(e)=obz_core::engine::wait_for_process(&child,Duration::from_millis(40)) {
                let _=child.kill();let _=child.wait();let _=out.join();let _=err.join();return Err(e.to_string());
            },
            Err(e)=>{let _=child.kill();let _=child.wait();let _=out.join();let _=err.join();return Err(e.to_string());}
        }
    };
    let bytes=out.join().map_err(|_|"归档信息读取失败")?;let _=err.join();
    if !status.success(){return Err("无法确认压缩包完整且为单卷，未解压或回收，请使用普通解压查看原因".into());}
    let text=String::from_utf8_lossy(&bytes);
    let (header,_)=text.split_once("----------").ok_or("归档信息不完整，不能自动回收")?;
    validate_archive_header(header)?;
    if header.lines().any(|l|l.trim().eq_ignore_ascii_case("Type = zip")) {confirm_zip_disk(&request.path)?;}
    check_cancel(control)
}
fn confirm_zip_disk(path: &str) -> Result<(), String> {
    let mut file=File::open(path).map_err(|e|e.to_string())?;
    let length=file.metadata().map_err(|e|e.to_string())?.len();
    let tail_length=length.min(65557);file.seek(SeekFrom::End(-(tail_length as i64))).map_err(|e|e.to_string())?;
    let mut bytes=Vec::new();file.read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    let position=(0..bytes.len().saturating_sub(21)).rev().find(|&p|bytes[p..].starts_with(b"PK\x05\x06")
        && p+22+u16::from_le_bytes([bytes[p+20],bytes[p+21]]) as usize==bytes.len()).ok_or("ZIP 结束记录无效，不能自动回收")?;
    let h=&bytes[position..];let u16at=|p|u16::from_le_bytes([h[p],h[p+1]]);
    if u16at(4)!=0 || u16at(6)!=0 || u16at(8)!=u16at(10) {return Err("ZIP 分卷记录不允许自动回收".into());}
    if position>=20 && bytes[position-20..position-16]==*b"PK\x06\x07" {
        let locator=&bytes[position-20..position];
        let disk=u32::from_le_bytes(locator[4..8].try_into().unwrap());
        let disks=u32::from_le_bytes(locator[16..20].try_into().unwrap());
        if disk!=0 || disks!=1 {return Err("ZIP64 分卷记录不允许自动回收".into());}
    }
    Ok(())
}
fn commit_recycle(control: &JobControl) -> Result<(), String> {
    let mut phase=control.progress.lock().unwrap_or_else(|e|e.into_inner());
    check_cancel(control)?;
    *phase=Progress{phase:"recycling".into(),percent:Some(100),message:"解压已完成，正在将原压缩包移入回收站（此阶段不可取消）".into()};
    Ok(())
}
fn verify_completed_files(result: &Value, expected: u64, control: &JobControl) -> Result<(),String> {
    check_cancel(control)?;
    if result.get("files").and_then(Value::as_u64)!=Some(expected) {
        return Err("实际解压文件数不匹配，原压缩包保持不变".into());
    }
    Ok(())
}
fn execute_job(mut request: Request, recycle_after: bool, control: &JobControl) -> Result<Value,String> {
    if !recycle_after {return operations::execute(request);}
    validate_recycle_request(&request)?;check_cancel(control)?;
    if let Some(id)=request.password_id.take() {
        if request.options.password.is_some(){return Err("choose password or password_id, not both".into());}
        request.options.password=Some(obz_core::services::Vault::open(operations::vault_path()?)?.get(&id)?);
    }
    let guard=crate::desktop::ArchiveRecycleGuard::acquire(&request.path)?;
    confirm_single_volume(&request,control)?;
    let entries=operations::list_archive(&request.path,&request.options)?;
    let expected=entries.iter().filter(|e|!e.is_dir).count() as u64;
    let mut result=operations::execute(request)?;
    let recycle=(||{
        verify_completed_files(&result,expected,control)?;
        commit_recycle(control)?;
        guard.recycle()
    })();
    result["recycled"]=Value::Bool(recycle.is_ok());
    if let Err(error)=recycle {result["recycle_error"]=Value::String(error);}
    Ok(result)
}

#[tauri::command]
pub fn start_operation(mut request: Request, recycle_after: Option<bool>) -> Result<u64, String> {
    let mut jobs = registry().lock().map_err(|_|"job registry unavailable")?;
    let running = jobs.values().filter(|j|j.state.lock().map(|s|s.status=="running").unwrap_or(true)).count();
    if running >= 2 { return Err("已有两个任务运行，请等待完成后重试".into()); }
    while jobs.len() >= 12 {
        let id = jobs.iter().find_map(|(id,j)|j.state.lock().ok().filter(|s|s.status!="running").map(|_|*id));
        if let Some(id)=id {jobs.remove(&id);} else {break;}
    }
    let control = JobControl::new(); request.options.control=Some(control.clone());
    let job=Arc::new(Job{control,state:Mutex::new(JobState{status:"running".into(),result:None,error:None}),finished:std::sync::Condvar::new()});
    let id=NEXT.fetch_add(1,Ordering::Relaxed); jobs.insert(id,job.clone());
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||execute_job(request,recycle_after.unwrap_or(false),&job.control)))
            .unwrap_or_else(|_|Err("归档任务异常结束".into()));
        let mut state=job.state.lock().unwrap_or_else(|e|e.into_inner());
        match result {
            Ok(value)=>{state.status="complete".into();state.result=Some(value);},
            Err(e)=>{state.status=if job.control.cancelled.load(Ordering::Relaxed){"cancelled"}else{"failed"}.into();state.error=Some(e);}
        }
        job.finished.notify_all();
    });
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
pub async fn wait_operation(id:u64)->Result<Snapshot,String>{
    let job=registry().lock().map_err(|_|"job registry unavailable")?.get(&id).cloned().ok_or("任务不存在或已释放")?;
    tauri::async_runtime::spawn_blocking(move||wait_snapshot(&job,id,Duration::from_millis(150)))
        .await.map_err(|e|e.to_string())?
}

#[tauri::command]
pub fn cancel_operation(id:u64)->Result<(),String>{
    let job=registry().lock().map_err(|_|"job registry unavailable")?.get(&id).cloned().ok_or("任务不存在")?;
    let phase=job.control.progress.lock().unwrap_or_else(|e|e.into_inner());
    if phase.phase=="recycling" {return Err("原包正在移入回收站，此阶段不可取消".into());}
    job.control.cancel(); Ok(())
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
    fn completion_wait_handles_finished_and_inflight_jobs_without_lost_wakeup() {
        let job=Arc::new(Job{control:JobControl::new(),state:Mutex::new(JobState{status:"running".into(),result:None,error:None}),finished:std::sync::Condvar::new()});
        assert_eq!(wait_snapshot(&job,1,Duration::from_millis(1)).unwrap().status,"running");
        let worker=job.clone();
        let thread=std::thread::spawn(move||{
            std::thread::sleep(Duration::from_millis(20));
            let mut state=worker.state.lock().unwrap();state.status="complete".into();state.result=Some(serde_json::json!({"valid":true}));
            worker.finished.notify_all();
        });
        let start=Instant::now();
        let state=wait_snapshot(&job,1,Duration::from_secs(5)).unwrap();
        assert_eq!(state.status,"complete");assert_eq!(state.result.unwrap()["valid"],true);
        assert!(start.elapsed()<Duration::from_secs(2));thread.join().unwrap();
        let start=Instant::now();assert_eq!(wait_snapshot(&job,1,Duration::from_secs(5)).unwrap().status,"complete");
        assert!(start.elapsed()<Duration::from_secs(2));
    }
    #[test] fn unknown_job_fails(){assert!(operation_status(u64::MAX).is_err());assert!(cancel_operation(u64::MAX).is_err());}
    #[test] fn background_failure_is_reported_and_released(){
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
