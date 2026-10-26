use obz_core::{engine::EngineOptions, legacy};
#[test]
fn real_multivolume_ace() {
    let path = format!(
        "{}/tests/fixtures/legacy/multi/unarc.ace",
        env!("CARGO_MANIFEST_DIR")
    );
    let opts = EngineOptions::default();
    let entries = legacy::list(&path, &opts).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "UNARC");
    assert_eq!(entries[0].size, 4007168);
    legacy::test(&path, &opts).unwrap();
}
#[test]
#[ignore = "external real encrypted ACE sample in tools/john-samples"]
fn real_encrypted_ace_probe() {
    let path = format!(
        "{}/../tools/john-samples/ACE/TESTFILES_blowfish128_jtr.ace",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut opts = EngineOptions::default();
    opts.password = Some("jtr".into());
    legacy::test(&path, &opts).unwrap();
}
#[test]
fn encrypted_ace1_wrong_password_and_content() {
    let p = format!(
        "{}/tests/fixtures/legacy/encrypted-license.ace",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut opts = EngineOptions::default();
    opts.password = Some("obz-fixture".into());
    let rows = legacy::list(&p, &opts).unwrap();
    assert!(rows[0].encrypted);
    let bytes = legacy::preview(&p, "LICENSE", &opts).unwrap();
    assert_eq!(bytes.len(), 11357);
    let out = tempfile::tempdir().unwrap();
    assert_eq!(
        legacy::extract(&p, out.path().to_str().unwrap(), &opts).unwrap(),
        1
    );
    assert_eq!(std::fs::read(out.path().join("LICENSE")).unwrap(), bytes);
    opts.password = Some("wrong".into());
    let bad = tempfile::tempdir().unwrap();
    assert!(legacy::extract(&p, bad.path().to_str().unwrap(), &opts).is_err());
    assert_eq!(std::fs::read_dir(bad.path()).unwrap().count(), 0);
}
#[test]
fn ace_worker_cancels_a_running_member() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let path = format!(
        "{}/tests/fixtures/legacy/multi/unarc.ace",
        env!("CARGO_MANIFEST_DIR")
    );
    let ctl = obz_core::engine::JobControl::new();
    let mut opts = EngineOptions::default();
    opts.control = Some(ctl.clone());
    let h = thread::spawn(move || legacy::test(&path, &opts));
    let start = Instant::now();
    while ctl.snapshot().phase != "ace-decoding" {
        assert!(start.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(150));
    let start = Instant::now();
    ctl.cancel();
    assert!(h.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(2));
}
