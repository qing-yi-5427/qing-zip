//! Narrow qualification and cancelable decoding for verified memory staging.
//! Unknown layouts retain the general external engine; no files are published here.
use super::*;
use std::io::Read;

const LIMIT: usize = 64 * 1024 * 1024;
const HEADER_LIMIT: usize = 8 * 1024 * 1024;
const ITEMS: usize = 16_384;

struct Streams {
    start: usize,
    size: usize,
    packed_crc: Option<u32>,
    folder: Folder,
    subs: Vec<SubStream>,
}

pub(crate) struct MemoryArchive {
    pub archive: Archive,
    packed_crc: Option<u32>,
}

fn expect(r: &mut Reader, byte: u8) -> Result<(), String> {
    if r.u8()? != byte {
        return Err("unsupported memory header layout".into());
    }
    Ok(())
}

fn strict_crcs(r: &mut Reader, count: usize) -> Result<Vec<Option<u32>>, String> {
    let flag = r.u8()?;
    if flag > 1 {
        return Err("invalid CRC defined flag".into());
    }
    let defined = if flag == 1 {
        vec![true; count]
    } else {
        read_bitmap(r, count)?
    };
    defined
        .into_iter()
        .map(|yes| if yes { r.u32().map(Some) } else { Ok(None) })
        .collect()
}

// One packed stream and one simple coder, in the standard node order only.
fn streams(r: &mut Reader) -> Result<Streams, String> {
    expect(r, K_PACK_INFO)?;
    let start = 32usize
        .checked_add(usize_value(r.varint()?)?)
        .ok_or("pack offset overflow")?;
    if count(r, ITEMS)? != 1 {
        return Err("memory stage requires one pack stream".into());
    }
    expect(r, K_SIZE)?;
    let size = usize_value(r.varint()?)?;
    let mut tag = r.u8()?;
    let packed_crc = if tag == K_CRC {
        let crc = strict_crcs(r, 1)?[0];
        tag = r.u8()?;
        crc
    } else {
        None
    };
    if tag != K_END {
        return Err("unsupported pack metadata".into());
    }
    expect(r, K_UNPACK_INFO)?;
    expect(r, K_FOLDER)?;
    if count(r, ITEMS)? != 1 || r.u8()? != 0 || count(r, MAX_CODERS)? != 1 {
        return Err("memory stage requires one local coder".into());
    }
    let flags = r.u8()?;
    if flags & !0x2f != 0 || flags & 15 == 0 {
        return Err("unsupported coder flags".into());
    }
    let codec = r.read((flags & 15) as usize)?.to_vec();
    let props = if flags & 0x20 != 0 {
        let n = count(r, 5)?;
        r.read(n)?.to_vec()
    } else {
        vec![]
    };
    expect(r, K_CODERS_UNPACK_SIZE)?;
    let output = r.varint()?;
    let mut tag = r.u8()?;
    let unpack_crc = if tag == K_CRC {
        let crc = strict_crcs(r, 1)?[0];
        tag = r.u8()?;
        crc
    } else {
        None
    };
    if tag != K_END {
        return Err("unsupported unpack metadata".into());
    }
    let folder = Folder {
        coders: vec![Coder {
            codec,
            num_in: 1,
            num_out: 1,
            props,
        }],
        bind_pairs: vec![],
        packed_indices: vec![0],
        unpack_sizes: vec![output],
        unpack_crc,
    };
    let mut subs = vec![SubStream {
        folder_index: 0,
        size: output,
        crc: unpack_crc,
    }];
    let tag = r.u8()?;
    if tag == K_SUB_STREAMS {
        let mut n = 1usize;
        let mut sizes = None;
        let mut crcs = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let tag = r.u8()?;
            if tag == K_END {
                break;
            }
            if !seen.insert(tag) {
                return Err("duplicate substream property".into());
            }
            match tag {
                K_NUM_UNPACK_STREAM => {
                    if sizes.is_some() || crcs.is_some() {
                        return Err("substream order".into());
                    }
                    n = count(r, ITEMS)?;
                    if n == 0 {
                        return Err("empty folder unsupported".into());
                    }
                }
                K_SIZE => {
                    if crcs.is_some() {
                        return Err("substream order".into());
                    }
                    let mut parts = Vec::with_capacity(n);
                    let mut sum = 0u64;
                    for _ in 1..n {
                        let v = r.varint()?;
                        sum = sum.checked_add(v).ok_or("substream size overflow")?;
                        parts.push(v);
                    }
                    parts.push(output.checked_sub(sum).ok_or("substream size overflow")?);
                    sizes = Some(parts);
                }
                K_CRC => {
                    crcs = Some(strict_crcs(
                        r,
                        if n == 1 && unpack_crc.is_some() { 0 } else { n },
                    )?);
                }
                _ => return Err("unsupported substream property".into()),
            }
        }
        let sizes = match sizes {
            Some(v) => v,
            None if n == 1 => vec![output],
            None => return Err("missing substream sizes".into()),
        };
        let crcs = crcs.unwrap_or_else(|| vec![None; n]);
        subs = sizes
            .into_iter()
            .enumerate()
            .map(|(i, size)| SubStream {
                folder_index: 0,
                size,
                crc: if n == 1 && unpack_crc.is_some() {
                    unpack_crc
                } else {
                    crcs.get(i).copied().flatten()
                },
            })
            .collect();
        expect(r, K_END)?;
    } else if tag != K_END {
        return Err("unsupported streams tail".into());
    }
    Ok(Streams {
        start,
        size,
        packed_crc,
        folder,
        subs,
    })
}

