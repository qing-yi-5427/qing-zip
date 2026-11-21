//! Read ordinary compressed TAR directly from the verified decoder pipe.
use super::*;

pub(super) fn codec(path: &str) -> Option<&'static str> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some("gzip")
    } else if [".tar.bz2", ".tbz", ".tbz2"].iter().any(|s| lower.ends_with(s)) {
        Some("bzip2")
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Some("xz")
    } else {
        None
    }
}

pub(super) fn has_ustar_header(mut file: &File) -> Result<bool, String> {
    if file.metadata().map_err(|error| error.to_string())?.len() < 512 {
        return Ok(false);
    }
    let mut block = [0u8; 512];
    file.rewind().and_then(|_| file.read_exact(&mut block)).map_err(|error| error.to_string())?;
    file.rewind().map_err(|error| error.to_string())?;
    let checksum = block[..148].iter().chain(&block[156..]).map(|byte| u32::from(*byte)).sum::<u32>() + 8 * 32;
    Ok(matches!(&block[257..263], b"ustar\0" | b"ustar ")
        && tar::Header::from_byte_slice(&block).cksum().is_ok_and(|value| value == checksum))
}

#[derive(Clone, Copy)]
pub(super) enum Request<'a> {
    List,
    Test,
    Preview(&'a Path, u64),
}
pub(super) struct Listing {
    pub(super) entries: Vec<Entry>,
    pub(super) bytes: Option<Vec<u8>>,
}

pub(super) fn scan(path: &str, opts: &EngineOptions, request: Request<'_>) -> Result<Option<Listing>, String> {
    let policy = crate::machine_policy::scope();
    if policy.restricted() || opts.codepage.is_some() {
        return Ok(None);
    }
    let format = codec(path).ok_or("unsupported compressed TAR container")?;
    let input = absolute(path)?;
    let _source = crate::engine_stream::pin(&input)?;
    let ctl = control(opts);
    ctl.set("decompressing", None, "Reading compressed TAR stream");
    let mut args = base("x", opts)?;
    args.extend([format!("-t{format}"), "-so".into(), "--".into(), input.display().to_string()]);
    let (reader, writer) = match directory_capture::pipe() {
        Ok(pipe) => pipe,
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let result = thread::scope(|scope| -> Result<Option<Listing>, String> {
        let decoder = scope.spawn(move || run(&args, None, opts, Some(writer)));
        let parsed = scan_stream(std::io::BufReader::with_capacity(128 * 1024, reader), &ctl, request);
        let status = decoder.join().map_err(|_| "compressed TAR decoder stopped unexpectedly".to_owned())?;
        ctl.check()?;
        let Some(parsed) = parsed? else {
            // The reader closed the pipe after an extended TAR header. The
            // compatibility path performs its own fully checked decode.
            return Ok(None);
        };
        status?;
        Ok(Some(parsed))
    })?;
    policy.unchanged()?;
    Ok(result)
}

#[cfg(test)]
thread_local! {
    static BEFORE_TAR_ENCODER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn create_stream(
    output: &Path, inputs: &[String], codec: &str, opts: &EngineOptions,
) -> Result<Option<Vec<String>>, String> {
    let policy = crate::machine_policy::scope();
    // Timestamp preservation requires the regular writer's seekable wrapper
    // metadata and source snapshot semantics.
    if policy.restricted() || output.to_str().and_then(self::codec) != Some(codec)
        || opts.store_atime.is_some() || opts.store_ctime.is_some()
        || opts.store_mtime.is_some() || opts.archive_timestamp != ArchiveTimestamp::Default
    {
        return Ok(None);
    }
    let (reader, writer) = match directory_capture::pipe() {
        Ok(pipe) => pipe,
        Err(error) if error.kind() == std::io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let paths = input_paths(inputs)?;
    let ctl = control(opts);
    ctl.check()?;
    no_links(output)?;
    if output.exists() {
        return Err("output archive already exists; use update to modify it".into());
    }
    let parent = output.parent().ok_or("output has no parent")?;
    reject_output_in_inputs(parent, &paths)?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    reject_output_in_inputs(parent, &paths)?;
    let stage = tempfile::tempdir_in(parent).map_err(|error| error.to_string())?;
    let candidate = stage.path().join(output.file_name().ok_or("output has no filename")?);
    let mut tar = base("a", opts)?;
    compression(&mut tar, "tar", opts)?;
    tar.extend(["-so".into(), "--".into(), stage.path().join("contents.tar").display().to_string()]);
    tar.extend(paths.iter().map(|path| path.display().to_string()));

    let mut compressor = base("a", opts)?;
    compression(&mut compressor, codec, opts)?;
    compressor.extend(["-sicontents.tar".into(), "--".into(), candidate.display().to_string()]);
    ctl.set("compressing", None, "Creating compressed TAR stream");
    #[cfg(test)]
    BEFORE_TAR_ENCODER.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() { hook(); }
    });
    thread::scope(|scope| -> Result<(), String> {
        let encoder = scope.spawn(move || run(&tar, None, opts, Some(writer)));
        let compressed = run_program_log_with_directory(
            executable()?, &compressor, None, opts, None, false, false, None, Some(reader),
        ).and_then(|(log, _)| read_engine_output(log));
        let encoded = encoder.join().map_err(|_| "TAR encoder stopped unexpectedly".to_owned())?;
        ctl.check()?;
        compressed?;
        encoded?;
        Ok(())
    })?;
    ctl.check()?;
    policy.validate_output()?;
    let outputs = compression_outputs::publish(&[(candidate, output.to_owned())], &ctl, false)?;
    ctl.set("complete", Some(100), "Archive created");
    Ok(Some(outputs))
}

struct Counted<R> {
    inner: R,
    bytes: u64,
}
impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buf)?;
        self.bytes = self.bytes.checked_add(count as u64).ok_or(std::io::ErrorKind::InvalidData)?;
        Ok(count)
    }
}

