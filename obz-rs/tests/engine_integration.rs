use engine::{EngineOptions, JobControl};
use obz_core::engine;
use std::{fs, path::Path};
fn p(path: &Path) -> String {
    path.display().to_string()
}
fn inputs(t: &tempfile::TempDir) -> Vec<String> {
    fs::create_dir_all(t.path().join("data/empty")).unwrap();
    fs::write(t.path().join("data/中文.txt"), b"hello archive").unwrap();
    fs::write(t.path().join("data/second.txt"), b"another file").unwrap();
    vec![p(&t.path().join("data"))]
}
#[test]
fn engine_zip_and_7z_encryption_roundtrip() {
    for format in ["zip", "7z"] {
        let t = tempfile::tempdir().unwrap();
        let input = inputs(&t);
        let out = p(&t.path().join(format!("a.{format}")));
        let opts = EngineOptions {
            password: Some("test-password-123".into()),
            threads: Some(2),
            ..Default::default()
        };
        engine::create(&out, &input, format, &opts).unwrap();
        engine::test(&out, &opts).unwrap();
        let entries = engine::list(&out, &opts).unwrap();
        assert!(entries
            .iter()
            .any(|e| e.name == "data/中文.txt" || e.name == "data\\中文.txt"));
        engine::extract(&out, &p(&t.path().join("out")), &opts).unwrap();
        assert_eq!(
            fs::read(t.path().join("out/data/中文.txt")).unwrap(),
            b"hello archive"
        );
        assert!(t.path().join("out/data/empty").is_dir());
        let wrong = EngineOptions {
            password: Some("wrong".into()),
            ..Default::default()
        };
        assert!(engine::test(&out, &wrong).is_err());
    }
}
#[test]
fn engine_select_preview_and_overwrite() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let out = p(&t.path().join("a.zip"));
    let opts = EngineOptions::default();
    engine::create(&out, &input, "zip", &opts).unwrap();
    let name = engine::list(&out, &opts)
        .unwrap()
        .into_iter()
        .find(|e| e.name.ends_with("中文.txt"))
        .unwrap()
        .name;
    assert_eq!(
        engine::preview(&out, &name, &opts).unwrap(),
        b"hello archive"
    );
    let selected = EngineOptions {
        selected: vec![name],
        ..Default::default()
    };
    let dest = p(&t.path().join("out"));
    assert_eq!(engine::extract(&out, &dest, &selected).unwrap(), 1);
    assert!(!t.path().join("out/data/second.txt").exists());
    assert!(engine::extract(&out, &dest, &selected).is_err());
    let skip = EngineOptions {
        overwrite: "skip".into(),
        ..selected.clone()
    };
    assert_eq!(engine::extract(&out, &dest, &skip).unwrap(), 0);
    let rename = EngineOptions {
        overwrite: "rename".into(),
        ..selected
    };
    assert_eq!(engine::extract(&out, &dest, &rename).unwrap(), 1);
    assert!(t.path().join("out/data/中文 (1).txt").exists());
}
#[test]
fn engine_update_preserves_original_on_failure() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let out = p(&t.path().join("a.zip"));
    let opts = EngineOptions::default();
    engine::create(&out, &input, "zip", &opts).unwrap();
    let original = fs::read(&out).unwrap();
    assert!(engine::update(&out, "add", &[p(&t.path().join("missing"))], &opts).is_err());
    assert_eq!(fs::read(&out).unwrap(), original);
    engine::update(
        &out,
        "rename",
        &["data/second.txt".into(), "data/renamed.txt".into()],
        &opts,
    )
    .unwrap();
    engine::update(&out, "delete", &["data/renamed.txt".into()], &opts).unwrap();
    assert!(!engine::list(&out, &opts)
        .unwrap()
        .iter()
        .any(|e| e.name.ends_with("renamed.txt")));
    fs::write(t.path().join("added.txt"), b"added").unwrap();
    engine::update(&out, "add", &[p(&t.path().join("added.txt"))], &opts).unwrap();
    assert_eq!(engine::preview(&out, "added.txt", &opts).unwrap(), b"added");
}
#[test]
fn engine_tar_and_single_stream_formats() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let opts = EngineOptions::default();
    let tar = p(&t.path().join("a.tar"));
    engine::create(&tar, &input, "tar", &opts).unwrap();
    engine::extract(&tar, &p(&t.path().join("untar")), &opts).unwrap();
    assert!(t.path().join("untar/data/empty").is_dir());
    for format in ["gzip", "bzip2", "xz"] {
        let out = p(&t.path().join(format!("a.{format}")));
        engine::create(&out, &[tar.clone()], format, &opts).unwrap();
        engine::test(&out, &opts).unwrap();
        engine::extract(&out, &p(&t.path().join(format)), &opts).unwrap();
    }
}
#[test]
fn engine_split_archives() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("random.bin");
    fs::write(
        &src,
        (0..50000u32)
            .flat_map(|i| i.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let opts = EngineOptions {
        volume_bytes: Some(1024),
        level: Some(0),
        ..Default::default()
    };
    let out = p(&t.path().join("split.7z"));
    let volumes = engine::create(&out, &[p(&src)], "7z", &opts).unwrap();
    assert!(volumes.len() > 1);
    let read = EngineOptions::default();
    engine::test(&volumes[0], &read).unwrap();
    engine::extract(&volumes[0], &p(&t.path().join("out")), &read).unwrap();
    assert_eq!(
        fs::read(t.path().join("out/random.bin")).unwrap(),
        fs::read(src).unwrap()
    );
}
#[test]
fn engine_cancel_before_operation() {
    let c = JobControl::new();
    c.cancel();
    let opts = EngineOptions {
        control: Some(c.clone()),
        ..Default::default()
    };
    assert!(engine::list("missing.zip", &opts)
        .unwrap_err()
        .contains("cancelled"));
}
#[test]
fn engine_sfx_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let opts = EngineOptions::default();
    let out = p(&t.path().join("archive.exe"));
    engine::create(&out, &input, "sfx", &opts).unwrap();
    engine::test(&out, &opts).unwrap();
    engine::extract(&out, &p(&t.path().join("out")), &opts).unwrap();
    assert_eq!(
        fs::read(t.path().join("out/data/中文.txt")).unwrap(),
        b"hello archive"
    );
}

