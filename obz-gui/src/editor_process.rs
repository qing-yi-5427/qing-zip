//! Launch only an explicit executable, with one separately quoted document argument.
//! No shell, file associations, command templates, or terminating running editors.
use std::{
    mem::size_of,
    path::{Path, PathBuf},
};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                QueryInformationJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            },
            SystemInformation::GetSystemDirectoryW,
            Threading::{
                CreateProcessW, ResumeThread, TerminateProcess, CREATE_SUSPENDED,
                PROCESS_INFORMATION, STARTUPINFOW,
            },
        },
    },
};
struct Handle(usize);
impl Handle {
    fn raw(&self) -> HANDLE {
        HANDLE(self.0 as *mut _)
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.raw());
        }
    }
}
pub struct EditorProcess {
    job: Handle,
}
fn wide(s: &str) -> Result<Vec<u16>, String> {
    if s.contains(['\0', '"']) {
        return Err("invalid editor or document path".into());
    }
    Ok(s.encode_utf16().chain(Some(0)).collect())
}
fn system_editor() -> Result<PathBuf, String> {
    let mut buf = [0u16; 32768];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    if n == 0 || n >= buf.len() {
        return Err("无法找到系统记事本".into());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n])).join("notepad.exe"))
}
impl EditorProcess {
    pub fn launch(editor: &str, document: &Path) -> Result<Self, String> {
        let editor = if editor.is_empty() {
            system_editor()?
        } else {
            PathBuf::from(editor)
        };
        if !editor.is_absolute()
            || !editor
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
            || !editor.is_file()
        {
            return Err("请选择完整路径的 EXE 编辑器".into());
        }
        let editor = std::fs::canonicalize(editor).map_err(|e| e.to_string())?;
        let exe = editor
            .to_str()
            .ok_or("invalid editor path")?
            .trim_start_matches("\\\\?\\");
        let doc = document
            .to_str()
            .ok_or("invalid document path")?
            .trim_start_matches("\\\\?\\");
        let application = wide(exe)?;
        let doc_wide = wide(doc)?;
        drop(doc_wide);
        let mut command = format!("\"{exe}\" \"{doc}\"")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let cwd = wide(
            editor
                .parent()
                .and_then(|p| p.to_str())
                .ok_or("invalid editor directory")?
                .trim_start_matches("\\\\?\\"),
        )?;
        unsafe {
            let job = Handle(
                CreateJobObjectW(None, PCWSTR::null())
                    .map_err(|e| e.to_string())?
                    .0 as usize,
            );
            let startup = STARTUPINFOW {
                cb: size_of::<STARTUPINFOW>() as u32,
                ..Default::default()
            };
            let mut info = PROCESS_INFORMATION::default();
            CreateProcessW(
                PCWSTR(application.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_SUSPENDED,
                None,
                PCWSTR(cwd.as_ptr()),
                &startup,
                &mut info,
            )
            .map_err(|e| e.to_string())?;
            let process = Handle(info.hProcess.0 as usize);
            let thread = Handle(info.hThread.0 as usize);
            if let Err(e) = AssignProcessToJobObject(job.raw(), process.raw()) {
                let _ = TerminateProcess(process.raw(), 1);
                return Err(format!("无法跟踪编辑器，未启动编辑：{e}"));
            }
            if ResumeThread(thread.raw()) == u32::MAX {
                let _ = TerminateProcess(process.raw(), 1);
                return Err("无法启动编辑器".into());
            }
            Ok(Self { job })
        }
    }
    /// A completed job does not prove the document is closed: any executable
    /// can hand it to an existing process, including after a slow startup.
    /// This status is informational only; external working copies are retained.
    pub fn state(&self) -> Result<&'static str, String> {
        unsafe {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            QueryInformationJobObject(
                Some(self.job.raw()),
                JobObjectBasicAccountingInformation,
                &mut info as *mut _ as *mut _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
            .map_err(|e| e.to_string())?;
            if info.ActiveProcesses > 0 {
                return Ok("running");
            }
            Ok("handoff")
        }
    }
}
#[tauri::command]
pub async fn pick_editor() -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("选择外部编辑器")
        .add_filter("程序", &["exe"])
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn wait(mut f: impl FnMut() -> bool) {
        let start = Instant::now();
        while !f() {
            assert!(start.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(30));
        }
    }
    #[test]
    fn tracks_own_children_without_binding_unrelated_same_name_editors_or_killing_them() {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join("editor & safe.exe");
        assert!(std::process::Command::new("rustc")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/editor.rs"))
            .arg("-o")
            .arg(&exe)
            .status()
            .unwrap()
            .success());
        let doc = temp.path().join("文档 & literal.txt");
        let other = temp.path().join("unrelated.txt");
        fs::write(&doc, "initial").unwrap();
        fs::write(&other, "other").unwrap();
        let marker =
            |path: &Path, suffix: &str| PathBuf::from(format!("{}{suffix}", path.display()));
        let editor = EditorProcess::launch(exe.to_str().unwrap(), &doc).unwrap();
        let unrelated = EditorProcess::launch(exe.to_str().unwrap(), &other).unwrap();
        wait(|| marker(&doc, ".ready").exists() && marker(&other, ".ready").exists());
        assert_eq!(editor.state().unwrap(), "running");
        fs::write(marker(&doc, ".command"), "write:changed").unwrap();
        wait(|| fs::read_to_string(&doc).unwrap() == "changed");
        thread::sleep(Duration::from_millis(2100));
        fs::write(marker(&doc, ".command"), "exit").unwrap();
        wait(|| editor.state().unwrap() == "handoff");
        assert_eq!(unrelated.state().unwrap(), "running");
        drop(unrelated);
        fs::write(marker(&other, ".command"), "write:still alive").unwrap();
        wait(|| fs::read_to_string(&other).unwrap() == "still alive");
        fs::write(marker(&other, ".command"), "exit").unwrap();
        let doc = temp.path().join("handoff.txt");
        fs::write(&doc, "initial").unwrap();
        let editor = EditorProcess::launch(exe.to_str().unwrap(), &doc).unwrap();
        wait(|| marker(&doc, ".ready").exists());
        assert_eq!(editor.state().unwrap(), "running");
        fs::write(marker(&doc, ".command"), "exit").unwrap();
        wait(|| editor.state().unwrap() == "handoff");
        // A slow launcher forwards to a worker that predates (and is outside)
        // this job. Its elapsed lifetime cannot authorize copy deletion.
        let delayed = temp.path().join("delayed-handoff.txt");
        fs::write(&delayed, "initial").unwrap();
        let mut existing = std::process::Command::new(&exe)
            .arg("--child")
            .arg(&delayed)
            .spawn()
            .unwrap();
        wait(|| marker(&delayed, ".ready").exists());
        let launcher = EditorProcess::launch(exe.to_str().unwrap(), &delayed).unwrap();
        wait(|| marker(&delayed, ".forwarded").exists());
        wait(|| launcher.state().unwrap() == "handoff");
        assert!(existing.try_wait().unwrap().is_none());
        fs::write(
            marker(&delayed, ".command"),
            "write:late handoff still editing",
        )
        .unwrap();
        wait(|| fs::read_to_string(&delayed).unwrap() == "late handoff still editing");
        fs::write(marker(&delayed, ".command"), "exit").unwrap();
        existing.wait().unwrap();
        assert!(EditorProcess::launch("editor.exe", &doc).is_err());
        assert!(EditorProcess::launch("C:\\run.cmd", &doc).is_err());
    }
}
