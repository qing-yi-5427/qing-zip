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
// A new full security snapshot on every call. No cached ACL, owner or group.
// Typical NTFS descriptors fit in one query; unusual/oversized descriptors keep
// the established Win32 path and its access checks.
pub(crate) fn policy_stamp(file: &File) -> Result<[u8; 32], String> {
    let mut storage=[0u32;1024];
    match native_policy_stamp(file,&mut storage)? {
        Some(stamp)=>Ok(stamp),
        None=>win32_policy_stamp(file),
    }
}
fn native_policy_stamp(file:&File,storage:&mut [u32])->Result<Option<[u8;32]>,String>{
    #[link(name="ntdll")]
    extern "system" {fn NtQuerySecurityObject(handle:Handle,information:u32,descriptor:Handle,length:u32,required:*mut u32)->i32;}
    let capacity=storage.len().checked_mul(4).and_then(|n|u32::try_from(n).ok()).ok_or("security query buffer overflow")?;
    let mut required=0u32;
    let status=unsafe{NtQuerySecurityObject(file.as_raw_handle(),7,storage.as_mut_ptr().cast(),capacity,&mut required)};
    if matches!(status as u32,0xc0000023|0x80000005|0xc0000002|0xc0000003|0xc00000bb){return Ok(None);}
    if status<0{return Err(format!("cannot record owned output security: NTSTATUS {:#010x}",status as u32));}
    if required<20 || required>capacity{return Err("invalid owned output security length".into());}
    let bytes=unsafe{GetSecurityDescriptorLength(storage.as_mut_ptr().cast())};
    if bytes<20 || bytes>required{return Err("invalid owned output security".into());}
    let mut hash=Sha256::new();
    hash.update(unsafe{std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(),bytes as usize)});
    Ok(Some(hash.finalize().into()))
}
fn win32_policy_stamp(file: &File) -> Result<[u8; 32], String> {
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
    match output::output_marker(file) {
        Ok(Some(zone)) => hash.update(zone),
        Ok(None) => {}
        // FAT32/exFAT reject FileStreamInfo even for ordinary files. The
        // volume flag is queried on the same held file. NTFS stream-query
        // failures and unknown streams still prevent deletion.
        Err(_) if !crate::motw::named_streams(file)? => {}
        Err(error) => return Err(error),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs,os::windows::fs::OpenOptionsExt};
    fn opened(path:&Path)->File {File::options().access_mode(0xe0080).share_mode(7).custom_flags(0x02000000).open(path).unwrap()}
    fn set_dacl(file:&File,sddl:&str){
        let text:Vec<_>=sddl.encode_utf16().chain(Some(0)).collect();let mut sd=std::ptr::null_mut();
        assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(),1,&mut sd,std::ptr::null_mut())},0);
        let _owned=Allocation(sd);let(mut present,mut defaulted,mut acl)=(0,0,std::ptr::null_mut());
        assert_ne!(unsafe{GetSecurityDescriptorDacl(sd,&mut present,&mut acl,&mut defaulted)},0);
        assert_eq!(unsafe{SetSecurityInfo(file.as_raw_handle(),1,4|0x80000000,std::ptr::null_mut(),std::ptr::null_mut(),acl,std::ptr::null_mut())},0);
    }
    #[test]
    fn receipt_native_full_security_matches_win32_and_preserves_acl_order(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("file");fs::write(&path,b"data").unwrap();
        for file in [opened(&path),opened(dir.path())] {
            assert_eq!(policy_stamp(&file).unwrap(),win32_policy_stamp(&file).unwrap());
            let token=token().unwrap();let sid=current_sid(&token).unwrap();
            for acl in [format!("D:P(A;;FA;;;{sid})(A;;FR;;;BU)"),format!("D:P(D;;WD;;;BU)(A;;FA;;;{sid})(A;;FR;;;WD)"),format!("D:P(A;;FA;;;{sid})(D;;WD;;;BU)(A;;FR;;;WD)")] {
                set_dacl(&file,&acl);assert_eq!(policy_stamp(&file).unwrap(),win32_policy_stamp(&file).unwrap());
            }
        }
    }
    #[test]
    fn receipt_native_small_buffer_declines_and_group_changes_are_observed(){
        #[link(name="advapi32")]extern "system"{fn GetSecurityDescriptorGroup(sd:Handle,group:*mut Handle,defaulted:*mut i32)->i32;}
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("file");fs::write(&path,b"data").unwrap();let file=opened(&path);
        assert!(native_policy_stamp(&file,&mut [0u32;2]).unwrap().is_none());
        let before=policy_stamp(&file).unwrap();let text:Vec<_>="G:BU".encode_utf16().chain(Some(0)).collect();let mut sd=std::ptr::null_mut();
        assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(),1,&mut sd,std::ptr::null_mut())},0);let _owned=Allocation(sd);
        let(mut group,mut defaulted)=(std::ptr::null_mut(),0);assert_ne!(unsafe{GetSecurityDescriptorGroup(sd,&mut group,&mut defaulted)},0);
        assert_eq!(unsafe{SetSecurityInfo(file.as_raw_handle(),1,2,std::ptr::null_mut(),group,std::ptr::null_mut(),std::ptr::null_mut())},0);
        let after=policy_stamp(&file).unwrap();assert_ne!(before,after);assert_eq!(after,win32_policy_stamp(&file).unwrap());
    }
    #[test]
    fn receipt_oversized_security_uses_win32_fallback_without_losing_fields(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("file");fs::write(&path,b"data").unwrap();let file=opened(&path);
        let sid=current_sid(&token().unwrap()).unwrap();
        set_dacl(&file,&format!("D:P(A;;FA;;;{sid}){}",(0..300).map(|i|format!("(A;;FR;;;S-1-5-21-111-222-333-{})",10000+i)).collect::<String>()));
        assert!(native_policy_stamp(&file,&mut [0u32;1024]).unwrap().is_none());
        assert_eq!(policy_stamp(&file).unwrap(),win32_policy_stamp(&file).unwrap());
    }
    #[test]
    fn receipt_security_and_ads_changes_still_preserve_owned_file(){
        for ads in [false,true] {
            let dir=tempfile::tempdir().unwrap();let path=dir.path().join("file");fs::write(&path,b"data").unwrap();
            let file=File::options().access_mode(0xf0080).share_mode(0).open(&path).unwrap();
            let receipt=OutputReceipt::capture(&path,&file).unwrap();
            if ads {fs::write(format!("{}:foreign",path.display()),b"keep").unwrap();}
            else {let sid=current_sid(&token().unwrap()).unwrap();set_dacl(&file,&format!("D:P(A;;FA;;;{sid})(A;;FR;;;BU)"));}
            let error=receipt.remove().unwrap_err();assert!(error.contains("changed output retained"),"{error}");
            drop(file);assert_eq!(fs::read(&path).unwrap(),b"data");
            if ads {assert_eq!(fs::read(format!("{}:foreign",path.display())).unwrap(),b"keep");}
        }
    }
    #[test]
    fn receipt_native_does_not_hide_missing_read_control(){
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("file");fs::write(&path,b"data").unwrap();
        let file=File::options().access_mode(0x80).share_mode(7).open(path).unwrap();
        assert!(native_policy_stamp(&file,&mut [0u32;1024]).is_err());assert!(win32_policy_stamp(&file).is_err());
    }
}
