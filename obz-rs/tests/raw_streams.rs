use obz_core::{
    engine::{self, EngineOptions, JobControl},
    operations, stream_formats,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const FORMATS: &[(&str, &str, u8)] = &[
    ("brotli", "br", 1),
    ("lz4", "lz4", 1),
    ("lz5", "lz5", 1),
    ("lizard", "liz", 10),
    ("zstd", "zst", 3),
];
fn p(path: &Path) -> String {
    path.display().to_string()
}
fn options(level: u8) -> EngineOptions {
    EngineOptions {
        level: Some(level),
        threads: Some(2),
        ..Default::default()
    }
}
fn reference(verb: &str, format: &str, path: &Path) -> std::process::Output {
    let mut command = Command::new(engine::executable().unwrap());
    command
        .args([verb, "-y", "-sccUTF-8"])
        .arg(format!("-t{format}"));
    if verb == "x" {
        command.arg("-so");
    }
    command.arg("--").arg(path).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().unwrap()
}
fn hash(path: &Path) -> Vec<u8> {
    let mut file = fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    hash.finalize().to_vec()
}

#[test]
fn entry_size_known_is_backward_compatible_and_roundtrips_unknown() {
    let old = serde_json::json!({"name":"file", "size":12, "packed_size":8,
        "is_dir":false,"crc":"","encrypted":false,"modified":"","method":"","comment":"","attributes":""});
    let mut entry: engine::Entry = serde_json::from_value(old).unwrap();
    assert!(entry.size_known);
    entry.size = 0;
    entry.size_known = false;
    let restored: engine::Entry =
        serde_json::from_value(serde_json::to_value(entry).unwrap()).unwrap();
    assert!(!restored.size_known);
    assert_eq!(restored.size, 0);
}

#[test]
fn all_raw_formats_roundtrip_reference_unknown_metadata_and_alias_dispatch() {
    for &(format, extension, level) in FORMATS {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let data = b"raw streaming fixture\n".repeat(8192);
        let source = root.join("data.txt");
        fs::write(&source, &data).unwrap();
        let archive = root.join(format!("data.txt.{extension}"));
        let opts = options(level);
        operations::execute(serde_json::from_value(serde_json::json!({"operation":"create","path":archive,"inputs":[source],"options":opts})).unwrap()).unwrap();
        let entries = operations::list_archive(&p(&archive), &opts).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "data.txt");
        assert!(!entries[0].size_known);
        assert!(reference("t", format, &archive).status.success());
        let original = reference("x", format, &archive);
        assert!(original.status.success());
        assert_eq!(original.stdout, data);
        operations::test_archive(&p(&archive), &opts).unwrap();
        assert_eq!(
            operations::preview_archive(&p(&archive), "data.txt", &opts).unwrap(),
            data
        );
        let dest = root.join("new-target");
        assert_eq!(
            operations::extract_archive(&p(&archive), &p(&dest), &opts).unwrap(),
            1
        );
        assert_eq!(fs::read(dest.join("data.txt")).unwrap(), data);
        let renamed = match format {
            "brotli" => Some("brotli"),
            "lizard" => Some("lizard"),
            "zstd" => Some("zstd"),
            _ => None,
        };
        if let Some(extension) = renamed {
            let alias = root.join(format!("alias.txt.{extension}"));
            fs::copy(&archive, &alias).unwrap();
            assert!(stream_formats::is_supported(&p(&alias)));
            assert_eq!(
                operations::preview_archive(&p(&alias), "alias.txt", &opts).unwrap(),
                data
            );
        }
    }
}

#[test]
fn truncated_and_trailing_streams_never_publish_output() {
    for &(format, extension, level) in FORMATS {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("data");
        fs::write(&source, b"raw data integrity\n".repeat(1000)).unwrap();
        let archive = root.join(format!("data.{extension}"));
        let opts = options(level);
        engine::create(&p(&archive), &[p(&source)], format, &opts).unwrap();
        let original = fs::read(&archive).unwrap();
        for (label, bytes) in [
            ("truncated", original[..original.len() - 1].to_vec()),
            ("trailing", [original.as_slice(), b"\0"].concat()),
        ] {
            let bad = root.join(format!("{label}.{extension}"));
            fs::write(&bad, bytes).unwrap();
            assert!(
                operations::test_archive(&p(&bad), &opts).is_err(),
                "{format}/{label}"
            );
            let dest = root.join(format!("output-{label}"));
            assert!(
                operations::extract_archive(&p(&bad), &p(&dest), &opts).is_err(),
                "{format}/{label}"
            );
            assert!(
                !dest.exists(),
                "bad stream created output: {format}/{label}"
            );
            assert!(engine::preview_raw_stream(&p(&bad), label, &opts, 32 * 1024 * 1024).is_err());
        }
    }
}

