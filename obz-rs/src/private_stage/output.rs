//! Copy/memory output stays private, including ADS, until handle-based commit.
//! Unlike verified-file moves, creation here inherits the actual destination's
//! storage policy and supports its ordinary (including complex) file DACL.
use super::*;
use std::os::windows::fs::MetadataExt;

#[link(name = "Kernel32")]
extern "system" {
    fn GetVolumeInformationByHandleW(
        file: Handle,
        name: *mut u16,
        name_size: u32,
        serial: *mut u32,
        component: *mut u32,
        flags: *mut u32,
        filesystem: *mut u16,
        filesystem_size: u32,
    ) -> i32;
    fn GetFileInformationByHandle(file: Handle, information: *mut u32) -> i32;
    fn ReOpenFile(file: Handle, access: u32, share: u32, flags: u32) -> Handle;
    fn GetFinalPathNameByHandleW(file:Handle,path:*mut u16,size:u32,flags:u32)->u32;
    fn SetFileInformationByHandle(file: Handle, class: i32, info: *const c_void, size: u32) -> i32;
}
#[link(name = "Advapi32")]
extern "system" {
    fn OpenThreadToken(thread: Handle, access: u32, as_self: i32, token: *mut Handle) -> i32;
}

fn pinned_path(file:&File)->Result<String,String>{
    let mut value=vec![0u16;32768];
    let length=unsafe{GetFinalPathNameByHandleW(file.as_raw_handle(),value.as_mut_ptr(),value.len() as u32,0)};
    if length==0 || length as usize>=value.len(){return Err(error("query private native directory path"));}
    String::from_utf16(&value[..length as usize]).map_err(|_|"invalid private native directory path".into())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    PrivateAcl,
    ExclusiveSingleStream,
}
fn mode(flags: u32) -> Result<Mode, String> {
    if flags & 8 != 0 {
        Ok(Mode::PrivateAcl)
    } else if flags & 0x40000 == 0 {
        Ok(Mode::ExclusiveSingleStream)
    } else {
        Err(
            "secure output is unavailable: named streams require persistent file permissions"
                .into(),
        )
    }
}
fn volume_mode(parent: &File) -> Result<Mode, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("volume_mode");
    let mut flags = 0;
    if unsafe {
        GetVolumeInformationByHandleW(
            parent.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
    {
        return Err(error("query secure output filesystem"));
    }
    mode(flags)
}
fn effective_token() -> Result<Token, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("effective_token");
    let mut result = std::ptr::null_mut();
    if unsafe { OpenThreadToken(-2isize as Handle, 8, 1, &mut result) } != 0 {
        return Ok(Token(result));
    }
    if io::Error::last_os_error().raw_os_error() == Some(1008) {
        token()
    } else {
        Err(error("query output token"))
    }
}
pub(super) fn control(sd: Handle) -> Result<u16, String> {
    let (mut bits, mut revision) = (0, 0);
    if unsafe { GetSecurityDescriptorControl(sd, &mut bits, &mut revision) } == 0 {
        return Err(error("query output permission control"));
    }
    Ok(bits)
}
fn acl_bytes(sd: Handle) -> Result<Vec<u8>, String> {
    let (mut present, mut defaulted, mut acl) = (0, 0, std::ptr::null_mut());
    if unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted) } == 0 {
        return Err(error("query output permissions"));
    }
    if present == 0 || acl.is_null() {
        return Err("output has no bounded file permissions".into());
    }
    let size = unsafe { std::ptr::read_unaligned(acl.cast::<u8>().add(2).cast::<u16>()) } as usize;
    if size < 8 {
        return Err("invalid output permission descriptor".into());
    }
    Ok(unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), size) }.to_vec())
}
fn verify_private_permissions(
    file: &File,
    expected: &Allocation,
    user_sid: &str,
) -> Result<bool, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("verify_private_permissions");
    let (mut owner, mut raw) = (std::ptr::null_mut(), std::ptr::null_mut());
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            1,
            1 | 4, // OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut raw,
        )
    };
    if result != 0 {
        return Err(format!(
            "query private output permissions and owner: {}",
            io::Error::from_raw_os_error(result as i32)
        ));
    }
    let memory = Allocation(raw);
    // The owner points into this one descriptor. Keep its allocation alive
    // through both checks, so owner and DACL come from the same query.
    private_permissions_match(memory.0, owner, expected.0, user_sid)
}
fn private_permissions_match(
    actual: Handle,
    owner: Handle,
    expected: Handle,
    user_sid: &str,
) -> Result<bool, String> {
    if actual.is_null() || owner.is_null() {
        return Err("private output has no permission descriptor or owner".into());
    }
    if control(actual)? & 0x1000 == 0 || acl_bytes(actual)? != acl_bytes(expected)? {
        return Ok(false);
    }
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(owner, &mut text) } == 0 {
        return Err(error("read private output owner"));
    }
    let _text = Allocation(text.cast());
    let mut length = 0;
    unsafe {
        while *text.add(length) != 0 {
            length += 1;
        }
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|e| e.to_string())?;
    Ok(sid == user_sid || matches!(sid.as_str(), "S-1-5-18" | "S-1-5-32-544"))
}
fn ordinary_permissions(parent: &File, token: &Token) -> Result<PrivateDescriptor, String> {ordinary_permissions_kind(parent,token,false)}
fn ordinary_permissions_kind(parent: &File, token: &Token,directory:bool) -> Result<PrivateDescriptor, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("ordinary_permissions_kind");

    // Read the live DACL on every call. Most descriptors fit in one query;
    // oversized descriptors retain the existing unbounded Win32 implementation.
    // u32 alignment is required for the self-relative security descriptor.
    let mut storage = [0u32; 1024];
    let mut required = 0u32;
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQuerySecurityObject(handle: Handle, information: u32, descriptor: Handle,
            length: u32, required: *mut u32) -> i32;
    }
    let status = unsafe { NtQuerySecurityObject(parent.as_raw_handle(), 4,
        storage.as_mut_ptr().cast(), std::mem::size_of_val(&storage) as u32, &mut required) };
    if status >= 0 {
        return ordinary_permissions_from_descriptor(storage.as_mut_ptr().cast(), token, directory);
    }
    if !matches!(status as u32, 0xc0000023 | 0x80000005) {
        return Err(nt_error("read live destination permissions", status));
    }
    let policy = descriptor(parent)?;
    ordinary_permissions_from_descriptor(policy.0, token, directory)
}
// Let callers qualify and derive one captured descriptor, without a second
// parent read between those operations. They still re-read before publication.
pub(super) fn ordinary_permissions_from_descriptor(policy: Handle, token: &Token, directory: bool) -> Result<PrivateDescriptor, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("ordinary_permissions_from_descriptor");
    let automatic = control(policy)? & 0x400 != 0;
    // Windows interprets conditional, no-propagate, creator-owner and default
    // token ACEs. Request the parent's actual inheritance model at derivation,
    // not just in the resulting control bits: requesting AI for a legacy parent
    // changes stored ACE inheritance flags even if AI is subsequently cleared.
    // Directory creation needs container inheritance even for a legacy parent;
    // AUTO_INHERIT (the API input) and persisted AR/AI control bits are distinct.
    // Preserve the existing file policy, verified against its native comparators.
    // Do not replace this with the restricted move eligibility rule.
    let mut raw = std::ptr::null_mut();
    let mut mapping = GenericMapping {
        read: 0x120089,
        write: 0x120116,
        execute: 0x1200a0,
        all: 0x1f01ff,
    };
    if unsafe {
        CreatePrivateObjectSecurityEx(
            policy,
            std::ptr::null_mut(),
            &mut raw,
            std::ptr::null_mut(),
            i32::from(directory),
            u32::from(automatic||directory),
            token.0,
            &mut mapping,
        )
    } == 0
    {
        return Err(error("derive ordinary output permissions"));
    }
    let computed = PrivateDescriptor(raw);
    if unsafe { SetSecurityDescriptorControl(computed.0, 0x500, if automatic { 0x500 } else { 0 }) }
        == 0
    {
        return Err(error("preserve ordinary output inheritance"));
    }
    acl_bytes(computed.0)?;
    Ok(computed)
}
fn acl_entries(bytes: &[u8]) -> Result<&[u8], String> {
    let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    let mut end = 8;
    for _ in 0..count {
        let header = bytes
            .get(end..end + 4)
            .ok_or("invalid archive permission ACE")?;
        let size = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if size < 4 || size > bytes.len() - end {
            return Err("invalid archive permission ACE size".into());
        }
        end += size;
    }
    Ok(&bytes[8..end])
}
pub(super) fn same_file_permissions(a: Handle, b: Handle) -> Result<bool, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("same_file_permissions");
    // Allocation slack after the last ACE has no access semantics. Keep every
    // ACE byte, their order, and all protection/inheritance control flags.
    Ok((control(a)? ^ control(b)?) & 0x1400 == 0
        && acl_entries(&acl_bytes(a)?)? == acl_entries(&acl_bytes(b)?)?)
}
fn ordinary_streams(file: &File) -> Result<Option<bool>, String> {
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(file: Handle, class: i32, data: *mut u32, size: u32)
            -> i32;
    }
    // FILE_STREAM_INFO: next, UTF-16 byte length, two 64-bit sizes, then name.
    let mut aligned = [0u64; 64];
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            7,
            aligned.as_mut_ptr().cast(),
            std::mem::size_of_val(&aligned) as u32,
        )
    } == 0
    {
        let e = io::Error::last_os_error();
        return if matches!(e.raw_os_error(), Some(38 | 122 | 234)) {
            Ok(None)
        } else {
            Err(format!("query archive streams: {e}"))
        };
    }
    let bytes = unsafe {
        std::slice::from_raw_parts(
            aligned.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&aligned),
        )
    };
    let mut offset = 0;
    let mut data = false;
    let mut zone = false;
    loop {
        let record = bytes
            .get(offset..)
            .filter(|r| r.len() >= 24)
            .ok_or("invalid file stream record")?;
        let next = u32::from_le_bytes(record[..4].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(record[4..8].try_into().unwrap()) as usize;
        let name = record
            .get(24..)
            .and_then(|v| v.get(..length))
            .ok_or("invalid file stream name")?;
        let matches = |s: &str| {
            name.iter()
                .copied()
                .eq(s.encode_utf16().flat_map(u16::to_le_bytes))
        };
        if !data && matches("::$DATA") {
            data = true;
        } else if !zone && matches(":Zone.Identifier:$DATA") {
            zone = true;
        } else {
            return Ok(None);
        }
        if next == 0 {
            break;
        }
        if next % 8 != 0 || next < 24 + length {
            return Err("invalid file stream offset".into());
        }
        offset = offset
            .checked_add(next)
            .ok_or("invalid file stream offset")?;
    }
    Ok(data.then_some(zone))
}
fn only_default_stream(file: &File) -> Result<bool, String> {
    Ok(ordinary_streams(file)? == Some(false))
}
pub(super) fn output_marker(file: &File) -> Result<Option<Vec<u8>>, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("output_marker");
    match ordinary_streams(file)? {
        Some(false) => Ok(None),
        Some(true) => crate::motw::output_marker(file)?
            .map(Some)
            .ok_or("output zone stream disappeared".into()),
        None => Err("unexpected private output stream".into()),
    }
}
pub(super) fn metadata_handle(file: &File, share: u32) -> Result<File, String> {
    reopen(file, 0x20180, share)
}
pub(super) fn reopen(file: &File, access: u32, share: u32) -> Result<File, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("reopen");
    // Metadata access is independent of default-stream read/write sharing.
    // Keep this precise identity available for metadata after closing the
    // exclusive writer. The ordinary publisher's callers never read its data.
    let raw = unsafe { ReOpenFile(file.as_raw_handle(), access, share, 0x02200000) };
    if raw as isize == -1 {
        return Err(error("retain output metadata handle"));
    }
    Ok(unsafe { File::from_raw_handle(raw) })
}

