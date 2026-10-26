//! 7z container reader: SignatureHeader / Header (plain or EncodedHeader)
//! parsing plus solid pack-stream decoding (COPY, LZMA, LZMA2, DEFLATE,
//! BCJ-x86). Faithful to the public 7z layout (LZMA SDK `7zFormat.txt`);
//! verified byte-identical against 7-Zip extraction.

use std::io::Cursor;

use crate::util::{crc32, BoundedOutput, Reader, MAX_NATIVE_OUTPUT};
const MAX_HEADER: usize = 32 * 1024 * 1024;
const MAX_ITEMS: usize = 100_000;
const MAX_CODERS: usize = 16;
fn usize_value(v: u64) -> Result<usize, String> {
    usize::try_from(v).map_err(|_| "native size overflow".into())
}
fn count(r: &mut Reader, limit: usize) -> Result<usize, String> {
    let n = usize_value(r.varint()?)?;
    if n > limit {
        return Err("native item count limit exceeded".into());
    }
    Ok(n)
}

pub const MAGIC: &[u8] = b"7z\xbc\xaf'\x1c";

const K_END: u8 = 0x00;
const K_HEADER: u8 = 0x01;
const K_ARCHIVE_PROPERTIES: u8 = 0x02;
const K_ADDITIONAL_STREAMS: u8 = 0x03;
const K_MAIN_STREAMS: u8 = 0x04;
const K_FILES_INFO: u8 = 0x05;
const K_PACK_INFO: u8 = 0x06;
const K_UNPACK_INFO: u8 = 0x07;
const K_SUB_STREAMS: u8 = 0x08;
const K_SIZE: u8 = 0x09;
const K_CRC: u8 = 0x0A;
const K_FOLDER: u8 = 0x0B;
const K_CODERS_UNPACK_SIZE: u8 = 0x0C;
const K_NUM_UNPACK_STREAM: u8 = 0x0D;
const K_EMPTY_STREAM: u8 = 0x0E;
const K_EMPTY_FILE: u8 = 0x0F;
const K_NAME: u8 = 0x11;
const K_MTIME: u8 = 0x14;
const K_WIN_ATTRIBUTES: u8 = 0x15;
const K_ENCODED_HEADER: u8 = 0x17;

const CODEC_COPY: &[u8] = b"\x00";
const CODEC_LZMA: &[u8] = b"\x03\x01\x01";
const CODEC_LZMA2: &[u8] = b"\x21";
const CODEC_DEFLATE: &[u8] = b"\x04\x01\x08";
const CODEC_BCJ_X86: &[u8] = b"\x03\x03\x01\x03";
const CODEC_7Z_AES: &[u8] = b"\x06\xf1\x07\x01";

#[derive(Clone)]
pub struct Coder {
    pub codec: Vec<u8>,
    pub num_in: usize,
    pub num_out: usize,
    pub props: Vec<u8>,
}

#[derive(Clone)]
pub struct Folder {
    pub coders: Vec<Coder>,
    pub bind_pairs: Vec<(usize, usize)>,
    pub packed_indices: Vec<usize>,
    pub unpack_sizes: Vec<u64>,
    pub unpack_crc: Option<u32>,
}

impl Folder {
    pub fn total_in(&self) -> usize {
        self.coders
            .iter()
            .fold(0usize, |n, c| n.saturating_add(c.num_in))
    }
    pub fn total_out(&self) -> usize {
        self.coders
            .iter()
            .fold(0usize, |n, c| n.saturating_add(c.num_out))
    }
    pub fn unpack_size(&self) -> u64 {
        let bound: Vec<usize> = self.bind_pairs.iter().map(|(_, b)| *b).collect();
        for i in 0..self.total_out() {
            if !bound.contains(&i) {
                return self.unpack_sizes.get(i).copied().unwrap_or(0);
            }
        }
        0
    }
}

#[derive(Clone, Copy)]
pub struct SubStream {
    pub folder_index: usize,
    pub size: u64,
    pub crc: Option<u32>,
}

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub crc: Option<u32>,
    pub offset_in_stream: u64,
    pub folder_index: Option<usize>,
    pub attrs: u32,
}

