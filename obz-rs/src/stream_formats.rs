//! Bounded ASAR and Brotli/LZ4 readers. No archive content is executed.
//! ASAR layout follows electron/asar disk.ts and filesystem.ts; Brotli has no
//! magic or built-in checksum, so .br is required and its full stream is parsed.
use crate::engine::{self, EngineOptions, Entry};
use serde::{
    de::{MapAccess, Visitor},
    Deserialize, Deserializer,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
const MAX_MEMBER: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_HEADER: u64 = 16 * 1024 * 1024;
fn check(opts: &EngineOptions) -> Result<(), String> {
    if let Some(c) = &opts.control {
        c.check()?;
    }
    if !["error", "overwrite", "skip", "rename"].contains(&opts.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    Ok(())
}
fn guarded<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err("malformed stream rejected by decoder".into()))
}
fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
/// Dispatch hint only: open/list always validates the entire header/stream.
/// Raw Brotli has no signature and is considered only with the .br extension.
pub fn is_supported(path: &str) -> bool {
    let ext = extension(path);
    if ext == "br" {
        return true;
    }
    let mut magic = [0; 4];
    if File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_err()
    {
        return false;
    }
    (ext == "asar" && magic == [4, 0, 0, 0]) || (ext == "lz4" && magic == [4, 0x22, 0x4d, 0x18])
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    files: Option<UniqueFiles>,
    size: Option<u64>,
    offset: Option<String>,
    link: Option<String>,
    unpacked: Option<bool>,
    integrity: Option<Integrity>,
    executable: Option<bool>,
}
struct UniqueFiles(BTreeMap<String, Node>);
impl<'de> Deserialize<'de> for UniqueFiles {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = UniqueFiles;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("unique ASAR entry names")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut result = BTreeMap::new();
                while let Some((name, node)) = map.next_entry::<String, Node>()? {
                    if result.insert(name, node).is_some() {
                        return Err(serde::de::Error::custom("duplicate ASAR JSON name"));
                    }
                }
                Ok(UniqueFiles(result))
            }
        }
        d.deserialize_map(V)
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Integrity {
    algorithm: String,
    hash: String,
    #[serde(rename = "blockSize")]
    block_size: u64,
    blocks: Vec<String>,
}
struct Item {
    entry: Entry,
    offset: u64,
    integrity: Option<Integrity>,
}
struct Archive {
    file: File,
    _temp: Option<tempfile::NamedTempFile>,
    items: Vec<Item>,
}
fn entry(name: String, size: u64, packed: u64, dir: bool, method: &str) -> Entry {
    Entry {
        name,
        size,
        packed_size: packed,
        is_dir: dir,
        crc: String::new(),
        encrypted: false,
        modified: String::new(),
        method: method.into(),
        comment: String::new(),
        attributes: String::new(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hex_hash_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn asar(file: File, opts: &EngineOptions) -> Result<Archive, String> {
    let mut archive = Archive {
        file,
        _temp: None,
        items: Vec::new(),
    };
    let len = archive.file.metadata().map_err(|e| e.to_string())?.len();
    if len > MAX_TOTAL {
        return Err("ASAR input exceeds 2 GiB bound".into());
    }
    let mut prefix = [0u8; 16];
    archive
        .file
        .read_exact(&mut prefix)
        .map_err(|_| "truncated ASAR pickle header")?;
    let u = |i| u32::from_le_bytes(prefix[i..i + 4].try_into().unwrap()) as u64;
    let header_size = u(4);
    let json_size = u(12);
    if u(0) != 4
        || header_size < 8
        || header_size > MAX_HEADER
        || header_size % 4 != 0
        || u(8) + 4 != header_size
        || json_size + 8 > header_size
        || header_size - (json_size + 8) > 3
        || 8 + header_size > len
    {
        return Err("invalid ASAR pickle sizes".into());
    }
    let mut json = vec![0; json_size as usize];
    archive
        .file
        .read_exact(&mut json)
        .map_err(|e| e.to_string())?;
    let root: Node =
        serde_json::from_slice(&json).map_err(|e| format!("invalid ASAR JSON: {e}"))?;
    let mut names = HashSet::new();
    let mut total = 0u64;
    fn visit(
        node: Node,
        path: String,
        depth: usize,
        data_start: u64,
        len: u64,
        items: &mut Vec<Item>,
        names: &mut HashSet<String>,
        total: &mut u64,
        opts: &EngineOptions,
    ) -> Result<(), String> {
        check(opts)?;
        if depth > 64 || items.len() >= 100000 {
            return Err("ASAR tree exceeds depth/member limit".into());
        }
        if node.link.is_some() {
            return Err("ASAR symbolic links are not extracted".into());
        }
        if node.unpacked == Some(true) {
            return Err("ASAR external unpacked members are not read".into());
        }
        if !path.is_empty() {
            engine::validate_name(&path)?;
            if !names.insert(path.to_lowercase()) {
                return Err("ASAR case-colliding names".into());
            }
        }
        if let Some(files) = node.files {
            if node.size.is_some() || node.offset.is_some() || node.integrity.is_some() {
                return Err("ASAR directory has conflicting file metadata".into());
            }
            if !path.is_empty() {
                items.push(Item {
                    entry: entry(path.clone(), 0, 0, true, "ASAR"),
                    offset: 0,
                    integrity: None,
                });
            }
            for (name, child) in files.0 {
                if name.is_empty() || name.contains(['/', '\\']) {
                    return Err("invalid ASAR member component".into());
                }
                let name = if path.is_empty() {
                    name
                } else {
                    format!("{path}/{name}")
                };
                visit(
                    child,
                    name,
                    depth + 1,
                    data_start,
                    len,
                    items,
                    names,
                    total,
                    opts,
                )?;
            }
        } else {
            if path.is_empty() {
                return Err("ASAR root must be directory".into());
            }
            let size = node.size.ok_or("ASAR file lacks size")?;
            let offset = node.offset.ok_or("ASAR file lacks offset")?;
            if offset.is_empty() || !offset.bytes().all(|b| b.is_ascii_digit()) {
                return Err("ASAR offset must be unsigned decimal string".into());
            }
            let offset = offset
                .parse::<u64>()
                .map_err(|_| "ASAR offset overflow")?
                .checked_add(data_start)
                .ok_or("ASAR offset overflow")?;
            if size > MAX_MEMBER || offset.checked_add(size).filter(|end| *end <= len).is_none() {
                return Err("ASAR member exceeds file or resource boundary".into());
            }
            *total = total
                .checked_add(size)
                .filter(|n| *n <= MAX_TOTAL)
                .ok_or("ASAR cumulative output exceeds 2 GiB bound")?;
            if let Some(i) = &node.integrity {
                if i.algorithm != "SHA256"
                    || !hex_hash_valid(&i.hash)
                    || i.block_size == 0
                    || i.block_size > MAX_MEMBER
                    || i.blocks.len() as u64 != size.div_ceil(i.block_size).max(1)
                    || !i.blocks.iter().all(|h| hex_hash_valid(h))
                {
                    return Err("unsupported or malformed ASAR integrity metadata".into());
                }
            }
            let mut e = entry(path, size, size, false, "ASAR");
            if node.executable == Some(true) {
                e.attributes = "executable metadata; not executed".into();
            }
            items.push(Item {
                entry: e,
                offset,
                integrity: node.integrity,
            });
        }
        Ok(())
    }
    visit(
        root,
        String::new(),
        0,
        8 + header_size,
        len,
        &mut archive.items,
        &mut names,
        &mut total,
        opts,
    )?;
    Ok(archive)
}
/// Pre-validate exact LZ4 frame boundaries. The dependency treats EOF at a
/// missing block header as a clean end, so relying on Read alone is insufficient.
fn lz4_frame(data: &[u8]) -> Result<(), String> {
    if data.len() < 11 || data[..4] != [4, 0x22, 0x4d, 0x18] {
        return Err("invalid LZ4 frame signature".into());
    }
    let flags = data[4];
    let bd = data[5];
    if flags >> 6 != 1 || flags & 2 != 0 || bd & 0x8f != 0 {
        return Err("invalid LZ4 reserved/version bits".into());
    }
    if flags & 1 != 0 {
        return Err("LZ4 external dictionaries unsupported".into());
    }
    let block = match (bd >> 4) & 7 {
        4 => 65536,
        5 => 262144,
        6 => 1048576,
        7 => 4194304,
        _ => return Err("invalid LZ4 block bound".into()),
    };
    let mut pos = 7 + if flags & 8 != 0 { 8 } else { 0 };
    let mut blocks = 0;
    if flags & 8 != 0 {
        let length = data.get(6..14).ok_or("truncated LZ4 content size")?;
        if u64::from_le_bytes(length.try_into().unwrap()) > MAX_MEMBER {
            return Err("LZ4 declared size exceeds bound".into());
        }
    }
    loop {
        let b = data.get(pos..pos + 4).ok_or("LZ4 missing end marker")?;
        let n = u32::from_le_bytes(b.try_into().unwrap());
        pos += 4;
        if n == 0 {
            if flags & 4 != 0 {
                pos += 4;
            }
            if pos != data.len() {
                return Err("LZ4 truncated checksum or trailing frames/data unsupported".into());
            }
            return Ok(());
        }
        let n = (n & 0x7fff_ffff) as usize;
        if n == 0 || n > block {
            return Err("LZ4 block exceeds frame bound".into());
        }
        pos = pos
            .checked_add(n + if flags & 16 != 0 { 4 } else { 0 })
            .filter(|p| *p <= data.len())
            .ok_or("truncated LZ4 block")?;
        blocks += 1;
        if blocks > 100000 {
            return Err("LZ4 block count exceeds bound".into());
        }
    }
}
fn stream(file: File, path: &str, opts: &EngineOptions) -> Result<Archive, String> {
    let mut packed = Vec::new();
    file.take(MAX_MEMBER + 1)
        .read_to_end(&mut packed)
        .map_err(|e| e.to_string())?;
    if packed.len() as u64 > MAX_MEMBER {
        return Err("compressed stream exceeds 512 MiB bound".into());
    }
    let mut temp = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let mut total = 0u64;
    let mut emit = |data: &[u8]| -> Result<(), String> {
        check(opts)?;
        total = total
            .checked_add(data.len() as u64)
            .filter(|n| *n <= MAX_MEMBER)
            .ok_or("decoded stream exceeds 512 MiB bound")?;
        temp.write_all(data).map_err(|e| e.to_string())
    };
    let ext = extension(path);
    if ext == "lz4" {
        lz4_frame(&packed)?;
        let mut decoder = lz4_flex::frame::FrameDecoder::new(std::io::Cursor::new(&packed));
        let mut buffer = [0u8; 65536];
        loop {
            check(opts)?;
            let n = decoder
                .read(&mut buffer)
                .map_err(|e| format!("LZ4 decode/checksum failure: {e}"))?;
            if n == 0 {
                break;
            }
            emit(&buffer[..n])?;
        }
    } else {
        let mut state = brotli::BrotliState::new(
            brotli::HeapAlloc::default(),
            brotli::HeapAlloc::default(),
            brotli::HeapAlloc::default(),
        );
        let mut available = packed.len();
        let mut offset = 0usize;
        let mut written = 0usize;
        let mut buffer = [0u8; 65536];
        loop {
            check(opts)?;
            let mut out_available = buffer.len();
            let mut out = 0;
            let before = offset;
            let result = brotli::BrotliDecompressStream(
                &mut available,
                &mut offset,
                &packed,
                &mut out_available,
                &mut out,
                &mut buffer,
                &mut written,
                &mut state,
            );
            emit(&buffer[..out])?;
            match result {
                brotli::BrotliResult::ResultSuccess => {
                    if available != 0 || offset != packed.len() {
                        return Err("Brotli trailing bytes rejected".into());
                    }
                    break;
                }
                brotli::BrotliResult::ResultFailure => return Err("invalid Brotli stream".into()),
                brotli::BrotliResult::NeedsMoreInput => {
                    return Err("truncated Brotli stream".into())
                }
                brotli::BrotliResult::NeedsMoreOutput => {
                    if out == 0 && offset == before {
                        return Err("Brotli decoder made no progress".into());
                    }
                }
            }
        }
    }
    drop(emit);
    temp.as_file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    let name = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("invalid stream filename")?
        .to_owned();
    engine::validate_name(&name)?;
    let method = if ext == "br" { "Brotli" } else { "LZ4 frame" };
    let item = Item {
        entry: entry(name, total, packed.len() as u64, false, method),
        offset: 0,
        integrity: None,
    };
    Ok(Archive {
        file: temp.reopen().map_err(|e| e.to_string())?,
        _temp: Some(temp),
        items: vec![item],
    })
}
fn open(path: &str, opts: &EngineOptions) -> Result<Archive, String> {
    check(opts)?;
    if !is_supported(path) {
        return Err("unsupported stream format/signature".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    if extension(path) == "asar" {
        asar(file, opts)
    } else {
        stream(file, path, opts)
    }
}
fn copy_item(
    file: &mut File,
    item: &Item,
    target: &mut impl Write,
    opts: &EngineOptions,
) -> Result<(), String> {
    file.seek(SeekFrom::Start(item.offset))
        .map_err(|e| e.to_string())?;
    let mut remaining = item.entry.size;
    let mut buffer = [0u8; 65536];
    let mut hash = Sha256::new();
    let mut block_hash = Sha256::new();
    let mut block_bytes = 0u64;
    let mut block_index = 0usize;
    loop {
        check(opts)?;
        if remaining == 0 {
            break;
        }
        let mut n = remaining.min(buffer.len() as u64);
        if let Some(i) = &item.integrity {
            n = n.min(i.block_size - block_bytes);
        }
        let n = n as usize;
        file.read_exact(&mut buffer[..n])
            .map_err(|_| "truncated member data")?;
        target.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        remaining -= n as u64;
        if let Some(i) = &item.integrity {
            hash.update(&buffer[..n]);
            block_hash.update(&buffer[..n]);
            block_bytes += n as u64;
            if block_bytes == i.block_size {
                let actual = hex(&block_hash.finalize_reset());
                if !actual.eq_ignore_ascii_case(&i.blocks[block_index]) {
                    return Err("ASAR block SHA256 mismatch".into());
                }
                block_index += 1;
                block_bytes = 0;
            }
        }
    }
    if let Some(i) = &item.integrity {
        if block_bytes > 0 || block_index == 0 {
            if !hex(&block_hash.finalize()).eq_ignore_ascii_case(&i.blocks[block_index]) {
                return Err("ASAR final block SHA256 mismatch".into());
            }
        }
        if !hex(&hash.finalize()).eq_ignore_ascii_case(&i.hash) {
            return Err("ASAR SHA256 mismatch".into());
        }
    }
    Ok(())
}
pub fn list(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    guarded(|| {
        Ok(open(path, opts)?
            .items
            .into_iter()
            .map(|i| i.entry)
            .collect())
    })
}
pub fn test(path: &str, opts: &EngineOptions) -> Result<(), String> {
    guarded(|| {
        let mut a = open(path, opts)?;
        for item in &a.items {
            if !item.entry.is_dir {
                copy_item(&mut a.file, item, &mut std::io::sink(), opts)?;
            }
        }
        Ok(())
    })
}
pub fn preview(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    guarded(|| {
        let mut a = open(path, opts)?;
        let item = a
            .items
            .iter()
            .find(|i| i.entry.name == name)
            .ok_or("member not found")?;
        if item.entry.is_dir || item.entry.size > 16 * 1024 * 1024 {
            return Err("preview requires file <=16 MiB".into());
        }
        let mut data = Vec::new();
        copy_item(&mut a.file, item, &mut data, opts)?;
        Ok(data)
    })
}
pub fn extract(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    guarded(|| {
        let mut a = open(path, opts)?;
        let chosen: Vec<_> = a
            .items
            .iter()
            .filter(|i| engine::selected(&i.entry.name, opts))
            .collect();
        if !opts.selected.is_empty() && chosen.is_empty() {
            return Err("selected entries not found".into());
        }
        let dest = std::path::absolute(dest).map_err(|e| e.to_string())?;
        engine::no_links(&dest)?;
        for item in &chosen {
            let target = dest.join(engine::validate_name(&item.entry.name)?);
            engine::no_links(&target)?;
            if !item.entry.is_dir && opts.overwrite == "error" && target.exists() {
                return Err("destination already exists".into());
            }
        }
        let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
        for item in &chosen {
            check(opts)?;
            let path = stage.path().join(engine::validate_name(&item.entry.name)?);
            if item.entry.is_dir {
                fs::create_dir_all(path).map_err(|e| e.to_string())?;
            } else {
                fs::create_dir_all(path.parent().ok_or("missing parent")?)
                    .map_err(|e| e.to_string())?;
                let mut f = File::create(path).map_err(|e| e.to_string())?;
                copy_item(&mut a.file, item, &mut f, opts)?;
            }
        }
        let mut count = 0;
        for item in chosen {
            check(opts)?;
            let target = dest.join(engine::validate_name(&item.entry.name)?);
            engine::no_links(&target)?;
            if item.entry.is_dir {
                fs::create_dir_all(target).map_err(|e| e.to_string())?;
                continue;
            }
            let Some(target) = engine::choose_target(target, &opts.overwrite)? else {
                continue;
            };
            let parent = target.parent().ok_or("missing parent")?;
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            engine::no_links(parent)?;
            let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
            let mut source =
                File::open(stage.path().join(engine::validate_name(&item.entry.name)?))
                    .map_err(|e| e.to_string())?;
            let mut buffer = [0; 65536];
            loop {
                check(opts)?;
                let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                temp.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            }
            engine::no_links(&target)?;
            if opts.overwrite == "overwrite" {
                temp.persist(target).map_err(|e| e.error.to_string())?;
            } else {
                temp.persist_noclobber(target)
                    .map_err(|e| e.error.to_string())?;
            }
            count += 1;
        }
        Ok(count)
    })
}