fn scan_stream(reader: impl Read, ctl: &JobControl, request: Request<'_>) -> Result<Option<Listing>, String> {
    let mut archive = tar::Archive::new(Counted { inner: reader, bytes: 0 });
    let mut payload_end = 0u64;
    let mut entries = Vec::new();
    let mut bytes = None;
    let mut metadata_bytes = 0usize;
    let mut seen = HashSet::new();
    // Raw iteration keeps PAX/GNU extension bodies from being loaded into
    // memory. Legacy encodings, sparse files and extension headers use the
    // existing format-aware engine path.
    for item in archive.entries().map_err(|error| error.to_string())?.raw(true) {
        ctl.check()?;
        let mut item = item.map_err(|error| format!("invalid TAR: {error}"))?;
        let header = item.header();
        let kind = header.entry_type();
        let path = item.path_bytes();
        let Ok(name) = std::str::from_utf8(&path) else { return Ok(None) };
        let size = item.size();
        if !(kind.is_file() || (kind.is_dir() && size == 0))
            || name.is_empty() || name.starts_with("./")
        {
            return Ok(None);
        }
        if name.contains(['\\', '\r', '\n', '\0']) {
            return Ok(None);
        }
        let name = name.trim_end_matches('/');
        let relative = validate_name(name)?;
        let name = if cfg!(windows) { name.replace('/', "\\") } else { name.to_owned() };
        validate_listing_member(&name, kind.is_dir(), "", &mut seen)?;
        let packed_size = (size.checked_add(511).ok_or("invalid TAR member size")? / 512) * 512;
        payload_end = item.raw_file_position().checked_add(packed_size).ok_or("invalid TAR member position")?;
        // Include the inventory's allocations and the later tree-validation
        // keys when deciding how much metadata can be retained.
        metadata_bytes = metadata_bytes.saturating_add(384 + name.len().saturating_mul(4));
        if metadata_bytes > 128 * 1024 * 1024 {
            return Err("压缩 TAR 目录超过 128 MiB 元数据预算，请拆成较小的归档".into());
        }
        let modified = header.mtime().ok().and_then(|mtime| i64::try_from(mtime).ok())
            .and_then(|mtime| chrono::DateTime::from_timestamp(mtime, 0))
            .map(|mtime| mtime.format("%Y-%m-%d %H:%M:%SZ").to_string()).unwrap_or_default();
        entries.push(Entry {
            name,
            size,
            size_known: true,
            is_dir: kind.is_dir(),
            packed_size,
            encrypted: false,
            created: String::new(),
            accessed: String::new(),
            modified,
            crc: String::new(),
            method: String::new(),
            comment: String::new(),
            attributes: String::new(),
        });
        if let Request::Preview(path, limit) = request {
            if kind.is_file() && relative == path {
                if size > limit {
                    return Err(format!("preview exceeds {limit} byte limit"));
                }
                let mut content = Vec::with_capacity(size as usize);
                (&mut item).take(limit + 1).read_to_end(&mut content).map_err(|error| error.to_string())?;
                if content.len() as u64 != size {
                    return Err("TAR preview size mismatch".into());
                }
                bytes = Some(content);
            }
        }
    }
    // Finish the compressed stream and require zero padding after TAR's end.
    // The decoder's exit code then checks the outer trailer and CRC.
    let mut raw = archive.into_inner();
    if raw.bytes != payload_end.checked_add(512).ok_or("invalid TAR member position")? {
        return Err("missing TAR end record".into());
    }
    let mut tail = [0; 32 * 1024];
    loop {
        ctl.check()?;
        let n = raw.read(&mut tail).map_err(|error| error.to_string())?;
        if n == 0 { break; }
        if tail[..n].iter().any(|&byte| byte != 0) {
            return Err("nonzero data after the TAR end record".into());
        }
    }
    if raw.bytes < payload_end.checked_add(1024).ok_or("invalid TAR length")? || raw.bytes % 512 != 0 {
        return Err("incomplete TAR end records".into());
    }
    expected_tree(&entries)?;
    Ok(Some(Listing { entries, bytes }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn ordinary_tar() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, mode, kind, body) in [
            ("docs", 0o711, tar::EntryType::Directory, b"".as_slice()),
            ("docs/note.txt", 0o444, tar::EntryType::Regular, b"hello".as_slice()),
            ("ignored.bin", 0o644, tar::EntryType::Regular, b"elsewhere".as_slice()),
        ] {
            let mut header = tar::Header::new_ustar();
            header.set_mode(mode);
            header.set_size(body.len() as u64);
            header.set_mtime(1_700_000_000);
            header.set_entry_type(kind);
            builder.append_data(&mut header, name, body).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn compressed_tar_stream_matches_reference_for_gzip_bzip2_xz() {
        let temporary = tempfile::tempdir().unwrap();
        let source = temporary.path().join("data.tar");
        fs::write(&source, ordinary_tar()).unwrap();
        let opts = EngineOptions::default();
        let tar_path = source.to_str().unwrap();
        let reference = list_raw(tar_path, &opts).unwrap();
        let target = validate_name("docs/note.txt").unwrap();
        for (codec, extension) in [("gzip", "gz"), ("bzip2", "bz2"), ("xz", "xz")] {
            let archive = temporary.path().join(format!("data.tar.{extension}"));
            let path = archive.to_str().unwrap();
            create(path, &[tar_path.to_owned()], codec, &opts).unwrap();
            let typed = scan(path, &opts, Request::List).unwrap().expect("ordinary TAR streams");
            assert_eq!(typed.entries, reference, "{codec} metadata");
            assert_eq!(list(path, &opts).unwrap(), reference);
            assert_eq!(preview_limited(path, "docs/note.txt", &opts, 5).unwrap(), b"hello");
            assert_eq!(scan(path, &opts, Request::Preview(&target, 5)).unwrap().unwrap().bytes.unwrap(), b"hello");
            assert!(preview_limited(path, "docs/note.txt", &opts, 4).is_err());
            test(path, &opts).unwrap();
            let bad = temporary.path().join(format!("bad.tar.{extension}"));
            let mut compressed = fs::read(&archive).unwrap();
            compressed.truncate(compressed.len() - 3);
            fs::write(&bad, compressed).unwrap();
            assert!(test(bad.to_str().unwrap(), &opts).is_err(), "outer {codec}");
        }

        let destination = temporary.path().join("selected");
        let selected = EngineOptions { selected: vec!["docs/note.txt".into()], ..opts };
        assert_eq!(extract(tar_path, destination.to_str().unwrap(), &selected).unwrap(), 1);
        assert_eq!(fs::read(destination.join("docs/note.txt")).unwrap(), b"hello");
        assert!(!destination.join("ignored.bin").exists());
    }

    #[test]
    fn compressed_tar_creation_streams_each_codec_and_publishes_only_when_both_finish() {
        let fixture = tempfile::tempdir().unwrap();
        let first = fixture.path().join("first.txt");
        let second = fixture.path().join("second.txt");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let inputs = [first.display().to_string(), second.display().to_string()];
        let opts = EngineOptions::default();
        for (format, suffix) in [("tgz", "gz"), ("tbz2", "bz2"), ("txz", "xz")] {
            let output = fixture.path().join(format!("stream.tar.{suffix}"));
            let archive = output.to_str().unwrap();
            assert_eq!(create(archive, &inputs, format, &opts).unwrap(), vec![archive]);
            assert_eq!(preview(archive, "first.txt", &opts).unwrap(), b"one");
            assert_eq!(preview(archive, "second.txt", &opts).unwrap(), b"two");
            test(archive, &opts).unwrap();
            let exact = fs::read(&output).unwrap();
            assert!(create(archive, &inputs, format, &opts).is_err());
            assert_eq!(fs::read(&output).unwrap(), exact);
        }

        BEFORE_TAR_ENCODER.with(|hook| {
            let path = second.clone();
            hook.replace(Some(Box::new(move || fs::remove_file(path).unwrap())));
        });
        let failed = fixture.path().join("failed.tar.gz");
        assert!(create(failed.to_str().unwrap(), &inputs, "tgz", &opts).is_err());
        assert!(!failed.exists());
        assert_eq!(fs::read(first).unwrap(), b"one");
    }

    #[test]
    fn compressed_tar_parser_rejects_missing_inner_end_records_and_payload() {
        let opts = JobControl::new();
        let data = ordinary_tar();
        for bad in [&[][..], &data[..700], &data[..data.len()-512], &data[..data.len()-1]] {
            assert!(scan_stream(bad, &opts, Request::Test).is_err());
        }
        let mut corrupt = data.clone();
        corrupt[0] ^= 1;
        assert!(scan_stream(&corrupt[..], &opts, Request::Test).is_err());
        let mut garbage = data;
        *garbage.last_mut().unwrap() = 1;
        assert!(scan_stream(&garbage[..], &opts, Request::Test).is_err());
    }
}
