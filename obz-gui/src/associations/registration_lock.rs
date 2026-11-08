// All copies of qing-zip that write this user's shell registrations take this
// lock across ownership checks, edits and any compensating rollback.
use std::{
    fmt::Write,
    marker::PhantomData,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    rc::Rc,
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            ERROR_INSUFFICIENT_BUFFER, HANDLE, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0,
            WAIT_TIMEOUT,
        },
        Security::{GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{
            CreateMutexW, GetCurrentProcess, OpenProcessToken, ReleaseMutex, WaitForSingleObject,
        },
    },
};

pub(crate) struct RegistrationLock {
    handle: OwnedHandle,
    // Windows mutex ownership is bound to the acquiring thread.
    _thread: PhantomData<Rc<()>>,
}
impl Drop for RegistrationLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(HANDLE(self.handle.as_raw_handle()));
        }
    }
}

fn user_lock_name() -> windows::core::Result<String> {
    let mut raw = HANDLE::default();
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw)?;
    }
    let token = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let handle = HANDLE(token.as_raw_handle());
    let mut required = 0;
    if let Err(error) = unsafe { GetTokenInformation(handle, TokenUser, None, 0, &mut required) } {
        if error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() {
            return Err(error);
        }
    }
    let mut storage = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        GetTokenInformation(
            handle,
            TokenUser,
            Some(storage.as_mut_ptr().cast()),
            required,
            &mut required,
        )?;
        let sid = (*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let bytes = std::slice::from_raw_parts(sid.0.cast::<u8>(), GetLengthSid(sid) as usize);
        let mut name = String::from(r"Global\qing-zip.registration.v1.");
        for byte in bytes {
            write!(name, "{byte:02x}").expect("format into String");
        }
        Ok(name)
    }
}
fn acquire(name: &str) -> Result<RegistrationLock, String> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let raw =
        unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.map_err(|e| e.to_string())?;
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    match unsafe { WaitForSingleObject(raw, 10_000) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(RegistrationLock {
            handle,
            _thread: PhantomData,
        }),
        WAIT_TIMEOUT => Err("另一份 qing-zip 正在更改系统注册，请稍后重试".into()),
        WAIT_FAILED => Err(std::io::Error::last_os_error().to_string()),
        result => Err(format!("unexpected registry mutex result {}", result.0)),
    }
}
pub(crate) fn registration_lock() -> Result<RegistrationLock, String> {
    acquire(&user_lock_name().map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::Command,
        time::{Duration, Instant},
    };
    #[test]
    fn locks_coordinate_separate_processes_and_allow_same_thread_reentry() {
        const DIR: &str = "QING_REGISTRATION_MUTEX_FIXTURE";
        const OWNER: &str = "QING_REGISTRATION_MUTEX_PID";
        let name = |owner: &str| format!("{}.test.{owner}", user_lock_name().unwrap());
        if let (Some(dir), Ok(pid)) = (std::env::var_os(DIR), std::env::var(OWNER)) {
            let dir = std::path::PathBuf::from(dir);
            fs::write(dir.join("ready"), b"").unwrap();
            let _lock = acquire(&name(&pid)).unwrap();
            fs::write(dir.join("acquired"), b"").unwrap();
            return;
        }
        let pid = std::process::id().to_string();
        let guard = acquire(&name(&pid)).unwrap();
        drop(acquire(&name(&pid)).unwrap());
        let dir = tempfile::tempdir().unwrap();
        let mut child=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","associations::registration_lock::tests::locks_coordinate_separate_processes_and_allow_same_thread_reentry"])
            .env(DIR,dir.path()).env(OWNER,&pid).spawn().unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while !dir.path().join("ready").exists() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert!(!dir.path().join("acquired").exists());
        drop(guard);
        assert!(child.wait().unwrap().success());
        assert!(dir.path().join("acquired").exists());
    }
}
