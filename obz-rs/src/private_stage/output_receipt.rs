use super::*;
use sha2::{Digest, Sha256};

/// Keeps a just-created output on its original exclusive data handle. Default
/// stream writes, replacement and deletion cannot race rollback. Other streams,
/// attributes, owner/group and DACL can still change; preserve them if they do.
pub(crate) struct OutputReceipt {
    path: PathBuf,
    file: File,
    evidence: [u8; 32],
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
    let mut hash = Sha256::new();
    hash.update(policy_stamp(file)?);
    hash.update((meta.file_attributes() & !0x20).to_le_bytes());
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
            file: file.try_clone().map_err(|e| e.to_string())?,
            evidence: evidence(file)?,
        })
    }
    pub(crate) fn remove(self) -> Result<(), String> {
        use std::os::windows::fs::OpenOptionsExt;
        let current = std::fs::File::options()
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(&self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let a = output::info(&self.file)?;
        let b = output::info(&current)?;
        if a[10] != 1
            || [a[7], a[11], a[12]] != [b[7], b[11], b[12]]
            || evidence(&self.file).ok() != Some(self.evidence)
        {
            return Err(format!("changed output retained: {}", self.path.display()));
        }
        drop(current);
        output::discard_private_handle(&self.file, &self.path)
    }
}
