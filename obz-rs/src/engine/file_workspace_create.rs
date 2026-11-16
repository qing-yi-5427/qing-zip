//! Single-component, no-replace local creation through the existing publishers.
use super::*;

pub fn create_item(parent: &str, name: &str, directory: bool, opts: &EngineOptions) -> Result<LocalEntry,String> {
    if parent.is_empty() { return Err("请先选择一个本地文件夹".into()); }
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err("请输入单个文件或文件夹名称".into());
    }
    let leaf = crate::extract::relative_path(name)?;
    if leaf.components().count()!=1 {return Err("名称必须是单个路径组件".into());}
    if opts.overwrite!="error" {return Err("新建项目不允许覆盖已有内容".into());}
    let ctl=opts.control.clone().unwrap_or_default();ctl.check()?;
    let parent=local_path(parent)?;
    let mut guards=CheckedDirectories::default();guards.check(&parent)?;
    guards.allow_publication(&parent)?;
    let target=parent.join(leaf);
    #[cfg(test)]
    BEFORE_CREATE.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
    let output=if directory {
        let pin=File::options().access_mode(0x200a0).share_mode(3).custom_flags(0x02200000)
            .open(&parent).map_err(|e|e.to_string())?;
        if directory_identity(&pin)?!=directory_identity(&guards.held.get(&parent).ok_or("缺少父目录锁")?.0)? {
            return Err("目标父目录已改变".into());
        }
        let mut temp=crate::private_stage::create_local_directory_output(&parent,&pin)?;
        let attempt=(||{
            let proof=LocalStreamProof{digest:streams(temp.as_file(),None,&ctl)?,control:ctl.clone()};
            ctl.check()?;
            temp.commit_local_streams(&target,false,&proof)
        })();
        match (attempt,temp.abort()) {
            (Ok(file),Ok(()))=>file,(Err(e),Ok(()))=>return Err(e),
            (Ok(_),Err(e))=>return Err(e),(Err(e),Err(cleanup))=>return Err(format!("{e}; {cleanup}")),
        }
    } else {
        with_output_temp(&parent,&guards,|temp|{ctl.check()?;publish_output(temp,&target,false)})?
    };
    let metadata=output.metadata().map_err(|e|e.to_string())?;
    if metadata.is_dir()!=directory || metadata.file_attributes()&0x400!=0 {
        return Err("新建项目类型验证失败".into());
    }
    Ok(LocalEntry{name:name.into(),path:target.display().to_string(),is_dir:directory,linked:false,size:metadata.len(),revision:super::stamp::entry_revision(&metadata)})
}

