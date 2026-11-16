//! Private input snapshot used only for explicitly stored access timestamps.
//! Traversal may change source directory atime; snapshot it before child lookup,
//! copy with the existing complete local-stream proofs, then set private times.
use super::*;
use std::{ffi::OsString,os::windows::{fs::{MetadataExt,OpenOptionsExt},io::AsRawHandle}};

pub(super) struct Snapshot {
    path: PathBuf,
    relative: PathBuf,
    source: File,
    metadata: fs::Metadata,
    identity: (u32,u64),
    digest: [u8;32],
    pub(super) children: Vec<OsString>,
}
fn digest(file:&File,ctl:&JobControl)->Result<[u8;32],String>{
    if file.metadata().map_err(|e|e.to_string())?.file_attributes()&0x4000!=0 {
        file_workspace_efs::archive_input_digest(file,ctl)
    }else{file_workspace::streams(file,None,ctl)}
}
impl Snapshot {
    pub(super) fn capture(path:&Path,relative:&Path,source:&File,metadata:&fs::Metadata,ctl:&JobControl)->Result<Self,String>{
        // Metadata was obtained before streams or any child-name resolution.
        Ok(Self{path:path.to_owned(),relative:relative.to_owned(),source:source.try_clone().map_err(|e|e.to_string())?,
            metadata:metadata.clone(),identity:directory_identity(source)?,digest:digest(source,ctl)?,children:Vec::new()})
    }
    fn verify(&self,ctl:&JobControl)->Result<(),String>{
        ctl.check()?;let now=self.source.metadata().map_err(|e|e.to_string())?;
        if directory_identity(&self.source)?!=self.identity || now.file_attributes()!=self.metadata.file_attributes()
            || now.creation_time()!=self.metadata.creation_time() || now.last_write_time()!=self.metadata.last_write_time()
            || now.file_size()!=self.metadata.file_size() || digest(&self.source,ctl)?!=self.digest {
            return Err("creation source changed while preparing timestamp snapshot".into());
        }
        if now.is_dir(){
            let mut actual=Vec::new();for entry in fs::read_dir(&self.path).map_err(|e|e.to_string())? {ctl.check()?;actual.push(entry.map_err(|e|e.to_string())?.file_name());}
            let mut expected=self.children.clone();actual.sort();expected.sort();
            if actual!=expected{return Err("creation source directory entries changed".into());}
        }
        Ok(())
    }
}
// Source and copied-file pins are released before private-tree cleanup.
pub(super) struct Prepared {
    snapshots:Vec<Snapshot>,
    targets:Vec<File>,
    target_digests:Vec<[u8;32]>,
    pub(super) roots:Vec<PathBuf>,
    _stage:ShellDragDirectory,
}
impl Prepared {
    pub(super) fn verify(&self,ctl:&JobControl)->Result<(),String>{
        for ((snapshot,target),expected) in self.snapshots.iter().zip(&self.targets).zip(&self.target_digests) {
            snapshot.verify(ctl)?;
            if digest(target,ctl)?!=*expected{return Err("private creation input changed".into());}
        }
        Ok(())
    }
}
pub(super) fn prepare(paths:&[PathBuf],parent:&Path,opts:&EngineOptions,ctl:&JobControl)->Result<Prepared,String>{
    ctl.check()?;reject_output_in_inputs(parent,paths)?;
    let stage=ShellDragDirectory::create_named_in(parent,"qing-zip-create-times-")?;
    let mut snapshots=Vec::new();let mut copy_options=EngineOptions::default();copy_options.control=Some(ctl.clone());
    let inputs=paths.iter().map(|p|p.display().to_string()).collect::<Vec<_>>();
    let copied=file_workspace::copy_for_archive(&inputs,stage.path().to_str().ok_or("invalid private input path")?,&copy_options,&mut snapshots)?;
    if copied.skipped!=0 || !copied.metadata_not_merged.is_empty() || copied.copied+copied.copied_directories!=snapshots.len(){return Err("incomplete private creation input".into());}
    let mut targets=Vec::new();
    for snapshot in &snapshots {
        ctl.check()?;let path=stage.path().join(&snapshot.relative);no_links(&path)?;
        let target=File::options().access_mode(0x80000100).share_mode(1).custom_flags(0x02200000).open(path).map_err(|e|e.to_string())?;
        if target.metadata().map_err(|e|e.to_string())?.file_attributes()&0x400!=0{return Err("private input link rejected".into());}
        targets.push(target);
    }
    let roots=paths.iter().map(|p|stage.path().join(p.file_name().unwrap())).collect();
    // The existing publisher proved every source/target stream mapping. EFS
    // may legitimately derive different target certificate policy; pin its
    // complete published digest instead of requiring identical certificate bytes.
    let target_digests=targets.iter().map(|file|digest(file,ctl)).collect::<Result<Vec<_>,_>>()?;
    let ready=Prepared{snapshots,targets,target_digests,roots,_stage:stage};ready.verify(ctl)?;
    // No subsequent path traversal of the private tree before the encoder: even
    // metadata lookups of child names can update parent directory access time.
    let mut order:Vec<usize>=(0..ready.snapshots.len()).collect();
    order.sort_by_key(|&i|std::cmp::Reverse(ready.snapshots[i].relative.components().count()));
    #[link(name="kernel32")] extern "system" {fn SetFileTime(h:*mut std::ffi::c_void,c:*const u64,a:*const u64,m:*const u64)->i32;}
    for i in order {
        ctl.check()?;let m=&ready.snapshots[i].metadata;
        if unsafe{SetFileTime(ready.targets[i].as_raw_handle(),&m.creation_time(),&m.last_access_time(),&m.last_write_time())}==0{return Err(std::io::Error::last_os_error().to_string());}
    }
    let _=opts;Ok(ready)
}
