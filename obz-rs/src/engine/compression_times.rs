use super::EngineOptions;
use serde::{Serialize, Deserialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all="snake_case")]
pub enum ArchiveTimestamp { #[default] Default, Latest }
pub(super) fn requested(o:&EngineOptions)->bool {
    o.store_mtime.is_some() || o.store_ctime.is_some() || o.store_atime.is_some() || o.archive_timestamp!=ArchiveTimestamp::Default
}
pub(crate) fn validate(format:&str,o:&EngineOptions)->Result<(),String>{
    if requested(o) && !matches!(format,"zip"|"7z"|"sfx") {return Err("时间保存选项仅支持 ZIP、7z 和 SFX".into());} Ok(())
}
pub(super) fn switches(format:&str,o:&EngineOptions)->Result<Vec<String>,String>{
    validate(format,o)?;let mut args=Vec::new();
    for (key,value) in [("tm",o.store_mtime),("tc",o.store_ctime),("ta",o.store_atime)] {
        if let Some(value)=value {args.push(format!("-m{key}={}",if value {"on"}else{"off"}));}
    }
    if o.archive_timestamp==ArchiveTimestamp::Latest {args.push("-stl".into());} Ok(args)
}
pub(super) fn native_flags(o:&EngineOptions)->u32 {
    let pair=|v:Option<bool>|v.map(|v|if v{2}else{1}).unwrap_or(0);
    pair(o.store_mtime)|(pair(o.store_ctime)<<2)|(pair(o.store_atime)<<4)|if o.archive_timestamp==ArchiveTimestamp::Latest {64}else{0}
}
pub(super) fn valid_native(flags:u32)->bool {
    flags&!127==0 && [0,2,4].iter().all(|shift|((flags>>shift)&3)!=3)
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn defaults_have_no_switches_and_explicit_choices_are_typed(){
  let mut o=EngineOptions::default();assert!(switches("zip",&o).unwrap().is_empty());assert_eq!(native_flags(&o),0);
  o.store_mtime=Some(false);o.store_ctime=Some(true);o.store_atime=Some(true);o.archive_timestamp=ArchiveTimestamp::Latest;
  assert_eq!(switches("zip",&o).unwrap(),["-mtm=off","-mtc=on","-mta=on","-stl"]);assert!(valid_native(native_flags(&o)));
  assert!(switches("tar",&o).is_err());assert!(!valid_native(3));assert!(!valid_native(128));
 }
}

// Explicit atime capture uses a dedicated enumeration handle. The -1 sentinel
// prevents this handle's reads from updating access time; it does not restore or
// rewrite a saved timestamp. Ordinary access checks remain authoritative.
#[cfg(windows)]
pub(super) fn directory_entries(path:&std::path::Path,pinned:&std::fs::File,mut visit:impl FnMut(std::path::PathBuf,bool)->Result<(),String>)->Result<(),String>{
 use std::os::windows::{fs::OpenOptionsExt,io::AsRawHandle,ffi::OsStringExt};
 use std::{fs::OpenOptions,ffi::{c_void,OsString}};
 #[link(name="kernel32")] extern "system" {
  fn SetFileTime(h:*mut c_void,c:*const u64,a:*const u64,m:*const u64)->i32;
  fn GetFileInformationByHandleEx(h:*mut c_void,class:i32,data:*mut u32,size:u32)->i32;
 }
 let file=OpenOptions::new().access_mode(0x181).share_mode(1).custom_flags(0x02200000).open(path)
  .map_err(|e|format!("cannot preserve directory access time: {e}"))?;
 if super::directory_identity(&file)?!=super::directory_identity(pinned)? || super::directory_attributes(&file)?&0x410!=0x10 {
  return Err("input directory changed before timestamp enumeration".into());
 }
 let never=u64::MAX;
 if unsafe{SetFileTime(file.as_raw_handle(),std::ptr::null(),&never,std::ptr::null())}==0 {return Err(std::io::Error::last_os_error().to_string());}
 let mut data=vec![0u8;65536];let mut first=true;
 loop {
  let ok=unsafe{GetFileInformationByHandleEx(file.as_raw_handle(),if first{11}else{10},data.as_mut_ptr().cast(),data.len() as u32)};first=false;
  if ok==0 {let e=std::io::Error::last_os_error();if e.raw_os_error()==Some(18){break;}return Err(e.to_string());}
  let mut offset=0usize;
  loop {
   if offset+104>data.len(){return Err("invalid directory timestamp enumeration".into());}
   let u32at=|n|u32::from_le_bytes(data[offset+n..offset+n+4].try_into().unwrap());
   let next=u32at(0) as usize;let attrs=u32at(56);let bytes=u32at(60) as usize;
   let end=offset.checked_add(104).and_then(|n|n.checked_add(bytes)).ok_or("invalid directory name length")?;
   if bytes%2!=0||end>data.len()||next!=0&&(next<104+bytes||offset+next>=data.len()){return Err("invalid directory record length".into());}
   let wide:Vec<u16>=data[offset+104..end].chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect();
   if wide!=[46]&&wide!=[46,46] {
    if wide.is_empty()||wide.iter().any(|c|[0,47,92].contains(c)){return Err("invalid directory entry name".into());}
    visit(path.join(OsString::from_wide(&wide)),attrs&0x10!=0)?;
   }
   if next==0{break;}offset+=next;
  }
 }
 Ok(())
}
