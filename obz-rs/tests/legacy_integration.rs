use obz_core::{
    engine::{self, EngineOptions},
    legacy,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn bz() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe")
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out)
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_owned(),
                    fs::read(p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
#[test]
fn lh7_creation_external_7zip_and_bandizip_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path();
    let src = p.join("source");
    fs::create_dir_all(src.join("empty")).unwrap();
    let raw = b"compressible LH7 fixture with repeating bytes\n".repeat(3000);
    fs::write(src.join("data.txt"), &raw).unwrap();
    let out = p.join("created.lzh");
    legacy::create_lzh(
        out.to_str().unwrap(),
        &[src.to_string_lossy().into_owned()],
        &EngineOptions::default(),
    )
    .unwrap();
    let bytes = fs::read(&out).unwrap();
    assert!(bytes.windows(5).any(|w| w == b"-lh7-"));
    assert!(bytes.len() < raw.len() / 2);
    engine::test(out.to_str().unwrap(), &EngineOptions::default()).unwrap();
    let ours = p.join("ours");
    engine::extract(
        out.to_str().unwrap(),
        ours.to_str().unwrap(),
        &EngineOptions::default(),
    )
    .unwrap();
    assert_eq!(fs::read(ours.join("source/data.txt")).unwrap(), raw);
    assert!(ours.join("source/empty").is_dir());
    let original = p.join("original");
    let result = Command::new(bz())
        .args(["x", "-y"])
        .arg(format!("-o:{}", original.display()))
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(tree(&ours), tree(&original));
}
#[test]
fn ace1_ace2_extraction_matches_original_bandizip() {
    for name in ["license1.ace", "license2.ace"] {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy")
            .join(name);
        let entries = legacy::list(fixture.to_str().unwrap(), &EngineOptions::default()).unwrap();
        assert!(!entries.is_empty());
        legacy::test(fixture.to_str().unwrap(), &EngineOptions::default()).unwrap();
        let ours = p.join("ours");
        legacy::extract(
            fixture.to_str().unwrap(),
            ours.to_str().unwrap(),
            &EngineOptions::default(),
        )
        .unwrap();
        let original = p.join("original");
        let result = Command::new(bz())
            .args(["x", "-y"])
            .arg(format!("-o:{}", original.display()))
            .arg(&fixture)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(tree(&ours), tree(&original));
    }
}

#[test]
fn lzh_chinese_filename_explicit_codepage_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("中文文件.txt");
    fs::write(&input, b"Chinese filename evidence").unwrap();
    let archive = t.path().join("chinese.lzh");
    let mut opts = EngineOptions::default();
    opts.codepage = Some(936);
    legacy::create_lzh(
        archive.to_str().unwrap(),
        &[input.to_string_lossy().into_owned()],
        &opts,
    )
    .unwrap();
    let dest = t.path().join("extract");
    engine::extract(archive.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
    assert_eq!(
        fs::read(dest.join("中文文件.txt")).unwrap(),
        b"Chinese filename evidence"
    );
}
#[test]
fn corrupted_ace_does_not_publish_files() {
    let t = tempfile::tempdir().unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/license2.ace");
    let mut bytes = fs::read(fixture).unwrap();
    let index = bytes.len() - 15;
    bytes[index] ^= 0x55;
    let archive = t.path().join("corrupt.ace");
    fs::write(&archive, bytes).unwrap();
    let dest = t.path().join("dest");
    assert!(legacy::extract(
        archive.to_str().unwrap(),
        dest.to_str().unwrap(),
        &EngineOptions::default()
    )
    .is_err());
    assert!(!dest.exists());
}
