//! One isolated archive Open, complete host manifest approval, then private decode.
use super::*;

pub(in crate::engine) struct Pending {
    // Drop/reap the worker before releasing the pinned runtime and worker image.
    session: Session,
    _runtime: Vec<crate::engine_stream::PinnedSource>,
    _image: crate::engine_stream::PinnedSource,
    nonce: [u8;16],
    manifest: (u8,Vec<u8>),
    verification: bool,
    hashing: bool,
}

#[cfg(test)]
thread_local! {
    static BEFORE_RUNTIME: std::cell::RefCell<Option<Box<dyn FnOnce(&Session)->Option<PathBuf>>>> =
        std::cell::RefCell::new(None);
}

fn manifest_entries(body:&[u8],nonce:&[u8;16])->Result<Vec<Entry>,String> {
    manifest_entries_mode(body,nonce,false)
}
fn manifest_entries_mode(body:&[u8],nonce:&[u8;16],verification:bool)->Result<Vec<Entry>,String> {
    let mut data=Fields(body);
    if data.take(16)? != nonce {return Err("full worker nonce mismatch".into());}
    let count=data.u32()? as usize;
    if (!verification && count==0) || count>MAX_ITEMS {return Err("full worker manifest count limit".into());}
    let mut entries=Vec::with_capacity(count);let mut names=HashSet::new();let mut units=0usize;let mut total=0u64;
    for _ in 0..count {
        let name=data.wide()?.replace('\\',"/");
        units=units.checked_add(name.encode_utf16().count()).filter(|n|*n<=MAX_NAME_UNITS).ok_or("full worker name budget")?;
        let key=path_key(&validate_name(&name)?);
        if !names.insert(key) {return Err("duplicate full worker member".into());}
        let size=data.u64()?;total=total.checked_add(size).ok_or("full worker size overflow")?;
        let is_dir=data.bit()?;let present=data.bit()?;let attrs=data.u32()?;
        let crc_present=data.bit()?;let crc=data.u32()?;
        let encrypted=if verification{data.bit()?}else{false};
        let times=[data.time()?,data.time()?,data.time()?];let kind=(attrs>>16)&0xf000;
        if (is_dir && size!=0) || attrs&0x400!=0 || !matches!(kind,0|0x4000|0x8000) ||
            (kind!=0 && (kind==0x4000)!=is_dir) {return Err("unsafe full worker member metadata".into());}
        let modified=match times[2] {
            Some(ticks)=> {let time=system_time(ticks).ok_or("invalid full worker timestamp")?;
                chrono::DateTime::<chrono::Utc>::from(time).format("%Y-%m-%d %H:%M:%S%.9fZ").to_string()},
            None=>String::new(),
        };
        // Extraction-only manifest: publishing consumes mtime and readonly;
        // memory qualification consumes name/type/size/CRC. Browse-only method,
        // compressed size and comments are intentionally not synthesized.
        // Safety properties were independently rejected by the native manifest
        // and above; unsupported layouts return to the original complete listing.
        entries.push(Entry{name,size,size_known:true,packed_size:0,is_dir,
            crc:if crc_present {format!("{crc:08X}")}else{String::new()},encrypted,modified,
            method:String::new(),comment:String::new(),attributes:format!("{}{}",if is_dir{"D"}else{"A"},if present && attrs&1!=0{"R"}else{""})});
    }
    data.end()?;expected_tree(&entries)?;
    Ok(entries)
}

fn acknowledge_threads(body:&[u8],nonce:&[u8;16],requested:Option<u32>,already_applied:bool)->Result<(),String> {
    let mut data=Fields(body);
    if data.take(16)?!=nonce {return Err("worker thread acknowledgement identity mismatch".into());}
    let applied=data.u32()?;data.end()?;
    if already_applied || applied==0 || requested!=Some(applied) {return Err("worker thread limit mismatch".into());}
    Ok(())
}

pub(in crate::engine) fn prepare(path:&str,source:&File,opts:&EngineOptions,ctl:&JobControl)
    ->Result<Option<(Vec<Entry>,Option<Pending>)>,String> {
    prepare_impl(path,source,opts,ctl,false,false)
}
// Verification grants capabilities only after the complete manifest is checked.
// Unlike full extraction, its handle budget depends on requested outputs, not
// total entries. Password, policy, format and inventory limits remain unchanged.
pub(in crate::engine) fn prepare_verification(path:&str,source:&File,opts:&EngineOptions,ctl:&JobControl)
    ->Result<Option<(Vec<Entry>,Option<Pending>)>,String> {
    prepare_impl(path,source,opts,ctl,true,false)
}
pub(in crate::engine) fn prepare_hash_verification(path:&str,source:&File,opts:&EngineOptions,ctl:&JobControl)
    ->Result<Option<(Vec<Entry>,Option<Pending>)>,String> {prepare_impl(path,source,opts,ctl,true,true)}
