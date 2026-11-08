//! Bounded stdout transport for raw-stream previews, never an unbounded file.
use crate::engine::{self, EngineOptions};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

pub(crate) fn preview(
    path: &Path,
    format: &str,
    opts: &EngineOptions,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let _policy = crate::machine_policy::scope();
    let control = opts.control.clone().unwrap_or_default();
    control.check()?;
    let mut bytes = Vec::new();
    if decode_native(path, format, opts, Some(limit), &mut |part| {
        if bytes.len() as u64 + part.len() as u64 > limit {
            return Err(format!("preview exceeds {limit} byte limit"));
        }
        bytes.extend_from_slice(part);
        Ok(())
    })? {
        return Ok(bytes);
    }
    validate_frame_bounds(path, format, opts)?;
    bounded_engine_preview(path, format, None, opts, limit)
}

// Callers have validated the format and exact optional member name under a
// pinned source. Capture at most limit+1 bytes even if a header lies.
pub(super) fn bounded_engine_preview(
    path: &Path, format: &str, member: Option<&str>, opts: &EngineOptions, limit: u64,
) -> Result<Vec<u8>, String> {
    let policy = crate::machine_policy::scope();
    let control = opts.control.clone().unwrap_or_default();
    control.check()?;
    let errors = tempfile::tempfile().map_err(|error| error.to_string())?;
    let mut command = Command::new(engine::executable()?);
    command
        .args(["x", "-so", "-y", "-sccUTF-8", "-spd", "-bso0", "-bsp0"])
        .arg(format!("-t{format}"));
    if let Some(threads) = opts.threads {
        command.arg(format!("-mmt={threads}"));
    }
    command.arg("--").arg(path);
    if let Some(name) = member { command.arg(name); }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(errors.try_clone().map_err(|error| error.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot launch stream decoder: {error}"))?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("stream decoder output unavailable".into());
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::Builder::new()
        .name("bounded-stream-preview".into())
        .spawn(move || {
            let result = (|| {
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 65536];
                loop {
                    // Read at most limit+1 in total; the excess byte is never retained.
                    let request = limit
                        .saturating_sub(bytes.len() as u64)
                        .saturating_add(1)
                        .min(buffer.len() as u64) as usize;
                    let count = stdout
                        .read(&mut buffer[..request])
                        .map_err(|error| error.to_string())?;
                    if count == 0 {
                        return Ok(bytes);
                    }
                    if bytes.len() as u64 + count as u64 > limit {
                        return Err(format!("preview exceeds {} byte limit", limit));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
            })();
            let _ = sender.send(result.is_err());
            result
        });
    let reader = match reader {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.to_string());
        }
    };
    let status = loop {
        if let Err(error) = control.check() {
            break Err(error);
        }
        if receiver.try_recv() == Ok(true) {
            break Ok(None);
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(Some(status)),
            Ok(None) => {
                if let Err(error) = engine::wait_for_process(&child, Duration::from_millis(20)) {
                    break Err(error.to_string());
                }
            }
            Err(error) => break Err(error.to_string()),
        }
    };
    if !matches!(&status, Ok(Some(_))) {
        let _ = child.kill();
        let _ = child.wait();
    }
    let decoded = reader
        .join()
        .map_err(|_| "stream preview reader failed".to_owned());
    let status = status?;
    let bytes = decoded??;
    let status = status.ok_or("stream preview interrupted")?;
    if !status.success() {
        let mut errors = errors;
        errors
            .seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        let mut text = String::new();
        errors.take(8192).read_to_string(&mut text).ok();
        return Err(format!(
            "stream decode/checksum failed ({}): {}",
            status.code().unwrap_or(-1),
            text.trim()
        ));
    }
    control.check()?;
    policy.unchanged()?;
    Ok(bytes)
}

/// Preserve exact EOF checks for ordinary Brotli. NanaZip's MT container has a
/// different framing and remains handled by its own decoder. Decoding and
/// publication happen in one pass; the input buffer never grows with the file.
pub(crate) fn decode_native(
    path: &Path,
    format: &str,
    opts: &EngineOptions,
    limit: Option<u64>,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<bool, String> {
    if crate::machine_policy::scope().restricted() { return Ok(false); }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_native_inner(path, format, opts, limit, emit)
    }))
    .unwrap_or_else(|_| Err("malformed Brotli stream rejected by decoder".into()))
}

