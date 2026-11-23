//! Tie ZIP update writers and archive integrity checkers to their calling process.
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, PipeWriter},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
        process::ExitStatusExt,
    },
    path::Path,
    process::{Command, ExitStatus},
    ptr,
    time::Duration,
};

pub(super) fn verified_bundled_nana_console(program: &Path) -> Option<crate::machine_policy::Backend> {
    if program.file_name()? != "NanaZip.Universal.Console.exe" {
        return None;
    }
    crate::machine_policy::verify_backend_at(program.parent()?)
        .ok().filter(|verified| verified.exe == program)
}

extern "C" {
    fn qing_spawn_owned_update(
        exe: *const u16,
        command: *mut u16,
        cwd: *const u16,
        input: RawHandle,
        output: RawHandle,
        error: RawHandle,
        process: *mut RawHandle,
        owner: *mut RawHandle,
    ) -> u32;
}

#[link(name = "Kernel32")]
extern "system" {
    fn WaitForSingleObject(handle: RawHandle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: RawHandle, status: *mut u32) -> i32;
    fn TerminateJobObject(job: RawHandle, exit_code: u32) -> i32;
}

pub(super) struct Process {
    job: Option<OwnedHandle>,
    process: OwnedHandle,
    pub(super) stdin: Option<PipeWriter>,
}

#[cfg(test)]
thread_local! {
    pub(super) static AFTER_LAUNCH: std::cell::RefCell<Option<Box<dyn FnOnce(&Process, &Command)>>> = const { std::cell::RefCell::new(None) };
}

fn wide_null(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide: Vec<_> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in engine command",
        ));
    }
    wide.push(0);
    Ok(wide)
}

// NanaZip and 7-Zip read Windows CRT-style arguments. Quote each argument and
// escape quotes and the runs of backslashes that precede a quote.
fn command_line(command: &Command) -> io::Result<Vec<u16>> {
    const QUOTE: u16 = b'"' as u16;
    const SLASH: u16 = b'\\' as u16;
    let mut line = Vec::new();
    for arg in std::iter::once(command.get_program()).chain(command.get_args()) {
        if !line.is_empty() {
            line.push(b' ' as u16);
        }
        line.push(QUOTE);
        let mut slashes = 0;
        for unit in arg.encode_wide() {
            match unit {
                0 => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "NUL in engine argument",
                    ))
                }
                SLASH => slashes += 1,
                _ => {
                    let escape = if unit == QUOTE { slashes + 1 } else { 0 };
                    line.extend(std::iter::repeat_n(SLASH, slashes + escape));
                    line.push(unit);
                    slashes = 0;
                }
            }
        }
        line.extend(std::iter::repeat_n(SLASH, slashes * 2));
        line.push(QUOTE);
    }
    line.push(0);
    Ok(line)
}

