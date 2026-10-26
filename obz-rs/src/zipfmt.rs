//! Minimal ZIP reader/writer (store + deflate) for the installer UI layer
//! and the generic `obz` archive commands.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{crc32, Reader, MAX_NATIVE_OUTPUT};

pub struct ZipEntry {
    pub name: String,
    pub size: u64,
    pub comp_size: u64,
    pub method: u16,
    pub crc: u32,
    pub local_offset: u32,
}

pub fn read_central_dir(data: &[u8], start: usize, end: usize) -> Result<Vec<ZipEntry>, String> {
    let slice = data.get(start..end).ok_or("ZIP range exceeds input")?;
    let eocd_rel = (0..slice.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            slice.get(i..i + 4) == Some(b"PK\x05\x06")
                && i + 22 + u16::from_le_bytes([slice[i + 20], slice[i + 21]]) as usize
                    == slice.len()
        })
        .ok_or("ZIP EOCD missing or truncated")?;
    let mut r = Reader::new(slice);
    r.p = eocd_rel + 4;
    let disk = r.u16()?;
    let cd_disk = r.u16()?;
    let disk_count = r.u16()?;
    let count = r.u16()? as usize;
    let cd_size = r.u32()? as usize;
    let cd_off = r.u32()? as usize;
    if disk != 0 || cd_disk != 0 || disk_count as usize != count {
        return Err("multi-disk ZIP unsupported".into());
    }
    if count == 0xffff || cd_size == u32::MAX as usize || cd_off == u32::MAX as usize {
        return Err("ZIP64 unsupported".into());
    }
    if count == 0 && cd_size == 0 {
        return Ok(Vec::new());
    }
    if cd_size > eocd_rel {
        return Err("invalid central directory size".into());
    }

    // Concatenated zips may record file-relative offsets; re-anchor the
    // central directory like Python's zipfile `concat` adjustment.
    let mut p = usize::MAX;
    for cand in [cd_off, eocd_rel.saturating_sub(cd_size)] {
        if cand + 4 <= slice.len() && &slice[cand..cand + 4] == b"PK\x01\x02" {
            p = cand;
            break;
        }
    }
    if p == usize::MAX {
        return Err("bad zip central directory record".into());
    }
    let cd_end = p
        .checked_add(cd_size)
        .filter(|&end| end <= eocd_rel)
        .ok_or("central directory exceeds EOCD")?;
    let delta = p as i64 - cd_off as i64;

    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if p + 46 > cd_end || &slice[p..p + 4] != b"PK\x01\x02" {
            return Err("bad zip central directory record".into());
        }
        let mut c = Reader::new(slice);
        c.p = p + 8;
        let flags = c.u16()?;
        if flags & 1 != 0 {
            return Err("encrypted ZIP unsupported".into());
        }
        let method = c.u16()?;
        c.u16()?; // mod time
        c.u16()?; // mod date
        let crc = c.u32()?;
        let comp_size = c.u32()? as u64;
        let size = c.u32()? as u64;
        let name_len = c.u16()? as usize;
        let extra_len = c.u16()? as usize;
        let comment_len = c.u16()? as usize;
        c.u16()?; // disk
        c.u16()?; // internal attrs
        c.u32()?; // external attrs
        let local_offset =
            u32::try_from(c.u32()? as i64 + delta).map_err(|_| "invalid local offset")?;
        let record_end = p + 46 + name_len + extra_len + comment_len;
        if record_end > cd_end {
            return Err("truncated central directory entry".into());
        }
        let name_bytes = c.read(name_len)?;
        entries.push(ZipEntry {
            name: String::from_utf8_lossy(name_bytes).into_owned(),
            size,
            comp_size,
            method,
            crc,
            local_offset,
        });
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(entries)
}

pub fn extract_entry(data: &[u8], base: usize, e: &ZipEntry) -> Result<Vec<u8>, String> {
    if e.size > MAX_NATIVE_OUTPUT as u64 || e.comp_size > MAX_NATIVE_OUTPUT as u64 {
        return Err("native ZIP output exceeds 512 MiB limit".into());
    }
    let abs = base
        .checked_add(e.local_offset as usize)
        .ok_or("ZIP offset overflow")?;
    let header_end = abs.checked_add(30).ok_or("ZIP offset overflow")?;
    let header = data
        .get(abs..header_end)
        .ok_or("truncated ZIP local header")?;
    if &header[..4] != b"PK\x03\x04" {
        return Err(format!("bad ZIP local header for {}", e.name));
    }
    let mut r = Reader::new(header);
    r.p = 6;
    if r.u16()? & 1 != 0 {
        return Err("encrypted ZIP unsupported".into());
    }
    if r.u16()? != e.method {
        return Err("ZIP method mismatch".into());
    }
    r.p = 26;
    let name_len = r.u16()? as usize;
    let extra_len = r.u16()? as usize;
    let data_start = header_end
        .checked_add(name_len + extra_len)
        .ok_or("ZIP offset overflow")?;
    let comp_size = usize::try_from(e.comp_size).map_err(|_| "ZIP size overflow")?;
    let data_end = data_start
        .checked_add(comp_size)
        .ok_or("ZIP size overflow")?;
    let comp = data
        .get(data_start..data_end)
        .ok_or("truncated ZIP compressed data")?;
    let out = match e.method {
        0 => {
            if comp.len() as u64 != e.size {
                return Err("stored ZIP size mismatch".into());
            }
            comp.to_vec()
        }
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(comp, e.size as usize)
            .map_err(|err| format!("inflate {}: {err:?}", e.name))?,
        other => return Err(format!("zip method {other} not supported for {}", e.name)),
    };
    if out.len() as u64 != e.size {
        return Err(format!("ZIP size mismatch for {}", e.name));
    }
    if crc32(&out) != e.crc {
        return Err(format!("zip CRC mismatch for {}", e.name));
    }
    Ok(out)
}

