//! A bounded metadata-only invalidation probe. All pins are scoped to this call;
//! nothing keeps a directory open between probes or reads file contents.
use super::*;
#[derive(Debug,PartialEq,Eq,Serialize)]
pub struct DirectoryStamp {pub revision:String,pub entries:Vec<EntryStamp>}
#[derive(Debug,PartialEq,Eq,Serialize)]
pub struct EntryStamp {pub name:String,pub revision:Option<String>}
pub(super) fn entry_revision(meta:&fs::Metadata)->String {format!("{}:{}:{}",meta.file_attributes(),meta.len(),meta.last_write_time())}
pub(super) fn directory_revision(file:&File)->Result<String,String>{let (volume,id)=directory_identity(file)?;let meta=file.metadata().map_err(|e|e.to_string())?;Ok(format!("{volume}:{id}:{}",meta.last_write_time()))}
pub fn directory_stamp(path:&str,names:&[String])->Result<DirectoryStamp,String>{
    use std::path::{Component,Prefix};
    if path.is_empty(){return Err("请选择一个文件夹".into());}
    if names.len()>200{return Err("刷新项目过多".into());}
    let root=local_path(path)?;
    let letter=match root.components().next(){Some(Component::Prefix(p))=>match p.kind(){Prefix::Disk(letter)|Prefix::VerbatimDisk(letter)=>letter,_=>return Err("此位置请使用手动刷新".into())},_=>return Err("此位置请使用手动刷新".into())};
    #[link(name="kernel32")]unsafe extern "system"{fn GetDriveTypeW(root:*const u16)->u32;}
    // A mapped network share has a Disk prefix too. Do not poll remote metadata.
    if unsafe{GetDriveTypeW([letter as u16,58,92,0].as_ptr())}==4{return Err("此位置请使用手动刷新".into());}
    let mut leaves=Vec::with_capacity(names.len());
    for name in names{
        if name.is_empty()||name.len()>1024||name.contains(['/', '\\']){return Err("无效的刷新项目名称".into());}
        let leaf=crate::extract::relative_path(name)?;
        if leaf.components().count()!=1{return Err("刷新名称必须是单个路径组件".into());}leaves.push(leaf);
    }
    let mut guards=CheckedDirectories::default();guards.check(&root)?;
    let handle=&guards.held.get(&root).ok_or("无法固定目录")?.0;
    let metadata=handle.metadata().map_err(|e|e.to_string())?;
    if !metadata.is_dir()||metadata.file_attributes()&0x400!=0{return Err("目录位置已改变".into());}
    let revision=directory_revision(handle)?;let mut entries=Vec::with_capacity(names.len());
    for (name,leaf) in names.iter().zip(leaves){
        // The parent and all ancestors remain pinned during this metadata read.
        // symlink_metadata does not follow a leaf reparse point.
        let revision=match fs::symlink_metadata(root.join(leaf)){
            Ok(meta)=>Some(entry_revision(&meta)),
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>None,
            Err(e)=>return Err(e.to_string()),
        };entries.push(EntryStamp{name:name.clone(),revision});
    }
    Ok(DirectoryStamp{revision,entries})
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn stamp_detects_growth_membership_and_releases_all_parent_pins(){
        let temp=tempfile::tempdir().unwrap();let parent=temp.path().join("parent");let root=parent.join("folder");fs::create_dir_all(&root).unwrap();fs::write(root.join("file"),b"a").unwrap();let names=vec!["file".into()];
        let stamp=directory_stamp(root.to_str().unwrap(),&names).unwrap();assert_eq!(directory_stamp(root.to_str().unwrap(),&names).unwrap(),stamp);
        fs::write(root.join("file"),b"longer").unwrap();assert_ne!(directory_stamp(root.to_str().unwrap(),&names).unwrap(),stamp);
        fs::remove_file(root.join("file")).unwrap();let missing=directory_stamp(root.to_str().unwrap(),&names).unwrap();assert_ne!(missing,stamp);
        fs::rename(&root,parent.join("renamed")).unwrap();fs::rename(&parent,temp.path().join("parent-renamed")).unwrap();
        assert!(directory_stamp(root.to_str().unwrap(),&names).is_err());
    }
    #[test]fn stamp_bounds_names_and_rejects_path_injection(){
        let temp=tempfile::tempdir().unwrap();let path=temp.path().to_str().unwrap();
        assert!(directory_stamp(path,&vec!["x".into();201]).is_err());
        for name in ["", ".", "..", "../outside", "a/b", "a\\b", "x:stream", "CON", "a.", "a "]{assert!(directory_stamp(path,&[name.into()]).is_err(),"{name}");}
        assert!(directory_stamp(path,&["不存在.txt".into()]).is_ok());
    }
}
