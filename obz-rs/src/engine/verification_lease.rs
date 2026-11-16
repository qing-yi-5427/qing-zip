//! Process-local full-content evidence with fresh, bounded reuse qualification.
//! This is evidence about retained object bytes, NOT permanent authorization or
//! proof that the containing directory has no newly added volume. Any future
//! reuse must recheck the complete current volume family, live path/access,
//! machine policy and parser/codepage/password configuration before granting
//! output capabilities. Only trusted native selected extraction consumes this;
//! all other routes continue complete decoding.
use super::{Entry, File, PathBuf};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};

/// Not serializable or constructible by callers. The exact handles survive the
/// verified decode and output commit; a path/mtime/size cannot recreate this.
pub struct VerifiedArchive {
    pub(crate) path: PathBuf,
    pub(crate) _first: crate::engine_stream::PinnedSource,
    pub(crate) parts: Vec<File>,
    pub(crate) entries: Vec<Entry>,
    revoked: AtomicBool,
    active:Mutex<Vec<Arc<AtomicBool>>>,
}
impl VerifiedArchive {
    /// Revocation is trusted-process state, never a deserializable permission.
    pub fn revoke(&self) {
        let active=self.active.lock().unwrap_or_else(|e|e.into_inner());
        self.revoked.store(true, Ordering::SeqCst);
        for cancelled in active.iter(){cancelled.store(true,Ordering::SeqCst);}
    }
    pub(super) fn matches(&self, path:&str, first:&File, parts:&[File], entries:&[Entry], opts:&super::EngineOptions)->bool {
        // Fresh callers have already pinned and listed EVERY member, validated
        // paths and policy. Held bytes alone never grant fresh access.
        if self.revoked.load(Ordering::SeqCst) || opts.password.is_some() || opts.codepage.is_some() || opts.threads.is_some()
            || entries.iter().any(|e| e.encrypted) || entries != self.entries || parts.len()!=self.parts.len()
            || std::fs::canonicalize(path).ok().as_ref()!=Some(&self.path) || !family_matches(&self.path,parts.len()+1) { return false; }
        let leaf=self.path.file_name().unwrap().to_str().unwrap();let prefix=&leaf[..leaf.len()-3];
        std::iter::once(first).chain(parts.iter()).zip(std::iter::once(&*self._first).chain(self.parts.iter())).enumerate()
            .all(|(index,(fresh,held))| {
                // A retained read grant is not a current authorization. Reopen
                // every exact current name with the normal no-link/read pins.
                let path=self.path.with_file_name(format!("{prefix}{:03}",index+1));
                let Ok(current)=crate::engine_stream::pin(&path) else{return false};
                let Ok(identity)=super::directory_identity(&current) else{return false};
                super::directory_identity(fresh).ok()==Some(identity) && super::directory_identity(held).ok()==Some(identity)
            })
    }

    pub fn path(&self) -> &std::path::Path { &self.path }
    pub fn volume_count(&self) -> usize { self.parts.len() + 1 }
    pub fn retained_handle_count(&self) -> usize { self._first.retained_handle_count()+self.parts.len() }
    pub fn member_count(&self) -> usize { self.entries.len() }
    pub fn retained_metadata_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<Entry>() + self.entries.iter()
            .map(|e| e.name.capacity()+e.crc.capacity()+e.method.capacity()+e.comment.capacity()+e.attributes.capacity()+e.modified.capacity()).sum::<usize>()
    }
}

