use obz_core::{engine::EngineOptions, korean};
fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/korean/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}
#[test]
fn real_codec_corpus() {
    for name in [
        "store5.egg",
        "lzma5.egg",
        "split_store.vol1.egg",
        "split_store.alz",
        "solid.egg",
        "azo_pe.egg",
        "store_empty.alz",
        "bzip2_dlz.alz",
    ] {
        let p = fixture(name);
        let opts = EngineOptions::default();
        assert!(korean::is_supported(&p));
        let es = korean::list(&p, &opts).unwrap();
        assert!(!es.is_empty(), "{name}");
        korean::test(&p, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
        let out = tempfile::tempdir().unwrap();
        assert_eq!(
            korean::extract(&p, out.path().to_str().unwrap(), &opts).unwrap(),
            es.iter().filter(|e| !e.is_dir).count()
        );
        for e in es.iter().filter(|e| !e.is_dir) {
            assert_eq!(
                std::fs::metadata(out.path().join(&e.name)).unwrap().len(),
                e.size
            );
        }
    }
}
#[test]
fn passwords_selection_and_overwrite() {
    for name in ["enc_aes256.egg", "enc_deflate.alz"] {
        let p = fixture(name);
        let mut opts = EngineOptions::default();
        opts.password = Some("wrong".into());
        let out = tempfile::tempdir().unwrap();
        assert!(korean::extract(&p, out.path().to_str().unwrap(), &opts).is_err());
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
        opts.password = Some("test1234".into());
        korean::test(&p, &opts).unwrap();
        opts.selected = vec!["a.txt".into()];
        assert_eq!(
            korean::extract(&p, out.path().to_str().unwrap(), &opts).unwrap(),
            1
        );
        let bytes = korean::preview(&p, "a.txt", &opts).unwrap();
        assert_eq!(bytes.len(), 3150);
        assert_eq!(std::fs::read(out.path().join("a.txt")).unwrap(), bytes);
        assert!(korean::extract(&p, out.path().to_str().unwrap(), &opts).is_err());
        opts.overwrite = "skip".into();
        assert_eq!(
            korean::extract(&p, out.path().to_str().unwrap(), &opts).unwrap(),
            0
        );
        opts.overwrite = "rename".into();
        assert_eq!(
            korean::extract(&p, out.path().to_str().unwrap(), &opts).unwrap(),
            1
        );
        assert!(out.path().join("a (1).txt").exists());
    }
}
#[test]
fn malicious_name_and_cancellation() {
    let mut bytes = std::fs::read(fixture("store_empty.alz")).unwrap();
    let at = bytes.windows(9).position(|w| w == b"empty.txt").unwrap();
    bytes[at..at + 9].copy_from_slice(b"../escape");
    let work = tempfile::tempdir().unwrap();
    let bad = work.path().join("bad.alz");
    std::fs::write(&bad, bytes).unwrap();
    assert!(korean::list(bad.to_str().unwrap(), &EngineOptions::default()).is_err());
    let mut opts = EngineOptions::default();
    let ctl = obz_core::engine::JobControl::new();
    ctl.cancel();
    opts.control = Some(ctl);
    assert!(korean::test(&fixture("store5.egg"), &opts).is_err());
}

#[test]
fn lea_encryption() {
    let mut opts = EngineOptions::default();
    opts.password = Some("test1234".into());
    korean::test(&fixture("lea256.egg"), &opts).unwrap();
}
