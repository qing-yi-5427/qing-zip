use obz_core::engine::{self, EngineOptions};
use std::{fs, path::Path};
fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
#[test]
fn solid_many_members_and_selected_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("solid");
    fs::create_dir(&src).unwrap();
    let bytes = "solid archive 中文 repeated data\n".repeat(4096);
    for i in 0..80 {
        fs::write(src.join(format!("{i:03}.txt")), &bytes).unwrap();
    }
    let archive = p(&t.path().join("solid.7z"));
    engine::create(&archive, &[p(&src)], "7z", &EngineOptions::default()).unwrap();
    let output = t.path().join("all");
    assert_eq!(
        engine::extract(&archive, &p(&output), &EngineOptions::default()).unwrap(),
        80
    );
    for i in 0..80 {
        assert_eq!(
            fs::read(output.join(format!("solid/{i:03}.txt"))).unwrap(),
            bytes.as_bytes()
        );
    }
    let chosen = EngineOptions {
        selected: vec!["solid/079.txt".into()],
        ..Default::default()
    };
    let output = t.path().join("selected");
    assert_eq!(engine::extract(&archive, &p(&output), &chosen).unwrap(), 1);
    assert!(!output.join("solid/000.txt").exists());
    assert_eq!(
        fs::read(output.join("solid/079.txt")).unwrap(),
        bytes.as_bytes()
    );
}
#[test]
fn anonymous_bzip2_stream_is_not_empty() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("source.txt");
    fs::write(&src, b"anonymous stream contents").unwrap();
    let archive = p(&t.path().join("stream.bz2"));
    engine::create(&archive, &[p(&src)], "bzip2", &EngineOptions::default()).unwrap();
    let entries = engine::list(&archive, &EngineOptions::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].size, 25);
    let output = t.path().join("out");
    assert_eq!(
        engine::extract(&archive, &p(&output), &EngineOptions::default()).unwrap(),
        1
    );
    assert_eq!(
        fs::read(output.join(&entries[0].name)).unwrap(),
        b"anonymous stream contents"
    );
}

#[test]
fn cancel_decode_does_not_publish_destination() {
    use obz_core::engine::JobControl;
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("large");
    fs::create_dir(&src).unwrap();
    let mut block = vec![0u8; 16 * 1024 * 1024];
    let mut seed = 0x12345678u32;
    for b in &mut block {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *b = seed as u8;
    }
    fs::write(src.join("a"), &block).unwrap();
    fs::write(src.join("b"), &block).unwrap();
    let archive = p(&t.path().join("cancel.7z"));
    engine::create(
        &archive,
        &[p(&src)],
        "7z",
        &EngineOptions {
            level: Some(0),
            ..Default::default()
        },
    )
    .unwrap();
    let destination = t.path().join("out");
    let dest = p(&destination);
    let ctl = JobControl::new();
    let child_ctl = ctl.clone();
    let worker = thread::spawn(move || {
        engine::extract_with_control(&archive, &dest, &EngineOptions::default(), &child_ctl)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while ctl.snapshot().phase != "decoding" && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(ctl.snapshot().phase, "decoding");
    ctl.cancel();
    assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(!destination.exists());
}

#[cfg(all(windows, target_arch = "x86_64", target_env = "msvc"))]
#[test]
fn split_copy_cli_discards_unselected_bytes_and_rejects_their_crc_failure() {
    use std::process::Command;

    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir(&input).unwrap();
    let keep = b"selected bytes";
    let discarded = b"independent unselected Copy payload!".repeat(100);
    fs::write(input.join("keep.txt"), keep).unwrap();
    fs::write(input.join("other.bin"), &discarded).unwrap();

    // Build the Copy archive from test-owned bytes, then partition it into
    // standard numeric volumes without relying on private fixtures or a decoder.
    let archive = temp.path().join("source.7z");
    let created = Command::new(engine::executable().unwrap())
        .current_dir(&input)
        .args(["a", "-t7z", "-mx=0", "-ms=off"])
        .arg(&archive)
        .args(["keep.txt", "other.bin"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{:?}", created.stderr);
    let bytes = fs::read(archive).unwrap();
    let positions: Vec<_> = bytes
        .windows(discarded.len())
        .enumerate()
        .filter_map(|(index, window)| (window == discarded.as_slice()).then_some(index))
        .collect();
    assert_eq!(
        positions.len(),
        1,
        "unselected Copy payload should occur once"
    );
    let volume_size = 1024;
    let volume = |number: usize| temp.path().join(format!("sample.7z.{number:03}"));
    for (index, chunk) in bytes.chunks(volume_size).enumerate() {
        fs::write(volume(index + 1), chunk).unwrap();
    }

    let extract = |dest: &Path| {
        Command::new(env!("CARGO_BIN_EXE_obz"))
            .env("OPEN_ARCHIVE_PROFILE", "1")
            .arg("extract")
            .arg(volume(1))
            .arg(dest)
            .args(["--include", "keep.txt"])
            .output()
            .unwrap()
    };
    let destination = temp.path().join("valid");
    let result = extract(&destination);
    let diagnostics = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{diagnostics}");
    let receipt = diagnostics
        .lines()
        .find(|line| line.starts_with("OA_SELECTED_WORKER "))
        .unwrap();
    assert!(
        receipt.split_whitespace().any(|s| s == "status=ok"),
        "{receipt}"
    );
    assert!(
        receipt
            .split_whitespace()
            .any(|s| s == format!("discarded={}", discarded.len())),
        "{receipt}"
    );
    assert_eq!(fs::read(destination.join("keep.txt")).unwrap(), keep);
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);

    // Flip a byte solely inside the unselected file. Its declared CRC remains.
    let offset = positions[0] + discarded.len() / 2;
    let damaged = volume(offset / volume_size + 1);
    let mut chunk = fs::read(&damaged).unwrap();
    chunk[offset % volume_size] ^= 0x40;
    fs::write(damaged, chunk).unwrap();
    let destination = temp.path().join("corrupt");
    let result = extract(&destination);
    let diagnostics = String::from_utf8_lossy(&result.stderr);
    assert!(!result.status.success(), "{diagnostics}");
    let receipt = diagnostics
        .lines()
        .find(|line| line.starts_with("OA_SELECTED_WORKER "))
        .unwrap();
    assert!(
        receipt.split_whitespace().any(|s| s == "status=error"),
        "{receipt}"
    );
    assert!(
        receipt.split_whitespace().any(|s| s == "code=3"),
        "{receipt}"
    );
    assert!(!destination.exists());
}
