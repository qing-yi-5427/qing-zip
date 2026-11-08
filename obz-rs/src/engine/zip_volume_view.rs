use super::*;
use std::{
    cell::RefCell,
    io::{ErrorKind, Write},
};

thread_local! {
    static ACTIVE: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|paths| {
            paths.borrow_mut().pop();
        });
    }
}

fn numeric_zip(path: &Path, extension: &str) -> bool {
    extension.len() >= 3
        && extension.bytes().all(|b| b.is_ascii_digit())
        && path
            .file_stem()
            .map(Path::new)
            .and_then(Path::extension)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
}

fn disk_part(extension: &str) -> bool {
    let bytes = extension.as_bytes();
    bytes.len() >= 3
        && bytes[0].eq_ignore_ascii_case(&b'z')
        && bytes[1..].iter().all(u8::is_ascii_digit)
}

// For an ordinary ZIP the end record bounds the disk set. Treat an ambiguous
// end record as unknown rather than letting a second engine discover siblings.
fn single_disk(file: &mut File) -> Result<bool, String> {
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let tail_len = length.min(22 + 65535 + 20) as usize;
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0; tail_len];
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    file.rewind().map_err(|e| e.to_string())?;
    let mut found = false;
    for (i, record) in tail.windows(22).enumerate().rev() {
        if &record[..4] != b"PK\x05\x06" {
            continue;
        }
        let comment = u16::from_le_bytes([record[20], record[21]]) as usize;
        if i + 22 + comment != tail.len() {
            continue;
        }
        if found {
            return Ok(false);
        }
        found = true;
        if record[4..8] != [0; 4] || record[8..10] != record[10..12] {
            return Ok(false);
        }
        if i >= 20 && &tail[i - 20..i - 16] == b"PK\x06\x07" {
            if tail[i - 16..i - 12] != [0; 4] || tail[i - 4..i] != [1, 0, 0, 0] {
                return Ok(false);
            }
        }
    }
    Ok(found)
}

pub(super) fn require_editable(path: &Path) -> Result<(), String> {
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if numeric_zip(path, extension) || disk_part(extension) {
        return Err("updating split ZIP archives is unsupported".into());
    }
    if extension.eq_ignore_ascii_case("zip") && !single_disk(&mut *crate::engine_stream::pin(path)?)?
    {
        return Err("updating split or incomplete ZIP archives is unsupported".into());
    }
    Ok(())
}

fn open_part(path: &Path) -> Result<Option<File>, String> {
    match crate::engine_stream::open_source_leaf(path) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot open ZIP volume {}: {e}", path.display())),
    }
}

pub(crate) fn with_source<T>(
    path: &str,
    opts: &EngineOptions,
    action: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, String> {
    let input = Path::new(path);
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("");
    let numeric = numeric_zip(input, ext);
    if !numeric && !disk_part(ext) && !ext.eq_ignore_ascii_case("zip") {
        return action(path);
    }
    let input = absolute(path)?;
    if ACTIVE.with(|paths| paths.borrow().contains(&input)) {
        return action(path);
    }
    let mut anchor = crate::engine_stream::pin(&input)?;
    if ext.eq_ignore_ascii_case("zip") && single_disk(&mut anchor)? {
        ACTIVE.with(|paths| paths.borrow_mut().push(input));
        let _scope = Scope;
        return action(path);
    }
    let ctl = control(opts);
    let mut parts = Vec::new();
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("invalid ZIP volume name")?;
    for index in 1..=65536u32 {
        ctl.check()?;
        let part = if numeric {
            input.with_file_name(format!("{stem}.{index:03}"))
        } else {
            input.with_extension(format!("z{index:02}"))
        };
        match open_part(&part)? {
            Some(file) => parts.push((part, file)),
            None if index == 1 && !numeric => {
                // The ZIP handler also accepts a self-extracting first disk.
                if let Some(file) = open_part(&input.with_extension("exe"))? {
                    parts.push((input.with_extension("exe"), file));
                } else {
                    break;
                }
            }
            None => break,
        }
        if index == 65536 {
            return Err("too many ZIP volumes".into());
        }
    }
    if !numeric {
        let terminal = input.with_extension("zip");
        let file = open_part(&terminal)?.ok_or("missing final ZIP volume")?;
        parts.push((terminal, file));
    }
    let requested = input.file_name().ok_or("ZIP volume has no filename")?;
    if !parts.iter().any(|(p, _)| {
        p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case(requested))
    }) {
        return Err("ZIP volume is outside the contiguous family".into());
    }
    // The console opens siblings by name at several stages. Give it only the
    // captured family, with no names a concurrent source-directory writer can add.
    let stage = private_family(&input, &mut parts, &ctl)?;
    let fixed = stage.path().join(requested);
    ACTIVE.with(|paths| paths.borrow_mut().push(fixed.clone()));
    let _scope = Scope;
    action(fixed.to_str().ok_or("invalid ZIP snapshot path")?)
}