#[test]
fn overwrite_selection_cancel_and_motw_preserve_single_stream_contract() {
    for &(format, extension, level) in FORMATS {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("data.txt");
        fs::write(&source, b"new data").unwrap();
        let archive = root.join(format!("data.txt.{extension}"));
        let opts = options(level);
        engine::create(&p(&archive), &[p(&source)], format, &opts).unwrap();
        #[cfg(windows)]
        fs::write(
            format!("{}:Zone.Identifier", archive.display()),
            b"[ZoneTransfer]\r\nZoneId=3\r\n",
        )
        .unwrap();
        let dest = root.join("existing");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("data.txt"), b"old").unwrap();
        assert!(operations::extract_archive(&p(&archive), &p(&dest), &opts).is_err());
        assert_eq!(fs::read(dest.join("data.txt")).unwrap(), b"old");
        let mut configured = opts.clone();
        configured.overwrite = "skip".into();
        assert_eq!(
            operations::extract_archive(&p(&archive), &p(&dest), &configured).unwrap(),
            0
        );
        configured.overwrite = "rename".into();
        assert_eq!(
            operations::extract_archive(&p(&archive), &p(&dest), &configured).unwrap(),
            1
        );
        assert_eq!(fs::read(dest.join("data (1).txt")).unwrap(), b"new data");
        configured.overwrite = "overwrite".into();
        assert_eq!(
            operations::extract_archive(&p(&archive), &p(&dest), &configured).unwrap(),
            1
        );
        assert_eq!(fs::read(dest.join("data.txt")).unwrap(), b"new data");
        #[cfg(windows)]
        assert_eq!(
            fs::read(format!(
                "{}:Zone.Identifier",
                dest.join("data.txt").display()
            ))
            .unwrap(),
            b"[ZoneTransfer]\r\nZoneId=3\r\n"
        );
        configured.selected = vec!["missing".into()];
        assert!(
            operations::extract_archive(&p(&archive), &p(&root.join("missing")), &configured)
                .is_err()
        );
        let ctl = JobControl::new();
        ctl.cancel();
        configured.control = Some(ctl);
        configured.selected.clear();
        assert!(operations::extract_archive(
            &p(&archive),
            &p(&root.join("cancelled")),
            &configured
        )
        .unwrap_err()
        .contains("cancelled"));
        assert!(!root.join("cancelled").exists());
    }
}

#[test]
fn previews_stop_at_requested_limit_for_unknown_lengths() {
    for &(format, extension, level) in FORMATS {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("text.txt");
        let bytes = vec![b'a'; 3 * 1024 * 1024];
        fs::write(&source, &bytes).unwrap();
        let archive = root.join(format!("text.txt.{extension}"));
        let opts = options(level);
        engine::create(&p(&archive), &[p(&source)], format, &opts).unwrap();
        assert!(!engine::list(&p(&archive), &opts).unwrap()[0].size_known);
        let before = Instant::now();
        assert!(
            engine::preview_raw_stream(&p(&archive), "text.txt", &opts, 2 * 1024 * 1024)
                .unwrap_err()
                .contains("limit")
        );
        assert!(before.elapsed() < Duration::from_secs(10));
        assert_eq!(
            engine::preview_raw_stream(&p(&archive), "text.txt", &opts, 32 * 1024 * 1024).unwrap(),
            bytes
        );
    }
}

#[test]
fn raw_options_and_file_count_fail_before_output_publication() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let input = root.join("input");
    fs::write(&input, b"fixture").unwrap();
    for &(format, extension, level) in FORMATS {
        let base = options(level);
        let bad = [
            EngineOptions {
                password: Some("synthetic".into()),
                ..base.clone()
            },
            EngineOptions {
                encryption: Some("aes256".into()),
                ..base.clone()
            },
            EngineOptions {
                volume_bytes: Some(4096),
                ..base.clone()
            },
            EngineOptions {
                solid: Some(false),
                ..base.clone()
            },
            EngineOptions {
                header_encryption: Some(false),
                ..base.clone()
            },
            EngineOptions {
                dictionary_bytes: Some(4096),
                ..base.clone()
            },
            EngineOptions {
                codepage: Some(65001),
                ..base.clone()
            },
            EngineOptions {
                method: Some("lzma2".into()),
                ..base.clone()
            },
        ];
        for (index, opts) in bad.iter().enumerate() {
            let archive = root.join(format!("bad-{format}-{index}.{extension}"));
            assert!(engine::create(&p(&archive), &[p(&input)], format, opts).is_err());
            assert!(!archive.exists());
        }
        let archive = root.join(format!("multiple.{extension}"));
        assert!(engine::create(&p(&archive), &[p(&input), p(&input)], format, &base).is_err());
        assert!(!archive.exists());
        assert!(engine::create(&p(&archive), &[p(root)], format, &base).is_err());
        assert!(!archive.exists());
        assert!(engine::create(
            &p(&root.join("misleading.bin")),
            &[p(&input)],
            format,
            &base
        )
        .is_err());
    }
}

