//! Reuse only the common prefix of existing, deny-write/delete directory pins.
//! Each child is opened relative to its verified parent. Unusual namespaces and
//! directories that cannot be pinned keep the original full-path validation.
use std::{
    ffi::{c_void, OsStr, OsString},
    fs::{File, Metadata},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Component, Path, Prefix},
};

#[link(name = "kernel32")]
extern "system" {
    fn GetFileInformationByHandleEx(
        handle: *mut c_void,
        class: i32,
        info: *mut u32,
        size: u32,
    ) -> i32;
    fn GetFinalPathNameByHandleW(
        handle: *mut c_void,
        path: *mut u16,
        length: u32,
        flags: u32,
    ) -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

fn ordinary_directory(file: &File) -> Result<bool, String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if meta.file_attributes() & 0x400 != 0 {
        return Err("linked checksum directory rejected".into());
    }
    // A case-sensitive directory must retain the original Win32 lookup behavior.
    let mut flags = 0;
    Ok(meta.is_dir()
        && unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), 23, &mut flags, 4) != 0 }
        && flags == 0)
}

fn final_name(file: &File, flags: u32) -> Option<String> {
    // A qualifying input has at most 247 UTF-16 units; larger or unnameable
    // results continue through the general path, including codepage issues.
    let mut buf = [0u16; 260];
    let n = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buf.as_mut_ptr(), 260, flags) }
        as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    String::from_utf16(&buf[..n]).ok()
}

fn matches_spelling(file: &File, path: &Path) -> bool {
    // A hard link or differently cased/short alias is never equated solely
    // by text. If Windows did not resolve this spelling, keep full validation.
    final_name(file, 0)
        .as_deref()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .map(Path::new)
        == Some(path)
}

#[derive(Default)]
pub(super) struct SharedParents {
    root: Option<(u8, String, File)>,
    parents: Vec<(OsString, File)>,
}

impl SharedParents {
    pub(super) fn open(&mut self, raw: &str) -> Result<Option<(File, Metadata)>, String> {
        let result = self.try_open(raw)?;
        if result.is_none() {
            self.parents.clear();
            self.root = None;
        }
        Ok(result)
    }

