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