pub struct Archive {
    pub pack_offset: usize,
    pub pack_sizes: Vec<usize>,
    pub folders: Vec<Folder>,
    pub substreams: Vec<SubStream>,
    pub entries: Vec<Entry>,
    pub header_offset: usize,
    pub header_size: usize,
    pub encoded_header: bool,
}

impl Archive {
    pub fn read_folder(&self, data: &[u8], index: usize) -> Result<Vec<u8>, String> {
        let folder = self.folders.get(index).ok_or("invalid folder index")?;
        let start = self
            .pack_sizes
            .get(..index)
            .ok_or("missing pack sizes")?
            .iter()
            .try_fold(self.pack_offset, |a, &b| a.checked_add(b))
            .ok_or("pack offset overflow")?;
        let end = start
            .checked_add(*self.pack_sizes.get(index).ok_or("missing pack size")?)
            .ok_or("pack size overflow")?;
        let packed = data.get(start..end).ok_or("pack stream exceeds input")?;
        let out = decode_folder(folder, packed)?;
        if out.len() as u64 != folder.unpack_size() {
            return Err("folder output size mismatch".into());
        }
        if folder.unpack_crc.is_some_and(|want| crc32(&out) != want) {
            return Err("folder CRC mismatch".into());
        }
        Ok(out)
    }

    pub fn extract_entry(&self, data: &[u8], entry: &Entry) -> Result<Vec<u8>, String> {
        let fi = match entry.folder_index {
            Some(f) => f,
            None => return Ok(Vec::new()),
        };
        let blob = self.read_folder(data, fi)?;
        let start = usize_value(entry.offset_in_stream)?;
        let end = start
            .checked_add(usize_value(entry.size)?)
            .ok_or("entry range overflow")?;
        if end > blob.len() {
            return Err(format!("entry {} exceeds folder output", entry.name));
        }
        let chunk = &blob[start..end];
        if let Some(want) = entry.crc {
            let got = crc32(chunk);
            if got != want {
                return Err(format!(
                    "CRC mismatch for {}: {got:08x} != {want:08x}",
                    entry.name
                ));
            }
        }
        Ok(chunk.to_vec())
    }
}

fn read_crcs(r: &mut Reader, count: usize) -> Result<Vec<Option<u32>>, String> {
    if count > MAX_ITEMS {
        return Err("CRC count limit exceeded".into());
    }
    if r.u8()? != 0 {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(Some(r.u32()?));
        }
        return Ok(out);
    }
    let flags = r.read((count + 7) / 8)?.to_vec();
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        if flags[i / 8] & (0x80 >> (i % 8)) != 0 {
            out.push(Some(r.u32()?));
        } else {
            out.push(None);
        }
    }
    Ok(out)
}

fn read_folder_def(r: &mut Reader) -> Result<Folder, String> {
    let num_coders = count(r, MAX_CODERS)?;
    if num_coders == 0 || num_coders > r.bytes_left() {
        return Err("invalid coder count".into());
    }
    let mut coders = Vec::with_capacity(num_coders);
    let mut total_in = 0usize;
    let mut total_out = 0usize;
    for _ in 0..num_coders {
        let flags = r.u8()?;
        let id_size = (flags & 0x0F) as usize;
        let complex = flags & 0x10 != 0;
        let has_props = flags & 0x20 != 0;
        if flags & 0x80 != 0 {
            return Err("alternative coder methods not supported".into());
        }
        let codec = r.read(id_size)?.to_vec();
        let (num_in, num_out) = if complex {
            (count(r, MAX_CODERS)?, count(r, MAX_CODERS)?)
        } else {
            (1, 1)
        };
        if num_in != 1 || num_out != 1 {
            return Err("complex native coder unsupported".into());
        }
        let props = if has_props {
            let n = count(r, MAX_HEADER)?;
            r.read(n)?.to_vec()
        } else {
            Vec::new()
        };
        total_in += num_in;
        total_out += num_out;
        coders.push(Coder {
            codec,
            num_in,
            num_out,
            props,
        });
    }
    let mut bind_pairs = Vec::new();
    for _ in 0..total_out.saturating_sub(1) {
        bind_pairs.push((count(r, MAX_CODERS)?, count(r, MAX_CODERS)?));
    }
    for &(input, output) in &bind_pairs {
        if input >= total_in || output >= total_out {
            return Err("invalid folder binding".into());
        }
    }
    let n_packed = total_in
        .checked_sub(bind_pairs.len())
        .ok_or("invalid folder stream count")?;
    let mut packed_indices = Vec::new();
    if n_packed > 1 {
        for _ in 0..n_packed {
            let index = count(r, MAX_CODERS)?;
            if index >= total_in {
                return Err("invalid packed stream index".into());
            }
            packed_indices.push(index);
        }
    } else if n_packed == 1 {
        // A single packed stream always has index 0; 7-Zip writers disagree on
        // emitting the trivial index. Keep the byte only when it is that index.
        let save = r.p;
        let first = r.u8()?;
        if first == 0
            && r.bytes_left() > 0
            && matches!(r.d[r.p], K_CODERS_UNPACK_SIZE | K_CRC | K_END)
        {
            packed_indices.push(0);
        } else {
            r.p = save;
            packed_indices.push(0);
        }
    }
    Ok(Folder {
        coders,
        bind_pairs,
        packed_indices,
        unpack_sizes: Vec::new(),
        unpack_crc: None,
    })
}

