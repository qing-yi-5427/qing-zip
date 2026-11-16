//! Explicit local creation only. Archive readers and ordinary publishers never
//! call these primitives. All mutations bind to pinned handles, never a reopened
//! pathname; an unexpected post-create failure preserves an unverified entry.
use super::*;
use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
use std::path::{Component, Prefix};
#[path="file_workspace_link_privilege.rs"]
mod privilege;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all="kebab-case")]
pub enum LinkKind { Hard, SymbolicFile, SymbolicDirectory, Junction, Wsl }
#[derive(Debug, Serialize)]
pub struct CreatedLink { pub path: String, pub target: String, pub kind: LinkKind }
#[repr(C)] struct Unicode { length:u16, maximum:u16, buffer:*mut u16 }
#[repr(C)] struct Attributes { length:u32, root:Handle, name:*mut Unicode, flags:u32, security:Handle, qos:Handle }
#[repr(C)] struct Status { status:usize, information:usize }
#[repr(C)] struct LinkInformation { replace:u32, root:Handle, length:u32, name:[u16;1] }
#[link(name="ntdll")] unsafe extern "system" {
    fn NtCreateFile(handle:*mut Handle,access:u32,attributes:*const Attributes,status:*mut Status,
        allocation:*const i64,attrs:u32,sharing:u32,disposition:u32,options:u32,ea:Handle,ea_size:u32)->i32;
    fn NtSetInformationFile(handle:Handle,status:*mut usize,data:*const c_void,size:u32,class:u32)->i32;
    fn RtlNtStatusToDosError(status:i32)->u32;
}
#[link(name="kernel32")] unsafe extern "system" {
    fn DeviceIoControl(handle:Handle,code:u32,input:*const u8,input_size:u32,output:*mut u8,
        output_size:u32,returned:*mut u32,overlapped:Handle)->i32;
    fn SetFileInformationByHandle(handle:Handle,class:i32,data:*const c_void,size:u32)->i32;
}
fn nt_error(action:&str,status:i32)->String {
    format!("{action}：{}",std::io::Error::from_raw_os_error(unsafe{RtlNtStatusToDosError(status)} as i32))
}
fn checked_path(text:&str)->Result<PathBuf,String>{
    if text.is_empty() || text.contains('\0') {return Err("请选择普通本地绝对路径".into());}
    let path=PathBuf::from(text);
    if !path.is_absolute() || !matches!(path.components().next(),Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_))){return Err("链接创建仅支持普通本地绝对路径".into());}
    for name in text.get(3..).ok_or("无效本地路径")?.split(['/', '\\']).filter(|s|!s.is_empty()) {
        checked_name(name)?;
    }
    local_path(text)
}
fn checked_name(name:&str)->Result<PathBuf,String>{
    if name.is_empty() || name.contains(['/', '\\']) || matches!(name,"."|"..") {return Err("请输入单个新名称".into());}
    let leaf=crate::extract::relative_path(name)?;
    if leaf.components().count()!=1 {return Err("名称必须是单个路径组件".into());}
    Ok(leaf)
}
// FILE_CREATE is atomic and returns the exact created object; OPEN_REPARSE_POINT
// plus a single component relative to the pinned parent excludes traversal.
fn child_io(parent:&File,name:&std::ffi::OsStr,create:Option<bool>,access:u32,sharing:u32)->std::io::Result<File>{
    let mut wide:Vec<_>=name.encode_wide().collect();
    if wide.is_empty() || wide.len()>32767 || wide.iter().any(|v|matches!(*v,0|47|58|92)) {return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"无效链接名称"));}
    let mut unicode=Unicode{length:(wide.len()*2) as u16,maximum:(wide.len()*2) as u16,buffer:wide.as_mut_ptr()};
    let attrs=Attributes{length:std::mem::size_of::<Attributes>() as u32,root:parent.as_raw_handle(),name:&mut unicode,flags:0x40,security:std::ptr::null_mut(),qos:std::ptr::null_mut()};
    let mut status=Status{status:0,information:0};let mut raw=std::ptr::null_mut();
    let result=unsafe{NtCreateFile(&mut raw,access,&attrs,&mut status,std::ptr::null(),0,sharing,
        if create.is_some(){2}else{1},0x200020|match create{Some(true)=>1,Some(false)=>0x40,None=>0},std::ptr::null_mut(),0)};
    if result<0{return Err(std::io::Error::from_raw_os_error(unsafe{RtlNtStatusToDosError(result)} as i32));}
    Ok(unsafe{File::from_raw_handle(raw)})
}
fn child(parent:&File,name:&std::ffi::OsStr,create:Option<bool>,access:u32,sharing:u32)->Result<File,String>{
    child_io(parent,name,create,access,sharing).map_err(|e|format!("打开或创建链接项目：{e}"))
}
// Observe only existing ordinary prefixes. Each open is relative to the
// preceding no-follow handle; a missing component leaves the rest as text.
fn check_junction_target(path:&Path)->Result<(),String>{
    let mut components=path.components();let prefix=components.next().ok_or("无效本地路径")?;
    if !matches!(components.next(),Some(Component::RootDir)){return Err("无效本地绝对路径".into());}
    let mut root=PathBuf::from(prefix.as_os_str());root.push("\\");
    let mut held=vec![File::options().access_mode(0x100080).share_mode(7).custom_flags(0x02200000).open(root).map_err(|e|e.to_string())?];
    for component in components{
        let Component::Normal(name)=component else{return Err("无效本地路径组件".into());};
        match child_io(held.last().ok_or("缺少目标前缀句柄")?,name,None,0x100080,7){
            Ok(file)=>{let meta=file.metadata().map_err(|e|e.to_string())?;
                if !meta.is_dir() || meta.file_attributes()&0x400!=0{return Err("目录联接点的现有目标路径必须是普通目录".into());}held.push(file);
            },
            Err(error) if matches!(error.raw_os_error(),Some(2|3))=>return Ok(()),
            Err(error)=>return Err(format!("检查目录联接点目标：{error}")),
        }
    }Ok(())
}
fn parent_pin(path:&Path,guards:&mut CheckedDirectories)->Result<File,String>{
    guards.check(path)?;
    let expected=guards.held.get(path).ok_or("缺少本地父目录锁")?;
    let pin=File::options().access_mode(0x81).share_mode(if expected.1{3}else{1}).custom_flags(0x02200000).open(path).map_err(|e|e.to_string())?;
    let (id,attrs)=directory_identity_and_attributes(&pin)?;
    if id!=directory_identity(&expected.0)? || attrs&0x410!=0x10{return Err("本地父目录已改变".into());}
    Ok(pin)
}
fn hard(source:&File,parent:&File,name:&std::ffi::OsStr)->Result<(),String>{
    let wide:Vec<_>=name.encode_wide().collect();
    let offset=std::mem::offset_of!(LinkInformation,name);
    let size=(offset+wide.len()*2).max(std::mem::size_of::<LinkInformation>());
    // usize storage supplies the ABI's pointer alignment on x86 and x64.
    let mut storage=vec![0usize;size.div_ceil(std::mem::size_of::<usize>())];
    let data=storage.as_mut_ptr().cast::<u8>();
    unsafe {
        let info=data.cast::<LinkInformation>();(*info).replace=0;(*info).root=parent.as_raw_handle();(*info).length=(wide.len()*2) as u32;
        std::ptr::copy_nonoverlapping(wide.as_ptr().cast::<u8>(),data.add(offset),wide.len()*2);
    }
    let mut status=[0usize;2];
    let result=unsafe{NtSetInformationFile(source.as_raw_handle(),status.as_mut_ptr(),data.cast(),size as u32,11)};
    if result<0{return Err(nt_error("创建硬链接",result));}
    Ok(())
}
fn symbolic_data(target:&Path)->Result<Vec<u8>,String>{
    let print:Vec<_>=target.as_os_str().encode_wide().collect();
    let substitute:Vec<_>="\\??\\".encode_utf16().chain(print.iter().copied()).collect();
    let length=12+(substitute.len()+print.len()+2)*2;
    if length+8>16384{return Err("符号链接目标过长".into());}
    let mut data=Vec::with_capacity(length+8);
    data.extend(0xa000000cu32.to_le_bytes());data.extend((length as u16).to_le_bytes());data.extend(0u16.to_le_bytes());
    for value in [0,(substitute.len()*2) as u16,((substitute.len()+1)*2) as u16,(print.len()*2) as u16]{data.extend(value.to_le_bytes());}
    data.extend(0u32.to_le_bytes());
    for value in substitute.into_iter().chain([0]).chain(print).chain([0]){data.extend(value.to_le_bytes());}
    Ok(data)
}
// Junctions use the mount-point reparse format: two UTF-16 names without
// symbolic-link flags. Validate the buffer before creating an owned directory.
fn junction_data(target:&Path)->Result<Vec<u8>,String>{
    let print:Vec<_>=target.as_os_str().encode_wide().collect();
    let substitute:Vec<_>="\\??\\".encode_utf16().chain(print.iter().copied()).collect();
    let length=8+(substitute.len()+print.len()+2)*2;
    if length+8>16384{return Err("目录联接点目标过长".into());}
    let mut data=Vec::with_capacity(length+8);
    data.extend(0xa0000003u32.to_le_bytes());data.extend((length as u16).to_le_bytes());data.extend(0u16.to_le_bytes());
    for value in [0,(substitute.len()*2) as u16,((substitute.len()+1)*2) as u16,(print.len()*2) as u16]{data.extend(value.to_le_bytes());}
    for value in substitute.into_iter().chain([0]).chain(print).chain([0]){data.extend(value.to_le_bytes());}
    Ok(data)
}
fn ioctl(file:&File,code:u32,input:&[u8],output:&mut [u8])->Result<usize,String>{
    let mut returned=0;
    if unsafe{DeviceIoControl(file.as_raw_handle(),code,input.as_ptr(),input.len() as u32,output.as_mut_ptr(),output.len() as u32,&mut returned,std::ptr::null_mut())}==0 {
        return Err(format!("设置或核验链接：{}",std::io::Error::last_os_error()));
    }
    if returned as usize>output.len(){return Err("无效链接响应长度".into());}
    Ok(returned as usize)
}
fn dispose_owned(file:&File)->Result<(),String>{
    let delete=1u8;
    if unsafe{SetFileInformationByHandle(file.as_raw_handle(),4,(&delete as *const u8).cast(),1)}==0 {
        return Err(format!("本次创建项目无法按句柄清理，已保留：{}",std::io::Error::last_os_error()));
    }
    Ok(())
}
fn wsl_data(target:&str)->Result<Vec<u8>,String>{
    if target.is_empty() || target.contains('\0'){return Err("请输入不含空字符的 WSL 目标路径".into());}
    if target.len()+12>16384{return Err("WSL 链接目标过长".into());}
    let mut data=Vec::with_capacity(target.len()+12);
    data.extend(0xa000001du32.to_le_bytes());data.extend(((target.len()+4) as u16).to_le_bytes());data.extend(0u16.to_le_bytes());
    data.extend(2u32.to_le_bytes());data.extend(target.as_bytes());Ok(data)
}
// Text-target links never open, read or pin a target for mutation. A missing
// target is valid. Only the destination is a publication boundary.
fn create_text_link(target:&str,parent:&str,name:&str,kind:LinkKind,opts:&EngineOptions)->Result<CreatedLink,String>{
    if opts.overwrite!="error"{return Err("链接创建不允许覆盖".into());}
    let (target,data)=match kind{
        LinkKind::Junction=>{
            let path=checked_path(target)?;
            check_junction_target(&path)?;
            let data=junction_data(&path)?;(path.display().to_string(),data)
        },
        LinkKind::Wsl=>(target.to_owned(),wsl_data(target)?),
        _=>return Err("无效文本链接类型".into()),
    };
    let parent_path=checked_path(parent)?;let leaf=checked_name(name)?;let path=parent_path.join(&leaf);
    let mut guards=CheckedDirectories::default();guards.check(&parent_path)?;guards.allow_publication(&parent_path)?;
    let parent=parent_pin(&parent_path,&mut guards)?;let parent_id=directory_identity(&parent)?;
    let transaction=super::super::file_workspace_transaction::Transaction::begin()
        .map_err(|e|format!("当前安全实现需要可用的文件事务；尚未创建链接：{e}"))?;
    let lease=transaction.lock_target(&parent_path,parent_id).map_err(|e|format!("无法取得创建位置的安全租约；尚未创建链接：{e}"))?;drop(lease);
    let ctl=opts.control.clone().unwrap_or_default();ctl.check()?;
    #[cfg(test)] BEFORE_LINK.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
    ctl.check()?;
    let (id,attrs)=directory_identity_and_attributes(&parent)?;if id!=parent_id || attrs&0x410!=0x10{return Err("创建位置身份或类型已改变".into());}
    let output=child(&parent,leaf.as_os_str(),Some(kind==LinkKind::Junction),0x110182,1)?;
    let result=(||{
        ioctl(&output,0x900a4,&data,&mut [])?;
        #[cfg(test)] if FAIL_TEXT_VERIFY.with(|flag|flag.replace(false)){return Err("injected text-link verification failure".into());}
        let mut observed=vec![0u8;16384];let size=ioctl(&output,0x900a8,&[],&mut observed)?;
        if observed[..size]!=data{return Err("链接数据核验不符".into());}Ok(())
    })();
    if let Err(error)=result{return match dispose_owned(&output){Ok(())=>Err(error),Err(cleanup)=>Err(format!("{error}；{cleanup}"))};}
    let (id,attrs)=directory_identity_and_attributes(&parent).map_err(|e|format!("链接已创建，位置核验失败；已保留：{e}"))?;
    if id!=parent_id || attrs&0x410!=0x10{return Err("链接已创建，位置身份或类型核验失败；已保留".into());}
    transaction.rollback().map_err(|e|format!("链接已创建，位置租约释放失败；已保留：{e}"))?;
    Ok(CreatedLink{path:path.display().to_string(),target,kind})
}
pub fn create_link(target:&str,parent:&str,name:&str,kind:LinkKind,opts:&EngineOptions)->Result<CreatedLink,String>{
    if matches!(kind,LinkKind::Junction|LinkKind::Wsl){return create_text_link(target,parent,name,kind,opts);}
    if opts.overwrite!="error" {return Err("链接创建不允许覆盖".into());}
    let target=checked_path(target)?;let parent=checked_path(parent)?;let leaf=checked_name(name)?;
    let output_path=parent.join(&leaf);
    let source_parent=target.parent().ok_or("不能将磁盘根目录作为链接目标")?;
    let source_name=target.file_name().ok_or("请选择普通文件或目录")?;
    let mut guards=CheckedDirectories::default();
    guards.check(source_parent)?;guards.check(&parent)?;
    guards.allow_publication(&parent)?;
    let parent_path=parent.clone();
    let source_parent_pin=parent_pin(source_parent,&mut guards)?;
    let parent=parent_pin(&parent,&mut guards)?;
    let parent_id=directory_identity(&parent)?;
    // The kernel opens a publication parent for WRITE when linking, so its pin
    // must share WRITE. A transaction WRITE_ATTRIBUTES lease separately excludes
    // reparse/EA writers, including writers opened before the lease (lock fails).
    // Dropping Locked closes the reader that denies WRITE sharing; the lease
    // remains until rollback, while our non-delete-sharing parent pin remains.
    let transaction=super::super::file_workspace_transaction::Transaction::begin()
        .map_err(|e|format!("当前安全实现需要可用的文件事务；尚未创建链接：{e}"))?;
    let lease=transaction.lock_target(&parent_path,parent_id)
        .map_err(|e|format!("无法取得创建位置的安全租约；尚未创建链接：{e}"))?;drop(lease);
    // READ_DATA/LIST_DIRECTORY participates in Windows sharing checks;
    // READ_ATTRIBUTES-only opens do not reliably block rename/delete. No data
    // is read, but the participating handle must deny WRITE and DELETE.
    let source=child(&source_parent_pin,source_name,None,0x100081,1)?;
    let metadata=source.metadata().map_err(|e|e.to_string())?;
    if metadata.file_attributes()&0x400!=0 || (!metadata.is_file()&&!metadata.is_dir()) {return Err("链接目标必须是普通文件或目录".into());}
    if metadata.is_dir()!=matches!(kind,LinkKind::SymbolicDirectory|LinkKind::Junction){return Err("链接类型与目标类型不符".into());}
    let source_id=directory_identity(&source)?;
    let ctl=opts.control.clone().unwrap_or_default();ctl.check()?;
    #[cfg(test)] BEFORE_LINK.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
    ctl.check()?;
    let (current_id,current_attrs)=directory_identity_and_attributes(&parent)?;
    if current_id!=parent_id || current_attrs&0x410!=0x10 {return Err("创建位置身份或类型已改变".into());}
    match kind {
        LinkKind::Junction|LinkKind::Wsl=>unreachable!("text links dispatched before source pinning"),
        LinkKind::Hard=>{
            if source_id.0!=directory_identity(&parent)?.0 {return Err("硬链接必须位于同一卷".into());}
            hard(&source,&parent,leaf.as_os_str())?;
            // Source denies DELETE sharing, including through the new name.
            // Never delete by name if post-creation observation fails.
            let output=child(&parent,leaf.as_os_str(),None,0x100080,1)
                .map_err(|e|format!("硬链接已创建，核验失败；为保护内容已保留：{e}"))?;
            if directory_identity(&output)?!=source_id{return Err("硬链接身份核验失败；已保留项目".into());}
        },
        LinkKind::SymbolicFile|LinkKind::SymbolicDirectory=>{
            let data=symbolic_data(&target)?;
            // Capture the caller's thread-local test hook before the helper.
            #[cfg(test)] let fail_verify=FAIL_SYMBOLIC_VERIFY.with(|flag|flag.replace(false));
            #[cfg(test)] let panic_verify=PANIC_SYMBOLIC_VERIFY.with(|flag|flag.replace(false));
            // The caller retains the exact created handle across helper errors,
            // including restore failure/panic. Cleanup never reopens a pathname.
            let output=std::sync::Mutex::new(None::<File>);
            let result=privilege::run(||{
                let mut receipt=output.lock().unwrap_or_else(|poison|poison.into_inner());
                *receipt=Some(child(&parent,leaf.as_os_str(),Some(kind==LinkKind::SymbolicDirectory),0x110182,1)?);
                let file=receipt.as_ref().ok_or("缺少符号链接创建句柄")?;
                ioctl(file,0x900a4,&data,&mut [])?;
                #[cfg(test)] if panic_verify{panic!("injected symbolic helper panic");}
                #[cfg(test)] if fail_verify{return Err("injected symbolic verification failure".into());}
                let mut observed=vec![0u8;16384];let size=ioctl(file,0x900a8,&[],&mut observed)?;
                if observed[..size]!=data{return Err("符号链接数据核验不符".into());}
                Ok(())
            });
            let output=output.into_inner().unwrap_or_else(|poison|poison.into_inner());
            if let Err(error)=result {return match output {
                Some(file)=>match dispose_owned(&file){Ok(())=>Err(error),Err(cleanup)=>Err(format!("{error}；{cleanup}"))},None=>Err(error),
            };}
        }
    }
    let (current_id,current_attrs)=directory_identity_and_attributes(&parent)
        .map_err(|e|format!("链接已创建，位置核验失败；已保留：{e}"))?;
    if current_id!=parent_id || current_attrs&0x410!=0x10 {return Err("链接已创建，位置身份或类型核验失败；已保留".into());}
    // A metadata-only lease rollback never undoes our non-transacted creation.
    transaction.rollback().map_err(|e|format!("链接已创建，位置租约释放失败；已保留：{e}"))?;
    Ok(CreatedLink{path:output_path.display().to_string(),target:target.display().to_string(),kind})
}
#[cfg(test)] thread_local!{static FAIL_TEXT_VERIFY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)] thread_local!{static BEFORE_LINK:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)] thread_local!{static FAIL_SYMBOLIC_VERIFY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)] thread_local!{static PANIC_SYMBOLIC_VERIFY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)] mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires an already-held symbolic-link privilege; native CI must run explicitly"]
    fn require_symbolic_file_and_directory_success(){
        privilege::require_existing_thread_contexts();
        let root=tempfile::tempdir().unwrap();let sources=root.path().join("sources");let destination=root.path().join("links");
        fs::create_dir(&sources).unwrap();fs::create_dir(&destination).unwrap();
        for directory in [false,true] {
            let caller_before=privilege::snapshot();
            let target=sources.join(if directory{"目录目标"}else{"文件目标.txt"});
            if directory{fs::create_dir(&target).unwrap();fs::write(target.join("child.txt"),b"directory child bytes").unwrap();}
            else{fs::write(&target,b"file target bytes").unwrap();}
            let metadata=fs::metadata(&target).unwrap();let attrs=metadata.file_attributes();let modified=metadata.modified().unwrap();
            let kind=if directory{LinkKind::SymbolicDirectory}else{LinkKind::SymbolicFile};let name=if directory{"directory-link"}else{"file-link"};
            // No privilege probe, alternate API result, or skip branch can turn
            // permission denial into success in this required acceptance test.
            let created=create_link(target.to_str().unwrap(),destination.to_str().unwrap(),name,kind,&EngineOptions::default()).expect("native CI must create both symbolic-link kinds with its existing permission");
            let link=PathBuf::from(&created.path);let link_meta=fs::symlink_metadata(&link).unwrap();
            assert_ne!(link_meta.file_attributes()&0x400,0);assert_eq!(link_meta.file_attributes()&0x10!=0,directory);assert_eq!(fs::read_link(&link).unwrap(),target);
            let parent=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(&destination).unwrap();
            let reparse=child(&parent,link.file_name().unwrap(),None,0x100080,7).unwrap();
            let mut data=vec![0u8;16384];let length=ioctl(&reparse,0x900a8,&[],&mut data).unwrap();
            assert_eq!(u32::from_le_bytes(data[..4].try_into().unwrap()),0xa000000c);assert_eq!(data[..length],symbolic_data(&target).unwrap());drop((reparse,parent));
            assert_eq!(fs::read(if directory{link.join("child.txt")}else{link.clone()}).unwrap(),if directory{b"directory child bytes".as_slice()}else{b"file target bytes".as_slice()});
            assert!(create_link(target.to_str().unwrap(),destination.to_str().unwrap(),name,kind,&EngineOptions::default()).is_err());
            assert_eq!(fs::read_link(&link).unwrap(),target);
            let conflict=destination.join(if directory{"directory-conflict"}else{"file-conflict"});fs::write(&conflict,b"foreign bytes").unwrap();
            assert!(create_link(target.to_str().unwrap(),destination.to_str().unwrap(),conflict.file_name().unwrap().to_str().unwrap(),kind,&EngineOptions::default()).is_err());assert_eq!(fs::read(&conflict).unwrap(),b"foreign bytes");
            let failing=if directory{"directory-failed"}else{"file-failed"};FAIL_SYMBOLIC_VERIFY.with(|flag|flag.set(true));
            let error=create_link(target.to_str().unwrap(),destination.to_str().unwrap(),failing,kind,&EngineOptions::default()).unwrap_err();
            assert!(error.contains("injected symbolic verification failure"),"{error}");assert!(fs::symlink_metadata(destination.join(failing)).is_err());
            for panic in [false,true] {
                if panic{PANIC_SYMBOLIC_VERIFY.with(|flag|flag.set(true));}else{privilege::FAIL_RESTORE.with(|flag|flag.set(true));}
                let failed=if panic{"panic-cleanup"}else{"restore-cleanup"};
                let error=create_link(target.to_str().unwrap(),destination.to_str().unwrap(),failed,kind,&EngineOptions::default()).unwrap_err();
                assert!(error.contains(if panic{"操作线程异常结束"}else{"injected thread identity restore failure"}),"{error}");
                assert!(fs::symlink_metadata(destination.join(failed)).is_err());assert_eq!(privilege::snapshot(),caller_before);
            }
            let after=fs::metadata(&target).unwrap();assert_eq!(after.file_attributes(),attrs);assert_eq!(after.modified().unwrap(),modified);assert_eq!(after.len(),metadata.len());
            assert_eq!(fs::read(if directory{target.join("child.txt")}else{target.clone()}).unwrap(),if directory{b"directory child bytes".as_slice()}else{b"file target bytes".as_slice()});
            if directory{assert_eq!(fs::read_dir(&target).unwrap().count(),1);fs::remove_dir(&link).unwrap();}else{fs::remove_file(&link).unwrap();}
            assert_eq!(privilege::snapshot(),caller_before);
            println!("required symbolic acceptance passed: {kind:?}, native reparse tag/target/type, real target access, no-replace and owned-handle failure cleanup");
        }
        assert_eq!(fs::read_dir(&destination).unwrap().count(),2);
    }
    #[test] fn junction_creation_access_no_replace_and_owned_cleanup(){
        let root=tempfile::tempdir().unwrap();let source=root.path().join("目录目标");let parent=root.path().join("links");
        fs::create_dir(&source).unwrap();fs::create_dir(&parent).unwrap();fs::write(source.join("child.txt"),b"keep child bytes").unwrap();
        let before=fs::metadata(&source).unwrap();let caller=privilege::snapshot();let opts=EngineOptions::default();
        let created=create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"联接点",LinkKind::Junction,&opts).unwrap();
        assert_eq!(created.kind,LinkKind::Junction);let link=PathBuf::from(&created.path);
        let metadata=fs::symlink_metadata(&link).unwrap();assert_eq!(metadata.file_attributes()&0x410,0x410);
        assert_eq!(fs::read_link(&link).unwrap(),source);assert_eq!(fs::read(link.join("child.txt")).unwrap(),b"keep child bytes");
        let pin=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(&parent).unwrap();
        let output=child(&pin,link.file_name().unwrap(),None,0x100080,7).unwrap();let mut observed=vec![0u8;16384];
        let size=ioctl(&output,0x900a8,&[],&mut observed).unwrap();assert_eq!(observed[..size],junction_data(&source).unwrap());drop((output,pin));
        assert!(create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"联接点",LinkKind::Junction,&opts).is_err());
        let foreign=parent.join("foreign");fs::create_dir(&foreign).unwrap();fs::write(foreign.join("payload"),b"foreign").unwrap();
        assert!(create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"foreign",LinkKind::Junction,&opts).is_err());
        assert_eq!(fs::read(foreign.join("payload")).unwrap(),b"foreign");
        FAIL_TEXT_VERIFY.with(|flag|flag.set(true));
        let error=create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"failed",LinkKind::Junction,&opts).unwrap_err();
        assert!(error.contains("injected text-link verification failure"),"{error}");assert!(fs::symlink_metadata(parent.join("failed")).is_err());
        let racing=parent.join("racing");let copy=racing.clone();
        BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||{fs::create_dir(&copy).unwrap();fs::write(copy.join("payload"),b"racing foreign").unwrap();})));
        assert!(create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"racing",LinkKind::Junction,&opts).is_err());
        assert_eq!(fs::read(racing.join("payload")).unwrap(),b"racing foreign");
        let file=source.join("child.txt");assert!(create_link(file.to_str().unwrap(),parent.to_str().unwrap(),"wrong",LinkKind::Junction,&opts).is_err());
        assert_eq!(fs::read_dir(&parent).unwrap().count(),3);assert_eq!(fs::read_dir(&source).unwrap().count(),1);
        let after=fs::metadata(&source).unwrap();assert_eq!(after.file_attributes(),before.file_attributes());assert_eq!(after.modified().unwrap(),before.modified().unwrap());
        assert_eq!(privilege::snapshot(),caller);fs::remove_dir(&link).unwrap();assert_eq!(fs::read(file).unwrap(),b"keep child bytes");
        println!("junction acceptance: native mount-point bytes, Unicode target, real child access, no-replace/race protection, owned-handle failure cleanup, source and caller context preserved");
    }
    #[test] fn dangling_junction_checks_existing_prefixes_without_target_creation(){
        use std::os::windows::process::CommandExt;
        let root=tempfile::tempdir().unwrap();let parent=root.path().join("links");fs::create_dir(&parent).unwrap();
        let missing=root.path().join("missing/child/目录");let opts=EngineOptions::default();
        let made=create_link(missing.to_str().unwrap(),parent.to_str().unwrap(),"dangling",LinkKind::Junction,&opts).unwrap();
        assert!(!root.path().join("missing").exists());assert_eq!(fs::read_link(&made.path).unwrap(),missing);
        fs::create_dir_all(&missing).unwrap();fs::write(missing.join("payload"),b"later bytes").unwrap();assert_eq!(fs::read(Path::new(&made.path).join("payload")).unwrap(),b"later bytes");
        let alias=root.path().join("alias");let output=std::process::Command::new("cmd.exe").args(["/c","mklink","/J"]).arg(&alias).arg(root.path().join("missing")).creation_flags(0x08000000).output().unwrap();assert!(output.status.success());
        for target in [alias.clone(),alias.join("absent/subdir"),missing.join("payload"),missing.join("payload/absent")] {
            assert!(create_link(target.to_str().unwrap(),parent.to_str().unwrap(),"rejected",LinkKind::Junction,&opts).is_err(),"{target:?}");
        }
        fs::remove_dir(&alias).unwrap();fs::remove_dir(&made.path).unwrap();assert_eq!(fs::read(missing.join("payload")).unwrap(),b"later bytes");
    }
    #[test] fn wsl_raw_targets_native_bytes_conflicts_and_cleanup(){
        let root=tempfile::tempdir().unwrap();let opts=EngineOptions::default();let caller=privilege::snapshot();
        for (i,target) in ["/home/用户/不存在", "../relative path/文件", "C:\\raw-text", "/mnt/g/sample"].iter().enumerate(){
            let name=format!("wsl-{i}");let made=create_link(target,root.path().to_str().unwrap(),&name,LinkKind::Wsl,&opts).unwrap();
            assert_eq!(&made.target,target);let meta=fs::symlink_metadata(&made.path).unwrap();assert_ne!(meta.file_attributes()&0x400,0);assert_eq!(meta.file_attributes()&0x10,0);
            let parent=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(root.path()).unwrap();
            let output=child(&parent,std::ffi::OsStr::new(&name),None,0x100080,7).unwrap();let mut observed=vec![0u8;16384];let size=ioctl(&output,0x900a8,&[],&mut observed).unwrap();
            assert_eq!(u32::from_le_bytes(observed[..4].try_into().unwrap()),0xa000001d);assert_eq!(u32::from_le_bytes(observed[8..12].try_into().unwrap()),2);assert_eq!(&observed[12..size],target.as_bytes());drop((output,parent));
            assert!(create_link(target,root.path().to_str().unwrap(),&name,LinkKind::Wsl,&opts).is_err());fs::remove_file(&made.path).unwrap();
        }
        for name in ["", "..", "x:ads", "a/b", "CON", "x."]{assert!(create_link("/target",root.path().to_str().unwrap(),name,LinkKind::Wsl,&opts).is_err());}
        let mut overwrite=EngineOptions::default();overwrite.overwrite="overwrite".into();assert!(create_link("/target",root.path().to_str().unwrap(),"bad",LinkKind::Wsl,&overwrite).is_err());
        let writer=File::options().access_mode(0x100).share_mode(7).custom_flags(0x02200000).open(root.path()).unwrap();
        assert!(create_link("/target",root.path().to_str().unwrap(),"writer",LinkKind::Wsl,&opts).is_err());drop(writer);
        for target in ["", "bad\0target", &"x".repeat(16373)]{assert!(create_link(target,root.path().to_str().unwrap(),"invalid",LinkKind::Wsl,&opts).is_err());}
        let max="x".repeat(16372);let made=create_link(&max,root.path().to_str().unwrap(),"max",LinkKind::Wsl,&opts).unwrap();fs::remove_file(&made.path).unwrap();
        fs::write(root.path().join("foreign"),b"foreign bytes").unwrap();assert!(create_link("/target",root.path().to_str().unwrap(),"foreign",LinkKind::Wsl,&opts).is_err());assert_eq!(fs::read(root.path().join("foreign")).unwrap(),b"foreign bytes");
        FAIL_TEXT_VERIFY.with(|flag|flag.set(true));let error=create_link("../target",root.path().to_str().unwrap(),"failed",LinkKind::Wsl,&opts).unwrap_err();assert!(error.contains("injected text-link verification failure"));assert!(fs::symlink_metadata(root.path().join("failed")).is_err());
        let racing=root.path().join("racing");let copy=racing.clone();BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||fs::write(copy,b"racing bytes").unwrap())));
        assert!(create_link("/target",root.path().to_str().unwrap(),"racing",LinkKind::Wsl,&opts).is_err());assert_eq!(fs::read(racing).unwrap(),b"racing bytes");
        let ctl=JobControl::new();let cancel=ctl.clone();let mut cancel_opts=EngineOptions::default();cancel_opts.control=Some(ctl);BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        assert!(create_link("/target",root.path().to_str().unwrap(),"cancelled",LinkKind::Wsl,&cancel_opts).is_err());assert!(fs::symlink_metadata(root.path().join("cancelled")).is_err());assert_eq!(fs::read_dir(root.path()).unwrap().count(),2);assert_eq!(privilege::snapshot(),caller);
        println!("WSL native acceptance: LX tag/version2/raw UTF8, Unicode relative/absolute targets, maximum buffer, file shape, conflicts/race/cancel and owned-handle cleanup; Linux runtime not tested");
    }
    #[test] fn junction_parent_lease_rejects_mutation(){
        let root=tempfile::tempdir().unwrap();let source=root.path().join("target");let parent=root.path().join("links");
        fs::create_dir(&source).unwrap();fs::create_dir(&parent).unwrap();let dst=parent.clone();
        BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||{
            assert!(fs::rename(&dst,dst.with_extension("moved")).is_err());
            for access in [0x100,0x18,0x40000000]{assert!(File::options().access_mode(access).share_mode(7).custom_flags(0x02200000).open(&dst).is_err());}
        })));
        create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"link",LinkKind::Junction,&EngineOptions::default()).unwrap();fs::remove_dir(parent.join("link")).unwrap();
        let writer=File::options().access_mode(0x100).share_mode(7).custom_flags(0x02200000).open(&parent).unwrap();
        assert!(create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"blocked",LinkKind::Junction,&EngineOptions::default()).is_err());
        assert_eq!(fs::read_dir(&parent).unwrap().count(),0);drop(writer);
        let ctl=JobControl::new();let cancel=ctl.clone();let mut opts=EngineOptions::default();opts.control=Some(ctl);
        BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        assert!(create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"cancelled",LinkKind::Junction,&opts).is_err());assert_eq!(fs::read_dir(&parent).unwrap().count(),0);
    }
    #[test] fn hard_link_identity_and_existing_data(){
        let root=tempfile::tempdir().unwrap();let source=root.path().join("原文件.txt");fs::write(&source,b"original bytes").unwrap();
        let result=create_link(source.to_str().unwrap(),root.path().to_str().unwrap(),"新链接.txt",LinkKind::Hard,&EngineOptions::default()).unwrap();
        let a=File::open(&source).unwrap();let b=File::open(&result.path).unwrap();assert_eq!(directory_identity(&a).unwrap(),directory_identity(&b).unwrap());drop((a,b));
        fs::write(&result.path,b"shared fixture bytes").unwrap();assert_eq!(fs::read(&source).unwrap(),b"shared fixture bytes");
        assert!(create_link(source.to_str().unwrap(),root.path().to_str().unwrap(),"新链接.txt",LinkKind::Hard,&EngineOptions::default()).is_err());
        assert_eq!(fs::read(&source).unwrap(),b"shared fixture bytes");
    }
    #[test] fn hard_link_share_probe(){
        for parent_share in [1,3,7] {for source_share in [1,3,7]{
            let root=tempfile::tempdir().unwrap();let target=root.path().join("source");fs::write(&target,b"keep").unwrap();
            let parent=File::options().access_mode(0x81).share_mode(parent_share).custom_flags(0x02200000).open(root.path()).unwrap();
            let source=child(&parent,std::ffi::OsStr::new("source"),None,0x100080,source_share).unwrap();
            let result=hard(&source,&parent,std::ffi::OsStr::new("link"));
            println!("hardlink share probe parent={parent_share}, source={source_share}: {result:?}");
            assert_eq!(result.is_ok(),parent_share!=1);
            drop((parent,source));assert_eq!(fs::read(&target).unwrap(),b"keep");
        }}
    }
    #[test] fn hard_link_parent_transaction_probe(){
        let root=tempfile::tempdir().unwrap();fs::write(root.path().join("source"),b"keep").unwrap();
        let parent=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(root.path()).unwrap();
        let source=child(&parent,std::ffi::OsStr::new("source"),None,0x100080,1).unwrap();
        let tx=super::super::super::file_workspace_transaction::Transaction::begin().unwrap();
        let lock=tx.lock_target(root.path(),directory_identity(&parent).unwrap()).unwrap();drop(lock);
        hard(&source,&parent,std::ffi::OsStr::new("link")).unwrap();
        tx.rollback().unwrap();
        assert_eq!(fs::read(root.path().join("link")).unwrap(),b"keep");
    }
    #[test] fn symbolic_existing_permission_probe(){
        #[link(name="kernel32")] unsafe extern "system" {fn CreateSymbolicLinkW(link:*const u16,target:*const u16,flags:u32)->u8;}
        let root=tempfile::tempdir().unwrap();
        for directory in [false,true] {
            let target=root.path().join(if directory{"target-dir"}else{"target-file"});
            if directory{fs::create_dir(&target).unwrap();}else{fs::write(&target,b"keep").unwrap();}
            let kind=if directory{LinkKind::SymbolicDirectory}else{LinkKind::SymbolicFile};
            let candidate=create_link(target.to_str().unwrap(),root.path().to_str().unwrap(),if directory{"candidate-dir"}else{"candidate-file"},kind,&EngineOptions::default());
            let api=root.path().join(if directory{"api-dir"}else{"api-file"});
            let link:Vec<_>=api.as_os_str().encode_wide().chain([0]).collect();let to:Vec<_>=target.as_os_str().encode_wide().chain([0]).collect();
            let api_ok=unsafe{CreateSymbolicLinkW(link.as_ptr(),to.as_ptr(),2|u32::from(directory))}!=0;
            let api_error=if api_ok{None}else{Some(std::io::Error::last_os_error().raw_os_error())};
            println!("symbolic permission fixture directory={directory}: handle_candidate={candidate:?}; CreateSymbolicLinkW(ALLOW_UNPRIVILEGED_CREATE)={api_ok}, error={api_error:?}");
            if api_ok {assert!(candidate.is_ok(),"官方API可成功而候选失败，权限兼容缺口：{candidate:?}");}
            if let Ok(created)=&candidate {
                let entry=fs::symlink_metadata(&created.path).unwrap();assert_ne!(entry.file_attributes()&0x400,0);
                assert_eq!(fs::read_link(&created.path).unwrap(),target);
                if directory{fs::remove_dir(&created.path).unwrap();}else{fs::remove_file(&created.path).unwrap();}
            }else{
                let error=candidate.as_ref().unwrap_err();
                assert!(error.contains("os error 1300")||error.contains("os error 1314"),"unexpected candidate failure: {error}");
                assert_eq!(api_error,Some(Some(1314)),"official API did not report missing symbolic-link privilege");
                assert!(!root.path().join(if directory{"candidate-dir"}else{"candidate-file"}).try_exists().unwrap());
            }
            if api_ok {if directory{fs::remove_dir(&api).unwrap();}else{fs::remove_file(&api).unwrap();}}
            if !directory{assert_eq!(fs::read(&target).unwrap(),b"keep");}
        }
    }
    #[test] fn unsafe_inputs_and_racing_foreign_target(){
        let root=tempfile::tempdir().unwrap();let source=root.path().join("source");fs::write(&source,b"source").unwrap();
        let parent=root.path().to_str().unwrap();let opts=EngineOptions::default();
        for name in ["",".","..","../x","a/b","a\\b","x:ads","CON","NUL.txt","x.","x ","x\0"] {
            assert!(create_link(source.to_str().unwrap(),parent,name,LinkKind::Hard,&opts).is_err(),"{name:?}");
        }
        for path in ["C:relative","\\\\server\\share\\file","\\\\?\\C:\\file","C:\\file:ads","C:\\a\\..\\file"] {
            assert!(create_link(path,parent,"new",LinkKind::Hard,&opts).is_err(),"{path:?}");
        }
        assert!(create_link(source.to_str().unwrap(),parent,"wrong",LinkKind::SymbolicDirectory,&opts).is_err());
        let foreign=root.path().join("foreign");let copy=foreign.clone();
        BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||fs::write(copy,b"foreign data").unwrap())));
        assert!(create_link(source.to_str().unwrap(),parent,"foreign",LinkKind::Hard,&opts).is_err());
        assert_eq!(fs::read(&foreign).unwrap(),b"foreign data");assert_eq!(fs::read(&source).unwrap(),b"source");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(),2);
    }
    #[test] fn pinned_source_parent_and_parent_lease_exclude_mutation(){
        let root=tempfile::tempdir().unwrap();let parent=root.path().join("parent");fs::create_dir(&parent).unwrap();
        let source=parent.join("source");fs::write(&source,b"original").unwrap();
        let attempt_parent=parent.clone();let attempt_source=source.clone();
        BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||{
            assert!(fs::rename(&attempt_source,attempt_source.with_extension("moved")).is_err());
            assert!(fs::remove_file(&attempt_source).is_err());
            assert!(fs::write(&attempt_source,b"foreign").is_err());
            assert!(fs::rename(&attempt_parent,attempt_parent.with_extension("moved")).is_err());
            // WRITE_ATTRIBUTES, WRITE_EA and WRITE_DATA are prerequisites for
            // changing directory reparse/EA metadata. Existing transactional
            // lease must exclude them despite the publication WRITE sharing.
            for access in [0x100,0x18,0x40000000] {
                assert!(File::options().access_mode(access).share_mode(7).custom_flags(0x02200000).open(&attempt_parent).is_err(),"parent writer {access:x}");
            }
        })));
        create_link(source.to_str().unwrap(),parent.to_str().unwrap(),"link",LinkKind::Hard,&EngineOptions::default()).unwrap();
        assert_eq!(fs::read(&source).unwrap(),b"original");assert_eq!(fs::read(parent.join("link")).unwrap(),b"original");
    }
    #[test] fn preexisting_parent_writer_fails_without_output_and_cancel_preserves_all(){
        let root=tempfile::tempdir().unwrap();let source=root.path().join("source");fs::write(&source,b"keep").unwrap();
        let writer=File::options().access_mode(0x100).share_mode(7).custom_flags(0x02200000).open(root.path()).unwrap();
        assert!(create_link(source.to_str().unwrap(),root.path().to_str().unwrap(),"absent",LinkKind::Hard,&EngineOptions::default()).is_err());
        assert!(!root.path().join("absent").exists());drop(writer);
        let ctl=JobControl::new();let cancel=ctl.clone();let mut opts=EngineOptions::default();opts.control=Some(ctl);
        BEFORE_LINK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        assert!(create_link(source.to_str().unwrap(),root.path().to_str().unwrap(),"cancelled",LinkKind::Hard,&opts).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(),1);assert_eq!(fs::read(source).unwrap(),b"keep");
    }
    #[test] fn reparse_source_or_ancestor_is_rejected_and_existing_entries_survive(){
        use std::os::windows::process::CommandExt;
        let root=tempfile::tempdir().unwrap();let real=root.path().join("real");fs::create_dir(&real).unwrap();
        fs::write(real.join("source"),b"keep").unwrap();let junction=root.path().join("junction");
        let output=std::process::Command::new("cmd.exe").args(["/c","mklink","/J"]).arg(&junction).arg(&real).creation_flags(0x08000000).output().unwrap();assert!(output.status.success());
        let opts=EngineOptions::default();
        assert!(create_link(junction.join("source").to_str().unwrap(),root.path().to_str().unwrap(),"rejected",LinkKind::Hard,&opts).is_err());
        assert!(create_link(junction.to_str().unwrap(),root.path().to_str().unwrap(),"rejected",LinkKind::SymbolicDirectory,&opts).is_err());
        assert!(create_link(real.join("source").to_str().unwrap(),junction.to_str().unwrap(),"rejected",LinkKind::Hard,&opts).is_err());
        assert!(create_link("/home/target",junction.to_str().unwrap(),"rejected",LinkKind::Wsl,&opts).is_err());
        for kind in [LinkKind::Hard,LinkKind::SymbolicFile,LinkKind::SymbolicDirectory,LinkKind::Junction] {
            let source=if matches!(kind,LinkKind::SymbolicDirectory|LinkKind::Junction){real.clone()}else{real.join("source")};
            for name in ["real","junction"] {assert!(create_link(source.to_str().unwrap(),root.path().to_str().unwrap(),name,kind,&opts).is_err());}
        }
        assert_eq!(fs::read(real.join("source")).unwrap(),b"keep");assert_eq!(fs::read_dir(&real).unwrap().count(),1);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(),2);fs::remove_dir(junction).unwrap();
    }
}

#[cfg(test)]
#[path="file_workspace_link_edit_probe.rs"]
mod edit_probe;

#[path="file_workspace_link_edit.rs"]
mod edit;
pub use edit::{inspect_link,edit_link,LinkReceipt,InspectedLink,EditedLink};
