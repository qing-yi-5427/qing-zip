//! Independent MIT EGG/ALZ readers. Decode privately before publishing user files.
use crate::engine::{self, EngineOptions, Entry};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::Ordering,
};
trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}
struct ControlledReader {
    inner: Box<dyn ReadSeek>,
    control: Option<crate::engine::JobControl>,
}
impl ControlledReader {
    fn check(&self) -> std::io::Result<()> {
        if self
            .control
            .as_ref()
            .is_some_and(|c| c.cancelled.load(Ordering::Relaxed))
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "operation cancelled",
            ))
        } else {
            Ok(())
        }
    }
}
impl Read for ControlledReader {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        self.inner.read(b)
    }
}
impl Seek for ControlledReader {
    fn seek(&mut self, p: std::io::SeekFrom) -> std::io::Result<u64> {
        self.check()?;
        self.inner.seek(p)
    }
}
enum Archive {
    Egg(unegg::archive::EggArchive<Box<dyn ReadSeek>>),
    Alz(unalz::archive::AlzArchive),
}
fn check(opts: &EngineOptions) -> Result<(), String> {
    if opts
        .control
        .as_ref()
        .is_some_and(|c| c.cancelled.load(Ordering::Relaxed))
    {
        Err("operation cancelled".into())
    } else {
        Ok(())
    }
}
pub fn is_supported(path: &str) -> bool {
    let mut b = [0; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok()
        && (b == *b"EGGA" || b == *b"ALZ\x01")
}
fn open(path: &str, opts: &EngineOptions) -> Result<Archive, String> {
    check(opts)?;
    if opts.codepage.is_some_and(|v| v != 949) {
        return Err("EGG/ALZ codepage override is unsupported; ALZ names use CP949".into());
    }
    if !["error", "overwrite", "skip", "rename"].contains(&opts.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    let mut b = [0; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| e.to_string())?;
    if b == *b"EGGA" {
        let reader: Box<dyn ReadSeek> =
            match unegg::volume::MultiVolumeReader::try_open(Path::new(path))
                .map_err(|e| e.to_string())?
            {
                Some(r) => Box::new(r),
                None => Box::new(File::open(path).map_err(|e| e.to_string())?),
            };
        Ok(Archive::Egg(
            unegg::archive::EggArchive::open(Box::new(ControlledReader {
                inner: reader,
                control: opts.control.clone(),
            }) as Box<dyn ReadSeek>)
            .map_err(|e| e.to_string())?,
        ))
    } else if b == *b"ALZ\x01" {
        let a = unalz::archive::AlzArchive::open(path).map_err(|e| e.to_string())?;
        if a.truncated {
            return Err("truncated ALZ archive".into());
        }
        Ok(Archive::Alz(a))
    } else {
        Err("not an EGG or ALZ archive".into())
    }
}
fn entries(a: &Archive) -> Result<Vec<Entry>, String> {
    if let Archive::Egg(egg) = a {
        let mut block_output = 0u64;
        for entry in &egg.entries {
            for block in &entry.blocks {
                block_output = block_output
                    .checked_add(block.uncompressed_size as u64)
                    .ok_or("EGG block size overflow")?;
            }
        }
        if block_output > 8 * 1024 * 1024 * 1024 {
            return Err("EGG block output exceeds 8 GiB staging limit".into());
        }
    }

    let out: Vec<Entry> = match a {
        Archive::Egg(a) => a
            .entries
            .iter()
            .map(|e| Entry {
                name: e.file_name.replace('\\', "/"),
                size: e.uncompressed_size,
                packed_size: e.blocks.iter().map(|b| b.compressed_size as u64).sum(),
                is_dir: e.is_directory(),
                crc: if e.blocks.len() == 1 {
                    format!("{:08X}", e.blocks[0].crc32)
                } else {
                    String::new()
                },
                encrypted: e.encrypt_info.is_some() || a.is_encrypted,
                modified: String::new(),
                method: e
                    .blocks
                    .iter()
                    .map(|b| b.compression_method.to_string())
                    .collect::<Vec<_>>()
                    .join("+"),
                comment: String::new(),
                attributes: String::new(),
            })
            .collect(),
        Archive::Alz(a) => a
            .entries
            .iter()
            .map(|e| Entry {
                name: e.file_name.replace('\\', "/"),
                size: e.uncompressed_size,
                packed_size: e.compressed_size,
                is_dir: e.is_directory(),
                crc: format!("{:08X}", e.file_crc),
                encrypted: e.is_encrypted(),
                modified: unalz::dostime::dos_datetime_to_string(e.file_time_date),
                method: e.compression_method.to_string(),
                comment: String::new(),
                attributes: String::new(),
            })
            .collect(),
    };
    if out.len() > 100_000 {
        return Err("EGG/ALZ entry count exceeds 100000 limit".into());
    }
    let mut total = 0u64;
    for e in &out {
        total = total.checked_add(e.size).ok_or("archive size overflow")?;
    }
    if total > 8 * 1024 * 1024 * 1024 {
        return Err("EGG/ALZ decoded size exceeds 8 GiB staging limit".into());
    }
    let normalized: Vec<_> = out
        .iter()
        .map(|e| {
            engine::validate_name(&e.name)
                .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        })
        .collect::<Result<_, _>>()?;
    let mut seen = HashSet::new();
    for name in &normalized {
        if !seen.insert(name) {
            return Err("duplicate or case-colliding archive names".into());
        }
    }
    let files: HashSet<_> = out
        .iter()
        .zip(&normalized)
        .filter(|(e, _)| !e.is_dir)
        .map(|(_, name)| name.as_str())
        .collect();
    for name in &normalized {
        for (at, _) in name.match_indices('/') {
            if files.contains(&name[..at]) {
                return Err("archive file/directory collision".into());
            }
        }
    }
    Ok(out)
}
pub fn list(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    entries(&open(path, opts)?)
}
// Decoder callbacks check cancellation inside ALZ reads/writes, including a single
// large member. CPU-only segments between callbacks remain cooperative.
struct CheckedIo<T> {
    inner: T,
    control: Option<crate::engine::JobControl>,
    written: u64,
    limit: u64,
}
impl<T> CheckedIo<T> {
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
impl<T: Read> Read for CheckedIo<T> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        self.inner.read(b)
    }
}
impl<T: Write> Write for CheckedIo<T> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.check()?;
        if b.len() as u64 > self.limit.saturating_sub(self.written) {
            return Err(std::io::Error::other(
                "ALZ decoded output exceeds declared size",
            ));
        }
        let n = self.inner.write(b)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.check()?;
        self.inner.flush()
    }
}
fn decode_alz(
    a: &mut unalz::archive::AlzArchive,
    e: &unalz::archive::AlzFileEntry,
    stage: &Path,
    opts: &EngineOptions,
) -> Result<(), String> {
    use unalz::{
        archive::CompressionMethod,
        crypto::ZipCrypto,
        decompress::{bzip2, deflate, raw},
    };
    check(opts)?;
    let target = stage.join(engine::validate_name(&e.file_name)?);
    if e.is_directory() {
        fs::create_dir_all(target).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut crypto = if e.is_encrypted() {
        let pwd = opts.password.as_deref().ok_or("ALZ password required")?;
        let header = e
            .enc_check
            .as_ref()
            .ok_or("missing ALZ encryption header")?;
        let key = unalz::encoding::password_to_cp949(pwd);
        let mut c = ZipCrypto::new(&key);
        if !c.check_header(
            header,
            e.file_crc,
            e.file_time_date,
            e.has_data_descriptor(),
        ) {
            return Err("invalid ALZ password".into());
        }
        let mut c = ZipCrypto::new(&key);
        let mut copy = *header;
        c.decrypt(&mut copy);
        Some(c)
    } else {
        None
    };
    a.reader
        .seek(SeekFrom::Start(e.data_pos))
        .map_err(|e| e.to_string())?;
    let mut reader = CheckedIo {
        inner: (&mut a.reader).take(e.compressed_size),
        control: opts.control.clone(),
        written: 0,
        limit: 0,
    };
    fs::create_dir_all(target.parent().ok_or("missing staged parent")?)
        .map_err(|e| e.to_string())?;
    let mut writer = CheckedIo {
        inner: File::create(&target).map_err(|e| e.to_string())?,
        control: opts.control.clone(),
        written: 0,
        limit: e.uncompressed_size,
    };
    let result = match e.compression_method {
        CompressionMethod::Store => raw::extract_raw(
            &mut reader,
            &mut writer,
            e.compressed_size,
            e.uncompressed_size,
            crypto.as_mut(),
        ),
        CompressionMethod::Deflate => deflate::extract_deflate(
            &mut reader,
            &mut writer,
            e.compressed_size,
            e.uncompressed_size,
            crypto.as_mut(),
        ),
        CompressionMethod::Bzip2 => bzip2::extract_bzip2(
            &mut reader,
            &mut writer,
            e.compressed_size,
            e.uncompressed_size,
            crypto.as_mut(),
        ),
        _ => return Err("unsupported ALZ compression method".into()),
    }
    .map_err(|e| e.to_string())?;
    if result != e.file_crc || writer.written != e.uncompressed_size {
        return Err("ALZ CRC or decoded size mismatch".into());
    }
    writer.flush().map_err(|e| e.to_string())?;
    if let Some(t) = unalz::dostime::dos_datetime_to_systime(e.file_time_date) {
        writer.inner.set_modified(t).map_err(|e| e.to_string())?;
    }
    check(opts)
}
fn decode(
    a: &mut Archive,
    stage: &Path,
    chosen: &[Entry],
    opts: &EngineOptions,
) -> Result<(), String> {
    check(opts)?;
    let names: Vec<_> = chosen.iter().map(|e| e.name.clone()).collect();
    match a {
        Archive::Egg(a) => {
            // Library handles solid streams and validates block CRC/authentication.
            let original: Vec<_> = a
                .entries
                .iter()
                .filter(|e| names.contains(&e.file_name.replace('\\', "/")))
                .map(|e| e.file_name.clone())
                .collect();
            unegg::extract::extract_files(a, stage, opts.password.as_deref(), false, &original)
                .map_err(|e| e.to_string())?;
        }
        Archive::Alz(a) => {
            let es = a.entries.clone();
            for e in es {
                if names.contains(&e.file_name.replace('\\', "/")) {
                    check(opts)?;
                    decode_alz(a, &e, stage, opts)?;
                }
            }
        }
    }
    check(opts)?;
    for e in chosen.iter().filter(|e| !e.is_dir) {
        let p = stage.join(engine::validate_name(&e.name)?);
        engine::no_links(&p)?;
        if fs::metadata(p).map_err(|e| e.to_string())?.len() != e.size {
            return Err(format!("extracted size mismatch: {}", e.name));
        }
    }
    Ok(())
}
pub fn extract(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    let mut a = open(path, opts)?;
    let es = entries(&a)?;
    let chosen: Vec<_> = es
        .into_iter()
        .filter(|e| engine::selected(&e.name, opts))
        .collect();
    if !opts.selected.is_empty() && chosen.is_empty() {
        return Err("selected archive entries were not found".into());
    }
    let dest = std::path::absolute(dest).map_err(|e| e.to_string())?;
    engine::no_links(&dest)?;
    for e in &chosen {
        let p = dest.join(engine::validate_name(&e.name)?);
        engine::no_links(&p)?;
        if !e.is_dir && opts.overwrite == "error" && p.exists() {
            return Err(format!("destination exists: {}", p.display()));
        }
    }
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    decode(&mut a, stage.path(), &chosen, opts)?;
    let mut count = 0;
    for e in &chosen {
        check(opts)?;
        let p = dest.join(engine::validate_name(&e.name)?);
        engine::no_links(&p)?;
        if e.is_dir {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
            continue;
        }
        let Some(p) = engine::choose_target(p, &opts.overwrite)? else {
            continue;
        };
        let parent = p.parent().ok_or("missing output parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        engine::no_links(parent)?;
        let source = stage.path().join(engine::validate_name(&e.name)?);
        let mut input = File::open(&source).map_err(|e| e.to_string())?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut tmp).map_err(|e| e.to_string())?;
        if let Ok(t) = input.metadata().and_then(|m| m.modified()) {
            tmp.as_file()
                .set_times(std::fs::FileTimes::new().set_modified(t))
                .map_err(|e| e.to_string())?;
        }
        check(opts)?;
        engine::no_links(&p)?;
        if opts.overwrite == "overwrite" {
            tmp.persist(p).map_err(|e| e.error.to_string())?;
        } else {
            tmp.persist_noclobber(p).map_err(|e| e.error.to_string())?;
        }
        count += 1;
    }
    Ok(count)
}
pub fn test(path: &str, opts: &EngineOptions) -> Result<(), String> {
    let mut a = open(path, opts)?;
    let es = entries(&a)?;
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    decode(&mut a, stage.path(), &es, opts)
}
pub fn preview(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    let mut a = open(path, opts)?;
    let es = entries(&a)?;
    let e = es
        .into_iter()
        .find(|e| e.name == name)
        .ok_or("archive entry not found")?;
    if e.is_dir || e.size > 16 * 1024 * 1024 {
        return Err("preview requires a file of at most 16 MiB".into());
    }
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file = stage.path().join(engine::validate_name(&e.name)?);
    decode(&mut a, stage.path(), &[e], opts)?;
    fs::read(file).map_err(|e| e.to_string())
}
#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn reader_stops_after_cancellation_during_io() {
        let ctl = crate::engine::JobControl::new();
        let mut reader = ControlledReader {
            inner: Box::new(std::io::Cursor::new(vec![1, 2, 3])),
            control: Some(ctl.clone()),
        };
        let mut byte = [0; 1];
        reader.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], 1);
        ctl.cancel();
        assert!(reader.read_exact(&mut byte).is_err());
        assert!(reader.seek(std::io::SeekFrom::Start(0)).is_err());
    }
}
#[cfg(test)]
mod alz_io_tests {
    use super::*;
    #[test]
    fn decoder_stops_within_a_large_alz_member() {
        struct CancelAfterWrite {
            control: crate::engine::JobControl,
            bytes: usize,
        }
        impl Write for CancelAfterWrite {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.bytes += b.len();
                self.control.cancel();
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let control = crate::engine::JobControl::new();
        let mut output = CheckedIo {
            inner: CancelAfterWrite {
                control: control.clone(),
                bytes: 0,
            },
            control: Some(control),
            written: 0,
            limit: 1024 * 1024,
        };
        let input = vec![42; 1024 * 1024];
        let result = unalz::decompress::raw::extract_raw(
            &mut &input[..],
            &mut output,
            input.len() as u64,
            input.len() as u64,
            None,
        );
        assert!(result.is_err());
        assert!(output.inner.bytes > 0 && output.inner.bytes < input.len());
    }
    #[test]
    fn active_alz_write_cancel_and_output_bound() {
        let ctl = crate::engine::JobControl::new();
        let mut out = CheckedIo {
            inner: Vec::new(),
            control: Some(ctl.clone()),
            written: 0,
            limit: 2,
        };
        out.write_all(b"a").unwrap();
        ctl.cancel();
        assert!(out.write_all(b"b").is_err());
        assert_eq!(out.inner, b"a");
        let mut out = CheckedIo {
            inner: Vec::new(),
            control: None,
            written: 0,
            limit: 2,
        };
        assert!(out.write_all(b"abc").is_err());
        assert!(out.inner.is_empty());
    }
}
