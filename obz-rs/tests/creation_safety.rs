use obz_core::engine::{self, EngineOptions};
use std::fs;
#[test]
fn all_directory_archive_paths_reject_source_containment_without_mutation() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    fs::create_dir_all(source.join("existing")).unwrap();
    let file = source.join("payload.txt");
    fs::write(&file, b"preserved source").unwrap();
    let before = fs::metadata(&file).unwrap().modified().unwrap();
    for format in [
        "zip", "7z", "tar", "tgz", "tar.gz", "tbz2", "tar.bz2", "txz", "tar.xz", "zipx",
    ] {
        for nested in [false, true] {
            let output = if nested {
                source.join("new/subdir/output.archive")
            } else {
                source.join("existing/output.archive")
            };
            let error = engine::create(
                output.to_str().unwrap(),
                &[source.to_string_lossy().into_owned()],
                format,
                &EngineOptions::default(),
            )
            .unwrap_err();
            assert!(error.contains("outside"), "{format}: {error}");
            assert!(!source.join("new").exists());
            assert!(!output.exists());
            assert_eq!(fs::read(&file).unwrap(), b"preserved source");
            assert_eq!(fs::metadata(&file).unwrap().modified().unwrap(), before);
        }
    }
    assert_eq!(fs::read_dir(&source).unwrap().count(), 2);
}
#[test]
fn compressed_tar_outside_source_can_create_new_output_parent() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"tar payload").unwrap();
    for format in ["tgz", "tbz2", "txz"] {
        let output = t
            .path()
            .join(format!("new-{format}/subdir/archive.{format}"));
        engine::create(
            output.to_str().unwrap(),
            &[source.to_string_lossy().into_owned()],
            format,
            &EngineOptions::default(),
        )
        .unwrap();
        let dest = t.path().join(format!("read-{format}"));
        engine::extract(
            output.to_str().unwrap(),
            dest.to_str().unwrap(),
            &EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(fs::read(dest.join("source/file")).unwrap(), b"tar payload");
    }
    assert_eq!(fs::read_dir(source).unwrap().count(), 1);
}
