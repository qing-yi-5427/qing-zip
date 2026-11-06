//! Same-volume staging is created privately and pinned in one NtCreateFile call.
//! Only after the verified directory is committed may the caller apply normal
//! inherited permissions through its pinned directory handle.
#![cfg(windows)]
use std::{
    ffi::{c_void, OsStr},
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
};
type Handle = *mut c_void;
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: Handle,
    name: *mut UnicodeString,
    attributes: u32,
    descriptor: Handle,
    qos: Handle,
}
#[repr(C)]
struct IoStatus {
    status: usize,
    information: usize,
}
#[repr(C)]
struct GenericMapping {
    read: u32,
    write: u32,
    execute: u32,
    all: u32,
}
#[link(name = "Advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        buffer: Handle,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        size: *mut u32,
    ) -> i32;
    fn GetSecurityInfo(
        handle: Handle,
        kind: u32,
        info: u32,
        owner: *mut Handle,
        group: *mut Handle,
        dacl: *mut Handle,
        sacl: *mut Handle,
        descriptor: *mut Handle,
    ) -> u32;
    fn GetSecurityDescriptorDacl(
        descriptor: Handle,
        present: *mut i32,
        dacl: *mut Handle,
        defaulted: *mut i32,
    ) -> i32;
    fn GetSecurityDescriptorControl(
        descriptor: Handle,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
    fn SetSecurityDescriptorControl(descriptor: Handle, mask: u16, control: u16) -> i32;
    fn GetAce(acl: Handle, index: u32, ace: *mut Handle) -> i32;
    #[cfg(test)]
    fn GetSecurityDescriptorLength(descriptor: Handle) -> u32;
    fn CreatePrivateObjectSecurityEx(
        parent: Handle,
        creator: Handle,
        new: *mut Handle,
        kind: Handle,
        container: i32,
        flags: u32,
        token: Handle,
        mapping: *mut GenericMapping,
    ) -> i32;
    fn DestroyPrivateObjectSecurity(descriptor: *mut Handle) -> i32;
    fn SetSecurityInfo(
        handle: Handle,
        kind: u32,
        info: u32,
        owner: Handle,
        group: Handle,
        dacl: Handle,
        sacl: Handle,
    ) -> u32;
}
#[link(name = "Kernel32")]
extern "system" {
    fn CloseHandle(handle: Handle) -> i32;
    fn LocalFree(memory: Handle) -> Handle;
}
#[link(name = "ntdll")]
extern "system" {
    fn NtCreateFile(
        handle: *mut Handle,
        access: u32,
        attributes: *const ObjectAttributes,
        status: *mut IoStatus,
        allocation_size: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: Handle,
        ea_length: u32,
    ) -> i32;
    fn NtSetInformationFile(
        handle: Handle,
        status: *mut usize,
        info: *const c_void,
        length: u32,
        class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
    fn NtSetSecurityObject(handle: Handle, information: u32, descriptor: Handle) -> i32;
}
#[link(name = "Bcrypt")]
extern "system" {
    fn BCryptGenRandom(algorithm: Handle, buffer: *mut u8, len: u32, flags: u32) -> i32;
}
struct Allocation(Handle);
impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct Token(Handle);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct PrivateDescriptor(Handle);
impl Drop for PrivateDescriptor {
    fn drop(&mut self) {
        unsafe {
            DestroyPrivateObjectSecurity(&mut self.0);
        }
    }
}
fn error(operation: &str) -> String {
    format!("{operation}: {}", io::Error::last_os_error())
}
fn token() -> Result<Token, String> {
    let mut result = std::ptr::null_mut();
    if unsafe { OpenProcessToken(-1isize as Handle, 8, &mut result) } == 0 {
        return Err(error("query stage token"));
    }
    Ok(Token(result))
}
fn current_sid(token: &Token) -> Result<String, String> {
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, 1, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 1024 * 1024 {
        return Err("invalid token user information".into());
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    if unsafe { GetTokenInformation(token.0, 1, buffer.as_mut_ptr().cast(), size, &mut size) } == 0
    {
        return Err(error("read stage token"));
    }
    let sid = unsafe { *(buffer.as_ptr().cast::<Handle>()) };
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(error("format stage owner"));
    }
    let _memory = Allocation(text.cast());
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(text, len)).map_err(|e| e.to_string())
    }
}
pub(crate) struct PrivateStage {
    path: Option<PathBuf>,
    parent: PathBuf,
    root: File,
    _parent_pin: File,
}
impl PrivateStage {
    pub(crate) fn path(&self) -> &Path {
        self.path.as_deref().expect("owned private stage")
    }
    pub(crate) fn keep(mut self) -> PathBuf {
        self.path.take().expect("owned private stage")
    }
    pub(crate) fn root(&self) -> &File {
        &self.root
    }
}
impl Drop for PrivateStage {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            if path.parent() == Some(self.parent.as_path())
                && path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(".qing-zip-stage-"))
            {
                // Root and parent identities remain pinned throughout cleanup.
                // Delete children while the root remains pinned, then delete
                // that exact root by handle, never after closing it.
                if let Ok(children) = std::fs::read_dir(path) {
                    for child in children.flatten() {
                        let path = child.path();
                        if path.is_dir() {
                            let _ = std::fs::remove_dir_all(path);
                        } else {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
                let mut status = IoStatus {
                    status: 0,
                    information: 0,
                };
                let mut delete = 1u8;
                unsafe {
                    NtSetInformationFile(
                        self.root.as_raw_handle(),
                        (&mut status as *mut IoStatus).cast(),
                        (&mut delete as *mut u8).cast(),
                        1,
                        13,
                    );
                }
            }
        }
    }
}
pub(crate) fn create(parent: &Path, parent_handle: &File) -> Result<PrivateStage, String> {
    let parent_pin = parent_handle.try_clone().map_err(|e| e.to_string())?;
    let token = token()?;
    let sid = current_sid(&token)?;
    let descriptor = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");
    let text: Vec<u16> = descriptor.encode_utf16().chain(Some(0)).collect();
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
        return Err(error("create private stage descriptor"));
    }
    let memory = Allocation(raw);
    for _ in 0..16 {
        let mut random = [0u8; 16];
        if unsafe { BCryptGenRandom(std::ptr::null_mut(), random.as_mut_ptr(), 16, 2) } < 0 {
            return Err("cannot generate stage name".into());
        }
        let name = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let leaf = format!(".qing-zip-stage-{name}");
        let path = parent.join(&leaf);
        let mut wide: Vec<u16> = leaf.encode_utf16().collect();
        let mut unicode = UnicodeString {
            length: (wide.len() * 2) as u16,
            maximum_length: (wide.len() * 2) as u16,
            buffer: wide.as_mut_ptr(),
        };
        let attributes = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root: parent_handle.as_raw_handle(),
            name: &mut unicode,
            attributes: 0x40,
            descriptor: memory.0,
            qos: std::ptr::null_mut(),
        };
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        // FILE_CREATE + private DACL + deny-write/delete pin are atomic. No
        // parent DELETE_CHILD actor can replace the directory before pinning.
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                0x170081,
                &attributes,
                &mut status,
                std::ptr::null(),
                0x10,
                1,
                2,
                0x200021,
                std::ptr::null_mut(),
                0,
            )
        };
        if result >= 0 {
            return Ok(PrivateStage {
                path: Some(path),
                parent: parent.to_owned(),
                root: unsafe { File::from_raw_handle(handle) },
                _parent_pin: parent_pin,
            });
        }
        let error = io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32);
        if error.raw_os_error() != Some(183) {
            return Err(format!("create private stage: {error}"));
        }
    }
    Err("could not reserve private stage directory".into())
}
/// Query one current leaf through a strongly pinned private directory. The
/// caller owns that qualification; no ancestor path is resolved by this call.
pub(crate) fn child_metadata(parent: &File, leaf: &OsStr) -> Result<std::fs::Metadata, String> {
    let mut wide: Vec<u16> = leaf.encode_wide().collect();
    if wide.is_empty()
        || wide.len() > 32767
        || wide.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
        || wide == [46]
        || wide == [46, 46]
    {
        return Err("invalid relative metadata leaf".into());
    }
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
        descriptor: std::ptr::null_mut(),
        qos: std::ptr::null_mut(),
    };
    let mut status = IoStatus {
        status: 0,
        information: 0,
    };
    let mut raw = std::ptr::null_mut();
    let result = unsafe {
        NtCreateFile(
            &mut raw,
            0x100081,
            &attributes,
            &mut status,
            std::ptr::null(),
            0,
            1,
            1,
            0x200020,
            std::ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(format!(
            "read relative stage metadata: {}",
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32)
        ));
    }
    let file = unsafe { File::from_raw_handle(raw) };
    checked_metadata(&file)
}
// Stable Rust does not expose Windows link counts on Metadata yet. Query the
// same fresh handle so no stage can reach metadata restoration or MOTW writes
// through a hard link to an unrelated file.
pub(crate) fn checked_metadata(file: &File) -> Result<std::fs::Metadata, String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.is_file() {
        #[link(name = "Kernel32")]
        extern "system" {
            fn GetFileInformationByHandle(file: Handle, information: *mut u32) -> i32;
        }
        let mut information = [0u32; 13];
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
            return Err(error("query decoded file links"));
        }
        if information[10] != 1 {
            return Err("hard linked decoded file rejected".into());
        }
    }
    Ok(metadata)
}
/// Root must be the committed, verified directory, pinned with WRITE_DAC and
/// READ_CONTROL. Every descendant must be pinned against writes/deletion until
/// this call returns. Parent must be its pinned destination parent. Never call before
/// commit: granting inherited access would expose the unverified staging tree.
fn descriptor(file: &File) -> Result<Allocation, String> {
    let mut descriptor = std::ptr::null_mut();
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            1,
            4,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if result != 0 {
        return Err(format!(
            "read destination permissions: {}",
            io::Error::from_raw_os_error(result as i32)
        ));
    }
    Ok(Allocation(descriptor))
}
/// Only the normal automatic, ordinary ACE model is restored on the fast path.
/// Legacy/no-propagate/object-specific policies keep native per-file creation.
pub(crate) fn supports_inheritance(parent: &File) -> Result<bool, String> {
    let memory = descriptor(parent)?;
    supports_descriptor(memory.0)
}
fn supports_descriptor(descriptor: Handle) -> Result<bool, String> {
    let (mut control, mut revision) = (0, 0);
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(error("read destination permission model"));
    }
    if control & 0x400 == 0 {
        return Ok(false);
    }
    let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
    if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(error("read destination ACL"));
    }
    if present == 0 || dacl.is_null() {
        return Ok(false);
    }
    let count = unsafe { std::ptr::read_unaligned((dacl as *const u8).add(4).cast::<u16>()) };
    if count > 256 {
        return Ok(false);
    }
    for i in 0..count {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(dacl, i as u32, &mut ace) } == 0 {
            return Err(error("read destination ACE"));
        }
        let bytes = ace as *const u8;
        if !matches!(unsafe { *bytes }, 0 | 1) || unsafe { *bytes.add(1) } & !0x1b != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}
