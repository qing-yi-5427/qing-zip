//! Bounded GUI-owned verification lifetimes; no frontend token grants trust.
use obz_core::{engine::verification_lease::{Capture, VerifiedArchive}, operations::Request};
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Registry { generation: u128, proofs: Vec<VerifiedArchive> }
static REGISTRY: OnceLock<Mutex<Registry>>=OnceLock::new();
fn registry()->&'static Mutex<Registry>{REGISTRY.get_or_init(Default::default)}
pub(crate) struct Pending { generation:u128, capture:Capture }
pub(crate) fn prepare(request:&mut Request,recycle:bool)->Option<Pending>{
    // First-stage invalidation includes listing/reopening and all edits. No
    // alias spelling can preserve a proof across a mutating request.
    if request.operation!="extract" || recycle {revoke();return None;}
    if request.options.selected.is_empty(){return None;}
    let capture=Capture::default();
    request.options.verification_capture=Some(capture.clone());
    let generation=registry().lock().unwrap_or_else(|e|e.into_inner()).generation;
    Some(Pending{generation,capture})
}
impl Pending {
    pub(crate) fn finish(self,success:bool){
        let Some(proof)=self.capture.take().filter(|_|success) else{return};
        let mut state=registry().lock().unwrap_or_else(|e|e.into_inner());
        if self.generation!=state.generation{return;}
        state.proofs.retain(|old|old.path()!=proof.path());
        while !state.proofs.is_empty() && (state.proofs.len()>=4 ||
            state.proofs.iter().map(VerifiedArchive::retained_handle_count).sum::<usize>()+proof.retained_handle_count()>4096 ||
            state.proofs.iter().map(VerifiedArchive::retained_metadata_bytes).sum::<usize>()+proof.retained_metadata_bytes()>16*1024*1024){
            state.proofs.remove(0);
        }
        state.proofs.push(proof);
    }
}
pub(crate) fn revoke(){
    let mut state=registry().lock().unwrap_or_else(|e|e.into_inner());
    state.generation=state.generation.checked_add(1).expect("verification generation exhausted");
    state.proofs.clear();
}
/// Revocation only; JavaScript cannot create or authorize reuse of a proof.
#[tauri::command]
pub fn revoke_archive_verification(){revoke();}

#[cfg(test)]
mod tests {
    use super::*;
    use obz_core::{engine::{self,EngineOptions},operations};
    use std::{fs,path::PathBuf,time::Duration};
    fn fixture()->(tempfile::TempDir,PathBuf){
        let t=tempfile::tempdir().unwrap();let input=t.path().join("payload.bin");
        fs::write(&input,(0..16384).map(|i|(i%251)as u8).collect::<Vec<_>>()).unwrap();
        engine::create(t.path().join("source.7z").to_str().unwrap(),&[input.display().to_string()],"7z",
            &EngineOptions{method:Some("Copy".into()),volume_bytes:Some(4096),..Default::default()}).unwrap();
        let first=t.path().join("source.7z.001");(t,first)
    }
    fn request(first:&std::path::Path,dest:&std::path::Path)->Request{Request{operation:"extract".into(),path:first.display().to_string(),dest:dest.display().to_string(),options:EngineOptions{selected:vec!["payload.bin".into()],..Default::default()},..Default::default()}}
    #[test]
    fn real_job_retains_proof_then_mutation_preparation_releases_it(){
        let _serial=crate::jobs::test_registry_guard();revoke();let(t,first)=fixture();
        let id=crate::jobs::start_operation(request(&first,&t.path().join("out")),None).unwrap();
        loop {let snap=serde_json::to_value(crate::jobs::operation_status(id).unwrap()).unwrap();if snap["status"]!="running" {assert_eq!(snap["status"],"complete","{:?}",snap["error"]);break;}std::thread::sleep(Duration::from_millis(10));}
        crate::jobs::forget_operation(id).unwrap();
        assert_eq!(registry().lock().unwrap().proofs.len(),1);
        assert!(fs::OpenOptions::new().write(true).open(&first).is_err());
        let mut mutation=Request{operation:"add".into(),path:first.display().to_string(),..Default::default()};assert!(prepare(&mut mutation,false).is_none());
        assert!(registry().lock().unwrap().proofs.is_empty());assert!(fs::OpenOptions::new().write(true).open(&first).is_ok());
    }
    #[test]
    fn revocation_during_completion_cannot_resurrect_proof(){
        let _serial=crate::jobs::test_registry_guard();revoke();let(t,first)=fixture();
        for cancel in [false,true] {
            let mut r=request(&first,&t.path().join(format!("out-{cancel}")));let pending=prepare(&mut r,false).unwrap();
            operations::execute_request(r).unwrap();
            if cancel {crate::jobs::cancel_all();}else{revoke_archive_verification();}
            // Even a stale success decision made before cancellation cannot
            // authorize installation after the generation was revoked.
            pending.finish(true);
            assert!(registry().lock().unwrap().proofs.is_empty());assert!(fs::OpenOptions::new().write(true).open(&first).is_ok());
        }
    }
    #[test]
    fn cancel_operation_revokes_a_just_completed_proof(){
        let _serial=crate::jobs::test_registry_guard();revoke();let(t,first)=fixture();
        let id=crate::jobs::start_operation(request(&first,&t.path().join("out")),None).unwrap();
        loop {let snap=serde_json::to_value(crate::jobs::operation_status(id).unwrap()).unwrap();if snap["status"]!="running" {assert_eq!(snap["status"],"complete");break;}std::thread::sleep(Duration::from_millis(10));}
        assert_eq!(registry().lock().unwrap().proofs.len(),1);
        crate::jobs::cancel_operation(id).unwrap();
        assert!(registry().lock().unwrap().proofs.is_empty());assert!(fs::OpenOptions::new().write(true).open(&first).is_ok());
        crate::jobs::forget_operation(id).unwrap();
    }
    #[test]
    fn failed_job_result_discards_completed_engine_capture(){
        let _serial=crate::jobs::test_registry_guard();revoke();let(t,first)=fixture();
        let mut r=request(&first,&t.path().join("out"));let pending=prepare(&mut r,false).unwrap();
        operations::execute_request(r).unwrap();pending.finish(false);
        assert!(registry().lock().unwrap().proofs.is_empty());assert!(fs::OpenOptions::new().write(true).open(&first).is_ok());
    }
}