impl Process {
    pub(super) fn launch(
        command: &Command,
        output: File,
        error: File,
        password: bool,
    ) -> io::Result<Self> {
        let executable = wide_null(command.get_program())?;
        let mut line = command_line(command)?;
        let cwd = command
            .get_current_dir()
            .map(|path| wide_null(path.as_os_str()))
            .transpose()?;
        let (input, stdin) = if password {
            let (reader, writer) = io::pipe()?;
            (Some(reader), Some(writer))
        } else {
            (None, None)
        };
        let mut process = ptr::null_mut();
        let mut job = ptr::null_mut();
        // Native startup duplicates only these streams, with the guard job
        // supplied as a creation attribute instead of an inherited handle.
        let status = {
            let _guard = super::ARCHIVE_SPAWN
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            unsafe {
                qing_spawn_owned_update(
                    executable.as_ptr(),
                    line.as_mut_ptr(),
                    cwd.as_ref().map_or(ptr::null(), |wide| wide.as_ptr()),
                    input
                        .as_ref()
                        .map_or(ptr::null_mut(), AsRawHandle::as_raw_handle),
                    output.as_raw_handle(),
                    error.as_raw_handle(),
                    &mut process,
                    &mut job,
                )
            }
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        // On success both returned handles are owned exclusively by this guard.
        let (job, process) = unsafe {
            (
                OwnedHandle::from_raw_handle(job),
                OwnedHandle::from_raw_handle(process),
            )
        };
        let child = Self {
            job: Some(job),
            process,
            stdin,
        };
        #[cfg(test)]
        AFTER_LAUNCH.with(|hook| {
            if let Some(observe) = hook.borrow_mut().take() {
                observe(&child, command);
            }
        });
        Ok(child)
    }

    fn ready(&self, milliseconds: u32) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), milliseconds) } {
            0 => Ok(true),
            258 => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn status(&mut self) -> io::Result<ExitStatus> {
        let mut exit = 0;
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut exit) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // Once the main engine has exited, close the job before publication.
        self.job.take();
        Ok(ExitStatus::from_raw(exit))
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.ready(0)? {
            return self.status().map(Some);
        }
        Ok(None)
    }

    pub(super) fn wait_ready(&self, timeout: Duration) -> io::Result<()> {
        self.ready(timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32)
            .map(|_| ())
    }

    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        self.ready(u32::MAX)?;
        self.status()
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        let Some(job) = self.job.take() else {
            return Ok(());
        };
        let result = unsafe { TerminateJobObject(job.as_raw_handle(), 1) };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{update_with_control, EngineOptions, JobControl, UpdateMode};
    use std::{cell::RefCell, fs, rc::Rc};

    #[link(name = "Kernel32")]
    extern "system" {
        fn IsProcessInJob(process: RawHandle, job: RawHandle, member: *mut i32) -> i32;
        fn GetHandleInformation(handle: RawHandle, flags: *mut u32) -> i32;
    }

    #[test]
    fn ordinary_zip_cancel_reaps_its_bound_engine_before_returning() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        let original = root.join("before.txt");
        fs::write(&original, b"existing archive member").unwrap();
        let archive = root.join("existing archive.zip");
        let archive_path = archive.to_str().unwrap();
        crate::zipfmt::create_zip(archive_path, &[original.display().to_string()], false).unwrap();
        let snapshot = fs::read(&archive).unwrap();
        let added = root.join("ordinary 中文 input.bin");
        let source = (0..1_000_017u32)
            .map(|n| (n % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&added, &source).unwrap();
        let control = JobControl::new();
        let cancel = control.clone();
        let handle = Rc::new(RefCell::new(None));
        let observed = handle.clone();
        // The console still owns non-default update modes even when the new
        // seekable Add helper is installed beside this test executable.
        let options = EngineOptions { update_mode: UpdateMode::Update, ..Default::default() };
        AFTER_LAUNCH.with(|hook| {
            hook.replace(Some(Box::new(move |child, _command| {
                let process = child.process.as_raw_handle();
                let job = child.job.as_ref().unwrap().as_raw_handle();
                let mut in_job = 0;
                let mut flags = 0;
                assert_ne!(unsafe { IsProcessInJob(process, job, &mut in_job) }, 0);
                assert_ne!(in_job, 0);
                assert_ne!(unsafe { GetHandleInformation(job, &mut flags) }, 0);
                assert_eq!(flags & 1, 0, "engine job handle must not be inheritable");
                let (ready, acquired) = std::sync::mpsc::channel();
                let competitor = std::thread::spawn(move || {
                    let _guard = crate::engine::ARCHIVE_SPAWN.lock().unwrap();
                    let _ = ready.send(());
                });
                acquired
                    .recv_timeout(Duration::from_secs(5))
                    .expect("archive launch lock is available before the engine is reaped");
                competitor.join().unwrap();
                observed.replace(Some(child.process.try_clone().unwrap()));
                cancel.cancel();
            })))
        });
        let result = update_with_control(
            archive_path,
            "add",
            &[added.display().to_string()],
            &options,
            &control,
        );
        assert_eq!(result.unwrap_err(), "operation cancelled");
        let child = handle
            .borrow_mut()
            .take()
            .expect("native update engine launched");
        assert_eq!(unsafe { WaitForSingleObject(child.as_raw_handle(), 0) }, 0);
        assert_eq!(fs::read(&archive).unwrap(), snapshot);
        assert_eq!(fs::read(&added).unwrap(), source);
        assert_eq!(
            fs::read_dir(root).unwrap().count(),
            3,
            "cancel cleans its ordinary stage"
        );
        crate::engine::update(
            archive_path,
            "add",
            &[added.display().to_string()],
            &options,
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        let mut extracted = Vec::new();
        use std::io::Read;
        zip.by_name(added.file_name().unwrap().to_str().unwrap())
            .unwrap()
            .read_to_end(&mut extracted)
            .unwrap();
        assert_eq!(extracted, source);
    }
}
