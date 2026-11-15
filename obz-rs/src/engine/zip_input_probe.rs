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

#[derive(Default)]
struct DirectoryWindow {
    start: u64,
    bytes: Vec<u8>,
}
impl DirectoryWindow {
    fn range<'a>(&'a mut self, file: &mut File, offset: u64, size: usize, end: u64)
        -> Result<Option<&'a [u8]>, String>
    {
        let Some(stop) = offset.checked_add(size as u64).filter(|n| *n <= end) else {
            return Ok(None);
        };
        let cached_end = self.start.checked_add(self.bytes.len() as u64);
        if offset < self.start || cached_end.is_none_or(|cached| stop > cached) {
            // A record is at most 46 + 3 * u16::MAX bytes. Cache complete
            // records across 128 KiB boundaries without allocating the CD or
            // an array sized by the untrusted record count.
            let amount = (end - offset).min((128 * 1024usize).max(size) as u64) as usize;
            self.bytes = read_at(file, offset, amount)?;
            self.start = offset;
        }
        let begin = (offset - self.start) as usize;
        Ok(Some(&self.bytes[begin..begin + size]))
    }
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
// Knowing the family is weaker than proving a complete standalone archive.
// A volume, encrypted RAR header or damaged start header must retain the old
// handler/volume policy, but none needs a full-file search for an unrelated ZIP.
pub(super) fn known_native_family(bytes: &[u8]) -> bool {
    bytes.starts_with(b"7z\xbc\xaf\x27\x1c")
        || bytes.starts_with(b"Rar!\x1a\x07\x00")
        || bytes.starts_with(b"Rar!\x1a\x07\x01\x00")
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
// ZIP search. A false/ambiguous EOCD, multi-disk record or unsupported ZIP64
// layout falls back to the captured-volume path.
pub(super) fn zip_search_limit(file: &mut File, ctl: &JobControl, deep: bool) -> Result<Option<u64>, String> {
    let result = probe(file, ctl, deep, false).map(|value| value.map(|layout| layout.search_limit));
    file.rewind().map_err(|e| e.to_string())?;
    result
}
pub(super) struct SelectedLayout {
    pub start: u64,
    pub end: u64,
    search_limit: u64,
}
// Stronger than format identification: every payload is contiguous, with the
// exact same conservative fields/extras admitted by the ordinary ZIP reader.
pub(super) fn selected_layout(file: &mut File, ctl: &JobControl) -> Result<Option<SelectedLayout>, String> {
    let result = probe(file, ctl, true, true);
    file.rewind().map_err(|e| e.to_string())?;
    result
}
fn probe(file: &mut File, ctl: &JobControl, deep: bool, strict: bool) -> Result<Option<SelectedLayout>, String> {
    const TAIL_LIMIT: u64 = 1024 * 1024;
    const SEARCH_BUDGET: u64 = 16 * 1024 * 1024;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    // Search from the end in fixed windows, not by growing an allocation or
    // copying the archive. The 21-byte overlap includes boundary EOCD headers.
    // Only archive-like extensions authorize looking beyond the first window.
    // This is identification, not an exhaustive recovery scan: malformed or
    // unrelated multi-gigabyte files must reach their normal handler promptly.
    // Count overlap reads too, so the actual search I/O has a fixed bound.
    let budget = length.min(if deep { SEARCH_BUDGET } else { TAIL_LIMIT });
    let mut searched = 0u64;
    let mut window_end = length;
    let (tail_start, tail) = loop {
        ctl.check()?;
        let remaining = budget - searched;
        if remaining < 22 {
            ctl.set("listing", None, "Checking archive with its format handler");
            return Ok(None);
        }
        let start = window_end.saturating_sub(TAIL_LIMIT.min(remaining));
        let window = read_at(file, start, (window_end - start) as usize)?;
        searched += window_end - start;
        ctl.set("listing", Some((searched * 100 / budget.max(1)).min(99) as u8),
            "Identifying archive directory");
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
    if count == 0 || count > directory_size / 46 { return Ok(None); }
    let Some(directory_start) = directory_end.checked_sub(directory_size) else { return Ok(None) };
    let Some(base) = directory_start.checked_sub(directory_offset) else { return Ok(None) };
    if locator_offset.is_some_and(|offset| offset.checked_add(base) != Some(directory_end)) { return Ok(None); }
    let mut directory = DirectoryWindow::default();
    let mut position = directory_start;
    let mut first_local = u64::MAX;
    let mut ranges = Vec::new();
    for index in 0..count {
        ctl.check()?;
        if index % 256 == 0 {
            ctl.set("listing", Some(((u128::from(index) * 100 / u128::from(count)) as u8).min(99)),
                "Reading archive directory");
        }
        let Some(header) = directory.range(file, position, 46, directory_end)? else { return Ok(None) };
        if !header.starts_with(b"PK\x01\x02") { return Ok(None); }
        let name_end = 46 + usize::from(zip_u16(header, 28));
        let extra_end = name_end + usize::from(zip_u16(header, 30));
        let record_end = extra_end + usize::from(zip_u16(header, 32));
        let Some(records) = directory.range(file, position, record_end, directory_end)? else { return Ok(None) };
        if strict && (zip_u16(records, 8) & !0x0806 != 0
            || !matches!(zip_u16(records, 10), 0 | 8)
            || zip_u32(records, 20) == u32::MAX || zip_u32(records, 24) == u32::MAX
            || !ordinary_zip_extra(&records[name_end..extra_end])) { return Ok(None); }
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
        let local_name_size = 30 + name_end - 46;
        if local.checked_add(local_name_size as u64).is_none_or(|end| end > directory_start) { return Ok(None); }
        first_local = first_local.min(local);
        // Every central record must point at a real local header in this file.
        let local_header = read_at(file, local, local_name_size)?;
        if !local_header.starts_with(b"PK\x03\x04") || zip_u16(&local_header, 26) != zip_u16(records, 28)
            || zip_u16(&local_header, 8) != zip_u16(records, 10)
            || zip_u16(&local_header, 6) != zip_u16(records, 8) { return Ok(None); }
        let Some(payload) = local.checked_add(30)
            .and_then(|n| n.checked_add(u64::from(zip_u16(&local_header, 26))))
            .and_then(|n| n.checked_add(u64::from(zip_u16(&local_header, 28)))) else { return Ok(None) };
        if packed_size == u32::MAX as u64 || payload.checked_add(packed_size).is_none_or(|end| end > directory_start) { return Ok(None); }
        if local_header[30..] != records[46..name_end] { return Ok(None); }
        if strict {
            if records[6..28] != local_header[4..26] { return Ok(None); }
            let extra = read_at(file, local + local_name_size as u64, usize::from(zip_u16(&local_header, 28)))?;
            if !ordinary_zip_extra(&extra) { return Ok(None); }
            ranges.push((local, payload + packed_size));
        }
        let Some(next) = position.checked_add(record_end as u64) else { return Ok(None) };
        position = next;
    }
    if position != directory_end { return Ok(None); }
    if strict {
        ranges.sort_unstable();
        let mut previous = base;
        for (start, end) in ranges {
            ctl.check()?;
            if start != previous { return Ok(None); }
            previous = end;
        }
        if previous != directory_start { return Ok(None); }
    }
    Ok(Some(SelectedLayout { start: base,
        end: tail_start + at as u64 + 22 + u64::from(zip_u16(end, 20)),
        search_limit: first_local.checked_add(1).ok_or("ZIP search offset overflow")? }))
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
    fn identification_budget_defers_longer_tail_to_handler_without_completing_job() {
        let mut input = fixture(8192, &vec![b'T'; 17 * 1024 * 1024]);
        let ctl = JobControl::new();
        assert_eq!(zip_search_limit(input.as_file_mut(), &ctl, true).unwrap(), None);
        assert_eq!(ctl.snapshot().phase, "listing");
        assert_eq!(ctl.snapshot().percent, None);
        assert_eq!(ctl.snapshot().message, "Checking archive with its format handler");
    }

    #[test]
    fn family_magic_does_not_imply_valid_standalone_or_hide_tail_zip() {
        for magic in [&b"7z\xbc\xaf\x27\x1c"[..], &b"Rar!\x1a\x07\x00"[..], &b"Rar!\x1a\x07\x01\x00"[..]] {
            assert!(known_native_family(magic));
            let mut input = fixture(8192, b"tail");
            input.rewind().unwrap();
            input.write_all(magic).unwrap();
            assert_eq!(signature(input.as_file_mut()).unwrap(), None);
            // A misleading prefix only suppresses deep search, not the bounded
            // tail-based proof that can still identify an embedded ZIP.
            assert_eq!(zip_search_limit(input.as_file_mut(), &JobControl::new(), false).unwrap(), Some(8193));
        }
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
    fn directory_window_crosses_cache_boundary_and_refuses_out_of_range_records() {
        let mut input = tempfile::NamedTempFile::new().unwrap();
        let data: Vec<u8> = (0..400_000).map(|i| (i % 251) as u8).collect();
        input.write_all(&data).unwrap();
        let mut window = DirectoryWindow::default();
        assert_eq!(window.range(input.as_file_mut(), 0, 46, data.len() as u64).unwrap().unwrap(), &data[..46]);
        assert_eq!(window.bytes.len(), 128 * 1024);
        let offset = 128 * 1024 - 17;
        let size = 46 + 3 * usize::from(u16::MAX);
        assert_eq!(window.range(input.as_file_mut(), offset as u64, size, data.len() as u64).unwrap().unwrap(), &data[offset..offset + size]);
        assert_eq!(window.bytes.len(), size);
        assert!(window.range(input.as_file_mut(), data.len() as u64 - 1, 46, data.len() as u64).unwrap().is_none());
        assert!(window.range(input.as_file_mut(), u64::MAX - 1, 46, u64::MAX).unwrap().is_none());
    }

    #[test]
    fn streamed_directory_rejects_duplicate_zip64_fields_and_inconsistent_counts() {
        for duplicate in [true, false] {
            let mut input = fixture(8192, b"tail");
            let mut data = fs::read(input.path()).unwrap();
            let central = data.windows(4).position(|p| p == b"PK\x01\x02").unwrap();
            let end = data.windows(4).position(|p| p == b"PK\x05\x06").unwrap();
            if duplicate {
                let name_end = central + 46 + usize::from(zip_u16(&data, central + 28));
                let size = zip_u32(&data, end + 12) + 8;
                let extra = zip_u16(&data, central + 30) + 8;
                data[central + 30..central + 32].copy_from_slice(&extra.to_le_bytes());
                data[end + 12..end + 16].copy_from_slice(&size.to_le_bytes());
                data.splice(name_end..name_end, [1, 0, 0, 0, 1, 0, 0, 0]);
            } else {
                // Counts agree with each other but not with the actual CD.
                data[end + 8..end + 12].copy_from_slice(&[2, 0, 2, 0]);
            }
            input.rewind().unwrap();input.write_all(&data).unwrap();
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