fn private_family(
    input: &Path,
    parts: &mut [(PathBuf, File)],
    ctl: &JobControl,
) -> Result<StagingDir, String> {
    let mut guards = CheckedDirectories::default();
    #[cfg(windows)]
    if let Some(parent) = input.parent() {
        // A private same-volume namespace avoids copying the compressed bytes.
        // Never change permissions/attributes on any hard link to the source.
        if parts
            .iter()
            .all(|(_, f)| f.metadata().is_ok_and(|m| !m.permissions().readonly()))
        {
            if let Ok(stage) = new_fallback_stage(parent, &mut guards) {
                let mut linked = true;
                for (part, pinned) in parts.iter() {
                    ctl.check()?;
                    let alias = stage
                        .path()
                        .join(part.file_name().ok_or("missing ZIP volume name")?);
                    if fs::hard_link(part, &alias).is_err() {
                        linked = false;
                        break;
                    }
                    let check = crate::engine_stream::open_source_leaf(&alias)
                        .map_err(|e| e.to_string())?;
                    if directory_identity(&check)? != directory_identity(pinned)? {
                        return Err("ZIP volume identity changed".into());
                    }
                }
                if linked {
                    return Ok(stage);
                }
            }
        }
    }
    let stage = new_fallback_stage(&std::env::temp_dir(), &mut guards)?;
    let mut scratch = vec![0u8; 1024 * 1024];
    for (part, file) in parts {
        let target = stage
            .path()
            .join(part.file_name().ok_or("missing ZIP volume name")?);
        let mut output = File::options()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|e| e.to_string())?;
        loop {
            ctl.check()?;
            let n = file.read(&mut scratch).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            output.write_all(&scratch[..n]).map_err(|e| e.to_string())?;
        }
    }
    Ok(stage)
}
#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::*;

    fn fixture(numeric: bool, readonly: bool) {
        let dir = tempfile::tempdir().unwrap();
        let names = if numeric {
            ["set.zip.001", "set.zip.002", "set.zip.003"]
        } else {
            ["set.z01", "set.z02", "set.zip"]
        };
        let next = if numeric { "set.zip.004" } else { "set.z03" };
        let first = dir.path().join(names[0]);
        for (i, name) in names.iter().enumerate() {
            let path = dir.path().join(name);
            fs::write(&path, [i as u8; 64]).unwrap();
            if readonly {
                let mut perms = fs::metadata(&path).unwrap().permissions();
                perms.set_readonly(true);
                fs::set_permissions(path, perms).unwrap();
            }
        }
        let source = if numeric {
            first
        } else {
            dir.path().join(names[2])
        };
        with_source(
            source.to_str().unwrap(),
            &EngineOptions::default(),
            |view| {
                let private = Path::new(view).parent().unwrap();
                assert_ne!(private, dir.path());
                for (i, name) in names.iter().enumerate() {
                    let original = dir.path().join(name);
                    assert!(File::options().write(true).open(&original).is_err());
                    assert!(fs::rename(&original, dir.path().join("moved")).is_err());
                    assert_eq!(fs::read(private.join(name)).unwrap(), vec![i as u8; 64]);
                }
                fs::write(dir.path().join(next), b"later").unwrap();
                assert!(!private.join(next).exists());
                with_source(view, &EngineOptions::default(), |nested| {
                    assert_eq!(nested, view);
                    Ok(())
                })
            },
        )
        .unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
        for (i, name) in names.iter().enumerate() {
            let path = dir.path().join(name);
            let mut perms = fs::metadata(&path).unwrap().permissions();
            assert_eq!(perms.readonly(), readonly);
            assert_eq!(fs::read(&path).unwrap(), vec![i as u8; 64]);
            if readonly {
                perms.set_readonly(false);
                fs::set_permissions(&path, perms).unwrap();
            }
            assert!(File::options().write(true).open(&path).is_ok());
        }
    }

    #[test]
    fn numbered_zip_view_excludes_late_parts_and_holds_the_whole_source() {
        fixture(true, false);
    }
    #[test]
    fn spanned_zip_view_excludes_late_parts_and_holds_the_whole_source() {
        fixture(false, false);
    }
    #[test]
    fn readonly_zip_volumes_keep_attributes_and_leave_no_temporary_links() {
        fixture(false, true);
    }
}