/// Installed only by a trusted in-process caller, never deserialized from IPC.
#[derive(Clone, Default)]
pub struct Capture(Arc<Mutex<Option<Arc<VerifiedArchive>>>>, Arc<AtomicBool>, Arc<Mutex<Option<ActiveReuse>>>);
impl Capture {
    pub fn take(&self) -> Option<Arc<VerifiedArchive>> { self.0.lock().unwrap_or_else(|e|e.into_inner()).take() }
    pub(super) fn begin_reuse(&self,proof:Arc<VerifiedArchive>,ctl:&super::JobControl)->bool {
        let mut ticket=self.2.lock().unwrap_or_else(|e|e.into_inner());
        if ticket.is_some(){return false;}
        let mut active=proof.active.lock().unwrap_or_else(|e|e.into_inner());
        if proof.revoked.load(Ordering::SeqCst) || ctl.check().is_err() || active.len()>=32{return false;}
        active.push(ctl.cancelled.clone());drop(active);
        *ticket=Some(ActiveReuse{proof,cancelled:ctl.cancelled.clone()});
        self.1.store(true,Ordering::SeqCst);true
    }
    pub(super) fn scope(&self)->ReuseScope {ReuseScope(self.clone())}
    pub(super) fn finish_reuse(&self)->Result<(),String>{
        let Some(ticket)=self.2.lock().unwrap_or_else(|e|e.into_inner()).take() else{return Ok(())};
        #[cfg(test)]
        BEFORE_REUSE_COMMIT.with(|h|{if let Some(f)=h.borrow_mut().take(){f();}});
        // Final publication fence: no IO under this lock. Revocation either
        // cancels this still-active operation, or follows completed publication.
        let mut active=ticket.proof.active.lock().unwrap_or_else(|e|e.into_inner());
        let revoked=ticket.proof.revoked.load(Ordering::SeqCst)||ticket.cancelled.load(Ordering::SeqCst);
        active.retain(|p|!Arc::ptr_eq(p,&ticket.cancelled));drop(active);
        if revoked{Err("archive verification lease revoked before publication completed".into())}else{
            #[cfg(test)]
            AFTER_REUSE_COMMIT.with(|h|{if let Some(f)=h.borrow_mut().take(){f();}});
            Ok(())
        }
    }
    pub(super) fn complete(&self, path: PathBuf, first: crate::engine_stream::PinnedSource,
        parts: Vec<File>, entries: Vec<Entry>) {
        if self.1.load(Ordering::SeqCst) || entries.iter().any(|e|e.encrypted) {return;}
        let Ok(path)=std::fs::canonicalize(path) else{return};
        if !family_matches(&path,parts.len()+1){return;}
        let proof=Arc::new(VerifiedArchive{path,_first:first,parts,entries,revoked:AtomicBool::new(false),active:Mutex::new(Vec::new())});
        // Best-effort retention must not turn a successful extraction into a
        // failure, nor retain an unbounded inventory in the GUI process.
        if proof.volume_count()<=1024 && proof.retained_handle_count()<=4096 && proof.retained_metadata_bytes()<=16*1024*1024 {
            *self.0.lock().unwrap_or_else(|e|e.into_inner())=Some(proof);
        }
    }
}

struct ActiveReuse {proof:Arc<VerifiedArchive>,cancelled:Arc<AtomicBool>}
impl Drop for ActiveReuse {fn drop(&mut self){self.proof.active.lock().unwrap_or_else(|e|e.into_inner()).retain(|p|!Arc::ptr_eq(p,&self.cancelled));}}
pub(super) struct ReuseScope(Capture);
impl Drop for ReuseScope {fn drop(&mut self){self.0.2.lock().unwrap_or_else(|e|e.into_inner()).take();}}

// Enumerate the entire numeric family, including gaps, extra tails and alternate
// digit widths. This is a namespace check, NOT discovery permission for a codec.
fn family_matches(path:&std::path::Path,count:usize)->bool {
    let Some(name)=path.file_name().and_then(|n|n.to_str()) else{return false};
    let Some(prefix)=name.strip_suffix("001") else{return false};
    let Some(parent)=path.parent() else{return false};
    let Ok(iter)=std::fs::read_dir(parent) else{return false};
    let expected=(1..=count).map(|n|format!("{prefix}{n:03}").to_lowercase()).collect::<std::collections::BTreeSet<_>>();
    let mut actual=std::collections::BTreeSet::new();
    for item in iter {
        let Ok(item)=item else{return false};let name=item.file_name();let Some(name)=name.to_str() else{continue};
        let lower=name.to_lowercase();let prefix=prefix.to_lowercase();
        if let Some(tail)=lower.strip_prefix(&prefix) {
            if !tail.is_empty() && tail.bytes().all(|c|c.is_ascii_digit()) {
                if !expected.contains(&lower) || !actual.insert(lower){return false;}
            }
        }
    }
    actual==expected
}