fn decode_native_inner(
    path: &Path,
    format: &str,
    opts: &EngineOptions,
    limit: Option<u64>,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<bool, String> {
    if format != "brotli" {
        return Ok(false);
    }
    let control = opts.control.clone().unwrap_or_default();
    control.check()?;
    let mut file = pin(path)?;
    let mut magic = [0u8; 4];
    let n = file.read(&mut magic).map_err(|e| e.to_string())?;
    if n == 4 && (u32::from_le_bytes(magic) & 0xfffffff0) == 0x184d2a50 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut state = brotli::BrotliState::new(
        brotli::HeapAlloc::default(),
        brotli::HeapAlloc::default(),
        brotli::HeapAlloc::default(),
    );
    let mut input = [0u8; 65536];
    let mut output = [0u8; 65536];
    let mut available = 0;
    let mut offset = 0;
    let mut written = 0;
    let mut emitted = 0u64;
    loop {
        control.check()?;
        if available == 0 {
            available = file.read(&mut input).map_err(|e| e.to_string())?;
            offset = 0;
        }
        let mut out_available = limit.map_or(output.len(), |bound| {
            bound
                .saturating_sub(emitted)
                .saturating_add(1)
                .min(output.len() as u64) as usize
        });
        let mut out = 0;
        let before = offset;
        let result = brotli::BrotliDecompressStream(
            &mut available,
            &mut offset,
            &input,
            &mut out_available,
            &mut out,
            &mut output,
            &mut written,
            &mut state,
        );
        emit(&output[..out])?;
        emitted = emitted
            .checked_add(out as u64)
            .ok_or("decoded size overflow")?;
        match result {
            brotli::BrotliResult::ResultSuccess => {
                if available != 0 || file.read(&mut input[..1]).map_err(|e| e.to_string())? != 0 {
                    return Err("Brotli trailing bytes rejected".into());
                }
                control.check()?;
                return Ok(true);
            }
            brotli::BrotliResult::ResultFailure => return Err("invalid Brotli stream".into()),
            brotli::BrotliResult::NeedsMoreInput => {
                if available != 0 {
                    return Err("Brotli decoder made no progress".into());
                }
                available = file.read(&mut input).map_err(|e| e.to_string())?;
                offset = 0;
                if available == 0 {
                    return Err("truncated Brotli stream".into());
                }
            }
            brotli::BrotliResult::NeedsMoreOutput => {
                if out == 0 && before == offset {
                    return Err("Brotli decoder made no progress".into());
                }
            }
        }
    }
}

/// Walk standard LZ4 frame headers without decoding or loading the payload.
/// This keeps the old missing-end-marker and trailing-data rejection even for
/// decoders that accept EOF at a block boundary. The codec checks checksums.
pub(crate) fn validate_frame_bounds(
    path: &Path,
    format: &str,
    opts: &EngineOptions,
) -> Result<(), String> {
    if format != "lz4" {
        return Ok(());
    }
    let control = opts.control.clone().unwrap_or_default();
    let mut file = pin(path)?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let mut magic = [0; 4];
    if file.read(&mut magic).map_err(|e| e.to_string())? != 4 || magic != [4, 0x22, 0x4d, 0x18] {
        return Ok(());
    }
    loop {
        control.check()?;
        let mut header = [0; 2];
        file.read_exact(&mut header)
            .map_err(|_| "truncated LZ4 header")?;
        let flags = header[0];
        let bd = header[1];
        if flags >> 6 != 1 || flags & 2 != 0 || bd & 0x8f != 0 {
            return Err("invalid LZ4 reserved/version bits".into());
        }
        if flags & 1 != 0 {
            return Err("LZ4 external dictionaries unsupported".into());
        }
        let block = match (bd >> 4) & 7 {
            4 => 65536,
            5 => 262144,
            6 => 1048576,
            7 => 4194304,
            _ => return Err("invalid LZ4 block bound".into()),
        };
        let offset = file.stream_position().map_err(|e| e.to_string())?
            + 1
            + if flags & 8 != 0 { 8 } else { 0 };
        if offset > length {
            return Err("truncated LZ4 content size/checksum".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        loop {
            control.check()?;
            let mut size = [0; 4];
            file.read_exact(&mut size)
                .map_err(|_| "LZ4 missing end marker")?;
            let size = u32::from_le_bytes(size);
            if size == 0 {
                if flags & 4 != 0 {
                    file.read_exact(&mut [0; 4])
                        .map_err(|_| "truncated LZ4 checksum")?;
                }
                break;
            }
            let size = size & 0x7fffffff;
            if size == 0 || size > block {
                return Err("LZ4 block exceeds frame bound".into());
            }
            let next = file.stream_position().map_err(|e| e.to_string())?
                + size as u64
                + if flags & 16 != 0 { 4 } else { 0 };
            if next > length {
                return Err("truncated LZ4 block".into());
            }
            file.seek(SeekFrom::Start(next))
                .map_err(|e| e.to_string())?;
        }
        if file.stream_position().map_err(|e| e.to_string())? == length {
            return Ok(());
        }
        file.read_exact(&mut magic)
            .map_err(|_| "LZ4 trailing bytes rejected")?;
        if magic != [4, 0x22, 0x4d, 0x18] {
            return Err("LZ4 trailing data rejected".into());
        }
    }
}

pub(crate) struct PinnedSource {
    file: File,
    _parents: Vec<File>,
}
impl std::ops::Deref for PinnedSource {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}
impl std::ops::DerefMut for PinnedSource {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

/// Retain the complete source path, then deny leaf writes/deletion while the
/// child reopens that path. Ancestors allow normal child creation but not rename.
pub(crate) fn pin(path: &Path) -> Result<PinnedSource, String> {
    engine::no_links(path)?;
    #[cfg(windows)]
    let parents = pin_parent_names(path).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let parents = Vec::new();
    let file = open_source_leaf(path).map_err(|error| error.to_string())?;
    Ok(PinnedSource {
        file,
        _parents: parents,
    })
}

#[cfg(windows)]
fn open_name(path: &Path, directory: bool) -> std::io::Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let handle = File::options().access_mode(0x80).share_mode(3)
        .custom_flags(0x02200000).open(path)?;
    let meta = handle.metadata()?;
    if meta.file_attributes() & 0x400 != 0 { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "source links are unsupported")); }
    if (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "unexpected source file type"));
    }
    Ok(handle)
}

