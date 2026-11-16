//! Target is committed and kept strongly pinned before any transactional source cleanup.
use super::*;
use archive_add::move_proof::Plan;
use std::os::windows::fs::MetadataExt;
#[derive(Clone,Serialize,Debug)]
#[serde(rename_all="snake_case")]
pub enum Status { Skipped, CopiedRetained, Moved, Uncertain }
#[derive(Serialize,Debug)]
pub struct Item {pub source:String,pub member:String,pub directory:bool,pub status:Status,pub reason:String}
#[derive(Default,Serialize,Debug)]
pub struct Report {pub target_committed:bool,pub cancelled:bool,pub partial:bool,pub summary:String,pub items:Vec<Item>,pub errors:Vec<String>}
impl Report {
    pub(super) fn summarize(&mut self){
        let moved=self.items.iter().filter(|i|matches!(i.status,Status::Moved)).count();
        let retained=self.items.iter().filter(|i|matches!(i.status,Status::CopiedRetained)).count();
        let skipped=self.items.iter().filter(|i|matches!(i.status,Status::Skipped)).count();
        let uncertain=self.items.iter().filter(|i|matches!(i.status,Status::Uncertain)).count();
        self.partial=self.cancelled||retained!=0||skipped!=0||uncertain!=0;
        self.summary=if !self.target_committed{format!("已跳过 {skipped} 项，压缩包未修改")}
        else{format!("{}；已移动 {moved} 项，保留来源 {retained} 项，跳过 {skipped} 项{}",if self.cancelled{"已取消后续移动，压缩包已保存"}else if self.partial{"部分移动完成，压缩包已保存"}else{"移动完成"},if uncertain>0{format!("，{uncertain} 项状态需核查")}else{String::new()})};
    }