fn read_unpack_info(r: &mut Reader) -> Result<Vec<Folder>, String> {
    if r.u8()? != K_FOLDER {
        return Err("UnPackInfo missing Folder".into());
    }
    let num = count(r, MAX_ITEMS)?;
    if num > r.bytes_left() {
        return Err("folder count exceeds header".into());
    }
    if r.u8()? != 0 {
        return Err("external folders not supported".into());
    }
    let mut folders = Vec::with_capacity(num);
    for _ in 0..num {
        folders.push(read_folder_def(r)?);
    }
    if r.u8()? != K_CODERS_UNPACK_SIZE {
        return Err("UnPackInfo missing unpack sizes".into());
    }
    for f in &mut folders {
        let n = f.total_out();
        f.unpack_sizes = (0..n).map(|_| r.varint()).collect::<Result<_, _>>()?;
        if f.unpack_sizes.iter().any(|&v| v > MAX_NATIVE_OUTPUT as u64) {
            return Err("native folder output exceeds 512 MiB limit".into());
        }
    }
    loop {
        match r.u8()? {
            K_END => return Ok(folders),
            K_CRC => {
                let crcs = read_crcs(r, num)?;
                for (f, c) in folders.iter_mut().zip(crcs) {
                    f.unpack_crc = c;
                }
            }
            other => return Err(format!("unexpected UnPackInfo node {other:#x}")),
        }
    }
}

fn read_streams_info(
    r: &mut Reader,
) -> Result<(usize, Vec<usize>, Vec<Folder>, Vec<SubStream>), String> {
    let mut pack_offset = 32usize;
    let mut pack_sizes = Vec::new();
    let mut folders = Vec::new();
    let mut substreams = Vec::new();
    loop {
        let nid = r.u8()?;
        if nid == K_END {
            break;
        }
        match nid {
            K_PACK_INFO => {
                pack_offset = 32usize
                    .checked_add(usize_value(r.varint()?)?)
                    .ok_or("pack offset overflow")?;
                let count = count(r, MAX_ITEMS)?;
                if count > r.bytes_left() {
                    return Err("pack count exceeds header".into());
                }
                if r.u8()? != K_SIZE {
                    return Err("PackInfo missing sizes".into());
                }
                pack_sizes = (0..count)
                    .map(|_| r.varint().and_then(usize_value))
                    .collect::<Result<_, _>>()?;
                let mut tail = r.u8()?;
                if tail == K_CRC {
                    let _ = read_crcs(r, count)?;
                    tail = r.u8()?;
                }
                if tail != K_END {
                    return Err("unexpected PackInfo tail".into());
                }
            }
            K_UNPACK_INFO => folders = read_unpack_info(r)?,
            K_SUB_STREAMS => substreams = read_substreams(r, &folders)?,
            other => return Err(format!("unexpected streams node {other:#x}")),
        }
    }
    if !folders.is_empty() && substreams.is_empty() {
        substreams = folders
            .iter()
            .enumerate()
            .map(|(i, f)| SubStream {
                folder_index: i,
                size: f.unpack_size(),
                crc: f.unpack_crc,
            })
            .collect();
    }
    Ok((pack_offset, pack_sizes, folders, substreams))
}

