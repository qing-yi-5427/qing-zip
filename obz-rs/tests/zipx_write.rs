//! Real ZIPX/XZ writer and independent-reader interoperability checks.
use obz_core::engine::{self, EngineOptions, JobControl};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn original() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe")
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for e in fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_owned();
            if p.is_dir() {
                out.insert(rel, None);
                visit(root, &p, out);
            } else {
                out.insert(rel, Some(fs::read(p).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn fixture(root: &Path) -> PathBuf {
    let src = root.join("资料目录");
    fs::create_dir_all(src.join("empty/子目录")).unwrap();
    fs::write(
        src.join("中文.txt"),
        b"native ZIPX compression fixture\n".repeat(4000),
    )
    .unwrap();
    fs::write(src.join("zero.bin"), b"").unwrap();
    let mut x = 0x12345678u32;
    let noise = (0..32768)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect::<Vec<_>>();
    fs::write(src.join("noise.bin"), noise).unwrap();
    src
}
fn validate_original(path: &Path, dest: &Path, password: Option<&str>) {
    let mut cmd = Command::new(original());
    cmd.args(["x", "-y"]).arg(format!("-o:{}", dest.display()));
    if let Some(p) = password {
        cmd.arg(format!("-p:{p}"));
    }
    let r = cmd.arg(path).output().unwrap();
    assert!(
        r.status.success(),
        "{} {}",
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
}
#[test]
fn zipx_xz_aes_unicode_empty_dirs_crosscheck() {
    let t = tempfile::tempdir().unwrap();
    let src = fixture(t.path());
    let before = tree(&src);
    for encrypted in [false, true] {
        let mut opts = EngineOptions::default();
        opts.level = Some(5);
        if encrypted {
            opts.password = Some("fixture-password".into());
        }
        let archive = t.path().join(format!("{encrypted}.zipx"));
        let outputs = engine::create(
            archive.to_str().unwrap(),
            &[src.to_string_lossy().into_owned()],
            "zipx",
            &opts,
        )
        .unwrap();
        assert_eq!(outputs, vec![archive.to_string_lossy().into_owned()]);
        let entries = engine::list(archive.to_str().unwrap(), &opts).unwrap();
        assert!(entries
            .iter()
            .any(|e| e.method.to_ascii_uppercase().contains("XZ")));
        assert!(entries
            .iter()
            .filter(|e| !e.is_dir)
            .all(|e| e.encrypted == encrypted));
        engine::test(archive.to_str().unwrap(), &opts).unwrap();
        let ours = t.path().join(format!("ours-{encrypted}"));
        engine::extract(archive.to_str().unwrap(), ours.to_str().unwrap(), &opts).unwrap();
        assert_eq!(tree(&ours.join("资料目录")), before);
        let theirs = t.path().join(format!("theirs-{encrypted}"));
        validate_original(&archive, &theirs, opts.password.as_deref());
        assert_eq!(tree(&ours), tree(&theirs));
        assert_eq!(tree(&src), before);
        if encrypted {
            let mut wrong = opts.clone();
            wrong.password = Some("wrong-password".into());
            assert!(engine::test(archive.to_str().unwrap(), &wrong).is_err());
        }
    }
}
#[test]
fn zipx_split_001_reads_in_original_and_never_replaces_parts() {
    let t = tempfile::tempdir().unwrap();
    let src = fixture(t.path());
    let before = tree(&src);
    let mut opts = EngineOptions::default();
    opts.password = Some("split-fixture".into());
    opts.volume_bytes = Some(4096);
    let archive = t.path().join("split.zipx");
    let outputs = engine::create(
        archive.to_str().unwrap(),
        &[src.to_string_lossy().into_owned()],
        "zipx",
        &opts,
    )
    .unwrap();
    assert!(outputs.len() > 2);
    assert!(outputs[0].ends_with(".001"));
    for p in &outputs {
        assert!(fs::metadata(p).unwrap().len() <= 4096);
    }
    let first = Path::new(&outputs[0]);
    engine::test(first.to_str().unwrap(), &opts).unwrap();
    let ours = t.path().join("ours");
    engine::extract(first.to_str().unwrap(), ours.to_str().unwrap(), &opts).unwrap();
    let theirs = t.path().join("theirs");
    validate_original(first, &theirs, opts.password.as_deref());
    assert_eq!(tree(&ours), tree(&theirs));
    assert_eq!(tree(&ours.join("资料目录")), before);
    assert_eq!(tree(&src), before);
    let reserved = t.path().join("collision.zipx.002");
    fs::write(&reserved, b"existing part must stay").unwrap();
    let path = t.path().join("collision.zipx");
    assert!(engine::create(
        path.to_str().unwrap(),
        &[src.to_string_lossy().into_owned()],
        "zipx",
        &opts
    )
    .is_err());
    assert_eq!(fs::read(reserved).unwrap(), b"existing part must stay");
    assert!(!t.path().join("collision.zipx.001").exists());
}
#[test]
fn zipx_store_and_cancel_do_not_damage_source() {
    let t = tempfile::tempdir().unwrap();
    let src = fixture(t.path());
    let before = tree(&src);
    let mut opts = EngineOptions::default();
    opts.level = Some(0);
    let archive = t.path().join("store.zipx");
    engine::create(
        archive.to_str().unwrap(),
        &[src.to_string_lossy().into_owned()],
        "zipx",
        &opts,
    )
    .unwrap();
    let dest = t.path().join("read");
    validate_original(&archive, &dest, None);
    assert_eq!(tree(&dest.join("资料目录")), before);
    let ctl = JobControl::new();
    ctl.cancel();
    opts.control = Some(ctl);
    let cancelled = t.path().join("cancelled.zipx");
    assert!(engine::create(
        cancelled.to_str().unwrap(),
        &[src.to_string_lossy().into_owned()],
        "zipx",
        &opts
    )
    .is_err());
    assert!(!cancelled.exists());
    assert_eq!(tree(&src), before);
}

#[test]
fn zipx_refuses_nested_output_before_mutating_source_tree() {
    let t = tempfile::tempdir().unwrap();
    let src = fixture(t.path());
    let before = tree(&src);
    let out = src.join("new/subdir/output.zipx");
    assert!(engine::create(
        out.to_str().unwrap(),
        &[src.to_string_lossy().into_owned()],
        "zipx",
        &EngineOptions::default()
    )
    .is_err());
    assert!(!src.join("new").exists());
    assert_eq!(tree(&src), before);
}