fn files(r: &mut Reader, num: usize) -> Result<FileInfoProps, String> {
    let mut empty_stream = vec![false; num];
    let mut empty_file = Vec::new();
    let mut names = None;
    let mut attrs = vec![0; num];
    let mut seen = std::collections::HashSet::new();
    loop {
        let tag = r.u8()?;
        if tag == K_END {
            break;
        }
        if !seen.insert(tag) && tag != 0x19 {
            return Err("duplicate file property".into());
        }
        let size = usize_value(r.varint()?)?;
        let mut p = Reader::new(r.read(size)?);
        match tag {
            K_EMPTY_STREAM => {
                if seen.contains(&K_EMPTY_FILE) {
                    return Err("empty property order".into());
                }
                empty_stream = read_bitmap(&mut p, num)?;
            }
            K_EMPTY_FILE => {
                if !seen.contains(&K_EMPTY_STREAM) {
                    return Err("empty property order".into());
                }
                empty_file = read_bitmap(&mut p, empty_stream.iter().filter(|b| **b).count())?;
            }
            K_NAME => {
                if p.u8()? != 0 || p.bytes_left() % 2 != 0 {
                    return Err("unsupported names".into());
                }
                let units = p
                    .read(p.bytes_left())?
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>();
                let text = String::from_utf16(&units).map_err(|_| "invalid UTF16 names")?;
                let mut fields = text.split('\0').collect::<Vec<_>>();
                if fields.pop() != Some("")
                    || fields.len() != num
                    || fields.iter().any(|n| n.is_empty())
                {
                    return Err("ambiguous names".into());
                }
                names = Some(fields.into_iter().map(str::to_owned).collect());
            }
            K_WIN_ATTRIBUTES | 0x12 | 0x13 | K_MTIME => {
                let defined = match p.u8()? {
                    0 => read_bitmap(&mut p, num)?,
                    1 => vec![true; num],
                    _ => return Err("invalid defined flag".into()),
                };
                if p.u8()? != 0 {
                    return Err("external property unsupported".into());
                }
                for i in 0..num {
                    if defined[i] {
                        if tag == K_WIN_ATTRIBUTES {
                            attrs[i] = p.u32()?;
                        } else {
                            p.u64()?;
                        }
                    }
                }
            }
            0x19 => {
                if p.read(p.bytes_left())?.iter().any(|b| *b != 0) {
                    return Err("nonzero dummy property".into());
                }
            }
            _ => return Err(format!("unknown file property {tag:x}")),
        }
        if p.bytes_left() != 0 {
            return Err("trailing file property".into());
        }
    }
    Ok(FileInfoProps {
        names: names.ok_or("missing names")?,
        empty_stream,
        empty_file,
        attrs,
    })
}

fn simple(folder: &Folder, limit: usize, header: bool) -> bool {
    if folder.coders.len() != 1
        || folder.unpack_sizes.len() != 1
        || folder.unpack_size() > limit as u64
    {
        return false;
    }
    let c = &folder.coders[0];
    match c.codec.as_slice() {
        CODEC_COPY => c.props.is_empty(),
        CODEC_LZMA2 => c.props.len() == 1 && c.props[0] <= if header { 24 } else { 28 }, // dictionary <=64MiB
        CODEC_LZMA if header => {
            c.props.len() == 5
                && c.props[0] < 225
                && u32::from_le_bytes(c.props[1..5].try_into().unwrap()) <= 16 * 1024 * 1024
        }
        _ => false,
    }
}

