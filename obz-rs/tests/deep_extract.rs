#![cfg(all(windows, target_pointer_width = "64"))]
//! Owned ordinary archive fixtures only; no host security/fault probes.
use obz_core::engine::{
    self,
    deep_extract::{Session, Settings},
    EngineOptions, JobControl,
};
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
fn zip(rows: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in rows {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(bytes).unwrap();
    }
    w.finish().unwrap().into_inner()
}
fn start(root: &Path, inputs: &[&Path], options: EngineOptions) -> Session {
    Session::start(
        inputs.iter().map(|p| p.display().to_string()).collect(),
        &root.join("output").display().to_string(),
        true,
        options,
        Settings {
            recovery_directory: Some(root.join("recovery").display().to_string()),
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn prefixed_wrong_extension_chain_preserves_final_photos_directory() {
    let t = tempfile::tempdir().unwrap();
    let final_zip = zip(&[("照片/a.jpg", b"photo")]);
    let middle = zip(&[("wrapper/第二层.zip", &final_zip)]);
    let mut bytes = b"ordinary non-archive prefix\r\n".to_vec();
    bytes.extend(zip(&[("第一层.zip", &middle)]));
    let source = t.path().join("资料.jpg");
    fs::write(&source, bytes).unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert!(
        !report.partial,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        fs::read(t.path().join("output/资料/照片/a.jpg")).unwrap(),
        b"photo"
    );
    assert!(source.exists());
}
#[test]
fn mixed_contents_apply_smart_child_folder_rule() {
    let t = tempfile::tempdir().unwrap();
    let inner = zip(&[("inner/a.txt", b"a")]);
    let source = t.path().join("outer.zip");
    fs::write(
        &source,
        zip(&[("readme.txt", b"readme"), ("inner.zip", &inner)]),
    )
    .unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{} {:?}",
        serde_json::to_string(&report).unwrap(),
        report.publish_error
    );
    assert_eq!(
        fs::read(t.path().join("output/outer/inner/a.txt")).unwrap(),
        b"a"
    );
    assert!(!t.path().join("output/outer/inner/inner").exists());
    assert!(!t.path().join("output/outer/inner.zip").exists());
}
#[test]
fn selected_outputs_only_are_expanded_and_budgeted() {
    let t = tempfile::tempdir().unwrap();
    let chosen = zip(&[("kept.txt", b"chosen")]);
    let unselected = vec![0u8; 65536];
    let source = t.path().join("selected.zip");
    fs::write(
        &source,
        zip(&[("chosen.zip", &chosen), ("unselected.bin", &unselected)]),
    )
    .unwrap();
    let options = EngineOptions {
        selected: vec!["chosen.zip".into()],
        ..Default::default()
    };
    let mut s = Session::start(
        vec![source.display().to_string()],
        &t.path().join("output").display().to_string(),
        true,
        options,
        Settings {
            bytes: 4096,
            recovery_directory: Some(t.path().join("recovery").display().to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        fs::read(t.path().join("output/selected/kept.txt")).unwrap(),
        b"chosen"
    );
    assert!(report.accounted_bytes < 4096);
}
#[test]
fn visible_header_encryption_reuses_session_password_and_resume_keeps_done_work() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("secret.txt");
    fs::write(&input, b"secret").unwrap();
    let encrypted = t.path().join("locked.zip");
    engine::create(
        encrypted.to_str().unwrap(),
        &[input.display().to_string()],
        "zip",
        &EngineOptions {
            password: Some("fixture-pass".into()),
            encryption: Some("AES256".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let source = t.path().join("outer.zip");
    fs::write(
        &source,
        zip(&[("locked.zip", &fs::read(encrypted).unwrap())]),
    )
    .unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let pending = s.drive(&JobControl::new()).unwrap();
    assert_eq!(pending.pending.len(), 1);
    let spent = pending.accounted_bytes;
    s.resume(
        pending.pending[0].node,
        false,
        Some("fixture-pass".into()),
        None,
    )
    .unwrap();
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        fs::read(t.path().join("output/outer/secret.txt")).unwrap(),
        b"secret"
    );
    assert_eq!(report.accounted_bytes, spent + 6);
    let mut reused = start(
        t.path(),
        &[&source],
        EngineOptions {
            password: Some("fixture-pass".into()),
            overwrite: "rename".into(),
            ..Default::default()
        },
    );
    assert!(reused.drive(&JobControl::new()).unwrap().finished);
}
#[test]
fn application_container_and_bad_batch_input_do_not_expand_or_abort_good_input() {
    let t = tempfile::tempdir().unwrap();
    let doc = zip(&[
        ("[Content_Types].xml", b"types"),
        ("word/document.xml", b"doc"),
    ]);
    let good = t.path().join("good.zip");
    let bad = t.path().join("bad.zip");
    fs::write(&good, zip(&[("paper.docx", &doc), ("a.txt", b"a")])).unwrap();
    fs::write(&bad, b"ordinary invalid archive").unwrap();
    let mut s = start(t.path(), &[&good, &bad], Default::default());
    let paused = s.drive(&JobControl::new()).unwrap();
    assert_eq!(paused.pending.len(), 1);
    assert_eq!(paused.protected_archives, vec!["paper.docx"]);
    s.resume(paused.pending[0].node, true, None, None).unwrap();
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(report.finished);
    assert!(report.partial);
    assert_eq!(
        fs::read(t.path().join("output/good/paper.docx")).unwrap(),
        doc
    );
}
#[test]
fn publication_retry_copies_only_unresolved_files() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("retry.zip");
    fs::write(&source, zip(&[("a.txt", b"a"), ("b.txt", b"b")])).unwrap();
    let output = t.path().join("output/retry");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("b.txt"), b"existing").unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let paused = s.drive(&JobControl::new()).unwrap();
    assert!(!paused.finished);
    assert!(
        paused.publish_error.is_some(),
        "{}",
        serde_json::to_string(&paused).unwrap()
    );
    assert!(paused.recovery_path.is_some());
    s.retry_publication("rename".into()).unwrap();
    let done = s.drive(&JobControl::new()).unwrap();
    assert!(done.finished, "{} {:?}", done.summary, done.publish_error);
    assert_eq!(done.files, 2);
    assert_eq!(fs::read(output.join("a.txt")).unwrap(), b"a");
    assert!(!output.join("a (2).txt").exists());
    assert_eq!(fs::read(output.join("b.txt")).unwrap(), b"existing");
}
#[test]
fn directory_merge_retry_preserves_mapping_without_duplicate_folders() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("retry.zip");
    fs::write(
        &source,
        zip(&[("folder/a.txt", b"a"), ("folder/b.txt", b"b")]),
    )
    .unwrap();
    let output = t.path().join("output/retry/folder");
    fs::create_dir_all(&output).unwrap();
    fs::write(output.join("b.txt"), b"existing").unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let paused = s.drive(&JobControl::new()).unwrap();
    assert!(paused.publish_error.is_some());
    s.retry_publication("rename".into()).unwrap();
    let done = s.drive(&JobControl::new()).unwrap();
    assert!(done.finished, "{}", serde_json::to_string(&done).unwrap());
    assert_eq!(done.files, 2);
    assert!(!output.parent().unwrap().join("folder (2)").exists());
    assert!(!output.join("a (2).txt").exists());
}
#[test]
fn raw_gzip_tar_stream_is_bounded_and_unpacked_as_another_level() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("hello.txt");
    fs::write(&input, b"hello").unwrap();
    let source = t.path().join("data.tgz");
    engine::create(
        source.to_str().unwrap(),
        &[input.display().to_string()],
        "tgz",
        &EngineOptions::default(),
    )
    .unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        fs::read(t.path().join("output/data/hello.txt")).unwrap(),
        b"hello"
    );
    assert_eq!(report.archives, 2);
    assert!(report.accounted_bytes > 5);
}
#[test]
fn real_sevenz_inside_prefixed_jpg_then_zip_preserves_terminal_folder() {
    let t = tempfile::tempdir().unwrap();
    let inner = t.path().join("第二层.zip");
    fs::write(&inner, zip(&[("照片/a.jpg", b"photo")])).unwrap();
    let seven = t.path().join("第一层.7z");
    engine::create(
        seven.to_str().unwrap(),
        &[inner.display().to_string()],
        "7z",
        &EngineOptions::default(),
    )
    .unwrap();
    let source = t.path().join("资料.jpg");
    let mut bytes = b"prefix\r\n".to_vec();
    bytes.extend(zip(&[("第一层.7z", &fs::read(seven).unwrap())]));
    fs::write(&source, bytes).unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(
        fs::read(t.path().join("output/资料/照片/a.jpg")).unwrap(),
        b"photo"
    );
}
#[test]
fn complete_split_sevenz_selection_resolves_to_one_archive() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("payload.bin");
    let bytes = (0..16384).map(|n| (n % 251) as u8).collect::<Vec<_>>();
    fs::write(&input, &bytes).unwrap();
    let source = t.path().join("split.7z");
    engine::create(
        source.to_str().unwrap(),
        &[input.display().to_string()],
        "7z",
        &EngineOptions {
            method: Some("Copy".into()),
            volume_bytes: Some(4096),
            ..Default::default()
        },
    )
    .unwrap();
    let first = t.path().join("split.7z.001");
    let second = t.path().join("split.7z.002");
    let mut s = start(t.path(), &[&second, &first], Default::default());
    let report = s.drive(&JobControl::new()).unwrap();
    assert!(
        report.finished,
        "{}",
        serde_json::to_string(&report).unwrap()
    );
    assert_eq!(report.archives, 1);
    assert_eq!(
        fs::read(t.path().join("output/split/payload.bin")).unwrap(),
        bytes
    );
}
#[test]
fn resumed_password_is_reused_for_two_paused_sibling_archives() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("secret.txt");
    fs::write(&input, b"secret").unwrap();
    let encrypted = t.path().join("locked.zip");
    engine::create(
        encrypted.to_str().unwrap(),
        &[input.display().to_string()],
        "zip",
        &EngineOptions {
            password: Some("fixture-pass".into()),
            encryption: Some("AES256".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let encrypted = fs::read(encrypted).unwrap();
    let source = t.path().join("siblings.zip");
    fs::write(
        &source,
        zip(&[("a.zip", &encrypted), ("b.zip", &encrypted)]),
    )
    .unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let paused = s.drive(&JobControl::new()).unwrap();
    assert_eq!(paused.pending.len(), 2);
    s.resume(
        paused.pending[0].node,
        false,
        Some("fixture-pass".into()),
        None,
    )
    .unwrap();
    let done = s.drive(&JobControl::new()).unwrap();
    assert!(done.finished, "{}", serde_json::to_string(&done).unwrap());
    assert_eq!(done.accounted_bytes, paused.accounted_bytes + 12);
    for folder in ["a", "b"] {
        assert_eq!(
            fs::read(
                t.path()
                    .join("output/siblings")
                    .join(folder)
                    .join("secret.txt")
            )
            .unwrap(),
            b"secret"
        );
    }
}
#[test]
fn missing_extension_and_other_compressed_tar_formats() {
    let t = tempfile::tempdir().unwrap();
    let opaque = t.path().join("opaque");
    let original = zip(&[("a.txt", b"plain")]);
    fs::write(&opaque, &original).unwrap();
    let mut s = start(t.path(), &[&opaque], Default::default());
    assert!(s.drive(&JobControl::new()).unwrap().finished);
    assert_eq!(
        fs::read(t.path().join("output/opaque/a.txt")).unwrap(),
        b"plain"
    );
    assert_eq!(fs::read(opaque).unwrap(), original);
    let input = t.path().join("hello.txt");
    fs::write(&input, b"hello").unwrap();
    for format in ["tbz2", "txz"] {
        let source = t.path().join(format!("data-{format}.{format}"));
        engine::create(
            source.to_str().unwrap(),
            &[input.display().to_string()],
            format,
            &Default::default(),
        )
        .unwrap();
        let mut s = start(t.path(), &[&source], Default::default());
        let done = s.drive(&JobControl::new()).unwrap();
        assert!(done.finished, "{}", serde_json::to_string(&done).unwrap());
        assert_eq!(
            fs::read(t.path().join(format!("output/data-{format}/hello.txt"))).unwrap(),
            b"hello"
        );
    }
}
#[test]
fn ordinary_rar5_nested_archive_uses_existing_decoder() {
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("rar-parent.zip");
    fs::write(
        &source,
        zip(&[("payload.rar", include_bytes!("fixtures/deep/rar5-m3.rar"))]),
    )
    .unwrap();
    let mut s = start(t.path(), &[&source], Default::default());
    let done = s.drive(&JobControl::new()).unwrap();
    assert!(done.finished, "{}", serde_json::to_string(&done).unwrap());
    assert_eq!(
        fs::metadata(t.path().join("output/rar-parent/bigtext_64k.bin"))
            .unwrap()
            .len(),
        65536
    );
}
