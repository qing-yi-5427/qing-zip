//! Bounded identification only. The official handler still lists, validates,
//! decrypts and extracts the archive; no payload is accepted by this probe.
use super::*;

fn u64_at(data: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(data[at..at + 8].try_into().unwrap())
}
fn read_at(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn vint(bytes: &[u8], position: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*position)?; *position += 1;
        if shift == 63 && byte > 1 { return None; }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 { return Some(value); }
    }
    None
}
pub(super) fn signature(file: &mut File) -> Result<Option<&'static str>, String> {
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let bytes = read_at(file, 0, length.min(64) as usize)?;
    let format = if bytes.starts_with(b"7z\xbc\xaf\x27\x1c") && bytes.len() >= 32 {
        (crc32fast::hash(&bytes[12..32]) == zip_u32(&bytes, 8)
            && u64_at(&bytes, 12).checked_add(u64_at(&bytes, 20)).and_then(|n| n.checked_add(32)).is_some_and(|n| n <= length))
            .then_some("7z")
    } else if bytes.starts_with(b"Rar!\x1a\x07\x00") && bytes.len() >= 14 {
        let size = usize::from(zip_u16(&bytes, 12));
        if bytes[9] != 0x73 || zip_u16(&bytes, 10) & 1 != 0 || size < 13 || 7 + size as u64 > length { None }
        else {
            let header = read_at(file, 7, size)?;
            ((crc32fast::hash(&header[2..]) as u16) == zip_u16(&header, 0)).then_some("rar")
        }
    } else if bytes.starts_with(b"Rar!\x1a\x07\x01\x00") {
        let mut pos = 12;
        if let Some(size) = vint(&bytes, &mut pos).filter(|n| *n <= 1024 * 1024) {
            if pos as u64 + size > length { None } else {
                let header = read_at(file, 8, pos - 8 + size as usize)?;
                let mut cursor = pos - 8;
                let kind = vint(&header, &mut cursor);
                let flags = vint(&header, &mut cursor);
                if flags.is_some_and(|n| n & 1 != 0) { let _ = vint(&header, &mut cursor); }
                let archive_flags = vint(&header, &mut cursor);
                (kind == Some(1) && flags.is_some_and(|n| n & !1 == 0)
                    && archive_flags.is_some_and(|n| n & 1 == 0)
                    && crc32fast::hash(&header[4..]) == zip_u32(&header, 0)).then_some("rar5")
            }
        } else { None }
    } else { None };
    file.rewind().map_err(|e| e.to_string())?;
    Ok(format)
}

