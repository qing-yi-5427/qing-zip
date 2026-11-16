//! Preparatory evidence only. Nothing in this module authorizes source deletion.
use super::*;
#[derive(Debug)]
pub(in crate::engine) struct Item {
    pub source: PathBuf,
    pub source_id: (u32,u64),
    pub original_member: String,
    pub member: String,
    pub directory: bool,
    pub bytes: u64,
    pub sha256: Option<[u8;32]>,
    pub skipped: Option<String>,
}
#[derive(Default,Debug)]
pub(in crate::engine) struct Plan { pub items: Vec<Item> }
impl Plan {
    pub(super) fn capture(&mut self, source:&Path, member:&Path, handle:&File, directory:bool, bytes:u64, sha256:Option<[u8;32]>) -> Result<(),String> {
        let id=directory_identity(handle)?;
        if id.1==0 { return Err("Source has no stable identity".into()); }
        let original_member=member.to_str().ok_or("invalid member")?.replace('\\',"/");
        self.items.push(Item{original_member,source:absolute(source.to_str().ok_or("invalid source path")?)?,source_id:id,member:member.to_str().ok_or("invalid member")?.replace('\\',"/"),directory,bytes,sha256,skipped:None});
        Ok(())
    }
    pub(super) fn validate_mapping(&self) -> Result<(),String> {
        let mut destinations=HashSet::new();
        for item in self.items.iter().filter(|i|i.skipped.is_none()) {
            validate_name(&item.member)?;
            if !destinations.insert(key(&item.member)) {
                return Err(format!("Ambiguous source-to-member mapping: {}",item.member));
            }
            if !item.directory && item.sha256.is_none() {return Err("Missing source snapshot SHA".into());}
        }
        Ok(())
    }
    pub(super) fn skip(&mut self,name:&str,reason:&str) {
        let prefix=format!("{}/",key(name));
        for item in &mut self.items {if key(&item.member)==key(name)||key(&item.member).starts_with(&prefix){item.skipped=Some(reason.into());}}
    }
    pub(super) fn rename(&mut self,old:&str,new:&str) {
        let prefix=format!("{old}/");
        for item in &mut self.items {
            if item.member==old {item.member=new.into();}
            else if let Some(tail)=item.member.strip_prefix(&prefix){item.member=format!("{new}/{tail}");}
        }
    }
}

/// This pin covers the private candidate only. It is NOT a committed-target
/// proof and cannot be used as permission to remove any source object.
pub(super) struct CandidateProof {
    // Keep the complete namespace guard, including all parent pins.
    _candidate: crate::engine_stream::PinnedSource,
    pub identity: (u32,u64),
    pub verified_members: Vec<String>,
}
#[allow(dead_code)]
pub(super) fn verify_candidate(candidate:&Path, plan:&Plan, opts:&EngineOptions) -> Result<CandidateProof,String> {
    use sha2::{Digest,Sha256};
    let ctl=control(opts);ctl.check()?;plan.validate_mapping()?;
    let pin=crate::engine_stream::pin(candidate)?;
    let identity=directory_identity(&pin)?;
    let text=candidate.to_str().ok_or("invalid candidate path")?;
    let mut verify_opts=opts.clone();verify_opts.selected.clear();verify_opts.overwrite="error".into();
    let entries=list(text,&verify_opts)?;
    let inventory=inventory(entries)?;
    let wanted:Vec<_>=plan.items.iter().filter(|i|i.skipped.is_none()).collect();
    for item in &wanted {
        if inventory.get(&key(&item.member))!=Some(&item.directory) {return Err(format!("Candidate member missing or wrong type: {}",item.member));}
    }
    let directory=ShellDragDirectory::create_named("qing-zip-move-proof-")?;
    let data=directory.path().join("verified");
    // Only files being proved are materialized. The ordinary native selected
    // extraction route still validates all names and decodes/checks the entire
    // archive. Do not replace this with preview or a raw native -include call.
    verify_opts.selected=wanted.iter().filter(|i|!i.directory).map(|i|i.member.clone()).collect();
    if verify_opts.selected.is_empty() {
        // Empty selection means "all" to extract(), so directory-only plans
        // must use complete integrity testing without materializing old files.
        test(text,&verify_opts)?;
    } else {
        extract(text,data.to_str().ok_or("invalid proof path")?,&verify_opts)?;
    }
    #[cfg(test)]
    AFTER_EXTRACT.with(|hook| {if let Some(run)=hook.borrow_mut().take(){run(&data);}});
    let mut verified_members=Vec::new();
    for item in wanted {
        ctl.check()?;
        if item.directory {
            // Its type and namespace were proved by the complete archive
            // inventory under the candidate pin. No local directory metadata
            // is claimed preserved, and this never authorizes directory removal.
            verified_members.push(item.member.clone());continue;
        }
        let target=data.join(validate_name(&item.member)?);
        // Retain the full ancestor chain and obtain metadata
        // from the exact handle used below, never a pre-open pathname check.
        let mut verified_pin=crate::engine_stream::pin(&target)?;
        let meta=crate::private_stage::checked_metadata(&verified_pin)?;
        let output_identity=directory_identity(&verified_pin)?;
        {
            if !meta.is_file()||meta.len()!=item.bytes {return Err("Candidate length mismatch".into());}
            let mut hasher=Sha256::new();let mut buffer=vec![0;1024*1024];
            loop {ctl.check()?;let n=verified_pin.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break;}hasher.update(&buffer[..n]);}
            let actual:[u8;32]=hasher.finalize().into();
            if Some(actual)!=item.sha256 {return Err(format!("Candidate payload mismatch: {}",item.member));}
        }
        if directory_identity(&verified_pin)?!=output_identity {return Err("Verification output identity changed".into());}
        verified_members.push(item.member.clone());
    }
    ctl.check()?;
    if directory_identity(&pin)?!=identity{return Err("Candidate identity changed".into());}
    Ok(CandidateProof{_candidate:pin,identity,verified_members})
}

