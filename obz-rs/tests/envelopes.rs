use obz_core::{engine::EngineOptions, envelopes};
fn fixture(n: &str) -> String {
    format!(
        "{}/tests/fixtures/envelopes/{n}",
        env!("CARGO_MANIFEST_DIR")
    )
}
#[test]
fn aes_all_published_versions() {
    for v in 0..4 {
        let path = fixture(&format!("v{v}.txt.aes"));
        let default = EngineOptions::default();
        assert!(envelopes::list(&path, &default).unwrap()[0].encrypted);
        let mut opts = default;
        opts.password = Some("Hello".into());
        envelopes::test(&path, &opts).unwrap();
        let rows = envelopes::list(&path, &opts).unwrap();
        let got = envelopes::preview(&path, &rows[0].name, &opts).unwrap();
        assert_eq!(
            got,
            std::fs::read(fixture(&format!("v{v}.expected"))).unwrap()
        );
        opts.password = Some("wrong".into());
        let dest = tempfile::tempdir().unwrap();
        assert!(envelopes::extract(&path, dest.path().to_str().unwrap(), &opts).is_err());
        assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
    }
}
#[test]
fn lzip_uu_xx_and_collision() {
    for name in ["payload.txt.lz", "payload.uue", "payload.xxe"] {
        let p = fixture(name);
        let opts = EngineOptions::default();
        assert!(envelopes::is_supported(&p));
        let entries = envelopes::list(&p, &opts).unwrap();
        assert_eq!(entries.len(), 1);
        let got = envelopes::preview(&p, "payload.txt", &opts).unwrap();
        assert_eq!(got, std::fs::read(fixture("payload.expected")).unwrap());
        let dest = tempfile::tempdir().unwrap();
        assert_eq!(
            envelopes::extract(&p, dest.path().to_str().unwrap(), &opts).unwrap(),
            1
        );
        assert!(envelopes::extract(&p, dest.path().to_str().unwrap(), &opts).is_err());
        assert_eq!(std::fs::read(dest.path().join("payload.txt")).unwrap(), got);
    }
}
#[test]
fn corruption_and_traversal_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let bad = temp.path().join("bad.lz");
    let mut data = std::fs::read(fixture("payload.txt.lz")).unwrap();
    let n = data.len();
    data[n - 20] ^= 1;
    std::fs::write(&bad, data).unwrap();
    assert!(envelopes::test(bad.to_str().unwrap(), &EngineOptions::default()).is_err());
    let bad = temp.path().join("bad.uue");
    let text = std::fs::read_to_string(fixture("payload.uue"))
        .unwrap()
        .replace("payload.txt", "../escape.txt");
    std::fs::write(&bad, text).unwrap();
    assert!(envelopes::test(bad.to_str().unwrap(), &EngineOptions::default()).is_err());
}
#[test]
#[cfg(windows)]
fn legacy_uu_filename_codepage() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = std::fs::read(fixture("payload.uue")).unwrap();
    let old = b"payload.txt";
    let at = data.windows(old.len()).position(|b| b == old).unwrap();
    data.splice(
        at..at + old.len(),
        [0xd6, 0xd0, 0xce, 0xc4, b'.', b't', b'x', b't'],
    );
    let path = dir.path().join("legacy.uue");
    std::fs::write(&path, data).unwrap();
    let mut opts = EngineOptions::default();
    opts.codepage = Some(936);
    assert_eq!(
        envelopes::list(path.to_str().unwrap(), &opts).unwrap()[0].name,
        "中文.txt"
    );
}
#[test]
fn hostile_aes_kdf_is_rejected_before_work() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = std::fs::read(fixture("v3.txt.aes")).unwrap();
    let mut at = 5;
    loop {
        let n = u16::from_be_bytes([data[at], data[at + 1]]) as usize;
        at += 2;
        if n == 0 {
            break;
        }
        at += n;
    }
    data[at..at + 4].fill(255);
    let path = dir.path().join("bad.aes");
    std::fs::write(&path, data).unwrap();
    assert!(
        envelopes::list(path.to_str().unwrap(), &EngineOptions::default())
            .unwrap_err()
            .contains("iteration")
    );
}
