//! Native installer adapter with the same selection and publication policy as archives.
use crate::{
    engine::{self, EngineOptions, Entry},
    setup,
    util::crc32,
    zipfmt,
};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};
const INPUT_LIMIT: u64 = 128 * 1024 * 1024;
fn package(data: &[u8]) -> Result<setup::SetupPackage<'_>, String> {
    std::panic::catch_unwind(|| setup::open(data))
        .map_err(|_| "malformed installer package".to_string())?
}
pub fn recognize(path: &str) -> Result<Option<Vec<u8>>, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > INPUT_LIMIT {
        return Ok(None);
    }
    let mut magic = [0; 2];
    if f.read_exact(&mut magic).is_err() || magic != *b"MZ" {
        return Ok(None);
    }
    let mut data = magic.to_vec();
    f.take(INPUT_LIMIT - 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > INPUT_LIMIT {
        return Ok(None);
    }
    if package(&data).is_ok() {
        Ok(Some(data))
    } else {
        Ok(None)
    }
}
fn inventory(pkg: &setup::SetupPackage<'_>) -> Result<Vec<Entry>, String> {
    let mut es = Vec::new();
    for e in &pkg.archive.entries {
        es.push(Entry {
            name: format!("payload/{}", e.name.replace('\\', "/")),
            size: e.size,
            packed_size: 0,
            is_dir: e.is_dir,
            crc: e.crc.map(|c| format!("{c:08X}")).unwrap_or_default(),
            encrypted: false,
            modified: String::new(),
            method: "7z payload".into(),
            comment: String::new(),
            attributes: String::new(),
        })
    }
    for e in &pkg.ui_entries {
        es.push(Entry {
            name: format!("ui/{}", e.name.replace('\\', "/")),
            size: e.size,
            packed_size: e.comp_size,
            is_dir: e.name.ends_with(['/', '\\']),
            crc: format!("{:08X}", e.crc),
            encrypted: false,
            modified: String::new(),
            method: format!("ZIP {}", e.method),
            comment: String::new(),
            attributes: String::new(),
        })
    }
    let mut seen = HashSet::new();
    let mut files = HashSet::new();
    let mut total = 0u64;
    for e in &es {
        let key = engine::validate_name(&e.name)?
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();
        if !seen.insert(key.clone()) {
            return Err("installer entry names collide".into());
        }
        if !e.is_dir {
            files.insert(key);
        }
        total = total.checked_add(e.size).ok_or("installer size overflow")?;
    }
    if es.len() > 100000 || total > 512 * 1024 * 1024 {
        return Err("installer exceeds 100000 entries / 512 MiB output limit".into());
    }
    for name in &seen {
        for (at, _) in name.match_indices('/') {
            if files.contains(&name[..at]) {
                return Err("installer file/directory collision".into());
            }
        }
    }
    Ok(es)
}
pub fn list(data: &[u8]) -> Result<Vec<Entry>, String> {
    inventory(&package(data)?)
}
fn check(opts: &EngineOptions) -> Result<(), String> {
    if let Some(c) = &opts.control {
        c.check()?;
    }
    if !["error", "overwrite", "skip", "rename"].contains(&opts.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    Ok(())
}
fn decode(
    pkg: &setup::SetupPackage<'_>,
    es: &[Entry],
    stage: &Path,
    opts: &EngineOptions,
) -> Result<(), String> {
    let names: HashSet<_> = es.iter().map(|e| e.name.as_str()).collect();
    let mut cache = HashMap::new();
    let mut decoded_budget = 0u64;
    for e in &pkg.archive.entries {
        check(opts)?;
        let name = format!("payload/{}", e.name.replace('\\', "/"));
        if !names.contains(name.as_str()) || e.is_dir {
            continue;
        }
        let bytes = if let Some(fi) = e.folder_index {
            if let std::collections::hash_map::Entry::Vacant(v) = cache.entry(fi) {
                let f = pkg
                    .archive
                    .folders
                    .get(fi)
                    .ok_or("missing installer folder")?;
                if f.unpack_sizes.iter().any(|&n| n > 512 * 1024 * 1024) {
                    return Err("installer folder exceeds 512 MiB memory limit".into());
                }
                decoded_budget = decoded_budget
                    .checked_add(f.unpack_size())
                    .ok_or("installer folder size overflow")?;
                if decoded_budget > 512 * 1024 * 1024 {
                    return Err("installer folder cache exceeds 512 MiB limit".into());
                }
                v.insert(pkg.archive.read_folder(pkg.data_sevenz(), fi)?);
                check(opts)?;
            }
            let start = usize::try_from(e.offset_in_stream).map_err(|_| "entry offset overflow")?;
            let end = start
                .checked_add(usize::try_from(e.size).map_err(|_| "entry size overflow")?)
                .ok_or("entry range overflow")?;
            cache[&fi]
                .get(start..end)
                .ok_or("entry exceeds decoded folder")?
        } else if e.size == 0 {
            &[]
        } else {
            return Err("missing installer file stream".into());
        };
        if e.crc.is_some_and(|c| crc32(bytes) != c) {
            return Err(format!("CRC mismatch: {name}"));
        }
        let target = stage.join(engine::validate_name(&name)?);
        fs::create_dir_all(target.parent().ok_or("missing parent")?).map_err(|e| e.to_string())?;
        fs::write(target, bytes).map_err(|e| e.to_string())?;
    }
    for e in &pkg.ui_entries {
        check(opts)?;
        let name = format!("ui/{}", e.name.replace('\\', "/"));
        if !names.contains(name.as_str()) || e.name.ends_with(['/', '\\']) {
            continue;
        }
        let bytes = zipfmt::extract_entry(pkg.data, pkg.zip_offset, e)?;
        if bytes.len() as u64 != e.size || crc32(&bytes) != e.crc {
            return Err(format!("CRC/size mismatch: {name}"));
        }
        let target = stage.join(engine::validate_name(&name)?);
        fs::create_dir_all(target.parent().ok_or("missing parent")?).map_err(|e| e.to_string())?;
        fs::write(target, bytes).map_err(|e| e.to_string())?;
    }
    check(opts)
}
fn decoded(
    pkg: &setup::SetupPackage<'_>,
    es: &[Entry],
    stage: &Path,
    opts: &EngineOptions,
) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode(pkg, es, stage, opts)
    }))
    .map_err(|_| "malformed installer data".to_string())?
}
pub fn extract(data: &[u8], dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    check(opts)?;
    let pkg = package(data)?;
    let es: Vec<_> = inventory(&pkg)?
        .into_iter()
        .filter(|e| engine::selected(&e.name, opts))
        .collect();
    if !opts.selected.is_empty() && es.is_empty() {
        return Err("selected installer entries were not found".into());
    }
    let dest = std::path::absolute(dest).map_err(|e| e.to_string())?;
    engine::no_links(&dest)?;
    for e in &es {
        let p = dest.join(engine::validate_name(&e.name)?);
        engine::no_links(&p)?;
        if !e.is_dir && opts.overwrite == "error" && p.exists() {
            return Err(format!("destination exists: {}", p.display()));
        }
    }
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    decoded(&pkg, &es, stage.path(), opts)?;
    let mut count = 0;
    for e in &es {
        check(opts)?;
        let p = dest.join(engine::validate_name(&e.name)?);
        engine::no_links(&p)?;
        if e.is_dir {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
            continue;
        }
        let Some(p) = engine::choose_target(p, &opts.overwrite)? else {
            continue;
        };
        let parent = p.parent().ok_or("missing output parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        engine::no_links(parent)?;
        let source = stage.path().join(engine::validate_name(&e.name)?);
        let mut f = File::open(source).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut temp).map_err(|e| e.to_string())?;
        temp.flush().map_err(|e| e.to_string())?;
        check(opts)?;
        engine::no_links(&p)?;
        if opts.overwrite == "overwrite" {
            temp.persist(p).map_err(|e| e.error.to_string())?;
        } else {
            temp.persist_noclobber(p).map_err(|e| e.error.to_string())?;
        }
        count += 1;
    }
    Ok(count)
}
pub fn test(data: &[u8], opts: &EngineOptions) -> Result<(), String> {
    check(opts)?;
    let pkg = package(data)?;
    let es = inventory(&pkg)?;
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    decoded(&pkg, &es, stage.path(), opts)
}
pub fn preview(data: &[u8], name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    check(opts)?;
    let pkg = package(data)?;
    let e = inventory(&pkg)?
        .into_iter()
        .find(|e| e.name == name)
        .ok_or("installer entry not found")?;
    if e.is_dir || e.size > 32 * 1024 * 1024 {
        return Err("preview requires a file of at most 32 MiB".into());
    }
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = stage.path().join(engine::validate_name(&e.name)?);
    decoded(&pkg, &[e], stage.path(), opts)?;
    fs::read(path).map_err(|e| e.to_string())
}