// A single, structurally complete, single-disk directory authorizes a bounded
// ZIP search. A false/ambiguous EOCD, multi-disk record, unsupported ZIP64
// layout or excessive directory falls back to the captured-volume path.
pub(super) fn zip_search_limit(file: &mut File, ctl: &JobControl, deep: bool) -> Result<Option<u64>, String> {
    let result = probe(file, ctl, deep);
    file.rewind().map_err(|e| e.to_string())?;
    result
}
fn probe(file: &mut File, ctl: &JobControl, deep: bool) -> Result<Option<u64>, String> {
    const TAIL_LIMIT: u64 = 1024 * 1024;
    const DIRECTORY_LIMIT: u64 = 8 * 1024 * 1024;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    // Search from the end in fixed windows, not by growing an allocation or
    // copying the archive. The 21-byte overlap includes boundary EOCD headers.
    // Only archive-like extensions authorize looking beyond the first window.
    let mut window_end = length;
    let (tail_start, tail) = loop {
        ctl.check()?;
        let start = window_end.saturating_sub(TAIL_LIMIT);
        let window = read_at(file, start, (window_end - start) as usize)?;
        let ends: Vec<usize> = window.windows(22).enumerate().filter_map(|(at, bytes)| {
            (bytes.starts_with(b"PK\x05\x06")
                && start + at as u64 + 22 + u64::from(zip_u16(bytes, 20)) <= length).then_some(at)
        }).collect();
        if ends.len() > 1 { return Ok(None); }
        if let Some(&at) = ends.first() {
            let absolute = start + at as u64;
            let context_start = absolute.saturating_sub(TAIL_LIMIT);
            let context_end = length.min(absolute + 22 + 65535);
            break (context_start, read_at(file, context_start, (context_end - context_start) as usize)?);
        }
        if start == 0 || !deep { return Ok(None); }
        window_end = start + 21;
    };
    let ends: Vec<usize> = tail.windows(22).enumerate().filter_map(|(at, bytes)| {
        (bytes.starts_with(b"PK\x05\x06") && at + 22 + usize::from(zip_u16(bytes, 20)) <= tail.len()).then_some(at)
    }).collect();
    if ends.len() != 1 { return Ok(None); }
    let at = ends[0];
    let end = &tail[at..at + 22];
    if zip_u16(end, 4) != 0 || zip_u16(end, 6) != 0 || zip_u16(end, 8) != zip_u16(end, 10) { return Ok(None); }
    let mut count = u64::from(zip_u16(end, 10));
    let mut directory_size = u64::from(zip_u32(end, 12));
    let mut directory_offset = u64::from(zip_u32(end, 16));
    let mut directory_end = tail_start + at as u64;
    let mut locator_offset = None;
    if at >= 20 && tail[at - 20..at - 16] == *b"PK\x06\x07" {
        let locator = &tail[at - 20..at];
        if zip_u32(locator, 4) != 0 || zip_u32(locator, 16) != 1 { return Ok(None); }
        let records: Vec<usize> = tail[..at - 20].windows(56).enumerate().filter_map(|(pos, bytes)| {
            (bytes.starts_with(b"PK\x06\x06") && u64_at(bytes, 4) >= 44
                && (pos as u64).checked_add(12).and_then(|p| p.checked_add(u64_at(bytes, 4))) == Some((at - 20) as u64)).then_some(pos)
        }).collect();
        if records.len() != 1 { return Ok(None); }
        let pos = records[0];
        let record = &tail[pos..pos + 56];
        if zip_u32(record, 16) != 0 || zip_u32(record, 20) != 0 || u64_at(record, 24) != u64_at(record, 32) { return Ok(None); }
        count = u64_at(record, 32);
        directory_size = u64_at(record, 40);
        directory_offset = u64_at(record, 48);
        directory_end = tail_start + pos as u64;
        locator_offset = Some(u64_at(locator, 8));
        // Non-sentinel classic fields must agree with the ZIP64 record.
        if (zip_u16(end, 10) != u16::MAX && u64::from(zip_u16(end, 10)) != count)
            || (zip_u32(end, 12) != u32::MAX && u64::from(zip_u32(end, 12)) != directory_size)
            || (zip_u32(end, 16) != u32::MAX && u64::from(zip_u32(end, 16)) != directory_offset) { return Ok(None); }
    } else if count == u16::MAX as u64 || directory_size == u32::MAX as u64 || directory_offset == u32::MAX as u64 { return Ok(None); }
    if count == 0 || count > 65536 || directory_size > DIRECTORY_LIMIT || directory_size < count * 46 { return Ok(None); }
    let Some(directory_start) = directory_end.checked_sub(directory_size) else { return Ok(None) };
    let Some(base) = directory_start.checked_sub(directory_offset) else { return Ok(None) };
    if locator_offset.is_some_and(|offset| offset.checked_add(base) != Some(directory_end)) { return Ok(None); }
    let directory = read_at(file, directory_start, directory_size as usize)?;
    let mut records = directory.as_slice();
    let mut first_local = u64::MAX;
    for _ in 0..count {
        ctl.check()?;
        if records.len() < 46 || !records.starts_with(b"PK\x01\x02") { return Ok(None); }
        let name_end = 46 + usize::from(zip_u16(records, 28));
        let extra_end = name_end + usize::from(zip_u16(records, 30));
        let record_end = extra_end + usize::from(zip_u16(records, 32));
        if record_end > records.len() { return Ok(None); }
        let mut packed_size = u64::from(zip_u32(records, 20));
        let mut offset = u64::from(zip_u32(records, 42));
        let mut disk = u32::from(zip_u16(records, 34));
        let mut extra = &records[name_end..extra_end];
        let mut zip64_seen = false;
        while !extra.is_empty() {
            if extra.len() < 4 { return Ok(None); }
            let size = usize::from(zip_u16(extra, 2));
            if 4 + size > extra.len() { return Ok(None); }
            if zip_u16(extra, 0) == 1 {
                if zip64_seen { return Ok(None); }
                zip64_seen = true;
                let mut value = &extra[4..4 + size];
                for field in [24usize, 20] {
                    if zip_u32(records, field) == u32::MAX {
                        if value.len() < 8 { return Ok(None); }
                        if field == 20 { packed_size = u64_at(value, 0); }
                        value = &value[8..];
                    }
                }
                if offset == u32::MAX as u64 {
                    if value.len() < 8 { return Ok(None); }
                    offset = u64_at(value, 0); value = &value[8..];
                }
                if disk == u16::MAX as u32 {
                    if value.len() < 4 { return Ok(None); }
                    disk = zip_u32(value, 0);
                }
            }
            extra = &extra[4 + size..];
        }
        if disk != 0 || offset == u32::MAX as u64 { return Ok(None); }
        let Some(local) = offset.checked_add(base) else { return Ok(None) };
        if local.checked_add(30).is_none_or(|end| end > directory_start) { return Ok(None); }
        first_local = first_local.min(local);
        // Every central record must point at a real local header in this file.
        let local_header = read_at(file, local, 30)?;
        if !local_header.starts_with(b"PK\x03\x04") || zip_u16(&local_header, 26) != zip_u16(records, 28)
            || zip_u16(&local_header, 8) != zip_u16(records, 10)
            || zip_u16(&local_header, 6) != zip_u16(records, 8) { return Ok(None); }
        let payload = local + 30 + u64::from(zip_u16(&local_header, 26)) + u64::from(zip_u16(&local_header, 28));
        if packed_size == u32::MAX as u64 || payload.checked_add(packed_size).is_none_or(|end| end > directory_start) { return Ok(None); }
        let name = read_at(file, local + 30, name_end - 46)?;
        if name != records[46..name_end] { return Ok(None); }
        records = &records[record_end..];
    }
    if !records.is_empty() { return Ok(None); }
    Ok(first_local.checked_add(1))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn fixture(prefix: usize, suffix: &[u8]) -> tempfile::NamedTempFile {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.start_file("-test.txt", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"verified data").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&vec![b'X'; prefix]).unwrap();
        file.write_all(&bytes).unwrap(); file.write_all(suffix).unwrap();
        file
    }
    #[test]
    fn zip_prefix_tail_is_bounded_and_ambiguous_eocd_is_not_accepted() {
        let mut input = fixture(8192, b"trailing bytes");
        assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), true).unwrap(), Some(8193));
        input.seek(SeekFrom::End(0)).unwrap();
        let mut fake = [0u8; 22]; fake[..4].copy_from_slice(b"PK\x05\x06");
        input.write_all(&fake).unwrap();
        assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), true).unwrap(), None);
    }
    #[test]
    fn long_tail_uses_bounded_progressive_windows_and_honors_cancel() {
        let mut input = fixture(8192, &vec![b'T'; 3 * 1024 * 1024]);
        assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), false).unwrap(), None);
        assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), true).unwrap(), Some(8193));
        let cancelled = JobControl::new(); cancelled.cancel();
        assert!(zip_search_limit(input.as_file_mut(), &cancelled, true).unwrap_err().contains("cancelled"));
    }

    #[test]
    fn progressive_search_keeps_end_records_across_window_boundaries() {
        for suffix in [1024 * 1024 - 10, 1024 * 1024 - 21, 2 * 1024 * 1024 - 32] {
            let mut input = fixture(8192, &vec![b'T'; suffix]);
            assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), true).unwrap(), Some(8193));
        }
    }

    #[test]
    fn central_disk_or_payload_crossing_directory_cannot_authorize_search() {
        for disk in [true, false] {
            let mut input = fixture(8192, b"tail");
            let mut data = fs::read(input.path()).unwrap();
            let central = data.windows(4).position(|p| p == b"PK\x01\x02").unwrap();
            if disk { data[central + 34] = 1; }
            else { data[central + 20..central + 24].copy_from_slice(&0xfffffffeu32.to_le_bytes()); }
            input.seek(SeekFrom::Start(0)).unwrap(); input.write_all(&data).unwrap();
            assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), true).unwrap(), None);
        }
    }
    #[test]
    fn rar_versions_choose_their_own_official_handler() {
        let mut rar4 = b"Rar!\x1a\x07\x00".to_vec();
        let header = [0x73, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0];
        rar4.extend_from_slice(&(crc32fast::hash(&header) as u16).to_le_bytes());
        rar4.extend_from_slice(&header);
        let mut rar5 = b"Rar!\x1a\x07\x01\x00".to_vec();
        let header = [3, 1, 0, 0];
        rar5.extend_from_slice(&crc32fast::hash(&header).to_le_bytes());
        rar5.extend_from_slice(&header);
        for (bytes, handler) in [(rar4, "rar"), (rar5, "rar5")] {
            let mut file = tempfile::NamedTempFile::new().unwrap();
            file.write_all(&bytes).unwrap();
            assert_eq!(signature(file.as_file_mut()).unwrap(), Some(handler));
        }
    }

    #[test]
    fn seven_signature_requires_crc_and_header_within_file() {
        let mut bytes = [0u8; 32]; bytes[..6].copy_from_slice(b"7z\xbc\xaf\x27\x1c");
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&bytes).unwrap();
        assert_eq!(signature(file.as_file_mut()).unwrap(), None);
        bytes[8..12].copy_from_slice(&crc32fast::hash(&[0u8;20]).to_le_bytes());
        file.seek(SeekFrom::Start(0)).unwrap(); file.write_all(&bytes).unwrap();
        assert_eq!(signature(file.as_file_mut()).unwrap(), Some("7z"));
        bytes[20] = 1;
        let crc = crc32fast::hash(&bytes[12..32]); bytes[8..12].copy_from_slice(&crc.to_le_bytes());
        file.seek(SeekFrom::Start(0)).unwrap(); file.write_all(&bytes).unwrap();
        assert_eq!(signature(file.as_file_mut()).unwrap(), None);
    }
}
