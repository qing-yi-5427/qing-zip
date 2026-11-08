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
    fn SetFileInformationByHandle(file: Handle, class: i32, info: *const c_void, size: u32) -> i32;
}
#[link(name = "Advapi32")]
extern "system" {
    fn OpenThreadToken(thread: Handle, access: u32, as_self: i32, token: *mut Handle) -> i32;
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
fn control(sd: Handle) -> Result<u16, String> {
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
fn ordinary_permissions(parent: &File, token: &Token) -> Result<PrivateDescriptor, String> {
    let policy = descriptor(parent)?;
    let automatic = control(policy.0)? & 0x400 != 0;
    // Windows interprets conditional, no-propagate, creator-owner and default
    // token ACEs. Request the parent's actual inheritance model at derivation,
    // not just in the resulting control bits: requesting AI for a legacy parent
    // changes stored ACE inheritance flags even if AI is subsequently cleared.
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
            policy.0,
            std::ptr::null_mut(),
            &mut raw,
            std::ptr::null_mut(),
            0,
            u32::from(automatic),
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
fn same_file_permissions(a: Handle, b: Handle) -> Result<bool, String> {
    // Allocation slack after the last ACE has no access semantics. Keep every
    // ACE byte, their order, and all protection/inheritance control flags.
    Ok((control(a)? ^ control(b)?) & 0x1400 == 0
        && acl_entries(&acl_bytes(a)?)? == acl_entries(&acl_bytes(b)?)?)
}
fn only_default_stream(file: &File) -> Result<bool, String> {
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
            Ok(false)
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
    let name = "::$DATA"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    Ok(bytes[..4] == [0; 4]
        && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize == name.len()
        && bytes[24..24 + name.len()] == name)
}

fn private_permissions(sid: &str) -> Result<Allocation, String> {
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
fn info(file: &File) -> Result<[u32; 13], String> {
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

pub(crate) struct PrivateOutputFile {
    file: Option<File>,
    parent: File,
    path: PathBuf,
    permissions: Option<PrivateDescriptor>,
    state: State,
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
        if self.state != State::Private {
            return Err("output is no longer private".into());
        }
        if target.parent() != self.path.parent() {
            return Err("output parent changed".into());
        }
        let name = leaf(target.file_name().ok_or("missing output leaf")?)?;
        let current = info(self.as_file())?;
        if current[0] & 0x410 != 0 || current[10] != 1 {
            return Err("private output identity changed".into());
        }
        if info(&self.parent)?[0] & 0x410 != 0x10 {
            return Err("output parent changed".into());
        }
        step(Step::BeforeCommit)?;
        rename(self.as_file(), &self.parent, &name, replace)?;
        // This is the commit boundary. No error below may delete the target,
        // retry a copy, or cause the source archive to be recycled as success.
        self.state = State::Committed;
        self.path = target.to_owned();
        step(Step::AfterCommit)
            .and_then(|_| step(Step::ApplyPermissions))
            .and_then(|_| {
                if let Some(permissions) = &self.permissions {
                    apply_native(self.as_file(), permissions)
                } else {
                    Ok(())
                }
            })
            .map_err(|e| {
                format!(
                    "output was committed but permission restoration failed: {}: {e}",
                    target.display()
                )
            })?;
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
        let file = self.as_file();
        let attrs = file
            .metadata()
            .map_err(|e| {
                format!(
                    "cleanup private output failed: {}: query attributes: {e}",
                    self.path.display()
                )
            })?
            .file_attributes();
        if attrs & 1 != 0 {
            let mut basic = [0u64; 5];
            // Preserve inherited storage flags; clear only readonly.
            basic[4] = u64::from(if attrs & !1 == 0 { 0x80 } else { attrs & !1 });
            if unsafe {
                SetFileInformationByHandle(file.as_raw_handle(), 0, basic.as_ptr().cast(), 40)
            } == 0
            {
                return Err(format!(
                    "cleanup private output failed: {}: {}",
                    self.path.display(),
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
                self.path.display(),
                nt_error("delete owned output", result)
            ));
        }
        self.state = State::Aborted;
        self.file.take();
        Ok(())
    }
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
    create_with_sharing(parent_path, parent, false)
}
pub(super) fn create_archive_file(
    parent_path: &Path,
    parent: &File,
) -> Result<PrivateOutputFile, String> {
    create_with_sharing(parent_path, parent, true)
}
fn create_with_sharing(
    parent_path: &Path,
    parent: &File,
    exclusive: bool,
) -> Result<PrivateOutputFile, String> {
    if info(parent)?[0] & 0x410 != 0x10 {
        return Err("invalid private output parent".into());
    }
    let mode = volume_mode(parent)?;
    let token = effective_token()?;
    let permissions = if mode == Mode::PrivateAcl {
        Some(ordinary_permissions(parent, &token)?)
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
        let mut random = [0u8; 16];
        if unsafe { BCryptGenRandom(std::ptr::null_mut(), random.as_mut_ptr(), 16, 2) } < 0 {
            return Err("cannot generate private output name".into());
        }
        let suffix = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let name = format!(".qing-zip-output-{suffix}");
        let mut wide: Vec<u16> = name.encode_utf16().collect();
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
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                0x17019f,
                &attributes,
                &mut status,
                std::ptr::null(),
                0x80,
                if mode == Mode::PrivateAcl && !exclusive {
                    1
                } else {
                    0
                },
                2,
                0x200060,
                std::ptr::null_mut(),
                0,
            )
        };
        if result < 0 {
            let code = unsafe { RtlNtStatusToDosError(result) };
            if matches!(code, 80 | 183) {
                continue;
            }
            return Err(nt_error("create private output", result));
        }
        let mut output = PrivateOutputFile {
            file: Some(unsafe { File::from_raw_handle(handle) }),
            parent: parent_pin,
            path: parent_path.join(name),
            permissions,
            state: State::Private,
        };
        let validation = (|| {
            let actual = info(output.as_file())?;
            if actual[0] & 0x410 != 0 || actual[10] != 1 {
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
                assert_private(output.as_file());
                if failure == Step::BeforeCommit {
                    assert!(!message.contains("was committed"));
                    if replace {
                        assert_eq!(fs::read(&target).unwrap(), b"old");
                    } else {
                        assert!(!target.exists());
                    }
                    output.abort().unwrap();
                    assert!(!scratch.exists());
                } else {
                    assert!(message.contains("was committed but permission restoration failed"));
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
        assert_eq!(fs::read(&path).unwrap(), b"unpublished");
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
