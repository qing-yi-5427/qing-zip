#![cfg(windows)]
use obz_core::{
    engine::{self, EngineOptions},
    operations::{self, Request},
};
use std::{fs, io::Write, path::Path};

const INTERNET: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/input.zip\r\n";
fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
fn zone(p: &Path) -> String {
    format!("{}:Zone.Identifier", p.display())
}
fn zip(p: &Path, files: &[(&str, &[u8])]) {
    let mut out = zip::ZipWriter::new(fs::File::create(p).unwrap());
    for (name, bytes) in files {
        out.start_file(
            *name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
        out.write_all(bytes).unwrap();
    }
    out.finish().unwrap();
    fs::write(zone(p), INTERNET).unwrap();
}
fn extract(p: &Path, dest: &Path, options: EngineOptions) -> Result<usize, String> {
    operations::extract_archive(&path(p), &path(dest), &options)
}
#[test]
fn selected_smart_extract_inherits_zone_and_leaves_unselected_and_unrelated_files_alone() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("bundle.zip");
    zip(
        &archive,
        &[
            ("bundle/a.txt", b"alpha"),
            ("bundle/b.txt", b"beta"),
            ("readme.txt", b"root"),
        ],
    );
    let dest = t.path().join("out");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("unrelated.txt"), b"existing").unwrap();
    let response = operations::execute(Request {
        operation: "extract".into(),
        path: path(&archive),
        dest: path(&dest),
        smart: true,
        options: EngineOptions {
            selected: vec!["bundle/a.txt".into()],
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    assert_eq!(response["files"], 1);
    assert_eq!(fs::read(dest.join("bundle/a.txt")).unwrap(), b"alpha");
    assert_eq!(
        fs::read(zone(&dest.join("bundle/a.txt"))).unwrap(),
        INTERNET
    );
    assert!(!dest.join("bundle/b.txt").exists());
    assert!(!dest.join("readme.txt").exists());
    assert!(!Path::new(&zone(&dest.join("unrelated.txt"))).exists());
}
#[test]
fn overwrite_skip_and_rename_tag_only_newly_published_files() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("bundle.zip");
    zip(&archive, &[("a.txt", b"new")]);
    for policy in ["error", "skip", "rename", "overwrite"] {
        let dest = t.path().join(policy);
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"original").unwrap();
        fs::write(zone(&dest.join("a.txt")), b"[ZoneTransfer]\r\nZoneId=4\r\n").unwrap();
        let result = extract(
            &archive,
            &dest,
            EngineOptions {
                overwrite: policy.into(),
                ..Default::default()
            },
        );
        if policy == "error" {
            assert!(result.is_err());
            assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"original");
            assert_eq!(
                fs::read(zone(&dest.join("a.txt"))).unwrap(),
                b"[ZoneTransfer]\r\nZoneId=4\r\n"
            );
            continue;
        }
        let result = result.unwrap();
        let files: Vec<_> = fs::read_dir(&dest)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        if policy == "overwrite" {
            assert_eq!(result, 1);
            assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"new");
            assert_eq!(fs::read(zone(&dest.join("a.txt"))).unwrap(), INTERNET);
        } else {
            assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"original");
            assert_eq!(
                fs::read(zone(&dest.join("a.txt"))).unwrap(),
                b"[ZoneTransfer]\r\nZoneId=4\r\n"
            );
            if policy == "rename" {
                assert_eq!(files.len(), 2);
                let new = files
                    .iter()
                    .find(|p| p.file_name().unwrap() != "a.txt")
                    .unwrap();
                assert_eq!(fs::read(new).unwrap(), b"new");
                assert_eq!(fs::read(zone(new)).unwrap(), INTERNET);
            } else {
                assert_eq!(result, 0);
                assert_eq!(files.len(), 1);
            }
        }
    }
}
#[test]
fn corrupt_archive_publishes_no_marked_or_unmarked_file() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("broken.zip");
    zip(&archive, &[("a.txt", b"unique-crc-fixture")]);
    let mut bytes = fs::read(&archive).unwrap();
    let pos = bytes
        .windows(b"unique-crc-fixture".len())
        .position(|p| p == b"unique-crc-fixture")
        .unwrap();
    bytes[pos] ^= 1;
    fs::write(&archive, bytes).unwrap();
    let dest = t.path().join("out");
    assert!(extract(&archive, &dest, EngineOptions::default()).is_err());
    assert!(!dest.join("a.txt").exists());
}
#[test]
fn nested_archives_keep_origin_on_second_extraction() {
    let t = tempfile::tempdir().unwrap();
    let inner = t.path().join("inner.zip");
    zip(&inner, &[("a.txt", b"nested")]);
    let outer = t.path().join("outer.zip");
    zip(&outer, &[("inner.zip", &fs::read(&inner).unwrap())]);
    extract(&outer, &t.path().join("first"), EngineOptions::default()).unwrap();
    extract(
        &t.path().join("first/inner.zip"),
        &t.path().join("second"),
        EngineOptions::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(zone(&t.path().join("second/a.txt"))).unwrap(),
        INTERNET
    );
}
#[test]
fn large_file_directory_commit_and_parallel_publication_keep_zone() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("large.zip");
    let large = vec![b'x'; 9 * 1024 * 1024];
    zip(&archive, &[("large.bin", &large)]);
    extract(&archive, &t.path().join("new"), EngineOptions::default()).unwrap();
    assert_eq!(
        fs::read(zone(&t.path().join("new/large.bin"))).unwrap(),
        INTERNET
    );
    let archive = t.path().join("many.zip");
    let names: Vec<_> = (0..140).map(|i| format!("item-{i}.txt")).collect();
    let files: Vec<_> = names
        .iter()
        .map(|s| (s.as_str(), b"small".as_slice()))
        .collect();
    zip(&archive, &files);
    let dest = t.path().join("existing");
    fs::create_dir(&dest).unwrap();
    extract(&archive, &dest, EngineOptions::default()).unwrap();
    for name in names {
        assert_eq!(fs::read(zone(&dest.join(name))).unwrap(), INTERNET);
    }
}
#[test]
fn seven_zip_and_brotli_adapter_propagate_source_zone() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("input.txt");
    fs::write(&input, b"sample").unwrap();
    let archive = t.path().join("input.7z");
    engine::create(
        &path(&archive),
        &[path(&input)],
        "7z",
        &EngineOptions::default(),
    )
    .unwrap();
    fs::write(zone(&archive), INTERNET).unwrap();
    extract(&archive, &t.path().join("seven"), EngineOptions::default()).unwrap();
    assert_eq!(
        fs::read(zone(&t.path().join("seven/input.txt"))).unwrap(),
        INTERNET
    );
    let br = t.path().join("file.br");
    {
        let mut encoder =
            brotli::CompressorWriter::new(fs::File::create(&br).unwrap(), 4096, 4, 22);
        encoder.write_all(b"brotli").unwrap();
    }
    fs::write(zone(&br), INTERNET).unwrap();
    extract(&br, &t.path().join("brotli"), EngineOptions::default()).unwrap();
    assert_eq!(
        fs::read(zone(&t.path().join("brotli/file"))).unwrap(),
        INTERNET
    );
}

