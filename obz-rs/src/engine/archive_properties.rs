//! Archive-level metadata queried only on demand through the existing engine.
use super::*;
#[derive(Default, Serialize, Debug)]
pub struct Layer {
    pub fields: BTreeMap<String, String>,
}
#[derive(Serialize, Debug)]
pub struct Properties {
    pub layers: Vec<Layer>,
    pub source_bytes: String,
    pub partial: bool,
}
fn kind(key: &str) -> Option<u8> {
    Some(match key {
        "Type" | "Method" | "Characteristics" => 0,
        "Solid" | "Multivolume" | "Encrypted" => 1,
        "Physical Size"
        | "Total Physical Size"
        | "Tail Size"
        | "Headers Size"
        | "Volumes"
        | "Blocks" => 2,
        "Offset" => 3,
        _ => return None,
    })
}
fn parse(bytes: &[u8], source: &Path, size: u64) -> Result<Properties, String> {
    // Entry names are irrelevant here. The bounded read can end inside a
    // multibyte entry name; validate UTF-8 only through the header separator.
    let stop = bytes
        .windows(12)
        .position(|b| b == b"\n----------\n")
        .map(|p| p + 12)
        .or_else(|| {
            bytes
                .windows(13)
                .position(|b| b == b"\n----------\r\n")
                .map(|p| p + 13)
        });
    let bytes = stop.map(|end| &bytes[..end]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| "invalid archive property UTF-8")?;
    let mut layers: Vec<Layer> = Vec::new();
    let mut listing = false;
    let mut member = false;
    let mut empty_value = false;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if !listing {
            if let Some(path) = line.strip_prefix("Listing archive: ") {
                if Path::new(path) == source {
                    listing = true;
                }
            }
            continue;
        }
        // The technical protocol has no escaping for a closing brace in a
        // multiline value. Never resume after that value, including fake '--'
        // and fake fields inside archive comments. Unknown stays unknown.
        if empty_value && line == "{" {
            if layers.is_empty() {
                return Err("archive property header missing".into());
            }
            return Ok(Properties {
                layers,
                source_bytes: size.to_string(),
                partial: true,
            });
        }
        empty_value = false;
        if line == "--" {
            if layers.len() >= 16 {
                return Err("too many archive wrapper layers".into());
            }
            layers.push(Layer::default());
            member = false;
            continue;
        }
        if line == "----------" {
            if layers.is_empty() || layers.iter().any(|l| !l.fields.contains_key("Type")) {
                return Err("archive property type missing".into());
            }
            return Ok(Properties {
                layers,
                source_bytes: size.to_string(),
                partial: false,
            });
        }
        if line == "----" {
            member = true;
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        empty_value = value.is_empty();
        if member || layers.is_empty() {
            continue;
        }
        let Some(variant) = kind(key) else { continue };
        if value.is_empty() {
            continue;
        }
        let value = match variant {
            0 => {
                if value.len() > 4096 {
                    return Err("archive property exceeds limit".into());
                }
                value.to_owned()
            }
            1 => match value {
                "+" | "-" => value.to_owned(),
                _ => return Err("invalid archive boolean property".into()),
            },
            2 => value
                .parse::<u64>()
                .map_err(|_| "invalid archive size property")?
                .to_string(),
            3 => value
                .parse::<i64>()
                .map_err(|_| "invalid archive offset property")?
                .to_string(),
            _ => unreachable!(),
        };
        let layer = layers.last_mut().unwrap();
        if layer.fields.insert(key.to_owned(), value).is_some() {
            return Err("ambiguous duplicate archive property".into());
        }
    }
    Err("archive property header incomplete or too large".into())
}
pub fn read(path: &str, opts: &EngineOptions) -> Result<Properties, String> {
    let _policy = crate::machine_policy::scope();
    let ctl = control(opts);
    ctl.check()?;
    if path.contains(['\r', '\n', '\0']) {
        return Err("archive path cannot be represented in technical property listing".into());
    }
    with_zip_source(path, opts, |path| {
        let source = absolute(path)?;
        let pin = crate::engine_stream::pin(&source)?;
        let _volumes = crate::engine_stream::pin_numbered_7z_parts(&source)?;
        let size = pin.metadata().map_err(|e| e.to_string())?.len();
        let mut args = base("l", opts)?;
        if let Some(format) = crate::stream_formats::raw_format(path) {
            args.push(format!("-t{format}"));
        }
        args.extend([
            "-slt".into(),
            "-slmu".into(),
            // Technical headers still print with an exact, unmatchable member
            // selector. This avoids formatting and transferring every entry.
            "-i!:ArchiveHeaderOnly:".into(),
            "--".into(),
            source.display().to_string(),
        ]);
        ctl.set("listing", None, "读取压缩包属性");
        let log = run_technical_listing(&mut args, opts, false)?;
        let mut bytes = Vec::new();
        log.take(1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        ctl.check()?;
        let result = parse(&bytes, &source, size)?;
        ctl.set("complete", Some(100), "压缩包属性已读取");
        Ok(result)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_prefix_ignores_truncated_utf8_entry_and_cancel_never_opens_source() {
        let mut bytes = b"Listing archive: a\n--\nType = zip\n----------\n".to_vec();
        bytes.extend([0xe4, 0xb8]);
        assert_eq!(
            parse(&bytes, Path::new("a"), 1).unwrap().layers[0].fields["Type"],
            "zip"
        );
        let ctl = JobControl::new();
        ctl.cancel();
        let opts = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        assert!(read("nonexistent.zip", &opts)
            .unwrap_err()
            .contains("cancelled"));
    }
    #[test]
    fn typed_layers_preserve_large_sizes_and_unknown_values() {
        let bytes=b"Listing archive: a\n--\nPath = a\nType = Split\nVolumes = 3\nTotal Physical Size = 9007199254740993\n----\nPath = internal\nSize = 123\n--\nType = 7z\nSolid = +\nOffset = -4\nPhysical Size = 100\n----------\nType = fake-entry";
        let p = parse(bytes, Path::new("a"), 42).unwrap();
        assert_eq!(p.layers.len(), 2);
        assert_eq!(
            p.layers[0].fields["Total Physical Size"],
            "9007199254740993"
        );
        assert_eq!(p.layers[1].fields["Solid"], "+");
        assert_eq!(p.layers[1].fields["Offset"], "-4");
        assert!(!p.layers[0].fields.contains_key("Solid"));
    }
    #[test]
    fn comments_cannot_forge_property_or_wrapper_headers() {
        for fake in [
            "Solid = +",
            "--\nType = fake\nVolumes = 99",
            "}\nSolid = +\n----------",
        ] {
            let bytes = format!(
                "Listing archive: a\n--\nType = zip\nComment = \n{{\n{fake}\n}}\n----------\n"
            );
            let p = parse(bytes.as_bytes(), Path::new("a"), 1).unwrap();
            assert!(p.partial);
            assert_eq!(p.layers.len(), 1);
            assert_eq!(p.layers[0].fields.len(), 1);
        }
        assert!(parse(
            b"Listing archive: a\n--\nType = zip\nType = 7z\n----------",
            Path::new("a"),
            1
        )
        .is_err());
        assert!(parse(
            b"Listing archive: b\n--\nType = zip\n----------",
            Path::new("a"),
            1
        )
        .is_err());
    }
}
