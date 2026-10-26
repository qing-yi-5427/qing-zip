use obz_core::{
    engine::{EngineOptions, JobControl},
    stream_formats as adapter,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
fn bz() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe")
}
fn files(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, p: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for e in fs::read_dir(p).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push((
                    p.strip_prefix(root).unwrap().to_owned(),
                    fs::read(p).unwrap(),
                ));
            }
        }
    }
    let mut result = Vec::new();
    walk(path, path, &mut result);
    result.sort();
    result
}
fn original_extract(path: &Path, dest: &Path) {
    let r = Command::new(bz())
        .args(["x", "-y"])
        .arg(format!("-o:{}", dest.display()))
        .arg(path)
        .output()
        .unwrap();
    assert!(
        r.status.success(),
        "{} {}",
        String::from_utf8_lossy(&r.stdout),
        String::from_utf8_lossy(&r.stderr)
    );
}
#[test]
fn official_electron_asar_matches_original_and_enforces_integrity() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir_all(src.join("nested/empty")).unwrap();
    fs::write(src.join("中文.txt"), b"asar byte contents").unwrap();
    fs::write(src.join("nested/same.txt"), b"asar byte contents").unwrap();
    fs::write(src.join("zero"), b"").unwrap();
    let path = t.path().join("test.asar");
    let tool = PathBuf::from("G:/AI/coding/14-share/REA/node_modules/@electron/asar/bin/asar.mjs");
    let r = Command::new("node")
        .arg(tool)
        .arg("pack")
        .arg(&src)
        .arg(&path)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let opts = EngineOptions::default();
    assert!(adapter::is_supported(path.to_str().unwrap()));
    adapter::test(path.to_str().unwrap(), &opts).unwrap();
    assert_eq!(
        adapter::preview(path.to_str().unwrap(), "中文.txt", &opts).unwrap(),
        b"asar byte contents"
    );
    let ours = t.path().join("ours");
    adapter::extract(path.to_str().unwrap(), ours.to_str().unwrap(), &opts).unwrap();
    assert_eq!(files(&src), files(&ours));
    let original = t.path().join("original");
    original_extract(&path, &original);
    assert_eq!(files(&ours), files(&original));
    let mut corrupt = fs::read(&path).unwrap();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    let damaged = t.path().join("damaged.asar");
    fs::write(&damaged, corrupt).unwrap();
    let dest = t.path().join("bad");
    assert!(adapter::extract(damaged.to_str().unwrap(), dest.to_str().unwrap(), &opts).is_err());
    assert!(!dest.exists());
    let mut selected = opts.clone();
    selected.selected = vec!["中文.txt".into()];
    let dest = t.path().join("selected");
    assert_eq!(
        adapter::extract(path.to_str().unwrap(), dest.to_str().unwrap(), &selected).unwrap(),
        1
    );
    assert_eq!(files(&dest).len(), 1);
}
fn pickle(json: &str, payload: &[u8]) -> Vec<u8> {
    let padding = (4 - json.len() % 4) % 4;
    let header = 8 + json.len() + padding;
    let mut v = Vec::new();
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend_from_slice(&(header as u32).to_le_bytes());
    v.extend_from_slice(&((header - 4) as u32).to_le_bytes());
    v.extend_from_slice(&(json.len() as u32).to_le_bytes());
    v.extend_from_slice(json.as_bytes());
    v.resize(8 + header, 0);
    v.extend_from_slice(payload);
    v
}
#[test]
fn asar_rejects_paths_links_unpacked_duplicates_and_out_of_bounds() {
    let t = tempfile::tempdir().unwrap();
    for (n, json) in [
        r#"{"files":{"../escape":{"size":1,"offset":"0"}}}"#,
        r#"{"files":{"file":{"size":1,"offset":"999"}}}"#,
        r#"{"files":{"file":{"size":1,"offset":"0","unpacked":true}}}"#,
        r#"{"files":{"file":{"link":"outside"}}}"#,
        r#"{"files":{"a":{"size":1,"offset":"0"},"a":{"size":1,"offset":"0"}}}"#,
        r#"{"files":{"A":{"size":1,"offset":"0"},"a":{"size":1,"offset":"0"}}}"#,
    ]
    .iter()
    .enumerate()
    {
        let p = t.path().join(format!("{n}.asar"));
        fs::write(&p, pickle(json, b"x")).unwrap();
        assert!(
            adapter::list(p.to_str().unwrap(), &EngineOptions::default()).is_err(),
            "accepted {json}"
        );
    }
}
#[test]
fn brotli_and_lz4_match_original_and_reject_truncation() {
    let t = tempfile::tempdir().unwrap();
    let raw = b"single stream compression evidence\n".repeat(10000);
    let mut br = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut br, 4096, 5, 22);
        encoder.write_all(&raw).unwrap();
    }
    let frame = lz4_flex::frame::FrameInfo::new()
        .content_size(Some(raw.len() as u64))
        .content_checksum(true)
        .block_checksums(true);
    let mut lz4 = lz4_flex::frame::FrameEncoder::with_frame_info(frame, Vec::new());
    lz4.write_all(&raw).unwrap();
    let lz4 = lz4.finish().unwrap();
    for (ext, bytes) in [("br", br), ("lz4", lz4)] {
        let path = t.path().join(format!("data.txt.{ext}"));
        fs::write(&path, &bytes).unwrap();
        let opts = EngineOptions::default();
        assert_eq!(
            adapter::preview(path.to_str().unwrap(), "data.txt", &opts).unwrap(),
            raw
        );
        adapter::test(path.to_str().unwrap(), &opts).unwrap();
        let ours = t.path().join(format!("ours-{ext}"));
        adapter::extract(path.to_str().unwrap(), ours.to_str().unwrap(), &opts).unwrap();
        let original = t.path().join(format!("original-{ext}"));
        original_extract(&path, &original);
        assert_eq!(files(&ours), files(&original));
        for (remove, label) in [(1, "last-byte"), (4, "end-marker")] {
            let p = t.path().join(format!("bad-{label}.{ext}"));
            fs::write(&p, &bytes[..bytes.len() - remove]).unwrap();
            let dest = t.path().join(format!("bad-{label}-{ext}"));
            assert!(adapter::extract(p.to_str().unwrap(), dest.to_str().unwrap(), &opts).is_err());
            assert!(!dest.exists());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        let p = t.path().join(format!("trailing.{ext}"));
        fs::write(&p, extra).unwrap();
        assert!(adapter::test(p.to_str().unwrap(), &opts).is_err());
        let mut cancel = opts.clone();
        let ctl = JobControl::new();
        ctl.cancel();
        cancel.control = Some(ctl);
        assert!(adapter::list(path.to_str().unwrap(), &cancel).is_err());
    }
}

