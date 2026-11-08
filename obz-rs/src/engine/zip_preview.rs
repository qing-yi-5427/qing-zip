use super::*;

fn ordinary_directory(source: &mut File, control: &JobControl) -> Result<Option<u16>, String> {
    let Some(end) = zip_volume_view::single_disk_record(source)? else {
        return Ok(None);
    };
    if !(4097..=65534).contains(&end.entries)
        || end.zip64
        || end.directory_size > 8 * 1024 * 1024
        || u64::from(end.directory) + u64::from(end.directory_size) != end.end_record
    {
        return Ok(None);
    }
    let mut header = [0; 4];
    source.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header != b"PK\x03\x04" {
        return Ok(None);
    }
    source
        .seek(SeekFrom::Start(u64::from(end.directory)))
        .map_err(|e| e.to_string())?;
    let mut directory = vec![0; end.directory_size as usize];
    for chunk in directory.chunks_mut(65536) {
        control.check()?;
        source.read_exact(chunk).map_err(|e| e.to_string())?;
    }
    let mut records = directory.as_slice();
    for _ in 0..end.entries {
        control.check()?;
        if records.len() < 46 || &records[..4] != b"PK\x01\x02" {
            return Ok(None);
        }
        let name_end = 46 + usize::from(zip_u16(records, 28));
        let extra_end = name_end + usize::from(zip_u16(records, 30));
        let next = extra_end + usize::from(zip_u16(records, 32));
        if next > records.len()
            || zip_u16(records, 34) != 0
            || zip_u16(records, 8) & !0x080e != 0
            || !matches!(zip_u16(records, 10), 0 | 8)
            || zip_u32(records, 20) == u32::MAX
            || zip_u32(records, 24) == u32::MAX
            || zip_u32(records, 42) >= end.directory
            || !ordinary_zip_extra(&records[name_end..extra_end])
        {
            return Ok(None);
        }
        records = &records[next..];
    }
    Ok(records.is_empty().then_some(end.entries))
}

/// A bounded set of previews authorized by one live official listing. The
/// source and every ancestor remain pinned until the caller drops this batch.
pub struct ZipPreviewBatch {
    _source: crate::engine_stream::PinnedSource,
    policy: crate::machine_policy::Scope,
    path: PathBuf,
    options: EngineOptions,
    members: HashMap<PathBuf, (Entry, u32)>,
}

impl ZipPreviewBatch {
    pub fn open(
        path: &str,
        names: &[String],
        options: &EngineOptions,
    ) -> Result<Option<Self>, String> {
        if names.is_empty() || names.len() > 8 {
            return Err("preview batch must contain between one and eight files".into());
        }
        let policy = crate::machine_policy::scope();
        let path = absolute(path)?;
        if policy.restricted()
            || options.password.is_some()
            || options.codepage.is_some()
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            || path.with_extension("z01").exists()
        {
            return Ok(None);
        }
        let control = control(options);
        control.check()?;
        let mut source = crate::engine_stream::pin(&path)?;
        let Some(count) = ordinary_directory(&mut source, &control)? else {
            return Ok(None);
        };
        let wanted: HashSet<_> = names
            .iter()
            .map(|name| validate_name(name))
            .collect::<Result<_, _>>()?;
        let entries = {
            let _timer = PhaseTimer::new("zip-preview-list");
            let mut args = base("l", options)?;
            args.extend([
                "-tzip".into(),
                "-slt".into(),
                "-slmu".into(),
                "-ba".into(),
                "--".into(),
                path.to_str().ok_or("invalid ZIP path")?.to_owned(),
            ]);
            parse_listing(&run_technical_listing(&mut args, options, run)?)?
        };
        if entries.len() != usize::from(count)
            || entries.iter().any(|e| {
                e.encrypted
                    || !e.is_dir
                        && (!e.size_known
                            || !matches!(e.method.as_str(), "Store" | "Deflate")
                            || e.crc.len() != 8
                            || !e.crc.bytes().all(|c| c.is_ascii_hexdigit()))
            })
        {
            return Ok(None);
        }
        expected_tree(&entries)?;
        let mut members = HashMap::new();
        for entry in entries {
            control.check()?;
            if !entry.is_dir {
                let relative = validate_name(&entry.name)?;
                if wanted.contains(&relative) {
                    let crc =
                        u32::from_str_radix(&entry.crc, 16).map_err(|_| "invalid ZIP entry CRC")?;
                    members.insert(relative, (entry, crc));
                }
            }
        }
        policy.validate_output()?;
        Ok(Some(Self {
            _source: source,
            policy,
            path,
            options: options.clone(),
            members,
        }))
    }