pub fn create_zip(out_path: &str, inputs: &[String], store_only: bool) -> Result<usize, String> {
    let mut locals: Vec<u8> = Vec::new();
    let mut centrals: Vec<u8> = Vec::new();
    let mut count = 0usize;
    let (dos_time, dos_date) = dos_stamp();

    let mut files: Vec<(String, String)> = Vec::new(); // (arcname, fs path)
    for src in inputs {
        let p = Path::new(src);
        if p.is_dir() {
            collect_dir(
                p,
                p.file_name().map(|s| s.to_string_lossy().into_owned()),
                &mut files,
            )?;
        } else {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            files.push((name, src.clone()));
        }
    }

    for (arc, fspath) in files {
        let raw = fs::read(&fspath).map_err(|e| format!("{fspath}: {e}"))?;
        let crc = crc32(&raw);
        let (method, comp) = if store_only {
            (0u16, raw.clone())
        } else {
            (8u16, miniz_oxide::deflate::compress_to_vec(&raw, 6))
        };
        let name = arc.replace('\\', "/");
        let offset = locals.len() as u32;
        let mut lh = Vec::new();
        lh.extend_from_slice(b"PK\x03\x04");
        lh.extend_from_slice(&20u16.to_le_bytes()); // version needed
        lh.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        lh.extend_from_slice(&method.to_le_bytes());
        lh.extend_from_slice(&dos_time.to_le_bytes());
        lh.extend_from_slice(&dos_date.to_le_bytes());
        lh.extend_from_slice(&crc.to_le_bytes());
        lh.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        lh.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        lh.extend_from_slice(&(name.len() as u16).to_le_bytes());
        lh.extend_from_slice(&0u16.to_le_bytes()); // extra
        lh.extend_from_slice(name.as_bytes());
        lh.extend_from_slice(&comp);
        locals.extend_from_slice(&lh);

        let mut ch = Vec::new();
        ch.extend_from_slice(b"PK\x01\x02");
        ch.extend_from_slice(&20u16.to_le_bytes()); // version made by
        ch.extend_from_slice(&20u16.to_le_bytes()); // version needed
        ch.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        ch.extend_from_slice(&method.to_le_bytes());
        ch.extend_from_slice(&dos_time.to_le_bytes());
        ch.extend_from_slice(&dos_date.to_le_bytes());
        ch.extend_from_slice(&crc.to_le_bytes());
        ch.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        ch.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        ch.extend_from_slice(&(name.len() as u16).to_le_bytes());
        ch.extend_from_slice(&0u16.to_le_bytes()); // extra
        ch.extend_from_slice(&0u16.to_le_bytes()); // comment
        ch.extend_from_slice(&0u16.to_le_bytes()); // disk
        ch.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        ch.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        ch.extend_from_slice(&offset.to_le_bytes());
        ch.extend_from_slice(name.as_bytes());
        centrals.extend_from_slice(&ch);
        count += 1;
    }

    let mut out = Vec::new();
    out.extend_from_slice(&locals);
    let cd_off = out.len() as u32;
    out.extend_from_slice(&centrals);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(count as u16).to_le_bytes());
    out.extend_from_slice(&(count as u16).to_le_bytes());
    out.extend_from_slice(&(centrals.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    let mut f = fs::File::create(out_path).map_err(|e| format!("{out_path}: {e}"))?;
    f.write_all(&out).map_err(|e| e.to_string())?;
    Ok(count)
}

fn collect_dir(
    dir: &Path,
    prefix: Option<String>,
    out: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut names: Vec<_> = rd.flatten().collect();
    names.sort_by_key(|e| e.file_name());
    for entry in names {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let arc = match &prefix {
            Some(p) => format!("{p}/{name}"),
            None => name,
        };
        if path.is_dir() {
            collect_dir(&path, Some(arc), out)?;
        } else {
            out.push((arc, path.to_string_lossy().into_owned()));
        }
    }
    Ok(())
}

fn dos_stamp() -> (u16, u16) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let t = secs % 86400;
    let time = ((t / 3600) as u16) << 11 | ((t % 3600 / 60) as u16) << 5 | (t % 60 / 2) as u16;
    let date = ((y - 1980) as u16) << 9 | (m as u16) << 5 | d as u16;
    (time, date)
}

/// Days since epoch -> (year, month, day). Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