fn prepare_impl(path:&str,source:&File,opts:&EngineOptions,ctl:&JobControl,verification:bool,hashing:bool)
    ->Result<Option<(Vec<Entry>,Option<Pending>)>,String> {
    if opts.threads==Some(0) {return Err("invalid compression settings".into());}
    if !opts.selected.is_empty() || (!verification && (opts.password.is_some() || opts.codepage.is_some())) ||
        crate::machine_policy::scope().restricted() {return Ok(None);}
    if verification {if let Some(password)=opts.password.as_deref(){crate::engine_password::validate(password)?;if password.encode_utf16().count()>MAX_PASSWORD_UNITS{return Err("password input length exceeds limit".into());}}}
    let mut probe=source.try_clone().map_err(|e|e.to_string())?;
    let mut magic=[0u8;4];probe.rewind().map_err(|e|e.to_string())?;
    if probe.read_exact(&mut magic).is_err(){return Ok(None);}
    let mode:u32=if magic==*b"PK\x03\x04" || magic==*b"PK\x05\x06" {
        // Exclusion only: this EOCD count grants no payload or output access.
        // Ineligible jobs retain the original complete listing and CRC checks;
        // avoid rewalking every central/local header solely to decline them.
        if !verification && zip_volume_view::single_disk_record(&mut probe)?
            .is_some_and(|record| !record.zip64 && usize::from(record.entries)>MAX_OUTPUT_HANDLES) {
            return Ok(None);
        }
        if !(verification && zip_input_probe::canonical_empty(&mut probe,ctl)?) {
        let Some(layout)=zip_input_probe::list_layout(&mut probe,ctl)? else{return Ok(None)};
        // This is a decline-only hint, never authority to publish. Counts include
        // directories; conservative declines retain the complete original listing
        // and CRC-verified decoder. Do not spawn a child we cannot grant outputs to.
        if layout.first_local!=0 || layout.count==0 || layout.count>MAX_ITEMS as u64 || (!verification && layout.count>MAX_OUTPUT_HANDLES as u64) {return Ok(None);}
        }
        2
    } else if zip_input_probe::signature(&mut probe)?==Some("7z") {1}else{return Ok(None)};
    // Existing positioned ZIP listing handles SFX/extension aliases; preserve it.
    if mode==2 && zip_volume_view::positioned_zip_hint(&absolute(path)?) {return Ok(None);}
    probe.rewind().map_err(|e|e.to_string())?;
    let Some(runtime)=runtime_dir()? else{return Ok(None)};
    let exe=std::env::current_exe().map_err(|e|e.to_string())?.with_file_name("obz.exe");
    #[cfg(test)]
    let exe=if exe.is_file(){exe}else{exe.parent().unwrap().parent().unwrap().join("obz.exe")};
    let helper=exe.with_file_name("obz-worker.exe");
    let exe=if helper.is_file(){helper}else{exe};
    if !exe.is_file(){return Ok(None);}
    let image=crate::engine_stream::pin(&exe)?;
    let mut nonce=[0u8;16];if unsafe{BCryptGenRandom(std::ptr::null_mut(),nonce.as_mut_ptr(),16,2)}<0{return Err("full worker nonce failed".into());}
    // The isolated child blocks on its first request. Pin its image before
    // launch, then overlap loader startup with full runtime identity checks.
    // No runtime path, parent/source/output handle or nonce is sent until every
    // DLL is verified. Field order reaps the child before releasing any pin on
    // failure, cancellation, fallback or success.
    let mut pending=Pending{session:Session::start_mode(&exe,3)?,_runtime:Vec::new(),_image:image,
        nonce,manifest:(0,Vec::new()),verification,hashing};
    #[cfg(test)]
    TEST_CHILDREN.with(|children|children.borrow_mut().push(pending.session.child.try_clone().unwrap()));
    #[cfg(test)]
    let runtime=BEFORE_RUNTIME.with(|slot|slot.borrow_mut().take())
        .and_then(|hook|hook(&pending.session)).unwrap_or(runtime);
    pending._runtime=pin_runtime(&runtime,ctl)?;
    ctl.check()?;
    let session=&mut pending.session;
    let mut request=nonce.to_vec();wide(&mut request,runtime.as_os_str())?;
    request.extend_from_slice(&duplicate(&session.child,unsafe{GetCurrentProcess()},0x100000)?.to_le_bytes());
    request.extend_from_slice(&1u32.to_le_bytes());request.extend_from_slice(&duplicate(&session.child,source.as_raw_handle(),0x100081)?.to_le_bytes());
    request.extend_from_slice(&source.metadata().map_err(|e|e.to_string())?.len().to_le_bytes());
    wide(&mut request,OsStr::new(if mode==1{"input.7z"}else{"input.zip"}))?;
    let units=opts.password.as_ref().map_or(0,|p|p.encode_utf16().count() as u32);
    request.extend_from_slice(&units.to_le_bytes());request.extend_from_slice(&(mode+if hashing{6}else if verification{4}else{0}).to_le_bytes());request.extend_from_slice(&opts.threads.unwrap_or(0).to_le_bytes());
    if verification{request.extend_from_slice(&u32::from(opts.codepage.is_some()).to_le_bytes());request.extend_from_slice(&opts.codepage.unwrap_or(0).to_le_bytes());}
    session.send(11,request,ctl)?;
    if let Some(password)=opts.password.as_deref(){session.send(6,SecretFrame::new(password),ctl)?;}
    let mut encrypted_header=false;
    let mut threads_applied=opts.threads.is_none();
    loop {
        let frame=session.next(ctl)?.ok_or("full worker ended before manifest")?;
        if frame.1.get(..16)!=Some(nonce.as_slice()){return Err("full worker response identity mismatch".into());}
        match frame.0 {
            2=> {
                if !threads_applied {return Err("worker did not confirm decoder thread limit".into());}
                let entries=manifest_entries_mode(&frame.1,&nonce,verification)?;
                if !verification && entries.iter().filter(|e|!e.is_dir).count()>MAX_OUTPUT_HANDLES {
                    // No output capability was granted. Session Drop reaps the waiting child.
                    return Ok(Some((entries,None)));
                }
                pending.manifest=frame;
                return Ok(Some((entries,Some(pending))));
            },
            8=> {
                acknowledge_threads(&frame.1,&nonce,opts.threads,threads_applied)?;
                threads_applied=true;
                if PhaseTimer::enabled(){eprintln!("OA_FULL_WORKER threads_applied={}",opts.threads.unwrap());}
            },
            7=> {let mut f=Fields(&frame.1);f.take(16)?;f.end()?;encrypted_header=true;},
            5=> {
                if encrypted_header && !verification {return Ok(None);}
                let mut stats=Stats{code:-1,..Default::default()};
                let used=consume_session(session,nonce,&[],opts,Path::new(""),&mut CheckedDirectories::default(),ctl,&mut stats,Some(frame),OutputPlan::Requested,verification)?;
                if used{return Err("full worker completed without manifest".into());}
                return Ok(None);
            },
            _=>return Err("unexpected full worker preapproval frame".into()),
        }
    }
}
pub(in crate::engine) enum VerificationOutput { None, Members }
impl Pending {
    pub(in crate::engine) fn verify_hashes(self,entries:&[Entry],opts:&EngineOptions,expected:&BTreeMap<String,[u8;32]>,ctl:&JobControl)->Result<(),String> {
        if !self.hashing{return Err("hash verification requires dedicated mode".into());}
        let mut sink=HashSink::new(entries,expected)?;
        self.decode_impl(entries,opts,Path::new(""),&mut CheckedDirectories::default(),ctl,OutputPlan::Hashes(&mut sink))
    }
    pub(in crate::engine) fn decode(self,entries:&[Entry],opts:&EngineOptions,root:&Path,guards:&mut CheckedDirectories,ctl:&JobControl)->Result<(),String> {
        self.decode_impl(entries,opts,root,guards,ctl,OutputPlan::Requested)
    }
    pub(in crate::engine) fn verify(self,entries:&[Entry],opts:&EngineOptions,root:&Path,guards:&mut CheckedDirectories,ctl:&JobControl,output:VerificationOutput)->Result<(),String> {
        if !self.verification{return Err("verification requires dedicated worker mode".into());}
        self.decode_impl(entries,opts,root,guards,ctl,match output{VerificationOutput::None=>OutputPlan::None,VerificationOutput::Members=>OutputPlan::Requested})
    }
    fn decode_impl(mut self,entries:&[Entry],opts:&EngineOptions,root:&Path,guards:&mut CheckedDirectories,ctl:&JobControl,output:OutputPlan)->Result<(),String> {
        let mut stats=Stats{code:-1,..Default::default()};
        let used=consume_session(&mut self.session,self.nonce,entries,opts,root,guards,ctl,&mut stats,Some(self.manifest),output,self.verification)?;
        if !used {return Err("full worker declined after approved manifest".into());}
        if PhaseTimer::enabled(){eprintln!("OA_FULL_WORKER decoded={} completed={}",stats.decoded,stats.completed);}
        Ok(())
    }
}

