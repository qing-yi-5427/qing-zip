//! Decode direct ZealFS v1 images only after the engine confirms their type and extent.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

#[derive(Debug, Default)]
pub(crate) struct ArchiveHeader {
    kind: Option<String>,
    offset: Option<String>,
    physical_size: Option<String>,
    ambiguous: bool,
}

pub(crate) struct SourceProbe {
    root: Option<([u8; 256], usize)>,
}

impl SourceProbe {
    /// This is only a cheap candidate filter, never a format decision. Cache
    /// the root bytes so each allocated page is read at most once.
    pub(crate) fn read(source: &mut (impl Read + Seek)) -> Result<Self, String> {
        let mut root = [0u8; 256];
        source.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut count = source.read(&mut root[..1]).map_err(|e| e.to_string())?;
        if count == 0 || root[0] != b'Z' {
            return Ok(Self { root: None });
        }
        while count < root.len() {
            let read = source.read(&mut root[count..]).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            count += read;
        }
        Ok(Self {
            root: Some((root, count)),
        })
    }
    pub(crate) fn is_candidate(&self) -> bool {
        self.root.is_some()
    }
}

/// The console's first Type is a fixed handler name emitted before archive
/// properties/comments. Never scan comment text or item records for a Type.
pub(crate) fn first_type(bytes: &[u8]) -> Result<&str, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "engine returned invalid UTF-8")?;
    let mut started = false;
    for line in text.lines() {
        if line == "--" {
            if started {
                return Err("ambiguous archive type header".into());
            }
            started = true;
        } else if started {
            if let Some(kind) = line.strip_prefix("Type = ") {
                if !kind.is_empty() {
                    return Ok(kind);
                }
            }
            if line == "{"
                || line == "----------"
                || line.split_once(" = ").is_some_and(|(key, _)| key != "Path")
            {
                return Err("archive type header missing".into());
            }
        }
    }
    Err("archive type header missing".into())
}

/// Read only the engine's archive-header section, never entry properties. Full
/// technical listings separate that section from entries with ten hyphens.
/// Nested handlers (for example Split -> 7z) have separate `--` header records;
/// `----` introduces a wrapper's member properties rather than a new header.
pub(crate) fn header_payload(
    bytes: &[u8],
    listing: bool,
) -> Result<(Vec<ArchiveHeader>, &[u8]), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "engine returned invalid UTF-8")?;
    let mut headers = Vec::<ArchiveHeader>::new();
    let mut member_properties = false;
    let mut may_start_multiline = false;
    let mut consumed = 0;
    for raw in text.split_inclusive('\n') {
        consumed += raw.len();
        let line = raw.trim_end_matches(['\r', '\n']);
        if may_start_multiline && line == "{" {
            // Used only for a confirmed ZealFS (or candidate Split) header.
            // This handler has no multiline metadata. Never interpret a
            // comment's unescaped closing brace as a trustworthy boundary.
            return Err("ambiguous multiline archive header metadata".into());
        }
        may_start_multiline = false;
        if line == "--" {
            headers.push(ArchiveHeader::default());
            member_properties = false;
            continue;
        }
        if headers.is_empty() {
            continue;
        } // banner, scan progress, password prompt
        if line == "----------" && listing {
            validate_headers(&headers)?;
            return Ok((headers, listing_body(&bytes[consumed..])));
        }
        if !listing && line.is_empty() && headers.last().is_some_and(|h| h.kind.is_some()) {
            validate_headers(&headers)?;
            return Ok((headers, &bytes[consumed..]));
        }
        if line == "----" {
            member_properties = true;
            continue;
        }
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        may_start_multiline = value.is_empty();
        if member_properties {
            continue;
        }
        let header = headers.last_mut().unwrap();
        let field = match key {
            "Type" => &mut header.kind,
            "Offset" => &mut header.offset,
            "Physical Size" => &mut header.physical_size,
            _ => continue,
        };
        if field.replace(value.to_owned()).is_some() {
            header.ambiguous = true;
        }
    }
    if listing {
        return Err("archive listing entry separator missing".into());
    }
    validate_headers(&headers)?;
    Ok((headers, &bytes[bytes.len()..]))
}