#[cfg(test)]
thread_local! {pub(super) static BEFORE_REUSE_FINISH:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
thread_local! {
    static BEFORE_REUSE_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};
    static AFTER_REUSE_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};
}
#[cfg(all(test,nana_selected_worker))]
mod tests {
    use super::*;
    use crate::engine::{self, EngineOptions, JobControl};
    use std::{fs, io::{Seek,SeekFrom,Write}, os::windows::fs::OpenOptionsExt};
    fn fixture()->(tempfile::TempDir,PathBuf,Vec<u8>){
        let t=tempfile::tempdir().unwrap();let input=t.path().join("payload.bin");
        let bytes=(0..32768).map(|i|(i%251) as u8).collect::<Vec<_>>();fs::write(&input,&bytes).unwrap();
        let extra=t.path().join("unselected.bin");fs::write(&extra,&bytes).unwrap();
        engine::create(t.path().join("source.7z").to_str().unwrap(),&[input.display().to_string(),extra.display().to_string()],"7z",
            &EngineOptions{method:Some("Copy".into()),volume_bytes:Some(4096),..Default::default()}).unwrap();
        let first=t.path().join("source.7z.001");assert!(first.exists());(t,first,bytes)
    }
    fn options(capture:&Capture)->EngineOptions{EngineOptions{selected:vec!["payload.bin".into()],verification_capture:Some(capture.clone()),..Default::default()}}
    #[test]
    fn complete_selected_decode_retains_every_original_volume_until_drop(){
        let(t,first,bytes)=fixture();let c=Capture::default();let out=t.path().join("out");
        assert_eq!(engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&options(&c)).unwrap(),1);
        assert_eq!(fs::read(out.join("payload.bin")).unwrap(),bytes);
        let proof=c.take().expect("real native complete verification must issue proof");
        assert!(proof.volume_count()>1);assert_eq!(proof.member_count(),2);
        assert!(!out.join("unselected.bin").exists());
        for n in 1..=proof.volume_count(){let p=first.with_file_name(format!("source.7z.{n:03}"));
            assert!(File::options().write(true).share_mode(7).open(&p).is_err());
            assert!(fs::rename(&p,p.with_extension("moved")).is_err());
        }
        assert!(fs::rename(t.path(),t.path().with_extension("moved")).is_err());
        drop(proof);assert!(File::options().write(true).share_mode(7).open(&first).is_ok());
    }
    #[test]
    fn failed_cancelled_and_corrupt_operations_never_issue_proof(){
        let(t,first,_)=fixture();let c=Capture::default();let mut opts=options(&c);
        let ctl=JobControl::new();ctl.cancel();opts.control=Some(ctl);
        assert!(engine::extract(first.to_str().unwrap(),t.path().join("cancel").to_str().unwrap(),&opts).is_err());assert!(c.take().is_none());
        let out=t.path().join("existing");fs::create_dir(&out).unwrap();fs::write(out.join("payload.bin"),b"keep").unwrap();
        assert!(engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&options(&c)).is_err());assert!(c.take().is_none());assert_eq!(fs::read(out.join("payload.bin")).unwrap(),b"keep");
        // Copy coder: this byte is in the second, unselected 32 KiB payload.
        let mut f=File::options().write(true).open(first.with_file_name("source.7z.010")).unwrap();f.seek(SeekFrom::Start(100)).unwrap();f.write_all(b"corrupt").unwrap();drop(f);
        assert!(engine::extract(first.to_str().unwrap(),t.path().join("bad").to_str().unwrap(),&options(&c)).is_err());assert!(c.take().is_none());
    }
    #[test]
    fn serialized_requests_cannot_supply_a_capture(){
        let opts:EngineOptions=serde_json::from_str(r#"{"verification_capture":true,"verification_reuse":true}"#).unwrap();
        assert!(opts.verification_capture.is_none());assert!(opts.verification_reuse.is_none());
    }
    #[test]
    fn real_selected_reuse_and_extra_volume_fallback() {
        let(t,first,bytes)=fixture();let full=Capture::default();
        engine::extract(first.to_str().unwrap(),t.path().join("first").to_str().unwrap(),&options(&full)).unwrap();
        let proof=full.take().unwrap();
        for (name,extra) in [("reuse",false),("extra-volume",true)] {
            if extra {fs::write(first.with_file_name("source.7z.999"),b"extra").unwrap();}
            let c=Capture::default();let mut opts=options(&c);opts.verification_reuse=Some(proof.clone());
            engine::extract(first.to_str().unwrap(),t.path().join(name).to_str().unwrap(),&opts).unwrap();
            assert_eq!(fs::read(t.path().join(name).join("payload.bin")).unwrap(),bytes);
            assert_eq!(c.1.load(Ordering::SeqCst),!extra,"must prove actual partial/full route");
            assert!(c.take().is_none(),"partial cannot issue a new full proof; extra family cannot qualify");
        }
    }
    #[test]
    fn solid_selected_reuse_checks_data_and_revocation_falls_back() {
        let t=tempfile::tempdir().unwrap();let mut paths=Vec::new();
        for n in 0..3 {let p=t.path().join(format!("{n}.txt"));fs::write(&p,vec![65+n;32768]).unwrap();paths.push(p.display().to_string());}
        engine::create(t.path().join("solid.7z").to_str().unwrap(),&paths,"7z",&EngineOptions{method:Some("LZMA2".into()),volume_bytes:Some(64),..Default::default()}).unwrap();
        let first=t.path().join("solid.7z.001");let c=Capture::default();
        let mut opts=EngineOptions{selected:vec!["2.txt".into()],verification_capture:Some(c.clone()),..Default::default()};
        engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&opts).unwrap();let proof=c.take().unwrap();
        for revoked in [false,true] {if revoked{proof.revoke();}let c=Capture::default();opts.verification_capture=Some(c.clone());opts.verification_reuse=Some(proof.clone());let out=t.path().join(format!("reuse-{revoked}"));
            engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&opts).unwrap();
            assert_eq!(fs::read(out.join("2.txt")).unwrap(),vec![67;32768]);assert!(!out.join("0.txt").exists());
            assert_eq!(c.1.load(Ordering::SeqCst),!revoked);
        }
    }

    #[test]
    fn reuse_revocation_or_new_tail_before_publication_keeps_outputs_private(){
        for revoke in [false,true] {
            let(t,first,_)=fixture();let c=Capture::default();
            engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&options(&c)).unwrap();let proof=c.take().unwrap();
            let captured=proof.clone();let tail=first.with_file_name("source.7z.0999");
            BEFORE_REUSE_FINISH.with(|h|*h.borrow_mut()=Some(Box::new(move||{if revoke{captured.revoke();}else{fs::write(tail,b"late tail").unwrap();}})));
            let c=Capture::default();let mut opts=options(&c);opts.verification_reuse=Some(proof);
            let out=t.path().join("unpublished");assert!(engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&opts).is_err());
            assert!(!out.join("payload.bin").exists());assert!(c.take().is_none());assert!(c.1.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn proof_rejects_changed_inventory_configuration_or_identity(){
        let(t,first,_)=fixture();let c=Capture::default();engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&options(&c)).unwrap();let proof=c.take().unwrap();
        let first_pin=crate::engine_stream::pin(&first).unwrap();let parts=crate::engine_stream::pin_numbered_7z_parts(&first).unwrap();let opts=options(&Capture::default());
        assert!(proof.matches(first.to_str().unwrap(),&first_pin,&parts,&proof.entries,&opts));
        let mut wrong=proof.entries.clone();wrong[0].attributes.push_str("changed");assert!(!proof.matches(first.to_str().unwrap(),&first_pin,&parts,&wrong,&opts));
        let mut changed=opts.clone();changed.codepage=Some(65001);assert!(!proof.matches(first.to_str().unwrap(),&first_pin,&parts,&proof.entries,&changed));
        changed=opts.clone();changed.password=Some("different".into());assert!(!proof.matches(first.to_str().unwrap(),&first_pin,&parts,&proof.entries,&changed));
        changed=opts.clone();changed.threads=Some(4);assert!(!proof.matches(first.to_str().unwrap(),&first_pin,&parts,&proof.entries,&changed));
        let other=t.path().join("other");fs::write(&other,b"same size is irrelevant").unwrap();let other=File::open(other).unwrap();assert!(!proof.matches(first.to_str().unwrap(),&other,&parts,&proof.entries,&opts));
    }

    #[test]
    fn selected_directory_and_empty_directory_reuse_preserve_projection(){
        let t=tempfile::tempdir().unwrap();let folder=t.path().join("folder");fs::create_dir_all(folder.join("empty")).unwrap();fs::write(folder.join("a.txt"),vec![42;16384]).unwrap();let other=t.path().join("other.txt");fs::write(&other,b"unselected").unwrap();
        engine::create(t.path().join("tree.7z").to_str().unwrap(),&[folder.display().to_string(),other.display().to_string()],"7z",&EngineOptions{method:Some("Copy".into()),volume_bytes:Some(4096),..Default::default()}).unwrap();let first=t.path().join("tree.7z.001");
        let mut proof=None;
        for round in 0..2 {let c=Capture::default();let out=t.path().join(format!("out-{round}"));let opts=EngineOptions{selected:vec!["folder/".into()],verification_capture:Some(c.clone()),verification_reuse:proof.clone(),..Default::default()};
            engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&opts).unwrap();assert_eq!(fs::read(out.join("folder/a.txt")).unwrap(),vec![42;16384]);assert!(out.join("folder/empty").is_dir());assert!(!out.join("other.txt").exists());
            assert_eq!(c.1.load(Ordering::SeqCst),round==1);if round==0{proof=Some(c.take().unwrap());}else{assert!(c.take().is_none());}
        }
    }

    #[test]
    fn retained_read_pin_does_not_bypass_fresh_read_permission(){
        use std::{ffi::c_void,os::windows::io::AsRawHandle};
        #[link(name="advapi32")] unsafe extern "system" {
            fn GetSecurityInfo(h:*mut c_void,kind:u32,info:u32,owner:*mut *mut c_void,group:*mut *mut c_void,dacl:*mut *mut c_void,sacl:*mut *mut c_void,sd:*mut *mut c_void)->u32;
            fn SetSecurityInfo(h:*mut c_void,kind:u32,info:u32,owner:*mut c_void,group:*mut c_void,dacl:*mut c_void,sacl:*mut c_void)->u32;
            fn GetSecurityDescriptorControl(sd:*mut c_void,control:*mut u16,revision:*mut u32)->i32;
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,sd:*mut *mut c_void,len:*mut u32)->i32;
            fn GetSecurityDescriptorDacl(sd:*mut c_void,present:*mut i32,dacl:*mut *mut c_void,defaulted:*mut i32)->i32;
        }
        #[link(name="kernel32")] unsafe extern "system" {fn LocalFree(p:*mut c_void)->*mut c_void;}
        struct Restore{file:File,sd:*mut c_void,acl:*mut c_void,flags:u32}
        impl Drop for Restore{fn drop(&mut self){unsafe{let restored=SetSecurityInfo(self.file.as_raw_handle(),1,self.flags,std::ptr::null_mut(),std::ptr::null_mut(),self.acl,std::ptr::null_mut());LocalFree(self.sd);assert_eq!(restored,0,"fixture DACL restoration failed");}}}
        let(t,first,_)=fixture();let c=Capture::default();engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&options(&c)).unwrap();let proof=c.take().unwrap();
        let file=File::options().access_mode(0x60000).share_mode(1).open(&first).unwrap();let mut sd=std::ptr::null_mut();let mut acl=std::ptr::null_mut();
        assert_eq!(unsafe{GetSecurityInfo(file.as_raw_handle(),1,4,std::ptr::null_mut(),std::ptr::null_mut(),&mut acl,std::ptr::null_mut(),&mut sd)},0);let mut flags=0;let mut revision=0;assert_ne!(unsafe{GetSecurityDescriptorControl(sd,&mut flags,&mut revision)},0);
        let restore=Restore{file,sd,acl,flags:4|if flags&0x1000!=0{0x80000000}else{0x20000000}};
        let text="D:P(D;;GR;;;WD)(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect::<Vec<_>>();let mut denied=std::ptr::null_mut();assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(),1,&mut denied,std::ptr::null_mut())},0);let mut present=0;let mut defaulted=0;let mut deny_acl=std::ptr::null_mut();assert_ne!(unsafe{GetSecurityDescriptorDacl(denied,&mut present,&mut deny_acl,&mut defaulted)},0);
        let result=unsafe{SetSecurityInfo(restore.file.as_raw_handle(),1,4|0x80000000,std::ptr::null_mut(),std::ptr::null_mut(),deny_acl,std::ptr::null_mut())};unsafe{LocalFree(denied)};assert_eq!(result,0);
        assert!(crate::engine_stream::pin(&first).is_err(),"real fresh read must be denied");
        assert!(!proof.matches(first.to_str().unwrap(),&proof._first,&proof.parts,&proof.entries,&options(&Capture::default())));
        drop(restore);assert!(crate::engine_stream::pin(&first).is_ok(),"original permissions restored");
    }

    #[test]
    fn revocation_at_final_commit_rolls_back_or_follows_completed_task(){
        for before in [true,false] {
            let(t,first,bytes)=fixture();let c=Capture::default();engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&options(&c)).unwrap();let proof=c.take().unwrap();
            let out=t.path().join("target");let leaf=out.join("payload.bin");let held=proof.clone();let callback=Box::new(move||{assert!(leaf.is_file(),"hook must run after final output has actually published");held.revoke();});
            if before {BEFORE_REUSE_COMMIT.with(|h|*h.borrow_mut()=Some(callback));}else{AFTER_REUSE_COMMIT.with(|h|*h.borrow_mut()=Some(callback));}
            let c=Capture::default();let mut opts=options(&c);opts.verification_reuse=Some(proof);let result=engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&opts);
            if before {assert!(result.is_err());assert!(!out.exists(),"owned new destination must roll back");}else{assert!(result.is_ok());assert_eq!(fs::read(out.join("payload.bin")).unwrap(),bytes);}
            assert!(c.take().is_none());
        }
    }

    #[test]
    fn existing_destination_revocation_retains_completed_outputs_and_foreign_files(){
        for overwrite in ["error","overwrite"] {
            let(t,first,bytes)=fixture();let c=Capture::default();engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&options(&c)).unwrap();let proof=c.take().unwrap();
            let out=t.path().join("existing");fs::create_dir(&out).unwrap();fs::write(out.join("foreign.txt"),b"keep").unwrap();if overwrite=="overwrite"{fs::write(out.join("payload.bin"),b"authorized replacement").unwrap();}
            let held=proof.clone();BEFORE_REUSE_COMMIT.with(|h|*h.borrow_mut()=Some(Box::new(move||held.revoke())));
            let c=Capture::default();let mut opts=options(&c);opts.overwrite=overwrite.into();opts.verification_reuse=Some(proof);assert!(engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&opts).is_err());
            assert_eq!(fs::read(out.join("foreign.txt")).unwrap(),b"keep");assert_eq!(fs::read(out.join("payload.bin")).unwrap(),bytes,"existing destination follows normal partial completion semantics");assert!(c.take().is_none());
        }
    }
    #[test]
    fn existing_conflict_never_overwrites_old_file_during_reuse(){
        let(t,first,_)=fixture();let c=Capture::default();engine::extract(first.to_str().unwrap(),t.path().join("full").to_str().unwrap(),&options(&c)).unwrap();let proof=c.take().unwrap();
        let out=t.path().join("existing");fs::create_dir(&out).unwrap();fs::write(out.join("payload.bin"),b"old").unwrap();let c=Capture::default();let mut opts=options(&c);opts.verification_reuse=Some(proof);
        assert!(engine::extract(first.to_str().unwrap(),out.to_str().unwrap(),&opts).is_err());assert_eq!(fs::read(out.join("payload.bin")).unwrap(),b"old");assert!(c.take().is_none());
    }

}