#[test]
fn engine_compound_tar_one_step_and_metadata() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let opts = EngineOptions::default();
    let file = t.path().join("data/中文.txt");
    let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1700000000);
    fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(stamp))
        .unwrap();
    for format in ["tgz", "tbz2", "txz"] {
        let out = p(&t.path().join(format!("archive.{format}")));
        engine::create(&out, &input, format, &opts).unwrap();
        assert!(engine::list(&out, &opts)
            .unwrap()
            .iter()
            .any(|e| e.name.ends_with("中文.txt")));
        let dest = t.path().join(format);
        engine::extract(&out, &p(&dest), &opts).unwrap();
        assert_eq!(
            fs::read(dest.join("data/中文.txt")).unwrap(),
            b"hello archive"
        );
        assert_eq!(
            fs::metadata(dest.join("data/中文.txt"))
                .unwrap()
                .modified()
                .unwrap(),
            stamp
        );
    }
}
#[test]
fn engine_cancels_running_compression_without_output() {
    let t = tempfile::tempdir().unwrap();
    let mut state = 1u64;
    let mut data = vec![0u8; 16 * 1024 * 1024];
    for b in &mut data {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *b = state as u8;
    }
    let src = t.path().join("noise.bin");
    fs::write(&src, data).unwrap();
    let control = JobControl::new();
    let worker = control.clone();
    let opts = EngineOptions {
        control: Some(control),
        level: Some(9),
        threads: Some(1),
        ..Default::default()
    };
    let out = p(&t.path().join("cancelled.7z"));
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        worker.cancel();
    });
    let result = engine::create(&out, &[p(&src)], "7z", &opts);
    stopper.join().unwrap();
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(!Path::new(&out).exists());
}
#[test]
fn engine_reads_original_bandizip_formats() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/original-bandizip");
    let expected = fs::read(root.join("hello.txt")).unwrap();
    let opts = EngineOptions::default();
    for name in ["created.zipx", "created.lzh", "libarchive.iso"] {
        let path = p(&root.join(name));
        engine::test(&path, &opts).unwrap();
        let list = engine::list(&path, &opts).unwrap();
        let entry = list
            .iter()
            .find(|e| !e.is_dir && e.name.to_lowercase().ends_with("hello.txt"))
            .unwrap();
        assert_eq!(
            engine::preview(&path, &entry.name, &opts).unwrap(),
            expected
        );
    }
    let split = p(&root.join("split.zip"));
    engine::test(&split, &opts).unwrap();
    let list = engine::list(&split, &opts).unwrap();
    let entry = list.iter().find(|e| !e.is_dir).unwrap();
    assert_eq!(
        engine::preview(&split, &entry.name, &opts).unwrap(),
        fs::read(root.join("noise.bin")).unwrap()
    );
}