    fn try_open(&mut self, raw: &str) -> Result<Option<(File, Metadata)>, String> {
        if raw.is_empty() || raw.contains(['\r', '\n', '\0']) {
            return Ok(None);
        }
        let path = std::path::absolute(raw).map_err(|e| e.to_string())?;
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return Ok(None);
        };
        let Prefix::Disk(letter) = prefix.kind() else {
            return Ok(None);
        };
        if crate::engine::attribute_query_path(&path).is_none()
            || unsafe { GetDriveTypeW([letter as u16, 58, 92, 0].as_ptr()) } != 3
        {
            return Ok(None);
        }
        let root = path.ancestors().last().unwrap();
        // Drive letters can be remapped. Obtain the root identity afresh before
        // accepting any prefix retained from the preceding input.
        let Ok(anchor) = File::options()
            .access_mode(0x81)
            .share_mode(1)
            .custom_flags(0x02200000)
            .open(root)
        else {
            return Ok(None);
        };
        let Some(volume) = final_name(&anchor, 1) else {
            return Ok(None);
        };
        // Only a physical volume root, excluding SUBST paths to subdirectories.
        // Comparing its mounted GUID also distinguishes disks with copied serials.
        if volume.len() != 49 || !volume.starts_with(r"\\?\Volume{") || !volume.ends_with(r"}\") {
            return Ok(None);
        }
        if self
            .root
            .as_ref()
            .is_none_or(|(drive, name, _)| *drive != letter || *name != volume)
        {
            if !ordinary_directory(&anchor)? {
                return Ok(None);
            }
            self.parents.clear();
            self.root = Some((letter, volume, anchor));
        }
        let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let names: Vec<_> = relative.iter().collect();
        let Some((leaf, dirs)) = names.split_last() else {
            return Ok(None);
        };
        let shared = self
            .parents
            .iter()
            .zip(dirs.iter())
            .take_while(|((name, _), item)| name.as_os_str() == **item)
            .count();
        self.parents.truncate(shared);
        // Check the same retained objects again before reusing their names.
        // No subsequent open can escape through a newly inserted path reparse.
        for (_, file) in &self.parents {
            if crate::engine::directory_attributes(file)? & 0x410 != 0x10 {
                return Err("linked checksum directory rejected".into());
            }
        }
        let root_file = &self.root.as_ref().unwrap().2;
        if crate::engine::directory_attributes(root_file)? & 0x410 != 0x10 {
            return Err("linked checksum root rejected".into());
        }
        for &part in &dirs[shared..] {
            let parent = self.parents.last().map_or(root_file, |(_, file)| file);
            let Ok(file) = crate::private_stage::open_checksum_child(parent, part, true) else {
                return Ok(None);
            };
            if !ordinary_directory(&file)? {
                return Ok(None);
            }
            self.parents.push((OsStr::to_owned(part), file));
        }
        let parent = self.parents.last().map_or(root_file, |(_, file)| file);
        let Ok(file) = crate::private_stage::open_checksum_child(parent, leaf, false) else {
            return Ok(None);
        };
        let before = file.metadata().map_err(|e| e.to_string())?;
        if !before.is_file() || before.file_attributes() & 0x400 != 0 {
            return Err("checksums require a regular file without reparse points".into());
        }
        if !matches_spelling(&file, &path) {
            return Ok(None);
        }
        Ok(Some((file, before)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn eligible_tempdir() -> tempfile::TempDir {
        // Hosted Windows uses a short name in TEMP. That input correctly takes
        // the fallback. Exercise the optimized route in the checkout instead.
        let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
        let plain = workspace.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
        tempfile::tempdir_in(plain).unwrap()
    }

    #[test]
    fn cached_source_keeps_leaf_and_ancestors_pinned() {
        let tmp = eligible_tempdir();
        let dir = tmp.path().join("folder");
        fs::create_dir(&dir).unwrap();
        let leaf = dir.join("file.txt");
        fs::write(&leaf, b"contents").unwrap();
        let mut pins = SharedParents::default();
        let (file, before) = pins
            .open(leaf.to_str().unwrap())
            .unwrap()
            .expect("regular local path");
        assert_eq!(before.len(), 8);
        assert!(File::options().write(true).open(&leaf).is_err());
        assert!(fs::rename(&leaf, dir.join("moved")).is_err());
        assert!(fs::rename(&dir, tmp.path().join("moved")).is_err());
        drop(file);
        assert!(pins.open(leaf.to_str().unwrap()).unwrap().is_some());
        drop(pins);
        fs::rename(&dir, tmp.path().join("moved")).unwrap();
    }
    #[test]
    fn public_batch_rejects_nested_junction_after_a_cached_sibling() {
        let tmp = eligible_tempdir();
        let good = tmp.path().join("good");
        let outside = tmp.path().join("outside");
        fs::create_dir(&good).unwrap();
        fs::create_dir(&outside).unwrap();
        let plain = good.join("ordinary.txt");
        fs::write(&plain, b"allowed").unwrap();
        fs::write(outside.join("external.txt"), b"external").unwrap();
        let link = good.join("alias");
        let created = std::process::Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(created.status.success());
        let mut paths = vec![plain.to_str().unwrap().to_owned(); 7];
        paths.push(link.join("external.txt").to_str().unwrap().to_owned());
        let result = super::super::checksum_files(&paths, &["SHA256".into()], &Default::default());
        fs::remove_dir(&link).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn public_batch_does_not_publish_partial_results_after_a_leaf_writer_or_cancel() {
        let tmp = eligible_tempdir();
        let file = tmp.path().join("payload");
        let blocked = tmp.path().join("blocked");
        fs::write(&file, vec![0u8; 4 * 1024 * 1024]).unwrap();
        fs::write(&blocked, b"other").unwrap();
        let mut paths = vec![file.to_str().unwrap().to_owned(); 16];
        let selection = vec!["SHA256".into()];
        let _writer = File::options()
            .write(true)
            .share_mode(7)
            .open(&blocked)
            .unwrap();
        paths.push(blocked.to_str().unwrap().to_owned());
        assert!(super::super::checksum_files(&paths, &selection, &Default::default()).is_err());

        paths.pop();
        let ctl = super::super::JobControl::new();
        let opts = super::super::EngineOptions {
            control: Some(ctl.clone()),
            ..Default::default()
        };
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| super::super::checksum_files(&paths, &selection, &opts));
            while !worker.is_finished() {
                if ctl.snapshot().percent.is_some_and(|percent| percent > 0) {
                    ctl.cancel();
                    break;
                }
                std::thread::yield_now();
            }
            assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
        });
    }
    #[test]
    fn existing_directory_writer_uses_ordinary_pins() {
        let tmp = eligible_tempdir();
        let dir = tmp.path().join("writable");
        fs::create_dir(&dir).unwrap();
        let leaf = dir.join("file");
        fs::write(&leaf, b"data").unwrap();
        let _writer = File::options()
            .access_mode(2)
            .share_mode(7)
            .custom_flags(0x02000000)
            .open(&dir)
            .unwrap();
        let mut pins = SharedParents::default();
        assert!(pins.open(leaf.to_str().unwrap()).unwrap().is_none());
        super::super::HashInput::open(leaf.to_str().unwrap()).unwrap();
    }
}
