#[path = "edit_recovery.rs"]
mod journal;
use super::{
    directory_identity, rename_handle_noclobber_in, validate_transaction_root, CheckedDirectories,
};
use crate::{
    engine_stream::{pin, PinnedSource},
    private_stage::{self, PrivateStage},
};
pub(super) use journal::recover_missing;
use std::{
    ffi::c_void,
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
};

#[link(name = "kernel32")]
extern "system" {
    fn ReOpenFile(file: *mut c_void, access: u32, sharing: u32, flags: u32) -> *mut c_void;
    fn GetFinalPathNameByHandleW(file: *mut c_void, path: *mut u16, size: u32, flags: u32) -> u32;
    fn GetVolumeInformationByHandleW(
        file: *mut c_void,
        volume: *mut u16,
        volume_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        filesystem: *mut u16,
        filesystem_len: u32,
    ) -> i32;
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

fn wide_name(path: &Path) -> io::Result<Vec<u16>> {
    // ReplaceFileW does not automatically extend long Win32 paths like std::fs.
    // Resolve the pinned parent only; never follow a replaceable leaf link.
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing replacement parent"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing replacement filename"))?;
    let absolute = parent.canonicalize()?.join(name);
    Ok(absolute.as_os_str().encode_wide().chain(Some(0)).collect())
}

#[derive(Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
enum FileId {
    Extended(u64, [u8; 16]),
    Legacy(u32, u64),
    Contents([u8; 32]),
    DirectoryName(Vec<u16>),
}

fn reopen(file: &File, access: u32, share: u32) -> Result<File, String> {
    let raw = unsafe { ReOpenFile(file.as_raw_handle(), access, share, 0x00200000) };
    if raw as isize == -1 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(unsafe { File::from_raw_handle(raw) })
}

fn actual_name(file: &File) -> Result<Vec<u16>, String> {
    let size =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
    if size == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let mut name = vec![0; size as usize + 1];
    let used = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            name.as_mut_ptr(),
            name.len() as u32,
            0,
        )
    };
    if used == 0 || used as usize >= name.len() {
        return Err("Cannot identify archive name".into());
    }
    name.truncate(used as usize);
    Ok(name)
}

fn id(handle: &File) -> Result<FileId, String> {
    id_checked(handle, None)
}