#[test]
fn engine_rejects_inconsistent_original_iso() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/original-bandizip/created.iso");
    assert!(engine::test(&p(&path), &EngineOptions::default()).is_err());
}
fn stored_zip(name: &str, content: &[u8]) -> Vec<u8> {
    let mut z = Vec::new();
    let crc = obz_core::util::crc32(content);
    z.extend_from_slice(b"PK\x03\x04");
    for n in [20u16, 0x800, 0, 0, 0] {
        z.extend(n.to_le_bytes())
    }
    z.extend(crc.to_le_bytes());
    z.extend((content.len() as u32).to_le_bytes());
    z.extend((content.len() as u32).to_le_bytes());
    z.extend((name.len() as u16).to_le_bytes());
    z.extend(0u16.to_le_bytes());
    z.extend(name.as_bytes());
    z.extend(content);
    let offset = z.len();
    z.extend(b"PK\x01\x02");
    for n in [20u16, 20, 0x800, 0, 0, 0] {
        z.extend(n.to_le_bytes())
    }
    z.extend(crc.to_le_bytes());
    z.extend((content.len() as u32).to_le_bytes());
    z.extend((content.len() as u32).to_le_bytes());
    z.extend((name.len() as u16).to_le_bytes());
    for _ in 0..4 {
        z.extend(0u16.to_le_bytes())
    }
    z.extend(0u32.to_le_bytes());
    z.extend(0u32.to_le_bytes());
    z.extend(name.as_bytes());
    let cd_size = z.len() - offset;
    z.extend(b"PK\x05\x06");
    for n in [0u16, 0, 1, 1] {
        z.extend(n.to_le_bytes())
    }
    z.extend((cd_size as u32).to_le_bytes());
    z.extend((offset as u32).to_le_bytes());
    z.extend(0u16.to_le_bytes());
    z
}
#[test]
fn engine_rejects_traversal_and_ambiguous_names() {
    for name in [
        "../escape.txt",
        "folder/../../escape.txt",
        "normal\nPath = ../escape.txt",
    ] {
        let t = tempfile::tempdir().unwrap();
        let archive = t.path().join("bad.zip");
        fs::write(&archive, stored_zip(name, b"bad")).unwrap();
        assert!(engine::extract(
            &p(&archive),
            &p(&t.path().join("out")),
            &EngineOptions::default()
        )
        .is_err());
        assert!(!t.path().join("escape.txt").exists());
    }
}
#[test]
fn engine_treats_switchlike_names_as_data() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("-sdel.txt");
    fs::write(&input, b"keep").unwrap();
    let out = p(&t.path().join("a.zip"));
    let opts = EngineOptions::default();
    engine::create(&out, &[p(&input)], "zip", &opts).unwrap();
    assert!(input.exists());
    assert_eq!(engine::preview(&out, "-sdel.txt", &opts).unwrap(), b"keep");
}
#[cfg(windows)]
#[test]
fn engine_rejects_destination_junction() {
    let t = tempfile::tempdir().unwrap();
    let outside = t.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let out = t.path().join("link");
    let result = std::process::Command::new("cmd.exe")
        .args(["/C", "mklink", "/J", &p(&out), &p(&outside)])
        .output()
        .unwrap();
    assert!(result.status.success());
    let archive = t.path().join("bad.zip");
    fs::write(&archive, stored_zip("file.txt", b"bad")).unwrap();
    assert!(engine::extract(&p(&archive), &p(&out), &EngineOptions::default()).is_err());
    assert!(!outside.join("file.txt").exists());
    fs::remove_dir(out).unwrap();
}

