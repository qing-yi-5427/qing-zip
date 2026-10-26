use obz_core::{
    engine::{self, EngineOptions},
    services,
    util::crc32,
};
use std::{fs, path::PathBuf, process::Command};
fn local(name: &str, raw: &[u8], deflate: bool, zip64: bool, descriptor: Option<bool>) -> Vec<u8> {
    let data = if deflate {
        miniz_oxide::deflate::compress_to_vec(raw, 6)
    } else {
        raw.to_vec()
    };
    let crc = crc32(raw);
    let mut h = vec![0u8; 30];
    h[..4].copy_from_slice(b"PK\x03\x04");
    h[4..6].copy_from_slice(&(if zip64 { 45u16 } else { 20 }).to_le_bytes());
    h[6..8].copy_from_slice(
        &(if descriptor.is_some() {
            0x808u16
        } else {
            0x800
        })
        .to_le_bytes(),
    );
    h[8..10].copy_from_slice(&(if deflate { 8u16 } else { 0 }).to_le_bytes());
    if descriptor.is_none() {
        h[14..18].copy_from_slice(&crc.to_le_bytes());
    }
    let packed = if zip64 {
        u32::MAX
    } else if descriptor.is_some() {
        0
    } else {
        data.len() as u32
    };
    let size = if zip64 {
        u32::MAX
    } else if descriptor.is_some() {
        0
    } else {
        raw.len() as u32
    };
    h[18..22].copy_from_slice(&packed.to_le_bytes());
    h[22..26].copy_from_slice(&size.to_le_bytes());
    h[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
    if zip64 {
        h[28..30].copy_from_slice(&20u16.to_le_bytes());
    }
    h.extend_from_slice(name.as_bytes());
    if zip64 {
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        h.extend_from_slice(&(data.len() as u64).to_le_bytes());
    }
    h.extend_from_slice(&data);
    if let Some(signature) = descriptor {
        if signature {
            h.extend_from_slice(b"PK\x07\x08");
        }
        h.extend_from_slice(&crc.to_le_bytes());
        if zip64 {
            h.extend_from_slice(&(data.len() as u64).to_le_bytes());
            h.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        } else {
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        }
    }
    h
}
#[test]
fn zip64_and_all_descriptor_layouts_crosscheck_original() {
    let t = tempfile::tempdir().unwrap();
    let raw = b"streamed payload PK\x07\x08 containing markers PK\x03\x04\n".repeat(100);
    for compressed in [false, true] {
        for zip64 in [false, true] {
            for descriptor in [None, Some(false), Some(true)] {
                let id = format!("{compressed}-{zip64}-{descriptor:?}").replace(':', "");
                let src = t.path().join(format!("{id}-broken.zip"));
                let out = t.path().join(format!("{id}-repaired.zip"));
                let mut source = local("中文.txt", &raw, compressed, zip64, descriptor);
                source.extend(local("empty", b"", compressed, zip64, descriptor));
                fs::write(&src, &source).unwrap();
                let report =
                    services::repair_zip(src.to_str().unwrap(), out.to_str().unwrap(), 1024 * 1024)
                        .unwrap();
                assert_eq!(report.recovered, vec!["中文.txt", "empty"]);
                assert!(!report.complete);
                assert_eq!(fs::read(&src).unwrap(), source);
                engine::test(out.to_str().unwrap(), &EngineOptions::default()).unwrap();
                let dest = t.path().join(id);
                engine::extract(
                    out.to_str().unwrap(),
                    dest.to_str().unwrap(),
                    &EngineOptions::default(),
                )
                .unwrap();
                assert_eq!(fs::read(dest.join("中文.txt")).unwrap(), raw);
                let original =
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe");
                let result = Command::new(original).arg("t").arg(&out).output().unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stdout)
                );
            }
        }
    }
}
#[test]
fn repair_rejects_bad_crc_truncated_descriptor_and_unsafe_paths() {
    let t = tempfile::tempdir().unwrap();
    for (i, mut bytes) in [
        local("bad", b"crc", true, true, Some(true)),
        local("truncated", b"data", false, true, Some(false)),
        local("../escape", b"no", true, true, Some(true)),
    ]
    .into_iter()
    .enumerate()
    {
        if i == 0 {
            let offset = bytes.len() - 20;
            bytes[offset] ^= 1;
        }
        if i == 1 {
            bytes.pop();
        }
        let src = t.path().join(format!("{i}.zip"));
        let out = t.path().join(format!("{i}-out.zip"));
        fs::write(&src, &bytes).unwrap();
        assert!(services::repair_zip(src.to_str().unwrap(), out.to_str().unwrap(), 10000).is_err());
        assert!(!out.exists());
        assert_eq!(fs::read(src).unwrap(), bytes);
    }
}
#[test]
fn repair_resource_limit_and_partial_recovery() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("input.zip");
    let out = t.path().join("out.zip");
    let mut bytes = local("good", b"keep", true, true, Some(false));
    bytes.extend(local("large", &vec![0; 100000], true, true, Some(true)));
    fs::write(&src, &bytes).unwrap();
    let report = services::repair_zip(src.to_str().unwrap(), out.to_str().unwrap(), 10000).unwrap();
    assert_eq!(report.recovered, vec!["good"]);
    assert!(!report.skipped.is_empty());
    assert_eq!(fs::read(src).unwrap(), bytes);
}
#[test]
fn python_nonseekable_force_zip64_interoperability() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("stream.zip");
    let out = t.path().join("recovered.zip");
    let script = r#"import io,sys,zipfile
