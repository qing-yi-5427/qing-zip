use obz_core::{engine::EngineOptions, rare};
use std::{fs, path::Path, process::Command};
fn p(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
#[test]
fn pma_original_samples_validate_and_extract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pma");
    for file in fs::read_dir(root).unwrap() {
        let file = file.unwrap().path();
        if file.extension().and_then(|s| s.to_str()) != Some("pma") {
            continue;
        }
        let archive = p(&file);
        let entries = rare::list(&archive, &EngineOptions::default()).unwrap();
        assert!(!entries.is_empty());
        let out = tempfile::tempdir().unwrap();
        let count = rare::extract(&archive, &p(out.path()), &EngineOptions::default()).unwrap();
        assert_eq!(count, entries.len());
        for e in entries {
            assert_eq!(fs::metadata(out.path().join(e.name)).unwrap().len(), e.size);
        }
    }
}
#[test]
fn zpaq_reference_writer_roundtrip_and_cancel() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("中文.txt"), b"zpaq unicode content").unwrap();
    let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/zpaq/zpaq64.exe");
    assert!(Command::new(exe)
        .current_dir(t.path())
        .args(["a", "sample.zpaq", "中文.txt", "-method", "1"])
        .output()
        .unwrap()
        .status
        .success());
    let archive = p(&t.path().join("sample.zpaq"));
    let entries = rare::list(&archive, &EngineOptions::default()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "中文.txt");
    let out = t.path().join("out");
    assert_eq!(
        rare::extract(&archive, &p(&out), &EngineOptions::default()).unwrap(),
        1
    );
    assert_eq!(
        fs::read(out.join("中文.txt")).unwrap(),
        b"zpaq unicode content"
    );
    let ctl = obz_core::engine::JobControl::new();
    ctl.cancel();
    assert!(rare::list(
        &archive,
        &EngineOptions {
            control: Some(ctl),
            ..Default::default()
        }
    )
    .unwrap_err()
    .contains("cancelled"));
}
#[test]
fn disc_containers_all_methods_and_corrupt_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/disc");
    for name in ["deflate.daa", "method1.isz", "method2.isz", "method3.isz"] {
        let archive = p(&root.join(name));
        let entries = rare::list(&archive, &EngineOptions::default()).unwrap();
        assert!(entries.iter().any(|e| e.name == "hello.txt"));
        let out = tempfile::tempdir().unwrap();
        rare::extract(&archive, &p(out.path()), &EngineOptions::default()).unwrap();
        let expected = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/original-bandizip/hello.txt"),
        )
        .unwrap();
        assert_eq!(fs::read(out.path().join("hello.txt")).unwrap(), expected);
    }
    let t = tempfile::tempdir().unwrap();
    let mut data = fs::read(root.join("deflate.daa")).unwrap();
    data[72] ^= 1;
    let bad = t.path().join("bad.daa");
    fs::write(&bad, data).unwrap();
    assert!(rare::list(&p(&bad), &EngineOptions::default())
        .unwrap_err()
        .contains("CRC"));
}
#[test]
fn zpaq_encrypted_fixture_and_wrong_password() {
    let archive =
        p(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zpaq/encrypted.zpaq"));
    let options = EngineOptions {
        password: Some("test-password".into()),
        ..Default::default()
    };
    assert_eq!(rare::list(&archive, &options).unwrap().len(), 2);
    assert_eq!(
        rare::preview(&archive, "hello.txt", &options).unwrap(),
        b"Open Archive ZPAQ fixture\n"
    );
    assert!(rare::test(
        &archive,
        &EngineOptions {
            password: Some("incorrect".into()),
            ..Default::default()
        }
    )
    .is_err());
}
#[test]
fn zpaq_parent_traversal_is_rejected_before_publication() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("work")).unwrap();
    fs::write(t.path().join("outside.txt"), b"safe test data").unwrap();
    let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/zpaq/zpaq64.exe");
    let archive = t.path().join("evil.zpaq");
    assert!(Command::new(exe)
        .current_dir(t.path().join("work"))
        .args(["a", &p(&archive), "../outside.txt", "-method", "1"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(rare::extract(
        &p(&archive),
        &p(&t.path().join("destination")),
        &EngineOptions::default()
    )
    .is_err());
    assert!(!t.path().join("destination").exists());
    assert_eq!(
        fs::read(t.path().join("outside.txt")).unwrap(),
        b"safe test data"
    );
}
