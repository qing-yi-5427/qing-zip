#![cfg(windows)]

use obz_core::engine::{self, EngineOptions, JobControl};
use std::{
    fs,
    io::Write,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

/// The library normally finds the authored worker beside the GUI or CLI.
/// Place this test host and the actual worker together in a private test package.
#[test]
fn regular_create_reports_progress_and_cancels_without_publication() {
    const CHILD: &str = "QING_TEST_PACKAGED_ZIP_CONTROL";
    if std::env::var_os(CHILD).is_none() {
        let package = tempfile::tempdir().unwrap();
        let host = package.path().join("test-host.exe");
        fs::copy(std::env::current_exe().unwrap(), &host).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_obz"), package.path().join("obz.exe")).unwrap();
        let result = Command::new(host)
            .args([
                "--exact",
                "regular_create_reports_progress_and_cancels_without_publication",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("OPEN_ARCHIVE_PROFILE", "1")
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&result.stderr);
        assert!(result.status.success(), "{errors}");
        assert!(
            errors.contains("OA_CREATE_WORKER status=complete"),
            "{errors}"
        );
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("source");
    fs::create_dir(&src).unwrap();
    let file = src.join("payload.bin");
    let mut writer = fs::File::create(&file).unwrap();
    let mut buf = [0u8; 65536];
    let mut state = 7u32;
    for b in &mut buf {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *b = state as u8;
    }
    for _ in 0..256 {
        writer.write_all(&buf).unwrap();
    }
    drop(writer);
    let target = tmp.path().join("cancel.zip");
    let control = JobControl::new();
    let stringify = |p: &Path| p.to_str().unwrap().to_owned();
    let output = stringify(&target);
    let inputs = vec![stringify(&src)];
    let token = control.clone();
    let inputs_copy = inputs.clone();
    let job = thread::spawn(move || {
        engine::create_with_control(
            &output,
            &inputs_copy,
            "zip",
            &EngineOptions {
                level: Some(9),
                threads: Some(1),
                ..Default::default()
            },
            &token,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = control.snapshot();
        if state.phase == "compressing" && state.percent.is_some() {
            break;
        }
        assert!(
            !job.is_finished() && Instant::now() < deadline,
            "no running native progress"
        );
        thread::sleep(Duration::from_millis(2));
    }
    control.cancel();
    assert!(job.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(!target.exists());
    assert!(!fs::read_dir(tmp.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".qing-zip-create-")));
    fs::write(&file, b"after cancellation").unwrap();
    let new = JobControl::new();
    engine::create_with_control(
        &stringify(&target),
        &inputs,
        "zip",
        &Default::default(),
        &new,
    )
    .unwrap();
    assert_eq!(new.snapshot().percent, Some(100));
    engine::test(&stringify(&target), &Default::default()).unwrap();
}