// Full listings (unlike -ba) can append a warning count after all entry
// records. Recognize only that exact terminal summary with its blank record
// boundary. Never filter arbitrary lines out of an entry or accept errors.
fn listing_body(bytes: &[u8]) -> &[u8] {
    let text = std::str::from_utf8(bytes).expect("header parser validated UTF-8");
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let Some(start) = trimmed.rfind('\n').map(|n| n + 1) else {
        return bytes;
    };
    let Some(count) = trimmed[start..].strip_prefix("Warnings: ") else {
        return bytes;
    };
    if count.is_empty()
        || !count.bytes().all(|b| b.is_ascii_digit())
        || count.parse::<u32>().ok().is_none_or(|n| n == 0)
    {
        return bytes;
    }
    let preceding = &text[..start];
    if !preceding.ends_with("\n\n\n") && !preceding.ends_with("\r\n\r\n\r\n") {
        return bytes;
    }
    &bytes[..start]
}

fn validate_headers(headers: &[ArchiveHeader]) -> Result<(), String> {
    if headers.is_empty()
        || headers
            .iter()
            .any(|h| h.kind.as_deref().is_none_or(str::is_empty))
    {
        return Err("archive type header missing".into());
    }
    if headers.iter().any(|h| h.ambiguous) {
        return Err("ambiguous archive type/offset metadata".into());
    }
    Ok(())
}

pub(crate) struct Image {
    pub entries: Vec<crate::engine::Entry>,
    pub files: std::collections::HashMap<String, Vec<u8>>,
}

pub(crate) fn check(
    headers: &[ArchiveHeader],
    source: &mut File,
    probe: &SourceProbe,
) -> Result<Option<Image>, String> {
    let Some(header) = headers.iter().find(|h| {
        h.kind
            .as_deref()
            .is_some_and(|k| k.eq_ignore_ascii_case("ZealFS"))
    }) else {
        return Ok(None);
    };
    // A nested/offset filesystem would need the decoder's exact substream.
    // Never apply a beginning-of-file check to a different image location.
    if headers.len() != 1 || header.offset.as_deref().is_some_and(|v| v != "0") {
        return Err("embedded or offset ZealFS images are unsupported".into());
    }
    let physical = header
        .physical_size
        .as_deref()
        .ok_or("ZealFS physical size missing")?
        .parse::<u64>()
        .map_err(|_| "invalid ZealFS physical size")?;
    let length = source.metadata().map_err(|e| e.to_string())?.len();
    let (root, count) = probe
        .root
        .as_ref()
        .ok_or("ZealFS source is not a direct image")?;
    if *count != 256 {
        return Err("invalid or truncated ZealFS metadata".into());
    }
    check_root(source, root, length, physical, crate::machine_policy::scope().restricted()).map(Some)
}

#[cfg(test)]
fn check_image(source: &mut (impl Read + Seek), length: u64, physical: u64) -> Result<(), String> {
    let invalid = || "invalid or truncated ZealFS metadata".to_owned();
    let mut root = [0u8; 256];
    source.seek(SeekFrom::Start(0)).map_err(|_| invalid())?;
    source.read_exact(&mut root).map_err(|_| invalid())?;
    check_root(source, &root, length, physical, false).map(drop)
}