fn read_substreams(r: &mut Reader, folders: &[Folder]) -> Result<Vec<SubStream>, String> {
    let mut counts: Vec<usize> = folders.iter().map(|_| 1).collect();
    let mut sizes: Option<Vec<Vec<u64>>> = None;
    let mut digests: Option<Vec<Option<u32>>> = None;
    loop {
        match r.u8()? {
            K_END => break,
            K_NUM_UNPACK_STREAM => {
                counts = (0..folders.len())
                    .map(|_| r.varint().and_then(usize_value))
                    .collect::<Result<_, _>>()?;
                if counts
                    .iter()
                    .try_fold(0usize, |a, &b| a.checked_add(b))
                    .filter(|&n| n <= MAX_ITEMS)
                    .is_none()
                {
                    return Err("substream count limit exceeded".into());
                }
            }
            K_SIZE => {
                let mut rows = Vec::with_capacity(folders.len());
                for (f, n) in folders.iter().zip(&counts) {
                    if *n == 0 {
                        rows.push(Vec::new());
                        continue;
                    }
                    let mut row: Vec<u64> =
                        (0..n - 1).map(|_| r.varint()).collect::<Result<_, _>>()?;
                    let sum = row
                        .iter()
                        .try_fold(0u64, |a, &b| a.checked_add(b))
                        .ok_or("substream size overflow")?;
                    row.push(
                        f.unpack_size()
                            .checked_sub(sum)
                            .ok_or("substream exceeds folder")?,
                    );
                    rows.push(row);
                }
                sizes = Some(rows);
            }
            K_CRC => {
                // Digests cover only streams whose CRC is not derivable.
                let unknown: usize = folders
                    .iter()
                    .zip(&counts)
                    .map(|(f, n)| {
                        if *n != 1 || f.unpack_crc.is_none() {
                            *n
                        } else {
                            0
                        }
                    })
                    .sum();
                if r.u8()? != 0 {
                    let mut v = Vec::with_capacity(unknown);
                    for _ in 0..unknown {
                        v.push(Some(r.u32()?));
                    }
                    digests = Some(v);
                } else {
                    let flags = r.read((unknown + 7) / 8)?.to_vec();
                    let mut v = Vec::with_capacity(unknown);
                    for i in 0..unknown {
                        if flags[i / 8] & (0x80 >> (i % 8)) != 0 {
                            v.push(Some(r.u32()?));
                        } else {
                            v.push(None);
                        }
                    }
                    digests = Some(v);
                }
            }
            other => return Err(format!("unexpected SubStreams node {other:#x}")),
        }
    }
    let default_sizes: Vec<Vec<u64>> = folders.iter().map(|f| vec![f.unpack_size()]).collect();
    let sizes = sizes.unwrap_or(default_sizes);
    let digests = digests.unwrap_or_default();
    let mut out = Vec::new();
    let mut k = 0usize;
    for (fi, (f, n)) in folders.iter().zip(&counts).enumerate() {
        for j in 0..*n {
            let crc = if *n == 1 && f.unpack_crc.is_some() {
                f.unpack_crc
            } else {
                let c = digests.get(k).copied().flatten();
                k += 1;
                c
            };
            out.push(SubStream {
                folder_index: fi,
                size: *sizes
                    .get(fi)
                    .and_then(|row| row.get(j))
                    .ok_or("missing substream size")?,
                crc,
            });
        }
    }
    Ok(out)
}

fn read_bitmap(r: &mut Reader, count: usize) -> Result<Vec<bool>, String> {
    if count > MAX_ITEMS {
        return Err("bitmap count limit exceeded".into());
    }
    // 7z defined/empty bitmaps pack MSB-first within each byte.
    let flags = r.read((count + 7) / 8)?.to_vec();
    Ok((0..count)
        .map(|i| flags[i / 8] & (0x80 >> (i % 8)) != 0)
        .collect())
}

struct FileInfoProps {
    names: Vec<String>,
    empty_stream: Vec<bool>,
    empty_file: Vec<bool>,
    attrs: Vec<u32>,
}

