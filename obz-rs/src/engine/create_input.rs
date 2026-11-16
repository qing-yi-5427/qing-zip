//! Private input snapshot used only for explicitly stored access timestamps.
//! Traversal may change source directory atime; snapshot it before child lookup,
//! copy with the existing complete local-stream proofs, then send immutable times.
use super::*;
use sha2::{Digest,Sha256};
use super::file_parts::copy_hash;
use std::{ffi::OsString,os::windows::{fs::{MetadataExt,OpenOptionsExt}}};

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
        let target=File::options().access_mode(0x80000000).share_mode(1).custom_flags(0x02200000).open(path).map_err(|e|e.to_string())?;
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

#[cfg(test)]
thread_local! {static BEFORE_TIME_VOLUME_PUBLISH:std::cell::RefCell<Option<Box<dyn FnMut(usize,&Path)>>> = std::cell::RefCell::new(None);}

// Only the explicit-time route uses this journaled multi-name publication.
// All parts exist and are verified before any destination name becomes visible.
// NTFS cannot atomically expose multiple unrelated filenames; rollback receipts
// protect competitors and report any owned output that could not be removed.
pub(super) fn split_and_publish(archive:&Path,output:&Path,bytes:u64,opts:&EngineOptions,ctl:&JobControl)->Result<Vec<String>,String>{
    if bytes==0 {return Err("volume size must be positive".into());}
    let parent=output.parent().ok_or("output has no parent")?;
    let parts=ShellDragDirectory::create_named_in(parent,"qing-zip-time-volumes-")?;
    let mut input=crate::engine_stream::pin(archive)?;
    let length=input.metadata().map_err(|e|e.to_string())?.len();
    let modified=input.metadata().map_err(|e|e.to_string())?.modified().map_err(|e|e.to_string())?;
    let mut stage_guards=CheckedDirectories::default();stage_guards.check(parts.path())?;
    let mut produced=Vec::new();let mut remaining=length;let mut index=1u64;
    let mut whole=Sha256::new();let mut copy_done=0;
    while remaining!=0 || produced.is_empty() {
        ctl.check()?;let mut name=output.file_name().ok_or("missing output name")?.to_os_string();name.push(format!(".{index:03}"));
        let part=parts.path().join(&name);let n=remaining.min(bytes);
        let mut file=create_staged_output(&part,&mut stage_guards)?;
        copy_hash(&mut *input,&mut file,n,length,&mut copy_done,&mut whole,ctl)?;
        if opts.archive_timestamp==ArchiveTimestamp::Latest {file.set_times(std::fs::FileTimes::new().set_modified(modified)).map_err(|e|e.to_string())?;}
        file.sync_all().map_err(|e|e.to_string())?;produced.push((name,part,n));remaining-=n;
        index=index.checked_add(1).ok_or("too many volumes")?;
    }
    let mut verified=Sha256::new();let mut buffer=[0u8;64*1024];let mut pins=Vec::new();
    for (_,path,size) in &produced {
        let mut file=crate::engine_stream::pin(path)?;let mut total=0u64;
        loop {ctl.check()?;let n=file.read(&mut buffer).map_err(|e|e.to_string())?;if n==0 {break;}verified.update(&buffer[..n]);total+=n as u64;}
        if total!=*size {return Err("private volume size changed".into());}
        pins.push(file);
    }
    if whole.finalize()!=verified.finalize(){return Err("private volume content verification failed".into());}
    let mut guards=CheckedDirectories::default();guards.check(parent)?;guards.allow_publication(parent)?;
    for (name,_,_) in &produced {let target=parent.join(name);no_links(&target)?;if target.try_exists().map_err(|e|e.to_string())? {return Err(format!("output exists: {}",target.display()));}}
    let journal=PublicationJournal::default();let mut results=Vec::new();
    ctl.set("publishing",Some(95),"正在保存分卷压缩包");
    let result=(||{
        for (index,((name,_path,n),file)) in produced.iter().zip(pins.iter_mut()).enumerate() {
            ctl.check()?;let target=parent.join(name);guards.check(&target)?;
            #[cfg(test)] BEFORE_TIME_VOLUME_PUBLISH.with(|hook|{if let Some(callback)=hook.borrow_mut().as_mut(){callback(index,&target);}});
            #[cfg(not(test))] let _=index;
            with_output_temp(parent,&guards,|out|{
                file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;let mut copied=0;
                copy_hash(&mut **file,out.as_file_mut(),*n,*n,&mut copied,&mut Sha256::new(),ctl)?;
                if opts.archive_timestamp==ArchiveTimestamp::Latest {out.as_file().set_times(std::fs::FileTimes::new().set_modified(modified)).map_err(|e|e.to_string())?;}
                out.as_file().sync_all().map_err(|e|e.to_string())?;ctl.check()?;
                publish_output_recorded(out,&target,false,Some(&journal)).map(drop)
            })?;
            results.push(target.display().to_string());
        }
        ctl.check()?;Ok(())
    })();
    if let Err(reason)=result {let errors=journal.rollback();return if errors.is_empty(){Err(reason)}else{Err(format!("{reason}; volume rollback incomplete: {}",errors.join("; ")))};}
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn internal_snapshot_pin_budget_fails_closed_without_changing_normal_default() {
        let directory=tempfile::tempdir().unwrap();
        let mut normal=CheckedDirectories::default();assert!(normal.archive_snapshot_pin_limit.is_none());
        normal.check(directory.path()).unwrap();
        let mut limited=CheckedDirectories::default();limited.archive_snapshot_pin_limit=Some(0);
        assert!(limited.check(directory.path()).unwrap_err().contains("pin budget exhausted"));
        assert!(limited.held.is_empty());
    }
    #[test]
    fn snapshot_volumes_roundtrip_and_latest_timestamp() {
        let dir=tempfile::tempdir().unwrap();let archive=dir.path().join("private.bin");
        let bytes=(0..513).map(|i|(i%251) as u8).collect::<Vec<_>>();fs::write(&archive,&bytes).unwrap();
        let file=File::options().write(true).open(&archive).unwrap();
        let when=std::time::UNIX_EPOCH+Duration::from_secs(1641081600);
        file.set_times(std::fs::FileTimes::new().set_modified(when)).unwrap();drop(file);
        let opts=EngineOptions{archive_timestamp:ArchiveTimestamp::Latest,..Default::default()};
        let outputs=split_and_publish(&archive,&dir.path().join("输出.7z"),128,&opts,&JobControl::new()).unwrap();
        assert_eq!(outputs.len(),5);let mut actual=Vec::new();
        for path in outputs {actual.extend(fs::read(&path).unwrap());assert_eq!(fs::metadata(path).unwrap().modified().unwrap(),when);}
        assert_eq!(actual,bytes);assert_eq!(fs::read(archive).unwrap(),bytes);
    }
    #[test]
    fn snapshot_volume_cancel_and_late_conflict_roll_back_owned_names() {
        for conflict in [false,true] {
            let dir=tempfile::tempdir().unwrap();let archive=dir.path().join("private.bin");
            let bytes=vec![42u8;513];fs::write(&archive,&bytes).unwrap();
            let ctl=JobControl::new();let cancel=ctl.clone();
            BEFORE_TIME_VOLUME_PUBLISH.with(|hook|*hook.borrow_mut()=Some(Box::new(move |index,path|{
                if index==1 {if conflict {fs::write(path,b"competitor").unwrap();} else {cancel.cancel();}}
            })));
            let result=split_and_publish(&archive,&dir.path().join("output.zip"),128,&EngineOptions::default(),&ctl);
            BEFORE_TIME_VOLUME_PUBLISH.with(|hook|hook.borrow_mut().take());
            assert!(result.is_err());assert!(!dir.path().join("output.zip.001").exists());
            if conflict {assert_eq!(fs::read(dir.path().join("output.zip.002")).unwrap(),b"competitor");}
            else {assert!(!dir.path().join("output.zip.002").exists());}
            assert_eq!(fs::read(archive).unwrap(),bytes);
        }
    }
}
