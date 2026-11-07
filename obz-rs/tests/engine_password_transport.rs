//! Real console compatibility checks; only synthetic passwords and temp data.
#[path = "../src/engine_password.rs"]
mod engine_password;

use engine_password::{listing_payload, needs_prompt_switch, PasswordInput};
use std::{fs, path::{Path, PathBuf}, process::{Command, Output, Stdio}, time::{Duration, SystemTime, UNIX_EPOCH}};

fn engines() -> Vec<(&'static str, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let nana = root.join("runtime/nanazip/NanaZip.Universal.Console.exe");
    let nana = if nana.exists() { nana } else { root.join("tools/nanazip-7.0.1882.0/bin/x64/NanaZip.Universal.Console.exe") };
    let result: Vec<_> = [("NanaZip", nana), ("7-Zip", root.join("runtime/7zip/7z.exe"))]
        .into_iter().filter(|(_, path)| path.exists()).collect();
    assert!(!result.is_empty(), "real archive engine runtime required");
    result
}

fn command(exe: &Path, cwd: &Path, verb: &str, password: Option<&str>, args: &[&str]) -> Command {
    let mut command = Command::new(exe);
    command.current_dir(cwd).args([verb, "-y", "-sccUTF-8", "-bsp2"]);
    if password.is_some() && needs_prompt_switch(verb) { command.arg("-p"); }
    command.args(args).stdin(if password.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(password) = password {
        assert!(command.get_args().all(|arg| !arg.to_string_lossy().contains(password)));
        assert!(command.get_envs().all(|(_, value)| value.is_none_or(|value| !value.to_string_lossy().contains(password))));
    }
    #[cfg(windows)] {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

fn run(exe: &Path, cwd: &Path, verb: &str, password: Option<&str>, args: &[&str]) -> Output {
    let mut child = command(exe, cwd, verb, password, args).spawn().unwrap();
    let input = password.map(|password| PasswordInput::spawn(&mut child, password).unwrap());
    let output = child.wait_with_output().unwrap();
    if let Some(input) = input { input.finish().unwrap(); }
    if let Some(password) = password {
        assert!(!output.stdout.windows(password.len()).any(|bytes| bytes == password.as_bytes()));
        assert!(!output.stderr.windows(password.len()).any(|bytes| bytes == password.as_bytes()));
    }
    output
}

fn success(output: Output) -> Output {
    assert!(output.status.success(), "engine failed: {}", String::from_utf8_lossy(&output.stderr));
    output
}

#[test]
fn real_engines_create_list_test_extract_update_and_reject_wrong_password() {
    for (name, exe) in engines() {
        for format in ["zip", "7z"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let text = b"synthetic archive password transport fixture\n";
            fs::write(root.join("data.txt"), text).unwrap();
            let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            // Stock 7-Zip rejects non-ASCII ZIP passwords independently of stdin.
            let suffix = if name == "7-Zip" && format == "zip" { " ASCII " } else { " 中文🔐 " };
            let password = format!("test {unique}{suffix}");
            let archive = format!("archive.{format}");
            let encryption = if format == "zip" { "-mem=AES256" } else { "-mhe=on" };
            success(run(&exe, root, "a", Some(&password), &[encryption, &archive, "data.txt"]));
            let listing = success(run(&exe, root, "l", Some(&password), &["-slt", "-ba", "--", &archive]));
            let listing = listing_payload(&listing.stdout);
            assert!(listing.starts_with(b"Path = data.txt\r\n") || listing.starts_with(b"Path = data.txt\n"));
            assert!(String::from_utf8_lossy(listing).contains("Encrypted = +"));
            success(run(&exe, root, "t", Some(&password), &["--", &archive]));
            let stream = success(run(&exe, root, "x", Some(&password), &["-so", "--", &archive, "data.txt"]));
            assert_eq!(stream.stdout, text, "password prompts must never corrupt preview bytes");
            success(run(&exe, root, "x", Some(&password), &["-oextracted", "--", &archive]));
            assert_eq!(fs::read(root.join("extracted/data.txt")).unwrap(), text);
            let wrong = format!("wrong {unique}");
            assert!(!run(&exe, root, "t", Some(&wrong), &["--", &archive]).status.success());
            assert!(!run(&exe, root, "t", None, &["--", &archive]).status.success());
            if format == "7z" {
                assert!(!run(&exe, root, "l", Some(&wrong), &["-slt", "-ba", "--", &archive]).status.success());
            }
            fs::write(root.join("added.txt"), b"second file").unwrap();
            // Encrypted headers require distinct open and update password reads.
            success(run(&exe, root, "a", Some(&password), &[encryption, "--", &archive, "added.txt"]));
            success(run(&exe, root, "rn", Some(&password), &["--", &archive, "added.txt", "renamed.txt"]));
            success(run(&exe, root, "d", Some(&password), &["--", &archive, "renamed.txt"]));
            let final_data = success(run(&exe, root, "x", Some(&password), &["-so", "--", &archive, "data.txt"]));
            assert_eq!(final_data.stdout, text);
        }
    }
}

#[test]
fn real_encrypted_split_volumes_roundtrip() {
    for (_, exe) in engines() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let data: Vec<u8> = (0..96 * 1024).map(|n| ((n * 17 + n / 31) % 251) as u8).collect();
        fs::write(root.join("data.bin"), &data).unwrap();
        let password = format!("split 中文 {}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos());
        success(run(&exe, root, "a", Some(&password), &["-mhe=on", "-mx=0", "-v32k", "archive.7z", "data.bin"]));
        assert!(root.join("archive.7z.003").exists());
        success(run(&exe, root, "t", Some(&password), &["archive.7z.001"]));
        let listing = success(run(&exe, root, "l", Some(&password), &["-slt", "-ba", "archive.7z.001"]));
        assert!(listing_payload(&listing.stdout).starts_with(b"Path = data.bin"));
        let output = success(run(&exe, root, "x", Some(&password), &["-so", "archive.7z.001", "data.bin"]));
        assert_eq!(output.stdout, data);
    }
}

#[test]
fn core_pipeline_preserves_unicode_passwords_and_atomic_updates() {
    use obz_core::engine::{self, EngineOptions};
    let nana = engine::executable().unwrap().file_name().unwrap().to_string_lossy().contains("NanaZip");
    for format in ["zip", "7z"] {
        if format == "zip" && !nana { continue; }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("data.txt");
        fs::write(&source, b"core fixture").unwrap();
        let archive = root.join(format!("archive.{format}")).display().to_string();
        let options = EngineOptions {
            password: Some(format!(" 中文 🔐 {} ", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())),
            ..Default::default()
        };
        engine::create(&archive, &[source.display().to_string()], format, &options).unwrap();
        engine::test(&archive, &options).unwrap();
        assert_eq!(engine::list(&archive, &options).unwrap().len(), 1);
        assert_eq!(engine::preview(&archive, "data.txt", &options).unwrap(), b"core fixture");
        engine::update(&archive, "rename", &["data.txt".into(), "renamed.txt".into()], &options).unwrap();
        let original = fs::read(&archive).unwrap();
        let wrong = EngineOptions { password: Some("wrong".into()), ..Default::default() };
        assert!(engine::test(&archive, &wrong).is_err());
        // ZIP directory edits need no decryption. Encrypted 7z headers must
        // reject the update before replacing the original archive.
        if format == "7z" {
            assert!(engine::update(&archive, "delete", &["renamed.txt".into()], &wrong).is_err());
        }
        assert_eq!(fs::read(&archive).unwrap(), original);
        engine::extract(&archive, &root.join("output").display().to_string(), &options).unwrap();
        assert_eq!(fs::read(root.join("output/renamed.txt")).unwrap(), b"core fixture");
    }
}

#[test]
fn windows_eof_character_cannot_silently_change_the_archive_password() {
    use obz_core::engine::{self, EngineOptions};
    let temp=tempfile::tempdir().unwrap();let root=temp.path();
    let source=root.join("data.txt");fs::write(&source,b"synthetic").unwrap();
    let archive=root.join("must-not-exist.7z");
    let options=EngineOptions{password:Some("prefix\u{1a}suffix".into()),..Default::default()};
    let error=engine::create(&archive.display().to_string(),&[source.display().to_string()],"7z",&options).unwrap_err();
    assert!(error.contains("Ctrl+Z"));assert!(!archive.exists());
}

#[test]
fn unread_long_password_cannot_deadlock_or_block_cancellation() {
    for (_, exe) in engines() {
        let temp = tempfile::tempdir().unwrap();
        let mut child = command(&exe, temp.path(), "i", Some("temporary"), &[])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let password = "x".repeat(1024 * 1024);
        let input = PasswordInput::spawn(&mut child, &password).unwrap();
        // This command does not read stdin. A synchronous pipe write can hang.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() { break; }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                input.finish().unwrap();
                panic!("unread stdin prevented engine exit");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        input.finish().unwrap();
    }
}

#[cfg(windows)]
#[test]
fn process_inspection_has_no_password_and_cancel_releases_input() {
    use std::os::windows::process::CommandExt;
    for (_, exe) in engines() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("data.txt"), b"synthetic").unwrap();
        let password = format!("inspection 中文 {}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos());
        success(run(&exe, root, "a", Some(&password), &["-mhe=on", "archive.7z", "data.txt"]));
        // It waits for the unread password, providing a deterministic inspection window.
        let mut child = command(&exe, root, "t", Some(&password), &["archive.7z"]).spawn().unwrap();
        let inspect = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
            &format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {}').CommandLine", child.id())])
            .creation_flags(0x08000000).output().unwrap();
        let command_line = String::from_utf8_lossy(&inspect.stdout);
        // Kill while the input writer is active/blocked; no detached writer or secret persists.
        let oversized = "x".repeat(8 * 1024 * 1024);
        let input = PasswordInput::spawn(&mut child, &oversized).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let before = std::time::Instant::now();
        input.finish().unwrap();
        assert!(before.elapsed() < Duration::from_secs(2));
        assert!(inspect.status.success());
        assert!(command_line.contains("archive.7z"));
        assert!(!command_line.contains(&password));
        // The executable path may itself contain "-p". Check the complete
        // argument tail for the password switch; Windows filenames cannot
        // contain a quote inside the quoted executable name.
        let line = command_line.trim_start();
        let (_, arguments) = match line.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"'),
            None => line.split_once(char::is_whitespace),
        }.expect("missing inspected command arguments");
        assert!(!arguments.contains("-p"));
    }
}