fn read_files_info(r: &mut Reader, num: usize) -> Result<FileInfoProps, String> {
    if num > MAX_ITEMS {
        return Err("file count limit exceeded".into());
    }
    let mut empty_stream = vec![false; num];
    let mut empty_file: Vec<bool> = Vec::new();
    let mut names: Option<Vec<String>> = None;
    let mut attrs = vec![0u32; num];

    loop {
        let pid = r.u8()?;
        if pid == K_END {
            break;
        }
        let size = usize_value(r.varint()?)?;
        let property = r.read(size)?;
        let mut p = Reader::new(property);
        match pid {
            K_EMPTY_STREAM => empty_stream = read_bitmap(&mut p, num)?,
            K_EMPTY_FILE => {
                let n_empty = empty_stream.iter().filter(|b| **b).count();
                empty_file = read_bitmap(&mut p, n_empty)?;
            }
            K_NAME => {
                if p.u8()? != 0 {
                    return Err("external names not supported".into());
                }
                let blob = p.read(p.bytes_left())?;
                let units: Vec<u16> = blob
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                names = Some(
                    String::from_utf16_lossy(&units)
                        .split('\0')
                        .take(num)
                        .map(|s| s.to_string())
                        .collect(),
                );
            }
            K_WIN_ATTRIBUTES => {
                let defined = if p.u8()? == 0 {
                    read_bitmap(&mut p, num)?
                } else {
                    vec![true; num]
                };
                if p.u8()? != 0 {
                    return Err("external attribute stream not supported".into());
                }
                for i in 0..num {
                    if defined[i] {
                        attrs[i] = p.u32()?;
                    }
                }
            }
            K_MTIME => {
                let defined = if p.u8()? == 0 {
                    read_bitmap(&mut p, num)?
                } else {
                    vec![true; num]
                };
                if p.u8()? != 0 {
                    return Err("external timestamp stream not supported".into());
                }
                for i in 0..num {
                    if defined[i] {
                        p.u64()?; // FILETIME, retained positionally
                    }
                }
            }
            // kAnti / kCTime / kATime / kStartPos / kDummy / kComment: skipped
            _ => {}
        }
    }

    Ok(FileInfoProps {
        names: names.unwrap_or_else(|| (0..num).map(|i| format!("file{i}")).collect()),
        empty_stream,
        empty_file,
        attrs,
    })
}

pub fn parse_archive(data: &[u8]) -> Result<Archive, String> {
    if data.len() < 32 || &data[..6] != MAGIC {
        return Err("not a 7z archive".into());
    }
    if data[6] != 0 {
        return Err(format!("unsupported 7z version {}.{}", data[6], data[7]));
    }
    let mut h = Reader::new(data);
    h.p = 8;
    let start_crc = h.u32()?;
    let next_off = usize_value(h.u64()?)?;
    let next_size = usize_value(h.u64()?)?;
    if next_size > MAX_HEADER {
        return Err("native header exceeds 32 MiB limit".into());
    }
    let next_crc = h.u32()?;
    if crc32(&data[12..32]) != start_crc {
        return Err("start header CRC mismatch".into());
    }

    let header_offset = 32usize.checked_add(next_off).ok_or("7z offset overflow")?;
    if header_offset
        .checked_add(next_size)
        .ok_or("7z size overflow")?
        > data.len()
    {
        return Err("truncated 7z next header".into());
    }
    let mut header: Vec<u8> = data[header_offset..header_offset + next_size].to_vec();
    if crc32(&header) != next_crc {
        return Err("next header CRC mismatch".into());
    }

    let encoded = *header.first().ok_or("empty 7z header")? == K_ENCODED_HEADER;
    if encoded {
        let mut r = Reader::new(&header);
        r.p = 1;
        let (pack_offset, pack_sizes, folders, _) = read_streams_info(&mut r)?;
        if folders.len() != 1 {
            return Err("encoded header must contain one folder".into());
        }
        let start = pack_offset;
        let end = start
            .checked_add(*pack_sizes.first().ok_or("missing header pack size")?)
            .ok_or("header pack size overflow")?;
        if end > data.len() {
            return Err("encoded header stream out of range".into());
        }
        header = decode_folder_limit(&folders[0], &data[start..end], MAX_HEADER)?;
    }

    let mut r = Reader::new(&header);
    if r.u8()? != K_HEADER {
        return Err("expected header node".into());
    }

    let mut pack_offset = 32usize;
    let mut pack_sizes: Vec<usize> = Vec::new();
    let mut folders: Vec<Folder> = Vec::new();
    let mut substreams: Vec<SubStream> = Vec::new();
    let mut files: Option<FileInfoProps> = None;
    let mut num_files = 0usize;

    while r.bytes_left() > 0 {
        let nid = r.u8()?;
        match nid {
            K_END => break,
            K_ARCHIVE_PROPERTIES => loop {
                let pid = r.u8()?;
                if pid == K_END {
                    break;
                }
                let size = r.varint()? as usize;
                r.read(size)?;
            },
            K_ADDITIONAL_STREAMS => return Err("additional streams not supported".into()),
            K_MAIN_STREAMS => {
                let (po, ps, f, s) = read_streams_info(&mut r)?;
                pack_offset = po;
                pack_sizes = ps;
                folders = f;
                substreams = s;
            }
            K_FILES_INFO => {
                num_files = count(&mut r, MAX_ITEMS)?;
                files = Some(read_files_info(&mut r, num_files)?);
            }
            other => return Err(format!("unexpected header node {other:#x}")),
        }
    }

    if !folders.is_empty() && substreams.is_empty() {
        substreams = folders
            .iter()
            .enumerate()
            .map(|(i, f)| SubStream {
                folder_index: i,
                size: f.unpack_size(),
                crc: f.unpack_crc,
            })
            .collect();
    }

    if folders
        .iter()
        .try_fold(0u64, |sum, f| sum.checked_add(f.unpack_size()))
        .filter(|&sum| sum <= MAX_NATIVE_OUTPUT as u64)
        .is_none()
    {
        return Err("native archive output exceeds 512 MiB limit".into());
    }
    let packed_end = pack_sizes
        .iter()
        .try_fold(pack_offset, |offset, &size| offset.checked_add(size))
        .ok_or("packed stream range overflow")?;
    if packed_end > data.len() {
        return Err("packed streams exceed input".into());
    }
    if folders.len() != pack_sizes.len() {
        return Err("native reader requires one pack stream per folder".into());
    }
    let entries = build_entries(files, &substreams, num_files)?;
    Ok(Archive {
        pack_offset,
        pack_sizes,
        folders,
        substreams,
        entries,
        header_offset,
        header_size: next_size,
        encoded_header: encoded,
    })
}