fn private_permissions(sid: &str) -> Result<Allocation, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("private_permissions");
    let text: Vec<u16> = format!("D:P(A;;FA;;;{sid})(A;;FA;;;SY)(A;;FA;;;BA)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut raw = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &mut raw,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(error("prepare private output permissions"));
    }
    Ok(Allocation(raw))
}
pub(super) fn info(file: &File) -> Result<[u32; 13], String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("info");
    let mut value = [0; 13];
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), value.as_mut_ptr()) } == 0 {
        return Err(error("query private output identity"));
    }
    Ok(value)
}
fn leaf(value: &OsStr) -> Result<Vec<u16>, String> {
    let name: Vec<u16> = value.encode_wide().collect();
    if name.is_empty()
        || name.len() > 32767
        || name.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
        || name == [46]
        || name == [46, 46]
    {
        return Err("invalid private output leaf".into());
    }
    Ok(name)
}
#[repr(C)]
struct RenameInformation {
    replace: u8,
    root: Handle,
    length: u32,
    name: [u16; 1],
}
fn rename(file: &File, parent: &File, name: &[u16], replace: bool) -> Result<(), String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("rename");
    let offset = std::mem::offset_of!(RenameInformation, name);
    let length = offset + (name.len() + 1) * 2;
    let mut buffer = vec![0usize; length.div_ceil(std::mem::size_of::<usize>())];
    let raw = buffer.as_mut_ptr().cast::<u8>();
    let mut status = [0usize; 2];
    let result = unsafe {
        let header = raw.cast::<RenameInformation>();
        (*header).replace = u8::from(replace);
        (*header).root = parent.as_raw_handle();
        (*header).length = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr().cast::<u8>(), raw.add(offset), name.len() * 2);
        NtSetInformationFile(
            file.as_raw_handle(),
            status.as_mut_ptr(),
            raw.cast(),
            length as u32,
            10,
        )
    };
    if result < 0 {
        return Err(nt_error("commit private output", result));
    }
    Ok(())
}
fn nt_error(operation: &str, status: i32) -> String {
    format!(
        "{operation}: {}",
        io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Private,
    Committed,
    Finalized,
    Aborted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    BeforeCommit,
    AfterCommit,
    ApplyPermissions,
}

fn directory_metadata_handle(file:&File)->Result<File,String>{
    directory_handle(file,0x120180,7)
}
fn directory_handle(file:&File,access:u32,share:u32)->Result<File,String>{
    let mut name=UnicodeString{length:0,maximum_length:0,buffer:std::ptr::null_mut()};
    let attrs=ObjectAttributes{length:std::mem::size_of::<ObjectAttributes>() as u32,root:file.as_raw_handle(),name:&mut name,attributes:0x40,descriptor:std::ptr::null_mut(),qos:std::ptr::null_mut()};
    let mut status=IoStatus{status:0,information:0};let mut handle=std::ptr::null_mut();
    let result=unsafe{NtCreateFile(&mut handle,access,&attrs,&mut status,std::ptr::null(),0,share,1,0x200021,std::ptr::null_mut(),0)};
    if result<0 {return Err(nt_error("retain directory metadata handle",result));}
    Ok(unsafe{File::from_raw_handle(handle)})
}

// A local directory object is published before any children are copied into
// it. Refuse unexpected children, including competitors introduced mid-commit.
fn verify_empty_directory(file:&File)->Result<(),String>{
    #[link(name="ntdll")]
    extern "system" {fn NtQueryDirectoryFile(handle:Handle,event:Handle,apc:Handle,context:Handle,status:*mut usize,buffer:*mut c_void,length:u32,class:u32,single:u8,name:Handle,restart:u8)->i32;}
    let mut buffer=[0u32;1024];let mut status=[0usize;2];
    for attempt in 0..3 {
        let result=unsafe{NtQueryDirectoryFile(file.as_raw_handle(),std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),status.as_mut_ptr(),buffer.as_mut_ptr().cast(),4096,12,1,std::ptr::null_mut(),u8::from(attempt==0))};
        if result as u32==0x80000006{return Ok(())}
        if result<0{return Err(nt_error("verify private directory",result))}
        let length=buffer[2] as usize;
        if length>4084 || length%2!=0 {return Err("invalid private directory entry".into());}
        let name=unsafe{std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(12).cast::<u16>(),length/2)};
        if name!=[46] && name!=[46,46] {return Err("private directory contains unexpected children".into());}
    }
    Err("private directory contains unexpected entries".into())
}

