//! Immutable parent time metadata and complete original-source proofs.
//! Snapshot each directory before child lookup; read originals through retained
//! identity pins, never copy the input tree or write its timestamps.
use super::*;
use std::os::windows::io::AsRawHandle;
type Handle = *mut std::ffi::c_void;
use sha2::{Digest,Sha256};
use super::file_parts::copy_hash;
use std::{ffi::OsString,os::windows::{fs::{MetadataExt,OpenOptionsExt}}};

extern "C" {
    fn qz_create_input_open(parent:Handle,name:*const u16,access:u32,share:u32,result:*mut Handle)->i32;
    fn qz_create_input_names(directory:Handle,restart:u32,names:*mut u8,capacity:u32,used:*mut u32)->i32;
}
fn open_child_io(parent:&File,name:&std::ffi::OsStr,access:u32,share:u32)->std::io::Result<File>{
    use std::os::windows::{ffi::OsStrExt,io::FromRawHandle};
    let name=name.encode_wide().chain(Some(0)).collect::<Vec<_>>();let mut handle=std::ptr::null_mut();
    let hr=unsafe{qz_create_input_open(parent.as_raw_handle(),name.as_ptr(),access,share,&mut handle)};
    if hr<0 {return Err(if (hr as u32)&0xffff0000==0x80070000 {std::io::Error::from_raw_os_error(hr&0xffff)}else{std::io::Error::other(format!("relative source open failed: {hr:#x}"))});}
    if handle.is_null(){return Err(std::io::Error::other("relative source handle missing"));}
    Ok(unsafe{File::from_raw_handle(handle)})
}
fn open_child(parent:&File,name:&std::ffi::OsStr,access:u32,share:u32)->Result<File,String>{
    open_child_io(parent,name,access,share).map_err(|e|e.to_string())
}
fn each_name(directory:&File,ctl:&JobControl,mut visit:impl FnMut(OsString)->Result<(),String>)->Result<(),String>{
    use std::os::windows::ffi::OsStringExt;
    let mut page=[0u8;65536];let mut first=true;
    loop {
        ctl.check()?;let mut used=0;
        let hr=unsafe{qz_create_input_names(directory.as_raw_handle(),u32::from(first),page.as_mut_ptr(),page.len() as u32,&mut used)};
        if hr==1{return Ok(());}if hr<0{return Err(format!("source directory enumeration failed: {hr:#x}"));}
        first=false;let mut at=0;let used=used as usize;if used>page.len(){return Err("invalid source directory page".into());}
        while at<used {
            ctl.check()?;if used-at<4{return Err("truncated source name".into());}
            let units=u32::from_le_bytes(page[at..at+4].try_into().unwrap()) as usize;at+=4;
            if units==0||units>(used-at)/2{return Err("invalid source name length".into());}
            let name=page[at..at+units*2].chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).collect::<Vec<_>>();at+=units*2;
            visit(OsString::from_wide(&name))?;
        }
    }
}

