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

