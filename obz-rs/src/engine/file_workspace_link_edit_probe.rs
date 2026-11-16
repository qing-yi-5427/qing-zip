//! Bounded native fixture: existing-object metadata writers must be excluded.
use super::*;
#[test]
fn existing_reparse_leaf_metadata_writer_probe(){
    for before in [false,true]{for access in [0x100,0x10,0x2]{
        let root=tempfile::tempdir().unwrap();let target=root.path().join("target");fs::create_dir(&target).unwrap();
        let made=create_link(target.to_str().unwrap(),root.path().to_str().unwrap(),"junction",LinkKind::Junction,&EngineOptions::default()).unwrap();
        let parent=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(root.path()).unwrap();
        let writer=if before{Some(child(&parent,std::ffi::OsStr::new("junction"),None,0x100000|access,7))}else{None};
        let pin=child(&parent,std::ffi::OsStr::new("junction"),None,0x100183,1);
        let writer=writer.unwrap_or_else(||child(&parent,std::ffi::OsStr::new("junction"),None,0x100000|access,7));
        let set=writer.as_ref().map(|f|ioctl(f,0x900a4,&junction_data(&target).unwrap(),&mut []));
        let delete=writer.as_ref().map(|f|ioctl(f,0x900ac,&[3,0,0,0xa0,0,0,0,0],&mut []));
        println!("existing leaf pin before={before} access={access:x} pin={:?} writer={:?} SET={set:?} DELETE={delete:?}",pin.as_ref().map(|_|()),writer.as_ref().map(|_|()));
        if access==0x100{assert!(pin.is_ok()&&writer.is_ok());assert!(matches!(set,Ok(Ok(_)))&&matches!(delete,Ok(Ok(_))))}
        else if access==0x10{assert!(writer.is_ok());assert!(matches!(set,Ok(Err(_)))&&matches!(delete,Ok(Err(_))))}
        else if before{assert!(pin.is_err()&&writer.is_ok());}else{assert!(pin.is_ok()&&writer.is_err());}
        if let Ok(pin)=&pin {let _=ioctl(pin,0x900a4,&junction_data(&target).unwrap(),&mut []);}
        drop((writer,pin,parent));fs::remove_dir(&made.path).unwrap();
    }}
}
use std::os::windows::io::OwnedHandle;
#[link(name="ktmw32")] unsafe extern "system" {
    fn CreateTransaction(attrs:Handle,guid:Handle,options:u32,isolation:u32,flags:u32,timeout:u32,description:*const u16)->Handle;
    fn RollbackTransaction(transaction:Handle)->i32;
    fn CommitTransaction(transaction:Handle)->i32;
}
#[link(name="kernel32")] unsafe extern "system" {
    fn CreateFileTransactedW(name:*const u16,access:u32,share:u32,security:Handle,disposition:u32,flags:u32,template:Handle,transaction:Handle,version:Handle,extended:Handle)->Handle;
}
fn transaction()->OwnedHandle{
    let raw=unsafe{CreateTransaction(std::ptr::null_mut(),std::ptr::null_mut(),0,0,0,0,std::ptr::null())};assert!(!raw.is_null()&&raw as isize!=-1);unsafe{OwnedHandle::from_raw_handle(raw)}
}
fn tx_open(tx:&OwnedHandle,path:&Path)->std::io::Result<File>{
    let name:Vec<_>=path.as_os_str().encode_wide().chain([0]).collect();
    let raw=unsafe{CreateFileTransactedW(name.as_ptr(),0x80000102,7,std::ptr::null_mut(),3,0x02200000,std::ptr::null_mut(),tx.as_raw_handle(),std::ptr::null_mut(),std::ptr::null_mut())};
    if raw as isize == -1 {Err(std::io::Error::last_os_error())}else{Ok(unsafe{File::from_raw_handle(raw)})}
}
fn get(file:&File)->Vec<u8>{let mut out=vec![0u8;16384];let n=ioctl(file,0x900a8,&[],&mut out).unwrap();out.truncate(n);out}
#[test]
fn existing_reparse_transactions_exclude_preexisting_metadata_writers(){
    for kind in [LinkKind::Junction,LinkKind::Wsl]{
        let root=tempfile::tempdir().unwrap();let target=root.path().join("old");
        let made=create_link(if kind==LinkKind::Wsl{"/old"}else{target.to_str().unwrap()},root.path().to_str().unwrap(),"link",kind,&EngineOptions::default()).unwrap();
        let parent=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(root.path()).unwrap();let name=std::ffi::OsStr::new("link");
        let writer=child(&parent,name,None,0x100100,7).unwrap();let old=get(&writer);let tx=transaction();let file=tx_open(&tx,Path::new(&made.path));
        let data=if kind==LinkKind::Wsl{wsl_data("/new").unwrap()}else{junction_data(&root.path().join("new")).unwrap()};
        let mutation=file.as_ref().map(|f|ioctl(f,0x900a4,&data,&mut []));println!("{kind:?} preexisting WRITE_ATTRIBUTES tx_open={:?} SET={mutation:?}",file.as_ref().map(|_|()));
        assert!(!matches!(mutation,Ok(Ok(_))),"preexisting writer must block opening or actual mutation");
        drop(file);assert_ne!(unsafe{RollbackTransaction(tx.as_raw_handle())},0);assert_eq!(get(&writer),old);drop((tx,writer,parent));
        if kind==LinkKind::Junction{fs::remove_dir(&made.path).unwrap();}else{fs::remove_file(&made.path).unwrap();}
    }
}
#[test]
fn existing_reparse_transaction_set_delete_commit_and_rollback(){
    for kind in [LinkKind::Junction,LinkKind::Wsl]{for remove in [false,true]{for commit in [false,true]{
        let root=tempfile::tempdir().unwrap();let target=root.path().join("old");
        let made=create_link(if kind==LinkKind::Wsl{"/old"}else{target.to_str().unwrap()},root.path().to_str().unwrap(),"link",kind,&EngineOptions::default()).unwrap();
        let parent=File::options().access_mode(0x81).share_mode(3).custom_flags(0x02200000).open(root.path()).unwrap();let name=std::ffi::OsStr::new("link");
        let before=child(&parent,name,None,0x100080,7).unwrap();let old=get(&before);drop(before);
        let tx=transaction();let file=tx_open(&tx,Path::new(&made.path)).unwrap();let new=if kind==LinkKind::Wsl{wsl_data("../new/目标").unwrap()}else{junction_data(&root.path().join("new")).unwrap()};
        let mut header=vec![0u8;8];header[..4].copy_from_slice(&old[..4]);
        ioctl(&file,if remove{0x900ac}else{0x900a4},if remove{&header}else{&new},&mut []).unwrap();
        for access in [0x100,0x10]{
            let writer=child(&parent,name,None,0x100000|access,7);
            if let Ok(writer)=&writer{
                assert!(ioctl(writer,0x900a4,&old,&mut []).is_err());assert!(ioctl(writer,0x900ac,&header,&mut []).is_err());
            }
            println!("{kind:?} remove={remove} commit={commit} external metadata access={access:x} open={:?}",writer.as_ref().map(|_|()));
        }
        if !remove{assert_eq!(get(&file),new);}drop(file);
        assert_ne!(unsafe{if commit{CommitTransaction(tx.as_raw_handle())}else{RollbackTransaction(tx.as_raw_handle())}},0);drop(tx);
        let after=child(&parent,name,None,0x100080,7).unwrap();
        if remove&&commit{let mut buf=vec![0;16384];assert!(ioctl(&after,0x900a8,&[],&mut buf).is_err());assert_eq!(after.metadata().unwrap().file_attributes()&0x400,0);}else{assert_eq!(get(&after),if commit{new}else{old});}
        drop((after,parent));if kind==LinkKind::Junction{fs::remove_dir(&made.path).unwrap();}else{fs::remove_file(&made.path).unwrap();}
    }}}
}
