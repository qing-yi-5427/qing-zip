//! Isolated bundled ACE2 fallback. Passwords use stdin, never command arguments.
use crate::engine::{EngineOptions, Entry};
use std::{
    fs::{self},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
fn runtime() -> Result<PathBuf, String> {
    let mut roots = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        for p in exe.ancestors().skip(1).take(4) {
            roots.push(p.join("runtime/python"));
        }
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/python"));
    roots
        .into_iter()
        .find(|p| p.join("python.exe").is_file() && p.join("ace_worker.py").is_file())
        .ok_or("bundled ACE2 Python runtime missing; restore runtime/python".into())
}
pub fn available() -> bool {
    runtime().is_ok()
}
#[cfg(windows)]
pub(crate) mod job {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Basic {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min: usize,
        max: usize,
        active: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Io {
        read: u64,
        write: u64,
        other: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Extended {
        basic: Basic,
        io: Io,
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attrs: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: u32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    pub struct Job(*mut c_void);
    impl Job {
        pub fn attach(child: &std::process::Child) -> Result<Self, String> {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                let job = Self(h);
                let mut info = Extended::default();
                info.basic.flags = 0x2000 | 0x100 | 0x8;
                info.basic.active = 1;
                info.process_memory = 1024 * 1024 * 1024;
                if SetInformationJobObject(
                    h,
                    9,
                    &info as *const _ as *const c_void,
                    std::mem::size_of::<Extended>() as u32,
                ) == 0
                    || AssignProcessToJobObject(h, child.as_raw_handle()) == 0
                {
                    return Err(std::io::Error::last_os_error().to_string());
                }
                Ok(job)
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
fn run(request: serde_json::Value, opts: &EngineOptions) -> Result<serde_json::Value, String> {
    if let Some(c) = &opts.control {
        c.check()?;
    }
    if let Some(c) = &opts.control {
        let phase = if request["action"] == "decode" {
            "ace-decoding"
        } else {
            "ace-listing"
        };
        c.set(phase, None, "Reading ACE archive");
    }
    let root = runtime()?;
    let logs = tempfile::tempfile().map_err(|e| e.to_string())?;
    let errors = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(root.join("python.exe"));
    cmd.arg("-I")
        .arg(root.join("ace_worker.py"))
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(logs.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(errors.try_clone().map_err(|e| e.to_string())?))
        .env("ACEFILE_FORCE_ACEBITSTREAM", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let _job = match job::Job::attach(&child) {
        Ok(j) => j,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("cannot constrain ACE worker: {e}"));
        }
    };
    let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > 1024 * 1024 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("ACE request too large".into());
    }
    if let Err(e) = child
        .stdin
        .take()
        .ok_or("worker stdin missing")?
        .write_all(&bytes)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e.to_string());
    }
    let start = Instant::now();
    let status = loop {
        let cancelled = opts
            .control
            .as_ref()
            .is_some_and(|c| c.cancelled.load(std::sync::atomic::Ordering::Relaxed));
        if cancelled
            || start.elapsed() > Duration::from_secs(600)
            || logs.metadata().map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024
            || errors.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancelled {
                "operation cancelled"
            } else {
                "ACE worker time/output limit exceeded"
            }
            .into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    };
    let mut log = logs;
    log.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    log.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    if out.len() > 32 * 1024 * 1024 {
        return Err("ACE worker output too large".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&out).map_err(|_| "ACE worker failed without valid response")?;
    if !status.success() {
        let mut msg = value
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("ACE worker failed")
            .to_owned();
        if let Some(p) = &opts.password {
            msg = msg.replace(p, "[redacted]");
        }
        return Err(msg);
    }
    Ok(value)
}
fn request(path: &str, opts: &EngineOptions) -> Result<serde_json::Value, String> {
    let p = std::path::absolute(path).map_err(|e| e.to_string())?;
    let encoding = opts
        .codepage
        .map(|cp| {
            if cp == 65001 {
                "utf-8".into()
            } else {
                format!("cp{cp}")
            }
        })
        .unwrap_or("cp437".into());
    Ok(serde_json::json!({"path":p,"encoding":encoding,"password":opts.password}))
}
pub fn list(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    let mut req = request(path, opts)?;
    req["action"] = "list".into();
    serde_json::from_value(run(req, opts)?).map_err(|e| e.to_string())
}
pub fn decode(
    path: &str,
    chosen: &[Entry],
    stage: &Path,
    opts: &EngineOptions,
) -> Result<(), String> {
    let all = list(path, opts)?;
    let private = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut req = request(path, opts)?;
    req["action"] = "decode".into();
    req["stage"] = serde_json::to_value(private.path()).map_err(|e| e.to_string())?;
    req["selected"] = serde_json::json!(chosen.iter().map(|e| &e.name).collect::<Vec<_>>());
    run(req, opts)?;
    for (i, e) in all
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.is_dir && chosen.iter().any(|c| c.name == e.name))
    {
        if let Some(c) = &opts.control {
            c.check()?;
        }
        let source = private.path().join(format!("{i}.bin"));
        if fs::metadata(&source).map_err(|e| e.to_string())?.len() != e.size {
            return Err("ACE worker output size mismatch".into());
        }
        let target = stage.join(crate::engine::validate_name(&e.name)?);
        fs::create_dir_all(target.parent().ok_or("missing stage parent")?)
            .map_err(|e| e.to_string())?;
        fs::copy(source, target).map_err(|e| e.to_string())?;
    }
    Ok(())
}
