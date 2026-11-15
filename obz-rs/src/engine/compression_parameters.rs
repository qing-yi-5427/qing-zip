use super::EngineOptions;

/// Deliberately reject out-of-range values instead of relying on encoder clamping.
/// NanaZip CompressDialog::SetOrder2 / DeflateConst / LzmaEnc / PpmdEncoder.
pub(crate) fn switches(format: &str, opts: &EngineOptions) -> Result<Vec<String>, String> {
    let mut switches = Vec::new();
    let method = opts
        .method
        .as_deref()
        .unwrap_or(if format == "zip" { "Deflate" } else { "LZMA2" })
        .to_ascii_lowercase();
    if let Some(bytes) = opts.solid_block_bytes {
        if !matches!(format, "7z" | "sfx") {
            return Err("固实块大小仅支持7z或SFX".into());
        }
        if bytes == 0 {
            return Err("固实块大小必须大于0".into());
        }
        if opts.solid == Some(false) {
            return Err("关闭固实压缩时不能设置固实块大小".into());
        }
        if opts.level == Some(0) || matches!(method.as_str(), "store" | "copy") {
            return Err("仅存储模式不能设置固实块大小".into());
        }
        switches.push(format!("-ms={bytes}b"));
    }
    if let Some(word) = opts.word_size {
        if !matches!(format, "zip" | "7z" | "sfx") {
            return Err("字长设置仅支持ZIP、7z或SFX".into());
        }
        if opts.level == Some(0) {
            return Err("仅存储模式不能设置字长".into());
        }
        let (property, min, max) = match method.as_str() {
            "lzma" | "lzma2" => ("fb", 5, 273),
            "deflate" if format == "zip" => ("fb", 3, 258),
            "deflate64" if format == "zip" => ("fb", 3, 257),
            "ppmd" => ("o", 2, if format == "zip" { 16 } else { 32 }),
            _ => return Err("所选压缩方法不支持字长设置".into()),
        };
        if !(min..=max).contains(&word) {
            return Err(format!("当前压缩方法的字长范围是{min}至{max}"));
        }
        switches.push(format!("-m{property}={word}"));
    }
    Ok(switches)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::*;
    use std::{fs, path::Path, process::Command};

    #[test]
    fn invalid_combinations_are_rejected_before_creating_output() {
        let t = tempfile::tempdir().unwrap();
        let input = t.path().join("in.txt");
        fs::write(&input, b"data").unwrap();
        for (format, options) in [
            (
                "zip",
                EngineOptions {
                    solid_block_bytes: Some(1),
                    ..Default::default()
                },
            ),
            (
                "7z",
                EngineOptions {
                    solid_block_bytes: Some(0),
                    ..Default::default()
                },
            ),
            (
                "7z",
                EngineOptions {
                    solid_block_bytes: Some(1024),
                    solid: Some(false),
                    ..Default::default()
                },
            ),
            (
                "zip",
                EngineOptions {
                    word_size: Some(259),
                    ..Default::default()
                },
            ),
            (
                "zip",
                EngineOptions {
                    method: Some("Deflate64".into()),
                    word_size: Some(258),
                    ..Default::default()
                },
            ),
            (
                "7z",
                EngineOptions {
                    word_size: Some(274),
                    ..Default::default()
                },
            ),
            (
                "7z",
                EngineOptions {
                    word_size: Some(4),
                    ..Default::default()
                },
            ),
            (
                "zip",
                EngineOptions {
                    method: Some("PPMd".into()),
                    word_size: Some(17),
                    ..Default::default()
                },
            ),
            (
                "7z",
                EngineOptions {
                    method: Some("PPMd".into()),
                    word_size: Some(33),
                    ..Default::default()
                },
            ),
            (
                "7z",
                EngineOptions {
                    method: Some("ZSTD".into()),
                    word_size: Some(32),
                    ..Default::default()
                },
            ),
            (
                "zip",
                EngineOptions {
                    level: Some(0),
                    word_size: Some(32),
                    ..Default::default()
                },
            ),
            (
                "zipx",
                EngineOptions {
                    word_size: Some(32),
                    ..Default::default()
                },
            ),
            (
                "tgz",
                EngineOptions {
                    word_size: Some(32),
                    ..Default::default()
                },
            ),
        ] {
            let out = t.path().join(format!("bad.{format}"));
            assert!(create(
                out.to_str().unwrap(),
                &[input.display().to_string()],
                format,
                &options
            )
            .is_err());
            assert!(!out.exists());
        }
        assert!(switches("7z", &EngineOptions::default())
            .unwrap()
            .is_empty());
    }

    fn compressed_bytes(path: &Path, format: &str) -> Vec<u8> {
        let all = fs::read(path).unwrap();
        if format == "zip" {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(&all)).unwrap();
            let f = z.by_index(0).unwrap();
            all[f.data_start().unwrap() as usize
                ..(f.data_start().unwrap() + f.compressed_size()) as usize]
                .to_vec()
        } else {
            // Data begins after 32-byte start header. Exclude next metadata header.
            let size = u64::from_le_bytes(all[12..20].try_into().unwrap()) as usize;
            all[32..32 + size].to_vec()
        }
    }

    #[test]
    fn real_word_size_outputs_match_official_parameters_and_change_compressed_bytes() {
        for (format, method, prop, low, high) in [
            ("zip", "Deflate", "fb", 8, 258),
            ("zip", "Deflate64", "fb", 8, 257),
            ("zip", "LZMA", "fb", 8, 273),
            ("7z", "LZMA", "fb", 8, 273),
            ("7z", "LZMA2", "fb", 8, 273),
            ("zip", "PPMd", "o", 2, 16),
            ("7z", "PPMd", "o", 2, 32),
        ] {
            let t = tempfile::tempdir().unwrap();
            let input = t.path().join("input.txt");
            let bytes = (0..12000)
                .map(|i| {
                    format!(
                        "prefix-{i:04}-{}-tail-{}\n",
                        i % 127,
                        "abcdefghijklmnop".repeat(i % 17 + 1)
                    )
                })
                .collect::<String>()
                .into_bytes();
            fs::write(&input, &bytes).unwrap();
            let mut variants = Vec::new();
            for word in [low, high] {
                let out = t.path().join(format!("qing-{word}.{format}"));
                let reference = t.path().join(format!("nana-{word}.{format}"));
                let opts = EngineOptions {
                    method: Some(method.into()),
                    word_size: Some(word),
                    threads: Some(1),
                    level: Some(5),
                    ..Default::default()
                };
                create(
                    out.to_str().unwrap(),
                    &[input.display().to_string()],
                    format,
                    &opts,
                )
                .unwrap();
                test(out.to_str().unwrap(), &opts).unwrap();
                let status = Command::new(executable().unwrap())
                    .args([
                        "a",
                        "-y",
                        "-spd",
                        "-mx=5",
                        "-mmt=1",
                        &format!("-t{format}"),
                        &format!("{}={method}", if format == "zip" { "-mm" } else { "-m0" }),
                        &format!("-m{prop}={word}"),
                        "--",
                    ])
                    .arg(&reference)
                    .arg(&input)
                    .output()
                    .unwrap();
                assert!(
                    status.status.success(),
                    "{}",
                    String::from_utf8_lossy(&status.stderr)
                );
                let payload = compressed_bytes(&out, format);
                assert_eq!(
                    payload,
                    compressed_bytes(&reference, format),
                    "{format}/{method}/{word}"
                );
                let dest = t.path().join(format!("out-{word}"));
                extract(out.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
                assert_eq!(fs::read(dest.join("input.txt")).unwrap(), bytes);
                variants.push(payload);
            }
            assert_ne!(
                variants[0], variants[1],
                "{format}/{method} did not respond to word size"
            );
        }
    }

    #[test]
    fn real_solid_block_limit_changes_block_count_and_preserves_content() {
        for format in ["7z", "sfx"] {
            let t = tempfile::tempdir().unwrap();
            let inputs = t.path().join("inputs");
            fs::create_dir(&inputs).unwrap();
            let bytes = vec![b'a'; 700_000];
            for i in 0..4 {
                fs::write(inputs.join(format!("{i}.txt")), &bytes).unwrap();
            }
            let mut counts = Vec::new();
            for limit in [1_048_576, 16_777_216] {
                let out = t.path().join(format!(
                    "limit-{limit}.{}",
                    if format == "sfx" { "exe" } else { "7z" }
                ));
                let opts = EngineOptions {
                    solid_block_bytes: Some(limit),
                    word_size: Some(64),
                    threads: Some(1),
                    ..Default::default()
                };
                create(
                    out.to_str().unwrap(),
                    &[inputs.display().to_string()],
                    format,
                    &opts,
                )
                .unwrap();
                test(out.to_str().unwrap(), &opts).unwrap();
                let output = Command::new(executable().unwrap())
                    .args(["l", "-slt", "--"])
                    .arg(&out)
                    .output()
                    .unwrap();
                assert!(output.status.success());
                let text = String::from_utf8_lossy(&output.stdout);
                let blocks = text
                    .lines()
                    .find_map(|l| l.strip_prefix("Blocks = "))
                    .unwrap()
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                counts.push(blocks);
                let dest = t.path().join(format!("out-{limit}"));
                extract(out.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
                for i in 0..4 {
                    assert_eq!(
                        fs::read(dest.join(format!("inputs/{i}.txt"))).unwrap(),
                        bytes
                    );
                }
            }
            assert_eq!(counts, vec![4, 1]);
        }
    }
}
