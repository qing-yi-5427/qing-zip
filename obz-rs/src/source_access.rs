//! Optional per-handle suppression of source access-time updates.
//! No saved timestamp is ever written back. This is not a directory traversal,
//! ADS-group, independent-handle, or external-process preservation guarantee.
use std::{fs::File, io, path::Path};
use serde::Serialize;
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all="snake_case")]
pub enum Phase { EnhancedOpen, EnhancedReopen, SuppressUpdates, AlternateStreamProof, EncryptedSourceProof }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preservation {
    Disabled,
    Applied,
    Unavailable { phase: Phase, os_error: Option<i32> },
}
/// Per-operation evidence for input primary-stream handles, not a global
/// filesystem timestamp guarantee. The external backend reports best-effort
/// only because its own attribute-permission fallback is not observable.
#[derive(Clone, Default, Serialize)]
pub struct Report {
    pub input: StageCounts,
    pub move_cleanup: StageCounts,
    pub applied_handles: usize,
    pub unavailable_handles: usize,
    pub backend_best_effort: bool,
    pub unconfirmed_sources: usize,
    pub directory_enumeration_covered: bool,
    pub warnings: Vec<Warning>,
    pub omitted_warnings: usize,
}
#[derive(Clone, Default, Serialize)]
pub struct StageCounts {pub applied:usize,pub unavailable:usize}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all="snake_case")]
pub enum Stage {Input,MoveCleanup}
#[derive(Clone, Serialize)]
pub struct Warning {pub source:String,pub stage:Stage,pub phase:Phase,pub os_error:Option<i32>}
impl Report {
    pub(crate) fn record(&mut self,path:&Path,status:Preservation){self.record_stage(path,status,Stage::Input)}
    pub(crate) fn record_cleanup(&mut self,path:&Path,status:Preservation){self.record_stage(path,status,Stage::MoveCleanup)}
    fn record_stage(&mut self,path:&Path,status:Preservation,stage:Stage){
        let count=match stage{Stage::Input=>&mut self.input,Stage::MoveCleanup=>&mut self.move_cleanup};
        match status{
            Preservation::Disabled=>(),Preservation::Applied=>{self.applied_handles+=1;count.applied+=1;},
            Preservation::Unavailable{phase,os_error}=>{self.unavailable_handles+=1;count.unavailable+=1;if self.warnings.len()<64{self.warnings.push(Warning{source:path.display().to_string(),stage,phase,os_error});}else{self.omitted_warnings+=1;}}
        }
    }
    pub(crate) fn record_unconfirmed(&mut self,path:&Path,phase:Phase){
        self.unconfirmed_sources+=1;
        if self.warnings.len()<64 {self.warnings.push(Warning{source:path.display().to_string(),stage:Stage::Input,phase,os_error:None});}
        else {self.omitted_warnings+=1;}
    }
    pub(crate) fn summary(&self)->String{
        format!("已尝试保护输入文件主流读取的访问时间；{} 个读取句柄已启用，{} 个未能启用{}。不包含目录枚举或其他程序的访问。",self.applied_handles,self.unavailable_handles,if self.backend_best_effort{"，外部引擎仅尽力保护，无法逐项确认"}else{""})
    }
}
pub(crate) struct Opened { pub file: File, pub preservation: Preservation }
/// Preserve the caller's exact access/share/flags. The caller must keep its
/// existing ancestor pins and validate the returned object's identity/type.
#[derive(Clone, Copy)]
pub(crate) struct OpenSpec { pub access: u32, pub share: u32, pub flags: u32 }
impl OpenSpec {
    fn open(self, path: &Path, access: u32) -> io::Result<File> {
        File::options().access_mode(access).share_mode(self.share)
            .custom_flags(self.flags).open(path)
    }
}
#[link(name="kernel32")]
extern "system" {
    fn ReOpenFile(file:*mut std::ffi::c_void,access:u32,share:u32,flags:u32)->*mut std::ffi::c_void;
    fn SetFileTime(file: *mut std::ffi::c_void, created: *const u64,
                   accessed: *const u64, modified: *const u64) -> i32;
}
#[cfg(test)]
thread_local! {pub(crate) static REOPEN_CALLS:std::cell::RefCell<Vec<usize>>=const{std::cell::RefCell::new(Vec::new())};pub(crate) static SUPPRESS_CALLS:std::cell::RefCell<Vec<usize>>=const{std::cell::RefCell::new(Vec::new())};pub(crate) static FAIL_SUPPRESS: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};}
fn suppress(file: &File) -> io::Result<()> {
    #[cfg(test)] SUPPRESS_CALLS.with(|v|v.borrow_mut().push(file.as_raw_handle() as usize));
    #[cfg(test)]
    if FAIL_SUPPRESS.with(|v|v.replace(false)) {return Err(io::Error::from_raw_os_error(50));}
    let sentinel=u64::MAX;
    if unsafe{SetFileTime(file.as_raw_handle(),std::ptr::null(),&sentinel,std::ptr::null())}==0 {
        Err(io::Error::last_os_error())
    }else{Ok(())}
}
/// Reopen the already-held object; no pathname lookup or release of the caller's
/// transaction/namespace lease. The caller validates identity before replacement.
pub(crate) fn protect_reader(file:&File,spec:OpenSpec)->Result<File,Preservation>{
    use std::os::windows::io::FromRawHandle;
    #[cfg(test)] REOPEN_CALLS.with(|v|v.borrow_mut().push(file.as_raw_handle() as usize));
    let raw=unsafe{ReOpenFile(file.as_raw_handle(),spec.access|0x100,spec.share,spec.flags)};
    if raw.is_null()||raw as isize == -1{return Err(Preservation::Unavailable{phase:Phase::EnhancedReopen,os_error:io::Error::last_os_error().raw_os_error()});}
    let reopened=unsafe{File::from_raw_handle(raw)};
    suppress(&reopened).map_err(|e|Preservation::Unavailable{phase:Phase::SuppressUpdates,os_error:e.raw_os_error()})?;
    Ok(reopened)
}
/// Duplicate the caller's already-open FILE_OBJECT and verify suppression on it.
/// Used for directory handles that ReOpenFile cannot enhance. No pathname lookup
/// and no assumed prior protection: missing WRITE_ATTRIBUTES is a typed failure.
pub(crate) fn protect_shared_reader(file:&File)->Result<File,Preservation>{
    let reader=file.try_clone().map_err(|e|Preservation::Unavailable{phase:Phase::EnhancedReopen,os_error:e.raw_os_error()})?;
    suppress(&reader).map_err(|e|Preservation::Unavailable{phase:Phase::SuppressUpdates,os_error:e.raw_os_error()})?;
    Ok(reader)
}
pub(crate) fn open_with(access:u32,preserve:bool,mut opener:impl FnMut(u32)->io::Result<File>)->io::Result<Opened>{
    if !preserve{return opener(access).map(|file|Opened{file,preservation:Preservation::Disabled});}
    match opener(access|0x100) {
        Ok(file)=>{let preservation=match suppress(&file){Ok(())=>Preservation::Applied,Err(e)=>Preservation::Unavailable{phase:Phase::SuppressUpdates,os_error:e.raw_os_error()}};Ok(Opened{file,preservation})},
        Err(enhanced)=>{
            let file=opener(access)?;
            Ok(Opened{file,preservation:Preservation::Unavailable{phase:Phase::EnhancedOpen,os_error:enhanced.raw_os_error()}})
        }
    }
}
pub(crate) fn open(path:&Path,spec:OpenSpec,preserve:bool)->io::Result<Opened>{
    // The relative-handle caller can reuse the same permission fallback without
    // converting a locked source object back into a pathname.
    open_with(spec.access,preserve,|access|spec.open(path,access))
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::{io::Read,os::windows::fs::MetadataExt};
    fn spec()->OpenSpec{OpenSpec{access:0x80000000,share:1,flags:0x00200000}}
    #[test]
    fn enabled_source_read_preserves_times_and_existing_share_contract(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");std::fs::write(&p,b"payload").unwrap();
        let baseline=std::time::UNIX_EPOCH+std::time::Duration::from_secs(946684800);
        File::options().write(true).open(&p).unwrap().set_times(std::fs::FileTimes::new().set_accessed(baseline).set_modified(baseline)).unwrap();
        let before=std::fs::metadata(&p).unwrap();let mut opened=open(&p,spec(),true).unwrap();assert_eq!(opened.preservation,Preservation::Applied);
        assert!(File::options().write(true).open(&p).is_err());assert!(std::fs::rename(&p,t.path().join("other")).is_err());
        let mut data=Vec::new();opened.file.read_to_end(&mut data).unwrap();assert_eq!(data,b"payload");drop(opened);
        let after=std::fs::metadata(&p).unwrap();assert_eq!(after.last_access_time(),before.last_access_time());assert_eq!(after.last_write_time(),before.last_write_time());assert_eq!(after.creation_time(),before.creation_time());
        // This records a real successful sentinel call. An unchanged timestamp
        // alone cannot prove suppression on hosts with automatic ATime disabled.
    }
    #[test]
    fn protected_readonly_duplicate_shares_object_but_not_attribute_write_access(){
        use std::os::windows::{io::FromRawHandle,fs::MetadataExt};
        use std::io::{Seek,Read};
        #[link(name="kernel32")]
        extern "system"{
            fn GetCurrentProcess()->*mut std::ffi::c_void;
            fn DuplicateHandle(source_process:*mut std::ffi::c_void,source:*mut std::ffi::c_void,target_process:*mut std::ffi::c_void,target:*mut *mut std::ffi::c_void,access:u32,inherit:i32,options:u32)->i32;
        }
        let temp=tempfile::tempdir().unwrap();let path=temp.path().join("source");std::fs::write(&path,b"same FILE_OBJECT").unwrap();
        File::options().write(true).open(&path).unwrap().set_times(std::fs::FileTimes::new().set_accessed(std::time::UNIX_EPOCH+std::time::Duration::from_secs(946684800))).unwrap();
        let mut opened=open(&path,spec(),true).unwrap();assert_eq!(opened.preservation,Preservation::Applied);
        let atime=opened.file.metadata().unwrap().last_access_time();let mut raw=std::ptr::null_mut();
        assert_ne!(unsafe{DuplicateHandle(GetCurrentProcess(),opened.file.as_raw_handle(),GetCurrentProcess(),&mut raw,0x100081,0,0)},0);
        let mut reader=unsafe{File::from_raw_handle(raw)};
        assert_eq!(suppress(&reader).unwrap_err().raw_os_error(),Some(5),"worker reader must not receive WRITE_ATTRIBUTES");
        let mut data=Vec::new();reader.read_to_end(&mut data).unwrap();assert_eq!(data,b"same FILE_OBJECT");
        assert_eq!(opened.file.stream_position().unwrap(),data.len() as u64,"DuplicateHandle must retain the protected FILE_OBJECT");
        drop(reader);drop(opened);assert_eq!(std::fs::metadata(path).unwrap().last_access_time(),atime);
    }
    #[test]
    fn suppression_failure_is_typed_and_still_returns_readable_source(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");std::fs::write(&p,b"data").unwrap();FAIL_SUPPRESS.with(|v|v.set(true));
        let mut opened=open(&p,spec(),true).unwrap();assert_eq!(opened.preservation,Preservation::Unavailable{phase:Phase::SuppressUpdates,os_error:Some(50)});let mut data=String::new();opened.file.read_to_string(&mut data).unwrap();assert_eq!(data,"data");
    }
    #[test]
    fn denied_attribute_access_falls_back_without_changing_source_acl(){
        use std::os::windows::ffi::OsStrExt;
        #[link(name="advapi32")]
        extern "system"{fn ConvertStringSecurityDescriptorToSecurityDescriptorW(s:*const u16,revision:u32,out:*mut *mut std::ffi::c_void,len:*mut u32)->i32;fn SetFileSecurityW(path:*const u16,flags:u32,sd:*mut std::ffi::c_void)->i32;}
        #[link(name="kernel32")]
        extern "system"{fn LocalFree(p:*mut std::ffi::c_void)->*mut std::ffi::c_void;}
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");std::fs::write(&p,b"readable").unwrap();let sddl:Vec<u16>="D:P(D;;0x100;;;WD)(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect();let path:Vec<u16>=p.as_os_str().encode_wide().chain(Some(0)).collect();let mut sd=std::ptr::null_mut();
        assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut sd,std::ptr::null_mut())},0);let set=unsafe{SetFileSecurityW(path.as_ptr(),0x80000004,sd)};unsafe{LocalFree(sd)};assert_ne!(set,0);
        assert!(spec().open(&p,0x80000100).is_err());let mut opened=open(&p,spec(),true).unwrap();assert_eq!(opened.preservation,Preservation::Unavailable{phase:Phase::EnhancedOpen,os_error:Some(5)});let mut text=String::new();opened.file.read_to_string(&mut text).unwrap();assert_eq!(text,"readable");drop(opened);assert!(spec().open(&p,0x80000100).is_err(),"helper must not grant attributes permission");
        let disabled=open(&p,spec(),false).unwrap();assert_eq!(disabled.preservation,Preservation::Disabled);
    }
    #[test]
    fn missing_source_and_existing_read_failure_keep_ordinary_error(){let t=tempfile::tempdir().unwrap();let p=t.path().join("absent");let ordinary=spec().open(&p,spec().access).err().unwrap();let actual=open(&p,spec(),true).err().unwrap();assert_eq!(actual.raw_os_error(),ordinary.raw_os_error());}
}