/// Fresh content verification while the caller retains a strong post-replace
/// candidate handle. Only a same-object read handle is reopened; no archive
/// pathname is opened, and the candidate is never copied to another file.
#[allow(dead_code)]
pub(in crate::engine) fn verify_locked_candidate(candidate:&File,plan:&Plan,opts:&EngineOptions)->Result<(),String> {
    use std::os::windows::io::{AsRawHandle,FromRawHandle};
    #[link(name="kernel32")]
    extern "system" {fn ReOpenFile(file:*mut std::ffi::c_void,access:u32,share:u32,flags:u32)->*mut std::ffi::c_void;}
    let ctl=control(opts);ctl.check()?;plan.validate_mapping()?;
    let identity=directory_identity(candidate)?;
    let raw=unsafe{ReOpenFile(candidate.as_raw_handle(),0x80000000,5,0x00200000)};
    if raw as isize == -1 {return Err(std::io::Error::last_os_error().to_string());}
    let reader=unsafe{File::from_raw_handle(raw)};
    if directory_identity(&reader)?!=identity {return Err("Candidate reader identity mismatch".into());}
    let mut options=opts.clone();options.selected.clear();options.overwrite="error".into();
    // Name is a decoder hint only. prepare obtains every input byte through
    // reader; the existing single-volume probe chooses ZIP or 7z by content.
    let (entries,pending)=selected_worker::full_worker::prepare_hash_verification("move-proof-candidate",&reader,&options,&ctl)?
        .ok_or("当前候选格式、密码、编码或策略不支持固定句柄验证，原包与源文件保持不变")?;
    let pending=pending.ok_or("候选成员数超过固定句柄验证上限，原包与源文件保持不变")?;
    let occupied=inventory(entries.clone())?;
    let sizes:BTreeMap<_,_>=entries.iter().filter(|e|!e.is_dir).map(|e|(key(&e.name),e.size)).collect();
    let wanted:Vec<_>=plan.items.iter().filter(|i|i.skipped.is_none()).collect();
    for item in &wanted {
        if occupied.get(&key(&item.member))!=Some(&item.directory){return Err("Candidate member/type mismatch".into());}
        if !item.directory && sizes.get(&key(&item.member))!=Some(&item.bytes){return Err("Candidate length mismatch".into());}
    }
    let mut expected=BTreeMap::new();
    for item in wanted {if !item.directory {
        expected.insert(item.member.clone(),item.sha256.ok_or("Missing snapshot SHA")?);
    }}
    pending.verify_hashes(&entries,&options,&expected,&ctl)?;
    #[cfg(test)]
    AFTER_VERIFY.with(|hook|{if let Some(run)=hook.borrow_mut().take(){run();}});
    ctl.check()?;
    if directory_identity(candidate)?!=identity{return Err("Candidate identity changed during validation".into());}
    Ok(())
}