#[cfg(windows)]
fn pin_parent_names(path: &Path) -> std::io::Result<Vec<File>> {
    let mut parents = Vec::new();
    for ancestor in path.ancestors().skip(1).collect::<Vec<_>>().into_iter().rev() {
        parents.push(open_name(ancestor, true)?);
    }
    Ok(parents)
}

/// Resolve filesystem spelling for shell queues without following a reparse
/// point or allowing any validated component to be renamed during resolution.
#[cfg(windows)]
pub fn canonical_input_path(path: &Path, directory: bool) -> std::io::Result<std::path::PathBuf> {
    let path = std::path::absolute(path)?;
    let _parents = pin_parent_names(&path)?;
    let _entry = open_name(&path, directory)?;
    path.canonicalize()
}

pub(crate) fn open_source_leaf(path: &Path) -> std::io::Result<File> {
    let mut options = File::options();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::other("raw stream must be a regular file"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other("raw stream links are unsupported"));
        }
    }
    Ok(file)
}

/// Keep existing contiguous .7z.002 and later volumes read-only while the
/// external engine lists and extracts them. The first pin holds their parents.
/// This does not reserve the name immediately after the last existing volume.
#[cfg(windows)]
pub(crate) fn pin_numbered_7z_parts(first: &Path) -> Result<Vec<File>, String> {
    let mut parts = Vec::new();
    let Some(stem) = first.file_stem().and_then(|name| name.to_str()) else {
        return Ok(parts);
    };
    if first.extension().and_then(|ext| ext.to_str()) != Some("001")
        || !Path::new(stem)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("7z"))
    {
        return Ok(parts);
    }
    for number in 2u64.. {
        let path = first.with_file_name(format!("{stem}.{number:03}"));
        match open_source_leaf(&path) {
            Ok(file) => parts.push(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!(
                    "cannot pin archive volume {}: {error}",
                    path.display()
                ))
            }
        }
    }
    Ok(parts)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn later_split_volumes_stay_read_only_until_extraction_releases_them() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("Sample.7Z.001");
        let second = first.with_extension("002");
        let third = first.with_extension("003");
        for p in [&first, &second, &third] {
            std::fs::write(p, b"fixture").unwrap();
        }
        let _first = pin(&first).unwrap();
        let parts = pin_numbered_7z_parts(&first).unwrap();
        assert_eq!(parts.len(), 2);
        for p in [&second, &third] {
            assert!(File::options().write(true).open(p).is_err());
            assert!(std::fs::rename(p, root.path().join("replaced")).is_err());
        }
        drop(parts);
        std::fs::write(&second, b"released").unwrap();
    }
    #[test]
    fn pinned_raw_source_blocks_leaf_writes_and_parent_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("source.br");
        std::fs::write(&path, b"fixture").unwrap();
        let guard = pin(&path).unwrap();
        assert!(File::options().write(true).open(&path).is_err());
        assert!(std::fs::rename(&path, parent.join("changed.br")).is_err());
        assert!(std::fs::rename(&parent, temp.path().join("moved")).is_err());
        std::fs::write(parent.join("sibling"), b"normal creation remains allowed").unwrap();
        drop(guard);
        std::fs::rename(&parent, temp.path().join("moved")).unwrap();
    }
}
