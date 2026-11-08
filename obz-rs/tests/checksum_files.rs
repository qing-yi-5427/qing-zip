use std::{fs, process::Command};

#[test]
fn checksum_cli_preserves_order_and_single_file_shape() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("a").join("same [2].txt");
    let second = root.path().join("b").join("same [2].txt");
    for file in [&first, &second] {
        fs::create_dir(file.parent().unwrap()).unwrap();
    }
    fs::write(&first, b"abc").unwrap();
    fs::write(&second, b"").unwrap();
    let run = |paths: &[&std::path::Path], args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_obz"))
            .arg("checksum")
            .args(paths)
            .args(args)
            .output()
            .unwrap()
    };
    for args in [
        &[][..],
        &["--hash", "SHA256", "--hash", "CRC32"][..],
        &["--hash", "*"][..],
    ] {
        let single = run(&[&first], args);
        assert!(
            single.status.success(),
            "{}",
            String::from_utf8_lossy(&single.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&single.stdout).unwrap();
        let batch = run(&[&first, &second, &first], args);
        assert!(
            batch.status.success(),
            "{}",
            String::from_utf8_lossy(&batch.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&batch.stdout).unwrap();
        let files = result["checksums"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0], value);
        assert_eq!(files[2], value);
        assert_eq!(files[1]["path"], second.to_str().unwrap());
        let sha = if args.is_empty() {
            &files[1]["sha256"]
        } else {
            &files[1]["hashes"]["SHA256"]
        };
        assert_eq!(
            sha,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    let error = run(
        &[&first, &root.path().join("missing")],
        &["--hash", "SHA256"],
    );
    assert!(!error.status.success());
    assert!(error.stdout.is_empty());
}

#[cfg(windows)]
#[test]
fn a_writable_source_cannot_return_a_mixed_checksum() {
    use std::fs::OpenOptions;
    let dir = tempfile::tempdir().unwrap();
    let stable = dir.path().join("stable");
    let busy = dir.path().join("busy");
    fs::write(&stable, b"one").unwrap();
    let _writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&busy)
        .unwrap();
    for args in [&[][..], &["--hash", "SHA256"][..]] {
        let output = Command::new(env!("CARGO_BIN_EXE_obz"))
            .arg("checksum")
            .args([&stable, &busy])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
