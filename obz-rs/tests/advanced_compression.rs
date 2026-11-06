use obz_core::engine::{self, EngineOptions};
use std::{fs, path::Path};
fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn extended_methods_roundtrip_through_bundled_engine() {
    for (format, method, level) in [
        ("zip", "deflate64", 3),
        ("zip", "zstd", 3),
        ("7z", "zstd", 3),
        ("7z", "brotli", 3),
        ("7z", "lz4", 3),
        ("7z", "lz5", 3),
        ("7z", "lizard", 10),
        ("7z", "kanzi", 3),
    ] {
        let t = tempfile::tempdir().unwrap();
        let input = t.path().join("data.txt");
        let data = b"extended compression fixture\n".repeat(4096);
        fs::write(&input, &data).unwrap();
        let archive = t.path().join(format!("archive.{format}"));
        let opts = EngineOptions {
            method: Some(method.into()),
            level: Some(level),
            threads: Some(2),
            ..Default::default()
        };
        engine::create(&p(&archive), &[p(&input)], format, &opts)
            .unwrap_or_else(|e| panic!("{format}/{method}: {e}"));
        engine::test(&p(&archive), &opts).unwrap();
        engine::extract(&p(&archive), &p(&t.path().join("out")), &opts).unwrap();
        assert_eq!(
            fs::read(t.path().join("out/data.txt")).unwrap(),
            data,
            "{format}/{method}"
        );
    }
}

#[test]
fn codecs_without_a_supported_encryption_write_interface_are_rejected() {
    for encryption in ["xchacha20", "xchacha20-poly1305", "axp", "axa", "xaa"] {
        let t = tempfile::tempdir().unwrap();
        let input = t.path().join("data.txt");
        fs::write(&input, b"encrypted extended fixture").unwrap();
        let archive = t.path().join("archive.7z");
        let opts = EngineOptions {
            encryption: Some(encryption.into()),
            password: Some("transport-test-password".into()),
            threads: Some(2),
            ..Default::default()
        };
        assert!(engine::create(&p(&archive), &[p(&input)], "7z", &opts)
            .unwrap_err()
            .contains("encryption method is unsupported"));
        assert!(!archive.exists());
    }
}

#[test]
fn dictionary_solid_and_visible_headers_roundtrip() {
    for solid in [true, false] {
        let t = tempfile::tempdir().unwrap();
        let input = t.path().join("data");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("a.txt"), b"repeated content".repeat(4096)).unwrap();
        fs::write(input.join("b.txt"), b"repeated content".repeat(4096)).unwrap();
        let archive = t.path().join("archive.7z");
        let opts = EngineOptions {
            dictionary_bytes: Some(1024 * 1024),
            solid: Some(solid),
            header_encryption: Some(false),
            password: Some("settings-test-password".into()),
            threads: Some(2),
            ..Default::default()
        };
        engine::create(&p(&archive), &[p(&input)], "7z", &opts).unwrap();
        assert_eq!(
            engine::list(&p(&archive), &EngineOptions::default())
                .unwrap()
                .iter()
                .filter(|e| !e.is_dir)
                .count(),
            2
        );
        engine::extract(&p(&archive), &p(&t.path().join("out")), &opts).unwrap();
        assert_eq!(
            fs::read(t.path().join("out/data/a.txt")).unwrap(),
            fs::read(input.join("a.txt")).unwrap()
        );
    }
}

#[test]
fn advanced_settings_reject_invalid_format_combinations_without_publication() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("data.txt");
    fs::write(&input, b"content").unwrap();
    for (i, format, opts) in [
        (
            0,
            "zip",
            EngineOptions {
                solid: Some(true),
                ..Default::default()
            },
        ),
        (
            1,
            "7z",
            EngineOptions {
                dictionary_bytes: Some(0),
                ..Default::default()
            },
        ),
        (
            2,
            "7z",
            EngineOptions {
                header_encryption: Some(true),
                ..Default::default()
            },
        ),
        (
            3,
            "sfx",
            EngineOptions {
                method: Some("zstd".into()),
                ..Default::default()
            },
        ),
        (
            4,
            "7z",
            EngineOptions {
                method: Some("lz4".into()),
                dictionary_bytes: Some(1024 * 1024),
                ..Default::default()
            },
        ),
        (
            5,
            "zip",
            EngineOptions {
                level: Some(10),
                ..Default::default()
            },
        ),
    ] {
        let archive = t.path().join(format!("invalid-{i}.{format}"));
        assert!(engine::create(&p(&archive), &[p(&input)], format, &opts).is_err());
        assert!(!archive.exists());
    }
}