pub(super) struct Snapshot {
    path: PathBuf,
    relative: PathBuf,
    source: File,
    metadata: fs::Metadata,
    identity: (u32,u64),
    logical_time:[u64;3],
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
        Self::capture_reported(path,relative,source,metadata,ctl,None)
    }
    fn capture_reported(path:&Path,relative:&Path,source:&File,metadata:&fs::Metadata,ctl:&JobControl,report:Option<&std::sync::Arc<std::sync::Mutex<crate::source_access::Report>>>)->Result<Self,String>{
        let mut uncertain=None;
        let proof=if metadata.file_attributes()&0x4000!=0 {
            uncertain=Some(crate::source_access::Phase::EncryptedSourceProof);file_workspace_efs::archive_input_digest(source,ctl)?
        }else{file_workspace::streams_observed(source,None,ctl,Some(&mut ||{uncertain=Some(crate::source_access::Phase::AlternateStreamProof);}))?};
        if let (Some(report),Some(phase))=(report,uncertain){report.lock().unwrap_or_else(|e|e.into_inner()).record_unconfirmed(path,phase);}
        if PhaseTimer::enabled() && relative.components().count()==1 {eprintln!("OA_TIME_CAPTURE atime={}",metadata.last_access_time());}
        // Metadata was obtained before streams or any child-name resolution.
        Ok(Self{path:path.to_owned(),relative:relative.to_owned(),source:source.try_clone().map_err(|e|e.to_string())?,
            metadata:metadata.clone(),identity:directory_identity(source)?,logical_time:[metadata.creation_time(),metadata.last_access_time(),metadata.last_write_time()],digest:proof,children:Vec::new()})
    }
    fn verify(&self,ctl:&JobControl)->Result<(),String>{
        ctl.check()?;let now=self.source.metadata().map_err(|e|e.to_string())?;
        if directory_identity(&self.source)?!=self.identity || now.file_attributes()!=self.metadata.file_attributes()
            || now.creation_time()!=self.metadata.creation_time() || now.last_write_time()!=self.metadata.last_write_time()
            || now.file_size()!=self.metadata.file_size() || digest(&self.source,ctl)?!=self.digest {
            return Err("creation source changed while preparing timestamp snapshot".into());
        }
        if now.is_dir(){
            let mut actual=Vec::new();each_name(&self.source,ctl,|name|{actual.push(name);Ok(())})?;
            let mut expected=self.children.clone();actual.sort();expected.sort();
            if actual!=expected{return Err("creation source directory entries changed".into());}
        }
        Ok(())
    }
}
// Original source and ancestor pins remain held through encoder exit and final
// verification. No source data is copied and no source timestamp is written.
pub(super) struct Prepared {
    snapshots: Vec<Snapshot>,
    ancestors: Vec<File>,
    root_indices: Vec<usize>,
    pub(super) roots: Vec<PathBuf>,
}
impl Prepared {
    pub(super) fn root_handles(&self)->impl Iterator<Item=(&Path,&File)> {
        self.root_indices.iter().map(|&i| (self.snapshots[i].path.as_path(),&self.snapshots[i].source))
    }
    pub(super) fn snapshot_manifest(&self,readers:Option<&[u64]>)->Result<Vec<u8>,String>{
        use std::os::windows::ffi::OsStrExt;
        let mut out=Vec::new();
        out.extend_from_slice(&u32::try_from(self.snapshots.len()).map_err(|_|"too many snapshot entries")?.to_le_bytes());
        let mut seen=HashSet::new();
        if readers.is_some_and(|v|v.len()!=self.snapshots.len()){return Err("reader manifest count mismatch".into());}
        for (index,s) in self.snapshots.iter().enumerate() {
            if !seen.insert(s.relative.clone()) {return Err("duplicate input projection".into());}
            let units=s.relative.as_os_str().encode_wide().collect::<Vec<_>>();
            if units.is_empty() || units.len()>32767 {return Err("snapshot path too long".into());}
            out.extend_from_slice(&(units.len() as u32).to_le_bytes());
            for u in units {out.extend_from_slice(&u.to_le_bytes());}
            out.extend_from_slice(&u32::from(s.metadata.is_dir()).to_le_bytes());
            for v in s.logical_time {out.extend_from_slice(&v.to_le_bytes());}
            // Physical proof is independent from logical archive time values.
            out.extend_from_slice(&s.identity.0.to_le_bytes());
            out.extend_from_slice(&s.identity.1.to_le_bytes());
            out.extend_from_slice(&s.metadata.file_size().to_le_bytes());
            out.extend_from_slice(&s.metadata.file_attributes().to_le_bytes());
            out.extend_from_slice(&s.metadata.creation_time().to_le_bytes());
            out.extend_from_slice(&s.metadata.last_write_time().to_le_bytes());
            if let Some(readers)=readers {out.extend_from_slice(&readers[index].to_le_bytes());}
            if out.len()>64*1024*1024 {return Err("timestamp manifest exceeds bounded worker message budget".into());}
        }
        Ok(out)
    }
    pub(super) fn readers(&self)->impl Iterator<Item=Option<&File>> {
        self.snapshots.iter().map(|s|(!s.metadata.is_dir()).then_some(&s.source))
    }
    pub(super) fn trace_root(&self,_label:&str)->Result<(),String>{Ok(())}
    pub(super) fn verify(&self,ctl:&JobControl)->Result<(),String>{
        for ancestor in &self.ancestors {
            ctl.check()?;
            let m=crate::private_stage::checked_metadata(ancestor)?;
            if !m.is_dir() || m.file_attributes()&0x400!=0 {return Err("source ancestor changed".into());}
        }
        for snapshot in &self.snapshots {snapshot.verify(ctl)?;}
        Ok(())
    }
}
pub(super) fn prepare(paths:&[PathBuf],parent:&Path,opts:&EngineOptions,ctl:&JobControl)->Result<Prepared,String>{
    use std::os::windows::ffi::OsStrExt;
    ctl.check()?;reject_output_in_inputs(parent,paths)?;
    let mut ancestors:Vec<File>=Vec::new();let mut held=std::collections::HashMap::new();
    let mut budget=0usize;
    // Top-down pins make subsequent path lookup unable to retarget an ancestor.
    for root in paths {
        for path in root.ancestors().skip(1).collect::<Vec<_>>().into_iter().rev() {
            if !held.contains_key(path) {
                budget=budget.checked_add(path.as_os_str().encode_wide().count()*2+128).ok_or("source pin budget overflow")?;
                if budget>64*1024*1024{return Err("source pin metadata budget exceeded".into());}
                let file=if let Some(parent)=path.parent() {
                    open_child(&ancestors[*held.get(parent).ok_or("missing ancestor pin")?],path.file_name().ok_or("invalid ancestor leaf")?,0x80,3)?
                }else{File::options().access_mode(0x80).share_mode(3).custom_flags(0x02200000).open(path).map_err(|e|e.to_string())?};
                let m=crate::private_stage::checked_metadata(&file)?;
                if !m.is_dir()||m.file_attributes()&0x400!=0{return Err("linked source ancestor rejected".into());}
                held.insert(path.to_owned(),ancestors.len());ancestors.push(file);
            }
        }
    }
    let mut snapshots=Vec::new();let mut root_indices=Vec::new();
    let mut first_times=std::collections::HashMap::new();
    for root in paths {
        root_indices.push(snapshots.len());
        let parent=root.parent().ok_or("missing source parent")?;
        let parent=ancestors[*held.get(parent).ok_or("source parent pin missing")?].try_clone().map_err(|e|e.to_string())?;
        let mut pending=vec![(root.to_owned(),PathBuf::from(root.file_name().ok_or("missing source name")?),parent)];
        while let Some((path,relative,parent_pin))=pending.pop() {
            ctl.check()?;
            budget=budget.checked_add((path.as_os_str().encode_wide().count()+relative.as_os_str().encode_wide().count())*2+256).ok_or("source pin budget overflow")?;
            if budget>64*1024*1024{return Err("source pin metadata budget exceeded".into());}
            let name=path.file_name().ok_or("missing source leaf")?;
            // Apply suppression before the first full-stream proof read. Only
            // parent owns WRITE_ATTRIBUTES; the child receives a read-only dup.
            let opened=crate::source_access::open_with(0x80000000,opts.preserve_input_atime,
                |access|open_child_io(&parent_pin,name,access,1)).map_err(|e|e.to_string())?;
            let file=opened.file;
            let metadata=crate::private_stage::checked_metadata(&file)?;
            if metadata.file_attributes()&0x400!=0 || (!metadata.is_dir()&&!metadata.is_file()){return Err("linked or special create input rejected".into());}
            if !metadata.is_dir() {
                if let Some(report)=&opts.input_atime_report {report.lock().unwrap_or_else(|e|e.into_inner()).record(&path,opened.preservation);}
            }
            let mut snapshot=Snapshot::capture_reported(&path,&relative,&file,&metadata,ctl,if opts.preserve_input_atime{opts.input_atime_report.as_ref()}else{None})?;
            // A physical file may legitimately appear at several logical paths
            // (overlapping input roots or hardlinks). Keep its first time snapshot.
            snapshot.logical_time=*first_times.entry(snapshot.identity).or_insert(snapshot.logical_time);
            if metadata.is_dir() {
                each_name(&file,ctl,|name| {
                    // Bound scheduled names as well as completed node records.
                    budget=budget.checked_add((path.as_os_str().encode_wide().count()+relative.as_os_str().encode_wide().count()+name.encode_wide().count()*2)*2+128).ok_or("source traversal budget overflow")?;
                    if budget>64*1024*1024{return Err("source traversal metadata budget exceeded".into());}
                    snapshot.children.push(name.clone());pending.push((path.join(&name),relative.join(name),file.try_clone().map_err(|e|e.to_string())?));Ok(())
                })?;
            }
            snapshots.push(snapshot);
        }
    }
    let ready=Prepared{snapshots,ancestors,root_indices,roots:paths.to_vec()};
    ready.snapshot_manifest(None)?;ready.verify(ctl)?;Ok(ready)
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
    fn direct_sources_remain_pinned_without_copy_and_detect_new_children() {
        let root=tempfile::tempdir().unwrap();let source=root.path().join("input");fs::create_dir(&source).unwrap();
        let file=source.join("a.txt");fs::write(&file,b"source data").unwrap();
        let opts=EngineOptions::default();let ctl=JobControl::new();
        let prepared=prepare(&[source.clone()],root.path(),&opts,&ctl).unwrap();
        assert_eq!(prepared.roots,vec![source.clone()]);assert_eq!(prepared.snapshots.len(),2);
        assert!(File::options().write(true).open(&file).is_err());
        assert!(fs::rename(&file,source.join("moved")).is_err());
        assert!(fs::rename(&source,root.path().join("renamed")).is_err());
        prepared.verify(&ctl).unwrap();
        fs::write(source.join("late.txt"),b"new child").unwrap();
        assert!(prepared.verify(&ctl).is_err());
        drop(prepared);assert_eq!(fs::read(file).unwrap(),b"source data");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(),1);
    }
    #[test]
    fn direct_file_roots_and_overlap_cancellation_preserve_sources() {
        let root=tempfile::tempdir().unwrap();let dir=root.path().join("dir");fs::create_dir(&dir).unwrap();
        let file=dir.join("file");fs::write(&file,b"immutable").unwrap();
        let opts=EngineOptions::default();let ctl=JobControl::new();
        let prepared=prepare(&[file.clone()],root.path(),&opts,&ctl).unwrap();
        assert_eq!(prepared.root_handles().count(),1);assert_eq!(prepared.snapshots.len(),1);
        assert!(!prepared.snapshots[0].metadata.is_dir());
        ctl.cancel();assert!(prepared.verify(&ctl).unwrap_err().contains("cancel"));drop(prepared);
        let overlap=prepare(&[dir,file.clone()],root.path(),&opts,&JobControl::new()).unwrap();
        assert_eq!(overlap.snapshots.len(),3);
        assert_eq!(overlap.snapshots[1].logical_time,overlap.snapshots[2].logical_time);drop(overlap);
        assert_eq!(fs::read(file).unwrap(),b"immutable");
    }
    #[test]
    fn direct_encoding_rejects_late_children_and_cancel_without_publication() {
        for cancel in [false,true] {
            let root=tempfile::tempdir().unwrap();let source=root.path().join("source");fs::create_dir(&source).unwrap();
            let input=source.join("random.bin");let mut bytes=vec![0u8;8*1024*1024];let mut seed=31u64;
            for b in &mut bytes {seed^=seed<<13;seed^=seed>>7;seed^=seed<<17;*b=seed as u8;}
            fs::write(&input,&bytes).unwrap();let output=root.path().join("out.7z");
            let ctl=JobControl::new();let finished=std::sync::atomic::AtomicBool::new(false);
            let (result,observed)=std::thread::scope(|scope|{
                let worker=scope.spawn(||{
                    let deadline=std::time::Instant::now()+Duration::from_secs(30);
                    while std::time::Instant::now()<deadline && !finished.load(Ordering::SeqCst) {
                        let progress=ctl.snapshot();
                        if progress.phase=="compressing"&&progress.percent.is_some() {
                            assert!(fs::rename(&input,source.join("replacement")).is_err());
                            assert!(File::options().write(true).open(&input).is_err());
                            if cancel {ctl.cancel();}else{fs::write(source.join("late.txt"),b"late source child").unwrap();}
                            return true;
                        }
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    false
                });
                let result=create_with_control(output.to_str().unwrap(),&[source.display().to_string()],"7z",
                    &EngineOptions{store_atime:Some(true),threads:Some(1),..Default::default()},&ctl);
                finished.store(true,Ordering::SeqCst);(result,worker.join().unwrap())
            });
            assert!(observed,"native encoding did not become observable: {result:?}");
            assert!(result.is_err());assert!(!output.exists());
            assert_eq!(fs::read(input).unwrap(),bytes);
            if !cancel {assert_eq!(fs::read(source.join("late.txt")).unwrap(),b"late source child");}
        }
    }
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
