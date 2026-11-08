use super::{
    directory_identity, rename_directory_noclobber_in, validate_transaction_root,
    CheckedDirectories,
};
use crate::{
    engine_stream::{pin, PinnedSource},
    private_stage::{self, PrivateStage},
};
use std::{
    ffi::c_void,
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};

#[link(name = "kernel32")]
extern "system" {
    fn ReplaceFileW(
        target: *const u16,
        replacement: *const u16,
        backup: *const u16,
        flags: u32,
        exclude: *mut c_void,
        reserved: *mut c_void,
    ) -> i32;
    fn GetFileInformationByHandleEx(
        handle: *mut c_void,
        class: i32,
        data: *mut u32,
        size: u32,
    ) -> i32;
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[derive(PartialEq, Eq)]
enum FileId {
    Extended(u64, [u8; 16]),
    Legacy(u32, u64),
}

fn id(handle: &File) -> Result<FileId, String> {
    #[repr(C)]
    struct Info {
        volume: u64,
        index: [u8; 16],
    }
    let mut info = Info {
        volume: 0,
        index: [0; 16],
    };
    if unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            18,
            (&mut info as *mut Info).cast(),
            std::mem::size_of::<Info>() as u32,
        )
    } != 0
        && info.index != [0; 16]
    {
        return Ok(FileId::Extended(info.volume, info.index));
    }
    let (volume, index) = directory_identity(handle)?;
    if index == 0 {
        return Err("The filesystem did not provide a stable archive identity".into());
    }
    Ok(FileId::Legacy(volume, index))
}

fn leaf(path: &Path, access: u32, share: u32) -> Result<File, String> {
    let file = File::options()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return Err("Archive is not a regular file".into());
    }
    Ok(file)
}

fn pin_directory(path: &Path) -> Result<File, String> {
    let handle = File::options()
        .access_mode(0x20081)
        .share_mode(3)
        .custom_flags(0x02200000)
        .open(path)
        .map_err(|e| e.to_string())?;
    validate_transaction_root(&handle, path)?;
    Ok(handle)
}

