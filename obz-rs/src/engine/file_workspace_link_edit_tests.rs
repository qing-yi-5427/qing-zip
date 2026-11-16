use super::*;
use std::io::Read;
#[test] fn strict_parser_actual_targets_and_malformed_data(){
    let junction=junction_data(Path::new("C:\\missing\\目标")).unwrap();let parsed=parse(&junction,true).unwrap();assert_eq!(parsed.target,"C:\\missing\\目标");assert_eq!(parsed.kind,LinkKind::Junction);
    let wsl=wsl_data("../用户/target").unwrap();assert_eq!(parse(&wsl,false).unwrap().target,"../用户/target");assert!(parse(&wsl,true).is_err());
    let symbolic=symbolic_data(Path::new("C:\\real")).unwrap();let mut mismatch=symbolic.clone();let start=20+u16_at(&mismatch,12).unwrap() as usize;
    for (i,word) in "C:\\fake".encode_utf16().enumerate(){mismatch[start+i*2..start+i*2+2].copy_from_slice(&word.to_le_bytes());}
    let parsed=parse(&mismatch,false).unwrap();assert_eq!(parsed.target,"C:\\real");assert_eq!(parsed.print,"C:\\fake");
    let relative=Parsed{kind:LinkKind::SymbolicFile,target:"../old".into(),print:String::new(),relative:true};let data=replacement(&relative,"..\\相对目标").unwrap();let parsed=parse(&data,false).unwrap();assert_eq!(parsed.target,"..\\相对目标");assert!(parsed.relative);
    for rooted in ["\\rooted", "/rooted", "C:drive-relative"]{assert!(replacement(&relative,rooted).is_err(),"{rooted}");}
    assert!(parse(&[],false).is_err());assert!(parse(&symbolic[..19],false).is_err());
    for (pos,bytes) in [(0,0xa0000042u32.to_le_bytes().to_vec()),(4,0u16.to_le_bytes().to_vec()),(6,1u16.to_le_bytes().to_vec()),(8,1u16.to_le_bytes().to_vec()),(10,1u16.to_le_bytes().to_vec()),(12,0xfffeu16.to_le_bytes().to_vec()),(16,2u32.to_le_bytes().to_vec()),(20,0xd800u16.to_le_bytes().to_vec())]{let mut data=symbolic.clone();data[pos..pos+bytes.len()].copy_from_slice(&bytes);assert!(parse(&data,false).is_err(),"offset {pos}");}
    let mut bad=wsl.clone();bad[8]=3;assert!(parse(&bad,false).is_err());bad=wsl.clone();bad[12]=0xff;assert!(parse(&bad,false).is_err());
}
fn raw(path:&Path,access:u32)->File{File::options().access_mode(access).share_mode(7).custom_flags(0x02200000).open(path).unwrap()}
fn attrs(file:&File,value:u32){let basic=[0u64,0,0,0,u64::from(value)];assert_ne!(unsafe{SetFileInformationByHandle(file.as_raw_handle(),0,basic.as_ptr().cast(),40)},0);}
fn ea(file:&File,put:bool)->Vec<u8>{
    #[link(name="ntdll")] unsafe extern "system"{
        fn NtSetEaFile(file:Handle,status:*mut Status,data:*const u8,size:u32)->i32;
        fn NtQueryEaFile(file:Handle,status:*mut Status,data:*mut u8,size:u32,single:u8,list:*const u8,list_size:u32,index:*const u32,restart:u8)->i32;
    }
    let mut status=Status{status:0,information:0};
    if put{let name=b"QING.LINK";let value=b"EA retained";let mut data=vec![0u8;8];data[5]=name.len() as u8;data[6..8].copy_from_slice(&(value.len() as u16).to_le_bytes());data.extend(name);data.push(0);data.extend(value);assert_eq!(unsafe{NtSetEaFile(file.as_raw_handle(),&mut status,data.as_ptr(),data.len() as u32)},0);}
    let mut data=vec![0u8;4096];assert_eq!(unsafe{NtQueryEaFile(file.as_raw_handle(),&mut status,data.as_mut_ptr(),data.len() as u32,0,std::ptr::null(),0,std::ptr::null(),1)},0);data.truncate(status.information);data
}
struct Fixture{root:tempfile::TempDir,path:PathBuf,kind:LinkKind,old:Vec<u8>,ea:Vec<u8>,attributes:u32}
impl Fixture{
 fn new(kind:LinkKind)->Self{
    let root=tempfile::tempdir().unwrap();let path=root.path().join("link");if kind==LinkKind::Junction{fs::create_dir(&path).unwrap();}else{fs::write(&path,b"hidden main bytes").unwrap();}
    fs::write(format!("{}:keep",path.display()),b"ADS retained").unwrap();let file=raw(&path,0x10019b);let ea=ea(&file,true);
    let old=if kind==LinkKind::Junction{junction_data(&root.path().join("missing-old")).unwrap()}else{wsl_data("../old/用户").unwrap()};ioctl(&file,0x900a4,&old,&mut []).unwrap();attrs(&file,if kind==LinkKind::Junction{0x12}else{0x23});let attributes=file.metadata().unwrap().file_attributes();drop(file);Self{root,path,kind,old,ea,attributes}
 }
 fn check(&self,reparse:bool){
    let mut file=raw(&self.path,0x80000000);assert_eq!(file.metadata().unwrap().file_attributes(),if reparse{self.attributes}else{self.attributes&!0x400});assert_eq!(ea(&file,false),self.ea);
    if self.kind==LinkKind::Wsl{let mut bytes=Vec::new();file.read_to_end(&mut bytes).unwrap();assert_eq!(bytes,b"hidden main bytes");}drop(file);
    let stream=PathBuf::from(format!("{}:keep",self.path.display()));let mut file=raw(&stream,0x80000000);let mut bytes=Vec::new();file.read_to_end(&mut bytes).unwrap();assert_eq!(bytes,b"ADS retained");
    if !reparse&&self.kind==LinkKind::Junction{assert_eq!(fs::read_dir(&self.path).unwrap().count(),0);}
 }
}
impl Drop for Fixture{fn drop(&mut self){let file=raw(&self.path,0x100100);attrs(&file,if self.kind==LinkKind::Junction{0x10}else{0x20});}}
#[test] fn existing_edit_clear_preserve_entry_payload_ads_ea_attributes(){
 for kind in [LinkKind::Junction,LinkKind::Wsl]{let f=Fixture::new(kind);let opts=EngineOptions::default();let before=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();f.check(true);assert_eq!(before.kind,kind);assert_eq!(before.receipt.file_id.len(),16);assert_eq!(before.receipt.reparse_sha256.len(),64);
    let target=if kind==LinkKind::Junction{f.root.path().join("new").join("目标").display().to_string()}else{"/new/用户/target".into()};let changed=edit_link(f.path.to_str().unwrap(),&before.receipt,Some(&target),&opts).unwrap();assert!(!changed.removed);f.check(true);
    let after=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();assert_eq!(after.target,target);assert_eq!(after.receipt.file_id,before.receipt.file_id);assert_eq!(Some(&after.receipt),changed.receipt.as_ref());
    assert!(edit_link(f.path.to_str().unwrap(),&before.receipt,None,&opts).unwrap_err().contains("查看后已改变"));let cleared=edit_link(f.path.to_str().unwrap(),&after.receipt,None,&opts).unwrap();assert!(cleared.removed);assert!(cleared.receipt.is_none());f.check(false);assert!(inspect_link(f.path.to_str().unwrap(),&opts).is_err());
 }
}
#[test] fn foreign_object_unknown_tag_and_hardlink_objects_fail_closed(){
 let f=Fixture::new(LinkKind::Wsl);let opts=EngineOptions::default();let before=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();let saved=f.path.with_extension("saved");fs::rename(&f.path,&saved).unwrap();fs::write(&f.path,b"foreign").unwrap();assert!(edit_link(f.path.to_str().unwrap(),&before.receipt,None,&opts).is_err());assert_eq!(fs::read(&f.path).unwrap(),b"foreign");fs::remove_file(&f.path).unwrap();fs::rename(&saved,&f.path).unwrap();
 let ordinary=f.root.path().join("ordinary");fs::write(&ordinary,b"ordinary bytes").unwrap();assert!(inspect_link(ordinary.to_str().unwrap(),&opts).is_err());let alias=f.root.path().join("alias");fs::hard_link(&ordinary,&alias).unwrap();let file=raw(&ordinary,0x100100);ioctl(&file,0x900a4,&wsl_data("/multi").unwrap(),&mut []).unwrap();drop(file);
 let readonly=inspect_link(ordinary.to_str().unwrap(),&opts).unwrap();assert!(!readonly.can_edit);assert!(readonly.edit_reason.unwrap().contains("多硬链接"));assert!(edit_link(ordinary.to_str().unwrap(),&before.receipt,None,&opts).is_err());let file=raw(&ordinary,0x100100);let mut header=[0;8];header[..4].copy_from_slice(&0xa000001du32.to_le_bytes());ioctl(&file,0x900ac,&header,&mut []).unwrap();drop(file);assert_eq!(fs::read(&ordinary).unwrap(),b"ordinary bytes");
 let mut unknown=f.old.clone();unknown[..4].copy_from_slice(&0xa0000042u32.to_le_bytes());assert!(parse(&unknown,false).is_err());f.check(true);
}
#[test] fn postwrite_failure_restores_old_bytes_and_confirms_rollback(){
 for remove in [false,true]{for fail_restore in [false,true]{let f=Fixture::new(LinkKind::Wsl);let opts=EngineOptions::default();let before=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();FAIL_VERIFY.with(|v|v.set(true));if fail_restore{FAIL_RESTORE.with(|v|v.set(true));}
    let error=edit_link(f.path.to_str().unwrap(),&before.receipt,if remove{None}else{Some("/new")},&opts).unwrap_err();assert!(error.contains("已确认事务回滚"),"{error}");assert_eq!(inspect_link(f.path.to_str().unwrap(),&opts).unwrap().receipt,before.receipt);f.check(true);
 }}
}
#[test] fn edit_excludes_metadata_writers_rename_and_cancel(){
 let f=Fixture::new(LinkKind::Junction);let opts=EngineOptions::default();let before=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();let writer=raw(&f.path,0x100100);assert!(edit_link(f.path.to_str().unwrap(),&before.receipt,None,&opts).is_err());drop(writer);
 let attempt=f.path.clone();let parent=f.root.path().to_path_buf();BEFORE_EDIT.with(|hook|*hook.borrow_mut()=Some(Box::new(move||{assert!(fs::rename(&attempt,attempt.with_extension("moved")).is_err());assert!(fs::remove_dir(&attempt).is_err());assert!(fs::rename(&parent,parent.with_extension("moved")).is_err());for path in [&attempt,&parent]{for access in [0x100,0x10]{assert!(File::options().access_mode(access).share_mode(7).custom_flags(0x02200000).open(path).is_err(),"metadata {path:?}/{access:x}");}}})));
 edit_link(f.path.to_str().unwrap(),&before.receipt,Some(f.root.path().join("new").to_str().unwrap()),&opts).unwrap();let before=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();let ctl=JobControl::new();let cancel=ctl.clone();let mut cancelled=EngineOptions::default();cancelled.control=Some(ctl);BEFORE_EDIT.with(|hook|*hook.borrow_mut()=Some(Box::new(move||cancel.cancel())));assert!(edit_link(f.path.to_str().unwrap(),&before.receipt,None,&cancelled).is_err());assert_eq!(inspect_link(f.path.to_str().unwrap(),&opts).unwrap().receipt,before.receipt);f.check(true);
}

