use obz_core::engine::{self, EngineOptions};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn original() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe")
}
fn verify(path: &Path, opts: &EngineOptions, expected: &[u8], dest: &Path) {
    engine::test(path.to_str().unwrap(), opts).unwrap();
    let entries = engine::list(path.to_str().unwrap(), opts).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].encrypted);
    let result = Command::new(original())
        .args(["x", "-y", "-p:synthetic-test"])
        .arg(format!("-o:{}", dest.display()))
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(dest.join("payload.txt")).unwrap(), expected);
    let mut wrong = opts.clone();
    wrong.password = Some("wrong-test-password".into());
    assert!(engine::test(path.to_str().unwrap(), &wrong).is_err());
}
fn encryption_strength(path: &Path, expected: &str) {
    let b = fs::read(path).unwrap();
    assert_eq!(&b[..4], b"PK\x03\x04");
    assert_ne!(u16::from_le_bytes(b[6..8].try_into().unwrap()) & 1, 0);
    let method = u16::from_le_bytes(b[8..10].try_into().unwrap());
    if expected == "ZipCrypto" {
        assert_ne!(method, 99);
        return;
    }
    assert_eq!(method, 99);
    let name = u16::from_le_bytes(b[26..28].try_into().unwrap()) as usize;
    let extra = u16::from_le_bytes(b[28..30].try_into().unwrap()) as usize;
    let mut pos = 30 + name;
    let end = pos + extra;
    let mut found = false;
    while pos + 4 <= end {
        let kind = u16::from_le_bytes(b[pos..pos + 2].try_into().unwrap());
        let len = u16::from_le_bytes(b[pos + 2..pos + 4].try_into().unwrap()) as usize;
        let value = &b[pos + 4..pos + 4 + len];
        if kind == 0x9901 {
            assert_eq!(&value[2..4], b"AE");
            assert_eq!(
                value[4],
                match expected {
                    "AES128" => 1,
                    "AES192" => 2,
                    _ => 3,
                }
            );
            found = true;
        }
        pos += 4 + len;
    }
    assert!(found);
}
#[test]
fn zip_algorithm_and_encryption_matrix_matches_original() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("payload.txt");
    let raw = b"compression method and encryption crosscheck\n".repeat(300);
    fs::write(&input, &raw).unwrap();
    for method in ["Deflate", "Store", "BZip2", "LZMA", "PPMd"] {
        for encryption in ["ZipCrypto", "AES128", "AES192", "AES256"] {
            let mut opts = EngineOptions::default();
            opts.password = Some("synthetic-test".into());
            opts.method = Some(method.into());
            opts.encryption = Some(encryption.into());
            let archive = t.path().join(format!("{method}-{encryption}.zip"));
            engine::create(
                archive.to_str().unwrap(),
                &[input.to_string_lossy().into_owned()],
                "zip",
                &opts,
            )
            .unwrap();
            let file = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
            let actual = file.by_index_data(0).unwrap().compression();
            let expected = match method {
                "Deflate" => zip::CompressionMethod::Deflated,
                "Store" => zip::CompressionMethod::Stored,
                "BZip2" => zip::CompressionMethod::Bzip2,
                "LZMA" => zip::CompressionMethod::Lzma,
                _ => zip::CompressionMethod::Ppmd,
            };
            assert_eq!(actual, expected);
            encryption_strength(&archive, encryption);
            verify(
                &archive,
                &opts,
                &raw,
                &t.path().join(format!("out-{method}-{encryption}")),
            );
        }
    }
    assert_eq!(fs::read(input).unwrap(), raw);
}
#[test]
fn seven_zip_algorithms_and_zipx_aes_strengths_match_original() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("payload.txt");
    let raw = b"advanced methods\n".repeat(400);
    fs::write(&input, &raw).unwrap();
    for method in ["LZMA2", "LZMA", "PPMd", "BZip2", "Copy"] {
        let mut opts = EngineOptions::default();
        opts.password = Some("synthetic-test".into());
        opts.method = Some(method.into());
        opts.encryption = Some("AES256".into());
        let path = t.path().join(format!("{method}.7z"));
        engine::create(
            path.to_str().unwrap(),
            &[input.to_string_lossy().into_owned()],
            "7z",
            &opts,
        )
        .unwrap();
        verify(&path, &opts, &raw, &t.path().join(format!("7z-{method}")));
    }
    for method in ["XZ", "Store"] {
        for encryption in ["AES128", "AES192", "AES256"] {
            let mut opts = EngineOptions::default();
            opts.password = Some("synthetic-test".into());
            opts.method = Some(method.into());
            opts.encryption = Some(encryption.into());
            let path = t.path().join(format!("{method}-{encryption}.zipx"));
            engine::create(
                path.to_str().unwrap(),
                &[input.to_string_lossy().into_owned()],
                "zipx",
                &opts,
            )
            .unwrap();
            encryption_strength(&path, encryption);
            verify(
                &path,
                &opts,
                &raw,
                &t.path().join(format!("zipx-{method}-{encryption}")),
            );
        }
    }
}
#[test]
fn illegal_method_encryption_combinations_are_rejected() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("payload.txt");
    fs::write(&input, b"source preserved").unwrap();
    for (i, (format, method, encryption, password)) in [
        ("zip", "XZ", "AES256", true),
        ("7z", "Deflate", "AES256", true),
        ("7z", "LZMA2", "ZipCrypto", true),
        ("zipx", "Deflate", "AES256", true),
        ("zipx", "XZ", "ZipCrypto", true),
        ("zip", "Deflate", "unknown", true),
        ("zip", "Deflate", "AES128", false),
        ("zipx", "XZ", "AES128", false),
        ("tar", "PPMd", "AES256", true),
    ]
    .into_iter()
    .enumerate()
    {
        let opts = EngineOptions {
            method: Some(method.into()),
            encryption: Some(encryption.into()),
            password: password.then(|| "synthetic-test".into()),
            ..EngineOptions::default()
        };
        let out = t.path().join(format!("invalid-{i}.{format}"));
        assert!(engine::create(
            out.to_str().unwrap(),
            &[input.to_string_lossy().into_owned()],
            format,
            &opts
        )
        .is_err());
        assert!(!out.exists());
    }
    assert_eq!(fs::read(input).unwrap(), b"source preserved");
    let opts: EngineOptions = serde_json::from_str("{}").unwrap();
    assert!(opts.method.is_none());
    assert!(opts.encryption.is_none());
}

#[test]
fn explicit_algorithm_with_zero_level_is_observable() {
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("payload.txt");
    fs::write(&input, b"method explicit precedence".repeat(100)).unwrap();
    for (format, method, expected) in [
        ("zip", "Deflate", zip::CompressionMethod::Deflated),
        ("zipx", "XZ", zip::CompressionMethod::Xz),
    ] {
        let out = t.path().join(format!("level0.{format}"));
        let options = EngineOptions {
            level: Some(0),
            method: Some(method.into()),
            ..EngineOptions::default()
        };
        engine::create(
            out.to_str().unwrap(),
            &[input.to_string_lossy().into_owned()],
            format,
            &options,
        )
        .unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(out).unwrap()).unwrap();
        assert_eq!(archive.by_index_data(0).unwrap().compression(), expected);
    }
    let out = t.path().join("level0.7z");
    let options = EngineOptions {
        level: Some(0),
        method: Some("LZMA2".into()),
        ..EngineOptions::default()
    };
    engine::create(
        out.to_str().unwrap(),
        &[input.to_string_lossy().into_owned()],
        "7z",
        &options,
    )
    .unwrap();
    assert!(engine::list(out.to_str().unwrap(), &options).unwrap()[0]
        .method
        .contains("LZMA2"));
}
