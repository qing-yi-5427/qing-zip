//! ZIP file names stored without the UTF-8 flag.
//!
//! macOS Archive Utility, Java and many Linux tools write UTF-8 names without
//! setting general-purpose bit 11. Readers that then use the Windows code page
//! (7-Zip, older Qing-Zip) show mojibake. GBK/Shift-JIS byte sequences are almost
//! never valid UTF-8, so a central directory whose unflagged non-ASCII names
//! all decode as UTF-8 is read as UTF-8, as Bandizip and WinRAR do.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

/// Central directories larger than this are left to the normal code page.
const MAX_DIRECTORY: u64 = 64 * 1024 * 1024;

/// True when the ZIP at `path` has at least one non-ASCII name without the
/// UTF-8 flag and every such name is valid UTF-8.
pub fn implicit_utf8_names(path: &str) -> bool {
    File::open(path).ok().and_then(|mut file| scan(&mut file).ok()).unwrap_or(false)
}

fn scan(file: &mut File) -> std::io::Result<bool> {
    let mut signature = [0u8; 4];
    file.read_exact(&mut signature)?;
    if &signature != b"PK\x03\x04" && &signature != b"PK\x05\x06" {
        return Ok(false);
    }
    let Some((offset, size)) = directory(file)? else { return Ok(false) };
    if size > MAX_DIRECTORY {
        return Ok(false);
    }
    let mut central = vec![0u8; size as usize];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut central)?;
    Ok(decide(&central))
}

/// Decide from raw central-directory records.
fn decide(central: &[u8]) -> bool {
    let mut found = false;
    let mut at = 0usize;
    while at + 46 <= central.len() && central[at..at + 4] == *b"PK\x01\x02" {
        let u16_at = |i: usize| u16::from_le_bytes([central[at + i], central[at + i + 1]]) as usize;
        let (flags, name_len) = (u16_at(8), u16_at(28));
        let end = at + 46 + name_len;
        if end > central.len() {
            return false;
        }
        let name = &central[at + 46..end];
        if flags & 0x800 == 0 && !name.is_ascii() {
            if std::str::from_utf8(name).is_err() {
                return false;
            }
            found = true;
        }
        at = end + u16_at(30) + u16_at(32);
    }
    found
}

/// Offset and size of the central directory, from the classic or ZIP64 end record.
fn directory(file: &mut File) -> std::io::Result<Option<(u64, u64)>> {
    let length = file.seek(SeekFrom::End(0))?;
    let tail_len = length.min(22 + u16::MAX as u64);
    let mut tail = vec![0u8; tail_len as usize];
    file.seek(SeekFrom::Start(length - tail_len))?;
    file.read_exact(&mut tail)?;
    let Some(eocd) = (0..tail.len().saturating_sub(21)).rev().find(|&i| tail[i..i + 4] == *b"PK\x05\x06") else {
        return Ok(None);
    };
    let le32 = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap()) as u64;
    let le64 = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    let (size, offset) = (le32(&tail, eocd + 12), le32(&tail, eocd + 16));
    if size != 0xFFFF_FFFF && offset != 0xFFFF_FFFF {
        return Ok(Some((offset, size)));
    }
    // ZIP64: the locator sits immediately before the classic end record.
    let locator = (length - tail_len) + eocd as u64;
    if locator < 20 {
        return Ok(None);
    }
    let mut record = [0u8; 20];
    file.seek(SeekFrom::Start(locator - 20))?;
    file.read_exact(&mut record)?;
    if record[..4] != *b"PK\x06\x07" {
        return Ok(None);
    }
    let mut end64 = [0u8; 56];
    file.seek(SeekFrom::Start(le64(&record, 8)))?;
    file.read_exact(&mut end64)?;
    if end64[..4] != *b"PK\x06\x06" {
        return Ok(None);
    }
    Ok(Some((le64(&end64, 48), le64(&end64, 40))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn central(names: &[(&[u8], u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, flags) in names {
            let mut record = vec![0u8; 46];
            record[..4].copy_from_slice(b"PK\x01\x02");
            record[8..10].copy_from_slice(&flags.to_le_bytes());
            record[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend(record);
            out.extend_from_slice(name);
        }
        out
    }

    #[test]
    fn unflagged_utf8_names_are_detected_and_legacy_names_are_not() {
        let utf8 = "报表/数据.txt".as_bytes();
        let gbk = b"\xb1\xa8\xb1\xed.txt"; // 报表.txt in GBK
        assert!(decide(&central(&[(utf8, 0), (b"a.txt", 0)])));
        assert!(!decide(&central(&[(gbk, 0)])));
        assert!(!decide(&central(&[(utf8, 0), (gbk, 0)])));
        assert!(!decide(&central(&[(b"plain.txt", 0)])));
        // Flagged names are already UTF-8; a legacy neighbour still decides.
        assert!(!decide(&central(&[(utf8, 0x800)])));
        assert!(decide(&central(&[(utf8, 0x800), ("日本.txt".as_bytes(), 0)])));
        assert!(!decide(&central(&[(utf8, 0x800), (gbk, 0)])));
    }

    #[test]
    fn reads_real_archives_including_zip64() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        for zip64 in [false, true] {
            let path = dir.path().join(format!("names-{zip64}.zip"));
            let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
            let options = zip::write::SimpleFileOptions::default().large_file(zip64);
            writer.start_file("a.txt", options).unwrap();
            writer.write_all(b"a").unwrap();
            writer.finish().unwrap();
            // The zip crate sets the UTF-8 flag only for non-ASCII names; patch one in raw.
            let mut bytes = std::fs::read(&path).unwrap();
            assert!(!implicit_utf8_names(path.to_str().unwrap()));
            let name = "a.txt".as_bytes();
            let at = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
            let start = at + 46;
            assert_eq!(&bytes[start..start + name.len()], name);
            bytes[start..start + 3].copy_from_slice("é".as_bytes().iter().chain(b"x").copied().collect::<Vec<_>>().as_slice());
            std::fs::write(&path, &bytes).unwrap();
            assert!(implicit_utf8_names(path.to_str().unwrap()), "zip64={zip64}");
        }
        let other = dir.path().join("not.zip");
        std::fs::write(&other, b"7z\xbc\xaf\x27\x1c").unwrap();
        assert!(!implicit_utf8_names(other.to_str().unwrap()));
    }
}