#[test] fn commit_failure_confirms_rollback_and_keeps_original_data(){
 let f=Fixture::new(LinkKind::Wsl);let opts=EngineOptions::default();let before=inspect_link(f.path.to_str().unwrap(),&opts).unwrap();
 super::super::super::super::file_workspace_transaction::FAIL_COMMIT.with(|v|v.set(true));
 let error=edit_link(f.path.to_str().unwrap(),&before.receipt,Some("/commit-failure"),&opts).unwrap_err();assert!(error.contains("已确认回滚"),"{error}");assert_eq!(inspect_link(f.path.to_str().unwrap(),&opts).unwrap().receipt,before.receipt);f.check(true);
}
#[test]
#[ignore="Requires an already-held symbolic-link privilege; native CI must run explicitly"]
fn require_symbolic_inspect_edit_clear_success(){
 for directory in [false,true]{
    let root=tempfile::tempdir().unwrap();let path=root.path().join("relative-link");let opts=EngineOptions::default();let caller=privilege::snapshot();
    let kind=if directory{LinkKind::SymbolicDirectory}else{LinkKind::SymbolicFile};if directory{fs::create_dir(&path).unwrap();}else{fs::write(&path,b"hidden original payload").unwrap();}
    let original=Parsed{kind,target:"..\\missing-old".into(),print:String::new(),relative:true};let old=replacement(&original,"..\\missing-old").unwrap();
    let file=raw(&path,0x100100);isolated(kind,||{ioctl(&file,0x900a4,&old,&mut [])?;Ok(())}).expect("native CI must set symbolic metadata with existing held privilege");drop(file);
    let before=inspect_link(path.to_str().unwrap(),&opts).unwrap();assert!(before.relative);assert_eq!(before.target,"..\\missing-old");
    let edited=edit_link(path.to_str().unwrap(),&before.receipt,Some("..\\new-相对目标"),&opts).expect("native CI must successfully edit relative/dangling symbolic target");assert!(!edited.removed);
    let after=inspect_link(path.to_str().unwrap(),&opts).unwrap();assert!(after.relative);assert_eq!(after.target,"..\\new-相对目标");
    for failure in [0,1,2]{
        if failure==0{FAIL_VERIFY.with(|v|v.set(true));}else if failure==1{privilege::FAIL_RESTORE.with(|v|v.set(true));}else{PANIC_VERIFY.with(|v|v.set(true));}
        let error=edit_link(path.to_str().unwrap(),&after.receipt,Some("..\\must-rollback"),&opts).unwrap_err();assert!(error.contains("已确认事务回滚"),"{error}");assert_eq!(inspect_link(path.to_str().unwrap(),&opts).unwrap().receipt,after.receipt);assert_eq!(privilege::snapshot(),caller);
    }
    assert!(edit_link(path.to_str().unwrap(),&after.receipt,None,&opts).expect("native CI must remove symbolic reparse metadata without deleting entry").removed);
    assert_eq!(fs::symlink_metadata(&path).unwrap().file_attributes()&0x400,0);if directory{fs::remove_dir(&path).unwrap();}else{assert_eq!(fs::read(&path).unwrap(),b"hidden original payload");}
    assert_eq!(privilege::snapshot(),caller);println!("required symbolic existing-link acceptance passed: directory={directory}; relative/dangling targets, edit/clear, failure/helper restore/panic rollback and unchanged caller token");
 }
}

