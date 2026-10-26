use obz_core::{
    archive_comment,
    engine::{self, EngineOptions, JobControl},
};
use std::{fs, path::Path};
fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn make(t: &tempfile::TempDir, encrypted: bool) -> (String, EngineOptions) {
    let source = t.path().join("data.txt");
    fs::write(&source, b"preserve encrypted member bytes").unwrap();
    let out = p(&t.path().join("archive.zip"));
    let opts = EngineOptions {
        password: encrypted.then(|| "test-password".into()),
        ..Default::default()
    };
    engine::create(&out, &[p(&source)], "zip", &opts).unwrap();
    (out, opts)
}
#[test]
fn plain_and_aes_comments_preserve_member_bytes() {
    for encrypted in [false, true] {
        let t = tempfile::tempdir().unwrap();
        let (path, opts) = make(&t, encrypted);
        let before = fs::read(&path).unwrap();
        archive_comment::set(&path, "中文 archive comment", &opts).unwrap();
        assert_eq!(archive_comment::get(&path).unwrap(), "中文 archive comment");
        let after = fs::read(&path).unwrap();
        assert_eq!(&before[..before.len() - 2], &after[..before.len() - 2]);
        engine::test(&path, &opts).unwrap();
        archive_comment::set(&path, "", &opts).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
#[test]
fn zip64_end_records_survive_comment_update() {
    let t = tempfile::tempdir().unwrap();
    let (path, opts) = make(&t, false);
    let mut bytes = fs::read(&path).unwrap();
    let pos = bytes.len() - 22;
    let end = bytes.split_off(pos);
    let count = u16::from_le_bytes(end[10..12].try_into().unwrap()) as u64;
    let size = u32::from_le_bytes(end[12..16].try_into().unwrap()) as u64;
    let offset = u32::from_le_bytes(end[16..20].try_into().unwrap()) as u64;
    let mut z = b"PK\x06\x06".to_vec();
    z.extend(44u64.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend(45u16.to_le_bytes());
    z.extend([0u8; 8]);
    z.extend(count.to_le_bytes());
    z.extend(count.to_le_bytes());
    z.extend(size.to_le_bytes());
    z.extend(offset.to_le_bytes());
    assert_eq!(z.len(), 56);
    bytes.extend(z);
    bytes.extend(b"PK\x06\x07");
    bytes.extend(0u32.to_le_bytes());
    bytes.extend((pos as u64).to_le_bytes());
    bytes.extend(1u32.to_le_bytes());
    let mut end = end;
    end[8..12].fill(255);
    end[12..20].fill(255);
    bytes.extend(end);
    fs::write(&path, &bytes).unwrap();
    engine::test(&path, &opts).unwrap();
    archive_comment::set(&path, "ZIP64注释", &opts).unwrap();
    assert_eq!(archive_comment::get(&path).unwrap(), "ZIP64注释");
    assert_eq!(
        &fs::read(&path).unwrap()[..bytes.len() - 2],
        &bytes[..bytes.len() - 2]
    );
    engine::test(&path, &opts).unwrap();
}
#[test]
fn invalid_multivolume_oversized_cancel_leave_source_unchanged() {
    let t = tempfile::tempdir().unwrap();
    let (path, opts) = make(&t, false);
    let before = fs::read(&path).unwrap();
    assert!(archive_comment::set(&path, &"界".repeat(21846), &opts).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let ctl = JobControl::new();
    ctl.cancel();
    assert!(archive_comment::set(
        &path,
        "cancel",
        &EngineOptions {
            control: Some(ctl),
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut split = before;
    let n = split.len();
    split[n - 18] = 1;
    fs::write(&path, &split).unwrap();
    assert!(archive_comment::set(&path, "no", &opts)
        .unwrap_err()
        .contains("multi-volume"));
    assert_eq!(fs::read(&path).unwrap(), split);
    fs::write(&path, b"not a zip").unwrap();
    assert!(archive_comment::set(&path, "no", &opts).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not a zip");
}
#[test]
fn embedded_eocd_signature_and_maximum_comment() {
    let t = tempfile::tempdir().unwrap();
    let (path, opts) = make(&t, false);
    let fake = String::from_utf8([b"PK\x05\x06".as_slice(), &[0u8; 18]].concat()).unwrap();
    archive_comment::set(&path, &fake, &opts).unwrap();
    assert_eq!(archive_comment::get(&path).unwrap(), fake);
    let max = "a".repeat(65535);
    archive_comment::set(&path, &max, &opts).unwrap();
    assert_eq!(archive_comment::get(&path).unwrap(), max);
}
