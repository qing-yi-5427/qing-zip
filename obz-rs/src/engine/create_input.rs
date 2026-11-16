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
        if PhaseTimer::enabled() && relative.components().count()==1 {eprintln!("OA_TIME_CAPTURE atime={}",metadata.last_access_time());}
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
    pub(super) fn snapshot_manifest(&self)->Result<Vec<u8>,String>{
        use std::os::windows::ffi::OsStrExt;
        let mut out=Vec::new();
        out.extend_from_slice(&u32::try_from(self.snapshots.len()).map_err(|_|"too many snapshot entries")?.to_le_bytes());
        let mut seen=HashSet::new();
        for s in &self.snapshots {
            if !seen.insert(s.relative.clone()) {return Err("duplicate input projection".into());}
            let units=s.relative.as_os_str().encode_wide().collect::<Vec<_>>();
            if units.is_empty() || units.len()>32767 {return Err("snapshot path too long".into());}
            out.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in units {out.extend_from_slice(&u.to_le_bytes());}
            out.extend_from_slice(&u32::from(s.metadata.is_dir()).to_le_bytes());
            for v in [s.metadata.creation_time(),s.metadata.last_access_time(),s.metadata.last_write_time()] {out.extend_from_slice(&v.to_le_bytes());}
            if out.len()>64*1024*1024 {return Err("timestamp manifest exceeds bounded worker message budget".into());}
        }
        Ok(out)
    }
    pub(super) fn container(&self)->&Path {self._stage.path()}
    pub(super) fn trace_root(&self,label:&str)->Result<(),String>{
        if PhaseTimer::enabled(){for (s,t) in self.snapshots.iter().zip(&self.targets).filter(|(s,_)|s.relative.components().count()==1){eprintln!("OA_TIME_POINT stage={} expected={} actual={}",label,s.metadata.last_access_time(),t.metadata().map_err(|e|e.to_string())?.last_access_time());}}
        Ok(())
    }
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
    // Archive times are immutable protocol records. The private tree retains
    // whatever times its safe publisher supplied; no filesystem timestamp is
    // used to transport the snapshot to the encoder.
    let _=opts;Ok(ready)
}