/// Only local copy producers supply complete stream proofs. Archive publishers
/// retain their ordinary single-stream validation path.
pub(crate) trait LocalOutputProof {
    fn verify(&self, file: &File) -> Result<Vec<u8>, String>;
}
pub(crate) struct NativeOutputReservation {
    parent: File,
    path: PathBuf,
    permissions: PrivateDescriptor,
}
pub(crate) fn reserve_native_output(path: &Path, parent: &File) -> Result<NativeOutputReservation,String> {
    if info(parent)?[0]&0x410 != 0x10 || volume_mode(parent)? != Mode::PrivateAcl {
        return Err("native encrypted output requires a protected ordinary parent".into());
    }
    Ok(NativeOutputReservation { parent:parent.try_clone().map_err(|e|e.to_string())?,
        path:path.to_owned(), permissions:ordinary_permissions(parent,&effective_token()?)? })
}
impl NativeOutputReservation {
    // Ownership transfers immediately into a guard: every error removes only
    // this exact owned object, including failures after the private rename.
    pub(crate) fn adopt(self, file:File, private_parent:&File, old_path:&Path) -> Result<PrivateOutputFile,String> {
        let mut output=PrivateOutputFile {direct:false,directory:false,file:Some(file),
            parent:self.parent,path:old_path.to_owned(),permissions:Some(self.permissions),state:State::Private};
        verify_native_private_child(private_parent,output.as_file())?;
        let token=effective_token()?;let sid=current_sid(&token)?;
        let live=ordinary_permissions(&output.parent,&token)?;
        if !same_file_permissions(output.permissions.as_ref().unwrap().0,live.0)? {
            return Err("destination permissions changed during native copy".into());
        }
        let wide:Vec<u16>=format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FA;;;SY)(A;;FA;;;BA)").encode_utf16().chain(Some(0)).collect();
        let mut raw=std::ptr::null_mut();
        if unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(wide.as_ptr(),1,&mut raw,std::ptr::null_mut())}==0{return Err(error("protect adopted encrypted output"));}
        let private=Allocation(raw);
        let status=unsafe{NtSetSecurityObject(output.as_file().as_raw_handle(),0x80000005,private.0)};
        if status<0{return Err(nt_error("protect adopted encrypted output",status));}
        if !verify_private_permissions(output.as_file(),&private_permissions(&sid)?,&sid)? {
            return Err("adopted encrypted output permissions were not retained".into());
        }
        let name=private_name()?;
        rename(output.as_file(),&output.parent,&leaf(OsStr::new(&name))?,false)?;
        output.path=self.path.join(name);
        Ok(output)
    }
}
pub(crate) struct PrivateOutputFile {
    direct: bool,
    directory:bool,
    file: Option<File>,
    parent: File,
    path: PathBuf,
    permissions: Option<PrivateDescriptor>,
    state: State,
}
// A native ordinary-file child marks inherited ACEs with INHERITED_ACE,
// whereas CreatePrivateObjectSecurity's legacy derivation leaves that marker
// clear. The marker records provenance, not access; all other bytes remain exact.
fn native_file_aces(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut entries = acl_entries(bytes)?.to_vec();
    let mut at = 0;
    while at < entries.len() {
        let size = usize::from(u16::from_le_bytes([entries[at+2], entries[at+3]]));
        entries[at+1] &= !0x10;
        at += size;
    }
    Ok(entries)
}
fn native_default_owner(token: &Token, user: &str) -> Result<String, String> {
    let mut size = 0;
    unsafe { GetTokenInformation(token.0, 4, std::ptr::null_mut(), 0, &mut size); }
    if size < std::mem::size_of::<Handle>() as u32 || size > 1024 * 1024 {
        return Err("invalid native token owner".into());
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    if unsafe { GetTokenInformation(token.0, 4, buffer.as_mut_ptr().cast(), size, &mut size) } == 0 {
        return Err(error("query native token owner"));
    }
    let owner = sid_text(unsafe { *(buffer.as_ptr().cast::<Handle>()) })?;
    // These are already authorized by the independently verified private ACL.
    // An unusual token owner must not gain implicit WRITE_DAC over our output.
    if owner != user && owner != "S-1-5-32-544" {
        return Err("native token owner is outside private principals".into());
    }
    Ok(owner)
}
/// EFS candidate only: verify native-created children still inherit exclusively
/// from our protected stage, rather than trusting the copy API's ACL behavior.
pub(crate) fn verify_native_private_child(parent:&File,file:&File)->Result<(),String>{
    let token=effective_token()?;let sid=current_sid(&token)?;
    let text:Vec<u16>=format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
        .encode_utf16().chain(Some(0)).collect();
    let mut raw=std::ptr::null_mut();
    if unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(),1,&mut raw,std::ptr::null_mut())}==0{return Err(error("derive private native stage"));}
    let expected_parent=Allocation(raw);
    if !verify_private_permissions(parent,&expected_parent,&sid)?{return Err("native stage permissions changed".into());}
    let expected=ordinary_permissions(parent,&token)?;let actual=descriptor(file)?;
    if control(expected.0)? != control(actual.0)?
        || native_file_aces(&acl_bytes(expected.0)?)? != native_file_aces(&acl_bytes(actual.0)?)? {
        #[cfg(test)] eprintln!("native private ACL mismatch expected control={:#x} entries={:?}; actual control={:#x} entries={:?}",control(expected.0)?,acl_entries(&acl_bytes(expected.0)?)?,control(actual.0)?,acl_entries(&acl_bytes(actual.0)?)?);
        return Err("native output did not retain private inherited permissions".into());
    }
    let (mut owner,mut raw)=(std::ptr::null_mut(),std::ptr::null_mut());
    let status=unsafe{GetSecurityInfo(file.as_raw_handle(),1,1,&mut owner,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),&mut raw)};
    if status!=0{return Err(format!("native output owner query failed: {status}"));}
    let _owner=Allocation(raw);let mut text=std::ptr::null_mut();
    if owner.is_null() || unsafe{ConvertSidToStringSidW(owner,&mut text)}==0{return Err(error("read native output owner"));}
    let _text=Allocation(text.cast());let mut length=0;
    unsafe{while *text.add(length)!=0{length+=1;}}
    let owner_sid=String::from_utf16(unsafe{std::slice::from_raw_parts(text,length)}).map_err(|e|e.to_string())?;
    let expected_owner = native_default_owner(&token, &sid)?;
    if owner_sid!=expected_owner{
        #[cfg(test)] eprintln!("native output owner expected={expected_owner} actual={owner_sid}");
        return Err("native output owner changed".into());
    }
    Ok(())
}
impl PrivateOutputFile {
    pub(crate) fn as_file(&self) -> &File {
        self.file.as_ref().expect("owned output handle")
    }
    pub(crate) fn as_file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("owned output handle")
    }
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn commit(&mut self, target: &Path, replace: bool) -> Result<File, String> {
        self.commit_with(target, replace, &mut |_| Ok(()))
    }
    fn commit_with(
        &mut self,
        target: &Path,
        replace: bool,
        step: &mut dyn FnMut(Step) -> Result<(), String>,
    ) -> Result<File, String> {
        self.commit_journaled(target, replace, None, step, None)
    }
    pub(crate) fn commit_owned(
        &mut self,
        target: &Path,
        journal: &crate::engine::PublicationJournal,
    ) -> Result<File, String> {
        self.commit_journaled(target, false, Some(journal), &mut |_| Ok(()), None)
    }
    /// Retain metadata access before the ordinary inherited DACL is finalized,
    /// just like the directory publisher's existing metadata handle.
    pub(crate) fn retain_directory_ea_metadata(&self)->Result<File,String>{
        if !self.directory || self.state!=State::Private{return Err("directory metadata ownership required".into());}
        directory_handle(self.as_file(),0x120188,7)
    }
    /// Narrow EFS-directory native operation. A metadata handle remains live
    /// during Raw import and the relative-empty-name takeover, so neither path
    /// substitution nor ID recycling can authorize a different output object.
    /// The protected DACL remains unchanged; general publication stays exclusive.
    pub(crate) fn native_directory<T>(&mut self, run:impl FnOnce(&File,&Path)->Result<T,String>)->Result<T,String>{
        if !self.directory || self.state!=State::Private || self.direct || self.permissions.is_none(){
            return Err("native directory requires an owned private ACL output".into());
        }
        verify_empty_directory(self.as_file())?;
        let token=effective_token()?;let sid=current_sid(&token)?;let private=private_permissions(&sid)?;
        if !verify_private_permissions(self.as_file(),&private,&sid)?{return Err("private directory permissions changed".into());}
        let original=info(self.as_file())?;let original_path=pinned_path(self.as_file())?;
        let metadata=directory_handle(self.as_file(),0x120088,5)?;
        self.file=Some(metadata);
        let result=run(self.as_file(),&self.path);
        // An empty name relative to the still-held original directory opens
        // that object, not a pathname. Unlike a FILE_OPEN_BY_FILE_ID handle,
        // this normal handle can subsequently rename and dispose the output.
        // Keep metadata alive until exclusive ownership is established.
        let strong=directory_handle(self.as_file(),0x17019f,0)?;
        let actual=info(&strong)?;
        if [original[7],original[11],original[12]] != [actual[7],actual[11],actual[12]] {
            return Err("native directory identity changed; no replacement was removed".into());
        }
        // Retain the owned strong handle even on rejection; abort operates on
        // this original object and cannot delete a competitor at the old name.
        self.file=Some(strong);
        if pinned_path(self.as_file())?!=original_path {return Err("native directory name changed; replacement retained".into());}
        if !verify_private_permissions(self.as_file(),&private,&sid)?{return Err("native directory permissions changed".into());}
        verify_empty_directory(self.as_file())?;
        result
    }
    /// Explicit local-file mode; default/archive output still rejects custom ADS.
    pub(crate) fn commit_local_streams(&mut self,target:&Path,replace:bool,proof:&dyn LocalOutputProof)->Result<File,String>{
        proof.verify(self.as_file())?;
        self.commit_journaled(target,replace,None,&mut |_|Ok(()),Some(proof))
    }
    fn commit_journaled(
        &mut self,
        target: &Path,
        replace: bool,
        journal: Option<&crate::engine::PublicationJournal>,
        step: &mut dyn FnMut(Step) -> Result<(), String>,
        local_proof: Option<&dyn LocalOutputProof>,
    ) -> Result<File, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("commit_journaled");
        if self.direct && (replace || journal.is_none() || target != self.path || local_proof.is_some()) {
            return Err("direct output requires its owned no-replace journal".into());
        }
        if self.directory && local_proof.is_none() {return Err("directory output requires local stream proof".into());}
        if self.directory {verify_empty_directory(self.as_file())?;}
        let mut pending = None;
        if self.state != State::Private {
            return Err("output is no longer private".into());
        }
        if target.parent() != self.path.parent() {
            return Err("output parent changed".into());
        }
        let name = leaf(target.file_name().ok_or("missing output leaf")?)?;
        let current = info(self.as_file())?;
        if current[0] & 0x410 != if self.directory {0x10}else{0} || current[10] != 1 {
            return Err("private output identity changed".into());
        }
        if info(&self.parent)?[0] & 0x410 != 0x10 {
            return Err("output parent changed".into());
        }
        step(Step::BeforeCommit)?;
        if let Some(expected) = &self.permissions {
            let token = effective_token()?;
            let live = ordinary_permissions_kind(&self.parent, &token,self.directory)?;
            if !same_file_permissions(expected.0, live.0)? {
                return Err("destination permissions changed; output was not published".into());
            }
            let committed_handle = if self.direct {self.as_file().try_clone().map_err(|e|e.to_string())?} else if self.directory {directory_metadata_handle(self.as_file())?}else{metadata_handle(self.as_file(), 7)?};
            let marker = match local_proof {Some(proof)=>Some(proof.verify(self.as_file())?),None=>output_marker(self.as_file())?};
            let private = descriptor(self.as_file())?;
            step(Step::ApplyPermissions)?;
            let transition = (|| {
                apply_native(self.as_file(), &live)?;
                let fresh = ordinary_permissions_kind(&self.parent, &token,self.directory)?;
                if !same_file_permissions(live.0, fresh.0)? {
                    return Err("destination permissions changed; output was not published".into());
                }
                if info(self.as_file())?[10] != 1 || (match local_proof {Some(proof)=>Some(proof.verify(self.as_file())?),None=>output_marker(self.as_file())?}) != marker {
                    return Err("private output links or streams changed".into());
                }
                if self.directory {verify_empty_directory(self.as_file())?;}
                pending = journal
                    .map(|_| OutputReceipt::capture(target, self.as_file()))
                    .transpose()?;
                if self.direct {
                    #[cfg(test)]
                    BEFORE_DIRECT_CLEAR.with(|hook|if let Some(run)=hook.borrow_mut().take(){run()});
                    // The creation-time DELETE_ON_CLOSE flag is cancelled only
                    // after bytes, metadata, live ACLs and rollback evidence exist.
                    // flags=0 does NOT cancel creation-time deletion; ON_CLOSE does.
                    let keep = 8u32;
                    if unsafe {SetFileInformationByHandle(self.as_file().as_raw_handle(),21,
                        (&keep as *const u32).cast(),4)} == 0 {return Err(error("commit direct output"));}
                    Ok(())
                } else {rename(self.as_file(), &self.parent, &name, replace)}
            })();
            if let Err(reason) = transition {
                // Restore the protected scratch permissions while the data
                // stream is still held exclusively, including collision paths.
                let status = unsafe {
                    NtSetSecurityObject(self.as_file().as_raw_handle(), 0x80000004, private.0)
                };
                if status < 0 {
                    let cleanup = self.abort();
                    return Err(format!(
                        "{reason}; restoring private output failed; cleanup: {cleanup:?}"
                    ));
                }
                return Err(reason);
            }
            self.state = State::Committed;
            self.path = target.to_owned();
            self.file = Some(committed_handle);
        } else {
            // Single-stream media have no persistent DACL to restore.
            step(Step::ApplyPermissions)?;
            pending = journal
                .map(|_| OutputReceipt::capture(target, self.as_file()))
                .transpose()?;
            rename(self.as_file(), &self.parent, &name, replace)?;
            self.state = State::Committed;
            self.path = target.to_owned();
        }
        if let (Some(journal), Some(receipt)) = (journal, pending) {
            journal.record(receipt);
        }
        // No policy snapshot is applied after the commit point (rename or
        // clearing DELETE_ON_CLOSE for the still-exclusively-held direct name).
        step(Step::AfterCommit)
            .map_err(|e| format!("output was committed: {}: {e}", target.display()))?;
        self.state = State::Finalized;
        Ok(self.file.take().expect("committed output handle"))
    }
    /// Restore inheritance at the original (console-compatible) depth while
    /// the default stream is still exclusively held and has no public name.
    /// Once ordinary inheritance is active it can follow subsequent native
    /// propagation. The public rename must never replay a prior DACL.
    pub(super) fn publish_archive(
        &mut self,
        target_parent: &File,
        ordinary: &File,
        name: &OsStr,
        before_commit: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.state != State::Private {
            return Err("invalid archive publication state".into());
        }
        let expected = self
            .permissions
            .as_ref()
            .ok_or("archive storage has no inherited permissions")?;
        let token = effective_token()?;
        let current = ordinary_permissions(&self.parent, &token)?;
        if !same_file_permissions(expected.0, current.0)? {
            return Err("archive staging permissions changed; archive was not published".into());
        }
        apply_native(self.as_file(), &current)?;
        let actual = descriptor(self.as_file())?;
        let fresh = ordinary_permissions(&self.parent, &token)?;
        let ordinary_acl = descriptor(ordinary)?;
        // Compare to an actual unprotected file created by NTFS under the
        // same parent. Legacy NTFS marks inherited ACEs differently from
        // the private-object derivation used to restore the data file.
        if !same_file_permissions(actual.0, ordinary_acl.0)?
            || !same_file_permissions(fresh.0, expected.0)?
        {
            return Err(
                "archive inheritance changed during finalization; archive was not published".into(),
            );
        }
        let metadata = info(self.as_file())?;
        if metadata[0] & 0x410 != 0 || metadata[10] != 1 || !only_default_stream(self.as_file())? {
            return Err("archive staging identity or streams changed".into());
        }
        before_commit()?;
        rename(self.as_file(), target_parent, &leaf(name)?, false)?;
        self.state = State::Finalized;
        self.file.take();
        Ok(())
    }
    pub(crate) fn abort(&mut self) -> Result<(), String> {
        self.abort_with(&mut || Ok(()))
    }
    fn abort_with(
        &mut self,
        before_delete: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.state != State::Private {
            return Ok(());
        }
        before_delete().map_err(|e| {
            format!(
                "cleanup private output failed: {}: {e}",
                self.path.display()
            )
        })?;
        discard_private_handle(self.as_file(), &self.path)?;
        self.state = State::Aborted;
        self.file.take();
        Ok(())
    }
}
pub(super) fn discard_private_handle(file: &File, path: &Path) -> Result<(), String> {
    let attrs = file
        .metadata()
        .map_err(|e| {
            format!(
                "cleanup private output failed: {}: query attributes: {e}",
                path.display()
            )
        })?
        .file_attributes();
    if attrs & 1 != 0 {
        let mut basic = [0u64; 5];
        // Preserve inherited storage flags; clear only readonly.
        basic[4] = u64::from(if attrs & !1 == 0 { 0x80 } else { attrs & !1 });
        if unsafe { SetFileInformationByHandle(file.as_raw_handle(), 0, basic.as_ptr().cast(), 40) }
            == 0
        {
            return Err(format!(
                "cleanup private output failed: {}: {}",
                path.display(),
                error("clear readonly")
            ));
        }
    }
    let mut status = [0usize; 2];
    let delete = 1u8;
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            status.as_mut_ptr(),
            (&delete as *const u8).cast(),
            1,
            13,
        )
    };
    if result < 0 {
        return Err(format!(
            "cleanup private output failed: {}: {}",
            path.display(),
            nt_error("delete owned output", result)
        ));
    }
    Ok(())
}
impl Drop for PrivateOutputFile {
    fn drop(&mut self) {
        let _ = self.abort();
    }
}