#[test]
fn readonly_staged_file_preserves_attribute_and_origin() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("readonly.bin");
    fs::write(&input, vec![b'x'; 9 * 1024 * 1024]).unwrap();
    let mut permissions = fs::metadata(&input).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&input, permissions).unwrap();
    let archive = t.path().join("readonly.7z");
    engine::create(
        &path(&archive),
        &[path(&input)],
        "7z",
        &EngineOptions::default(),
    )
    .unwrap();
    fs::write(zone(&archive), INTERNET).unwrap();
    let dest = t.path().join("extracted");
    extract(&archive, &dest, EngineOptions::default()).unwrap();
    let output = dest.join("readonly.bin");
    assert_eq!(fs::read(zone(&output)).unwrap(), INTERNET);
    assert!(fs::metadata(&output).unwrap().permissions().readonly());
    for file in [&input, &output] {
        let mut permissions = fs::metadata(file).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(file, permissions).unwrap();
    }
}

#[test]
fn malformed_source_metadata_fails_before_output_creation() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("input.zip");
    zip(&archive, &[("a.txt", b"safe")]);
    for (index, bytes) in [b"[ZoneTransfer]\nZoneId=wrong".to_vec(), vec![b'x'; 65537]]
        .iter()
        .enumerate()
    {
        fs::write(zone(&archive), bytes).unwrap();
        let dest = t.path().join(format!("out-{index}"));
        let error = extract(&archive, &dest, EngineOptions::default()).unwrap_err();
        assert!(error.contains("zone metadata"), "{error}");
        assert!(!dest.exists());
    }
}

#[test]
fn caller_json_cannot_disable_origin_propagation() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("input.zip");
    zip(&archive, &[("a.txt", b"safe")]);
    let options: EngineOptions =
        serde_json::from_value(serde_json::json!({"zone_identifier":null})).unwrap();
    let dest = t.path().join("out");
    extract(&archive, &dest, options).unwrap();
    assert_eq!(fs::read(zone(&dest.join("a.txt"))).unwrap(), INTERNET);
}

#[test]
fn direct_engine_api_inherits_origin_and_unmarked_source_stays_unmarked() {
    let t = tempfile::tempdir().unwrap();
    let archive = t.path().join("input.zip");
    zip(&archive, &[("a.txt", b"safe")]);
    let dest = t.path().join("marked");
    engine::extract(&path(&archive), &path(&dest), &EngineOptions::default()).unwrap();
    assert_eq!(fs::read(zone(&dest.join("a.txt"))).unwrap(), INTERNET);
    fs::remove_file(zone(&archive)).unwrap();
    let dest = t.path().join("unmarked");
    engine::extract(&path(&archive), &path(&dest), &EngineOptions::default()).unwrap();
    assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"safe");
    assert!(!Path::new(&zone(&dest.join("a.txt"))).exists());
}