#[test]
fn engine_creates_zipx() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let opts = EngineOptions::default();
    for format in ["zipx"] {
        let path = p(&t.path().join(format!("new.{format}")));
        engine::create(&path, &input, format, &opts).unwrap();
        engine::test(&path, &opts).unwrap();
        let list = engine::list(&path, &opts).unwrap();
        let entry = list
            .iter()
            .find(|e| e.name.ends_with("中文.txt"))
            .unwrap_or_else(|| panic!("listing: {:?}", list));
        assert_eq!(
            engine::preview(&path, &entry.name, &opts).unwrap(),
            b"hello archive"
        );
        if format == "zipx" {
            assert!(entry.method.to_lowercase().contains("xz"));
        }
    }
}
#[test]
#[ignore = "streams a sparse 4 GiB file; run explicitly for ZIP64 acceptance"]
fn engine_zip64_larger_than_four_gib() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("large.bin");
    let size = 4 * 1024 * 1024 * 1024u64 + 1;
    let file = fs::File::create(&src).unwrap();
    file.set_len(size).unwrap();
    drop(file);
    let opts = EngineOptions {
        level: Some(1),
        threads: Some(2),
        ..Default::default()
    };
    let out = p(&t.path().join("large.zip"));
    engine::create(&out, &[p(&src)], "zip", &opts).unwrap();
    let entries = engine::list(&out, &opts).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].size, size);
    engine::test(&out, &opts).unwrap();
    let bytes = fs::read(&out).unwrap();
    let cd = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    assert_eq!(
        u32::from_le_bytes(bytes[cd + 24..cd + 28].try_into().unwrap()),
        u32::MAX
    );
    let name_len = u16::from_le_bytes(bytes[cd + 28..cd + 30].try_into().unwrap()) as usize;
    let extra = cd + 46 + name_len;
    assert_eq!(
        u16::from_le_bytes(bytes[extra..extra + 2].try_into().unwrap()),
        1
    );
}

#[test]
fn engine_legacy_codepage_override() {
    let t = tempfile::tempdir().unwrap();
    let mut bytes = stored_zip("aaaa.txt", b"legacy");
    let name = [0xd6, 0xd0, 0xce, 0xc4, b'.', b't', b'x', b't'];
    bytes[6..8].copy_from_slice(&0u16.to_le_bytes());
    bytes[30..38].copy_from_slice(&name);
    let cd = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    bytes[cd + 8..cd + 10].copy_from_slice(&0u16.to_le_bytes());
    bytes[cd + 46..cd + 54].copy_from_slice(&name);
    let out = p(&t.path().join("legacy.zip"));
    fs::write(&out, bytes).unwrap();
    let opts = EngineOptions {
        codepage: Some(936),
        ..Default::default()
    };
    assert_eq!(engine::list(&out, &opts).unwrap()[0].name, "中文.txt");
    assert_eq!(engine::preview(&out, "中文.txt", &opts).unwrap(), b"legacy");
}

#[test]
fn engine_rejects_output_inside_input_tree() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let out = p(&t.path().join("data/new.zip"));
    assert!(
        engine::create(&out, &input, "zip", &EngineOptions::default())
            .unwrap_err()
            .contains("outside")
    );
    assert!(!Path::new(&out).exists());
}

#[test]
fn engine_concurrent_updates_preserve_both_additions() {
    let t = tempfile::tempdir().unwrap();
    let input = inputs(&t);
    let archive = p(&t.path().join("updates.zip"));
    engine::create(&archive, &input, "zip", &EngineOptions::default()).unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut handles = Vec::new();
    for i in 0..2 {
        let src = t.path().join(format!("added-{i}.txt"));
        fs::write(&src, format!("content-{i}")).unwrap();
        let archive = archive.clone();
        let gate = gate.clone();
        handles.push(std::thread::spawn(move || {
            gate.wait();
            engine::update(&archive, "add", &[p(&src)], &EngineOptions::default()).unwrap();
        }));
    }
    gate.wait();
    for handle in handles {
        handle.join().unwrap();
    }
    let entries = engine::list(&archive, &EngineOptions::default()).unwrap();
    for i in 0..2 {
        assert!(entries.iter().any(|e| e.name == format!("added-{i}.txt")));
    }
}