fn build_entries(
    files: Option<FileInfoProps>,
    substreams: &[SubStream],
    num_files: usize,
) -> Result<Vec<Entry>, String> {
    let props = match files {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    let mut entries = Vec::with_capacity(num_files);
    let mut cursor: Vec<u64> = Vec::new();
    let mut stream_i = 0usize;
    let mut empty_i = 0usize;
    for i in 0..num_files {
        let name = props
            .names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("file{i}"));
        let attrs = props.attrs.get(i).copied().unwrap_or(0);
        if props.empty_stream.get(i).copied().unwrap_or(false) {
            let is_dir = match props.empty_file.get(empty_i) {
                Some(is_file) => !is_file,
                None => true,
            };
            empty_i += 1;
            entries.push(Entry {
                name,
                is_dir,
                size: 0,
                crc: None,
                offset_in_stream: 0,
                folder_index: None,
                attrs,
            });
        } else {
            let sub = substreams.get(stream_i);
            stream_i += 1;
            let (size, crc, folder_index) = match sub {
                Some(s) => (s.size, s.crc, Some(s.folder_index)),
                None => return Err("missing file substream".into()),
            };
            let off = if let Some(f) = folder_index {
                if f >= MAX_ITEMS {
                    return Err("folder index exceeds limit".into());
                }
                while cursor.len() <= f {
                    cursor.push(0);
                }
                let offset = cursor[f];
                cursor[f] = offset
                    .checked_add(size)
                    .ok_or("entry stream size overflow")?;
                offset
            } else {
                0
            };
            entries.push(Entry {
                name,
                is_dir: false,
                size,
                crc,
                offset_in_stream: off,
                folder_index,
                attrs,
            });
        }
    }
    Ok(entries)
}