fn decode(
    folder: &Folder,
    packed: &[u8],
    limit: usize,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    check()?;
    let expected = usize_value(folder.unpack_size())?;
    if expected > limit {
        return Err("memory folder budget exceeded".into());
    }
    let c = &folder.coders[0];
    let mut output = Vec::new();
    output
        .try_reserve_exact(expected)
        .map_err(|e| e.to_string())?;
    if output.capacity() > limit {
        return Err("memory folder allocation limit".into());
    }
    output.resize(expected, 0);
    let fill = |reader: &mut dyn Read, output: &mut [u8]| -> Result<(), String> {
        for chunk in output.chunks_mut(65536) {
            check()?;
            reader
                .read_exact(chunk)
                .map_err(|e| format!("7z decode: {e}"))?;
        }
        check()?;
        if reader
            .read(&mut [0u8; 1])
            .map_err(|e| format!("7z decode: {e}"))?
            != 0
        {
            return Err("7z decoded size mismatch".into());
        }
        Ok(())
    };
    match c.codec.as_slice() {
        CODEC_COPY => fill(&mut Cursor::new(packed), &mut output)?,
        CODEC_LZMA2 => {
            strict_lzma2(packed, expected, check)?;
            let p = c.props[0];
            let dict = (2u32 | (p as u32 & 1)) << (p / 2 + 11);
            let mut reader =
                lzma_rust2::Lzma2Reader::new_mem_limit(Cursor::new(packed), dict, 65 * 1024, None)
                    .map_err(|e| e.to_string())?;
            fill(&mut reader, &mut output)?;
        }
        CODEC_LZMA => {
            let mut reader = lzma_rust2::LzmaReader::new_with_props(
                Cursor::new(packed),
                expected as u64,
                c.props[0],
                u32::from_le_bytes(c.props[1..5].try_into().unwrap()),
                None,
            )
            .map_err(|e| e.to_string())?;
            fill(&mut reader, &mut output)?;
            let (input, unused) = reader.into_parts();
            if input.position() as usize != packed.len() || !unused.is_empty() {
                return Err("7z packed header trailing data".into());
            }
        }
        _ => return Err("unqualified coder".into()),
    }
    if folder
        .unpack_crc
        .is_some_and(|want| crc32fast::hash(&output) != want)
    {
        return Err("7z folder CRC mismatch".into());
    }
    Ok(output)
}

fn packed<'a>(data: &'a [u8], s: &Streams) -> Result<&'a [u8], String> {
    let end = s.start.checked_add(s.size).ok_or("pack range overflow")?;
    let bytes = data.get(s.start..end).ok_or("pack range exceeds input")?;
    if s.packed_crc
        .is_some_and(|want| crc32fast::hash(bytes) != want)
    {
        return Err("7z pack CRC mismatch".into());
    }
    Ok(bytes)
}