#[test] fn shared_name_slices_parse_safely_and_record_native_validation(){
 let root=tempfile::tempdir().unwrap();let path=root.path().join("shared-name-junction");fs::create_dir(&path).unwrap();let target=root.path().join("dangling");
 let sub=format!("\\??\\{}",target.display()).encode_utf16().collect::<Vec<_>>();let len=(sub.len()*2) as u16;let mut data=Vec::new();
 data.extend(0xa0000003u32.to_le_bytes());data.extend((8+len+2).to_le_bytes());data.extend(0u16.to_le_bytes());for value in [0,len,8,len-8]{data.extend(value.to_le_bytes());}for word in sub{data.extend(word.to_le_bytes());}data.extend(0u16.to_le_bytes());
 let parsed=parse(&data,true).unwrap();assert_eq!(parsed.target,target.display().to_string());assert_eq!(parsed.print,target.display().to_string());
 let file=raw(&path,0x100100);let result=ioctl(&file,0x900a4,&data,&mut []);println!("native junction overlapping name slices: {result:?}; parser permits bounded aliases");
 match result{Ok(_)=>{assert_eq!(get(&file).unwrap(),data);drop(file);let inspected=inspect_link(path.to_str().unwrap(),&EngineOptions::default()).unwrap();assert_eq!(inspected.target,target.display().to_string());edit_link(path.to_str().unwrap(),&inspected.receipt,None,&EngineOptions::default()).unwrap();},Err(error)=>{assert!(error.contains("os error 4392"),"{error}");assert_eq!(file.metadata().unwrap().file_attributes()&0x400,0);}}
}