fn decode_folder(folder: &Folder, packed: &[u8]) -> Result<Vec<u8>, String> {
    decode_folder_limit(folder, packed, MAX_NATIVE_OUTPUT)
}
fn decode_folder_limit(folder: &Folder, packed: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if folder.coders.is_empty()
        || folder.coders.len() > MAX_CODERS
        || folder.total_in() > MAX_CODERS
        || folder.total_out() > MAX_CODERS
    {
        return Err("native coder count exceeds limit".into());
    }
    if packed.len() > MAX_NATIVE_OUTPUT
        || folder.unpack_sizes.len() != folder.total_out()
        || folder.unpack_sizes.iter().any(|&n| n > limit as u64)
    {
        return Err("native decoded output limit exceeded".into());
    }
    if folder
        .bind_pairs
        .iter()
        .any(|&(a, b)| a >= folder.total_in() || b >= folder.total_out())
    {
        return Err("invalid coder binding".into());
    }
    let mut input_bindings = std::collections::HashSet::new();
    let mut output_bindings = std::collections::HashSet::new();
    for &(a, b) in &folder.bind_pairs {
        if !input_bindings.insert(a) || !output_bindings.insert(b) {
            return Err("duplicate coder binding".into());
        }
    }
    if folder.coders.iter().any(|c| c.codec == CODEC_7Z_AES) {
        return Err("encrypted (7z-AES) streams are not supported".into());
    }
    if folder.packed_indices.len() > 1 {
        return Err("multi packed-stream folders are not supported".into());
    }
    let total_in = folder.total_in();
    let total_out = folder.total_out();
    let bound_ins: Vec<usize> = folder.bind_pairs.iter().map(|(a, _)| *a).collect();
    let bound_outs: Vec<usize> = folder.bind_pairs.iter().map(|(_, b)| *b).collect();

    let mut in_data: Vec<Option<Vec<u8>>> = vec![None; total_in];
    let packed_slots: Vec<usize> = (0..total_in).filter(|i| !bound_ins.contains(i)).collect();
    match packed_slots.len() {
        0 => {}
        1 => in_data[packed_slots[0]] = Some(packed.to_vec()),
        _ => return Err("folder input split unsupported".into()),
    }

    let mut out_data: Vec<Option<Vec<u8>>> = vec![None; total_out];
    let mut out_base = Vec::with_capacity(folder.coders.len());
    let mut base = 0usize;
    for c in &folder.coders {
        out_base.push(base);
        base += c.num_out;
    }

    let mut in_base = Vec::new();
    let mut base = 0;
    for coder in &folder.coders {
        in_base.push(base);
        base += coder.num_in;
    }
    let mut pending: Vec<usize> = (0..folder.coders.len()).collect();
    while !pending.is_empty() {
        let mut progressed = false;
        let mut next_pending = Vec::new();
        for &ci in &pending {
            let coder = &folder.coders[ci];
            if coder.num_in != 1 || coder.num_out != 1 {
                return Err("complex coder unsupported".into());
            }
            let slots: Vec<usize> = (in_base[ci]..in_base[ci] + coder.num_in).collect();
            if slots.iter().all(|s| in_data[*s].is_some()) {
                let args: Vec<Vec<u8>> =
                    slots.iter().map(|s| in_data[*s].take().unwrap()).collect();
                let ob = out_base[ci];
                let expected = folder.unpack_sizes.get(ob).copied();
                let result = decode_coder(coder, &args, expected)?;
                if let Some((a, _)) = folder.bind_pairs.iter().find(|(_, b)| *b == ob) {
                    *in_data.get_mut(*a).ok_or("invalid coder binding")? = Some(result);
                } else {
                    out_data[ob] = Some(result);
                }

                progressed = true;
            } else {
                next_pending.push(ci);
            }
        }
        pending = next_pending;
        if !progressed && !pending.is_empty() {
            return Err("coder chain has unresolved inputs".into());
        }
    }

    for i in 0..total_out {
        if !bound_outs.contains(&i) {
            return out_data[i]
                .take()
                .ok_or_else(|| "folder produced no output".into());
        }
    }
    Err("folder produced no output".into())
}