fn atomic_replace(target: &Path, replacement: &Path, backup: &Path) -> io::Result<()> {
    let (target, replacement, backup) = (wide(target), wide(replacement), wide(backup));
    if unsafe {
        ReplaceFileW(
            target.as_ptr(),
            replacement.as_ptr(),
            backup.as_ptr(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) struct EditOutput {
    output_pin: File,
    stage: PrivateStage,
    path: PathBuf,
}
impl EditOutput {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) struct EditSource {
    path: PathBuf,
    read: PinnedSource,
    bridge: File,
    parent: File,
    _ancestors: CheckedDirectories,
    identity: FileId,
}
impl EditSource {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("missing archive parent")?;
        let mut ancestors = CheckedDirectories::default();
        ancestors.allow_publication(parent)?;
        ancestors.check(parent)?;
        let parent_handle = pin_directory(parent)?;
        let read = pin(path)?;
        // A second handle keeps in-place writes denied during ReplaceFileW.
        // The first handle also denies renames until the verified output is ready.
        let bridge = leaf(path, 0x80000000, 5)?;
        let identity = id(&read)?;
        if id(&bridge)? != identity {
            return Err("Archive identity changed".into());
        }
        Ok(Self {
            path: path.to_path_buf(),
            read,
            bridge,
            parent: parent_handle,
            _ancestors: ancestors,
            identity,
        })
    }
    pub(crate) fn output(&self) -> Result<EditOutput, String> {
        let parent = self.path.parent().ok_or("missing archive parent")?;
        let stage = private_stage::create(parent, &self.parent)?;
        let output = stage.path().join("output");
        std::fs::create_dir(&output).map_err(|e| e.to_string())?;
        let path = output.join(self.path.file_name().ok_or("missing archive filename")?);
        let output_pin = pin_directory(&output)?;
        Ok(EditOutput {
            stage,
            path,
            output_pin,
        })
    }
    pub(super) fn direct_zip(&self, path: &Path) -> Result<bool, String> {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        {
            return Ok(false);
        }
        let mut magic = [0; 4];
        let count = self
            .read
            .seek_read(&mut magic, 0)
            .map_err(|e| e.to_string())?;
        Ok(count == 4 && matches!(&magic, b"PK\x03\x04" | b"PK\x05\x06"))
    }

    pub(crate) fn commit(self, output: EditOutput) -> Result<(), String> {
        self.commit_with(output, || {}, || {})
    }
    fn commit_with(
        self,
        output: EditOutput,
        before_replace: impl FnOnce(),
        after_replace: impl FnOnce(),
    ) -> Result<(), String> {
        let EditOutput {
            stage,
            path: staged,
            output_pin,
        } = output;
        let path = &self.path;
        let staged = &staged;
        let recovery_dir = stage.path().join("recovery");
        std::fs::create_dir(&recovery_dir).map_err(|e| e.to_string())?;
        validate_transaction_root(
            &output_pin,
            staged.parent().ok_or("missing private output parent")?,
        )?;
        let recovery_pin = pin_directory(&recovery_dir)?;
        let backup = recovery_dir.join("previous-archive");
        let proposed = leaf(staged, 0, 7)?;
        let proposed_id = id(&proposed)?;
        // Do not close the write-denying bridge until the displaced object has
        // been identified. This prevents an undetectable in-place writer.
        drop(self.read);
        let current = leaf(path, 0, 7)?;
        if id(&current)? != self.identity {
            return Err("Archive changed before commit; edit cancelled".into());
        }
        drop(current);
        before_replace();
        let swapped = atomic_replace(path, staged, &backup);
        after_replace();
        if let Err(e) = swapped {
            if !matches!(backup.try_exists(), Ok(false)) {
                // Some ReplaceFileW errors occur after the original was moved.
                // Only restore into a vacant name. Keep every file on failure.
                let _ = restore_if_absent(&backup, path, &self.parent);
                return Err(recovery(
                    stage,
                    path,
                    &format!("Archive replacement failed: {e}"),
                ));
            }
            return Err(e.to_string());
        }
        // The backup denotes what Windows actually replaced, including a path
        // replacement that won the race immediately before the system call.
        let displaced = match leaf(&backup, 0, 7).and_then(|f| id(&f)) {
            Ok(identity) => identity,
            Err(reason) => return Err(recovery(stage, path, &reason)),
        };
        if displaced != self.identity {
            // Move only our published object, through a pinned DELETE handle.
            // If anyone changed its bytes, that version is preserved too. If
            // anyone replaced it again, the newer object stays at the name.
            if let Ok(current) = leaf(path, 0x10080, 1) {
                if id(&current).ok() == Some(proposed_id) {
                    drop(proposed);
                    let kept = recovery_dir.join("edited-result");
                    let _ = rename_directory_noclobber_in(&current, &kept, Some(&recovery_pin));
                }
            }
            let restored = restore_if_absent(&backup, path, &self.parent);
            let reason = if restored {
                "Concurrent archive replacement detected; external archive restored"
            } else {
                "Concurrent archive replacement detected; destination retained"
            };
            return Err(recovery(stage, path, reason));
        }
        drop(self.bridge);
        Ok(())
    }
}

fn restore_if_absent(backup: &Path, path: &Path, parent: &File) -> bool {
    let Ok(file) = leaf(backup, 0x10080, 1) else {
        return false;
    };
    rename_directory_noclobber_in(&file, path, Some(parent)).unwrap_or(false)
}

fn recovery(stage: PrivateStage, path: &Path, reason: &str) -> String {
    let kept: PathBuf = stage.keep();
    format!(
        "{reason}. Archive: {}. Preserved recovery directory: {}",
        path.display(),
        kept.display()
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn replacing_writers_and_in_place_writers_are_denied_while_preparing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.zip");
        fs::write(&path, b"original").unwrap();
        let incoming = temp.path().join("incoming");
        fs::write(&incoming, b"external").unwrap();
        let edit = EditSource::open(&path).unwrap();
        assert!(File::options().write(true).open(&path).is_err());
        assert!(atomic_replace(&path, &incoming, &temp.path().join("displaced")).is_err());
        let output = edit.output().unwrap();
        fs::write(output.path(), b"proposal").unwrap();
        edit.commit_with(
            output,
            || {
                assert!(File::options().write(true).open(&path).is_err());
            },
            || {},
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"proposal");
    }

    #[test]
    fn preserve_external_update_even_at_final_replace_boundary() {
        for second_writer in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let parent = temp.path();
            let path = parent.join("sample.zip");
            fs::write(&path, b"old bytes").unwrap();
            let edit = EditSource::open(&path).unwrap();
            let output = edit.output().unwrap();
            let recovery = output.stage.path().to_owned();
            fs::write(output.path(), b"our proposed bytes").unwrap();
            let second = parent.join("replacement");
            fs::write(&second, b"first external bytes").unwrap();
            let third = parent.join("newest");
            fs::write(&third, b"second external bytes").unwrap();
            let err = edit
                .commit_with(
                    output,
                    || atomic_replace(&path, &second, &parent.join("old-displaced")).unwrap(),
                    || {
                        if second_writer {
                            atomic_replace(&path, &third, &parent.join("proposal-displaced"))
                                .unwrap();
                        }
                    },
                )
                .unwrap_err();
            assert!(err.contains(&recovery.display().to_string()), "{err}");
            let public = fs::read(&path).unwrap();
            if second_writer {
                assert_eq!(public, b"second external bytes");
                assert_eq!(
                    fs::read(recovery.join("recovery/previous-archive")).unwrap(),
                    b"first external bytes"
                );
            } else {
                assert_eq!(public, b"first external bytes", "{err}");
                assert_eq!(
                    fs::read(recovery.join("recovery/edited-result")).unwrap(),
                    b"our proposed bytes"
                );
            }
        }
    }
}
