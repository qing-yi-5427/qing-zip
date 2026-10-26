//! Shared extraction policy for the GUI, CLI and installer layers.
use crate::{
    sevenz::Archive,
    util::crc32,
    zipfmt::{self, ZipEntry},
};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub fn relative_path(name: &str) -> Result<PathBuf, String> {
    let name = name.replace('\\', "/");
    if name.is_empty() || name.starts_with('/') {
        return Err("unsafe archive path".into());
    }
    let mut path = PathBuf::new();
    for part in name.trim_end_matches('/').split('/') {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(format!("unsafe archive path: {name}"));
        }
        path.push(part);
    }
    Ok(path)
}

fn check_links(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                let mut link = meta.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    link |= meta.file_attributes() & 0x400 != 0;
                }
                if link {
                    return Err(format!(
                        "refusing linked extraction path: {}",
                        ancestor.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn target(root: &Path, rel: &Path, directory: bool) -> Result<PathBuf, String> {
    let path = root.join(rel);
    check_links(&path)?;
    let parent = if directory {
        path.as_path()
    } else {
        path.parent().ok_or("missing parent")?
    };
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    check_links(&path)?;
    Ok(path)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    // Replace the directory entry instead of truncating a possible hard link.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = path.with_file_name(format!(
        ".obz-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        f.write_all(bytes).map_err(|e| e.to_string())?;
        drop(f);
        check_links(path)?;
        fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn sevenz(archive: &Archive, data: &[u8], dest: &str) -> Result<usize, String> {
    let paths = archive
        .entries
        .iter()
        .map(|e| relative_path(&e.name))
        .collect::<Result<Vec<_>, _>>()?;
    let root = Path::new(dest);
    let mut cache = HashMap::new();
    let mut count = 0;
    for (entry, rel) in archive.entries.iter().zip(paths) {
        if entry.is_dir {
            target(root, &rel, true)?;
            continue;
        }
        let chunk = if let Some(fi) = entry.folder_index {
            if !cache.contains_key(&fi) {
                cache.insert(fi, archive.read_folder(data, fi)?);
            }
            let start =
                usize::try_from(entry.offset_in_stream).map_err(|_| "entry offset overflow")?;
            let size = usize::try_from(entry.size).map_err(|_| "entry size overflow")?;
            let end = start.checked_add(size).ok_or("entry range overflow")?;
            cache[&fi]
                .get(start..end)
                .ok_or("entry exceeds folder output")?
        } else if entry.size == 0 {
            &[]
        } else {
            return Err("missing file stream".into());
        };
        if entry.crc.is_some_and(|want| crc32(chunk) != want) {
            return Err(format!("CRC mismatch for {}", entry.name));
        }
        write_file(&target(root, &rel, false)?, chunk)?;
        count += 1;
    }
    Ok(count)
}

pub fn zip(data: &[u8], base: usize, entries: &[ZipEntry], dest: &str) -> Result<usize, String> {
    let paths = entries
        .iter()
        .map(|e| relative_path(&e.name))
        .collect::<Result<Vec<_>, _>>()?;
    let root = Path::new(dest);
    let mut count = 0;
    for (entry, rel) in entries.iter().zip(paths) {
        if entry.name.ends_with(['/', '\\']) {
            target(root, &rel, true)?;
            continue;
        }
        let bytes = zipfmt::extract_entry(data, base, entry)?;
        write_file(&target(root, &rel, false)?, &bytes)?;
        count += 1;
    }
    Ok(count)
}