class Sink(io.BytesIO):
 def seekable(self): return False
 def seek(self,*args): raise io.UnsupportedOperation()
s=Sink()
with zipfile.ZipFile(s,'w',compression=zipfile.ZIP_DEFLATED) as z:
 with z.open('python.txt','w',force_zip64=True) as f: f.write(b'python streaming evidence'*100)
b=s.getvalue();open(sys.argv[1],'wb').write(b[:b.index(b'PK\x01\x02')])"#;
    let r = Command::new("python")
        .args(["-c", script])
        .arg(&src)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    services::repair_zip(src.to_str().unwrap(), out.to_str().unwrap(), 100000).unwrap();
    engine::test(out.to_str().unwrap(), &EngineOptions::default()).unwrap();
}

#[test]
fn original_bandizip_zipcrypto_repair_preserves_encryption() {
    let t = tempfile::tempdir().unwrap();
    let original = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe");
    let raw = b"private synthetic test content\n".repeat(100);
    fs::write(t.path().join("secret.txt"), &raw).unwrap();
    for level in [0, 5] {
        let archive = t.path().join(format!("encrypted-{level}.zip"));
        let result = Command::new(&original)
            .current_dir(t.path())
            .args(["c", "-y", "-fmt:zip", "-p:test-only-password"])
            .arg(format!("-l:{level}"))
            .arg(&archive)
            .arg("secret.txt")
            .output()
            .unwrap();
        assert!(result.status.success());
        let mut bytes = fs::read(&archive).unwrap();
        let cd = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes.truncate(cd);
        fs::write(&archive, &bytes).unwrap();
        let out = t.path().join(format!("fixed-{level}.zip"));
        assert!(services::repair_zip_with_password(
            archive.to_str().unwrap(),
            out.to_str().unwrap(),
            100000,
            Some("wrong")
        )
        .is_err());
        assert!(!out.exists());
        let report = services::repair_zip_with_password(
            archive.to_str().unwrap(),
            out.to_str().unwrap(),
            100000,
            Some("test-only-password"),
        )
        .unwrap();
        assert_eq!(report.recovered, vec!["secret.txt"]);
        assert_eq!(fs::read(&archive).unwrap(), bytes);
        let mut opts = EngineOptions::default();
        opts.password = Some("test-only-password".into());
        engine::test(out.to_str().unwrap(), &opts).unwrap();
        assert!(engine::list(out.to_str().unwrap(), &opts).unwrap()[0].encrypted);
        let dest = t.path().join(format!("extracted-{level}"));
        engine::extract(out.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
        assert_eq!(fs::read(dest.join("secret.txt")).unwrap(), raw);
        let result = Command::new(&original)
            .args(["t", "-p:test-only-password"])
            .arg(&out)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
}

fn encrypt_fixture(mut local: Vec<u8>, descriptor: bool, zip64: bool) -> Vec<u8> {
    // Fixture encryptor; independent external readers validate emitted ciphertext.
    let name = u16::from_le_bytes(local[26..28].try_into().unwrap()) as usize;
    let extra = u16::from_le_bytes(local[28..30].try_into().unwrap()) as usize;
    let start = 30 + name + extra;
    let tail = if descriptor {
        if zip64 {
            24
        } else {
            16
        }
    } else {
        0
    };
    let end = local.len() - tail;
    let mut clear = vec![0u8; 12];
    clear[11] = if descriptor { local[11] } else { local[17] };
    clear.extend_from_slice(&local[start..end]);
    let mut keys = [0x12345678u32, 0x23456789, 0x34567890];
    fn crc(mut c: u32, b: u8) -> u32 {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xedb88320
            } else {
                c >> 1
            };
        }
        c
    }
    fn update(k: &mut [u32; 3], b: u8) {
        k[0] = crc(k[0], b);
        k[1] = k[1]
            .wrapping_add(k[0] & 255)
            .wrapping_mul(134775813)
            .wrapping_add(1);
        k[2] = crc(k[2], (k[1] >> 24) as u8);
    }
    for b in b"fixture-password" {
        update(&mut keys, *b);
    }
    let ciphertext: Vec<u8> = clear
        .into_iter()
        .map(|b| {
            let x = (keys[2] | 2) & 65535;
            let out = b ^ ((x.wrapping_mul(x ^ 1) >> 8) as u8);
            update(&mut keys, b);
            out
        })
        .collect();
    local[6] |= 1;
    let packed = (ciphertext.len()) as u64;
    if zip64 {
        local[start - 8..start].copy_from_slice(&packed.to_le_bytes());
    } else if !descriptor {
        local[18..22].copy_from_slice(&(packed as u32).to_le_bytes());
    }
    let mut out = local[..start].to_vec();
    out.extend_from_slice(&ciphertext);
    out.extend_from_slice(&local[end..]);
    if descriptor {
        let d = out.len() - tail + 8;
        if zip64 {
            out[d..d + 8].copy_from_slice(&packed.to_le_bytes());
        } else {
            out[d..d + 4].copy_from_slice(&(packed as u32).to_le_bytes());
        }
    }
    out
}
#[test]
fn encrypted_zip64_descriptor_combinations_remain_encrypted() {
    let t = tempfile::tempdir().unwrap();
    for compressed in [false, true] {
        for zip64 in [false, true] {
            for descriptor in [false, true] {
                let id = format!("{compressed}-{zip64}-{descriptor}");
                let source = encrypt_fixture(
                    local(
                        "secret.txt",
                        b"encrypted fixture content",
                        compressed,
                        zip64,
                        if descriptor { Some(true) } else { None },
                    ),
                    descriptor,
                    zip64,
                );
                let src = t.path().join(format!("{id}.zip"));
                let out = t.path().join(format!("{id}-fixed.zip"));
                fs::write(&src, &source).unwrap();
                services::repair_zip_with_password(
                    src.to_str().unwrap(),
                    out.to_str().unwrap(),
                    100000,
                    Some("fixture-password"),
                )
                .unwrap();
                let mut opts = EngineOptions::default();
                opts.password = Some("fixture-password".into());
                engine::test(out.to_str().unwrap(), &opts).unwrap();
                assert!(engine::list(out.to_str().unwrap(), &opts).unwrap()[0].encrypted);
                let dest = t.path().join(id);
                engine::extract(out.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
                assert_eq!(
                    fs::read(dest.join("secret.txt")).unwrap(),
                    b"encrypted fixture content"
                );
            }
        }
    }
}

#[test]
fn aes_repair_authenticates_and_preserves_ciphertext() {
    let t = tempfile::tempdir().unwrap();
    let seven = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/7zip/7z.exe");
    let raw = b"AES repair synthetic private payload\n".repeat(70);
    fs::write(t.path().join("secret.txt"), &raw).unwrap();
    for method in ["Copy", "Deflate"] {
        for strength in ["AES128", "AES192", "AES256"] {
            let id = format!("{method}-{strength}");
            let src = t.path().join(format!("{id}.zip"));
            let result = Command::new(&seven)
                .current_dir(t.path())
                .args(["a", "-tzip", "-p:synthetic-only"])
                .arg(format!("-mem={strength}"))
                .arg(format!("-mm={method}"))
                .arg(&src)
                .arg("secret.txt")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            // 7zip's password argument includes every character after -p, including ':'.
            let mut bytes = fs::read(&src).unwrap();
            let cd = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
            bytes.truncate(cd);
            fs::write(&src, &bytes).unwrap();
            let out = t.path().join(format!("{id}-fixed.zip"));
            assert!(services::repair_zip_with_password(
                src.to_str().unwrap(),
                out.to_str().unwrap(),
                100000,
                Some("wrong")
            )
            .is_err());
            assert!(!out.exists());
            services::repair_zip_with_password(
                src.to_str().unwrap(),
                out.to_str().unwrap(),
                100000,
                Some(":synthetic-only"),
            )
            .unwrap();
            assert_eq!(fs::read(&src).unwrap(), bytes);
            let mut opts = EngineOptions::default();
            opts.password = Some(":synthetic-only".into());
            engine::test(out.to_str().unwrap(), &opts).unwrap();
            assert!(engine::list(out.to_str().unwrap(), &opts).unwrap()[0].encrypted);
            let original =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe");
            let r = Command::new(&original)
                .args(["t", "-p::synthetic-only"])
                .arg(&out)
                .output()
                .unwrap();
            assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
            for zip64 in [false, true] {
                for signature in [false, true] {
                    let mut streamed = bytes.clone();
                    let name = u16::from_le_bytes(streamed[26..28].try_into().unwrap()) as usize;
                    let extra = u16::from_le_bytes(streamed[28..30].try_into().unwrap()) as usize;
                    let start = 30 + name + extra;
                    let packed = u32::from_le_bytes(streamed[18..22].try_into().unwrap());
                    let size = u32::from_le_bytes(streamed[22..26].try_into().unwrap());
                    let crc = u32::from_le_bytes(streamed[14..18].try_into().unwrap());
                    streamed[6] |= 8;
                    streamed[14..18].fill(0);
                    if zip64 {
                        streamed[4..6].copy_from_slice(&51u16.to_le_bytes());
                        streamed[18..26].fill(255);
                        streamed[28..30].copy_from_slice(&((extra + 20) as u16).to_le_bytes());
                        let mut z = vec![1, 0, 16, 0];
                        z.extend_from_slice(&(size as u64).to_le_bytes());
                        z.extend_from_slice(&(packed as u64).to_le_bytes());
                        streamed.splice(start..start, z);
                    } else {
                        streamed[18..26].fill(0);
                    }
                    if signature {
                        streamed.extend_from_slice(b"PK\x07\x08");
                    }
                    streamed.extend_from_slice(&crc.to_le_bytes());
                    if zip64 {
                        streamed.extend_from_slice(&(packed as u64).to_le_bytes());
                        streamed.extend_from_slice(&(size as u64).to_le_bytes());
                    } else {
                        streamed.extend_from_slice(&packed.to_le_bytes());
                        streamed.extend_from_slice(&size.to_le_bytes());
                    }
                    let input = t
                        .path()
                        .join(format!("{id}-{zip64}-{signature}-stream.zip"));
                    let output = t
                        .path()
                        .join(format!("{id}-{zip64}-{signature}-stream-fixed.zip"));
                    fs::write(&input, &streamed).unwrap();
                    services::repair_zip_with_password(
                        input.to_str().unwrap(),
                        output.to_str().unwrap(),
                        100000,
                        Some(":synthetic-only"),
                    )
                    .unwrap();
                    engine::test(output.to_str().unwrap(), &opts).unwrap();
                    let r = Command::new(&original)
                        .args(["t", "-p::synthetic-only"])
                        .arg(&output)
                        .output()
                        .unwrap();
                    assert!(r.status.success());
                    assert_eq!(fs::read(input).unwrap(), streamed);
                }
            }
            let corrupt = t.path().join(format!("{id}-corrupt.zip"));
            let badout = t.path().join(format!("{id}-bad-out.zip"));
            let last = bytes.len() - 1;
            bytes[last] ^= 0x80;
            fs::write(&corrupt, &bytes).unwrap();
            assert!(services::repair_zip_with_password(
                corrupt.to_str().unwrap(),
                badout.to_str().unwrap(),
                100000,
                Some(":synthetic-only")
            )
            .is_err());
            assert!(!badout.exists());
        }
    }
}

fn legacy_filename(mut record: Vec<u8>, raw: &[u8]) -> Vec<u8> {
    let n = u16::from_le_bytes(record[26..28].try_into().unwrap()) as usize;
    record.splice(30..30 + n, raw.iter().copied());
    record[26..28].copy_from_slice(&(raw.len() as u16).to_le_bytes());
    let flags = u16::from_le_bytes(record[6..8].try_into().unwrap()) & !0x800;
    record[6..8].copy_from_slice(&flags.to_le_bytes());
    record
}
#[test]
fn legacy_names_normalize_to_utf8_in_plain_zipcrypto_and_aes() {
    let t = tempfile::tempdir().unwrap();
    let original = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extracted/bz.x64.exe");
    let seven = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/7zip/7z.exe");
    fs::write(t.path().join("source.txt"), b"legacy filename test").unwrap();
    let aes = t.path().join("aes.zip");
    assert!(Command::new(&seven)
        .current_dir(t.path())
        .args(["a", "-tzip", "-pfixture-password", "-mem=AES256"])
        .arg(&aes)
        .arg("source.txt")
        .output()
        .unwrap()
        .status
        .success());
    let mut aes = fs::read(aes).unwrap();
    let end = aes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    aes.truncate(end);
    for (id, raw, name, cp) in [
        (
            "gbk",
            b"\xd6\xd0\xce\xc4.txt".as_slice(),
            "中文.txt",
            Some(936),
        ),
        ("cp437", b"caf\x82.txt".as_slice(), "café.txt", None),
    ] {
        for mode in 0..3 {
            let data = match mode {
                0 => local(
                    "source.txt",
                    b"legacy filename test",
                    true,
                    true,
                    Some(true),
                ),
                1 => encrypt_fixture(
                    local(
                        "source.txt",
                        b"legacy filename test",
                        true,
                        true,
                        Some(true),
                    ),
                    true,
                    true,
                ),
                _ => aes.clone(),
            };
            let bytes = legacy_filename(data, raw);
            let source = t.path().join(format!("{id}-{mode}.zip"));
            let output = t.path().join(format!("{id}-{mode}-fixed.zip"));
            fs::write(&source, &bytes).unwrap();
            let password = if mode == 0 {
                None
            } else {
                Some("fixture-password")
            };
            let report = services::repair_zip_with_options(
                source.to_str().unwrap(),
                output.to_str().unwrap(),
                100000,
                password,
                cp,
            )
            .unwrap();
            assert_eq!(report.recovered, vec![name]);
            assert_eq!(fs::read(&source).unwrap(), bytes);
            let dest = t.path().join(format!("{id}-{mode}-original"));
            let result = Command::new(&original)
                .args(["x", "-y", "-p:fixture-password"])
                .arg(format!("-o:{}", dest.display()))
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert_eq!(fs::read(dest.join(name)).unwrap(), b"legacy filename test");
            let header = fs::read(&output).unwrap();
            assert_ne!(
                u16::from_le_bytes(header[6..8].try_into().unwrap()) & 0x800,
                0
            );
        }
    }
}
#[test]
fn utf8_flag_and_invalid_legacy_encoding_are_strict() {
    let t = tempfile::tempdir().unwrap();
    for flag in [false, true] {
        let mut bytes = legacy_filename(local("old.txt", b"data", false, false, None), b"\x81.txt");
        if flag {
            bytes[7] |= 8;
        }
        let src = t.path().join(format!("{flag}.zip"));
        let out = t.path().join(format!("{flag}-out.zip"));
        fs::write(&src, bytes).unwrap();
        assert!(services::repair_zip_with_options(
            src.to_str().unwrap(),
            out.to_str().unwrap(),
            10000,
            None,
            Some(65001)
        )
        .is_err());
        assert!(!out.exists());
    }
}