fn decode_coder(coder: &Coder, args: &[Vec<u8>], expected: Option<u64>) -> Result<Vec<u8>, String> {
    let expected = usize_value(expected.ok_or("missing coder output size")?)?;
    if expected > MAX_NATIVE_OUTPUT {
        return Err("native decoded output limit exceeded".into());
    }
    let input = args.first().ok_or("missing coder input")?;
    let out = match coder.codec.as_slice() {
        CODEC_COPY => {
            if input.len() != expected {
                return Err("COPY size mismatch".into());
            }
            Ok(input.clone())
        }
        CODEC_DEFLATE => miniz_oxide::inflate::decompress_to_vec_with_limit(input, expected)
            .map_err(|e| format!("deflate: {e:?}")),
        CODEC_LZMA2 => lzma2_decode(input, expected),
        CODEC_LZMA => lzma1_decode(input, &coder.props, expected),
        CODEC_BCJ_X86 => {
            let start = if coder.props.len() >= 4 {
                u32::from_le_bytes([
                    coder.props[0],
                    coder.props[1],
                    coder.props[2],
                    coder.props[3],
                ])
            } else {
                0
            };
            if input.len() != expected {
                return Err("BCJ size mismatch".into());
            }
            Ok(bcj_x86_decode(input, start))
        }
        other => Err(format!("unsupported codec {:02x?}", other)),
    }?;
    if out.len() != expected {
        return Err("native coder output size mismatch".into());
    }
    Ok(out)
}

fn lzma1_decode(blob: &[u8], props: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    if props.len() < 5 {
        return Err("LZMA coder without properties".into());
    }
    // Feed lzma-rs a header of the 5 property bytes only; size is provided.
    let mut input = Vec::with_capacity(5 + blob.len());
    input.extend_from_slice(&props[..5]);
    input.extend_from_slice(blob);
    let mut out = BoundedOutput::new(expected);
    let opts = lzma_rs::decompress::Options {
        unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(expected as u64)),
        memlimit: Some(MAX_NATIVE_OUTPUT),
        ..Default::default()
    };
    lzma_rs::lzma_decompress_with_options(&mut Cursor::new(&input), &mut out, &opts)
        .map_err(|e| format!("LZMA decode: {e}"))?;
    Ok(out.bytes)
}

fn lzma2_decode(blob: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    preflight_lzma2(blob, expected)?;
    let mut out = BoundedOutput::new(expected);
    lzma_rs::lzma2_decompress(&mut Cursor::new(blob), &mut out)
        .map_err(|e| format!("LZMA2 decode: {e}"))?;
    Ok(out.bytes)
}

fn preflight_lzma2(blob: &[u8], expected: usize) -> Result<(), String> {
    let mut r = Reader::new(blob);
    let mut total = 0usize;
    loop {
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
    if total != expected {
        return Err("LZMA2 output size mismatch".into());
    }
    Ok(())
}

/// BCJ x86 decoder, adapted from XZ Utils x86.c (0BSD).
/// Original authors: Igor Pavlov and Lasse Collin.
fn bcj_x86_decode(data: &[u8], pc: u32) -> Vec<u8> {
    const MASK_TO_BIT: [u32; 5] = [0, 1, 2, 2, 3];
    let mut out = data.to_vec();
    let mut mask = 0u32;
    let mut prev = pc.wrapping_sub(5);
    let mut i = 0;
    while i + 4 < out.len() {
        if out[i] != 0xe8 && out[i] != 0xe9 {
            i += 1;
            continue;
        }
        let pos = pc.wrapping_add(i as u32);
        let offset = pos.wrapping_sub(prev);
        prev = pos;
        if offset > 5 {
            mask = 0;
        } else {
            for _ in 0..offset {
                mask = (mask & 0x77) << 1;
            }
        }
        let b = out[i + 4];
        if matches!(b, 0 | 0xff) && mask >> 1 <= 4 && mask >> 1 != 3 {
            let mut src = u32::from_le_bytes(out[i + 1..i + 5].try_into().unwrap());
            let mut dest;
            loop {
                dest = src.wrapping_sub(pos.wrapping_add(5));
                if mask == 0 {
                    break;
                }
                let bit = MASK_TO_BIT[(mask >> 1) as usize];
                let b = (dest >> (24 - bit * 8)) as u8;
                if !matches!(b, 0 | 0xff) {
                    break;
                }
                src = dest ^ ((1u32 << (32 - bit * 8)) - 1);
            }
            out[i + 1..i + 5].copy_from_slice(&dest.to_le_bytes());
            out[i + 4] = if dest & 0x01000000 != 0 { 0xff } else { 0 };
            i += 5;
            mask = 0;
        } else {
            i += 1;
            mask |= 1;
            if matches!(b, 0 | 0xff) {
                mask |= 0x10;
            }
        }
    }
    out
}

mod memory;
pub(crate) use memory::parse_memory_archive;

#[cfg(test)]
pub(crate) use memory::tests::fixture as memory_test_fixture;