    pub fn read(&self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        if limit == 0 || limit > 32 * 1024 * 1024 {
            return Err("invalid preview limit".into());
        }
        self.policy.validate_output()?;
        let (entry, crc) = self
            .members
            .get(&validate_name(name)?)
            .ok_or("找不到要预览的文件")?;
        if entry.size > limit {
            return Err(format!(
                "此文件超过 {} MiB 预览限制，请解压后查看",
                limit / 1024 / 1024
            ));
        }
        let bytes = crate::engine_stream::bounded_engine_preview(
            &self.path,
            "zip",
            Some(&entry.name),
            &self.options,
            entry.size,
        )?;
        if bytes.len() as u64 != entry.size || crc32fast::hash(&bytes) != *crc {
            return Err("ZIP preview size or CRC mismatch".into());
        }
        self.policy.validate_output()?;
        Ok(bytes)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, payload: &[u8], extra: &str) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let cfg = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for i in 0..4096 {
            zip.start_file(format!("f{i}"), cfg).unwrap();
        }
        zip.start_file("sample.txt", cfg).unwrap();
        zip.write_all(payload).unwrap();
        zip.start_file(extra, cfg).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn batch_holds_its_source_checks_crc_size_and_cancel_then_releases_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("source.zip");
        let names = vec!["sample.txt".into()];
        let opts = EngineOptions {
            control: Some(JobControl::new()),
            ..Default::default()
        };
        write_zip(&path, b"first payload", "empty");
        let open =
            |opts: &EngineOptions| ZipPreviewBatch::open(path.to_str().unwrap(), &names, opts);
        let batch = open(&opts).unwrap().expect("eligible single-volume ZIP");
        assert_eq!(batch.read("sample.txt", 32).unwrap(), b"first payload");
        assert!(batch.read("sample.txt", 3).is_err());
        assert!(batch.read("empty", 32).is_err());
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::rename(&path, root.path().join("moved.zip")).is_err());
        opts.control.as_ref().unwrap().cancel();
        assert!(batch
            .read("sample.txt", 32)
            .unwrap_err()
            .contains("cancelled"));
        drop(batch);
        let opts = EngineOptions::default();
        write_zip(&path, b"other payload", "empty");
        assert_eq!(
            open(&opts)
                .unwrap()
                .unwrap()
                .read("sample.txt", 32)
                .unwrap(),
            b"other payload"
        );
        let mut data = fs::read(&path).unwrap();
        let at = data
            .windows(13)
            .position(|s| s == b"other payload")
            .unwrap();
        data[at] ^= 1;
        fs::write(&path, data).unwrap();
        assert!(open(&opts)
            .unwrap()
            .unwrap()
            .read("sample.txt", 32)
            .is_err());
        assert!(open(&EngineOptions {
            codepage: Some(936),
            ..Default::default()
        })
        .unwrap()
        .is_none());
        assert!(open(&EngineOptions {
            password: Some("pw".into()),
            ..Default::default()
        })
        .unwrap()
        .is_none());
        crate::machine_policy::with_rules([&[], &["Zip"], &[], &[]], || {
            assert!(open(&opts).unwrap().is_none());
        });
        fs::write(path.with_extension("z01"), b"companion").unwrap();
        assert!(open(&opts).unwrap().is_none());
    }

    #[test]
    fn unsafe_or_duplicate_unrequested_names_reject_the_entire_batch() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bad.zip");
        for other in ["../outside", "SAMPLE.TXT"] {
            write_zip(&path, b"valid preview", other);
            assert!(ZipPreviewBatch::open(
                path.to_str().unwrap(),
                &["sample.txt".into()],
                &Default::default()
            )
            .is_err());
        }
    }
}