#[test]
fn cli_dispatches_new_stream_formats() {
    use serde_json::{json, Value};
    use std::process::Stdio;
    fn call(request: Value) -> Value {
        let mut child = Command::new(env!("CARGO_BIN_EXE_obz"))
            .arg("json")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    let t = tempfile::tempdir().unwrap();
    let asar = t.path().join("small.asar");
    fs::write(
        &asar,
        pickle(
            r#"{"files":{"hello.txt":{"size":5,"offset":"0"}}}"#,
            b"hello",
        ),
    )
    .unwrap();
    let mut br = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut br, 4096, 5, 22);
        encoder.write_all(b"hello").unwrap();
    }
    let brotli = t.path().join("hello.txt.br");
    fs::write(&brotli, br).unwrap();
    let mut lz4 = lz4_flex::frame::FrameEncoder::new(Vec::new());
    lz4.write_all(b"hello").unwrap();
    let frame = t.path().join("hello.txt.lz4");
    fs::write(&frame, lz4.finish().unwrap()).unwrap();
    for (i, path) in [asar, brotli, frame].iter().enumerate() {
        let listed = call(json!({"operation":"list","path":path}));
        assert_eq!(listed["entries"][0]["name"], "hello.txt");
        call(json!({"operation":"test","path":path}));
        let dest = t.path().join(format!("out-{i}"));
        call(json!({"operation":"extract","path":path,"dest":dest}));
        assert_eq!(fs::read(dest.join("hello.txt")).unwrap(), b"hello");
    }
}
