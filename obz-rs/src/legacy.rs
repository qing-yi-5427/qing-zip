//! Independent LZH writing and ACE reading. CRC verification and bounded decoding
//! are mandatory. This module does not execute files contained in an archive.
use crate::engine::{self, EngineOptions, Entry};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::Ordering,
    time::UNIX_EPOCH,
};
use unarc_rs::unified::{
    ArchiveEntryKind, ArchiveFormat, ArchiveOptions, UnifiedArchive, VolumeProvider,
};

const MAX_ENTRY: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
fn check(opts: &EngineOptions) -> Result<(), String> {
    if opts
        .control
        .as_ref()
        .is_some_and(|c| c.cancelled.load(Ordering::Relaxed))
    {
        return Err("operation cancelled".into());
    }
    if !["error", "overwrite", "skip", "rename"].contains(&opts.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    Ok(())
}
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("malformed legacy archive rejected by decoder".into()))
}
fn crc16(bytes: &[u8]) -> u16 {
    let mut c = 0u16;
    for &b in bytes {
        c ^= b as u16;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xa001
            } else {
                c >> 1
            };
        }
    }
    c
}

#[cfg(windows)]
fn lzh_name(name: &str, codepage: Option<u32>) -> Result<Vec<u8>, String> {
    #[link(name = "Kernel32")]
    extern "system" {
        fn WideCharToMultiByte(
            cp: u32,
            flags: u32,
            input: *const u16,
            len: i32,
            output: *mut u8,
            capacity: i32,
            default: *const u8,
            used_default: *mut i32,
        ) -> i32;
        fn GetOEMCP() -> u32;
    }
    let codepage = codepage.unwrap_or_else(|| unsafe { GetOEMCP() });
    let units: Vec<u16> = name.encode_utf16().collect();
    let len = i32::try_from(units.len()).map_err(|_| "LZH name too long")?;
    let mut replaced = 0;
    let flags = if codepage == 65001 { 0x80 } else { 0x400 };
    let used = if codepage == 65001 {
        std::ptr::null_mut()
    } else {
        &mut replaced
    };
    let size = unsafe {
        WideCharToMultiByte(
            codepage,
            flags,
            units.as_ptr(),
            len,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            used,
        )
    };
    if size == 0 {
        return Err(format!(
            "LZH filename encoding failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut out = vec![0; size as usize];
    let written = unsafe {
        WideCharToMultiByte(
            codepage,
            flags,
            units.as_ptr(),
            len,
            out.as_mut_ptr(),
            size,
            std::ptr::null(),
            used,
        )
    };
    if written != size || replaced != 0 {
        return Err("LZH filename cannot be represented in selected legacy codepage; use ZIP/7z for Unicode names".into());
    }
    Ok(out)
}
#[cfg(not(windows))]
fn lzh_name(name: &str, _: Option<u32>) -> Result<Vec<u8>, String> {
    if !name.is_ascii() {
        return Err("non-ASCII LZH creation requires Windows codepage conversion".into());
    }
    Ok(name.as_bytes().to_vec())
}
fn write_lzh_header(
    writer: &mut impl Write,
    name: &str,
    method: oxiarc_lzhuf::LzhMethod,
    crc: u16,
    size: u32,
    packed: u32,
    mtime: u32,
    codepage: Option<u32>,
) -> Result<(), String> {
    let normalized = name.replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    let (dir, file) = if method == oxiarc_lzhuf::LzhMethod::Lhd {
        (Some(normalized), "")
    } else {
        match normalized.rsplit_once('/') {
            Some((d, f)) => (Some(d), f),
            None => (None, normalized),
        }
    };
    let mut extensions = Vec::new();
    let mut extension = |kind: u8, bytes: &[u8]| -> Result<(), String> {
        let len = u16::try_from(bytes.len() + 3).map_err(|_| "LZH header too long")?;
        extensions.extend_from_slice(&len.to_le_bytes());
        extensions.push(kind);
        extensions.extend_from_slice(bytes);
        Ok(())
    };
    if !file.is_empty() {
        extension(1, &lzh_name(file, codepage)?)?;
    }
    if let Some(dir) = dir {
        let mut path = Vec::new();
        for part in dir.split('/') {
            path.extend_from_slice(&lzh_name(part, codepage)?);
            path.push(0xff);
        }
        extension(2, &path)?;
    }
    // End-of-archive markers in older readers inspect the low size byte.
    drop(extension);
    if (26 + extensions.len()) % 256 == 0 {
        extensions.extend_from_slice(&4u16.to_le_bytes());
        extensions.extend_from_slice(&[0x3f, 0]);
    }
    let total = u16::try_from(26 + extensions.len()).map_err(|_| "LZH header too long")?;
    let mut header = Vec::new();
    header.extend_from_slice(&total.to_le_bytes());
    header.extend_from_slice(&method.id());
    header.extend_from_slice(&packed.to_le_bytes());
    header.extend_from_slice(&size.to_le_bytes());
    header.extend_from_slice(&mtime.to_le_bytes());
    header.push(0x20);
    header.push(2);
    header.extend_from_slice(&crc.to_le_bytes());
    header.push(b'W');
    header.extend_from_slice(&extensions);
    header.extend_from_slice(&[0, 0]);
    writer.write_all(&header).map_err(|e| e.to_string())
}

pub fn create_lzh(
    out: &str,
    inputs: &[String],
    opts: &EngineOptions,
) -> Result<Vec<String>, String> {
    guarded(|| {
        check(opts)?;
        if opts.password.is_some() || opts.volume_bytes.is_some() {
            return Err("LZH creation does not support encryption or split volumes".into());
        }
        if inputs.is_empty() {
            return Err("LZH inputs cannot be empty".into());
        }
        let output = Path::new(out);
        engine::no_links(output)?;
        if output.exists() {
            return Err("output already exists".into());
        }
        let mut files = Vec::<(String, PathBuf, bool)>::new();
        let mut seen = HashSet::new();
        fn collect(
            path: &Path,
            name: String,
            files: &mut Vec<(String, PathBuf, bool)>,
            seen: &mut HashSet<String>,
            opts: &EngineOptions,
        ) -> Result<(), String> {
            check(opts)?;
            engine::no_links(path)?;
            engine::validate_name(&name)?;
            if !seen.insert(name.to_lowercase()) {
                return Err("duplicate LZH input filename".into());
            }
            let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if !m.is_dir() && !m.is_file() {
                return Err("non-regular LZH input rejected".into());
            }
            files.push((name.clone(), path.to_path_buf(), m.is_dir()));
            if m.is_dir() {
                for e in fs::read_dir(path).map_err(|e| e.to_string())? {
                    let e = e.map_err(|e| e.to_string())?;
                    let n = e
                        .file_name()
                        .into_string()
                        .map_err(|_| "non-Unicode input filename")?;
                    collect(&e.path(), format!("{name}/{n}"), files, seen, opts)?;
                }
            }
            Ok(())
        }
        for input in inputs {
            let p = Path::new(input);
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("invalid input filename")?;
            collect(p, name.into(), &mut files, &mut seen, opts)?;
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        {
            let writer = temp.as_file_mut();
            for (name, path, is_dir) in files {
                check(opts)?;
                if is_dir {
                    write_lzh_header(
                        writer,
                        &name,
                        oxiarc_lzhuf::LzhMethod::Lhd,
                        0,
                        0,
                        0,
                        0,
                        opts.codepage,
                    )?;
                    continue;
                }
                let file = File::open(&path).map_err(|e| e.to_string())?;
                let metadata = file.metadata().map_err(|e| e.to_string())?;
                if metadata.len() > MAX_ENTRY {
                    return Err("LZH writer currently limits each input to 512 MiB".into());
                }
                let mut bytes = Vec::new();
                file.take(MAX_ENTRY + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 > MAX_ENTRY {
                    return Err("LZH input grew beyond size limit".into());
                }
                let method = if opts.level == Some(0) {
                    oxiarc_lzhuf::LzhMethod::Lh0
                } else {
                    oxiarc_lzhuf::LzhMethod::Lh7
                };
                let packed = oxiarc_lzhuf::encode_lzh(&bytes, method).map_err(|e| e.to_string())?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
                    .unwrap_or(0);
                write_lzh_header(
                    writer,
                    &name,
                    method,
                    crc16(&bytes),
                    bytes.len() as u32,
                    u32::try_from(packed.len()).map_err(|_| "LZH output exceeds classic limit")?,
                    modified,
                    opts.codepage,
                )?;
                writer.write_all(&packed).map_err(|e| e.to_string())?;
            }
            writer.write_all(&[0]).map_err(|e| e.to_string())?;
        }
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        check(opts)?;
        engine::no_links(output)?;
        temp.persist_noclobber(output)
            .map_err(|e| e.error.to_string())?;
        Ok(vec![out.into()])
    })
}

pub fn is_supported(path: &str) -> bool {
    let mut data = vec![0u8; 512 * 1024];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(n) = file.read(&mut data) else {
        return false;
    };
    data[..n]
        .windows(7)
        .position(|w| w == b"**ACE**")
        .is_some_and(|p| p >= 7)
}
fn settings(opts: &EngineOptions) -> Result<ArchiveOptions, String> {
    check(opts)?;
    if opts.codepage.is_some() {
        return Err("ACE codepage override is not supported by this decoder".into());
    }
    let mut a = ArchiveOptions::new()
        .with_verify_crc(true)
        .with_max_entry_size(Some(MAX_ENTRY))
        .with_max_total_size(Some(MAX_TOTAL));
    if let Some(password) = &opts.password {
        a = a.with_password(password);
    }
    Ok(a)
}
struct ControlledFile {
    file: File,
    control: Option<engine::JobControl>,
}
impl ControlledFile {
    fn check(&self) -> std::io::Result<()> {
        if self
            .control
            .as_ref()
            .is_some_and(|c| c.cancelled.load(Ordering::Relaxed))
        {
            Err(std::io::Error::other("operation cancelled"))
        } else {
            Ok(())
        }
    }
}
impl Read for ControlledFile {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        self.file.read(out)
    }
}
impl Seek for ControlledFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.check()?;
        self.file.seek(pos)
    }
}
struct AceVolumes {
    paths: Vec<PathBuf>,
    control: Option<engine::JobControl>,
}
impl VolumeProvider for AceVolumes {
    fn open_volume(&self, number: u32) -> Option<Box<dyn Read + Send>> {
        if self
            .control
            .as_ref()
            .is_some_and(|c| c.cancelled.load(Ordering::Relaxed))
        {
            return None;
        }
        let path = self.paths.get(number as usize)?;
        engine::no_links(path).ok()?;
        let file = File::open(path).ok()?;
        if file.metadata().ok()?.len() > MAX_TOTAL {
            return None;
        }
        Some(Box::new(ControlledFile {
            file,
            control: self.control.clone(),
        }))
    }
}
fn open(path: &str, opts: &EngineOptions) -> Result<UnifiedArchive<ControlledFile>, String> {
    check(opts)?;
    let input = File::open(path).map_err(|e| e.to_string())?;
    let mut total = input.metadata().map_err(|e| e.to_string())?.len();
    if total > MAX_TOTAL {
        return Err("ACE input exceeds 2 GiB safety limit".into());
    }
    let mut paths = vec![PathBuf::from(path)];
    // ACE conventional volumes: archive.ace, archive.c00, archive.c01, ... .
    // Bound and validate siblings before giving the decoder a volume provider.
    for i in 0..1000 {
        check(opts)?;
        let next = Path::new(path).with_extension(format!("c{i:02}"));
        match fs::metadata(&next) {
            Ok(m) => {
                engine::no_links(&next)?;
                if !m.is_file() {
                    return Err("ACE volume is not a regular file".into());
                }
                total = total
                    .checked_add(m.len())
                    .filter(|n| *n <= MAX_TOTAL)
                    .ok_or("ACE volumes exceed 2 GiB safety limit")?;
                paths.push(next);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    if paths.len() > 1000 {
        return Err("ACE volume count exceeds 1000 limit".into());
    }
    let settings = settings(opts)?.with_volume_provider(AceVolumes {
        paths,
        control: opts.control.clone(),
    });
    UnifiedArchive::open_with_format_and_options(
        ControlledFile {
            file: input,
            control: opts.control.clone(),
        },
        ArchiveFormat::Ace,
        settings,
    )
    .map_err(|e| e.to_string())
}

fn entry_view(e: &unarc_rs::unified::ArchiveEntry) -> Result<Entry, String> {
    engine::validate_name(e.name())?;
    if !matches!(
        e.kind(),
        ArchiveEntryKind::File | ArchiveEntryKind::Directory
    ) {
        return Err("ACE non-regular member rejected".into());
    }
    if e.original_size() > MAX_ENTRY || e.compressed_size() > MAX_TOTAL {
        return Err("ACE entry exceeds size limit".into());
    }
    Ok(Entry {
        name: e.name().replace('\\', "/"),
        size: e.original_size(),
        packed_size: e.compressed_size(),
        is_dir: e.is_directory(),
        crc: format!("{:08X}", e.crc()),
        encrypted: e.is_encrypted(),
        modified: e
            .modified_time()
            .map(|v| format!("{v:?}"))
            .unwrap_or_default(),
        method: e.compression_method().into(),
        comment: String::new(),
        attributes: String::new(),
    })
}
pub fn list(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    if crate::legacy_worker::available() {
        check(opts)?;
        let entries = crate::legacy_worker::list(path, opts)?;
        let mut seen = HashSet::new();
        let mut total = 0u64;
        for e in &entries {
            let key = engine::validate_name(&e.name)?
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            if !seen.insert(key) {
                return Err("duplicate ACE names".into());
            }
            total = total
                .checked_add(e.size)
                .filter(|n| *n <= MAX_TOTAL)
                .ok_or("ACE total size limit")?;
            if e.size > MAX_ENTRY {
                return Err("ACE entry size limit".into());
            }
        }
        if entries.len() > 100000 {
            return Err("ACE entry count limit".into());
        }
        return Ok(entries);
    }
    guarded(|| {
        let mut a = open(path, opts)?;
        let mut entries = Vec::new();
        let mut names = HashSet::new();
        let mut total = 0u64;
        while let Some(e) = a.next_entry().map_err(|e| e.to_string())? {
            check(opts)?;
            let view = entry_view(&e)?;
            if !names.insert(
                engine::validate_name(&view.name)?
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase(),
            ) {
                return Err("duplicate ACE entry names".into());
            }
            total = total
                .checked_add(view.size)
                .filter(|v| *v <= MAX_TOTAL)
                .ok_or("ACE output exceeds 2 GiB safety limit")?;
            a.skip(&e).map_err(|e| e.to_string())?;
            entries.push(view);
            if entries.len() > 100000 {
                return Err("ACE entry count exceeds limit".into());
            }
        }
        let files: HashSet<_> = entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| {
                engine::validate_name(&e.name)
                    .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
            })
            .collect::<Result<_, _>>()?;
        for e in &entries {
            let name = engine::validate_name(&e.name)?
                .to_string_lossy()
                .replace('\\', "/")
                .to_lowercase();
            for (index, _) in name.match_indices('/') {
                if files.contains(&name[..index]) {
                    return Err("ACE file/directory collision".into());
                }
            }
        }
        Ok(entries)
    })
}
fn decode(path: &str, chosen: &[Entry], stage: &Path, opts: &EngineOptions) -> Result<(), String> {
    if crate::legacy_worker::available() {
        return crate::legacy_worker::decode(path, chosen, stage, opts);
    }
    let mut a = open(path, opts)?;
    let settings = settings(opts)?;
    let names: HashSet<_> = chosen.iter().map(|e| e.name.as_str()).collect();
    while let Some(e) = a.next_entry().map_err(|e| e.to_string())? {
        check(opts)?;
        let view = entry_view(&e)?;
        if names.contains(view.name.as_str()) {
            let target = stage.join(engine::validate_name(&view.name)?);
            if view.is_dir {
                fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                a.skip_with_options(&e, &settings)
                    .map_err(|e| e.to_string())?;
                continue;
            }
            let bytes = a
                .read_with_options(&e, &settings)
                .map_err(|e| e.to_string())?;
            if bytes.len() as u64 != view.size {
                return Err("ACE decoded size mismatch".into());
            }
            fs::create_dir_all(target.parent().ok_or("missing parent")?)
                .map_err(|e| e.to_string())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
        } else {
            a.skip_with_options(&e, &settings)
                .map_err(|e| e.to_string())?;
        }
    }
    check(opts)
}
pub fn extract(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    guarded(|| {
        let entries = list(path, opts)?;
        let chosen: Vec<_> = entries
            .into_iter()
            .filter(|e| engine::selected(&e.name, opts))
            .collect();
        if !opts.selected.is_empty() && chosen.is_empty() {
            return Err("selected ACE entries not found".into());
        }
        let dest = std::path::absolute(dest).map_err(|e| e.to_string())?;
        engine::no_links(&dest)?;
        for e in &chosen {
            let path = dest.join(engine::validate_name(&e.name)?);
            engine::no_links(&path)?;
            if !e.is_dir && opts.overwrite == "error" && path.exists() {
                return Err("destination already exists".into());
            }
        }
        let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
        decode(path, &chosen, stage.path(), opts)?;
        let mut n = 0;
        for e in chosen {
            check(opts)?;
            let target = dest.join(engine::validate_name(&e.name)?);
            engine::no_links(&target)?;
            if e.is_dir {
                fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                continue;
            }
            let Some(target) = engine::choose_target(target, &opts.overwrite)? else {
                continue;
            };
            let parent = target.parent().ok_or("missing parent")?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            engine::no_links(parent)?;
            let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            let mut input = File::open(stage.path().join(engine::validate_name(&e.name)?))
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut input, &mut temp).map_err(|e| e.to_string())?;
            check(opts)?;
            engine::no_links(&target)?;
            if opts.overwrite == "overwrite" {
                temp.persist(target).map_err(|e| e.error.to_string())?;
            } else {
                temp.persist_noclobber(target)
                    .map_err(|e| e.error.to_string())?;
            }
            n += 1;
        }
        Ok(n)
    })
}
pub fn test(path: &str, opts: &EngineOptions) -> Result<(), String> {
    guarded(|| {
        let entries = list(path, opts)?;
        let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
        decode(path, &entries, stage.path(), opts)
    })
}
pub fn preview(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    guarded(|| {
        let entry = list(path, opts)?
            .into_iter()
            .find(|e| e.name == name)
            .ok_or("ACE entry not found")?;
        if entry.is_dir || entry.size > 16 * 1024 * 1024 {
            return Err("preview requires file no larger than 16 MiB".into());
        }
        let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
        let file = stage.path().join(engine::validate_name(&entry.name)?);
        decode(path, &[entry], stage.path(), opts)?;
        fs::read(file).map_err(|e| e.to_string())
    })
}
