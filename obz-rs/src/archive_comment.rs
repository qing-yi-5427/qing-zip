//! ZIP archive comments: preserve all member bytes, replace only EOCD comment.
use crate::engine::{self, EngineOptions};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
fn read_at(file: &mut File, offset: u64, len: usize) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let mut b = vec![0; len];
    file.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
}
fn u16at(b: &[u8], n: usize) -> u16 {
    u16::from_le_bytes([b[n], b[n + 1]])
}
fn u32at(b: &[u8], n: usize) -> u32 {
    u32::from_le_bytes(b[n..n + 4].try_into().unwrap())
}
fn u64at(b: &[u8], n: usize) -> u64 {
    u64::from_le_bytes(b[n..n + 8].try_into().unwrap())
}
struct Directory {
    end: u64,
    comment: Vec<u8>,
}
fn inspect(file: &mut File, ctl: Option<&engine::JobControl>) -> Result<Directory, String> {
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    if len < 22 {
        return Err("not a ZIP archive".into());
    }
    let start = len.saturating_sub(65535 + 22);
    let tail = read_at(file, start, (len - start) as usize)?;
    let mut last_error = "valid ZIP end-of-directory record not found".to_owned();
    for i in (0..=tail.len() - 22).rev() {
        if let Some(ctl) = ctl {
            ctl.check()?;
        }
        if tail[i..i + 4] != *b"PK\x05\x06" || i + 22 + u16at(&tail, i + 20) as usize != tail.len()
        {
            continue;
        }
        let candidate = (|| -> Result<Directory, String> {
            let e = &tail[i..i + 22];
            let pos = start + i as u64;
            if u16at(e, 4) != 0 || u16at(e, 6) != 0 || u16at(e, 8) != u16at(e, 10) {
                return Err("multi-volume ZIP comments are unsupported".into());
            }
            let mut count = u16at(e, 10) as u64;
            let mut size = u32at(e, 12) as u64;
            let offset = u32at(e, 16) as u64;
            let mut directory_end = pos;
            let base;
            let locator = if pos >= 20 {
                read_at(file, pos - 20, 20)?
            } else {
                Vec::new()
            };
            if locator.starts_with(b"PK\x06\x07") {
                if u32at(&locator, 4) != 0 || u32at(&locator, 16) != 1 {
                    return Err("multi-volume ZIP64 comments are unsupported".into());
                }
                let zip64_pos = u64at(&locator, 8);
                if zip64_pos > pos.saturating_sub(76) {
                    return Err("invalid ZIP64 record offset".into());
                }
                let z = read_at(file, zip64_pos, 56)?;
                if !z.starts_with(b"PK\x06\x06")
                    || u64at(&z, 4) < 44
                    || zip64_pos
                        .checked_add(12)
                        .and_then(|v| v.checked_add(u64at(&z, 4)))
                        != Some(pos - 20)
                {
                    return Err("invalid ZIP64 record".into());
                }
                if u32at(&z, 16) != 0 || u32at(&z, 20) != 0 || u64at(&z, 24) != u64at(&z, 32) {
                    return Err("multi-volume ZIP64 comments are unsupported".into());
                }
                count = u64at(&z, 32);
                size = u64at(&z, 40);
                let offset = u64at(&z, 48);
                directory_end = zip64_pos;
                base = directory_end
                    .checked_sub(size)
                    .and_then(|v| v.checked_sub(offset))
                    .ok_or("invalid ZIP64 directory range")?;
            } else {
                if count == 65535 || size == u32::MAX as u64 || offset == u32::MAX as u64 {
                    return Err("ZIP64 locator missing".into());
                }
                base = pos
                    .checked_sub(size)
                    .and_then(|v| v.checked_sub(offset))
                    .ok_or("invalid ZIP directory range")?;
                if count == 0 && (size != 0 || offset != 0 || pos != 0) {
                    return Err("invalid empty ZIP record".into());
                }
            }
            let central = directory_end
                .checked_sub(size)
                .ok_or("invalid central directory size")?;
            if count > size / 46 || count > 1_000_000 {
                return Err("invalid or excessive ZIP member count".into());
            }
            let mut cursor = central;
            for _ in 0..count {
                if let Some(ctl) = ctl {
                    ctl.check()?;
                }
                if cursor.checked_add(46).is_none_or(|v| v > directory_end) {
                    return Err("truncated ZIP central directory".into());
                }
                let h = read_at(file, cursor, 46)?;
                if !h.starts_with(b"PK\x01\x02") {
                    return Err("invalid ZIP central directory signature".into());
                }
                if u16at(&h, 34) != 0 {
                    return Err("multi-volume member unsupported".into());
                }
                let local = u32at(&h, 42) as u64;
                if local != u32::MAX as u64 {
                    let local = base
                        .checked_add(local)
                        .ok_or("ZIP member offset overflow")?;
                    if local.checked_add(30).is_none_or(|n| n > central)
                        || read_at(file, local, 4)? != b"PK\x03\x04"
                    {
                        return Err("invalid ZIP member offset".into());
                    }
                }
                cursor = cursor
                    .checked_add(
                        46 + u16at(&h, 28) as u64 + u16at(&h, 30) as u64 + u16at(&h, 32) as u64,
                    )
                    .ok_or("ZIP directory overflow")?;
            }
            if cursor != directory_end {
                return Err("unexpected central directory trailing bytes".into());
            }
            return Ok(Directory {
                end: pos,
                comment: tail[i + 22..].to_vec(),
            });
        })();
        match candidate {
            Ok(d) => return Ok(d),
            Err(e) => last_error = e,
        }
    }
    Err(last_error)
}
pub fn get(path: &str) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    String::from_utf8(inspect(&mut file, None)?.comment)
        .map_err(|_| "ZIP comment is not UTF-8".into())
}
pub fn set(path: &str, text: &str, opts: &EngineOptions) -> Result<(), String> {
    if text.len() > 65535 {
        return Err("ZIP comment exceeds 65535 UTF-8 bytes".into());
    }
    let ctl = opts.control.clone().unwrap_or_default();
    ctl.check()?;
    let path = Path::new(path);
    engine::no_links(path)?;
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    engine::no_links(&path)?;
    let _lock = engine::update_lock(&path, &ctl)?;
    let mut source = File::open(&path).map_err(|e| e.to_string())?;
    let before = source.metadata().map_err(|e| e.to_string())?;
    let directory = inspect(&mut source, Some(&ctl))?;
    let mut temp =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("missing parent directory")?)
            .map_err(|e| e.to_string())?;
    source.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let length = directory.end + 20;
    let mut remaining = length;
    let mut buf = vec![0u8; 1024 * 1024];
    while remaining > 0 {
        ctl.check()?;
        let take = buf.len().min(remaining as usize);
        let n = source.read(&mut buf[..take]).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("ZIP changed while copying".into());
        }
        temp.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        remaining -= n as u64;
        ctl.set(
            "comment",
            Some((((length - remaining) as u128 * 100 / length as u128).min(99)) as u8),
            "Updating archive comment",
        );
    }
    temp.write_all(&(text.len() as u16).to_le_bytes())
        .map_err(|e| e.to_string())?;
    temp.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    let after = source.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("ZIP changed during comment update".into());
    }
    fs::set_permissions(temp.path(), before.permissions()).map_err(|e| e.to_string())?;
    ctl.check()?;
    engine::no_links(&path)?;
    drop(source);
    temp.persist(&path).map_err(|e| e.error.to_string())?;
    ctl.set("complete", Some(100), "Archive comment updated");
    Ok(())
}
