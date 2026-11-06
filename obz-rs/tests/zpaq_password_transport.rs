use obz_core::{engine::{EngineOptions, JobControl}, engine_password::PasswordInput, operations, rare};
use std::{fs, io::Write, path::{Path, PathBuf}, process::{Command, Output, Stdio}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

fn executable() -> PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/zpaq/obz-zpaq.exe") }
fn fixture() -> String { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zpaq/encrypted.zpaq").display().to_string() }
fn command(root: &Path, args: &[&str]) -> Command {
    let mut command=Command::new(executable());
    command.current_dir(root).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
    command
}
fn run(root: &Path, args: &[&str], password: &str) -> Output {
    assert!(args.iter().all(|arg| !arg.contains(password)));
    let mut child=command(root,args).spawn().unwrap();
    let input=PasswordInput::spawn(&mut child,password).unwrap();
    let output=child.wait_with_output().unwrap();input.finish().unwrap();
    assert!(!output.stdout.windows(password.len()).any(|s|s==password.as_bytes()));
    assert!(!output.stderr.windows(password.len()).any(|s|s==password.as_bytes()));
    output
}

#[test]
fn original_encrypted_archive_still_opens_without_password_arguments() {
    let options=EngineOptions{password:Some("test-password".into()),..Default::default()};
    assert_eq!(rare::list(&fixture(),&options).unwrap().len(),2);
    rare::test(&fixture(),&options).unwrap();
    assert_eq!(rare::preview(&fixture(),"hello.txt",&options).unwrap(),b"Open Archive ZPAQ fixture\n");
    let temp=tempfile::tempdir().unwrap();let output=temp.path().join("output");
    rare::extract(&fixture(),&output.display().to_string(),&options).unwrap();
    assert_eq!(fs::read(output.join("hello.txt")).unwrap(),b"Open Archive ZPAQ fixture\n");
    for password in [None,Some("wrong".into())] {
        let options=EngineOptions{password,..Default::default()};
        assert!(rare::list(&fixture(),&options).is_err());
        assert!(rare::test(&fixture(),&options).is_err());
        let target=temp.path().join("failed");
        assert!(rare::extract(&fixture(),&target.display().to_string(),&options).is_err());
        assert!(!target.exists());
    }
}

#[test]
fn unicode_password_and_all_recovery_candidates_use_stdin() {
    let temp=tempfile::tempdir().unwrap();let root=temp.path();
    fs::write(root.join("data.txt"),b"zpaq UTF-8 archive fixture").unwrap();
    let password=format!(" spaced 中文🔐 {} ",SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos());
    let archive=root.join("unicode.zpaq").display().to_string();
    let output=run(root,&["a",&archive,"data.txt","-key-stdin","-method","0","-threads","1"],&password);
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    let options=EngineOptions{password:Some(password.clone()),..Default::default()};
    rare::test(&archive,&options).unwrap();
    assert_eq!(rare::preview(&archive,"data.txt",&options).unwrap(),b"zpaq UTF-8 archive fixture");
    let request=serde_json::from_value(serde_json::json!({"operation":"recover","path":archive,"candidates":["wrong first","wrong second",password],"max_attempts":3})).unwrap();
    let result=operations::execute(request).unwrap();
    assert_eq!(result["password"],password);assert_eq!(result["attempts"],3);
    let request=serde_json::from_value(serde_json::json!({"operation":"recover","path":archive,"candidates":["wrong"],"max_attempts":1})).unwrap();
    assert_eq!(operations::execute(request).unwrap()["status"],"exhausted");
}

#[test]
fn malformed_input_fails_closed_without_echo() {
    let temp=tempfile::tempdir().unwrap();
    for input in [b"".as_slice(),b"\n",b"unterminated",b"a\0b\n",b"a\rb\n"] {
        let mut child=command(temp.path(),&["l",&fixture(),"-key-stdin","-threads","1"]).spawn().unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        let output=child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("unterminated"));
    }
}

#[test]
fn cancellation_while_sending_password_preserves_destination() {
    let temp=tempfile::tempdir().unwrap();let dest=temp.path().join("cancelled");
    let control=JobControl::new();let cancellation=control.clone();
    let options=EngineOptions{password:Some("x".repeat(8*1024*1024)),control:Some(control),..Default::default()};
    let cancel=std::thread::spawn(move||{std::thread::sleep(Duration::from_millis(50));cancellation.cancel();});
    let start=Instant::now();
    assert_eq!(rare::extract(&fixture(),&dest.display().to_string(),&options).unwrap_err(),"operation cancelled");
    cancel.join().unwrap();assert!(start.elapsed()<Duration::from_secs(3));assert!(!dest.exists());
}

#[cfg(windows)]
#[test]
fn live_zpaq_process_exposes_only_key_stdin_switch() {
    use std::os::windows::process::CommandExt;
    let temp=tempfile::tempdir().unwrap();
    let mut child=command(temp.path(),&["l",&fixture(),"-key-stdin","-threads","1"]).spawn().unwrap();
    let inspect=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",
        &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {}').CommandLine",child.id())])
        .creation_flags(0x08000000).output().unwrap();
    let input=PasswordInput::spawn(&mut child,"test-password").unwrap();
    let output=child.wait_with_output().unwrap();input.finish().unwrap();
    assert!(output.status.success());assert!(inspect.status.success());
    let arguments=String::from_utf8_lossy(&inspect.stdout);
    assert!(arguments.contains("-key-stdin"));assert!(!arguments.contains("test-password"));
}