/// Caller retains its full ancestor guard chain. The parent is an already
/// identity-checked directory handle with READ_CONTROL/TRAVERSE, deny-delete.
pub(crate) fn create_output(
    parent_path: &Path,
    parent: &File,
) -> Result<PrivateOutputFile, String> {
    create_exclusive(parent_path, parent)
}
pub(super) fn create_archive_file(
    parent_path: &Path,
    parent: &File,
) -> Result<PrivateOutputFile, String> {
    create_exclusive(parent_path, parent)
}
pub(super) fn private_name() -> Result<String, String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("private_name");
    let mut random = [0u8; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), random.as_mut_ptr(), 16, 2) } < 0 {
        return Err("cannot generate private output name".into());
    }
    let suffix = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Ok(format!(".qing-zip-output-{suffix}"))
}
pub(crate) fn create_local_directory_output(parent_path:&Path,parent:&File)->Result<PrivateOutputFile,String>{
    create_exclusive_kind(parent_path,parent,true)
}
fn create_exclusive(parent_path: &Path, parent: &File) -> Result<PrivateOutputFile, String> {
    create_exclusive_kind(parent_path,parent,false)
}
pub(crate) fn direct_output_supported(parent:&File)->bool {
    let mut filesystem=[0u16;32];let mut flags=0u32;
    let ok=unsafe{GetVolumeInformationByHandleW(parent.as_raw_handle(),std::ptr::null_mut(),0,
        std::ptr::null_mut(),std::ptr::null_mut(),&mut flags,filesystem.as_mut_ptr(),filesystem.len() as u32)};
    ok!=0 && flags&8!=0 && String::from_utf16_lossy(&filesystem[..filesystem.iter().position(|v|*v==0).unwrap_or(filesystem.len())]).eq_ignore_ascii_case("NTFS")
}
pub(crate) fn create_direct_output(parent_path: &Path, parent: &File, name: &OsStr) -> Result<PrivateOutputFile,String> {
    leaf(name)?;
    create_exclusive_named(parent_path,parent,false,Some(name))
}
#[cfg(test)]
thread_local! {static BEFORE_DIRECT_CLEAR:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const{std::cell::RefCell::new(None)};}
fn create_exclusive_kind(parent_path: &Path, parent: &File,directory:bool) -> Result<PrivateOutputFile, String> {
    create_exclusive_named(parent_path,parent,directory,None)
}
fn create_exclusive_named(parent_path: &Path, parent: &File,directory:bool, direct_name: Option<&OsStr>) -> Result<PrivateOutputFile,String> {
    #[cfg(feature = "output-phase-profile")]
    let _phase = crate::output_profile::Span::new("create_exclusive_kind");
    if info(parent)?[0] & 0x410 != 0x10 {
        return Err("invalid private output parent".into());
    }
    let mode = volume_mode(parent)?;
    if direct_name.is_some() && mode != Mode::PrivateAcl {return Err("direct output requires persistent private permissions".into());}
    let token = effective_token()?;
    let permissions = if mode == Mode::PrivateAcl {
        Some(ordinary_permissions_kind(parent, &token,directory)?)
    } else {
        None
    };
    let private = if mode == Mode::PrivateAcl {
        // Reuse only this held token's user SID within this synchronous call.
        // Resolve the effective thread/process token afresh for every output.
        let sid = current_sid(&token)?;
        Some((private_permissions(&sid)?, sid))
    } else {
        None
    };
    let parent_pin = parent.try_clone().map_err(|e| e.to_string())?;
    for _ in 0..16 {
        let name = match direct_name {Some(name)=>name.to_os_string(),None=>private_name()?.into()};
        let mut wide: Vec<u16> = name.encode_wide().collect();
        let mut unicode = UnicodeString {
            length: (wide.len() * 2) as u16,
            maximum_length: (wide.len() * 2) as u16,
            buffer: wide.as_mut_ptr(),
        };
        let attributes = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root: parent.as_raw_handle(),
            name: &mut unicode,
            attributes: 0x40,
            descriptor: private.as_ref().map_or(std::ptr::null_mut(), |(p, _)| p.0),
            qos: std::ptr::null_mut(),
        };
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        let result = {
            #[cfg(feature = "output-phase-profile")]
            let _phase = crate::output_profile::Span::new("NtCreateFile.output");
            unsafe {
            NtCreateFile(
                &mut handle,
                0x17019f,
                &attributes,
                &mut status,
                std::ptr::null(),
                if directory {0x10}else{0x80},
                0,
                2,
                if directory {0x200021}else{0x200060 | if direct_name.is_some(){0x1000}else{0}},
                std::ptr::null_mut(),
                0,
            )
        } };
        if result < 0 {
            let code = unsafe { RtlNtStatusToDosError(result) };
            if direct_name.is_none() && matches!(code, 80 | 183) {
                continue;
            }
            return Err(nt_error("create private output", result));
        }
        let mut output = PrivateOutputFile {
            direct: direct_name.is_some(),
            directory,
            file: Some(unsafe { File::from_raw_handle(handle) }),
            parent: parent_pin,
            path: parent_path.join(name),
            permissions,
            state: State::Private,
        };
        let validation = (|| {
            let actual = info(output.as_file())?;
            if actual[0] & 0x410 != if directory {0x10}else{0} || actual[10] != 1 {
                return Err("invalid private output file".into());
            }
            if volume_mode(output.as_file())? != mode {
                return Err("output filesystem capability changed".into());
            }
            if let Some((expected, sid)) = &private {
                if !verify_private_permissions(output.as_file(), expected, sid)? {
                    return Err("filesystem did not preserve private output permissions".into());
                }
            }
            Ok(())
        })();
        if let Err(reason) = validation {
            return Err(match output.abort() {
                Ok(()) => reason,
                Err(cleanup) => format!("{reason}; {cleanup}"),
            });
        }
        return Ok(output);
    }
    Err("could not reserve private output file".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_directory_overlap_recovers_exclusive_ownership_on_success_and_error(){
        for fail in [false,true]{
            let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
            let mut output=create_local_directory_output(root.path(),&parent).unwrap();
            let path=output.path.clone();let id=info(output.as_file()).unwrap();
            let result=output.native_directory(|file,_|{
                assert_eq!(info(file).unwrap()[11..13],id[11..13]);
                if fail{Err("injected native cancellation".into())}else{Ok(())}
            });
            assert_eq!(result.is_err(),fail,"{result:?}");
            assert_eq!(info(output.as_file()).unwrap()[11..13],id[11..13]);
            output.abort().unwrap();assert!(!path.exists());
        }
    }
    #[test]
    fn native_directory_overlap_rejects_name_replacement_without_deleting_competitor(){
        let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
        let mut output=create_local_directory_output(root.path(),&parent).unwrap();
        let path=output.path.clone();let moved=root.path().join("moved-owned");
        let result=output.native_directory(|_,name|{
            std::fs::rename(name,&moved).unwrap();std::fs::create_dir(name).unwrap();
            std::fs::write(name.join("competitor"),b"retain").unwrap();Ok(())
        });
        let error=result.unwrap_err();assert!(error.contains("name changed"),"{error}");
        output.abort().unwrap();assert!(!moved.exists());
        assert_eq!(std::fs::read(path.join("competitor")).unwrap(),b"retain");
    }
    #[test]
    fn native_directory_overlap_rejects_child_and_preserves_it_during_cleanup(){
        let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
        let mut output=create_local_directory_output(root.path(),&parent).unwrap();
        let path=output.path.clone();
        let result=output.native_directory(|_,name|{
            std::fs::write(name.join("competitor"),b"retain").unwrap();Ok(())
        });
        assert!(result.is_err(),"{result:?}");assert!(output.abort().is_err());
        drop(output);assert_eq!(std::fs::read(path.join("competitor")).unwrap(),b"retain");
    }
    #[test]
    fn native_file_ace_equivalence_only_ignores_inherited_marker() {
        let mut original = vec![2,0,16,0,1,0,0,0,0,0,8,0,1,2,3,4];
        let expected = super::native_file_aces(&original).unwrap();
        original[9] = 0x10;
        assert_eq!(expected, super::native_file_aces(&original).unwrap());
        for flag in [1,2,4,8,0x40,0x80] {
            original[9] = 0x10 | flag;
            assert_ne!(expected, super::native_file_aces(&original).unwrap());
        }
        original[9] = 0x10;
        for offset in [8,12,13,14,15] {
            original[offset] ^= 1;
            assert_ne!(expected, super::native_file_aces(&original).unwrap());
            original[offset] ^= 1;
        }
        original[4] = 2;
        original.extend_from_slice(&[0,0,8,0,5,6,7,8]);
        original[2] = 24;
        assert_ne!(expected, super::native_file_aces(&original).unwrap());
    }

    use super::*;
    use std::{
        fs,
        io::Write,
        os::windows::fs::OpenOptionsExt,
        time::{Duration, UNIX_EPOCH},
    };

    fn open_parent(path: &Path) -> File {
        fs::OpenOptions::new()
            .access_mode(0x600a0)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(path)
            .unwrap()
    }
    fn policy(parent: &File, sddl: &str) {
        let wide: Vec<_> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut raw = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    1,
                    &mut raw,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let memory = Allocation(raw);
        if sddl.contains("PAI") {
            assert_ne!(
                unsafe { SetSecurityDescriptorControl(memory.0, 0x500, 0x500) },
                0
            );
        }
        assert!(
            unsafe { NtSetSecurityObject(parent.as_raw_handle(), 4 | 0x80000000, memory.0) } >= 0
        );
    }
    #[test]
    fn native_private_child_rejects_changed_parent_and_added_principal() {
        let temp = tempfile::tempdir().unwrap();
        let parent = open_parent(temp.path());
        let sid = current_sid(&effective_token().unwrap()).unwrap();
        let private = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        policy(&parent, &private);
        let path = temp.path().join("native-child");
        let child = fs::OpenOptions::new().write(true).access_mode(0x60003).share_mode(7)
            .create_new(true).open(&path).unwrap();
        verify_native_private_child(&parent, &child).unwrap();
        policy(&child, &format!("D:P(A;;FA;;;{sid})(A;;FA;;;SY)(A;;FA;;;BA)(A;;GR;;;WD)"));
        assert!(verify_native_private_child(&parent, &child).is_err());
        drop(child);
        fs::remove_file(&path).unwrap();
        let child = fs::OpenOptions::new().write(true).access_mode(0x60003).share_mode(7)
            .create_new(true).open(&path).unwrap();
        verify_native_private_child(&parent, &child).unwrap();
        policy(&parent, &format!("{private}(A;OICI;GR;;;WD)"));
        assert_eq!(verify_native_private_child(&parent, &child).unwrap_err(), "native stage permissions changed");
    }
    fn security(file: &File) -> Vec<u8> {
        let mut raw = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    1,
                    7,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut raw,
                )
            },
            0
        );
        let memory = Allocation(raw);
        let size = unsafe { GetSecurityDescriptorLength(memory.0) } as usize;
        unsafe { std::slice::from_raw_parts(memory.0.cast::<u8>(), size).to_vec() }
    }
    fn stored_security(path: &Path) -> Vec<u8> {
        #[link(name = "Advapi32")]
        extern "system" {
            fn GetFileSecurityW(
                path: *const u16,
                information: u32,
                descriptor: *mut u8,
                length: u32,
                needed: *mut u32,
            ) -> i32;
        }
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut size = 0;
        unsafe {
            GetFileSecurityW(path.as_ptr(), 7, std::ptr::null_mut(), 0, &mut size);
        }
        assert!(size >= 20);
        let mut bytes = vec![0; size as usize];
        assert_ne!(
            unsafe { GetFileSecurityW(path.as_ptr(), 7, bytes.as_mut_ptr(), size, &mut size) },
            0
        );
        bytes
    }
    fn assert_private(file: &File) {
        let expected =
            private_permissions(&current_sid(&effective_token().unwrap()).unwrap()).unwrap();
        let actual = descriptor(file).unwrap();
        assert_ne!(control(actual.0).unwrap() & 0x1000, 0);
        assert_eq!(acl_bytes(actual.0).unwrap(), acl_bytes(expected.0).unwrap());
    }
    #[test]
    fn filesystem_capabilities_never_treat_streams_without_acl_as_private() {
        assert_eq!(mode(0).unwrap(), Mode::ExclusiveSingleStream);
        assert_eq!(mode(8).unwrap(), Mode::PrivateAcl);
        assert_eq!(mode(8 | 0x40000).unwrap(), Mode::PrivateAcl);
        assert!(mode(0x40000).is_err());
    }
    #[test]
    fn private_permission_snapshot_rejects_untrusted_owner_or_changed_dacl() {
        #[link(name = "Advapi32")]
        extern "system" {
            fn GetSecurityDescriptorOwner(
                descriptor: Handle,
                owner: *mut Handle,
                defaulted: *mut i32,
            ) -> i32;
        }
        let user = current_sid(&effective_token().unwrap()).unwrap();
        let acl = format!("(A;;FA;;;{user})(A;;FA;;;SY)(A;;FA;;;BA)");
        let expected = private_permissions(&user).unwrap();
        for (label, sddl, accepted) in [
            ("user", format!("O:{user}D:P{acl}"), true),
            ("system", format!("O:SYD:P{acl}"), true),
            ("administrators", format!("O:BAD:P{acl}"), true),
            ("other owner", format!("O:BUD:P{acl}"), false),
            ("missing owner", format!("D:P{acl}"), false),
            ("missing DACL", format!("O:{user}"), false),
            ("null DACL", format!("O:{user}D:PNO_ACCESS_CONTROL"), false),
            ("empty DACL", format!("O:{user}D:P"), false),
            ("unprotected DACL", format!("O:{user}D:{acl}"), false),
            (
                "extra reader",
                format!("O:{user}D:P{acl}(A;;GR;;;BU)"),
                false,
            ),
        ] {
            let wide: Vec<_> = sddl.encode_utf16().chain(Some(0)).collect();
            let mut raw = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        wide.as_ptr(),
                        1,
                        &mut raw,
                        std::ptr::null_mut(),
                    )
                },
                0,
                "fixture {label}: {}",
                io::Error::last_os_error()
            );
            let memory = Allocation(raw);
            let (mut owner, mut defaulted) = (std::ptr::null_mut(), 0);
            assert_ne!(
                unsafe { GetSecurityDescriptorOwner(memory.0, &mut owner, &mut defaulted) },
                0
            );
            assert_eq!(
                private_permissions_match(memory.0, owner, expected.0, &user).unwrap_or(false),
                accepted,
                "{label}"
            );
        }
    }
    #[test]
    fn direct_output_commit_and_private_failure_preserve_contract() {
        for foreign_after in [false,true] {
            let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
            let path=root.path().join("final.txt");let journal=crate::engine::PublicationJournal::default();
            let mut output=create_direct_output(root.path(),&parent,OsStr::new("final.txt")).unwrap();
            assert!(path.exists());assert!(File::open(&path).is_err());
            output.as_file_mut().write_all(b"complete verified bytes").unwrap();
            let returned=output.commit_owned(&path,&journal).unwrap();drop(returned);
            assert!(File::open(&path).is_err(),"journal must keep exclusive data handle");
            if foreign_after {
                let mut ads=File::options().write(true).create_new(true).share_mode(7)
                    .open(format!("{}:foreign",path.display())).unwrap();
                ads.write_all(b"keep this change").unwrap();drop(ads);
                assert!(!journal.rollback_for_output_test().is_empty());
                assert_eq!(fs::read(format!("{}:foreign",path.display())).unwrap(),b"keep this change");
            } else {
                drop(journal);
                assert_eq!(fs::read(&path).unwrap(),b"complete verified bytes");
            }
        }
        let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
        let path=root.path().join("private.txt");let journal=crate::engine::PublicationJournal::default();
        let mut output=create_direct_output(root.path(),&parent,OsStr::new("private.txt")).unwrap();
        output.as_file_mut().write_all(b"incomplete").unwrap();
        let mut ads=File::options().write(true).create_new(true).share_mode(7)
            .open(format!("{}:foreign",path.display())).unwrap();ads.write_all(b"uncommitted").unwrap();drop(ads);
        assert!(output.commit_owned(&path,&journal).is_err());
        output.abort().unwrap();assert!(!path.exists(),"private abort matches existing scratch contract");
    }
    #[test]
    fn direct_output_collision_links_revocation_and_cancel_keep_others() {
        let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
        let outside=root.path().join("keep");fs::write(&outside,b"original").unwrap();
        fs::hard_link(&outside,root.path().join("collision")).unwrap();
        assert!(create_direct_output(root.path(),&parent,OsStr::new("collision")).is_err());
        assert_eq!(fs::read(&outside).unwrap(),b"original");
        for (index,step) in [Step::BeforeCommit,Step::ApplyPermissions].iter().enumerate() {
            let name=format!("cancel-{index}");let path=root.path().join(&name);
            let journal=crate::engine::PublicationJournal::default();
            let mut output=create_direct_output(root.path(),&parent,OsStr::new(&name)).unwrap();
            output.as_file_mut().write_all(b"complete").unwrap();
            assert!(output.commit_journaled(&path,false,Some(&journal),&mut |current| {
                if current==*step {Err("cancelled".into())}else{Ok(())}
            },None).is_err());
            output.abort().unwrap();assert!(!path.exists());
        }
        let user=current_sid(&effective_token().unwrap()).unwrap();
        policy(&parent,&format!("D:PAI(A;OICI;FA;;;{user})(A;OICI;GR;;;BU)"));
        let path=root.path().join("revoked");let journal=crate::engine::PublicationJournal::default();
        let mut output=create_direct_output(root.path(),&parent,OsStr::new("revoked")).unwrap();
        output.as_file_mut().write_all(b"complete").unwrap();
        policy(&parent,&format!("D:PAI(A;OICI;FA;;;{user})"));
        assert!(output.commit_owned(&path,&journal).unwrap_err().contains("permissions changed"));
        output.abort().unwrap();assert!(!path.exists());assert_eq!(fs::read(&outside).unwrap(),b"original");
    }
    #[test]
    fn direct_output_matches_native_acls_and_late_revocation() {
        let root=tempfile::tempdir().unwrap();let parent=open_parent(root.path());
        let user=current_sid(&effective_token().unwrap()).unwrap();
        for (index,extra) in ["(A;OICI;GR;;;BU)","(A;OICINP;GR;;;BU)","(D;OICI;GW;;;BG)(A;OICI;GR;;;BU)"].iter().enumerate() {
            policy(&parent,&format!("D:PAI(A;OICI;FA;;;{user})(A;OICI;FA;;;SY){extra}"));
            let normal=File::options().read(true).write(true).create_new(true)
                .open(root.path().join(format!("native-{index}"))).unwrap();
            let name=format!("direct-{index}");let path=root.path().join(&name);
            let journal=crate::engine::PublicationJournal::default();
            let mut output=create_direct_output(root.path(),&parent,OsStr::new(&name)).unwrap();
            output.as_file_mut().write_all(b"complete").unwrap();
            let returned=output.commit_owned(&path,&journal).unwrap();
            assert_eq!(security(&returned),security(&normal));drop(returned);
            assert!(journal.rollback_for_output_test().is_empty());assert!(!path.exists());
        }
        let name=OsStr::new("late-revoke");let path=root.path().join(name);
        let journal=crate::engine::PublicationJournal::default();
        let mut output=create_direct_output(root.path(),&parent,name).unwrap();
        output.as_file_mut().write_all(b"complete").unwrap();
        let reason=output.commit_journaled(&path,false,Some(&journal),&mut |step| {
            if step==Step::ApplyPermissions {policy(&parent,&format!("D:PAI(A;OICI;FA;;;{user})"));}
            Ok(())
        },None).unwrap_err();
        assert!(reason.contains("permissions changed"));output.abort().unwrap();assert!(!path.exists());
    }
    fn direct_checkpoint(name:&str) {
        if std::env::var("QING_TEST_DIRECT_CHECKPOINT").ok().as_deref()==Some(name) {
            fs::write(Path::new(&std::env::var("QING_TEST_DIRECT_ROOT").unwrap()).join("ready"),name).unwrap();
            loop {std::thread::sleep(Duration::from_millis(10));}
        }
    }
    #[test]
    fn direct_crash_child() {
        let Ok(root)=std::env::var("QING_TEST_DIRECT_ROOT") else{return};
        let root=Path::new(&root);let parent=open_parent(root);let path=root.join("final.txt");
        let journal=crate::engine::PublicationJournal::default();
        let mut output=create_direct_output(root,&parent,OsStr::new("final.txt")).unwrap();
        direct_checkpoint("created");
        output.as_file_mut().write_all(b"partial").unwrap();direct_checkpoint("partial");
        output.as_file_mut().write_all(b"-complete").unwrap();
        let mut attributes=output.as_file().metadata().unwrap().permissions();attributes.set_readonly(true);
        output.as_file().set_permissions(attributes).unwrap();
        BEFORE_DIRECT_CLEAR.with(|hook|*hook.borrow_mut()=Some(Box::new(||direct_checkpoint("before-clear"))));
        let returned=output.commit_owned(&path,&journal).unwrap();drop(returned);
        direct_checkpoint("committed");drop(journal);direct_checkpoint("released");
    }
    #[test]
    fn direct_output_process_termination_never_leaves_incomplete_final_name() {
        for checkpoint in ["created","partial","before-clear","committed","released"] {
            let root=tempfile::tempdir().unwrap();
            let mut child=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","private_stage::output::tests::direct_crash_child","--nocapture"])
                .env("QING_TEST_DIRECT_ROOT",root.path()).env("QING_TEST_DIRECT_CHECKPOINT",checkpoint)
                .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
            let started=std::time::Instant::now();
            while !root.path().join("ready").exists() && started.elapsed()<Duration::from_secs(10) {
                if child.try_wait().unwrap().is_some(){break;}
                std::thread::sleep(Duration::from_millis(5));
            }
            let reached=root.path().join("ready").exists();let _=child.kill();child.wait().unwrap();
            assert!(reached,"child failed to reach {checkpoint}");
            let path=root.path().join("final.txt");
            if matches!(checkpoint,"committed"|"released") {
                assert_eq!(fs::read(&path).unwrap(),b"partial-complete");
                let mut attributes=fs::metadata(&path).unwrap().permissions();attributes.set_readonly(false);fs::set_permissions(&path,attributes).unwrap();
            } else {assert!(!path.exists(),"incomplete final survived at {checkpoint}");}
        }
    }
    #[test]
    fn live_parent_query_matches_win32_for_large_and_changed_dacls() {
        let root = tempfile::tempdir().unwrap();
        let parent = open_parent(root.path());
        let token = effective_token().unwrap();
        let user = current_sid(&token).unwrap();
        for large in [false, true, false] {
            let mut acl = format!("D:PAI(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)");
            if large {
                for index in 1000..1300 {
                    acl.push_str(&format!("(A;OICI;GR;;;S-1-5-21-123-456-789-{index})"));
                }
            }
            policy(&parent, &acl);
            let original = descriptor(&parent).unwrap();
            if large { assert!(acl_bytes(original.0).unwrap().len() > 4096); }
            for directory in [false, true] {
                let expected = ordinary_permissions_from_descriptor(original.0, &token, directory).unwrap();
                let actual = ordinary_permissions_kind(&parent, &token, directory).unwrap();
                assert!(same_file_permissions(expected.0, actual.0).unwrap());
            }
        }
        let limited = fs::OpenOptions::new().access_mode(0x80).share_mode(3)
            .custom_flags(0x02200000).open(root.path()).unwrap();
        assert!(ordinary_permissions_kind(&limited, &token, false).is_err(),
            "missing READ_CONTROL must not be bypassed by fallback");
    }
    #[test]
    fn private_output_reads_changed_parent_permissions_for_each_file() {
        let root = tempfile::tempdir().unwrap();
        let parent = open_parent(root.path());
        let user = current_sid(&effective_token().unwrap()).unwrap();
        let common = format!("(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        let mut first_security = None;
        for (index, extra) in ["(A;OICI;GR;;;BU)", "(A;OICI;GR;;;WD)"].iter().enumerate() {
            policy(&parent, &format!("D:PAI{common}{extra}"));
            let ordinary_path = root.path().join(format!("ordinary-{index}.txt"));
            let ordinary = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&ordinary_path)
                .unwrap();
            let mut output = create_output(root.path(), &parent).unwrap();
            assert_private(output.as_file());
            let committed = output
                .commit(&root.path().join(format!("final-{index}.txt")), false)
                .unwrap();
            let observed = security(&committed);
            assert_eq!(observed, security(&ordinary));
            if let Some(first) = first_security.as_ref() {
                assert_ne!(
                    &observed, first,
                    "each file must derive the current parent DACL"
                );
            } else {
                first_security = Some(observed);
            }
        }
    }
    #[test]
    fn private_output_matches_nine_ordinary_acl_models() {
        let root = tempfile::tempdir().unwrap();
        let user = current_sid(&effective_token().unwrap()).unwrap();
        let common = format!("(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        let policies = [
            format!("D:PAI{common}(A;OICI;GR;;;BU)"),
            format!("D:P{common}(A;OICI;GR;;;BU)"),
            format!("D:PAI{common}(A;OICINP;GR;;;BU)"),
            format!("D:PAI{common}(A;OICI;FA;;;CO)(A;OICI;GR;;;BU)"),
            format!("D:P{common}(A;OICI;FR;;;OW)"),
            format!("D:PAI(D;OICI;WD;;;BG){common}(A;OICI;GR;;;BU)"),
            format!("D:P(A;;FA;;;{user})(A;;FA;;;SY)"),
            "D:NO_ACCESS_CONTROL".into(),
            format!("D:PAI{common}(XA;OICI;FR;;;WD;(Member_of {{SID(S-1-5-32-545)}}))"),
        ];
        for (index, sddl) in policies.iter().enumerate() {
            let path = root.path().join(index.to_string());
            fs::create_dir(&path).unwrap();
            let parent = open_parent(&path);
            policy(&parent, sddl);
            let ordinary = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path.join("ordinary.txt"))
                .unwrap();
            let mut output = create_output(&path, &parent).unwrap();
            assert_private(output.as_file());
            let scratch = output.path().to_path_buf();
            output.as_file_mut().write_all(b"verified").unwrap();
            // Same-owner writes are denied by stream sharing as well.
            assert!(fs::OpenOptions::new().write(true).open(&scratch).is_err());
            let committed = output.commit(&path.join("final.txt"), false).unwrap();
            fs::rename(path.join("ordinary.txt"), path.join("ordinary-renamed.txt")).unwrap();
            assert_eq!(
                security(&ordinary),
                security(&committed),
                "ACL model {index}"
            );
            assert_eq!(
                stored_security(&path.join("ordinary-renamed.txt")),
                stored_security(&path.join("final.txt")),
                "stored ACL model {index}"
            );
            assert!(!scratch.exists());
            assert_eq!(fs::read(path.join("final.txt")).unwrap(), b"verified");
        }
    }
    #[test]
    fn private_output_complex_nested_matches_native_creation_and_rename() {
        let root = tempfile::tempdir().unwrap();
        let parent = open_parent(root.path());
        policy(
            &parent,
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)(A;OICIIO;GR;;;CO)(A;CINP;GR;;;SY)",
        );
        let nested = root.path().join("nested/deep");
        fs::create_dir_all(&nested).unwrap();
        let nested_parent = open_parent(&nested);
        let ordinary = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .access_mode(0xc0010000)
            .share_mode(7)
            .open(nested.join("ordinary.txt"))
            .unwrap();
        let created = security(&ordinary);
        fs::rename(
            nested.join("ordinary.txt"),
            nested.join("ordinary-moved.txt"),
        )
        .unwrap();
        assert_eq!(security(&ordinary), created, "Win32 same-parent rename");
        rename(
            &ordinary,
            &nested_parent,
            &leaf(OsStr::new("ordinary-native.txt")).unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(security(&ordinary), created, "native same-parent rename");
        let absolute: Vec<u16> = nested
            .join("ordinary-legacy.txt")
            .as_os_str()
            .encode_wide()
            .collect();
        let offset = std::mem::offset_of!(RenameInformation, name);
        let size = offset + (absolute.len() + 1) * 2;
        let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            let header = buffer.as_mut_ptr().cast::<RenameInformation>();
            (*header).length = (absolute.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(
                absolute.as_ptr().cast::<u8>(),
                buffer.as_mut_ptr().cast::<u8>().add(offset),
                absolute.len() * 2,
            );
            assert_ne!(
                SetFileInformationByHandle(
                    ordinary.as_raw_handle(),
                    3,
                    buffer.as_ptr().cast(),
                    size as u32
                ),
                0
            );
        }
        assert_eq!(
            security(&ordinary),
            created,
            "previous output publisher same-parent rename"
        );
        let mut output = create_output(&nested, &nested_parent).unwrap();
        let file = output
            .commit(&nested.join("private-final.txt"), false)
            .unwrap();
        assert_eq!(
            security(&file),
            created,
            "private output must match ordinary creation and publication"
        );
    }
    #[test]
    fn private_output_commit_failures_preserve_boundary_and_target() {
        for replace in [false, true] {
            for failure in [
                Step::BeforeCommit,
                Step::AfterCommit,
                Step::ApplyPermissions,
            ] {
                let root = tempfile::tempdir().unwrap();
                let parent = open_parent(root.path());
                let target = root.path().join("final.txt");
                if replace {
                    fs::write(&target, b"old").unwrap();
                }
                let mut output = create_output(root.path(), &parent).unwrap();
                let scratch = output.path().to_path_buf();
                output.as_file_mut().write_all(b"new").unwrap();
                let message = output
                    .commit_with(&target, replace, &mut |step| {
                        if step == failure {
                            Err("injected failure".into())
                        } else {
                            Ok(())
                        }
                    })
                    .unwrap_err();
                if failure != Step::AfterCommit {
                    assert_private(output.as_file());
                    assert!(!message.contains("was committed"));
                    if replace {
                        assert_eq!(fs::read(&target).unwrap(), b"old");
                    } else {
                        assert!(!target.exists());
                    }
                    output.abort().unwrap();
                    assert!(!scratch.exists());
                } else {
                    assert!(message.contains("was committed"));
                    output.abort().unwrap(); // Postcommit abort MUST NOT remove the target.
                    assert_eq!(fs::read(&target).unwrap(), b"new");
                    assert!(!scratch.exists());
                    drop(output);
                    assert_eq!(fs::read(&target).unwrap(), b"new");
                }
            }
        }
    }
    #[test]
    fn revoking_inherited_read_before_or_during_finalization_preserves_existing_contents() {
        let user = current_sid(&effective_token().unwrap()).unwrap();
        let base = format!("D:PAI(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
        for replace in [false, true] {
            for revoke in [Step::BeforeCommit, Step::ApplyPermissions] {
                let root = tempfile::tempdir().unwrap();
                let parent = open_parent(root.path());
                policy(&parent, &format!("{base}(A;OICI;FR;;;LS)"));
                let target = root.path().join("final.txt");
                if replace {
                    fs::write(&target, b"original").unwrap();
                }
                let mut output = create_output(root.path(), &parent).unwrap();
                output.as_file_mut().write_all(b"unpublished").unwrap();
                let scratch = output.path().to_owned();
                let error = output
                    .commit_with(&target, replace, &mut |step| {
                        if step == revoke {
                            policy(&parent, &base);
                        }
                        Ok(())
                    })
                    .unwrap_err();
                assert!(error.contains("permissions changed"));
                assert_private(output.as_file());
                output.abort().unwrap();
                assert!(!scratch.exists());
                if replace {
                    assert_eq!(fs::read(&target).unwrap(), b"original");
                } else {
                    assert!(!target.exists());
                }
            }
        }
    }
    #[test]
    fn private_output_failed_cleanup_reports_private_residual_identity() {
        let root = tempfile::tempdir().unwrap();
        let parent = open_parent(root.path());
        let mut output = create_output(root.path(), &parent).unwrap();
        output.as_file_mut().write_all(b"unpublished").unwrap();
        let path = output.path().to_owned();
        let error = output
            .abort_with(&mut || Err("injected delete failure".into()))
            .unwrap_err();
        assert!(error.contains(&path.display().to_string()));
        assert!(error.contains("cleanup private output failed"));
        assert_private(output.as_file());
        // The private writer also denies separate opens of its default stream.
        assert!(fs::read(&path).is_err());
        let mut contents = [0; 11];
        assert_eq!(
            std::os::windows::fs::FileExt::seek_read(output.as_file(), &mut contents, 0).unwrap(),
            contents.len()
        );
        assert_eq!(&contents, b"unpublished");
        output.abort().unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn private_output_collision_cancel_readonly_and_ads_keep_exact_metadata() {
        let root = tempfile::tempdir().unwrap();
        let parent = open_parent(root.path());
        let old = root.path().join("old.txt");
        fs::write(&old, b"preserve").unwrap();
        let mut output = create_output(root.path(), &parent).unwrap();
        let scratch = output.path().to_path_buf();
        output.as_file_mut().write_all(b"new").unwrap();
        assert!(output.commit(&old, false).is_err());
        assert_eq!(fs::read(&old).unwrap(), b"preserve");
        let mut attributes = output.as_file().metadata().unwrap().permissions();
        attributes.set_readonly(true);
        output.as_file().set_permissions(attributes).unwrap();
        output.abort().unwrap();
        assert!(!scratch.exists());

        let source = root.path().join("marked.zip");
        fs::write(&source, b"original archive").unwrap();
        let zone = b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.test/archive.zip\r\n";
        fs::write(format!("{}:Zone.Identifier", source.display()), zone).unwrap();
        let marker = crate::motw::archive_context(&source).unwrap();
        let modified =
            UNIX_EPOCH + Duration::from_secs(1_577_934_245) + Duration::from_nanos(123_456_700);
        let mut output = create_output(root.path(), &parent).unwrap();
        output.as_file_mut().write_all(b"verified").unwrap();
        crate::motw::apply_to_file(output.as_file(), Some(&marker)).unwrap();
        output
            .as_file()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let mut attributes = output.as_file().metadata().unwrap().permissions();
        attributes.set_readonly(true);
        output.as_file().set_permissions(attributes).unwrap();
        let target = root.path().join("readonly.txt");
        let committed = output.commit(&target, false).unwrap();
        assert_eq!(committed.metadata().unwrap().modified().unwrap(), modified);
        assert!(committed.metadata().unwrap().permissions().readonly());
        assert_eq!(
            fs::read(format!("{}:Zone.Identifier", target.display())).unwrap(),
            zone
        );
        assert_eq!(fs::read(&source).unwrap(), b"original archive");
        let mut collision = create_output(root.path(), &parent).unwrap();
        collision
            .as_file_mut()
            .write_all(b"cannot overwrite readonly")
            .unwrap();
        assert!(collision.commit(&target, true).is_err());
        collision.abort().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"verified");
        let mut attrs = committed.metadata().unwrap().permissions();
        attrs.set_readonly(false);
        committed.set_permissions(attrs).unwrap();
    }
    #[test]
    fn private_output_keeps_target_storage_inheritance() {
        #[link(name = "Kernel32")]
        extern "system" {
            fn DeviceIoControl(
                file: Handle,
                code: u32,
                input: *const u8,
                input_len: u32,
                output: *mut u8,
                output_len: u32,
                count: *mut u32,
                overlapped: Handle,
            ) -> i32;
        }
        let root = tempfile::tempdir().unwrap();
        for (index, compressed) in [false, true].into_iter().enumerate() {
            let path = root.path().join(index.to_string());
            fs::create_dir(&path).unwrap();
            let storage = fs::OpenOptions::new()
                .access_mode(0x17019f)
                .share_mode(3)
                .custom_flags(0x02200000)
                .open(&path)
                .unwrap();
            let mut basic = [0u64; 5];
            basic[4] = 0x2010;
            assert_ne!(
                unsafe {
                    SetFileInformationByHandle(
                        storage.as_raw_handle(),
                        0,
                        basic.as_ptr().cast(),
                        40,
                    )
                },
                0
            );
            if compressed {
                let compression = 1u16;
                let mut returned = 0;
                assert_ne!(
                    unsafe {
                        DeviceIoControl(
                            storage.as_raw_handle(),
                            0x9c040,
                            (&compression as *const u16).cast(),
                            2,
                            std::ptr::null_mut(),
                            0,
                            &mut returned,
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
            }
            drop(storage);
            let parent = open_parent(&path);
            let ordinary = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path.join("ordinary.txt"))
                .unwrap();
            let mut output = create_output(&path, &parent).unwrap();
            output.as_file_mut().write_all(b"storage").unwrap();
            assert_eq!(
                output.as_file().metadata().unwrap().file_attributes(),
                ordinary.metadata().unwrap().file_attributes()
            );
            let committed = output.commit(&path.join("final.txt"), false).unwrap();
            let attrs = committed.metadata().unwrap().file_attributes();
            assert_eq!(attrs, ordinary.metadata().unwrap().file_attributes());
            assert_ne!(attrs & 0x2000, 0); // old A-only publisher lost this bit
            assert_eq!(attrs & 0x800 != 0, compressed);
        }
    }
}