    pub(super) fn plan(&mut self,plan:&Plan){self.items=plan.items.iter().map(|item|Item{source:item.source.display().to_string(),member:item.member.clone(),directory:item.directory,status:if item.skipped.is_some(){Status::Skipped}else{Status::CopiedRetained},reason:item.skipped.clone().unwrap_or_else(||"目标副本已保留，来源尚未清理".into())}).collect();}
}
#[cfg(test)]
thread_local! {pub(super) static BEFORE_CLEANUP:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};pub(super) static AFTER_SOURCE_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};pub(super) static BEFORE_SOURCE_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}
/// The target guard is required by type and remains borrowed until every source
/// transaction has reached a known outcome. Cancellation never deletes the target.
pub(super) fn finish(plan:&Plan,_target:&edit_source::CommittedTargetGuard,opts:&EngineOptions,report:&mut Report){
    let ctl=control(opts);
    #[cfg(test)] BEFORE_CLEANUP.with(|hook|if let Some(run)=hook.borrow_mut().take(){run()});
    let mut volumes:BTreeMap<u32,Vec<usize>>=BTreeMap::new();
    for (index,item) in plan.items.iter().enumerate(){if item.skipped.is_none()&&!item.directory{volumes.entry(item.source_id.0).or_default().push(index);}}
    for indices in volumes.values(){for batch in indices.chunks(128){
        if ctl.check().is_err(){report.cancelled=true;return;}
        cleanup_batch(plan,batch,&ctl,report);
        if report.cancelled{return;}
    }}
    // Directory cleanup is nonrecursive and bottom-up. Unknown/new children
    // make the OS reject deletion; skipped/retained descendants keep the parent.
    let mut directories:Vec<_>=plan.items.iter().enumerate().filter(|(_,i)|i.directory&&i.skipped.is_none()).map(|(n,_)|n).collect();
    directories.sort_by_key(|i|std::cmp::Reverse(plan.items[*i].source.components().count()));
    for index in directories{
        if ctl.check().is_err(){report.cancelled=true;return;}
        let source=&plan.items[index].source;
        if plan.items.iter().enumerate().any(|(child,item)|child!=index&&item.source.starts_with(source)&&!matches!(report.items[child].status,Status::Moved)){
            report.items[index].reason="部分子项目仍保留，源文件夹未删除".into();continue;
        }
        cleanup_batch(plan,&[index],&ctl,report);
        if report.cancelled{return;}
    }
}
fn cleanup_batch(plan:&Plan,indices:&[usize],ctl:&JobControl,report:&mut Report){
    use super::file_workspace_transaction::Transaction;
    // Unbounded parent-name pins: unlike CheckedDirectories this cannot fall
    // back to an unpinned path after a cache limit or sharing conflict.
    let mut namespace=Vec::new();let mut eligible=Vec::new();
    for &index in indices{
        match crate::engine_stream::pin_parent_names(&plan.items[index].source){Ok(pins)=>{namespace.push(pins);eligible.push(index);},Err(error)=>report.items[index].reason=format!("无法固定来源路径，保留来源：{error}")}
    }
    if eligible.is_empty(){return;}
    let transaction=match Transaction::begin(){Ok(t)=>t,Err(error)=>{for &i in &eligible{report.items[i].reason=error.clone();}return;}};
    let mut deleted=Vec::new();let mut touched=Vec::new();
    let prepared=(||->Result<(),String>{
        for &index in &eligible{
            ctl.check()?;let item=&plan.items[index];
            let result=(||->Result<(),String>{
                let source=transaction.lock(&item.source,item.source_id)?;
                let meta=source.reader().metadata().map_err(|e|e.to_string())?;
                if meta.is_dir()!=item.directory || meta.file_attributes()&0x4400!=0{return Err("来源类型或属性已改变，保留来源".into());}
                let digest=file_workspace::archive_primary_proof(source.reader(),ctl)?;
                if !item.directory&&(meta.len()!=item.bytes||Some(digest)!=item.sha256){return Err("来源内容已改变，目标保留快照，来源保持不变".into());}
                touched.push(index);source.delete()?;Ok(())
            })();
            match result{Ok(())=>deleted.push(index),Err(error)=>report.items[index].reason=error}
        }
        ctl.check()?;Ok(())
    })();
    if let Err(error)=prepared{report.cancelled=ctl.check().is_err();report.errors.push(error);if let Err(error)=transaction.rollback(){report.errors.push(error);for &i in &touched{report.items[i].status=Status::Uncertain;}}return;}
    #[cfg(test)] BEFORE_SOURCE_COMMIT.with(|hook|if let Some(run)=hook.borrow_mut().take(){run()});
    if let Err(error)=ctl.check(){report.cancelled=true;report.errors.push(error);if let Err(error)=transaction.rollback(){report.errors.push(error);for &i in &touched{report.items[i].status=Status::Uncertain;}}return;}
    match transaction.commit_checked(){
        Ok(())=>{for index in deleted{report.items[index].status=Status::Moved;report.items[index].reason.clear();}
            #[cfg(test)] AFTER_SOURCE_COMMIT.with(|hook|if let Some(run)=hook.borrow_mut().take(){run()});
        },
        Err(error)=>{report.errors.push(error.message.clone());for index in touched{report.items[index].reason=error.message.clone();report.items[index].status=match error.outcome{super::file_workspace_transaction::CommitOutcome::ConfirmedRolledBack=>Status::CopiedRetained,super::file_workspace_transaction::CommitOutcome::Unknown=>Status::Uncertain};}}
    }
    drop(namespace);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(format:&str,collision:bool)->(tempfile::TempDir,PathBuf,PathBuf){
        let t=tempfile::tempdir().unwrap();let old=t.path().join("seed");fs::create_dir_all(old.join("target/docs")).unwrap();
        fs::write(old.join("old.txt"),b"unrelated").unwrap();if collision{fs::write(old.join("target/docs/a.txt"),b"old-a").unwrap();}
        let archive=t.path().join(format!("case.{format}"));create(archive.to_str().unwrap(),&[old.join("old.txt").display().to_string(),old.join("target").display().to_string()],format,&EngineOptions::default()).unwrap();
        let local=t.path().join("docs");fs::create_dir(&local).unwrap();fs::write(local.join("a.txt"),b"new-a").unwrap();fs::write(local.join("b.txt"),b"new-b").unwrap();(t,archive,local)
    }
    #[test]
    fn archive_move_cancel_after_committed_batch_keeps_exact_partial_ledger(){
        let(_t,archive,local)=fixture("zip",false);for i in 0..130{fs::write(local.join(format!("extra-{i}.txt")),b"payload").unwrap();}
        let ctl=JobControl::new();let cancel=ctl.clone();AFTER_SOURCE_COMMIT.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        let opts=EngineOptions{control:Some(ctl.clone()),..Default::default()};let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&opts).unwrap();
        assert!(report.cancelled&&report.target_committed&&report.partial);assert!(report.summary.contains("已取消后续移动"));assert_eq!(report.items.iter().filter(|i|matches!(i.status,Status::Moved)).count(),128);
        for item in &report.items{assert_eq!(Path::new(&item.source).exists(),!matches!(item.status,Status::Moved));}assert!(local.exists());test(archive.to_str().unwrap(),&EngineOptions::default()).unwrap();
    }
    #[test]
    fn archive_move_transaction_unavailable_or_commit_rollback_reports_retained(){
        for commit in [false,true]{let(_t,archive,local)=fixture("zip",false);
            BEFORE_CLEANUP.with(|h|*h.borrow_mut()=Some(Box::new(move||if commit{super::super::file_workspace_transaction::FAIL_COMMIT.with(|v|v.set(true));}else{super::super::file_workspace_transaction::FAIL_BEGIN.with(|v|v.set(true));})));
            let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&EngineOptions::default()).unwrap();
            assert!(report.target_committed);assert_eq!(fs::read(local.join("a.txt")).unwrap(),b"new-a");assert_eq!(fs::read(local.join("b.txt")).unwrap(),b"new-b");
            assert!(report.items.iter().all(|i|matches!(i.status,Status::CopiedRetained)),"{:?}",report);
        }
    }
    #[test]
    fn archive_move_empty_merge_and_replaced_source_identity_are_safe(){
        let(_t,archive,local)=fixture("zip",false);fs::remove_file(local.join("a.txt")).unwrap();fs::remove_file(local.join("b.txt")).unwrap();
        let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&EngineOptions::default()).unwrap();assert!(report.target_committed);assert!(!local.exists(),"{:?}",report);
        let(_t,archive,local)=fixture("zip",false);let changed=local.join("a.txt");let saved=local.join("saved-original.txt");let target=archive.clone();
        BEFORE_CLEANUP.with(|h|*h.borrow_mut()=Some(Box::new(move||{assert!(File::options().write(true).open(target).is_err());fs::rename(&changed,&saved).unwrap();fs::write(changed,b"replacement").unwrap();})));
        let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&EngineOptions::default()).unwrap();assert!(report.target_committed);assert_eq!(fs::read(local.join("a.txt")).unwrap(),b"replacement");assert_eq!(fs::read(local.join("saved-original.txt")).unwrap(),b"new-a");assert!(!local.join("b.txt").exists());
    }
    #[test]
    fn archive_move_ea_is_retained_and_not_hidden_by_transaction_reader(){
        use std::os::windows::{fs::OpenOptionsExt,io::AsRawHandle};
        #[repr(C)]struct Io{status:usize,information:usize}
        #[link(name="ntdll")]unsafe extern "system"{fn NtSetEaFile(handle:*mut std::ffi::c_void,io:*mut Io,buffer:*const u8,length:u32)->i32;}
        let(_t,archive,local)=fixture("zip",false);let file=File::options().access_mode(0x10).share_mode(7).open(local.join("a.txt")).unwrap();
        let ea=[0u8,0,0,0,0,1,2,0,b'A',0,b'x',b'y'];let mut io=Io{status:0,information:0};assert!(unsafe{NtSetEaFile(file.as_raw_handle(),&mut io,ea.as_ptr(),ea.len()as u32)}>=0);drop(file);
        let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&EngineOptions::default()).unwrap();assert!(report.target_committed);assert!(local.join("a.txt").exists());assert!(!local.join("b.txt").exists());
        assert!(report.items.iter().any(|i|i.member.ends_with("a.txt")&&matches!(i.status,Status::CopiedRetained)&&i.reason.contains("附加数据")),"{:?}",report);
    }
    #[test]
    fn archive_move_actual_zip_and_sevenz_four_policies_and_safe_directory_cleanup(){
        for format in ["zip","7z"] {for policy in ["error","skip","rename","overwrite"]{
            let(t,archive,local)=fixture(format,true);let before=fs::read(&archive).unwrap();let opts=EngineOptions{overwrite:policy.into(),..Default::default()};
            let result=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&opts);
            if policy=="error"{assert!(result.is_err());assert_eq!(fs::read(&archive).unwrap(),before);assert!(local.join("a.txt").exists());continue;}
            let report=result.unwrap();assert!(report.target_committed);assert!(!report.cancelled);assert!(report.errors.is_empty(),"{:?}",report);
            assert!(!local.join("b.txt").exists(),"{:?}",report);assert_eq!(local.exists(),policy=="skip","{:?}",report);
            let out=t.path().join("out");extract(archive.to_str().unwrap(),out.to_str().unwrap(),&EngineOptions::default()).unwrap();assert_eq!(fs::read(out.join("old.txt")).unwrap(),b"unrelated");
            assert_eq!(fs::read(out.join("target/docs/b.txt")).unwrap(),b"new-b");
            assert_eq!(fs::read(out.join(if policy=="rename"{"target/docs/a (2).txt"}else{"target/docs/a.txt"})).unwrap(),if policy=="skip"{b"old-a"}else{b"new-a"});
        }}
    }
    #[test]
    fn archive_move_preserves_extra_streams_and_new_children(){
        for directory_ads in [false,true]{let(t,archive,local)=fixture("zip",false);
            let stream=if directory_ads{format!("{}:memo",local.display())}else{format!("{}:memo",local.join("a.txt").display())};fs::write(&stream,b"extra-data").unwrap();
            let newcomer=local.join("new-child.txt");let added=newcomer.clone();BEFORE_CLEANUP.with(|h|*h.borrow_mut()=Some(Box::new(move||fs::write(added,b"new child").unwrap())));
            let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&EngineOptions::default()).unwrap();
            assert!(report.target_committed);assert_eq!(fs::read(&stream).unwrap(),b"extra-data");assert_eq!(fs::read(newcomer).unwrap(),b"new child");assert!(!local.join("b.txt").exists());assert_eq!(local.join("a.txt").exists(),!directory_ads);assert!(local.exists());drop(t);
        }
    }
    #[test]
    fn archive_move_changed_source_and_cancel_before_source_commit_retain_data(){
        for cancel in [false,true]{let(_t,archive,local)=fixture("zip",false);let ctl=JobControl::new();let opts=EngineOptions{control:Some(ctl.clone()),..Default::default()};
            if cancel{let ctl=ctl.clone();BEFORE_SOURCE_COMMIT.with(|h|*h.borrow_mut()=Some(Box::new(move||ctl.cancel())));}else{let changed=local.join("a.txt");BEFORE_CLEANUP.with(|h|*h.borrow_mut()=Some(Box::new(move||fs::write(changed,b"newer-data").unwrap())));}
            let report=super::super::archive_move(archive.to_str().unwrap(),&[local.display().to_string()],"target",&opts).unwrap();assert!(report.target_committed);assert_eq!(report.cancelled,cancel);assert!(local.join("a.txt").exists());
            if cancel{assert_eq!(fs::read(local.join("b.txt")).unwrap(),b"new-b");assert!(report.items.iter().all(|i|!matches!(i.status,Status::Moved)));}else{assert_eq!(fs::read(local.join("a.txt")).unwrap(),b"newer-data");assert!(!local.join("b.txt").exists());}
            test(archive.to_str().unwrap(),&EngineOptions::default()).unwrap();
        }
    }
}