#[cfg(test)]
thread_local! {static BEFORE_CREATE:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn local_create_unicode_empty_file_directory_and_conflicts(){
        let root=tempfile::tempdir().unwrap();let opts=EngineOptions::default();let p=root.path().to_str().unwrap();
        let file=create_item(p,"新建 空文件.txt",false,&opts).unwrap();assert_eq!(fs::read(&file.path).unwrap(),b"");
        let dir=create_item(p,"新建 文件夹",true,&opts).unwrap();assert!(Path::new(&dir.path).is_dir());
        fs::write(&file.path,b"existing").unwrap();
        for directory in [false,true] {assert!(create_item(p,"新建 空文件.txt",directory,&opts).is_err());assert!(create_item(p,"新建 文件夹",directory,&opts).is_err());}
        assert_eq!(fs::read(&file.path).unwrap(),b"existing");assert_eq!(fs::read_dir(root.path()).unwrap().count(),2);
    }
    #[test] fn local_create_rejects_unsafe_names_and_cancel(){
        let root=tempfile::tempdir().unwrap();let p=root.path().to_str().unwrap();let mut opts=EngineOptions::default();
        for name in ["",".","..","../escape","a/b","a\\b","x:ads","CON","NUL.txt","a.","a ","C:escape","bad\0name"] {
            assert!(create_item(p,name,false,&opts).is_err(),"{name:?}");
        }
        let ctl=JobControl::new();ctl.cancel();opts.control=Some(ctl);
        assert!(create_item(p,"cancelled",true,&opts).is_err());assert_eq!(fs::read_dir(root.path()).unwrap().count(),0);
    }
    #[test] fn local_create_racing_target_is_never_replaced(){
        for directory in [false,true] {
            let root=tempfile::tempdir().unwrap();let target=root.path().join("competing");let copy=target.clone();
            BEFORE_CREATE.with(|h|*h.borrow_mut()=Some(Box::new(move||fs::write(copy,b"competitor").unwrap())));
            assert!(create_item(root.path().to_str().unwrap(),"competing",directory,&EngineOptions::default()).is_err());
            assert_eq!(fs::read(target).unwrap(),b"competitor");assert_eq!(fs::read_dir(root.path()).unwrap().count(),1);
        }
    }
    #[test] fn local_create_cancel_after_parent_pin_leaves_no_scratch(){
        for directory in [false,true] {
            let root=tempfile::tempdir().unwrap();let ctl=JobControl::new();let cancel=ctl.clone();
            BEFORE_CREATE.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())));
            let mut opts=EngineOptions::default();opts.control=Some(ctl);
            assert!(create_item(root.path().to_str().unwrap(),"cancelled",directory,&opts).is_err());
            assert_eq!(fs::read_dir(root.path()).unwrap().count(),0);
        }
    }
    #[test] fn local_create_parent_pin_blocks_replacement(){
        for directory in [false,true] {
            let root=tempfile::tempdir().unwrap();let parent=root.path().join("parent");fs::create_dir(&parent).unwrap();
            let pinned=parent.clone();BEFORE_CREATE.with(|h|*h.borrow_mut()=Some(Box::new(move||{
                assert!(fs::rename(&pinned,pinned.with_extension("moved")).is_err());
            })));
            create_item(parent.to_str().unwrap(),"created",directory,&EngineOptions::default()).unwrap();
            assert_eq!(fs::read_dir(parent).unwrap().count(),1);
        }
    }
    #[test] fn local_create_junction_parent_is_rejected(){
        use std::os::windows::process::CommandExt;
        let root=tempfile::tempdir().unwrap();let outside=tempfile::tempdir().unwrap();let link=root.path().join("junction");
        let status=std::process::Command::new("cmd.exe").args(["/c","mklink","/J"]).arg(&link).arg(outside.path()).creation_flags(0x08000000).output().unwrap();assert!(status.status.success());
        for directory in [false,true]{assert!(create_item(link.to_str().unwrap(),"no-write",directory,&EngineOptions::default()).is_err());}
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(),0);
        fs::remove_dir(link).unwrap();
    }

    #[test] fn local_create_denied_parent_keeps_acl_and_existing_data(){
        #[link(name="advapi32")] extern "system" {
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,result:*mut *mut c_void,size:*mut u32)->i32;
            fn SetKernelObjectSecurity(file:Handle,information:u32,sd:*mut c_void)->i32;
            fn GetFileSecurityW(path:*const u16,information:u32,sd:*mut u8,length:u32,needed:*mut u32)->i32;
        }
        #[link(name="kernel32")] extern "system" {fn LocalFree(pointer:*mut c_void)->*mut c_void;}
        fn security(path:&Path)->Vec<u8>{
            use std::os::windows::ffi::OsStrExt;
            let wide:Vec<_>=path.as_os_str().encode_wide().chain(Some(0)).collect();let mut size=0;
            unsafe{GetFileSecurityW(wide.as_ptr(),4,std::ptr::null_mut(),0,&mut size);}
            assert!(size>0);let mut data=vec![0u8;size as usize];
            assert_ne!(unsafe{GetFileSecurityW(wide.as_ptr(),4,data.as_mut_ptr(),size,&mut size)},0);data
        }
        let root=tempfile::tempdir().unwrap();let parent=root.path().join("denied");fs::create_dir(&parent).unwrap();
        fs::write(parent.join("existing"),b"keep").unwrap();
        let handle=File::options().access_mode(0x40080).share_mode(7).custom_flags(0x02200000).open(&parent).unwrap();
        struct Restore {handle:File,original:Vec<u8>,restored:bool}
        impl Restore {fn restore(&mut self){assert_ne!(unsafe{SetKernelObjectSecurity(self.handle.as_raw_handle(),0x80000004,self.original.as_ptr() as *mut c_void)},0);self.restored=true;}}
        impl Drop for Restore {fn drop(&mut self){if !self.restored{unsafe{SetKernelObjectSecurity(self.handle.as_raw_handle(),0x80000004,self.original.as_ptr() as *mut c_void);}}}}
        let inherited=security(&parent);assert_ne!(unsafe{SetKernelObjectSecurity(handle.as_raw_handle(),0x80000004,inherited.as_ptr() as *mut c_void)},0);
        let original=security(&parent);let mut restore=Restore{handle,original,restored:false};let text:Vec<_>="D:P(D;;0x00000006;;;WD)(A;;FA;;;OW)(A;;FA;;;SY)".encode_utf16().chain(Some(0)).collect();let mut sd=std::ptr::null_mut();
        assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(),1,&mut sd,std::ptr::null_mut())},0);
        assert_ne!(unsafe{SetKernelObjectSecurity(restore.handle.as_raw_handle(),0x80000004,sd)},0);unsafe{LocalFree(sd);}
        let denied=security(&parent);
        let results=[false,true].map(|directory|create_item(parent.to_str().unwrap(),"not-created",directory,&EngineOptions::default()));
        let unchanged=security(&parent)==denied;let count=fs::read_dir(&parent).unwrap().count();
        // Restore only our fixture through the pre-opened management handle.
        restore.restore();assert_eq!(security(&parent),restore.original);
        assert!(results.iter().all(Result::is_err));assert!(unchanged);assert_eq!(count,1);assert_eq!(fs::read(parent.join("existing")).unwrap(),b"keep");
    }

}
