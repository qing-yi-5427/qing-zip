//! Create-only access-time-preserving input proof. Payload is never staged here.
//! BackupRead's ADS locks are released before independent protected reads.
use super::*;
use crate::source_access::{OpenSpec, Preservation};
use std::io::{Read, Seek, SeekFrom};
const PLAN_BYTES: usize = 16 * 1024 * 1024;
const MAX_PINS: usize = 4096;

pub(in crate::engine) struct PreservedProof {
    pub digest: [u8; 32],
    pub ordinary: bool,
    // Unavailable means the complete ORIGINAL proof ran, not that proof was skipped.
    pub protection: Preservation,
}
enum CaptureError { Protection(Preservation), Proof(String) }
impl From<String> for CaptureError { fn from(s: String) -> Self { Self::Proof(s) } }
impl From<Preservation> for CaptureError { fn from(s: Preservation) -> Self { Self::Protection(s) } }

pub(in crate::engine) fn streams_preserving_access(source: &File, ctl: &JobControl) -> Result<PreservedProof, String> {
    ctl.check()?;
    let mut plan = match Plan::capture(source, ctl) {
        Ok(plan) => plan,
        Err(CaptureError::Protection(status)) => {
            ctl.check()?;
            return Ok(PreservedProof { digest: streams(source, None, ctl)?, protection: status, ordinary: false });
        }
        Err(CaptureError::Proof(error)) => return Err(error),
    };
    plan.verify()?;
    let mut ordinary = false;
    let digest = streams_with_input(source, None, ctl, &mut plan, None, &mut ordinary)?;
    plan.verify()?;
    Ok(PreservedProof { digest, protection: Preservation::Applied, ordinary })
}