pub(crate) fn parse_memory_archive(
    data: &[u8],
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Option<MemoryArchive>, String> {
    check()?;
    if data.len() < 32 || data.len() > LIMIT || &data[..6] != MAGIC || data[6] != 0 {
        return Ok(None);
    }
    let mut r = Reader::new(data);
    r.p = 8;
    let start_crc = r.u32()?;
    let off = usize_value(r.u64()?)?;
    let len = usize_value(r.u64()?)?;
    let header_crc = r.u32()?;
    if crc32fast::hash(&data[12..32]) != start_crc {
        return Err("7z start header CRC mismatch".into());
    }
    let off = 32usize.checked_add(off).ok_or("7z offset overflow")?;
    if len > HEADER_LIMIT || off.checked_add(len) != Some(data.len()) {
        return Ok(None);
    }
    let encoded = &data[off..];
    if crc32fast::hash(encoded) != header_crc {
        return Err("7z next header CRC mismatch".into());
    }
    let mut encoded_start = off;
    let decoded;
    let header = if encoded.first() == Some(&K_ENCODED_HEADER) {
        let mut r = Reader::new(encoded);
        r.p = 1;
        let s = match streams(&mut r) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        if r.bytes_left() != 0
            || !simple(&s.folder, HEADER_LIMIT, true)
            || s.subs.len() != 1
            || s.start.checked_add(s.size) != Some(off)
        {
            return Ok(None);
        }
        encoded_start = s.start;
        decoded = decode(&s.folder, packed(data, &s)?, HEADER_LIMIT, check)?;
        if s.subs[0].size != decoded.len() as u64
            || s.subs[0]
                .crc
                .is_some_and(|want| crc32fast::hash(&decoded) != want)
        {
            return Err("7z encoded header substream CRC mismatch".into());
        }
        &decoded
    } else {
        encoded
    };
    let mut r = Reader::new(header);
    if r.u8()? != K_HEADER || r.u8()? != K_MAIN_STREAMS {
        return Ok(None);
    }
    let s = match streams(&mut r) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    if !simple(&s.folder, LIMIT, false)
        || s.start != 32
        || s.start.checked_add(s.size) != Some(encoded_start)
    {
        return Ok(None);
    }
    if r.u8()? != K_FILES_INFO {
        return Ok(None);
    }
    let n = match count(&mut r, ITEMS) {
        Ok(n) => n,
        Err(_) => return Ok(None),
    };
    let props = match files(&mut r, n) {
        Ok(p) => p,
        Err(_) => return Ok(None),
    };
    if r.u8()? != K_END || r.bytes_left() != 0 {
        return Ok(None);
    }
    let entries = match build_entries(Some(props), &s.subs, n) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    if entries.iter().filter(|e| e.folder_index.is_some()).count() != s.subs.len() {
        return Ok(None);
    }
    let mut end = 0u64;
    for e in entries.iter().filter(|e| e.folder_index.is_some()) {
        if e.folder_index != Some(0) || e.offset_in_stream != end {
            return Ok(None);
        }
        end = end.checked_add(e.size).ok_or("entry range overflow")?;
    }
    if end != s.folder.unpack_size() {
        return Ok(None);
    }
    packed(data, &s)?;
    check()?;
    Ok(Some(MemoryArchive {
        packed_crc: s.packed_crc,
        archive: Archive {
            pack_offset: s.start,
            pack_sizes: vec![s.size],
            folders: vec![s.folder],
            substreams: s.subs,
            entries,
            header_offset: off,
            header_size: len,
            encoded_header: encoded.first() == Some(&K_ENCODED_HEADER),
        },
    }))
}

impl MemoryArchive {
    pub(crate) fn decode(
        &self,
        data: &[u8],
        check: &dyn Fn() -> Result<(), String>,
    ) -> Result<Vec<u8>, String> {
        let start = self.archive.pack_offset;
        let end = start
            .checked_add(self.archive.pack_sizes[0])
            .ok_or("pack range overflow")?;
        let bytes = data.get(start..end).ok_or("pack range outside input")?;
        if self
            .packed_crc
            .is_some_and(|want| crc32fast::hash(bytes) != want)
        {
            return Err("7z pack CRC mismatch".into());
        }
        decode(&self.archive.folders[0], bytes, LIMIT, check)
    }
}

fn strict_lzma2(
    blob: &[u8],
    expected: usize,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<(), String> {
    let mut r = Reader::new(blob);
    let mut total = 0usize;
    loop {
        check()?;
        let control = r.u8()?;
        if control == 0 {
            break;
        }
        let (unpacked, packed) = if control == 1 || control == 2 {
            let b = r.read(2)?;
            let n = u16::from_be_bytes([b[0], b[1]]) as usize + 1;
            (n, n)
        } else if control >= 0x80 {
            let b = r.read(4)?;
            let unpacked =
                (((control & 0x1f) as usize) << 16 | ((b[0] as usize) << 8) | b[1] as usize) + 1;
            let packed = u16::from_be_bytes([b[2], b[3]]) as usize + 1;
            if control >= 0xc0 {
                r.u8()?;
            }
            (unpacked, packed)
        } else {
            return Err("invalid LZMA2 control".into());
        };
        total = total
            .checked_add(unpacked)
            .filter(|&n| n <= expected)
            .ok_or("LZMA2 declared output exceeds limit")?;
        r.read(packed)?;
    }
    if total != expected || r.bytes_left() != 0 {
        return Err("LZMA2 output size mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn uint(out: &mut Vec<u8>, n: u64) {
        out.push(0xff);
        out.extend(n.to_le_bytes());
    }
    fn stream(
        start: usize,
        payload: &[u8],
        count: usize,
        folder_crc: u32,
        pack_crc: u32,
    ) -> Vec<u8> {
        let mut h = vec![K_PACK_INFO];
        uint(&mut h, start as u64);
        uint(&mut h, 1);
        h.push(K_SIZE);
        uint(&mut h, payload.len() as u64);
        h.extend([K_CRC, 1]);
        h.extend(pack_crc.to_le_bytes());
        h.extend([K_END, K_UNPACK_INFO, K_FOLDER]);
        uint(&mut h, 1);
        h.push(0);
        uint(&mut h, 1);
        h.extend([1, 0, K_CODERS_UNPACK_SIZE]);
        uint(&mut h, payload.len() as u64);
        h.extend([K_CRC, 1]);
        h.extend(folder_crc.to_le_bytes());
        h.extend([K_END, K_SUB_STREAMS, K_NUM_UNPACK_STREAM]);
        uint(&mut h, count as u64);
        if count > 1 {
            h.push(K_SIZE);
            for _ in 1..count {
                uint(&mut h, (payload.len() / count) as u64);
            }
            h.extend([K_CRC, 1]);
            for part in payload.chunks(payload.len() / count) {
                h.extend(crc32fast::hash(part).to_le_bytes());
            }
        }
        h.extend([K_END, K_END]);
        h
    }
    fn wrap(packs: &[u8], header: &[u8]) -> Vec<u8> {
        let mut start = Vec::new();
        start.extend((packs.len() as u64).to_le_bytes());
        start.extend((header.len() as u64).to_le_bytes());
        start.extend(crc32fast::hash(header).to_le_bytes());
        let mut out = MAGIC.to_vec();
        out.extend([0, 4]);
        out.extend(crc32fast::hash(&start).to_le_bytes());
        out.extend(start);
        out.extend(packs);
        out.extend(header);
        out
    }
    pub(crate) fn fixture(encoded: bool, bad_pack: bool, bad_folder: bool) -> Vec<u8> {
        let payload = vec![42u8; 160 * 1024];
        let crc = crc32fast::hash(&payload);
        let mut h = vec![K_HEADER, K_MAIN_STREAMS];
        h.extend(stream(
            0,
            &payload,
            160,
            crc ^ u32::from(bad_folder),
            crc ^ u32::from(bad_pack),
        ));
        h.push(K_FILES_INFO);
        uint(&mut h, 160);
        let mut names = vec![0];
        for i in 0..160 {
            for u in format!("folder/file-{i}.txt\0").encode_utf16() {
                names.extend(u.to_le_bytes());
            }
        }
        h.push(K_NAME);
        uint(&mut h, names.len() as u64);
        h.extend(names);
        h.extend([K_END, K_END]);
        if encoded {
            let mut next = vec![K_ENCODED_HEADER];
            next.extend(stream(
                payload.len(),
                &h,
                1,
                crc32fast::hash(&h),
                crc32fast::hash(&h),
            ));
            let mut packs = payload;
            packs.extend(&h);
            wrap(&packs, &next)
        } else {
            wrap(&payload, &h)
        }
    }
    #[test]
    fn complete_copy_solid_and_encoded_headers_verify() {
        for encoded in [false, true] {
            let data = fixture(encoded, false, false);
            let a = parse_memory_archive(&data, &|| Ok(())).unwrap().unwrap();
            assert_eq!(a.archive.entries.len(), 160);
            assert_eq!(a.decode(&data, &|| Ok(())).unwrap(), vec![42; 160 * 1024]);
        }
    }
    #[test]
    fn pack_folder_and_encoded_header_crc_fail_closed() {
        let data = fixture(false, true, false);
        assert!(parse_memory_archive(&data, &|| Ok(()))
            .err()
            .unwrap()
            .contains("pack CRC"));
        let data = fixture(false, false, true);
        let a = parse_memory_archive(&data, &|| Ok(())).unwrap().unwrap();
        assert!(a
            .decode(&data, &|| Ok(()))
            .unwrap_err()
            .contains("folder CRC"));
        let good = fixture(true, false, false);
        let off = 32 + u64::from_le_bytes(good[12..20].try_into().unwrap()) as usize;
        let mut r = Reader::new(&good[off..]);
        r.p = 1;
        let st = streams(&mut r).unwrap();
        let mut next = vec![K_ENCODED_HEADER];
        next.extend(stream(
            st.start - 32,
            &good[st.start..off],
            1,
            st.folder.unpack_crc.unwrap() ^ 1,
            st.packed_crc.unwrap(),
        ));
        let bad = wrap(&good[32..off], &next);
        assert!(parse_memory_archive(&bad, &|| Ok(()))
            .err()
            .unwrap()
            .contains("folder CRC"));
    }
    #[test]
    fn cancel_during_stream_and_reject_lzma2_trailing_bytes() {
        let data = fixture(false, false, false);
        let a = parse_memory_archive(&data, &|| Ok(())).unwrap().unwrap();
        let calls = std::cell::Cell::new(0);
        let error = a
            .decode(&data, &|| {
                calls.set(calls.get() + 1);
                if calls.get() >= 3 {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error, "cancelled");
        assert!(strict_lzma2(&[1, 0, 0, 42, 0], 1, &|| Ok(())).is_ok());
        assert!(strict_lzma2(&[1, 0, 0, 42, 0, 99], 1, &|| Ok(())).is_err());
    }
    #[test]
    fn encoded_substream_crc_is_checked_without_folder_crc() {
        let good = fixture(false, false, false);
        let off = 32 + u64::from_le_bytes(good[12..20].try_into().unwrap()) as usize;
        let h = &good[off..];
        let mut stream = stream(off - 32, h, 1, crc32fast::hash(h), crc32fast::hash(h));
        // Remove the folder CRC only, then put an independently wrong digest in SubStreamsInfo.
        let marker = stream
            .windows(2)
            .enumerate()
            .filter(|(_, w)| *w == [K_CRC, 1])
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        assert_eq!(marker.len(), 2);
        stream.drain(marker[1]..marker[1] + 6);
        let pos = stream.len() - 2;
        stream.splice(pos..pos, [K_CRC, 1, 0, 0, 0, 0]);
        let mut next = vec![K_ENCODED_HEADER];
        next.extend(stream);
        let mut packs = good[32..off].to_vec();
        packs.extend(h);
        let bad = wrap(&packs, &next);
        assert!(parse_memory_archive(&bad, &|| Ok(()))
            .err()
            .unwrap()
            .contains("substream CRC"));
    }
    #[test]
    fn unknown_file_semantics_and_large_dictionary_fall_back() {
        let good = fixture(false, false, false);
        let off = 32 + u64::from_le_bytes(good[12..20].try_into().unwrap()) as usize;
        let mut header = good[off..].to_vec();
        let at = header.len() - 2;
        header.splice(at..at, [0x10, 1, 0]);
        assert!(
            parse_memory_archive(&wrap(&good[32..off], &header), &|| Ok(()))
                .unwrap()
                .is_none()
        );
        let folder = Folder {
            coders: vec![Coder {
                codec: CODEC_LZMA2.to_vec(),
                num_in: 1,
                num_out: 1,
                props: vec![29],
            }],
            bind_pairs: vec![],
            packed_indices: vec![0],
            unpack_sizes: vec![1],
            unpack_crc: None,
        };
        assert!(!simple(&folder, LIMIT, false));
        assert!(!simple(&folder, HEADER_LIMIT, true));
    }
    #[test]
    fn strict_names_duplicates_and_crc_flags() {
        for bytes in [
            vec![K_NAME, 4, 0, 65, 0, 0, K_END],
            vec![K_NAME, 5, 0, 0, 0xd8, 0, 0, K_END],
            vec![K_NAME, 5, 0, 65, 0, 0, 0, K_NAME, 5, 0, 65, 0, 0, 0, K_END],
        ] {
            assert!(files(&mut Reader::new(&bytes), 1).is_err());
        }
        assert!(strict_crcs(&mut Reader::new(&[2, 0, 0, 0, 0]), 1).is_err());
        assert!(files(&mut Reader::new(&[0x19, 1, 1, K_END]), 1).is_err());
    }
    #[test]
    #[ignore]
    fn local_corpus_qualification() {
        let data = std::fs::read(std::env::var("OA_MEMORY_CORPUS").unwrap()).unwrap();
        let archive = parse_memory_archive(&data, &|| Ok(()))
            .unwrap()
            .expect("corpus qualified");
        let bytes = archive.decode(&data, &|| Ok(())).unwrap();
        assert_eq!(bytes.len() as u64, archive.archive.folders[0].unpack_size());
    }
}