#[test]
fn effective_codec_level_bounds_apply_to_raw_and_sevenz_without_clamping() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = root.join("input");
    fs::write(&source, b"level fixture").unwrap();
    for &(method, extension, min, max) in &[
        ("zstd", "zst", 0, 9),
        ("brotli", "br", 0, 11),
        ("lz4", "lz4", 1, 12),
        ("lz5", "lz5", 1, 15),
        ("lizard", "liz", 10, 49),
    ] {
        for format in [method, "7z"] {
            for level in [min, max] {
                let ext = if format == "7z" { "7z" } else { extension };
                let archive = root.join(format!("{format}-{method}-{level}.{ext}"));
                let opts = EngineOptions {
                    method: Some(method.into()),
                    level: Some(level),
                    threads: Some(1),
                    ..Default::default()
                };
                engine::create(&p(&archive), &[p(&source)], format, &opts).unwrap();
                engine::test(&p(&archive), &opts).unwrap();
            }
            for level in [min.checked_sub(1), Some(max + 1)].into_iter().flatten() {
                let ext = if format == "7z" { "7z" } else { extension };
                let archive = root.join(format!("bad-{format}-{method}-{level}.{ext}"));
                let opts = EngineOptions {
                    method: Some(method.into()),
                    level: Some(level),
                    ..Default::default()
                };
                assert!(engine::create(&p(&archive), &[p(&source)], format, &opts)
                    .unwrap_err()
                    .contains("compression level"));
                assert!(!archive.exists());
            }
        }
    }
}

#[test]
#[ignore = "513 MiB physical random input, all five codecs; run explicitly for size/memory acceptance"]
#[cfg(windows)]
fn raw_streams_larger_than_old_512_mib_limit_remain_streaming() {
    #[link(name = "Bcrypt")]
    extern "system" {
        fn BCryptGenRandom(
            handle: *mut std::ffi::c_void,
            buffer: *mut u8,
            size: u32,
            flags: u32,
        ) -> i32;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let source = root.join("large.bin");
    let mut file = fs::File::create(&source).unwrap();
    let mut block = vec![0; 1024 * 1024];
    for _ in 0..513 {
        assert_eq!(
            unsafe {
                BCryptGenRandom(
                    std::ptr::null_mut(),
                    block.as_mut_ptr(),
                    block.len() as u32,
                    2,
                )
            },
            0
        );
        file.write_all(&block).unwrap();
    }
    drop(file);
    let expected = hash(&source);
    let size = fs::metadata(&source).unwrap().len();
    for &(format, extension, level) in FORMATS {
        let archive = root.join(format!("large.bin.{extension}"));
        let opts = options(level);
        engine::create(&p(&archive), &[p(&source)], format, &opts).unwrap();
        assert!(fs::metadata(&archive).unwrap().len() > 512 * 1024 * 1024);
        let before = Instant::now();
        let entries = engine::list(&p(&archive), &opts).unwrap();
        assert!(!entries[0].size_known);
        assert!(
            before.elapsed() < Duration::from_secs(5),
            "list decoded the large stream"
        );
        engine::test(&p(&archive), &opts).unwrap();
        assert!(
            engine::preview_raw_stream(&p(&archive), "large.bin", &opts, 32 * 1024 * 1024)
                .unwrap_err()
                .contains("limit")
        );
        let dest = root.join("large-output");
        engine::extract(&p(&archive), &p(&dest), &opts).unwrap();
        assert_eq!(fs::metadata(dest.join("large.bin")).unwrap().len(), size);
        assert_eq!(hash(&dest.join("large.bin")), expected);
        // Targets are exact owned paths inside this test TempDir, never arbitrary input.
        fs::remove_file(dest.join("large.bin")).unwrap();
        fs::remove_dir(&dest).unwrap();
        let ctl = JobControl::new();
        let cancel = ctl.clone();
        let mut cancelled = opts.clone();
        cancelled.control = Some(ctl);
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            cancel.cancel();
        });
        assert!(engine::extract(&p(&archive), &p(&dest), &cancelled)
            .unwrap_err()
            .contains("cancelled"));
        thread.join().unwrap();
        assert!(!dest.exists());
        fs::remove_file(&archive).unwrap();
    }
    // The old plain Brotli route has no MT framing. It must also remain a
    // single streaming pass beyond the former packed/decoded 512 MiB caps.
    let archive = root.join("large.bin.br");
    {
        let mut encoder =
            brotli::CompressorWriter::new(fs::File::create(&archive).unwrap(), 65536, 0, 22);
        std::io::copy(&mut fs::File::open(&source).unwrap(), &mut encoder).unwrap();
    }
    assert!(fs::metadata(&archive).unwrap().len() > 512 * 1024 * 1024);
    let opts = EngineOptions::default();
    assert!(!engine::list(&p(&archive), &opts).unwrap()[0].size_known);
    let dest = root.join("native-output");
    engine::extract(&p(&archive), &p(&dest), &opts).unwrap();
    assert_eq!(hash(&dest.join("large.bin")), expected);
    assert!(
        engine::preview_raw_stream(&p(&archive), "large.bin", &opts, 12345)
            .unwrap_err()
            .contains("limit")
    );
}