fn check_root(
    source: &mut (impl Read + Seek),
    root: &[u8; 256],
    length: u64,
    physical: u64,
    official_only: bool,
) -> Result<Image, String> {
    const PAGE: usize = 256;
    let invalid = || "invalid or truncated ZealFS metadata".to_owned();
    if root[0] != b'Z' || root[1] != 1 {
        return Err("unsupported ZealFS image version or offset".into());
    }
    let bitmap_size = usize::from(root[2]);
    if !(1..=32).contains(&bitmap_size) {
        return Err(invalid());
    }
    let pages = bitmap_size.checked_mul(8).ok_or_else(invalid)?;
    let extent = pages.checked_mul(PAGE).ok_or_else(invalid)? as u64;
    if extent > length
        || extent != physical
        || root[4] & 1 == 0
        || root[4 + bitmap_size..36].iter().any(|b| *b != 0)
    {
        return Err(invalid());
    }
    let allocated: usize = root[4..4 + bitmap_size]
        .iter()
        .map(|v| v.count_ones() as usize)
        .sum();
    if pages.checked_sub(allocated) != Some(usize::from(root[3])) {
        return Err(invalid());
    }
    // An entry owns its complete chain, including the first page of an empty
    // file. Never revisit allocated pages, even through a different directory.
    let mut seen_pages = [false; 256];
    let mut read_page = |page: usize, count: usize| -> Result<[u8; PAGE], String> {
        if page == 0 || page >= pages || root[4 + page / 8] & (1 << (page % 8)) == 0
            || std::mem::replace(&mut seen_pages[page], true)
        {
            return Err(invalid());
        }
        let mut buffer = [0; PAGE];
        source.seek(SeekFrom::Start((page * PAGE) as u64)).map_err(|_| invalid())?;
        source.read_exact(&mut buffer[..count]).map_err(|_| invalid())?;
        Ok(buffer)
    };
    let mut directories = vec![(String::new(), *root, 64)];
    let mut image = Image { entries: Vec::new(), files: Default::default() };
    while let Some((parent, records, start)) = directories.pop() {
        let mut sibling_names = std::collections::HashSet::new();
        for record in records[start..].chunks_exact(32) {
            if record[0] & !0x81 != 0 { return Err(invalid()); }
            if record[0] & 0x80 == 0 { continue; }
            let field = &record[1..17];
            let leaf = std::str::from_utf8(field.split(|b| *b == 0).next().unwrap())
                .map_err(|_| "invalid UTF-8 ZealFS entry name")?;
            if leaf.contains(['/', '\\']) || !sibling_names.insert(leaf.to_lowercase()) {
                return Err("invalid or duplicate ZealFS entry name".into());
            }
            crate::extract::relative_path(leaf)?;
            let name = if parent.is_empty() { leaf.to_owned() } else { format!("{parent}/{leaf}") };
            let size = u16::from_le_bytes([record[18], record[19]]) as usize;
            let is_dir = record[0] & 1 != 0;
            // Under machine policy use the official decoder for every payload.
            // Inspect only directory records and one link byte per file page.
            let read_len = if is_dir || !official_only { PAGE } else { 1 };
            let mut page = read_page(record[17] as usize, read_len)?;
            if is_dir {
                if size != PAGE { return Err(invalid()); }
                directories.push((name.clone(), page, 0));
            } else {
                if size > (pages - 1) * (PAGE - 1) { return Err(invalid()); }
                let mut bytes = Vec::new();
                let mut remaining = size;
                loop {
                    let count = remaining.min(PAGE - 1);
                    if !official_only { bytes.extend_from_slice(&page[1..1 + count]); }
                    remaining -= count;
                    if remaining == 0 {
                        if page[0] != 0 { return Err(invalid()); }
                        break;
                    }
                    page = read_page(page[0] as usize, read_len)?;
                }
                if !official_only { image.files.insert(name.to_lowercase(), bytes); }
            }
            image.entries.push(crate::engine::Entry {
                name, size: size as u64, packed_size: size as u64, size_known: true,
                is_dir, crc: String::new(), encrypted: false, modified: String::new(),
                method: String::new(), comment: String::new(), attributes: String::new(),
            });
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    pub(crate) fn fixture(nested: bool, empty_directory: bool) -> Vec<u8> {
        let mut data = vec![0u8; 2048];
        data[..5].copy_from_slice(&[b'Z', 1, 1, 6, 3]);
        data[64] = 0x80;
        data[65..74].copy_from_slice(b"hello.txt");
        data[81] = 1;
        data[82..84].copy_from_slice(&5u16.to_le_bytes());
        data[257..262].copy_from_slice(b"hello");
        if empty_directory || nested {
            data[3] = 5;
            data[4] = 7;
            data[96] = 0x81;
            data[97..103].copy_from_slice(b"nested");
            data[113] = 2;
            data[114..116].copy_from_slice(&256u16.to_le_bytes());
        }
        if nested {
            data[3] = 4;
            data[4] = 15;
            data[512] = 0x80;
            data[513..521].copy_from_slice(b"lost.txt");
            data[529] = 3;
            data[530..532].copy_from_slice(&4u16.to_le_bytes());
            data[769..773].copy_from_slice(b"lost");
        }
        data
    }
    fn inspect(bytes: Vec<u8>) -> Result<(), String> {
        let length = bytes.len() as u64;
        check_image(&mut Cursor::new(bytes), length, 2048)
    }
    #[test]
    fn root_and_nested_directories_are_verified() {
        inspect(fixture(false, false)).unwrap();
        inspect(fixture(false, true)).unwrap();
        inspect(fixture(true, false)).unwrap();
        let mut data = fixture(true, false);
        data[512] = 0x81;
        assert!(inspect(data).is_err());
    }
    #[test]
    fn every_chain_and_nested_name_is_checked() {
        // Populate a second payload page so that the leaf crosses its 255-byte boundary.
        let mut valid = fixture(true, false);
        valid[3] -= 1;
        valid[4] |= 1 << 4;
        valid[530..532].copy_from_slice(&256u16.to_le_bytes());
        valid[768] = 4;
        valid[1025] = 99;
        let mut source = Cursor::new(&valid[..]);
        let probe = SourceProbe::read(&mut source).unwrap();
        let image = check_root(&mut source, &probe.root.unwrap().0, 2048, 2048, false).unwrap();
        assert_eq!(image.files["nested/lost.txt"].len(), 256);
        assert_eq!(image.files["nested/lost.txt"][255], 99);
        for (offset, value) in [(768, 0), (768, 1), (768, 2), (768, 3), (768, 6), (1024, 2), (513, b'/')] {
            let mut bad = valid.clone();
            bad[offset] = value;
            assert!(inspect(bad).is_err(), "offset {offset}={value}");
        }
        let mut conflict = valid;
        let mut second: [u8; 32] = conflict[512..544].try_into().unwrap();
        second[1] = b'L';
        conflict[544..576].copy_from_slice(&second);
        assert!(inspect(conflict).unwrap_err().contains("duplicate"));
    }

    #[test]
    fn malformed_metadata_fails_closed() {
        for (index, value) in [
            (0, 0),
            (1, 2),
            (2, 0),
            (2, 33),
            (3, 8),
            (4, 6),
            (5, 1),
            (113, 0),
            (113, 8),
            (113, 7),
            (114, 1),
            (115, 0),
            (96, 0x83),
            (128, 2),
        ] {
            let mut data = fixture(false, true);
            data[index] = value;
            assert!(inspect(data).is_err(), "byte {index}={value}");
        }
        let mut data = fixture(false, true);
        data.truncate(650);
        assert!(inspect(data).is_err());
        assert!(check_image(&mut Cursor::new(fixture(false, false)), 2048, 4096).is_err());
    }
    #[test]
    fn actual_zeal_requires_single_known_offset_and_consistent_size() {
        let mut source = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut source, &fixture(false, false)).unwrap();
        let probe = SourceProbe::read(&mut source).unwrap();
        for header in [
            "--\nType = ZealFS\nPhysical Size = 2048\nOffset = 512\n",
            "--\nType = Split\n----\nPath = child\n--\nType = ZealFS\nPhysical Size = 2048\n",
            "--\nType = ZealFS\n",
            "--\nType = ZealFS\nPhysical Size = nope\n",
            "--\nType = ZealFS\nPhysical Size = 4096\n",
        ] {
            let output = format!("{header}\n----------\n");
            let (headers, _) = header_payload(output.as_bytes(), true).unwrap();
            assert!(check(&headers, &mut source, &probe).is_err());
        }
    }
    #[test]
    fn at_most_root_and_six_pages_are_read() {
        struct Counted {
            inner: Cursor<Vec<u8>>,
            read: usize,
        }
        impl Read for Counted {
            fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(data)?;
                self.read += n;
                Ok(n)
            }
        }
        impl Seek for Counted {
            fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
                self.inner.seek(to)
            }
        }
        let mut bytes = fixture(false, false);
        bytes[64..256].fill(0);
        bytes[3] = 1;
        bytes[4] = 0x7f;
        for i in 0..6 {
            let start = 64 + i * 32;
            bytes[start] = 0x81;
            bytes[start + 1] = b'a' + i as u8;
            bytes[start + 17] = i as u8 + 1;
            bytes[start + 19] = 1;
        }
        let mut source = Counted {
            inner: Cursor::new(bytes),
            read: 0,
        };
        let probe = SourceProbe::read(&mut source).unwrap();
        let (root, count) = probe.root.as_ref().unwrap();
        assert_eq!(*count, 256);
        check_root(&mut source, root, 2048, 2048, false).unwrap();
        assert_eq!(source.read, 1792);
    }
    #[test]
    fn header_is_separate_from_entries_comments_and_wrapper_members() {
        let output = b"banner\r\nEnter password (will not be echoed):\r\n--\r\nPath = archive.zip\r\nType = zip\r\nComment = \r\n{\r\nType = ZealFS\r\n----------\r\n}\r\n\r\n----------\r\nPath = Type = ZealFS\r\nSize = 0\r\n\r\n";
        assert_eq!(first_type(output).unwrap(), "zip");
        assert!(header_payload(output, true).is_err()); // never guess an unescaped comment's end
        let output = b"--\nPath = archive.001\nType = Split\n----\nPath = archive\nType = ignored member property\n--\nPath = archive\nType = 7z\n\n----------\nPath = data\nSize = 1\n";
        let (headers, _) = header_payload(output, true).unwrap();
        assert_eq!(
            headers
                .iter()
                .map(|h| h.kind.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["Split", "7z"]
        );
        let (_, anonymous) = header_payload(
            b"--\nPath = raw\nType = bzip2\n\n----------\nSize = 5\n",
            true,
        )
        .unwrap();
        assert!(anonymous.starts_with(b"Size ="));
        assert!(
            header_payload(b"--\nPath = empty\nType = zip\n\n----------\n", true)
                .unwrap()
                .1
                .is_empty()
        );
    }
    #[test]
    fn ambiguous_missing_or_unterminated_header_is_rejected() {
        for text in [
            "--\nPath = x\nType = zip\nType = ZealFS\n\n----------\n",
            "--\nPath = x\n\n----------\n",
            "--\nPath = x\nType = zip\n",
            "--\nPath = x\nType = ZealFS\nComment = \n{\n----------\n",
            "--\nPath = x\nComment = \n{\nType = zip\n}\n\n----------\n",
            "--\nPath = x\nType = zip\nWARNING = \n{\nType = ZealFS\n}\n\n----------\n",
        ] {
            assert!(header_payload(text.as_bytes(), true).is_err());
        }
        let (headers, _) = header_payload(
            b"banner\n--\nPath = x\nType = ZealFS\nPhysical Size = 2048\n\nEverything is Ok\n",
            false,
        )
        .unwrap();
        assert_eq!(headers[0].physical_size.as_deref(), Some("2048"));
    }
    #[test]
    fn only_terminal_warning_summary_is_separated_from_entries() {
        for newline in ["\n", "\r\n"] {
            let body = format!("Path = x{newline}Size = 0{newline}{newline}{newline}");
            let output = format!("{body}Warnings: 1{newline}");
            assert_eq!(listing_body(output.as_bytes()), body.as_bytes());
        }
        for text in [
            "Path = Warnings: 1\nSize = 0\n\n",
            "Path = x\nWarnings: 1\n",
            "Path = x\n\n\nWarnings: 1\n\nPath = other\n",
            "Path = x\n\n\nErrors: 1\n",
            "Path = x\n\n\nWarnings: nope\n",
        ] {
            assert_eq!(listing_body(text.as_bytes()), text.as_bytes());
        }
    }

    #[test]
    fn engine_decodes_nested_direct_filesystems_and_rejects_wrappers() {
        use crate::engine::{self, EngineOptions};
        // The fallback 7-Zip distribution has no ZealFS handler. Its rejection
        // is already safe; this fixture exercises the extended bundled backend.
        if !engine::executable()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("NanaZip")
        {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let options = EngineOptions::default();
        for (label, empty) in [("root", false), ("empty", true)] {
            let archive = temp.path().join(format!("{label}.bin"));
            std::fs::write(&archive, fixture(false, empty)).unwrap();
            let path = archive.to_str().unwrap();
            let entries = engine::list(path, &options).unwrap();
            assert!(entries.iter().any(|e| e.name == "hello.txt"));
            assert_eq!(entries.len(), if empty { 2 } else { 1 });
            engine::test(path, &options).unwrap();
            assert_eq!(
                engine::preview(path, "hello.txt", &options).unwrap(),
                b"hello"
            );
            let output = temp.path().join(format!("{label}-out"));
            engine::extract(path, output.to_str().unwrap(), &options).unwrap();
            assert_eq!(std::fs::read(output.join("hello.txt")).unwrap(), b"hello");
            if empty {
                assert!(output.join("nested").is_dir());
            }
        }
        for suffix in ["bin", "zip", "7z"] {
            let archive = temp.path().join(format!("nested.{suffix}"));
            std::fs::write(&archive, fixture(true, false)).unwrap();
            let path = archive.to_str().unwrap();
            let entries = engine::list(path, &options).unwrap();
            assert_eq!(entries.len(), 3);
            assert!(entries.iter().any(|e| e.name == "nested/lost.txt" && e.size == 4));
            engine::test(path, &options).unwrap();
            assert_eq!(engine::preview(path, "nested/lost.txt", &options).unwrap(), b"lost");
            for selected in [vec![], vec!["nested".into()]] {
                let whole = selected.is_empty();
                let options = EngineOptions { selected, ..Default::default() };
                let output = temp.path().join(format!("extracted-{suffix}-{whole}"));
                engine::extract(path, output.to_str().unwrap(), &options).unwrap();
                assert_eq!(std::fs::read(output.join("nested/lost.txt")).unwrap(), b"lost");
                assert_eq!(output.join("hello.txt").is_file(), whole);
            }
        }
        let disguised = temp.path().join("nested.tgz");
        std::fs::write(&disguised, fixture(true, false)).unwrap();
        assert!(engine::list(disguised.to_str().unwrap(), &options).is_err());
        let bad = temp.path().join("bad-unselected.zealfs");
        let mut damaged = fixture(true, false);
        damaged[768] = 3; // cycle in nested/lost.txt, outside the requested selection
        std::fs::write(&bad, damaged).unwrap();
        let existing = temp.path().join("preserve");
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("sentinel"), b"original").unwrap();
        let selected = EngineOptions { selected: vec!["hello.txt".into()], ..Default::default() };
        assert!(engine::extract(bad.to_str().unwrap(), existing.to_str().unwrap(), &selected).is_err());
        assert_eq!(std::fs::read(existing.join("sentinel")).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(existing).unwrap().count(), 1);
        let split = fixture(true, false);
        let first = temp.path().join("nested.zealfs.001");
        std::fs::write(&first, &split[..1024]).unwrap();
        std::fs::write(temp.path().join("nested.zealfs.002"), &split[1024..]).unwrap();
        for error in [
            engine::list(first.to_str().unwrap(), &options).unwrap_err(),
            engine::test(first.to_str().unwrap(), &options).unwrap_err(),
        ] {
            assert!(error.contains("ZealFS"), "{error}");
        }
        let malformed = temp.path().join("malformed.zealfs");
        let mut bytes = fixture(false, true);
        bytes[3] = 0; // engine ignores the inconsistent free-page counter
        std::fs::write(&malformed, bytes).unwrap();
        assert!(engine::list(malformed.to_str().unwrap(), &options)
            .unwrap_err()
            .contains("ZealFS"));
        assert!(engine::test(malformed.to_str().unwrap(), &options)
            .unwrap_err()
            .contains("ZealFS"));
    }

    #[cfg(nana_selected_worker)]
    #[test]
    fn machine_policy_keeps_payloads_on_the_official_engine() {
        use crate::{engine::{self, EngineOptions}, machine_policy::with_rules};
        let temp = tempfile::tempdir().unwrap();
        let flat = temp.path().join("flat.zealfs");
        let nested = temp.path().join("nested.zealfs");
        std::fs::write(&flat, fixture(false, true)).unwrap();
        std::fs::write(&nested, fixture(true, false)).unwrap();
        let flat = flat.to_str().unwrap();
        let nested = nested.to_str().unwrap();
        let opts = EngineOptions::default();
        with_rules([&["ZealFS"], &[], &[], &[]], || {
            engine::list(flat, &opts).unwrap();
            engine::test(flat, &opts).unwrap();
            assert_eq!(engine::preview(flat, "hello.txt", &opts).unwrap(), b"hello");
            let dest = temp.path().join("flat-out");
            engine::extract(flat, dest.to_str().unwrap(), &opts).unwrap();
            assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello");
            let rejected = temp.path().join("rejected");
            for result in [
                engine::list(nested, &opts).map(drop),
                engine::test(nested, &opts),
                engine::preview(nested, "hello.txt", &opts).map(drop),
                engine::extract(nested, rejected.to_str().unwrap(), &opts).map(drop),
            ] {
                assert!(result.unwrap_err().contains("omits entries"));
            }
            assert!(!rejected.exists());
        });
    }

    #[test]
    fn actual_zip_header_not_magic_or_comments_controls_the_guard() {
        use crate::engine::{self, EngineOptions};
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("前缀.zip");
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.set_comment("}\n\n----------\nPath = injected.txt\nSize = 0\n--\nType = ZealFS\nType = zip\nType = 7z\nPhysical Size = 2048").unwrap();
        writer
            .start_file("hello.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"hello").unwrap();
        std::fs::write(&archive, writer.finish().unwrap().into_inner()).unwrap();
        let path = archive.to_str().unwrap();
        let options = EngineOptions::default();
        let entries = engine::list(path, &options).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "hello.txt");
        engine::test(path, &options).unwrap();
        assert_eq!(
            engine::preview(path, "hello.txt", &options).unwrap(),
            b"hello"
        );
        let output = temp.path().join("output");
        engine::extract(path, output.to_str().unwrap(), &options).unwrap();
        assert_eq!(std::fs::read(output.join("hello.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 1);
        assert!(!output.join("injected.txt").exists());
        // Only the decoder knows which embedded stream it actually opened.
        // A ZIP header must bypass the image check even with a Zeal-like prefix.
        let prefix = temp.path().join("prefix.bin");
        std::fs::write(&prefix, b"Z\x01not a ZealFS image").unwrap();
        let (headers, _) =
            header_payload(b"--\nType = zip\nOffset = 512\n\n----------\n", true).unwrap();
        let mut source = File::open(&prefix).unwrap();
        let probe = SourceProbe::read(&mut source).unwrap();
        check(&headers, &mut source, &probe).unwrap();
    }

    #[test]
    fn rar5_multiline_comment_stays_on_the_original_bare_protocol() {
        use crate::engine::{self, EngineOptions};
        // Minimal Store RAR5 + CMT service from the published RAR5 structures.
        // All lengths here fit one-byte vint; no compression implementation.
        fn block(body: &[u8], payload: &[u8]) -> Vec<u8> {
            assert!(body.len() < 128);
            let mut header = vec![body.len() as u8];
            header.extend_from_slice(body);
            let mut out = crc32fast::hash(&header).to_le_bytes().to_vec();
            out.extend_from_slice(&header);
            out.extend_from_slice(payload);
            out
        }
        fn item(kind: u8, name: &[u8], data: &[u8]) -> Vec<u8> {
            assert!(data.len() < 128 && name.len() < 128);
            let mut body = vec![
                kind,
                2,
                data.len() as u8,
                4,
                data.len() as u8,
                if kind == 3 { 0 } else { 0x20 },
            ];
            body.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
            body.extend_from_slice(&[0, 0, name.len() as u8]);
            body.extend_from_slice(name);
            block(&body, data)
        }
        let mut bytes = b"Rar!\x1a\x07\x01\x00".to_vec();
        bytes.extend(block(&[1, 0, 0], &[]));
        bytes.extend(item(
            3,
            b"CMT",
            b"}\n\n----------\n--\nType = ZealFS\nPhysical Size = 2048\n",
        ));
        bytes.extend(item(2, b"hello.txt", b"hello"));
        bytes.extend(block(&[5, 0, 0], &[]));
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("comment.rar");
        std::fs::write(&path, bytes).unwrap();
        let path = path.to_str().unwrap();
        let options = EngineOptions::default();
        let entries = engine::list(path, &options).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "hello.txt");
        engine::test(path, &options).unwrap();
        assert_eq!(
            engine::preview(path, "hello.txt", &options).unwrap(),
            b"hello"
        );
        let output = temp.path().join("output");
        engine::extract(path, output.to_str().unwrap(), &options).unwrap();
        assert_eq!(std::fs::read(output.join("hello.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read_dir(output).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn pinned_guard_source_keeps_file_and_parent_identity_until_finished() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        let path = parent.join("image.bin");
        std::fs::write(&path, fixture(false, false)).unwrap();
        let mut source = crate::engine_stream::pin(&path).unwrap();
        let probe = SourceProbe::read(&mut *source).unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(std::fs::rename(&parent, temp.path().join("moved")).is_err());
        let (headers, _) = header_payload(
            b"--\nType = ZealFS\nPhysical Size = 2048\n\n----------\n",
            true,
        )
        .unwrap();
        check(&headers, &mut source, &probe).unwrap();
        drop(source);
        std::fs::rename(&parent, temp.path().join("moved")).unwrap();
    }
}
