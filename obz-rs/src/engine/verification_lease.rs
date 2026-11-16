//! Process-local evidence ownership. No API here authorizes skipping validation.
//! This is evidence about retained object bytes, NOT permanent authorization or
//! proof that the containing directory has no newly added volume. Any future
//! reuse must recheck the complete current volume family, live path/access,
//! machine policy and parser/codepage/password configuration before granting
//! output capabilities. The present implementation never skips any decode.
use super::{Entry, File, PathBuf};
use std::sync::{Arc, Mutex};

/// Not serializable or constructible by callers. The exact handles survive the
/// verified decode and output commit; a path/mtime/size cannot recreate this.
pub struct VerifiedArchive {
    pub(crate) path: PathBuf,
    pub(crate) _first: crate::engine_stream::PinnedSource,
    pub(crate) parts: Vec<File>,
    pub(crate) entries: Vec<Entry>,
}
impl VerifiedArchive {
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
pub struct Capture(Arc<Mutex<Option<VerifiedArchive>>>);
impl Capture {
    pub fn take(&self) -> Option<VerifiedArchive> { self.0.lock().unwrap_or_else(|e|e.into_inner()).take() }
    pub(super) fn complete(&self, path: PathBuf, first: crate::engine_stream::PinnedSource,
        parts: Vec<File>, entries: Vec<Entry>) {
        let proof=VerifiedArchive{path,_first:first,parts,entries};
        // Best-effort retention must not turn a successful extraction into a
        // failure, nor retain an unbounded inventory in the GUI process.
        if proof.volume_count()<=1024 && proof.retained_handle_count()<=4096 && proof.retained_metadata_bytes()<=16*1024*1024 {
            *self.0.lock().unwrap_or_else(|e|e.into_inner())=Some(proof);
        }
    }
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
        let opts:EngineOptions=serde_json::from_str(r#"{"verification_capture":true}"#).unwrap();
        assert!(opts.verification_capture.is_none());
    }
}