#[cfg(test)]
thread_local! {static TEST_CHILDREN:std::cell::RefCell<Vec<OwnedHandle>>=const{std::cell::RefCell::new(Vec::new())};}
#[cfg(test)]
mod tests {
    use super::*;
    fn manifest(names:&[&str],attrs:u32)->Vec<u8> {
        let mut frame=vec![42u8;16];frame.extend_from_slice(&(names.len() as u32).to_le_bytes());
        for name in names {wide(&mut frame,OsStr::new(name)).unwrap();frame.extend_from_slice(&3u64.to_le_bytes());
            for value in [0u32,1,attrs,1,crc32fast::hash(b"abc")] {frame.extend_from_slice(&value.to_le_bytes());}
            for _ in 0..3 {frame.extend_from_slice(&0u32.to_le_bytes());frame.extend_from_slice(&0u64.to_le_bytes());}
        } frame
    }
    #[test]
    fn manifest_rejects_traversal_aliases_links_partial_and_wrong_job() {
        let good=manifest(&["folder/a.txt"],0x20);let entries=manifest_entries(&good,&[42;16]).unwrap();
        assert_eq!(entries[0].name,"folder/a.txt");assert_eq!(entries[0].size,3);
        assert!(manifest_entries(&good,&[41;16]).is_err());
        for names in [&["../a"][..],&["A","a"][..],&["a","a/b"][..],&["a:stream"][..]] {
            assert!(manifest_entries(&manifest(names,0x20),&[42;16]).is_err(),"{names:?}");
        }
        for attrs in [0x400,0xa0000000] {assert!(manifest_entries(&manifest(&["a"],attrs),&[42;16]).is_err());}
        let mut trailing=good.clone();trailing.push(0);assert!(manifest_entries(&trailing,&[42;16]).is_err());
        for length in [0,15,19,good.len()-1] {assert!(manifest_entries(&good[..length],&[42;16]).is_err());}
    }
    fn fixture(path:&Path,empty:bool) {
        let mut zip=zip::ZipWriter::new(File::create(path).unwrap());
        if !empty {zip.start_file("a.txt",zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();zip.write_all(b"abc").unwrap();}
        zip.finish().unwrap();
    }
    fn reap_proof() {
        let children=TEST_CHILDREN.with(|v|std::mem::take(&mut *v.borrow_mut()));assert!(!children.is_empty());
        for child in children {assert_eq!(unsafe{WaitForSingleObject(child.as_raw_handle(),0)},0,"worker must be reaped before returning");}
    }
    fn assert_waiting_without_request(session:&Session) {
        std::thread::sleep(Duration::from_millis(80));
        assert!(session.exit_code().unwrap().is_none());
        #[link(name="psapi")]
        extern "system" {
            fn EnumProcessModulesEx(process:Handle, modules:*mut Handle, bytes:u32, needed:*mut u32, flags:u32)->i32;
            fn GetModuleBaseNameW(process:Handle,module:Handle,name:*mut u16,size:u32)->u32;
        }
        let mut modules=[std::ptr::null_mut();1024];let mut needed=0;
        assert_ne!(unsafe{EnumProcessModulesEx(session.child.as_raw_handle(),modules.as_mut_ptr(),
            std::mem::size_of_val(&modules) as u32,&mut needed,3)},0);
        assert!((needed as usize)<=std::mem::size_of_val(&modules));
        for module in modules.iter().take(needed as usize/std::mem::size_of::<Handle>()) {
            let mut name=[0u16;1024];
            let used=unsafe{GetModuleBaseNameW(session.child.as_raw_handle(),*module,name.as_mut_ptr(),name.len() as u32)};
            assert!(used>0);let name=String::from_utf16(&name[..used as usize]).unwrap().to_ascii_lowercase();
            assert!(!name.starts_with("nanazip.")&&!name.starts_with("k7"),"unverified custom DLL loaded: {name}");
        }
        assert!(matches!(session.receiver.as_ref().unwrap().try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)), "no request or manifest before runtime verification");
    }
    #[test]
    #[ignore="explicit 1GiB verification diagnostic; run alone"]
    fn hash_verification_one_gibibyte_uses_one_ack_thread_and_no_outputs() {
        use sha2::{Digest,Sha256};
        fn digest(path:&Path)->String {let mut file=File::open(path).unwrap();let mut hash=Sha256::new();let mut buffer=vec![0;1024*1024];loop{let n=file.read(&mut buffer).unwrap();if n==0{break;}hash.update(&buffer[..n]);}hash.finalize().iter().map(|b|format!("{b:02x}")).collect()}
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("one-gib.zip");
        let block:Vec<u8>=(0..65536).map(|n|(n%251)as u8).collect();let mut hash=Sha256::new();
        let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());zip.start_file("one-gib.bin",zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        for _ in 0..16384{zip.write_all(&block).unwrap();hash.update(&block);}zip.finish().unwrap();let expected:[u8;32]=hash.finalize().into();
        let archive_before=digest(&archive);let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions{threads:Some(4),..Default::default()};let ctl=JobControl::new();
        HASH_GRANTS.with(|v|v.borrow_mut().clear());verification_sink::CHUNKS.with(|v|v.set(0));worker_ack::WRITERS_STARTED.with(|v|v.set(0));
        let started=std::time::Instant::now();let(entries,pending)=prepare_hash_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        pending.unwrap().verify_hashes(&entries,&opts,&BTreeMap::from([("one-gib.bin".into(),expected)]),&ctl).unwrap();reap_proof();let elapsed=started.elapsed().as_secs_f64();
        let chunks=verification_sink::CHUNKS.with(|v|v.get());let writers=worker_ack::WRITERS_STARTED.with(|v|v.get());assert!(chunks>=16384);assert_eq!(writers,1);HASH_GRANTS.with(|v|assert_eq!(&*v.borrow(),&[(1,0,0)]));
        assert_eq!(digest(&archive),archive_before);assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
        eprintln!("HASH_STREAM_DIAGNOSTIC {}",serde_json::json!({"bytes":1073741824u64,"chunks":chunks,"ack_writer_threads":writers,"seconds_debug_diagnostic":elapsed,"archive_sha256_before_after":archive_before,"payload_sha256":expected.iter().map(|b|format!("{b:02x}")).collect::<String>(),"output_handles":0,"output_files":0}));
    }
    #[test]
    fn hash_verification_mid_chunk_cancel_reaps_worker_and_preserves_candidate() {
        use sha2::{Digest,Sha256};
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");let payload=vec![42u8;200000];
        let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());zip.start_file("a.bin",zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(&payload).unwrap();zip.finish().unwrap();
        let before=fs::read(&archive).unwrap();let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();let opts=EngineOptions::default();
        let (entries,pending)=prepare_hash_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        let cancel=ctl.clone();verification_sink::AFTER_CHUNK.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        let expected=BTreeMap::from([("a.bin".into(),Sha256::digest(&payload).into())]);
        assert!(pending.unwrap().verify_hashes(&entries,&opts,&expected,&ctl).unwrap_err().contains("cancel"));reap_proof();assert_eq!(fs::read(&archive).unwrap(),before);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);
    }
    #[test]
    fn hash_verification_streams_more_than_512_members_without_output_handles() {
        use sha2::{Digest,Sha256};
        for format in ["zip","7z"] {
            let temp=tempfile::tempdir().unwrap();let input=temp.path().join("inputs");fs::create_dir(&input).unwrap();
            let payload=vec![42u8;140000];let mut expected=BTreeMap::new();
            for index in 0..600 {let bytes=if index==0{payload.as_slice()}else if index==1{&[][..]}else{&b"abc"[..]};
                fs::write(input.join(format!("{index}.txt")),bytes).unwrap();expected.insert(format!("inputs/{index}.txt"),Sha256::digest(bytes).into());}
            let archive=temp.path().join(format!("case.{format}"));create(archive.to_str().unwrap(),&[input.display().to_string()],format,&EngineOptions::default()).unwrap();
            let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions{threads:Some(4),..Default::default()};let ctl=JobControl::new();
            let (entries,pending)=prepare_hash_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
            HASH_GRANTS.with(|v|v.borrow_mut().clear());
            assert_eq!(entries.iter().filter(|e|!e.is_dir).count(),600);pending.unwrap().verify_hashes(&entries,&opts,&expected,&ctl).unwrap();reap_proof();
            HASH_GRANTS.with(|v|assert_eq!(&*v.borrow(),&[(600,0,0)]));
            let (entries,pending)=prepare_hash_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();expected.insert("inputs/599.txt".into(),[0;32]);
            assert!(pending.unwrap().verify_hashes(&entries,&opts,&expected,&ctl).unwrap_err().contains("payload mismatch"));reap_proof();
            assert_eq!(fs::read_dir(temp.path()).unwrap().count(),2,"only source tree and candidate archive exist");
        }
    }
    #[test]
    fn verification_codepage_cannot_hide_unsafe_or_colliding_names_and_secret_limit() {
        let temp=tempfile::tempdir().unwrap();
        for (index,names) in [vec!["../xx.txt"],vec!["xx.txt","XX.txt"]].iter().enumerate(){
            let archive=temp.path().join(format!("bad-{index}.zip"));let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());
            for name in names{zip.start_file(*name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(b"abc").unwrap();}zip.finish().unwrap();
            let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions{codepage:Some(936),..Default::default()};
            assert!(!matches!(prepare_verification(archive.to_str().unwrap(),&source,&opts,&JobControl::new()),Ok(Some(_))));
            let children=TEST_CHILDREN.with(|v|std::mem::take(&mut *v.borrow_mut()));for child in children{assert_eq!(unsafe{WaitForSingleObject(child.as_raw_handle(),0)},0);}
        }
        let archive=temp.path().join("good.zip");fixture(&archive,false);let source=crate::engine_stream::pin(&archive).unwrap();
        for password in ["x".repeat(32768),"😀".repeat(16384)]{
            let opts=EngineOptions{password:Some(password),..Default::default()};
            assert!(prepare(archive.to_str().unwrap(),&source,&opts,&JobControl::new()).unwrap().is_none());
            let error=prepare_verification(archive.to_str().unwrap(),&source,&opts,&JobControl::new()).err().unwrap();assert_eq!(error,"password input length exceeds limit");
            assert!(TEST_CHILDREN.with(|v|v.borrow().is_empty()));
        }
        let mut extended=manifest(&["a"],0x20);extended.splice(54..54,0u32.to_le_bytes());
        assert!(!manifest_entries_mode(&extended,&[42;16],true).unwrap()[0].encrypted);
        extended[54]=2;assert!(manifest_entries_mode(&extended,&[42;16],true).is_err());
        assert!(manifest_entries_mode(&extended[..53],&[42;16],true).is_err());
    }
    #[test]
    fn verification_native_request_rejects_bad_codepage_presence_length_and_mode() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();let runtime=runtime_dir().unwrap().unwrap();let _pins=pin_runtime(&runtime,&ctl).unwrap();
        let exe=std::env::current_exe().unwrap().with_file_name("obz-worker.exe");
        let exe=if exe.is_file(){exe}else{exe.parent().unwrap().parent().unwrap().join("obz-worker.exe")};let _image=crate::engine_stream::pin(&exe).unwrap();
        for bad in 0..5 {
            let mut session=Session::start_mode(&exe,3).unwrap();TEST_CHILDREN.with(|v|v.borrow_mut().push(session.child.try_clone().unwrap()));
            let mut request=vec![42;16];wide(&mut request,runtime.as_os_str()).unwrap();request.extend_from_slice(&duplicate(&session.child,unsafe{GetCurrentProcess()},0x100000).unwrap().to_le_bytes());
            request.extend_from_slice(&1u32.to_le_bytes());request.extend_from_slice(&duplicate(&session.child,source.as_raw_handle(),0x100081).unwrap().to_le_bytes());request.extend_from_slice(&source.metadata().unwrap().len().to_le_bytes());wide(&mut request,OsStr::new("input.zip")).unwrap();
            request.extend_from_slice(&(if bad==4{32768u32}else{0}).to_le_bytes());request.extend_from_slice(&(if bad==3{99u32}else{6}).to_le_bytes());request.extend_from_slice(&0u32.to_le_bytes());
            request.extend_from_slice(&(if bad==0{2u32}else{1}).to_le_bytes());request.extend_from_slice(&936u32.to_le_bytes());
            if bad==1{request.pop();}if bad==2{request.push(0);}
            session.send(11,request,&ctl).unwrap();assert_ne!(session.wait_exit(&ctl).unwrap(),0);drop(session);reap_proof();
        }
    }
    #[test]
    fn verification_password_aes_zip_and_sevenz_header_encryption() {
        for format in ["zip","7z"] {for headers in [false,true] {for level in [0,5] {
            if format=="zip" && headers {continue;}
            let temp=tempfile::tempdir().unwrap();let archive=temp.path().join(format!("case.{format}"));fs::write(temp.path().join("a.txt"),b"secret payload").unwrap();
            let engine=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
            let mut args=vec!["a".to_string(),format!("-t{format}"),"-pfixture-password".into(),format!("-mx={level}")];
            if format=="zip"{args.push("-mem=AES256".into());}if headers{args.push("-mhe=on".into());}args.push(archive.display().to_string());args.push("a.txt".into());
            let created=std::process::Command::new(engine).current_dir(temp.path()).args(args).output().unwrap();assert!(created.status.success(),"{}",String::from_utf8_lossy(&created.stdout));
            let source=crate::engine_stream::pin(&archive).unwrap();
            for correct in [true,false] {
                let opts=EngineOptions{password:Some(if correct{"fixture-password"}else{"wrong-password"}.into()),threads:Some(4),..Default::default()};let ctl=JobControl::new();
                let result=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl);
                let stage=temp.path().join(format!("stage-{correct}"));fs::create_dir(&stage).unwrap();
                let result=result.and_then(|v|{let (entries,pending)=v.ok_or("unexpected decline")?;
                    assert!(entries.iter().any(|e|e.encrypted));pending.ok_or("no worker")?.verify(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::Members)});
                if correct{result.unwrap();assert_eq!(fs::read(stage.join("a.txt")).unwrap(),b"secret payload");}else{assert!(result.is_err());}
                reap_proof();
                if correct {
                    use sha2::{Digest,Sha256};
                    let (entries,pending)=prepare_hash_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
                    pending.unwrap().verify_hashes(&entries,&opts,&BTreeMap::from([("a.txt".into(),Sha256::digest(b"secret payload").into())]),&ctl).unwrap();reap_proof();
                }
            }
        }}}
    }
    #[test]
    fn verification_empty_zip_and_sevenz_and_explicit_zip_codepage() {
        let temp=tempfile::tempdir().unwrap();let empty=temp.path().join("empty.zip");fixture(&empty,true);
        let engine=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
        let seven=temp.path().join("empty.7z");let created=std::process::Command::new(engine).current_dir(temp.path()).args(["a","-t7z",seven.to_str().unwrap(),"missing-*" ]).output().unwrap();assert!(created.status.success());
        for archive in [&empty,&seven] {let source=crate::engine_stream::pin(archive).unwrap();let opts=EngineOptions::default();let ctl=JobControl::new();
            let (entries,pending)=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().expect("empty supported");assert!(entries.is_empty());
            pending.unwrap().verify(&entries,&opts,temp.path(),&mut CheckedDirectories::default(),&ctl,VerificationOutput::None).unwrap();reap_proof();}
        let archive=temp.path().join("cp.zip");let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());zip.start_file("xx.txt",zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(b"codepage").unwrap();zip.finish().unwrap();
        let mut bytes=fs::read(&archive).unwrap();for index in 0..bytes.len()-6{if &bytes[index..index+6]==b"xx.txt"{bytes[index]=0xd6;bytes[index+1]=0xd0;}}fs::write(&archive,bytes).unwrap();
        let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions{codepage:Some(936),threads:Some(4),..Default::default()};let ctl=JobControl::new();
        let (entries,pending)=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();assert_eq!(entries[0].name,"中.txt");
        let stage=temp.path().join("cp-out");fs::create_dir(&stage).unwrap();pending.unwrap().verify(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::Members).unwrap();assert_eq!(fs::read(stage.join("中.txt")).unwrap(),b"codepage");reap_proof();
        use sha2::{Digest,Sha256};
        let (entries,pending)=prepare_hash_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        pending.unwrap().verify_hashes(&entries,&opts,&BTreeMap::from([("中.txt".into(),Sha256::digest(b"codepage").into())]),&ctl).unwrap();reap_proof();
    }
    #[test]
    fn verification_overlap_keeps_zero_capabilities_until_runtime_is_verified() {
        for action in 0..3 {
            let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
            let broken=temp.path().join("runtime");fs::create_dir(&broken).unwrap();fs::write(broken.join("K7Base.dll"),b"wrong runtime").unwrap();
            let ctl=JobControl::new();let cancel=ctl.clone();
            BEFORE_RUNTIME.with(|slot|*slot.borrow_mut()=Some(Box::new(move |session|{
                assert_waiting_without_request(session);
                if action==1{cancel.cancel();}if action==2{Some(broken)}else{None}
            })));
            let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions::default();
            let result=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl);
            if action!=0{let error=result.err().unwrap();assert!(error.contains(if action==1{"cancel"}else{"DLL identity mismatch"}),"{error}");}
            else{let (entries,pending)=result.unwrap().unwrap();let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();
                pending.unwrap().verify(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::None).unwrap();
                assert_eq!(fs::read_dir(stage).unwrap().count(),0);}
            reap_proof();
        }
    }
    #[test]
    fn delayed_runtime_validation_sends_no_request_and_then_decodes() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        BEFORE_RUNTIME.with(|slot|*slot.borrow_mut()=Some(Box::new(|session|{
            assert_waiting_without_request(session);None
        })));
        let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();let opts=EngineOptions::default();
        let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();
        pending.unwrap().decode(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl).unwrap();
        assert_eq!(fs::read(stage.join("a.txt")).unwrap(),b"abc");reap_proof();
    }
    #[test]
    fn runtime_hash_failure_reaps_unapproved_child_without_outputs() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let broken=temp.path().join("runtime");fs::create_dir(&broken).unwrap();
        fs::write(broken.join("K7Base.dll"),b"not the bundled DLL").unwrap();
        BEFORE_RUNTIME.with(|slot|*slot.borrow_mut()=Some(Box::new(move |session|{
            assert_waiting_without_request(session);Some(broken)
        })));
        let source=crate::engine_stream::pin(&archive).unwrap();
        let error=prepare(archive.to_str().unwrap(),&source,&EngineOptions::default(),&JobControl::new()).err().unwrap();
        assert!(error.contains("DLL identity mismatch"),"{error}");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(),2);reap_proof();
    }
    #[test]
    fn runtime_validation_cancellation_reaps_unapproved_child() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let ctl=JobControl::new();let cancel=ctl.clone();
        BEFORE_RUNTIME.with(|slot|*slot.borrow_mut()=Some(Box::new(move |session|{
            assert_waiting_without_request(session);cancel.cancel();None
        })));
        let source=crate::engine_stream::pin(&archive).unwrap();
        let error=prepare(archive.to_str().unwrap(),&source,&EngineOptions::default(),&ctl).err().unwrap();
        assert!(error.contains("cancel"),"{error}");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(),1);reap_proof();
    }
    #[test]
    fn overlapped_runtime_normal_7z_decodes_all_bytes() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.7z");
        let payload=(0..10000).flat_map(|i:u32|i.to_le_bytes()).collect::<Vec<_>>();
        fs::write(temp.path().join("a.bin"),&payload).unwrap();
        let result=std::process::Command::new(executable().unwrap()).current_dir(temp.path())
            .args(["a","-t7z","case.7z","a.bin"]).output().unwrap();assert!(result.status.success());
        let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();
        let opts=EngineOptions{threads:Some(4),..Default::default()};
        let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();
        pending.unwrap().decode(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl).unwrap();
        use sha2::Digest;
        assert_eq!(sha2::Sha256::digest(fs::read(stage.join("a.bin")).unwrap()),sha2::Sha256::digest(payload));
        reap_proof();
    }

    #[test]
    fn verification_zero_outputs_and_large_inventory_enforce_actual_handle_budget() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("many.zip");
        let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());
        for i in 0..600 {zip.start_file(format!("{i}.txt"),zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();zip.write_all(b"abc").unwrap();}
        zip.finish().unwrap();let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();
        let opts=EngineOptions::default();assert!(prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().is_none());
        for mode in 0..3 {
            let (entries,pending)=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
            assert_eq!(entries.len(),600);let stage=temp.path().join(format!("stage-{mode}"));fs::create_dir(&stage).unwrap();
            let mut options=opts.clone();if mode==1{options.selected=vec!["599.txt".into()];}
            let result=pending.unwrap().verify(&entries,&options,&stage,&mut CheckedDirectories::default(),&ctl,
                if mode==0{VerificationOutput::None}else{VerificationOutput::Members});
            if mode==2 {assert!(result.unwrap_err().contains("handle budget"));assert_eq!(fs::read_dir(&stage).unwrap().count(),0);}
            else {result.unwrap();assert_eq!(fs::read_dir(&stage).unwrap().count(),mode);if mode==1{assert_eq!(fs::read(stage.join("599.txt")).unwrap(),b"abc");}}
            reap_proof();
        }
    }
    #[test]
    fn verification_large_sevenz_materializes_only_the_requested_file() {
        let temp=tempfile::tempdir().unwrap();let input=temp.path().join("inputs");fs::create_dir(&input).unwrap();
        for i in 0..513{fs::write(input.join(format!("{i}.txt")),b"abc").unwrap();}
        let archive=temp.path().join("many.7z");crate::engine::create(archive.to_str().unwrap(),&[input.display().to_string()],"7z",&EngineOptions::default()).unwrap();
        let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();let opts=EngineOptions::default();
        let (_,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();assert!(pending.is_none());reap_proof();
        let (entries,pending)=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        let selected=entries.iter().find(|e|e.name.ends_with("/512.txt")).unwrap().name.clone();
        let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();
        let options=EngineOptions{selected:vec![selected.clone()],..Default::default()};
        pending.unwrap().verify(&entries,&options,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::Members).unwrap();reap_proof();
        assert_eq!(fs::read(stage.join(selected)).unwrap(),b"abc");assert_eq!(fs::read_dir(stage.join("inputs")).unwrap().count(),1);
    }
    #[test]
    fn verification_zero_outputs_rejects_corruption_cancel_and_wrong_mode() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let opts=EngineOptions::default();let ctl=JobControl::new();let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();
        {
            let source=crate::engine_stream::pin(&archive).unwrap();
            let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
            assert!(pending.unwrap().verify(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::None).unwrap_err().contains("dedicated"));reap_proof();
            let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
            // Deliberately bypass the typed public guard to exercise native mode 1/2 rejection.
            assert!(pending.unwrap().decode_impl(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,OutputPlan::None).is_err());reap_proof();
            let (entries,pending)=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();ctl.cancel();
            assert!(pending.unwrap().verify(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::None).is_err());reap_proof();
        }
        let mut bytes=fs::read(&archive).unwrap();let offset=bytes.windows(3).position(|b|b==b"abc").unwrap();bytes[offset]^=1;fs::write(&archive,bytes).unwrap();
        let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();
        let (entries,pending)=prepare_verification(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        assert!(pending.unwrap().verify(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl,VerificationOutput::None).is_err());
        assert_eq!(fs::read_dir(&stage).unwrap().count(),0);reap_proof();
    }
    #[test]
    fn pending_manifest_grants_nothing_until_approval_and_cancel_reaps_child() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions::default();let ctl=JobControl::new();
        let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().expect("qualified single archive");
        let pending=pending.unwrap();let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();assert_eq!(fs::read_dir(&stage).unwrap().count(),0);
        assert!(File::options().write(true).open(&archive).is_err(),"same source stays deny-write pinned");
        ctl.cancel();let mut guards=CheckedDirectories::default();
        assert!(pending.decode(&entries,&opts,&stage,&mut guards,&ctl).unwrap_err().contains("cancel"));
        assert_eq!(fs::read_dir(&stage).unwrap().count(),0);reap_proof();
    }
    #[test]
    fn mismatch_before_ack_and_empty_fallback_reap_without_outputs() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let source=crate::engine_stream::pin(&archive).unwrap();let opts=EngineOptions::default();let ctl=JobControl::new();
        let (mut entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();entries[0].size+=1;
        let stage=temp.path().join("stage");fs::create_dir(&stage).unwrap();
        assert!(pending.unwrap().decode(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl).is_err());
        assert_eq!(fs::read_dir(&stage).unwrap().count(),0);reap_proof();
        let empty=temp.path().join("empty.zip");fixture(&empty,true);let empty_pin=crate::engine_stream::pin(&empty).unwrap();
        assert!(prepare(empty.to_str().unwrap(),&empty_pin,&opts,&ctl).unwrap().is_none());
        assert!(TEST_CHILDREN.with(|v|v.borrow().is_empty()),"empty ZIP uses existing cheap probe");
        let many=temp.path().join("many.zip");
        let mut zip=zip::ZipWriter::new(File::create(&many).unwrap());
        for i in 0..513 {zip.start_file(format!("{i}.txt"),zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();zip.write_all(b"abc").unwrap();}
        zip.finish().unwrap();let many_pin=crate::engine_stream::pin(&many).unwrap();
        assert!(prepare(many.to_str().unwrap(),&many_pin,&opts,&ctl).unwrap().is_none());
        assert!(TEST_CHILDREN.with(|v|v.borrow().is_empty()),"over-limit ZIP must not spawn a worker");
    }
    #[test]
    fn approved_decode_checks_crc_and_password_route_stays_original() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("good.zip");fixture(&archive,false);
        let source=crate::engine_stream::pin(&archive).unwrap();let ctl=JobControl::new();let opts=EngineOptions::default();
        let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        let stage=temp.path().join("good");fs::create_dir(&stage).unwrap();
        pending.unwrap().decode(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl).unwrap();
        assert_eq!(fs::read(stage.join("a.txt")).unwrap(),b"abc");reap_proof();
        let mut password=opts.clone();password.password=Some("secret".into());
        assert!(prepare(archive.to_str().unwrap(),&source,&password,&ctl).unwrap().is_none());
        assert!(TEST_CHILDREN.with(|v|v.borrow().is_empty()));
        let broken=temp.path().join("broken.zip");let mut bytes=fs::read(&archive).unwrap();
        let offset=bytes.windows(3).position(|v|v==b"abc").unwrap();bytes[offset]^=1;fs::write(&broken,bytes).unwrap();
        let source=crate::engine_stream::pin(&broken).unwrap();
        let (entries,pending)=prepare(broken.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
        let stage=temp.path().join("bad");fs::create_dir(&stage).unwrap();
        let error=pending.unwrap().decode(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl).unwrap_err();
        assert!(error.contains("CRC")||error.contains("extraction")||error.contains("callback"),"{error}");reap_proof();
    }

    #[test]
    fn unsupported_zip_method_reaps_before_original_fallback() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("xz.zip");
        let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());
        zip.start_file("a.txt",zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Xz)).unwrap();
        zip.write_all(b"abc").unwrap();zip.finish().unwrap();
        let source=crate::engine_stream::pin(&archive).unwrap();
        assert!(prepare(archive.to_str().unwrap(),&source,&EngineOptions::default(),&JobControl::new()).unwrap().is_none());reap_proof();
    }

    #[test]
    fn trailing_zip_declines_before_ack_and_retains_original_decode_decision() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("tail.zip");fixture(&archive,false);
        File::options().append(true).open(&archive).unwrap().write_all(b"trailing data").unwrap();
        let source=crate::engine_stream::pin(&archive).unwrap();
        assert!(prepare(archive.to_str().unwrap(),&source,&EngineOptions::default(),&JobControl::new()).unwrap().is_none());reap_proof();
    }

    #[test]
    fn dedicated_image_is_pinned_and_still_waits_for_host_approval() {
        #[link(name="kernel32")]
        extern "system" { fn QueryFullProcessImageNameW(process:std::os::windows::io::RawHandle,flags:u32,name:*mut u16,length:*mut u32)->i32; }
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let source=crate::engine_stream::pin(&archive).unwrap();
        let (_,pending)=prepare(archive.to_str().unwrap(),&source,&EngineOptions::default(),&JobControl::new()).unwrap().unwrap();
        let pending=pending.unwrap();let mut image=vec![0u16;32768];let mut length=image.len() as u32;
        assert_ne!(unsafe{QueryFullProcessImageNameW(pending.session.child.as_raw_handle(),0,image.as_mut_ptr(),&mut length)},0);
        let image=PathBuf::from(String::from_utf16(&image[..length as usize]).unwrap());
        assert_eq!(image.file_name().unwrap(),"obz-worker.exe");
        assert!(File::options().write(true).open(&image).is_err());
        assert!(pending.session.exit_code().unwrap().is_none());
        drop(pending);reap_proof();
        let result=std::process::Command::new(&image).args(["--full-archive-worker","unapproved-path"]).output().unwrap();
        assert_eq!(result.status.code(),Some(3));assert!(result.stdout.is_empty());
    }

    #[test]
    fn explicit_thread_limits_are_acknowledged_before_real_decode() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("case.zip");fixture(&archive,false);
        let source=crate::engine_stream::pin(&archive).unwrap();
        for threads in [1,4,8] {
            let opts=EngineOptions{threads:Some(threads),..Default::default()};
            let ctl=JobControl::new();
            let (entries,pending)=prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().unwrap();
            let stage=temp.path().join(format!("threads-{threads}"));fs::create_dir(&stage).unwrap();
            pending.unwrap().decode(&entries,&opts,&stage,&mut CheckedDirectories::default(),&ctl).unwrap();
            assert_eq!(fs::read(stage.join("a.txt")).unwrap(),b"abc");reap_proof();
        }
        let opts=EngineOptions{threads:Some(0),..Default::default()};
        assert!(prepare(archive.to_str().unwrap(),&source,&opts,&JobControl::new()).err().unwrap().contains("invalid compression settings"));
        assert!(TEST_CHILDREN.with(|v|v.borrow().is_empty()));
    }

    #[test]
    fn over_limit_count_is_only_a_decline_hint_and_cannot_hide_bad_crc() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("many.zip");
        let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());
        for i in 0..513 {zip.start_file(format!("{i}.txt"),zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();zip.write_all(b"abc").unwrap();}
        zip.finish().unwrap();
        let offset={let mut zip=zip::ZipArchive::new(File::open(&archive).unwrap()).unwrap();let offset=zip.by_index(512).unwrap().data_start().unwrap();offset};
        let mut bytes=fs::read(&archive).unwrap();bytes[offset as usize]^=1;fs::write(&archive,bytes).unwrap();
        let source=crate::engine_stream::pin(&archive).unwrap();
        let opts=EngineOptions::default();let ctl=JobControl::new();
        assert!(prepare(archive.to_str().unwrap(),&source,&opts,&ctl).unwrap().is_none());
        assert!(TEST_CHILDREN.with(|v|v.borrow().is_empty()));
        drop(source);
        let dest=temp.path().join("out");
        assert!(extract(archive.to_str().unwrap(),dest.to_str().unwrap(),&opts).is_err());
        assert!(!dest.exists() || fs::read_dir(dest).unwrap().next().is_none());
    }
    #[test]
    fn encrypted_header_without_password_declines_and_reaps_before_output() {
        let temp=tempfile::tempdir().unwrap();let archive=temp.path().join("encrypted.7z");
        fs::write(temp.path().join("a.txt"),b"abc").unwrap();
        let engine=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
            .join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe");
        let result=std::process::Command::new(engine).current_dir(temp.path())
            .args(["a","-t7z","-mhe=on","-pfixture-password","encrypted.7z","a.txt"])
            .output().unwrap();assert!(result.status.success());
        let source=crate::engine_stream::pin(&archive).unwrap();
        assert!(prepare(archive.to_str().unwrap(),&source,&EngineOptions::default(),&JobControl::new()).unwrap().is_none());
        reap_proof();
    }
    #[test]
    fn decoder_thread_ack_cannot_change_job_limit_or_repeat() {
        let mut frame=vec![42;16];frame.extend_from_slice(&4u32.to_le_bytes());
        acknowledge_threads(&frame,&[42;16],Some(4),false).unwrap();
        assert!(acknowledge_threads(&frame,&[41;16],Some(4),false).is_err());
        for requested in [None,Some(1),Some(8)] {assert!(acknowledge_threads(&frame,&[42;16],requested,false).is_err());}
        assert!(acknowledge_threads(&frame,&[42;16],Some(4),true).is_err());
        frame.push(0);assert!(acknowledge_threads(&frame,&[42;16],Some(4),false).is_err());
    }

}