fn id_checked(handle: &File, control: Option<&super::JobControl>) -> Result<FileId, String> {
    if let Some(control) = control {
        control.check()?;
    }
    let mut fs = [0u16; 32];
    let volume_ok = unsafe {
        GetVolumeInformationByHandleW(
            handle.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    let fs = String::from_utf16_lossy(&fs[..fs.iter().position(|x| *x == 0).unwrap_or(fs.len())]);
    if volume_ok == 0 || !(fs.eq_ignore_ascii_case("NTFS") || fs.eq_ignore_ascii_case("ReFS")) {
        if handle.metadata().map_err(|e| e.to_string())?.is_dir() {
            return actual_name(handle).map(FileId::DirectoryName);
        }
        // FAT directory-entry IDs can change when a file is renamed. Preserve
        // payload identity across the two private names without depending on it.
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut file = reopen(handle, 0x80000000, 7)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            if let Some(control) = control {
                control.check()?;
            }
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        return Ok(FileId::Contents(hasher.finalize().into()));
    }
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
    let (target, replacement, backup) = (
        wide_name(target)?,
        wide_name(replacement)?,
        wide_name(backup)?,
    );
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

#[cfg(test)]
thread_local! {
    static BEFORE_PUBLIC:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=std::cell::RefCell::new(None);
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
    original_name: Vec<u16>,
    read: PinnedSource,
    bridge: File,
    parent: File,
    _ancestors: CheckedDirectories,
    control: super::JobControl,
}
impl EditSource {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        Self::open_controlled(path, &super::JobControl::new())
    }
    pub(crate) fn open_controlled(
        path: &Path,
        control: &super::JobControl,
    ) -> Result<Self, String> {
        control.check()?;
        let parent = path.parent().ok_or("missing archive parent")?;
        let mut ancestors = CheckedDirectories::default();
        ancestors.allow_publication(parent)?;
        ancestors.check(parent)?;
        let parent_handle = pin_directory(parent)?;
        let read = pin(path)?;
        // A second handle keeps in-place writes denied during ReplaceFileW.
        // The first handle also denies renames until the verified output is ready.
        let bridge = reopen(&read, 0x80000000, 5)?;
        let original_name = actual_name(&read)?;
        Ok(Self {
            path: path.to_path_buf(),
            original_name,
            read,
            bridge,
            parent: parent_handle,
            _ancestors: ancestors,
            control: control.clone(),
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
        // ReplaceFileW may briefly vacate its target and then overwrite a file
        // created in that gap. Only use it between two private, pinned names.
        let name = path.file_name().ok_or("missing archive filename")?;
        let merge_dir = stage.path().join("merge");
        let recovery_dir = stage.path().join("original");
        std::fs::create_dir(&merge_dir).map_err(|e| e.to_string())?;
        std::fs::create_dir(&recovery_dir).map_err(|e| e.to_string())?;
        let merge_parent = pin_directory(&merge_dir)?;
        let recovery_parent = pin_directory(&recovery_dir)?;
        let merge_path = merge_dir.join(name);
        let backup = recovery_dir.join(name);
        validate_transaction_root(
            &output_pin,
            staged.parent().ok_or("missing private output parent")?,
        )?;
        let permissions = crate::private_stage::EditPermissions::capture(&self.read)?;
        let proposed = leaf(staged, 0, 7)?;
        let identity = id_checked(&self.read, Some(&self.control))?;
        let proposed_id = id_checked(&proposed, Some(&self.control))?;
        let intent = journal::Intent::write(
            &stage,
            &self.parent,
            path,
            &identity,
            &proposed_id,
            permissions.as_ref(),
        )?;

        // The read handle used by Nana must not request DELETE. A second
        // handle continues to deny writes during the handoff. If a competing
        // rename wins in that interval, leave its replacement untouched.
        drop(self.read);
        before_replace();
        let original = reopen(&self.bridge, 0x10080, 1)?;
        if actual_name(&original)? != self.original_name
            || id_checked(&leaf(path, 0, 7)?, Some(&self.control))? != identity
        {
            return Err("Archive changed before commit; edit cancelled".into());
        }
        self.control.check()?;
        if !rename_handle_noclobber_in(&original, &merge_path, Some(&merge_parent))? {
            return Err("Could not claim the original archive for editing".into());
        }
        // After this point an unwind must not recursively remove the original.
        let mut recovery = PreserveOnDrop(Some(stage));
        drop(original);
        after_replace();
        let publish: Result<(), String> = (|| {
            self.control.check()?;
            atomic_replace(&merge_path, staged, &backup).map_err(|e| e.to_string())?;
            let candidate = leaf(&merge_path, 0x10080, 1)?;
            if id_checked(&candidate, Some(&self.control))? != proposed_id {
                return Err("Private replacement archive changed".into());
            }
            drop(proposed);
            self.control.check()?;
            let scratch = scratch_path(recovery.path(), path, false)?;
            publish_with_live_permissions(
                &candidate,
                &merge_path,
                &merge_parent,
                path,
                &self.parent,
                &scratch,
                permissions.as_ref(),
                || {
                    #[cfg(test)]
                    BEFORE_PUBLIC.with(|slot| {
                        if let Some(hook) = slot.borrow_mut().take() {
                            hook();
                        }
                    });
                    self.control.check()
                },
            )?;
            Ok(())
        })();
        drop(self.bridge);
        if let Err(reason) = publish {
            // ReplaceFileW can fail with the original at either private name.
            // Restore by verified handle, and only into a vacant public name.
            let original_scratch = scratch_path(recovery.path(), path, true)?;
            let mut restored = false;
            for (saved, saved_parent) in [(&backup, &recovery_parent), (&merge_path, &merge_parent)]
            {
                if let Ok(handle) = leaf(saved, 0x10080, 1) {
                    if id(&handle).ok().as_ref() == Some(&identity)
                        && publish_with_live_permissions(
                            &handle,
                            saved,
                            saved_parent,
                            path,
                            &self.parent,
                            &original_scratch,
                            permissions.as_ref(),
                            || Ok(()),
                        )
                        .is_ok()
                    {
                        restored = true;
                        break;
                    }
                }
            }
            if restored {
                let scratch = scratch_path(recovery.path(), path, false)?;
                if std::fs::symlink_metadata(&scratch).is_ok() {
                    intent.disarm();
                    return Err(format!(
                        "{reason}. Original archive restored. Check {} and {}",
                        scratch.display(),
                        recovery.path().display()
                    ));
                }
                drop(recovery_parent);
                drop(merge_parent);
                drop(output_pin);
                intent.disarm();
                recovery.finished();
                return Err(format!("{reason}. Original archive restored."));
            }
            if std::fs::symlink_metadata(path).is_ok() {
                intent.disarm();
            }
            return Err(format!("{reason}. Archive: {}. Private recovery directory (original filename preserved): {}",
                path.display(), recovery.path().display()));
        }
        drop(recovery_parent);
        drop(merge_parent);
        drop(output_pin);
        intent.disarm();
        recovery.finished();
        Ok(())
    }
}

fn scratch_path(stage: &Path, target: &Path, original: bool) -> Result<PathBuf, String> {
    let id = stage
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix(".qing-zip-stage-"))
        .ok_or("invalid edit stage name")?;
    let kind = if original { "original" } else { "result" };
    Ok(target
        .parent()
        .ok_or("missing edit parent")?
        .join(format!(".qing-zip-edit-{id}-{kind}")))
}

#[allow(clippy::too_many_arguments)]
fn publish_with_live_permissions(
    file: &File,
    previous: &Path,
    previous_parent: &File,
    target: &Path,
    target_parent: &File,
    scratch: &Path,
    security: Option<&crate::private_stage::EditPermissions>,
    before_final: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if std::fs::symlink_metadata(target).is_ok() {
        return Err("Archive name is occupied".into());
    }
    if let Some(security) = security {
        let acl_handle = reopen(file, 0x60000, 7)?;
        security.protect(&acl_handle)?;
        if previous != scratch && !rename_handle_noclobber_in(file, scratch, Some(target_parent))? {
            return Err("Could not reserve the temporary archive name".into());
        }
        let result = (|| {
            security.inherit(&acl_handle)?;
            before_final()?;
            if !rename_handle_noclobber_in(file, target, Some(target_parent))? {
                return Err("Archive name is occupied or publication was denied".into());
            }
            Ok(())
        })();
        if let Err(reason) = result {
            security.protect(&acl_handle)?;
            if previous != scratch
                && !rename_handle_noclobber_in(file, previous, Some(previous_parent))?
            {
                return Err(format!(
                    "{reason}. Private candidate retained at {}",
                    scratch.display()
                ));
            }
            return Err(reason);
        }
    } else {
        before_final()?;
        if !rename_handle_noclobber_in(file, target, Some(target_parent))? {
            return Err("Archive name is occupied or publication was denied".into());
        }
    }
    Ok(())
}

// The transaction owns user data between claiming the old name and a verified
// publication or rollback. Losing the process in that interval leaves the
// private recovery directory and its original archive on disk.
struct PreserveOnDrop(Option<PrivateStage>);
impl PreserveOnDrop {
    fn path(&self) -> &Path {
        self.0.as_ref().expect("active edit recovery").path()
    }
    fn finished(&mut self) {
        drop(self.0.take());
    }
}
impl Drop for PreserveOnDrop {
    fn drop(&mut self) {
        if let Some(stage) = self.0.take() {
            stage.keep();
        }
    }
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
    fn cancelling_live_direct_edit_keeps_archive_and_alternate_streams() {
        use std::{
            io::Write,
            time::{Duration, Instant},
        };
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cancel.zip");
        let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("keep.bin", options).unwrap();
        let data = vec![b'x'; 1024 * 1024];
        for _ in 0..64 {
            writer.write_all(&data).unwrap();
        }
        writer.start_file("old.txt", options).unwrap();
        writer.write_all(b"keep old spelling").unwrap();
        drop(writer.finish().unwrap());
        let mark = PathBuf::from(format!("{}:Zone.Identifier", path.display()));
        fs::write(&mark, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        let original_id = id(&File::open(&path).unwrap()).unwrap();
        let ctl = super::super::JobControl::new();
        std::thread::scope(|scope| {
            let job = scope.spawn(|| {
                super::super::update_with_control(
                    path.to_str().unwrap(),
                    "rename",
                    &["old.txt".into(), "new.txt".into()],
                    &Default::default(),
                    &ctl,
                )
            });
            let deadline = Instant::now() + Duration::from_secs(20);
            while ctl.progress.lock().unwrap().phase != "updating" {
                assert!(
                    Instant::now() < deadline && !job.is_finished(),
                    "did not observe an in-flight native edit"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            ctl.cancel();
            assert!(job.join().unwrap().is_err());
        });
        assert!(id(&File::open(&path).unwrap()).unwrap() == original_id);
        let mut archive = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.len(), 2);
        assert!(archive.by_name("old.txt").is_ok());
        assert!(archive.by_name("new.txt").is_err());
        assert_eq!(fs::read(mark).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    fn powershell(path: &Path, script: &str) -> String {
        let run = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("QING_TEST_ACL", path)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        String::from_utf8_lossy(&run.stdout).trim().to_string()
    }
    fn install_reader(path: &Path, protected: bool) {
        let protect = if protected {
            "$acl.SetAccessRuleProtection($true,$true);"
        } else {
            ""
        };
        let script=format!("$ErrorActionPreference='Stop';$p=$env:QING_TEST_ACL;$acl=Get-Acl -LiteralPath $p;{protect}$sid=[System.Security.Principal.SecurityIdentifier]::new('S-1-5-32-546');$inherit=if([IO.Directory]::Exists($p)){{[Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit'}}else{{[Security.AccessControl.InheritanceFlags]::None}};$rule=[Security.AccessControl.FileSystemAccessRule]::new($sid,[Security.AccessControl.FileSystemRights]::Read,$inherit,[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Allow);$acl.AddAccessRule($rule);Set-Acl -LiteralPath $p -AclObject $acl");
        powershell(path, &script);
    }
    fn permissions(path: &Path) -> String {
        powershell(path,"$a=Get-Acl -LiteralPath $env:QING_TEST_ACL; $a.GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]::Access)")
    }

    #[test]
    fn late_parent_revocation_matches_normal_file_and_preserves_explicit_or_protected_access() {
        for (explicit, protected) in [(false, false), (true, false), (true, true)] {
            let temp = tempfile::tempdir().unwrap();
            let parent = temp.path().join("destination");
            fs::create_dir(&parent).unwrap();
            install_reader(&parent, true);
            let archive = parent.join("source.zip");
            fs::write(&archive, b"old").unwrap();
            let peer = parent.join("reference.zip");
            fs::write(&peer, b"reference").unwrap();
            if explicit {
                install_reader(&archive, protected);
                install_reader(&peer, protected);
            }
            let before = permissions(&archive);
            let edit = EditSource::open(&archive).unwrap();
            let output = edit.output().unwrap();
            fs::write(output.path(), b"updated").unwrap();
            let scope = parent.clone();
            BEFORE_PUBLIC.with(|slot|*slot.borrow_mut()=Some(Box::new(move||{
                powershell(&scope,"$ErrorActionPreference='Stop';$p=$env:QING_TEST_ACL;$acl=Get-Acl -LiteralPath $p;$null=& icacls.exe $p '/remove:g' '*S-1-5-32-546';if($LASTEXITCODE){throw 'parent DACL revoke failed'}");
            })));
            edit.commit(output).unwrap();
            let after = permissions(&archive);
            let expected = permissions(&peer);
            let fresh = parent.join("new-file");
            fs::write(&fresh, b"fresh").unwrap();
            let native_new = permissions(&fresh);
            assert!(before.contains(";;;BG)"));
            assert!(!native_new.contains(";;;BG)"));
            assert_eq!(after, expected);
            if !explicit {
                assert_eq!(after, native_new);
            }
            assert_eq!(fs::read(&archive).unwrap(), b"updated");
        }
    }

    #[test]
    fn cancellation_after_claim_rolls_back_without_publishing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("original.zip");
        fs::write(&path, b"original contents").unwrap();
        let control = super::super::JobControl::new();
        let edit = EditSource::open_controlled(&path, &control).unwrap();
        let output = edit.output().unwrap();
        fs::write(output.path(), b"cancelled proposal").unwrap();
        let result = edit.commit_with(output, || {}, || control.cancel());
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"original contents");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn framework_child_pauses_with_original_in_recovery() {
        let Some(root) = std::env::var_os("QING_TEST_INTERRUPTED_EDIT") else {
            return;
        };
        let root = PathBuf::from(root);
        let edit = EditSource::open(&root.join("archive.zip")).unwrap();
        let output = edit.output().unwrap();
        fs::copy(root.join("proposal.zip"), output.path()).unwrap();
        fn pause(path: &Path) {
            fs::write(path.join("ready-to-kill"), b"claimed").unwrap();
            loop {
                std::thread::park();
            }
        }
        let public_scratch = std::env::var_os("QING_TEST_PUBLIC_SCRATCH").is_some();
        if public_scratch {
            let copy = root.clone();
            BEFORE_PUBLIC.with(|slot| *slot.borrow_mut() = Some(Box::new(move || pause(&copy))));
        }
        let _ = edit.commit_with(
            output,
            || {},
            || {
                if !public_scratch {
                    pause(&root)
                }
            },
        );
    }

    #[test]
    fn terminated_edit_recovers_only_its_verified_original_and_never_an_external_namesake() {
        use std::{
            io::Write,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        for mode in [
            "list",
            "gui-path",
            "external",
            "tampered-original",
            "public-scratch",
            "parent-revoked",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            if mode == "parent-revoked" {
                install_reader(root, true);
            }
            let path = root.join("archive.zip");
            for (name, file) in [
                ("original-member", &path),
                ("proposed-member", &root.join("proposal.zip")),
            ] {
                let mut zip = zip::ZipWriter::new(File::create(file).unwrap());
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(name.as_bytes()).unwrap();
                drop(zip.finish().unwrap());
            }
            let original = fs::read(&path).unwrap();
            let marker = PathBuf::from(format!("{}:Zone.Identifier", path.display()));
            fs::write(&marker, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            if mode == "public-scratch" {
                cmd.env("QING_TEST_PUBLIC_SCRATCH", "yes");
            }
            let mut child = cmd
                .args([
                    "--exact",
                    "engine::edit_source::tests::framework_child_pauses_with_original_in_recovery",
                    "--nocapture",
                ])
                .env("QING_TEST_INTERRUPTED_EDIT", root)
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            let start = Instant::now();
            while !root.join("ready-to-kill").is_file() && start.elapsed() < Duration::from_secs(12)
            {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(3));
            }
            let ready = root.join("ready-to-kill").is_file();
            if ready {
                assert!(!path.exists());
                assert!(
                    !recover_missing(&path).unwrap(),
                    "must not interrupt a live commit"
                );
            }
            let _ = child.kill();
            let _ = child.wait();
            assert!(ready, "child did not reach the commit boundary");
            if mode == "parent-revoked" {
                powershell(root,"$null=& icacls.exe $env:QING_TEST_ACL '/remove:g' '*S-1-5-32-546';if($LASTEXITCODE){throw 'parent revoke failed'}");
            }
            if mode == "external" {
                fs::copy(root.join("proposal.zip"), &path).unwrap();
                assert!(!recover_missing(&path).unwrap());
                assert_eq!(
                    fs::read(&path).unwrap(),
                    fs::read(root.join("proposal.zip")).unwrap()
                );
            } else if mode == "tampered-original" {
                let stage = fs::read_dir(root)
                    .unwrap()
                    .flatten()
                    .find(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(".qing-zip-stage-")
                    })
                    .unwrap()
                    .path();
                let saved = stage.join("merge/archive.zip");
                fs::rename(&saved, root.join("true-original")).unwrap();
                fs::copy(root.join("proposal.zip"), &saved).unwrap();
                assert!(!recover_missing(&path).unwrap());
                assert!(!path.exists());
            } else {
                if mode == "list" {
                    let entries = crate::operations::list_archive(
                        path.to_str().unwrap(),
                        &Default::default(),
                    )
                    .unwrap();
                    assert!(entries.iter().any(|e| e.name == "original-member"));
                } else {
                    crate::engine_stream::canonical_input_path(&path, false).unwrap();
                }
                if mode == "public-scratch" {
                    assert!(!fs::read_dir(root).unwrap().flatten().any(|e| e
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".qing-zip-edit-")));
                }
                if mode == "parent-revoked" {
                    let peer = root.join("fresh");
                    fs::write(&peer, b"check").unwrap();
                    assert!(!permissions(&peer).contains(";;;BG)"));
                    assert_eq!(permissions(&path), permissions(&peer));
                }
                assert_eq!(fs::read(&path).unwrap(), original);
                assert_eq!(fs::read(marker).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
            }
        }
    }

    #[test]
    fn replacement_at_handoff_and_publication_is_never_overwritten() {
        for at_publication in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let parent = temp.path();
            let path = parent.join("sample.zip");
            fs::write(&path, b"old bytes").unwrap();
            let edit = EditSource::open(&path).unwrap();
            let output = edit.output().unwrap();
            let recovery = output.stage.path().to_owned();
            fs::write(output.path(), b"our proposed bytes").unwrap();
            let incoming = parent.join("external");
            fs::write(&incoming, b"external bytes").unwrap();
            let err = edit
                .commit_with(
                    output,
                    || {
                        if !at_publication {
                            atomic_replace(&path, &incoming, &parent.join("external-backup"))
                                .unwrap();
                        }
                    },
                    || {
                        if at_publication {
                            fs::rename(&incoming, &path).unwrap();
                        }
                    },
                )
                .unwrap_err();
            assert_eq!(fs::read(&path).unwrap(), b"external bytes");
            if at_publication {
                assert!(err.contains(&recovery.display().to_string()), "{err}");
                assert_eq!(
                    fs::read(recovery.join("original/sample.zip")).unwrap(),
                    b"old bytes"
                );
                assert_eq!(
                    fs::read(recovery.join("merge/sample.zip")).unwrap(),
                    b"our proposed bytes"
                );
            } else {
                assert!(!recovery.exists());
            }
        }
    }
}