#[cfg(test)]
thread_local! {pub(crate) static AFTER_VERIFY:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}
#[cfg(test)]
thread_local! { static AFTER_EXTRACT: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
mod tests {
    use super::*;
    fn item(name:&str, directory:bool)->Item {
        Item{source:PathBuf::from(format!("C:/fixture/{name}")),source_id:(7,42),original_member:name.into(),member:name.into(),directory,bytes:if directory{0}else{3},sha256:if directory{None}else{Some([1;32])},skipped:None}
    }
    #[test]
    fn proof_rename_tracks_entire_subtree_and_original_projection() {
        let mut p=Plan{items:vec![item("docs",true),item("docs/a.txt",false),item("docs/nested/b.txt",false),item("docs-other/c.txt",false)]};
        p.rename("docs","docs (2)");p.validate_mapping().unwrap();
        assert_eq!(p.items[1].member,"docs (2)/a.txt");
        assert_eq!(p.items[2].member,"docs (2)/nested/b.txt");
        assert_eq!(p.items[2].original_member,"docs/nested/b.txt");
        assert_eq!(p.items[3].member,"docs-other/c.txt");
    }
    #[test]
    fn proof_partial_skip_does_not_mark_other_members_or_parent() {
        let mut p=Plan{items:vec![item("dest/docs",true),item("dest/docs/a.txt",false),item("dest/docs/b.txt",false)]};
        p.skip("dest/docs/a.txt","existing");p.validate_mapping().unwrap();
        assert!(p.items[0].skipped.is_none());assert_eq!(p.items[1].skipped.as_deref(),Some("existing"));assert!(p.items[2].skipped.is_none());
        // A parent with skipped children must never imply permission to remove
        // that local directory. These rows are candidate-content evidence only.
    }
    #[test]
    fn proof_duplicate_destination_is_rejected_even_for_distinct_sources() {
        let mut a=item("A.txt",false);let mut b=item("a.txt",false);
        a.source=PathBuf::from("C:/one/A.txt");b.source=PathBuf::from("C:/two/a.txt");
        assert!(Plan{items:vec![a,b]}.validate_mapping().is_err());
    }
    #[test]
    fn proof_skip_whole_directory_obeys_component_boundary() {
        let mut p=Plan{items:vec![item("docs",true),item("docs/a",false),item("docs2/a",false)]};
        p.skip("docs","type collision");assert!(p.items[0].skipped.is_some());assert!(p.items[1].skipped.is_some());assert!(p.items[2].skipped.is_none());
    }
    fn write_zip(path:&Path, files:&BTreeMap<String,Vec<u8>>) {
        let mut writer=zip::ZipWriter::new(File::create(path).unwrap());
        let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name,bytes) in files {writer.start_file(name,options).unwrap();writer.write_all(bytes).unwrap();}
        writer.finish().unwrap();
    }
    fn fixture()->(tempfile::TempDir,PathBuf,PathBuf,BTreeMap<String,Vec<u8>>) {
        let dir=tempfile::tempdir().unwrap();let archive=dir.path().join("original.zip");
        let local=dir.path().join("local");fs::create_dir_all(local.join("docs")).unwrap();
        fs::write(local.join("docs/a.txt"),b"new a").unwrap();fs::write(local.join("docs/b.txt"),b"new b").unwrap();
        let old=BTreeMap::from([("folder/docs/a.txt".into(),b"old a".to_vec()),("untouched.bin".into(),b"UNSELECTED-INTEGRITY-PAYLOAD".to_vec())]);
        write_zip(&archive,&old);(dir,archive,local,old)
    }
    fn candidate(prepared:&Prepared, old:&BTreeMap<String,Vec<u8>>, path:&Path) {
        let mut contents=old.clone();
        for name in &prepared.names {let p=prepared.cwd.join(name);if p.is_file(){contents.insert(name.clone(),fs::read(p).unwrap());}}
        write_zip(path,&contents);
    }
    #[test]
    fn proof_real_four_conflicts_nested_partial_skip_and_no_old_materialization() {
        for policy in ["error","skip","rename","overwrite"] {
            let (temp,original,local,old)=fixture();let before=fs::read(&original).unwrap();
            let options=EngineOptions{overwrite:policy.into(),..Default::default()};
            let prepared=prepare_for_move_proof(&original,&[local.join("docs").display().to_string()],"folder",&options);
            if policy=="error" {assert!(prepared.is_err());assert_eq!(fs::read(&original).unwrap(),before);continue;}
            let prepared=prepared.unwrap();let plan=prepared.proof.as_ref().unwrap();
            assert!(plan.items.iter().all(|i|i.source.exists()&&i.source_id.1!=0));
            let a=plan.items.iter().find(|i|i.original_member=="folder/docs/a.txt").unwrap();
            assert_eq!(a.skipped.is_some(),policy=="skip");
            assert_eq!(a.member,if policy=="rename"{"folder/docs/a (2).txt"}else{"folder/docs/a.txt"});
            let output=temp.path().join("candidate.zip");candidate(&prepared,&old,&output);
            let skip=policy=="skip";
            AFTER_EXTRACT.with(|hook|*hook.borrow_mut()=Some(Box::new(move|data|{
                assert!(!data.join("untouched.bin").exists());
                if skip{assert!(!data.join("folder/docs/a.txt").exists());}
                assert!(data.join("folder/docs/b.txt").is_file());
            })));
            let proof=verify_candidate(&output,plan,&options).unwrap();
            assert!(proof.identity.1!=0);assert!(proof.verified_members.iter().any(|n|n=="folder/docs/b.txt"));
            assert_eq!(fs::read(&original).unwrap(),before);assert_eq!(fs::read(local.join("docs/a.txt")).unwrap(),b"new a");
        }
    }
    #[test]
    fn proof_real_unselected_corruption_sha_mismatch_and_cancel_reject() {
        for failure in ["crc","sha","cancel"] {
            let (temp,original,local,old)=fixture();let before=fs::read(&original).unwrap();
            let ctl=JobControl::new();let options=EngineOptions{overwrite:"overwrite".into(),control:Some(ctl.clone()),..Default::default()};
            let mut prepared=prepare_for_move_proof(&original,&[local.join("docs").display().to_string()],"folder",&options).unwrap();
            let output=temp.path().join("candidate.zip");candidate(&prepared,&old,&output);
            if failure=="crc" {
                let mut bytes=fs::read(&output).unwrap();let marker=b"UNSELECTED-INTEGRITY-PAYLOAD";
                let offset=bytes.windows(marker.len()).position(|w|w==marker).unwrap();bytes[offset]^=1;fs::write(&output,bytes).unwrap();
            } else if failure=="sha" {prepared.proof.as_mut().unwrap().items.iter_mut().find(|i|!i.directory).unwrap().sha256=Some([0;32]);}
            else {AFTER_EXTRACT.with(|hook|*hook.borrow_mut()=Some(Box::new(move|_|ctl.cancel())));}
            assert!(verify_candidate(&output,prepared.proof.as_ref().unwrap(),&options).is_err(),"{failure}");
            assert_eq!(fs::read(&original).unwrap(),before);assert!(local.join("docs/a.txt").exists());
        }
    }
    #[test]
    fn proof_real_duplicate_root_names_are_rejected_without_overwriting_sources() {
        let (temp,original,local,_)=fixture();let other=temp.path().join("other");fs::create_dir_all(other.join("docs")).unwrap();
        fs::write(other.join("docs/a.txt"),b"other source").unwrap();
        assert!(prepare_for_move_proof(&original,&[local.join("docs").display().to_string(),other.join("docs").display().to_string()],"folder",&EngineOptions{overwrite:"overwrite".into(),..Default::default()}).is_err());
        assert_eq!(fs::read(local.join("docs/a.txt")).unwrap(),b"new a");assert_eq!(fs::read(other.join("docs/a.txt")).unwrap(),b"other source");
    }

    #[test]
    fn proof_real_directory_only_does_not_materialize_old_files() {
        let (temp,original,local,old)=fixture();fs::create_dir(local.join("empty")).unwrap();
        let prepared=prepare_for_move_proof(&original,&[local.join("empty").display().to_string()],"nested/target",&EngineOptions::default()).unwrap();
        let output=temp.path().join("candidate.zip");
        let mut writer=zip::ZipWriter::new(File::create(&output).unwrap());let options=zip::write::SimpleFileOptions::default();
        for (name,bytes) in old {writer.start_file(name,options).unwrap();writer.write_all(&bytes).unwrap();}
        writer.add_directory("nested/target/empty/",options).unwrap();writer.finish().unwrap();
        AFTER_EXTRACT.with(|hook|*hook.borrow_mut()=Some(Box::new(|data|assert!(!data.exists()))));
        let result=verify_candidate(&output,prepared.proof.as_ref().unwrap(),&EngineOptions::default()).unwrap();
        assert_eq!(result.verified_members,vec!["nested/target/empty"]);
        drop(result);
        verify_locked_candidate(&strong(&output),prepared.proof.as_ref().unwrap(),&EngineOptions::default()).unwrap();
    }
    #[test]
    fn locked_verification_official_codec_matrix(){
        use sha2::{Digest,Sha256};
        let engine=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
        for (format,methods) in [("zip",vec!["Copy","Deflate","Deflate64","BZip2","LZMA","PPMd","ZSTD","XZ"]),("7z",vec!["Copy","LZMA","LZMA2","PPMd","BZip2","Deflate","Deflate64","ZSTD","BCJ","BCJ2","Delta"])]{
            for method in methods {
                let temp=tempfile::tempdir().unwrap();let archive=temp.path().join(format!("case.{format}"));let payload=b"codec payload \xe8\0\0\0\0".repeat(8192);fs::write(temp.path().join("a.txt"),&payload).unwrap();
                let mut args=vec!["a".to_string(),format!("-t{format}"),"-mmt=2".into()];
                if format=="zip"{args.push(format!("-mm={method}"));}else{args.push(format!("-m0={method}"));if ["BCJ","BCJ2","Delta"].contains(&method){args.push("-m1=LZMA2".into());}}
                args.extend([archive.display().to_string(),"a.txt".into()]);let output=std::process::Command::new(&engine).current_dir(temp.path()).args(args).output().unwrap();assert!(output.status.success(),"{format}/{method}: {}",String::from_utf8_lossy(&output.stderr));
                let mut expected=item("a.txt",false);expected.bytes=payload.len() as u64;expected.sha256=Some(Sha256::digest(&payload).into());
                verify_locked_candidate(&strong(&archive),&Plan{items:vec![expected]},&EngineOptions{threads:Some(2),..Default::default()}).unwrap_or_else(|e|panic!("{format}/{method}: {e}"));
                eprintln!("verified {format}/{method}");
            }
        }
    }
    #[test]
    fn expanded_codecs_actual_move_encryption_corruption_and_cancel(){
        let engine=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
        for (format,methods) in [("zip",vec!["Deflate64","BZip2","LZMA","PPMd","ZSTD","XZ"]),("7z",vec!["PPMd","BZip2","Deflate","Deflate64","ZSTD","BCJ","BCJ2","Delta"])]{
            for method in methods {for encrypted in [false,true]{
                let temp=tempfile::tempdir().unwrap();let archive=temp.path().join(format!("case.{format}"));let payload=b"old payload \xe8\0\0\0\0".repeat(8192);fs::write(temp.path().join("old.txt"),&payload).unwrap();
                let mut args=vec!["a".to_string(),format!("-t{format}"),"-mmt=2".into()];
                if format=="zip"{args.push(format!("-mm={method}"));}else{args.push(format!("-m0={method}"));if ["BCJ","BCJ2","Delta"].contains(&method){args.push("-m1=LZMA2".into());}}
                if encrypted{args.push("-pcodec-fixture".into());args.push(if format=="zip"{"-mem=AES256".into()}else{"-mhe=on".into()});}
                args.extend([archive.display().to_string(),"old.txt".into()]);let output=std::process::Command::new(&engine).current_dir(temp.path()).args(args).output().unwrap();assert!(output.status.success(),"{format}/{method}/{encrypted}: {}",String::from_utf8_lossy(&output.stderr));
                let source=temp.path().join("new.txt");fs::write(&source,b"new payload").unwrap();let before=fs::read(&archive).unwrap();
                let ctl=JobControl::new();let opts=EngineOptions{threads:Some(2),password:encrypted.then(||"codec-fixture".into()),control:Some(ctl.clone()),..Default::default()};
                // Fresh post-replace verification cancellation must restore the
                // original bytes, even when the old entry uses a different codec.
                let cancel=ctl.clone();AFTER_VERIFY.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())));
                assert!(crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&opts).is_err());assert_eq!(fs::read(&archive).unwrap(),before);assert!(source.exists());
                let opts=EngineOptions{control:None,..opts};
                if encrypted{let wrong=EngineOptions{password:Some("wrong-fixture".into()),..opts.clone()};assert!(crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&wrong).is_err());assert_eq!(fs::read(&archive).unwrap(),before);assert!(source.exists());}
                let result=crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&opts).unwrap_or_else(|e|panic!("{format}/{method}/{encrypted}: {e}"));assert!(result.target_committed&&!result.partial,"{result:?}");assert!(!source.exists());
                let dest=temp.path().join("extract");extract(archive.to_str().unwrap(),dest.to_str().unwrap(),&opts).unwrap();assert_eq!(fs::read(dest.join("old.txt")).unwrap(),payload);assert_eq!(fs::read(dest.join("folder/new.txt")).unwrap(),b"new payload");
                // Damage actual compressed/encrypted data, leaving the header
                // inventory readable whenever the native format permits it.
                let mut corrupt=before;if format=="zip"{let start=30+u16::from_le_bytes(corrupt[26..28].try_into().unwrap()) as usize+u16::from_le_bytes(corrupt[28..30].try_into().unwrap()) as usize;let packed=u32::from_le_bytes(corrupt[18..22].try_into().unwrap()) as usize;assert!(packed>10);corrupt[start+packed/2]^=0x55;}else{corrupt[40]^=0x55;}
                fs::write(&archive,&corrupt).unwrap();assert!(test(archive.to_str().unwrap(),&opts).is_err(),"fixture corruption must be independently rejected {format}/{method}/{encrypted}");fs::write(&source,b"new payload").unwrap();assert!(crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&opts).is_err(),"corrupt {format}/{method}/{encrypted}");assert_eq!(fs::read(&archive).unwrap(),corrupt);assert!(source.exists());
                eprintln!("real Move {format}/{method}/encrypted={encrypted}: cancel, success, CRC rejection");
            }}
        }
    }
    #[test]
    fn architecture_filters_actual_move_known_password_and_failures(){
        let engine=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
        for (format,methods) in [("7z",vec!["ARM","ARMT","ARM64","IA64","PPC","SPARC","RISCV"])]{
            for method in methods {for encrypted in [true]{
                let temp=tempfile::tempdir().unwrap();let archive=temp.path().join(format!("case.{format}"));let payload=b"old payload \xe8\0\0\0\0".repeat(8192);fs::write(temp.path().join("old.txt"),&payload).unwrap();
                let mut args=vec!["a".to_string(),format!("-t{format}"),"-mmt=2".into()];
                if format=="zip"{args.push(format!("-mm={method}"));}else{args.push(format!("-m0={method}"));if ["ARM","ARMT","ARM64","IA64","PPC","SPARC","RISCV"].contains(&method){args.push("-m1=LZMA2".into());}}
                if encrypted{args.push("-pcodec-fixture".into());args.push(if format=="zip"{"-mem=AES256".into()}else{"-mhe=on".into()});}
                args.extend([archive.display().to_string(),"old.txt".into()]);let output=std::process::Command::new(&engine).current_dir(temp.path()).args(args).output().unwrap();assert!(output.status.success(),"{format}/{method}/{encrypted}: {}",String::from_utf8_lossy(&output.stderr));
                let source=temp.path().join("new.txt");fs::write(&source,b"new payload").unwrap();let before=fs::read(&archive).unwrap();
                let ctl=JobControl::new();let opts=EngineOptions{threads:Some(2),password:encrypted.then(||"codec-fixture".into()),control:Some(ctl.clone()),..Default::default()};
                // Fresh post-replace verification cancellation must restore the
                // original bytes, even when the old entry uses a different codec.
                let cancel=ctl.clone();AFTER_VERIFY.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())));
                assert!(crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&opts).is_err());assert_eq!(fs::read(&archive).unwrap(),before);assert!(source.exists());
                let opts=EngineOptions{control:None,..opts};
                if encrypted{let wrong=EngineOptions{password:Some("wrong-fixture".into()),..opts.clone()};assert!(crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&wrong).is_err());assert_eq!(fs::read(&archive).unwrap(),before);assert!(source.exists());}
                let result=crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&opts).unwrap_or_else(|e|panic!("{format}/{method}/{encrypted}: {e}"));assert!(result.target_committed&&!result.partial,"{result:?}");assert!(!source.exists());
                let dest=temp.path().join("extract");extract(archive.to_str().unwrap(),dest.to_str().unwrap(),&opts).unwrap();assert_eq!(fs::read(dest.join("old.txt")).unwrap(),payload);assert_eq!(fs::read(dest.join("folder/new.txt")).unwrap(),b"new payload");
                // Damage actual compressed/encrypted data, leaving the header
                // inventory readable whenever the native format permits it.
                let mut corrupt=before;if format=="zip"{let start=30+u16::from_le_bytes(corrupt[26..28].try_into().unwrap()) as usize+u16::from_le_bytes(corrupt[28..30].try_into().unwrap()) as usize;let packed=u32::from_le_bytes(corrupt[18..22].try_into().unwrap()) as usize;assert!(packed>10);corrupt[start+packed/2]^=0x55;}else{corrupt[40]^=0x55;}
                fs::write(&archive,&corrupt).unwrap();assert!(test(archive.to_str().unwrap(),&opts).is_err(),"fixture corruption must be independently rejected {format}/{method}/{encrypted}");fs::write(&source,b"new payload").unwrap();assert!(crate::engine::archive_move(archive.to_str().unwrap(),&[source.display().to_string()],"folder",&opts).is_err(),"corrupt {format}/{method}/{encrypted}");assert_eq!(fs::read(&archive).unwrap(),corrupt);assert!(source.exists());
                eprintln!("real Move {format}/{method}/encrypted={encrypted}: cancel, success, CRC rejection");
            }}
        }
    }
    #[test]
    fn locked_merge_only_directory_verifies_all_content_without_outputs() {
        for format in ["zip","7z"] {
            let temp=tempfile::tempdir().unwrap();let seed=temp.path().join("docs");fs::create_dir(&seed).unwrap();fs::write(seed.join("old.txt"),b"old content").unwrap();
            let archive=temp.path().join(format!("candidate.{format}"));
            create(archive.to_str().unwrap(),&[seed.display().to_string()],format,&EngineOptions::default()).unwrap();
            let plan=Plan{items:vec![item("docs",true)]};
            verify_locked_candidate(&strong(&archive),&plan,&EngineOptions::default()).unwrap();
            assert_eq!(fs::read(seed.join("old.txt")).unwrap(),b"old content");
        }
    }
    #[test]
    fn proof_real_sevenz_candidate_checks_selected_payload_and_full_archive() {
        let (temp,_,local,_)=fixture();let seed=temp.path().join("seed");fs::create_dir(&seed).unwrap();fs::write(seed.join("untouched.txt"),b"old sevenz").unwrap();
        let original=temp.path().join("original.7z");create(original.to_str().unwrap(),&[seed.display().to_string()],"7z",&EngineOptions::default()).unwrap();
        let inputs=vec![local.join("docs").display().to_string()];
        let prepared=prepare_for_move_proof(&original,&inputs,"folder",&EngineOptions::default()).unwrap();
        let output=temp.path().join("candidate.7z");fs::copy(&original,&output).unwrap();
        archive_add(output.to_str().unwrap(),&inputs,"folder",&EngineOptions::default()).unwrap();
        AFTER_EXTRACT.with(|hook|*hook.borrow_mut()=Some(Box::new(|data|{
            assert!(!data.join("seed/untouched.txt").exists());assert!(data.join("folder/docs/a.txt").is_file());
        })));
        verify_candidate(&output,prepared.proof.as_ref().unwrap(),&EngineOptions::default()).unwrap();
        assert!(local.join("docs/a.txt").exists());
    }

    fn strong(path:&Path)->File {
        use std::os::windows::fs::OpenOptionsExt;
        File::options().access_mode(0x80010000).share_mode(1).custom_flags(0x00200000).open(path).unwrap()
    }
    #[test]
    fn proof_locked_real_zip_and_sevenz_updates_without_candidate_copy() {
        for format in ["zip","7z"] { for policy in ["error","skip","rename","overwrite"] {
            let (temp,zip,local,_)=fixture();let original=if format=="zip"{zip}else{
                let seed=temp.path().join("seed");fs::create_dir_all(seed.join("folder/docs")).unwrap();fs::write(seed.join("folder/docs/a.txt"),b"old a").unwrap();
                let archive=temp.path().join("original.7z");create(archive.to_str().unwrap(),&[seed.join("folder").display().to_string()],"7z",&EngineOptions::default()).unwrap();archive
            };
            let before=fs::read(&original).unwrap();let inputs=vec![local.join("docs").display().to_string()];
            let opts=EngineOptions{overwrite:policy.into(),threads:Some(4),..Default::default()};
            let plan=prepare_for_move_proof(&original,&inputs,"folder",&opts);
            if policy=="error"{assert!(plan.is_err());assert_eq!(fs::read(&original).unwrap(),before);continue;}
            let plan=plan.unwrap();let candidate=temp.path().join(format!("candidate.{format}"));fs::copy(&original,&candidate).unwrap();
            // Exercise real native update + verification + publication, not
            // the synthetic ZipWriter candidate constructor used above.
            archive_add(candidate.to_str().unwrap(),&inputs,"folder",&opts).unwrap();
            let pinned=strong(&candidate);
            assert!(crate::engine_stream::pin(&candidate).is_err(),"normal path pin must conflict with the strong DELETE handle");
            verify_locked_candidate(&pinned,plan.proof.as_ref().unwrap(),&opts).unwrap();
            assert!(File::options().write(true).open(&candidate).is_err());
            assert_eq!(fs::read(&original).unwrap(),before);assert!(local.join("docs/a.txt").exists());
        }}
    }
    #[test]
    fn proof_locked_rejects_unselected_damage() {
        for mode in ["crc","sha","cancel"] {
            let (temp,original,local,old)=fixture();let ctl=JobControl::new();let mut opts=EngineOptions{overwrite:"overwrite".into(),control:Some(ctl.clone()),..Default::default()};
            let mut plan=prepare_for_move_proof(&original,&[local.join("docs").display().to_string()],"folder",&opts).unwrap();
            let output=temp.path().join("candidate.zip");candidate(&plan,&old,&output);
            if mode=="crc"{let mut bytes=fs::read(&output).unwrap();let marker=b"UNSELECTED-INTEGRITY-PAYLOAD";let n=bytes.windows(marker.len()).position(|w|w==marker).unwrap();bytes[n]^=1;fs::write(&output,bytes).unwrap();}
            if mode=="sha"{plan.proof.as_mut().unwrap().items.iter_mut().find(|i|!i.directory).unwrap().sha256=Some([0;32]);}
            if mode=="password"{opts.password=Some("not inherited".into());}
            if mode=="cancel"{AFTER_VERIFY.with(|h|*h.borrow_mut()=Some(Box::new(move||ctl.cancel())));}
            let pin=strong(&output);assert!(verify_locked_candidate(&pin,plan.proof.as_ref().unwrap(),&opts).is_err(),"{mode}");
            assert!(local.join("docs/a.txt").exists());
        }
    }

    #[test]
    fn proof_guarded_real_updates_fresh_private_callback_commit_and_rollback() {
        for format in ["zip","7z"] {for policy in ["error","skip","rename","overwrite"] {for reject in [false,true] {
            let (temp,zip,local,_)=fixture();let original=if format=="zip"{zip}else{
                let seed=temp.path().join("seed");fs::create_dir_all(seed.join("folder/docs")).unwrap();fs::write(seed.join("folder/docs/a.txt"),b"old a").unwrap();
                let a=temp.path().join("original.7z");create(a.to_str().unwrap(),&[seed.join("folder").display().to_string()],"7z",&EngineOptions::default()).unwrap();a
            };
            let before=fs::read(&original).unwrap();let inputs=vec![local.join("docs").display().to_string()];
            let opts=EngineOptions{overwrite:policy.into(),..Default::default()};
            let lock=update_lock(&original,&control(&opts)).unwrap();
            let prepared=prepare_for_move_proof(&original,&inputs,"folder",&opts);
            if policy=="error"{assert!(prepared.is_err());assert_eq!(fs::read(&original).unwrap(),before);continue;}
            let prepared=prepared.unwrap();
            let source=edit_source::EditSource::open_controlled(&original,&control(&opts)).unwrap();
            let output=source.output().unwrap();fs::copy(&original,output.path()).unwrap();
            // This invokes the production update transaction on the private
            // candidate. The outer original remains pinned and unmodified.
            archive_add(output.path().to_str().unwrap(),&inputs,"folder",&opts).unwrap();
            let binding=edit_source::CandidateIdentityBinding::from_pinned(crate::engine_stream::pin(output.path()).unwrap()).unwrap();
            let called=std::cell::Cell::new(false);
            let result=source.commit_tracked_guarded(output,binding,&opts,|exact|{
                called.set(true);verify_locked_candidate(exact,prepared.proof.as_ref().unwrap(),&opts)?;
                if reject {Err("injected proof failure after fresh verification".into())}else{Ok(())}
            });
            assert!(called.get());
            if reject{assert!(result.is_err());assert_eq!(fs::read(&original).unwrap(),before);}
            else {
                let guard=result.unwrap();assert!(File::options().write(true).open(&original).is_err());drop(guard);drop(lock);
                let dest=temp.path().join("verify-final");extract(original.to_str().unwrap(),dest.to_str().unwrap(),&EngineOptions::default()).unwrap();
                assert_eq!(fs::read(dest.join("folder/docs/b.txt")).unwrap(),b"new b");
                let a=if policy=="rename"{"folder/docs/a (2).txt"}else{"folder/docs/a.txt"};
                assert_eq!(fs::read(dest.join(a)).unwrap(),if policy=="skip"{b"old a".to_vec()}else{b"new a".to_vec()});
            }
            assert_eq!(fs::read(local.join("docs/a.txt")).unwrap(),b"new a");
            assert!(local.join("docs/b.txt").exists());
        }}}
    }

    #[test]
    fn proof_guarded_encrypted_large_update_streams_sha_before_publication() {
        for format in ["zip","7z"] {
            let (temp,_,local,_)=fixture();
            for i in 0..520{fs::write(local.join("docs").join(format!("new-{i}.txt")),format!("payload-{i}")).unwrap();}
            let original=temp.path().join(format!("encrypted.{format}"));let seed=temp.path().join("old.txt");fs::write(&seed,b"old archive member").unwrap();
            let opts=EngineOptions{password:Some("fixture-password".into()),header_encryption:(format=="7z").then_some(true),overwrite:"overwrite".into(),threads:Some(4),..Default::default()};
            create(original.to_str().unwrap(),&[seed.display().to_string()],format,&opts).unwrap();
            let prepared=prepare_for_move_proof(&original,&[local.join("docs").display().to_string()],"nested",&opts).unwrap();
            let source=edit_source::EditSource::open_controlled(&original,&control(&opts)).unwrap();let output=source.output().unwrap();fs::copy(&original,output.path()).unwrap();
            archive_add(output.path().to_str().unwrap(),&[local.join("docs").display().to_string()],"nested",&opts).unwrap();
            let binding=edit_source::CandidateIdentityBinding::from_pinned(crate::engine_stream::pin(output.path()).unwrap()).unwrap();
            let guard=source.commit_tracked_guarded(output,binding,&opts,|exact|verify_locked_candidate(exact,prepared.proof.as_ref().unwrap(),&opts)).unwrap();
            assert!(File::options().write(true).open(&original).is_err());drop(guard);
            let dest=temp.path().join("check");extract(original.to_str().unwrap(),dest.to_str().unwrap(),&opts).unwrap();
            for i in 0..520{assert_eq!(fs::read(dest.join("nested/docs").join(format!("new-{i}.txt"))).unwrap(),format!("payload-{i}").as_bytes());assert!(local.join("docs").join(format!("new-{i}.txt")).exists());}
            assert_eq!(fs::read(dest.join("old.txt")).unwrap(),b"old archive member");
        }
    }
    #[test]
    fn proof_guarded_wrong_password_restores_original_archive() {
        let (temp,_,local,_)=fixture();let original=temp.path().join("encrypted.7z");
        let opts=EngineOptions{password:Some("fixture-password".into()),overwrite:"overwrite".into(),..Default::default()};
        create(original.to_str().unwrap(),&[local.join("docs").display().to_string()],"7z",&opts).unwrap();
        let before=fs::read(&original).unwrap();let prepared=prepare_for_move_proof(&original,&[local.join("docs").display().to_string()],"nested",&opts).unwrap();
        let source=edit_source::EditSource::open_controlled(&original,&control(&opts)).unwrap();let output=source.output().unwrap();fs::copy(&original,output.path()).unwrap();
        archive_add(output.path().to_str().unwrap(),&[local.join("docs").display().to_string()],"nested",&opts).unwrap();
        let binding=edit_source::CandidateIdentityBinding::from_pinned(crate::engine_stream::pin(output.path()).unwrap()).unwrap();
        let wrong=EngineOptions{password:Some("wrong-password".into()),..opts.clone()};
        let result=source.commit_tracked_guarded(output,binding,&opts,|exact|verify_locked_candidate(exact,prepared.proof.as_ref().unwrap(),&wrong));
        assert!(result.is_err());assert_eq!(fs::read(&original).unwrap(),before);assert!(local.join("docs/a.txt").exists());
    }

}
