//! ZIP file names stored without the UTF-8 flag.
//!
//! macOS Archive Utility, Java and many Linux tools write UTF-8 names without
//! setting general-purpose bit 11. Use UTF-8 when the central directory has
//! unflagged non-ASCII names and all of them decode as UTF-8. Some legacy byte
//! sequences are valid UTF-8 too, so the explicit code-page choice takes priority.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Central directories larger than this are left to the normal code page.
const MAX_DIRECTORY: u64 = 64 * 1024 * 1024;

pub fn implicit_utf8_names(path: &str) -> bool {
    crate::engine_stream::open_source_leaf(Path::new(path))
        .ok()
        .and_then(|mut file| scan(&mut file).ok())
        .unwrap_or(false)
}

fn scan(file: &mut File) -> std::io::Result<bool> {
    let mut signature = [0u8; 4];
    file.read_exact(&mut signature)?;
    if &signature != b"PK\x03\x04" && &signature != b"PK\x05\x06" {
        return Ok(false);
    }
    let Ok(Some(end)) = crate::engine::zip_volume_view::single_disk_record(file) else {
        return Ok(false);
    };
    let (offset, size, entries, boundary) = if end.zip64 {
        let Some(locator) = end.end_record.checked_sub(20) else {
            return Ok(false);
        };
        let mut record = [0u8; 20];
        file.seek(SeekFrom::Start(locator))?;
        file.read_exact(&mut record)?;
        let offset64 = le64(&record, 8);
        if offset64
            .checked_add(56)
            .is_none_or(|minimum_end| minimum_end > locator)
        {
            return Ok(false);
        }
        let mut end64 = [0u8; 56];
        file.seek(SeekFrom::Start(offset64))?;
        file.read_exact(&mut end64)?;
        if end64[..4] != *b"PK\x06\x06"
            || offset64
                .checked_add(12)
                .and_then(|header| header.checked_add(le64(&end64, 4)))
                != Some(locator)
            || end64[16..24] != [0; 8]
            || end64[24..32] != end64[32..40]
        {
            return Ok(false);
        }
        (
            le64(&end64, 48),
            le64(&end64, 40),
            le64(&end64, 32),
            offset64,
        )
    } else {
        (
            u64::from(end.directory),
            u64::from(end.directory_size),
            u64::from(end.entries),
            end.end_record,
        )
    };
    if size > MAX_DIRECTORY || offset.checked_add(size) != Some(boundary) {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(offset))?;
    decide(BufReader::new(file), size, entries)
}

fn le64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// Decide from the full set of central-directory member names, retaining only
/// one name at a time. Extra fields and file comments do not need to be decoded.
fn decide(
    mut central: BufReader<impl Read + Seek>,
    mut remaining: u64,
    entries: u64,
) -> std::io::Result<bool> {
    let mut found = false;
    let mut header = [0u8; 46];
    let mut name = Vec::new();
    for _ in 0..entries {
        if remaining < header.len() as u64 {
            return Ok(false);
        }
        central.read_exact(&mut header)?;
        if header[..4] != *b"PK\x01\x02" {
            return Ok(false);
        }
        let u16_at = |i: usize| u16::from_le_bytes([header[i], header[i + 1]]);
        let name_len = u16_at(28) as usize;
        let skipped = u64::from(u16_at(30)) + u64::from(u16_at(32));
        let record_size = header.len() as u64 + name_len as u64 + skipped;
        if record_size > remaining {
            return Ok(false);
        }
        if u16_at(8) & 0x800 == 0 {
            name.resize(name_len, 0);
            central.read_exact(&mut name)?;
            if !name.is_ascii() {
                if std::str::from_utf8(&name).is_err() {
                    return Ok(false);
                }
                found = true;
            }
            central.seek_relative(skipped as i64)?;
        } else {
            central.seek_relative((name_len as u64 + skipped) as i64)?;
        }
        remaining -= record_size;
    }
    Ok(found && remaining == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected(names: &[(&[u8], u16)]) -> bool {
        let mut out = Vec::new();
        for (name, flags) in names {
            let mut record = vec![0u8; 46];
            record[..4].copy_from_slice(b"PK\x01\x02");
            record[8..10].copy_from_slice(&flags.to_le_bytes());
            record[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend(record);
            out.extend_from_slice(name);
        }
        let size = out.len() as u64;
        decide(
            BufReader::new(std::io::Cursor::new(out)),
            size,
            names.len() as u64,
        )
        .unwrap()
    }

    #[test]
    fn unflagged_utf8_names_are_detected_and_legacy_names_are_not() {
        let utf8 = "报表/数据.txt".as_bytes();
        let gbk = b"\xb1\xa8\xb1\xed.txt"; // 报表.txt in GBK
        assert!(detected(&[(utf8, 0), (b"a.txt", 0)]));
        assert!(!detected(&[(gbk, 0)]));
        assert!(!detected(&[(utf8, 0), (gbk, 0)]));
        assert!(!detected(&[(b"plain.txt", 0)]));
        // Flagged names are already UTF-8; a legacy neighbour still decides.
        assert!(!detected(&[(utf8, 0x800)]));
        assert!(detected(&[(utf8, 0x800), ("日本.txt".as_bytes(), 0)]));
        assert!(!detected(&[(utf8, 0x800), (gbk, 0)]));
    }

    #[test]
    fn reads_real_archives_including_zip64() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        for zip64 in [false, true] {
            let path = dir.path().join(format!("names-{zip64}.zip"));
            let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
            if zip64 {
                writer.set_raw_zip64_extensible_data_sector(Box::new([]));
            }
            let options = zip::write::SimpleFileOptions::default().large_file(zip64);
            writer.start_file("a.txt", options).unwrap();
            writer.write_all(b"a").unwrap();
            writer.finish().unwrap();
            // The zip crate sets the UTF-8 flag only for non-ASCII names; patch one in raw.
            let mut bytes = std::fs::read(&path).unwrap();
            assert_eq!(
                bytes.windows(4).any(|signature| signature == b"PK\x06\x07"),
                zip64
            );
            assert!(!implicit_utf8_names(path.to_str().unwrap()));
            let name = "a.txt".as_bytes();
            let at = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
            let start = at + 46;
            assert_eq!(&bytes[start..start + name.len()], name);
            bytes[start..start + 3].copy_from_slice("éx".as_bytes());
            let comment = b"contains PK\x05\x06 within the archive comment";
            let comment_size = bytes.len() - 2;
            bytes[comment_size..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
            bytes.extend(comment);
            std::fs::write(&path, &bytes).unwrap();
            assert!(implicit_utf8_names(path.to_str().unwrap()), "zip64={zip64}");
        }
        let other = dir.path().join("not.zip");
        std::fs::write(&other, b"7z\xbc\xaf\x27\x1c").unwrap();
        assert!(!implicit_utf8_names(other.to_str().unwrap()));
    }
}
