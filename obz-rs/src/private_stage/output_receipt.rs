use super::*;
use sha2::{Digest, Sha256};

/// Keeps a just-created output on its original exclusive data handle. Default
/// stream writes, replacement and deletion cannot race rollback. Other streams,
/// attributes, owner/group and DACL can still change; preserve them if they do.
pub(crate) struct OutputReceipt {
    path: PathBuf,
    identity: OutputIdentity,
    evidence: [u8; 32],
}
enum OutputIdentity {
    Held(File),
    Revision {
        stamp: crate::engine::OutputRevision,
        contents: [u8; 32],
        accessed: u64,
    },
}

// A created file can be usable through its current handle even when its new
// DACL denies a later reopen. Keep the handle unless the process token can
// regain all rollback access rights. Never compact on an impersonated thread.
fn can_reopen(file: &File) -> bool {
    use std::os::windows::fs::MetadataExt;
    #[link(name = "Advapi32")]
    extern "system" {
        fn OpenThreadToken(thread: Handle, rights: u32, as_self: i32, out: *mut Handle) -> i32;
        fn DuplicateToken(token: Handle, level: i32, out: *mut Handle) -> i32;
        fn AccessCheck(
            sd: Handle,
            token: Handle,
            requested: u32,
            mapping: *const GenericMapping,
            privileges: *mut u32,
            privilege_bytes: *mut u32,
            granted: *mut u32,
            allowed: *mut i32,
        ) -> i32;
    }
    let Ok(meta) = file.metadata() else {
        return false;
    };
    if meta.file_attributes() & !(0x20 | 0x80) != 0 || meta.len() > 1024 * 1024 {
        return false;
    }
    let mut raw = std::ptr::null_mut();
    if unsafe { OpenThreadToken(-2isize as Handle, 8, 1, &mut raw) } != 0 {
        drop(Token(raw));
        return false;
    }
    if io::Error::last_os_error().raw_os_error() != Some(1008) {
        return false;
    }
    if unsafe { OpenProcessToken(-1isize as Handle, 10, &mut raw) } == 0 {
        return false;
    }
    let primary = Token(raw);
    if unsafe { DuplicateToken(primary.0, 2, &mut raw) } == 0 {
        return false;
    }
    let impersonation = Token(raw);
    let Ok(sd) = security_descriptor(file, 7) else {
        return false;
    };
    let mapping = GenericMapping {
        read: 0x120089,
        write: 0x120116,
        execute: 0x1200a0,
        all: 0x1f01ff,
    };
    let (mut privileges, mut size, mut granted, mut allowed) = ([0u32; 64], 256, 0, 0);
    unsafe {
        AccessCheck(
            sd.0,
            impersonation.0,
            0x130189,
            &mapping,
            privileges.as_mut_ptr(),
            &mut size,
            &mut granted,
            &mut allowed,
        ) != 0
            && allowed != 0
            && granted == 0x130189
    }
}
fn suppress_automatic_access_time(file: &File) -> bool {
    #[link(name = "Kernel32")]
    extern "system" {
        fn SetFileTime(
            file: Handle,
            created: *const u64,
            access: *const u64,
            written: *const u64,
        ) -> i32;
    }
    let no_update = [u64::MAX];
    unsafe {
        SetFileTime(
            file.as_raw_handle(),
            std::ptr::null(),
            no_update.as_ptr(),
            std::ptr::null(),
        ) != 0
    }
}
fn contents(file: &File) -> Option<[u8; 32]> {
    use std::os::windows::fs::FileExt;
    let mut hash = Sha256::new();
    let mut offset = 0;
    let mut buf = [0u8; 8192];
    loop {
        let n = file.seek_read(&mut buf, offset).ok()?;
        if n == 0 {
            return Some(hash.finalize().into());
        }
        offset += n as u64;
        if offset > 1024 * 1024 {
            return None;
        }
        hash.update(&buf[..n]);
    }
}
pub(crate) fn policy_stamp(file: &File) -> Result<[u8; 32], String> {
    let mut raw = std::ptr::null_mut();
    let rc = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            1,
            7,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut raw,
        )
    };
    if rc != 0 {
        return Err(format!("cannot record owned output security: {rc}"));
    }
    let sd = Allocation(raw);
    let bytes = unsafe { GetSecurityDescriptorLength(sd.0) } as usize;
    if bytes < 20 || bytes > 1024 * 1024 {
        return Err("invalid owned output security".into());
    }
    let mut hash = Sha256::new();
    hash.update(unsafe { std::slice::from_raw_parts(sd.0.cast::<u8>(), bytes) });
    Ok(hash.finalize().into())
}
pub(crate) fn directory_stamp(file: &File) -> Result<[u8; 32], String> {
    use std::os::windows::fs::MetadataExt;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    // A directory can contain named data streams even when it has no children.
    // An empty stream list returns ERROR_HANDLE_EOF on NTFS. Any stream or
    // indeterminate result prevents treating this directory as disposable.
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(file: Handle, class: i32, data: *mut u32, size: u32)
            -> i32;
    }
    let mut stream = [0u64; 32];
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            7,
            stream.as_mut_ptr().cast(),
            std::mem::size_of_val(&stream) as u32,
        )
    };
    if ok != 0
        || (std::io::Error::last_os_error().raw_os_error() != Some(38)
            && crate::motw::named_streams(file)?)
    {
        return Err("cannot establish an empty directory stream list".into());
    }
    let mut hash = Sha256::new();
    hash.update(policy_stamp(file)?);
    hash.update(meta.file_attributes().to_le_bytes());
    hash.update(meta.creation_time().to_le_bytes());
    Ok(hash.finalize().into())
}
fn evidence(file: &File) -> Result<[u8; 32], String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    use std::os::windows::fs::MetadataExt;
    let mut hash = Sha256::new();
    hash.update(meta.file_attributes().to_le_bytes());
    hash.update(meta.file_size().to_le_bytes());
    hash.update(meta.creation_time().to_le_bytes());
    hash.update(meta.last_write_time().to_le_bytes());
    hash.update(policy_stamp(file)?);
    if let Some(zone) = output::output_marker(file)? {
        hash.update(zone);
    }
    Ok(hash.finalize().into())
}
impl OutputReceipt {
    pub(crate) fn capture(path: &Path, file: &File) -> Result<Self, String> {
        Ok(Self {
            path: path.to_owned(),
            identity: OutputIdentity::Held(file.try_clone().map_err(|e| e.to_string())?),
            evidence: evidence(file)?,
        })
    }
    pub(crate) fn compact(mut self) -> Self {
        let OutputIdentity::Held(file) = &self.identity else {
            return self;
        };
        let verified = (|| {
            use std::os::windows::fs::MetadataExt;
            if !can_reopen(file) || !suppress_automatic_access_time(file) {
                return None;
            }
            let bytes = contents(file)?;
            let stamp = crate::engine::OutputRevision::capture(file)?;
            (evidence(file).ok()? == self.evidence).then_some((
                stamp,
                bytes,
                file.metadata().ok()?.last_access_time(),
            ))
        })();
        if let Some((stamp, contents, accessed)) = verified {
            self.identity = OutputIdentity::Revision {
                stamp,
                contents,
                accessed,
            };
        }
        self
    }
    pub(crate) fn remove(self) -> Result<(), String> {
        use std::os::windows::fs::OpenOptionsExt;
        if let OutputIdentity::Revision {
            stamp,
            contents: original,
            accessed,
        } = &self.identity
        {
            // Open the exact path without following a link. Deny data writes,
            // renames and deletion throughout proof and handle-based discard.
            let file = File::options()
                .access_mode(0x130189)
                .share_mode(0)
                .custom_flags(0x00200000)
                .open(&self.path)
                .map_err(|e| {
                    format!(
                        "changed or busy output retained: {}: {e}",
                        self.path.display()
                    )
                })?;
            let info = output::info(&file)?;
            use std::os::windows::fs::MetadataExt;
            if !suppress_automatic_access_time(&file)
                || file
                    .metadata()
                    .map_err(|e| e.to_string())?
                    .last_access_time()
                    != *accessed
                || info[0] & 0x410 != 0
                || info[10] != 1
                || !stamp.matches(&file)
                || evidence(&file).ok() != Some(self.evidence)
                || contents(&file).as_ref() != Some(original)
                || !stamp.matches(&file)
            {
                return Err(format!("changed output retained: {}", self.path.display()));
            }
            return output::discard_private_handle(&file, &self.path);
        }
        let OutputIdentity::Held(file) = self.identity else {
            unreachable!()
        };
        let current = std::fs::File::options()
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(&self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let a = output::info(&file)?;
        let b = output::info(&current)?;
        if a[10] != 1
            || [a[7], a[11], a[12]] != [b[7], b[11], b[12]]
            || evidence(&file).ok() != Some(self.evidence)
        {
            return Err(format!("changed output retained: {}", self.path.display()));
        }
        drop(current);
        output::discard_private_handle(&file, &self.path)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::fs::OpenOptionsExt};

    #[test]
    fn compact_receipt_retains_replacements_rewrites_and_stream_changes() {
        let temp = tempfile::tempdir().unwrap();
        for mode in ["clean", "replacement", "changed", "stream", "written-back"] {
            let path = temp.path().join(mode);
            fs::write(&path, b"original").unwrap();
            let file = File::options()
                .access_mode(0x17019f)
                .share_mode(0)
                .open(&path)
                .unwrap();
            let receipt = OutputReceipt::capture(&path, &file).unwrap().compact();
            assert!(
                matches!(receipt.identity, OutputIdentity::Revision { .. }),
                "NTFS USN required"
            );
            drop(file);
            let when = fs::metadata(&path).unwrap().modified().unwrap();
            match mode {
                "replacement" => {
                    fs::remove_file(&path).unwrap();
                    fs::write(&path, b"original").unwrap();
                }
                "changed" => fs::write(&path, b"modified").unwrap(),
                "written-back" => {
                    fs::write(&path, b"modified").unwrap();
                    fs::write(&path, b"original").unwrap();
                }
                "stream" => {
                    fs::write(format!("{}:foreign", path.display()), b"annotation").unwrap()
                }
                _ => {}
            }
            if mode != "clean" {
                File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(when)
                    .unwrap();
            }
            let result = receipt.remove();
            if mode == "clean" {
                result.unwrap();
                assert!(!path.exists());
            } else {
                assert!(result.is_err() && path.exists(), "{mode}: {result:?}");
            }
        }
    }
}
