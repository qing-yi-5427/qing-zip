//! Enable only an already-held right on a duplicate effective token. The caller
//! and process tokens are never adjusted. A fresh, joined thread contains even
//! an OS failure to revert impersonation; no privileged thread is pooled/reused.
use std::{ffi::c_void, os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle}};
type Handle=*mut c_void;
#[repr(C)] #[derive(Clone,Copy)] struct Luid {low:u32,high:i32}
#[repr(C)] struct Privilege {count:u32,luid:Luid,attributes:u32}
#[link(name="advapi32")] unsafe extern "system" {
    fn OpenThreadToken(thread:Handle,access:u32,as_self:i32,token:*mut Handle)->i32;
    fn OpenProcessToken(process:Handle,access:u32,token:*mut Handle)->i32;
    fn GetTokenInformation(token:Handle,class:u32,data:*mut c_void,size:u32,needed:*mut u32)->i32;
    fn DuplicateTokenEx(token:Handle,access:u32,security:*const c_void,level:u32,kind:u32,result:*mut Handle)->i32;
    fn LookupPrivilegeValueW(system:*const u16,name:*const u16,luid:*mut Luid)->i32;
    fn AdjustTokenPrivileges(token:Handle,disable_all:i32,state:*const Privilege,size:u32,previous:*mut Privilege,needed:*mut u32)->i32;
    fn SetThreadToken(thread:*const Handle,token:Handle)->i32;
    fn RevertToSelf()->i32;
}
#[link(name="kernel32")] unsafe extern "system" {
    fn GetCurrentThread()->Handle;
    fn GetCurrentProcess()->Handle;
    fn SetLastError(error:u32);
}
fn error(action:&str)->String{format!("{action}：{}",std::io::Error::last_os_error())}
fn prepare()->Result<OwnedHandle,String>{
    let mut raw=std::ptr::null_mut();
    let thread_token=unsafe{OpenThreadToken(GetCurrentThread(),0x0a,1,&mut raw)}!=0;
    if !thread_token {
        let failure=std::io::Error::last_os_error();
        // Never replace an inaccessible, restricted, or low-level impersonation
        // identity with the process identity. NO_TOKEN is the sole fallback.
        if failure.raw_os_error()!=Some(1008){return Err(format!("无法读取调用者线程身份：{failure}"));}
        if unsafe{OpenProcessToken(GetCurrentProcess(),0x0a,&mut raw)}==0{return Err(error("无法读取调用者进程身份"));}
    }
    // OwnedHandle owns a real token HANDLE (not the pseudo process/thread
    // handles). Its standard-library Send implementation carries ownership to
    // one fresh scoped thread and closes it only after that thread is finished.
    let source=unsafe{OwnedHandle::from_raw_handle(raw)};
    let mut level=2u32;
    if thread_token {
        let mut needed=0;
        if unsafe{GetTokenInformation(source.as_raw_handle(),9,(&mut level as *mut u32).cast(),4,&mut needed)}==0{return Err(error("无法读取调用者模拟级别"));}
        if level<2{return Err("调用者模拟级别不足；不会改用进程身份".into());}
    }
    let mut copy=std::ptr::null_mut();
    // Preserve the effective source's impersonation level and restrictions.
    if unsafe{DuplicateTokenEx(source.as_raw_handle(),0x2c,std::ptr::null(),level,2,&mut copy)}==0{return Err(error("无法复制调用者身份"));}
    let copy=unsafe{OwnedHandle::from_raw_handle(copy)};
    let name:Vec<_>="SeCreateSymbolicLinkPrivilege".encode_utf16().chain([0]).collect();let mut luid=Luid{low:0,high:0};
    if unsafe{LookupPrivilegeValueW(std::ptr::null(),name.as_ptr(),&mut luid)}==0{return Err(error("无法查询符号链接权限"));}
    let enable=Privilege{count:1,luid,attributes:2};
    unsafe{SetLastError(0);}
    let adjusted=unsafe{AdjustTokenPrivileges(copy.as_raw_handle(),0,&enable,0,std::ptr::null_mut(),std::ptr::null_mut())};
    let status=std::io::Error::last_os_error();
    if adjusted==0{return Err(format!("无法准备已有符号链接权限：{status}"));}
    // AdjustTokenPrivileges may succeed while reporting NOT_ALL_ASSIGNED.
    if status.raw_os_error()!=Some(0){return Err(format!("调用者未持有所需的符号链接权限，或权限无法启用：{status}"));}
    Ok(copy)
}
#[cfg(test)] thread_local!{pub(super) static FAIL_RESTORE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
struct Impersonation {active:bool,#[cfg(test)] fail_restore:bool}
impl Impersonation {
    fn enter(token:&OwnedHandle)->Result<Self,String>{
        if unsafe{SetThreadToken(std::ptr::null(),token.as_raw_handle())}==0{return Err(error("无法设置独立操作线程身份"));}
        Ok(Self{active:true,#[cfg(test)] fail_restore:false})
    }
    fn restore(&mut self)->Result<(),String>{
        if !self.active{return Ok(());}
        #[cfg(test)] if std::mem::take(&mut self.fail_restore){return Err("injected thread identity restore failure".into());}
        if unsafe{RevertToSelf()}==0{return Err(error("独立操作线程恢复身份失败"));}
        self.active=false;Ok(())
    }
}
impl Drop for Impersonation {fn drop(&mut self){let _=self.restore();}}
pub(super) fn run<T:Send>(work:impl FnOnce()->Result<T,String>+Send)->Result<T,String>{
    // No output is created until both preparation and impersonation succeed.
    let token=prepare()?;
    #[cfg(test)] let fail_restore=FAIL_RESTORE.with(|flag|flag.replace(false));
    std::thread::scope(|scope|{
        let worker=std::thread::Builder::new().name("qing-link-privilege".into()).spawn_scoped(scope,move||{
            let mut guard=Impersonation::enter(&token)?;
            #[cfg(test)] {guard.fail_restore=fail_restore;}
            let result=work();let restored=guard.restore();
            match (result,restored){
                (Ok(value),Ok(()))=>Ok(value),(Err(e),Ok(()))=>Err(e),
                (Ok(_),Err(e))=>Err(e),(Err(e),Err(restore))=>Err(format!("{e}；{restore}")),
            }
        }).map_err(|e|format!("无法创建独立符号链接操作线程：{e}"))?;
        // A panic unwinds its RAII guard; joining terminates this thread before
        // the caller handles its retained, identity-bound output receipt.
        worker.join().map_err(|_|"独立符号链接操作线程异常结束".to_string())?
    })
}
#[cfg(test)]
pub(super) fn snapshot()->(Vec<u8>,Option<Vec<u8>>){
    fn query(token:&OwnedHandle,class:u32)->Vec<u8>{
        let mut needed=0;unsafe{GetTokenInformation(token.as_raw_handle(),class,std::ptr::null_mut(),0,&mut needed);}
        assert!(needed>0&&needed<=16384);let mut data=vec![0u8;needed as usize];
        assert_ne!(unsafe{GetTokenInformation(token.as_raw_handle(),class,data.as_mut_ptr().cast(),needed,&mut needed)},0);data
    }
    let mut raw=std::ptr::null_mut();assert_ne!(unsafe{OpenProcessToken(GetCurrentProcess(),8,&mut raw)},0);
    let process=unsafe{OwnedHandle::from_raw_handle(raw)};
    let thread=if unsafe{OpenThreadToken(GetCurrentThread(),8,1,&mut raw)}!=0 {
        Some(query(&unsafe{OwnedHandle::from_raw_handle(raw)},10))
    }else{assert_eq!(std::io::Error::last_os_error().raw_os_error(),Some(1008));None};
    (query(&process,3),thread)
}
#[cfg(test)]
pub(super) fn require_existing_thread_contexts(){
    #[link(name="advapi32")] unsafe extern "system" {
        fn CreateRestrictedToken(token:Handle,flags:u32,disabled_count:u32,disabled:*const c_void,
            deleted_count:u32,deleted:*const c_void,restricted_count:u32,restricted:*const c_void,result:*mut Handle)->i32;
    }
    let outside=snapshot();
    for mode in [0,1,2] {
        // prepare() must succeed before this fixture runs: no skip for missing
        // rights. All synthetic caller contexts live only on a fresh thread.
        let effective=prepare().expect("native CI must hold the existing symbolic-link privilege");
        // Reopen only this duplicate to obtain DUPLICATE access for test tokens.
        // The source is never a broader process identity.
        #[link(name="kernel32")] unsafe extern "system" {
            fn DuplicateHandle(process:Handle,source:Handle,target_process:Handle,target:*mut Handle,access:u32,inherit:i32,options:u32)->i32;
        }
        let mut source_raw=std::ptr::null_mut();
        assert_ne!(unsafe{DuplicateHandle(GetCurrentProcess(),effective.as_raw_handle(),GetCurrentProcess(),&mut source_raw,0x2e,0,0)},0);
        let source=unsafe{OwnedHandle::from_raw_handle(source_raw)};let mut raw=std::ptr::null_mut();
        if mode==2 {
            let name:Vec<_>="SeCreateSymbolicLinkPrivilege".encode_utf16().chain([0]).collect();let mut luid=Luid{low:0,high:0};
            assert_ne!(unsafe{LookupPrivilegeValueW(std::ptr::null(),name.as_ptr(),&mut luid)},0);
            let deleted=Privilege{count:1,luid,attributes:0};
            assert_ne!(unsafe{CreateRestrictedToken(source.as_raw_handle(),0,0,std::ptr::null(),1,(&deleted.luid as *const Luid).cast(),0,std::ptr::null(),&mut raw)},0);
        }else{
            assert_ne!(unsafe{DuplicateTokenEx(source.as_raw_handle(),0x2c,std::ptr::null(),if mode==1{1}else{2},2,&mut raw)},0);
        }
        let caller=unsafe{OwnedHandle::from_raw_handle(raw)};
        std::thread::scope(|scope|scope.spawn(move||{
            let mut guard=Impersonation::enter(&caller).unwrap();let before=snapshot();let mut executed=false;
            let result=run(||{executed=true;Ok(())});assert_eq!(snapshot(),before);
            match mode {
                0=>{result.unwrap();assert!(executed);},
                1=>{assert!(result.unwrap_err().contains("模拟级别不足"));assert!(!executed);},
                _=>{assert!(result.unwrap_err().contains("os error 1300"));assert!(!executed);},
            }
            guard.restore().unwrap();
        }).join().unwrap());
    }
    assert_eq!(snapshot(),outside);
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn preparation_preserves_caller_identity_and_process_privileges(){
        let before=snapshot();let fixture=tempfile::tempdir().unwrap();let output=fixture.path().join("only-if-prepared");
        let result=run(||std::fs::write(&output,b"owned fixture").map_err(|e|e.to_string()));
        assert_eq!(snapshot(),before);
        match result {
            Ok(())=>{assert_eq!(std::fs::read(output).unwrap(),b"owned fixture");println!("existing-right preparation succeeded; caller identity and process privileges unchanged");},
            Err(e)=>{assert!(e.contains("os error 1300")||e.contains("os error 1314"),"unexpected preparation failure: {e}");assert!(!output.exists());println!("missing existing right: {e}; no output or caller identity/privilege changes");},
        }
    }
}