fn derive(parent: Handle, container: bool, token: &Token) -> Result<PrivateDescriptor, String> {
    let mut computed = std::ptr::null_mut();
    let mut mapping = GenericMapping {
        read: 0x120089,
        write: 0x120116,
        execute: 0x1200a0,
        all: 0x1f01ff,
    };
    if unsafe {
        CreatePrivateObjectSecurityEx(
            parent,
            std::ptr::null_mut(),
            &mut computed,
            std::ptr::null_mut(),
            i32::from(container),
            1,
            token.0,
            &mut mapping,
        )
    } == 0
    {
        return Err(error("derive destination permissions"));
    }
    let computed = PrivateDescriptor(computed);
    // Native setters distinguish a requested automatic-inheritance update from
    // merely carrying the existing AI bit. Preserve the automatic model when
    // applying this already-derived descriptor to a previously protected file.
    if unsafe { SetSecurityDescriptorControl(computed.0, 0x500, 0x500) } == 0 {
        return Err(error("prepare automatic destination permissions"));
    }
    Ok(computed)
}
fn apply(root: &File, computed: &PrivateDescriptor) -> Result<(), String> {
    let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
    if unsafe { GetSecurityDescriptorDacl(computed.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(error("read inherited destination permissions"));
    }
    // Never accidentally grant Everyone full control by passing a NULL DACL.
    if present == 0 || dacl.is_null() {
        return Err("destination has no bounded inherited permissions".into());
    }
    let result = unsafe {
        SetSecurityInfo(
            root.as_raw_handle(),
            1,
            4 | 0x2000_0000,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        )
    };
    if result != 0 {
        return Err(format!(
            "apply inherited destination permissions: {}",
            io::Error::from_raw_os_error(result as i32)
        ));
    }
    Ok(())
}
// The complete descriptor (including AR/AI control bits) is already derived.
// Set it through the pinned handle without Win32's recursive inheritance pass.
fn apply_native(root: &File, computed: &PrivateDescriptor) -> Result<(), String> {
    let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
    if unsafe { GetSecurityDescriptorDacl(computed.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(error("read inherited destination permissions"));
    }
    if present == 0 || dacl.is_null() {
        return Err("destination has no bounded inherited permissions".into());
    }
    let status = unsafe { NtSetSecurityObject(root.as_raw_handle(), 4 | 0x2000_0000, computed.0) };
    if status < 0 {
        return Err(format!(
            "apply native inherited permissions: {}",
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
        ));
    }
    Ok(())
}
fn apply_compatible(root: &File, computed: &PrivateDescriptor) -> Result<(), String> {
    match apply_native(root, computed) {
        Ok(()) => Ok(()),
        // Preserve the proven Win32 behavior on filesystems that reject the
        // native update. Both routes use the same held identity and descriptor.
        Err(native) => apply(root, computed).map_err(|fallback| format!("{native}; {fallback}")),
    }
}
pub(crate) struct PermissionNode {
    pub(crate) file: File,
    pub(crate) parent_index: Option<usize>,
    pub(crate) is_dir: bool,
}
/// Nodes are parent-before-child, pinned READ_DATA|READ_CONTROL|WRITE_DAC with read-only
/// sharing. Never depend on SetSecurityInfo's silent best-effort propagation:
/// explicitly update every pinned object, using cached sibling descriptors.
pub(crate) fn inherit_tree(
    root: &File,
    parent: &File,
    nodes: &[PermissionNode],
) -> Result<(), String> {
    inherit_tree_with(root, parent, nodes, apply_compatible)
}
fn inherit_tree_with(
    root: &File,
    parent: &File,
    nodes: &[PermissionNode],
    mut apply: impl FnMut(&File, &PrivateDescriptor) -> Result<(), String>,
) -> Result<(), String> {
    let prepared = prepare_permissions(parent, nodes)?;
    apply(root, &prepared.root)?;
    for (node, permission) in nodes.iter().zip(prepared.nodes) {
        apply(&node.file, &permission)?;
    }
    Ok(())
}
struct PreparedPermissions {
    root: std::rc::Rc<PrivateDescriptor>,
    nodes: Vec<std::rc::Rc<PrivateDescriptor>>,
}
fn prepare_permissions(
    parent: &File,
    nodes: &[PermissionNode],
) -> Result<PreparedPermissions, String> {
    let token = token()?;
    let root_parent = descriptor(parent)?;
    if !supports_descriptor(root_parent.0)? {
        return Err("destination permission model changed before publication".into());
    }
    let root_permissions = std::rc::Rc::new(derive(root_parent.0, true, &token)?);
    let mut cache = std::collections::HashMap::new();
    let mut permissions: Vec<std::rc::Rc<PrivateDescriptor>> = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        if node
            .parent_index
            .is_some_and(|p| p >= index || !nodes[p].is_dir)
        {
            return Err("invalid permission tree order".into());
        }
        if !cache.contains_key(&node.parent_index) {
            let parent = node
                .parent_index
                .map_or(&root_permissions, |i| &permissions[i]);
            cache.insert(
                node.parent_index,
                (
                    std::rc::Rc::new(derive(parent.0, true, &token)?),
                    std::rc::Rc::new(derive(parent.0, false, &token)?),
                ),
            );
        }
        let pair = &cache[&node.parent_index];
        permissions.push(if node.is_dir {
            pair.0.clone()
        } else {
            pair.1.clone()
        });
    }
    // Derive from one immutable policy snapshot before granting public access.
    Ok(PreparedPermissions {
        root: root_permissions,
        nodes: permissions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::fs::OpenOptionsExt};
    fn open(path: &Path) -> File {
        fs::OpenOptions::new()
            .access_mode(0x60080)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(path)
            .unwrap()
    }
    fn dacl(path: &Path) -> Vec<u8> {
        let file = open(path);
        let mut descriptor = std::ptr::null_mut();
        let mut acl = std::ptr::null_mut();
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    1,
                    4,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut acl,
                    std::ptr::null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let _memory = Allocation(descriptor);
        assert!(!acl.is_null());
        let len =
            unsafe { std::ptr::read_unaligned((acl as *const u8).add(2).cast::<u16>()) } as usize;
        unsafe { std::slice::from_raw_parts(acl as *const u8, len).to_vec() }
    }
    fn full_descriptor(path: &Path) -> Vec<u8> {
        let file = open(path);
        let mut descriptor = std::ptr::null_mut();
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
                    &mut descriptor,
                )
            },
            0
        );
        let memory = Allocation(descriptor);
        let size = unsafe { GetSecurityDescriptorLength(memory.0) } as usize;
        assert!(size > 20);
        // Owner, group, DACL, protection/auto-inheritance bits and revision are
        // included, unlike a DACL-only comparison.
        unsafe { std::slice::from_raw_parts(memory.0.cast::<u8>(), size).to_vec() }
    }
    fn set_policy(parent_handle: &File, policy: &str) {
        let text: Vec<_> = policy.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let _memory = Allocation(descriptor);
        let (mut present, mut defaulted, mut acl) = (0, 0, std::ptr::null_mut());
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted)
            },
            0
        );
        assert_eq!(
            unsafe {
                SetSecurityInfo(
                    parent_handle.as_raw_handle(),
                    1,
                    4 | 0x8000_0000,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    acl,
                    std::ptr::null_mut(),
                )
            },
            0
        );
    }
    fn rename(stage: &PrivateStage, parent: &File, name: &str) {
        let name: Vec<u16> = name.encode_utf16().collect();
        let offset = std::mem::size_of::<usize>() * 2 + 4;
        let bytes = offset + name.len() * 2;
        let mut buffer = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
        let raw = buffer.as_mut_ptr().cast::<u8>();
        unsafe {
            std::ptr::write(
                raw.add(std::mem::size_of::<usize>()).cast::<Handle>(),
                parent.as_raw_handle(),
            );
            std::ptr::write(
                raw.add(std::mem::size_of::<usize>() * 2).cast::<u32>(),
                (name.len() * 2) as u32,
            );
            std::ptr::copy_nonoverlapping(
                name.as_ptr().cast::<u8>(),
                raw.add(offset),
                name.len() * 2,
            );
        }
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let result = unsafe {
            NtSetInformationFile(
                stage.root().as_raw_handle(),
                (&mut status as *mut IoStatus).cast(),
                raw.cast(),
                bytes as u32,
                10,
            )
        };
        assert_eq!(
            result,
            0,
            "{}",
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32)
        );
    }
    fn inheritance_case(policy: &str, native: bool) {
        let t = tempfile::tempdir().unwrap();
        let parent = t.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let parent_handle = open(&parent);
        set_policy(&parent_handle, policy);
        let normal = parent.join("normal");
        fs::create_dir_all(normal.join("sub/deep")).unwrap();
        fs::write(normal.join("sub/deep/file"), b"reference").unwrap();
        let stage = create(&parent, &parent_handle).unwrap();
        fs::create_dir_all(stage.path().join("sub/deep")).unwrap();
        fs::write(stage.path().join("sub/deep/file"), b"private").unwrap();
        assert_ne!(dacl(stage.path()), dacl(&normal));
        assert_ne!(
            dacl(&stage.path().join("sub/deep/file")),
            dacl(&normal.join("sub/deep/file"))
        );
        assert!(fs::rename(stage.path(), parent.join("hijacked")).is_err());
        assert!(fs::remove_dir(stage.path()).is_err());
        rename(&stage, &parent_handle, "committed");
        let committed = parent.join("committed");
        let root = stage.root().try_clone().unwrap();
        let _ = stage.keep();
        let mut permissions = fs::metadata(committed.join("sub/deep/file"))
            .unwrap()
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(committed.join("sub/deep/file"), permissions).unwrap();
        // Pins forbid replacing both directories and files while recursive ACL
        // inheritance opens the tree. Security rights do not need WRITE sharing.
        let pins: Vec<_> = ["sub", "sub/deep", "sub/deep/file"]
            .iter()
            .enumerate()
            .map(|(i, rel)| PermissionNode {
                file: fs::OpenOptions::new()
                    .access_mode(0x60081)
                    .share_mode(1)
                    .custom_flags(0x02200000)
                    .open(committed.join(rel))
                    .unwrap(),
                parent_index: i.checked_sub(1),
                is_dir: i < 2,
            })
            .collect();
        if native {
            inherit_tree_with(&root, &parent_handle, &pins, apply_native).unwrap();
        } else {
            inherit_tree(&root, &parent_handle, &pins).unwrap();
        }
        for rel in ["", "sub", "sub/deep", "sub/deep/file"] {
            assert_eq!(dacl(&committed.join(rel)), dacl(&normal.join(rel)), "{rel}");
            assert_eq!(
                full_descriptor(&committed.join(rel)),
                full_descriptor(&normal.join(rel)),
                "full SD {rel}"
            );
        }
        assert!(fs::metadata(committed.join("sub/deep/file"))
            .unwrap()
            .permissions()
            .readonly());
        drop(pins);
        drop(root);
        // A matching current ACL is insufficient: inheritance flags must also
        // produce exactly normal permissions on items created in the future.
        for base in [&normal, &committed] {
            fs::create_dir_all(base.join("sub/deep/future/child")).unwrap();
            fs::write(base.join("sub/deep/future/child/file"), b"new data").unwrap();
        }
        for rel in [
            "sub/deep/future",
            "sub/deep/future/child",
            "sub/deep/future/child/file",
        ] {
            assert_eq!(
                full_descriptor(&committed.join(rel)),
                full_descriptor(&normal.join(rel)),
                "future SD {rel}"
            );
        }
    }
    #[test]
    fn private_creation_and_committed_inheritance_match_normal_creation() {
        let user = current_sid(&token().unwrap()).unwrap();
        inheritance_case(
            &format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"),
            false,
        );
    }
    #[test]
    fn creator_owner_inheritance_matches_normal_creation() {
        let user = current_sid(&token().unwrap()).unwrap();
        inheritance_case(&format!("D:P(A;;FA;;;{user})(A;OICIIO;FA;;;CO)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"), false);
    }
    #[test]
    fn absent_inherited_aces_use_normal_token_defaults() {
        let user = current_sid(&token().unwrap()).unwrap();
        inheritance_case(&format!("D:P(A;;FA;;;{user})"), false);
    }
    #[test]
    fn native_acl_matches_full_descriptor_and_future_inheritance() {
        let user = current_sid(&token().unwrap()).unwrap();
        for policy in [
            format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"),
            format!("D:P(A;;FA;;;{user})(A;OICIIO;FA;;;CO)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"),
            format!("D:P(A;;FA;;;{user})"),
            format!("D:P(D;OICI;GW;;;BG)(A;OICI;FA;;;{user})(A;OICI;GRGX;;;BU)"),
        ] {
            inheritance_case(&policy, true);
        }
    }
    #[test]
    fn abandoned_stage_deletes_only_its_pinned_directory() {
        let t = tempfile::tempdir().unwrap();
        let parent = open(t.path());
        let stage = create(t.path(), &parent).unwrap();
        fs::create_dir(stage.path().join("sub")).unwrap();
        fs::write(stage.path().join("sub/file"), b"temporary").unwrap();
        let path = stage.path().to_owned();
        let pin = stage.root().try_clone().unwrap();
        drop(stage);
        drop(pin);
        assert!(!path.exists());
    }
    #[test]
    fn abandoned_stage_cleans_readonly_files_at_root_and_nested_levels() {
        let t = tempfile::tempdir().unwrap();
        let parent = open(t.path());
        let stage = create(t.path(), &parent).unwrap();
        fs::create_dir(stage.path().join("sub")).unwrap();
        for name in ["readonly.txt", "sub/readonly.txt"] {
            let path = stage.path().join(name);
            fs::write(&path, b"temporary").unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        }
        let path = stage.path().to_owned();
        drop(stage);
        assert!(!path.exists());
    }
    #[test]
    fn no_propagate_policy_is_excluded_from_fast_inheritance() {
        let t = tempfile::tempdir().unwrap();
        let parent = open(t.path());
        let user = current_sid(&token().unwrap()).unwrap();
        set_policy(
            &parent,
            &format!("D:PAI(A;OICI;FA;;;{user})(A;CINP;GR;;;SY)"),
        );
        assert!(!supports_inheritance(&parent).unwrap());
        set_policy(
            &parent,
            &format!("D:PAI(A;OICI;FA;;;{user})(A;OICI;GRGX;;;BU)"),
        );
        assert!(supports_inheritance(&parent).unwrap());
    }
    #[test]
    #[ignore = "6000-file ACL phase benchmark; run explicitly"]
    fn profile_native_and_win32_acl_phases() {
        use std::{collections::BTreeMap, time::Instant};
        let bench_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("analysis/acl-phase-bench");
        fs::create_dir_all(&bench_root).unwrap();
        let user = current_sid(&token().unwrap()).unwrap();
        for (round, native) in [false, true, true, false].into_iter().enumerate() {
            let t = tempfile::tempdir_in(&bench_root).unwrap();
            let parent = open(t.path());
            set_policy(
                &parent,
                &format!("D:PAI(A;OICI;FA;;;{user})(A;OICI;0x1301bf;;;AU)"),
            );
            let normal = t.path().join("reference");
            fs::create_dir(&normal).unwrap();
            fs::write(normal.join("file"), b"fixture").unwrap();
            let stage = create(t.path(), &parent).unwrap();
            let mut expected = BTreeMap::new();
            for group in 0..60 {
                let folder = format!("group{group:02}");
                fs::create_dir(stage.path().join(&folder)).unwrap();
                expected.insert(folder.clone(), (true, 0));
                for item in 0..100 {
                    let name = format!("{folder}/file{item:03}.txt");
                    fs::write(stage.path().join(&name), b"fixture").unwrap();
                    expected.insert(name, (false, 7));
                }
            }
            let root = stage.root().try_clone().unwrap();
            let started = Instant::now();
            rename(&stage, &parent, "published");
            let _ = stage.keep();
            let committed = started.elapsed().as_secs_f64();
            let path = t.path().join("published");
            let started = Instant::now();
            let pins = crate::engine::pin_committed_tree(&path, &expected).unwrap();
            let pinned = started.elapsed().as_secs_f64();
            let started = Instant::now();
            let prepared = prepare_permissions(&parent, &pins).unwrap();
            let derived = started.elapsed().as_secs_f64();
            let started = Instant::now();
            let setter = if native { apply_native } else { apply };
            setter(&root, &prepared.root).unwrap();
            for (node, permissions) in pins.iter().zip(prepared.nodes) {
                setter(&node.file, &permissions).unwrap();
            }
            let applied = started.elapsed().as_secs_f64();
            for relative in [
                "group00/file000.txt",
                "group29/file099.txt",
                "group59/file099.txt",
            ] {
                assert_eq!(
                    full_descriptor(&path.join(relative)),
                    full_descriptor(&normal.join("file"))
                );
            }
            assert_eq!(pins.len(), 6060);
            println!(
                "{}",
                serde_json::json!({"route": if native {"native"} else {"win32"}, "round":round,
                "files":6000,"commit_seconds":committed,"pin_seconds":pinned,"derive_seconds":derived,"apply_seconds":applied})
            );
            drop(pins);
            drop(root);
            drop(parent);
        }
    }
}