#[link(name="kernel32")]
extern "system" {
    fn BackupSeek(file: Handle, low: u32, high: u32, done_low: *mut u32, done_high: *mut u32, context: *mut Handle) -> i32;
    fn GetFileInformationByHandleEx(file: Handle, kind: i32, data: *mut u32, size: u32) -> i32;
}
fn close_backup(input: &mut Backup<'_>) -> Result<(), String> {
    if input.context.is_null() { return Ok(()); }
    let mut n = 0;
    if unsafe { BackupRead(input.file.as_raw_handle(), std::ptr::null_mut(), 0, &mut n, 1, 0, &mut input.context) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    input.context = std::ptr::null_mut();
    Ok(())
}
fn skip(input: &mut Backup<'_>, amount: u64) -> Result<(), String> {
    if amount == 0 { return Ok(()); }
    let (mut low, mut high) = (0, 0);
    if unsafe { BackupSeek(input.file.as_raw_handle(), amount as u32, (amount >> 32) as u32, &mut low, &mut high, &mut input.context) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if (u64::from(low) | (u64::from(high) << 32)) != amount { return Err("incomplete backup payload seek".into()); }
    Ok(())
}
#[derive(Clone, PartialEq, Eq)]
struct Record { metadata: Vec<u8>, kind: u32, stream: usize, offset: u64, length: u64 }
fn stream_name(name: &[u8]) -> Result<Vec<u16>, String> {
    let units = name.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>();
    let text = String::from_utf16(&units).map_err(|_| "invalid backup stream name")?;
    let inner = text.strip_prefix(':').and_then(|s| s.get(..s.len().checked_sub(6)?)).ok_or("invalid backup stream name")?;
    if !text.to_ascii_uppercase().ends_with(":$DATA") || inner.is_empty() || inner.chars().any(|c| matches!(c, ':' | '/' | '\\' | '\0')) {
        return Err("invalid backup stream name".into());
    }
    Ok(units)
}
// Only bounded metadata is consumed here. The original parser still validates
// every record and computes the complete compression/layout/content digest.
fn scan(source: &File, ctl: &JobControl, mut visit: impl FnMut(Record, &[u8]) -> Result<(), CaptureError>) -> Result<(), CaptureError> {
    let mut input = Backup { file: source, context: std::ptr::null_mut(), writing: false };
    let mut budget = 0usize;
    for _ in 0..4096 {
        ctl.check()?;
        let mut header = [0;20]; let n = input.read(&mut header)?;
        if n == 0 { close_backup(&mut input)?; return Ok(()); }
        input.exact(&mut header[n..])?;
        let kind = u32::from_le_bytes(header[..4].try_into().unwrap());
        let flags = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let size = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let namesize = u32::from_le_bytes(header[16..].try_into().unwrap()) as usize;
        if !matches!(kind,1|2|4|9) || namesize > 65536 || namesize % 2 != 0 { return Err("unsupported backup record".to_string().into()); }
        let extra = if kind == 2 {
            if size > super::super::file_workspace_ea::MAX_EA_BYTES as u64 { return Err("EA proof exceeds bound".to_string().into()); }
            size as usize
        } else if kind == 9 { 8 } else { 0 };
        budget = budget.checked_add(20 + namesize + extra + std::mem::size_of::<Record>()).ok_or("backup plan overflow".to_string())?;
        if budget > PLAN_BYTES { return Err("backup proof metadata exceeds 16 MiB".to_string().into()); }
        let mut name = vec![0;namesize]; input.exact(&mut name)?;
        if matches!(kind,1|9) && namesize != 0 { return Err("unexpected backup stream name".to_string().into()); }
        if kind != 2 && !matches!(flags,0|8) { return Err("unsupported backup flags".to_string().into()); }
        if kind == 4 { stream_name(&name)?; }
        let mut record = Record { metadata: header.to_vec(), kind, stream: 0, offset: 0, length: size };
        record.metadata.extend_from_slice(&name);
        if kind == 2 {
            let mut ea = vec![0;extra]; input.exact(&mut ea)?;
            super::super::file_workspace_ea::validate(flags,namesize,&ea,||ctl.check())?;
            record.metadata.extend_from_slice(&ea); record.length=0;
        } else if kind == 9 {
            if flags != 8 || size < 8 { return Err("invalid sparse backup record".to_string().into()); }
            let mut offset=[0;8];input.exact(&mut offset)?;
            record.offset=u64::from_le_bytes(offset);record.length-=8;record.metadata.extend_from_slice(&offset);
        }
        let remaining=record.length;
        // Capture the named stream pin before BackupRead moves on to another stream.
        visit(record,&name)?;
        skip(&mut input,remaining)?;
    }
    Err("too many backup records".to_string().into())
}

type Stamp = ((u32,u64),u64,u64,u64,u32);
fn stamp(file: &File) -> Result<Stamp,String> {
    let m=crate::private_stage::checked_metadata(file)?;
    if m.file_attributes() & 0x4400 != 0 { return Err("preserved proof rejects encrypted/reparse source".into()); }
    Ok((directory_identity(file)?,m.file_size(),m.creation_time(),m.last_write_time(),m.file_attributes()))
}
fn inventory(file:&File)->Result<Vec<(Vec<u8>,u64,u64)>,String>{
    let mut size=65536;
    loop {
        let mut data=vec![0u8;size];
        if unsafe{GetFileInformationByHandleEx(file.as_raw_handle(),7,data.as_mut_ptr().cast(),size as u32)}==0 {
            let error=std::io::Error::last_os_error();
            // FileStreamInfo reports ERROR_HANDLE_EOF for an NTFS directory
            // without named streams. Accept only that explicit empty result
            // on a proven directory; BackupRead is still independently scanned.
            if error.raw_os_error()==Some(38) && file.metadata().map_err(|e|e.to_string())?.is_dir() {return Ok(Vec::new());}
            if matches!(error.raw_os_error(),Some(122|234)) && size < PLAN_BYTES {size*=2;continue;}
            return Err(error.to_string());
        }
        let mut at=0usize;let mut out=Vec::new();
        loop {
            if at>data.len()-24 {return Err("invalid stream inventory".into());}
            let next=u32::from_le_bytes(data[at..at+4].try_into().unwrap()) as usize;
            let length=u32::from_le_bytes(data[at+4..at+8].try_into().unwrap()) as usize;
            if length>65536 || length%2!=0 || length>data.len()-at-24 {return Err("invalid stream inventory name".into());}
            out.push((data[at+24..at+24+length].to_vec(),u64::from_le_bytes(data[at+8..at+16].try_into().unwrap()),u64::from_le_bytes(data[at+16..at+24].try_into().unwrap())));
            if out.len()>MAX_PINS {return Err("too many source streams".into());}
            if next==0 {break;}
            if next<24+length || next>data.len()-at {return Err("invalid stream inventory offset".into());}
            at+=next;
        }
        out.sort();return Ok(out);
    }
}
struct Plan {
    files:Vec<File>, stamps:Vec<Stamp>, records:Vec<Record>, names:Vec<Vec<u16>>,
    inventory:Vec<(Vec<u8>,u64,u64)>, retained_payload_bytes:usize, ctl:JobControl, record:usize, position:u64,
}
impl Plan {
    fn capture(source:&File,ctl:&JobControl)->Result<Self,CaptureError>{
        let before=stamp(source)?;
        let main=if source.metadata().map_err(|e|e.to_string())?.is_dir() {
            crate::source_access::protect_shared_reader(source)?
        } else {crate::source_access::protect_reader(source,OpenSpec{access:0x80000000,share:1,flags:0x02200000})?};
        if stamp(&main)?!=before {return Err("protected source identity changed".to_string().into());}
        let initial_inventory=inventory(&main)?;
        let retained_payload_bytes=initial_inventory.iter().try_fold(
            initial_inventory.capacity().checked_mul(std::mem::size_of::<(Vec<u8>,u64,u64)>()).ok_or("backup budget overflow".to_string())?,
            |total,entry|total.checked_add(entry.0.capacity()).ok_or("backup budget overflow".to_string()))?;
        let scan_source=main.try_clone().map_err(|e|e.to_string())?;
        let mut plan=Self{files:vec![main],stamps:vec![before],records:Vec::new(),names:vec![Vec::new()],inventory:initial_inventory,retained_payload_bytes,ctl:ctl.clone(),record:0,position:0};
        plan.check_budget()?;
        let mut active=0;
        scan(&scan_source,ctl,|mut record,name|{
            if record.kind==1 {active=0;}
            if record.kind==4 {
                if plan.files.len()==MAX_PINS {return Err("too many protected stream handles".to_string().into());}
                let units=stream_name(name)?;
                let reader=super::super::file_workspace_compression::stream(&scan_source,&units,false)?;
                let expected=stamp(&reader)?;
                let reader=crate::source_access::protect_reader(&reader,OpenSpec{access:0x80000000,share:1,flags:0x02200000})?;
                if stamp(&reader)?!=expected || expected.0!=plan.stamps[0].0 {return Err("protected stream identity changed".to_string().into());}
                plan.account_payload(units.capacity().checked_mul(2).ok_or("backup budget overflow".to_string())?)?;
                active=plan.files.len();plan.stamps.push(expected);plan.files.push(reader);plan.names.push(units);
            }
            record.stream=active;
            let eof=plan.stamps[active].1;
            if record.kind!=2 && record.offset.checked_add(record.length).is_none_or(|end|end>eof) {return Err("backup payload exceeds pinned stream".to_string().into());}
            plan.push_record(record)?;Ok(())
        })?;
        Ok(plan)
    }
    // Retained metadata, names, inventories and handle bookkeeping are bounded
    // together. Temporary native inventory and replay-scan buffers each have the
    // same independent cap; the payload is never retained in either buffer.
    // Payload capacities are immutable once retained. Account each once, with
    // inventory (also immutable) folded once at construction. Each append checks
    // current Vec bookkeeping capacities in O(1), including allocation growth.
    fn account_payload(&mut self,bytes:usize)->Result<(),String>{
        self.retained_payload_bytes=self.retained_payload_bytes.checked_add(bytes).ok_or("backup budget overflow")?;
        Ok(())
    }
    fn push_record(&mut self,record:Record)->Result<(),String>{
        self.account_payload(record.metadata.capacity())?;
        self.records.push(record);
        self.check_budget()
    }
    fn budget_bytes(&self)->Result<usize,String>{
        [(self.records.capacity(),std::mem::size_of::<Record>()),
         (self.names.capacity(),std::mem::size_of::<Vec<u16>>()),
         (self.files.capacity(),std::mem::size_of::<File>()),
         (self.stamps.capacity(),std::mem::size_of::<Stamp>())].into_iter()
            .try_fold(self.retained_payload_bytes,|total,(capacity,unit)|
                total.checked_add(capacity.checked_mul(unit).ok_or("backup budget overflow")?).ok_or("backup budget overflow").map_err(String::from))
    }
    fn check_budget(&self)->Result<(),String>{
        if self.budget_bytes()?>PLAN_BYTES {return Err("retained backup proof plan exceeds 16 MiB".into());}
        Ok(())
    }
    fn verify(&self)->Result<(),String>{
        self.ctl.check()?;
        for (file,expected) in self.files.iter().zip(&self.stamps) {if &stamp(file)?!=expected{return Err("protected stream changed".into());}}
        if inventory(&self.files[0])?!=self.inventory{return Err("source stream inventory changed".into());}
        let mut at=0usize;let mut active=0usize;let mut next_stream=1;
        let result=scan(&self.files[0],&self.ctl,|mut record,name|{
            if record.kind==1 {active=0;}
            if record.kind==4 {
                let units=stream_name(name)?;
                if self.names.get(next_stream)!=Some(&units){return Err("backup stream order changed".to_string().into());}
                active=next_stream;next_stream+=1;
            }
            record.stream=active;
            if self.records.get(at)!=Some(&record){return Err("backup metadata changed".to_string().into());}
            at+=1;Ok(())
        });
        match result {Ok(())=>(),Err(CaptureError::Proof(e))=>return Err(e),Err(CaptureError::Protection(_))=>return Err("unexpected protection state".into())}
        if at!=self.records.len() || next_stream!=self.files.len(){return Err("incomplete backup metadata verification".into());}
        if inventory(&self.files[0])?!=self.inventory{return Err("source stream inventory changed".into());}
        Ok(())
    }
}
impl BackupInput for Plan {
    fn read(&mut self,bytes:&mut [u8])->Result<usize,String>{
        self.ctl.check()?;
        if bytes.is_empty(){return Ok(0);}
        loop {
            let Some(record)=self.records.get(self.record) else{return Ok(0)};
            let metadata=record.metadata.len() as u64;
            if self.position<metadata {
                let count=bytes.len().min((metadata-self.position) as usize);
                bytes[..count].copy_from_slice(&record.metadata[self.position as usize..self.position as usize+count]);self.position+=count as u64;return Ok(count);
            }
            let position=self.position-metadata;
            if position<record.length {
                let count=(record.length-position).min(bytes.len() as u64).min(65536) as usize;
                let file=&mut self.files[record.stream];
                file.seek(SeekFrom::Start(record.offset+position)).map_err(|e|e.to_string())?;
                let n=file.read(&mut bytes[..count]).map_err(|e|e.to_string())?;
                if n==0{return Err("protected backup payload ended early".into());}
                self.position+=n as u64;return Ok(n);
            }
            self.record+=1;self.position=0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn reader(p:&Path)->File {File::options().read(true).share_mode(1).custom_flags(0x02200000).open(p).unwrap()}
    fn old_time(p:&Path)->u64 {
        File::options().access_mode(0x100).share_mode(7).custom_flags(0x02200000).open(p).unwrap()
            .set_times(std::fs::FileTimes::new().set_accessed(std::time::UNIX_EPOCH+std::time::Duration::from_secs(946684800))).unwrap();
        fs::metadata(p).unwrap().last_access_time()
    }
    #[test]
    fn preserved_proof_matches_native_for_plain_empty_and_sparse_ads(){
        for kind in ["plain","empty","sparse"] {
            let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main content").unwrap();
            let ads=PathBuf::from(format!("{}:proof",p.display()));
            let mut a=File::options().read(true).write(true).create_new(true).open(&ads).unwrap();
            if kind=="plain" {a.write_all(&vec![73;150000]).unwrap();}
            if kind=="sparse" {
                use super::super::super::file_workspace_sparse as sparse;
                let eof=1024*1024+300;a.set_len(eof).unwrap();
                sparse::restore(&a,&sparse::Layout{eof,ranges:Vec::new()},&JobControl::new()).unwrap();
                a.seek(SeekFrom::Start(eof-300)).unwrap();a.write_all(&[42;300]).unwrap();
            }
            drop(a);let ctl=JobControl::new();
            let native=streams(&reader(&p),None,&ctl).unwrap();let before=old_time(&p);
            let input=reader(&p);let proof=streams_preserving_access(&input,&ctl).unwrap();
            assert_eq!(proof.protection,Preservation::Applied);assert_eq!(proof.digest,native,"{kind}");
            drop(input);assert_eq!(fs::metadata(&p).unwrap().last_access_time(),before,"{kind}");
        }
    }
    #[test]
    fn preserved_proof_plain_file_and_empty_directory(){
        for directory in [false,true] {
            let t=tempfile::tempdir().unwrap();let p=t.path().join("source");
            if directory{fs::create_dir(&p).unwrap();}else{fs::write(&p,[]).unwrap();}
            let native=streams(&reader(&p),None,&JobControl::new()).unwrap();let before=old_time(&p);
            let file=File::options().access_mode(0x80000100).share_mode(1).custom_flags(0x02200000).open(&p).unwrap();let proof=streams_preserving_access(&file,&JobControl::new()).unwrap();
            assert_eq!(proof.protection,Preservation::Applied);assert_eq!(proof.digest,native);drop(file);
            assert_eq!(fs::metadata(&p).unwrap().last_access_time(),before);
        }
    }
    #[test]
    fn preserved_proof_suppression_failure_runs_complete_native_proof(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main").unwrap();
        fs::write(format!("{}:ads",p.display()),b"alternate").unwrap();let file=reader(&p);let ctl=JobControl::new();
        let native=streams(&file,None,&ctl).unwrap();
        crate::source_access::FAIL_SUPPRESS.with(|v|v.set(true));
        let proof=streams_preserving_access(&file,&ctl).unwrap();assert_eq!(proof.digest,native);
        assert!(matches!(proof.protection,Preservation::Unavailable{phase:crate::source_access::Phase::SuppressUpdates,..}));
    }
    #[test]
    fn preserved_plan_cancellation_releases_pins_and_rejects_replacement(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main").unwrap();
        fs::write(format!("{}:ads",p.display()),b"alternate").unwrap();let file=reader(&p);let ctl=JobControl::new();
        let mut plan=Plan::capture(&file,&ctl).unwrap_or_else(|_|panic!("capture failed"));
        assert!(fs::rename(&p,t.path().join("replacement")).is_err());
        assert!(File::options().write(true).open(format!("{}:ads",p.display())).is_err());
        ctl.cancel();assert!(plan.read(&mut [0;20]).is_err());drop(plan);drop(file);
        assert!(File::options().write(true).open(format!("{}:ads",p.display())).is_ok());
    }
    #[test]
    fn preserved_plan_detects_new_stream_or_blocks_creation(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main").unwrap();let file=reader(&p);
        let plan=Plan::capture(&file,&JobControl::new()).unwrap_or_else(|_|panic!("capture failed"));
        if fs::write(format!("{}:new",p.display()),b"new stream").is_ok(){assert!(plan.verify().is_err());}
        else {assert!(plan.verify().is_ok());}
    }
    fn put_ea(path:&Path){
        #[repr(C)] struct IoStatus{status:usize,information:usize}
        #[link(name="ntdll")] unsafe extern "system" {fn NtSetEaFile(file:Handle,status:*mut IoStatus,data:*const u8,size:u32)->i32;}
        let mut data=vec![0,0,0,0,0x80,4,3,0];data.extend_from_slice(b"Test\0xyz");
        let file=File::options().access_mode(0x18).share_mode(7).open(path).unwrap();let mut status=IoStatus{status:0,information:0};
        assert_eq!(unsafe{NtSetEaFile(file.as_raw_handle(),&mut status,data.as_ptr(),data.len() as u32)},0);
    }
    #[test]
    fn preserved_proof_compression_sparse_and_ea_keep_complete_salted_digest(){
        use super::super::super::{file_workspace_compression as compression,file_workspace_sparse as sparse};
        for sparse_ads in [false,true] {
            let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,vec![43;90000]).unwrap();
            let main=File::options().read(true).write(true).open(&p).unwrap();compression::set(&main,2).unwrap();drop(main);
            put_ea(&p);let ap=PathBuf::from(format!("{}:proof",p.display()));
            let mut ads=File::options().read(true).write(true).create_new(true).open(&ap).unwrap();
            if sparse_ads {
                let eof=300000;ads.set_len(eof).unwrap();sparse::restore(&ads,&sparse::Layout{eof,ranges:Vec::new()},&JobControl::new()).unwrap();
                ads.seek(SeekFrom::Start(262144)).unwrap();ads.write_all(b"sparse ADS bytes").unwrap();
            } else {compression::set(&ads,2).unwrap();ads.write_all(&vec![76;180000]).unwrap();}
            drop(ads);let ctl=JobControl::new();let native=streams(&reader(&p),None,&ctl).unwrap();let before=old_time(&p);
            let file=reader(&p);let proof=streams_preserving_access(&file,&ctl).unwrap();
            assert_eq!(proof.protection,Preservation::Applied);assert_eq!(proof.digest,native);drop(file);
            assert_eq!(fs::metadata(&p).unwrap().last_access_time(),before);
        }
    }
    #[test]
    fn preserved_proof_real_attribute_denial_falls_back_without_acl_change(){
        use std::os::windows::ffi::OsStrExt;
        #[link(name="advapi32")] unsafe extern "system" {
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(s:*const u16,revision:u32,out:*mut *mut c_void,len:*mut u32)->i32;
            fn SetFileSecurityW(path:*const u16,flags:u32,sd:*mut c_void)->i32;
            fn GetFileSecurityW(path:*const u16,flags:u32,sd:*mut u8,len:u32,needed:*mut u32)->i32;
        }
        #[link(name="kernel32")] unsafe extern "system"{fn LocalFree(p:*mut c_void)->*mut c_void;}
        fn descriptor(path:&[u16])->Vec<u8>{
            let mut needed=0;unsafe{GetFileSecurityW(path.as_ptr(),4,std::ptr::null_mut(),0,&mut needed)};
            assert!(needed>0);let mut b=vec![0;needed as usize];assert_ne!(unsafe{GetFileSecurityW(path.as_ptr(),4,b.as_mut_ptr().cast(),needed,&mut needed)},0);b
        }
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main").unwrap();fs::write(format!("{}:ads",p.display()),b"alternate").unwrap();
        let path=p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
        let sddl="D:P(D;;0x100;;;WD)(A;;FA;;;OW)".encode_utf16().chain(Some(0)).collect::<Vec<_>>();let mut sd=std::ptr::null_mut();
        assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut sd,std::ptr::null_mut())},0);
        let set=unsafe{SetFileSecurityW(path.as_ptr(),0x80000004,sd)};unsafe{LocalFree(sd)};assert_ne!(set,0);
        let before=descriptor(&path);assert!(File::options().access_mode(0x80000100).share_mode(1).open(&p).is_err());
        let file=reader(&p);let ctl=JobControl::new();let expected=streams(&file,None,&ctl).unwrap();let proof=streams_preserving_access(&file,&ctl).unwrap();
        assert_eq!(proof.digest,expected);assert!(matches!(proof.protection,Preservation::Unavailable{os_error:Some(5),..}));
        assert_eq!(descriptor(&path),before);drop(file);
    }

    #[test]
    fn preserved_directory_ads_inventory_is_not_mistaken_for_empty(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::create_dir(&p).unwrap();
        fs::write(format!("{}:ads",p.display()),b"directory metadata").unwrap();
        let file=File::options().access_mode(0x80000100).share_mode(1).custom_flags(0x02200000).open(&p).unwrap();
        assert_eq!(inventory(&file).unwrap().len(),1);
        let expected=streams(&file,None,&JobControl::new()).unwrap();
        let proof=streams_preserving_access(&file,&JobControl::new()).unwrap();
        assert_eq!(proof.digest,expected);assert_eq!(proof.protection,Preservation::Applied);
    }
    #[test]
    fn preserved_directory_without_attribute_right_does_not_claim_protection(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::create_dir(&p).unwrap();let file=reader(&p);
        let expected=streams(&file,None,&JobControl::new()).unwrap();
        let proof=streams_preserving_access(&file,&JobControl::new()).unwrap();assert_eq!(proof.digest,expected);
        assert!(matches!(proof.protection,Preservation::Unavailable{phase:crate::source_access::Phase::SuppressUpdates,os_error:Some(5)}));
    }

    #[test]
    fn preserved_plan_rejects_ea_mutation_and_enforces_retained_budget(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main").unwrap();let file=reader(&p);
        let mut plan=Plan::capture(&file,&JobControl::new()).unwrap_or_else(|_|panic!("capture failed"));
        put_ea(&p);assert!(plan.verify().is_err());
        let mut extra=plan.records[0].clone();extra.metadata.reserve_exact(PLAN_BYTES);assert!(plan.push_record(extra).is_err());
    }

    #[test]
    fn preserved_plan_incremental_budget_matches_all_actual_capacities(){
        let t=tempfile::tempdir().unwrap();let p=t.path().join("source");fs::write(&p,b"main").unwrap();
        for i in 0..80 {fs::write(format!("{}:stream{i}",p.display()),b"payload").unwrap();}
        let file=reader(&p);let plan=Plan::capture(&file,&JobControl::new()).unwrap_or_else(|_|panic!("capture failed"));
        let actual=plan.records.capacity()*std::mem::size_of::<Record>()
            +plan.records.iter().map(|r|r.metadata.capacity()).sum::<usize>()
            +plan.names.capacity()*std::mem::size_of::<Vec<u16>>()
            +plan.names.iter().map(|n|n.capacity()*2).sum::<usize>()
            +plan.inventory.capacity()*std::mem::size_of::<(Vec<u8>,u64,u64)>()
            +plan.inventory.iter().map(|i|i.0.capacity()).sum::<usize>()
            +plan.files.capacity()*std::mem::size_of::<File>()
            +plan.stamps.capacity()*std::mem::size_of::<Stamp>();
        assert_eq!(plan.budget_bytes().unwrap(),actual);assert_eq!(plan.files.len(),81);
    }

}
