//! Bounded GUI-owned verification lifetimes; no frontend token grants trust.
use obz_core::{engine::verification_lease::{Capture, VerifiedArchive}, operations::Request};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default)]
struct Registry { generation: u128, proofs: Vec<Arc<VerifiedArchive>> }
static REGISTRY: OnceLock<Mutex<Registry>>=OnceLock::new();
fn registry()->&'static Mutex<Registry>{REGISTRY.get_or_init(Default::default)}
pub(crate) struct Pending { generation:u128, capture:Capture }
pub(crate) fn prepare(request:&mut Request,recycle:bool)->Option<Pending>{
    // Listing is the UI's non-mutating refresh. Full metadata is still compared
    // again before reuse; every other non-extract request revokes before work.
    if request.operation=="list" && !recycle{return None;}
    if request.operation!="extract" || recycle {revoke();return None;}
    if request.options.selected.is_empty(){return None;}
    let capture=Capture::default();
    request.options.verification_capture=Some(capture.clone());
    let state=registry().lock().unwrap_or_else(|e|e.into_inner());
    let path=std::fs::canonicalize(&request.path).ok();
    request.options.verification_reuse=state.proofs.iter().find(|p|Some(p.path())==path.as_deref()).cloned();
    let generation=state.generation;
    Some(Pending{generation,capture})
}
impl Pending {
    pub(crate) fn finish(self,success:bool){
        let Some(proof)=self.capture.take().filter(|_|success) else{return};
        let mut state=registry().lock().unwrap_or_else(|e|e.into_inner());
        if self.generation!=state.generation{return;}
        state.proofs.retain(|old|{if old.path()==proof.path(){old.revoke();false}else{true}});
        while !state.proofs.is_empty() && (state.proofs.len()>=4 ||
            state.proofs.iter().map(|p|p.retained_handle_count()).sum::<usize>()+proof.retained_handle_count()>4096 ||
            state.proofs.iter().map(|p|p.retained_metadata_bytes()).sum::<usize>()+proof.retained_metadata_bytes()>16*1024*1024){
            state.proofs.remove(0).revoke();
        }
        state.proofs.push(proof);
    }
}
pub(crate) fn revoke(){
    let mut state=registry().lock().unwrap_or_else(|e|e.into_inner());
    state.generation=state.generation.checked_add(1).expect("verification generation exhausted");
    for proof in &state.proofs {proof.revoke();}
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
    #[test]
    fn consecutive_real_jobs_reuse_native_proof_without_reissuing_it(){
        let _serial=crate::jobs::test_registry_guard();revoke();let(t,first)=fixture();
        let mut previous=None;
        for round in 0..2 {
            let out=t.path().join(format!("round-{round}"));let id=crate::jobs::start_operation(request(&first,&out),None).unwrap();
            loop {let snap=serde_json::to_value(crate::jobs::operation_status(id).unwrap()).unwrap();if snap["status"]!="running"{assert_eq!(snap["status"],"complete","{snap}");break;}std::thread::sleep(Duration::from_millis(10));}
            crate::jobs::forget_operation(id).unwrap();assert_eq!(fs::read(out.join("payload.bin")).unwrap(),fs::read(t.path().join("payload.bin")).unwrap());
            let current=registry().lock().unwrap().proofs[0].clone();
            if let Some(prior)=previous {assert!(Arc::ptr_eq(&prior,&current),"reuse must retain original proof, never issue fresh evidence from partial decoding");}
            previous=Some(current);
        }
        drop(previous);revoke();assert!(fs::OpenOptions::new().write(true).open(first).is_ok());
    }

}
