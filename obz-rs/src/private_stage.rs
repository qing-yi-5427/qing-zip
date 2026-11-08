//! Same-volume staging is created privately and pinned in one NtCreateFile call.
//! Whole-directory publication restores inheritance after committing the tree.
//! Individual files retain their private DACL until the atomic rename has
//! committed; the held source handle then restores destination inheritance.
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
#[cfg(target_pointer_width = "64")]
mod witnessed;
#[cfg(target_pointer_width = "64")]
pub(crate) use witnessed::qualifies_legacy_parent;
mod output;
mod output_receipt;
pub(crate) use output_receipt::{directory_stamp,OutputReceipt};

mod archive_output;
pub(crate) use archive_output::ArchiveOutput;
pub(crate) use output::{create_output, PrivateOutputFile};
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
    #[cfg(test)]
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
    sid_text(unsafe { *(buffer.as_ptr().cast::<Handle>()) })
}
fn sid_text(sid: Handle) -> Result<String, String> {
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
    #[cfg(test)]
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
                delete_empty_directory(&self.root);
            }
        }
    }
}
pub(crate) fn delete_empty_directory(root: &File) {
    let mut status = IoStatus { status: 0, information: 0 };
    let mut delete = 1u8;
    unsafe {
        NtSetInformationFile(root.as_raw_handle(), (&mut status as *mut IoStatus).cast(),
            (&mut delete as *mut u8).cast(), 1, 13);
    }
}
pub(crate) fn create(parent: &Path, parent_handle: &File) -> Result<PrivateStage, String> {
    let (path, root, parent_pin) = create_root(parent, parent_handle, ".qing-zip-stage-", 0x170081, 1)?;
    Ok(PrivateStage { path: Some(path), parent: parent.to_owned(), root, _parent_pin: parent_pin })
}

pub(crate) fn create_drag_root(parent: &Path, parent_handle: &File) -> Result<(PathBuf, File, File), String> {
    // A separate extraction request must be able to open this directory as
    // an ordinary ancestor. Its held pin denies rename, but requests no DELETE.
    // Windows needs write sharing on the destination parent to rename a
    // verified child into it. Keep its namespace pinned against deletion.
    create_root(parent, parent_handle, "qing-zip-drag-", 0x120081, 3)
}

fn protected_descriptor(sid: &str) -> Result<Allocation, String> {
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
    Ok(Allocation(raw))
}
fn create_root(parent: &Path, parent_handle: &File, prefix: &str, access: u32, share: u32) -> Result<(PathBuf, File, File), String> {
    let token = token()?;
    let memory = protected_descriptor(&current_sid(&token)?)?;
    create_root_with_descriptor(parent, parent_handle, prefix, access, share, memory.0)
        .map_err(|error| format!("create private stage: {error}"))
}
fn create_root_with_descriptor(parent: &Path, parent_handle: &File, prefix: &str, access: u32,
    share: u32, security: Handle) -> io::Result<(PathBuf, File, File)> {
    let parent_pin = parent_handle.try_clone()?;
    for _ in 0..16 {
        let mut random = [0u8; 16];
        if unsafe { BCryptGenRandom(std::ptr::null_mut(), random.as_mut_ptr(), 16, 2) } < 0 {
            return Err(io::Error::other("cannot generate stage name"));
        }
        let name = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let leaf = format!("{prefix}{name}");
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
            descriptor: security,
            qos: std::ptr::null_mut(),
        };
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        // FILE_CREATE + requested descriptor + deny-delete pin are atomic. No
        // parent DELETE_CHILD actor can replace the directory before pinning.
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &attributes,
                &mut status,
                std::ptr::null(),
                0x10,
                share,
                2,
                0x200021,
                std::ptr::null_mut(),
                0,
            )
        };
        if result >= 0 {
            return Ok((path, unsafe { File::from_raw_handle(handle) }, parent_pin));
        }
        let error = io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32);
        if error.raw_os_error() != Some(183) {
            return Err(error);
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not reserve stage directory"))
}
/// Query one current leaf through a strongly pinned private directory. The
/// caller owns that qualification; no ancestor path is resolved by this call.
pub(crate) fn child_metadata(parent: &File, leaf: &OsStr) -> Result<std::fs::Metadata, String> {
    checked_metadata(&open_private_child(parent, leaf, ChildAccess::Metadata)?)
}

/// The caller holds the direct, strongly pinned parent inside its private stage.
pub(crate) fn create_child_file(parent: &File, leaf: &OsStr) -> Result<File, String> {
    open_private_child(parent, leaf, ChildAccess::CreateFile)
}

/// FILE_CREATE yields an identity handle only for a directory created by this call.
pub(crate) fn create_journal_directory(parent:&File,leaf:&OsStr)->Result<File,String>{
    open_private_child(parent,leaf,ChildAccess::CreateJournalDirectory)
}
pub(crate) fn create_output_directory(parent: &File, leaf: &OsStr) -> Result<File, String> {
    open_private_child(parent, leaf, ChildAccess::CreateDirectory)
}

pub(crate) fn open_output_directory(parent: &File, leaf: &OsStr) -> Result<File, String> {
    let file = open_private_child(parent, leaf, ChildAccess::OpenDirectory)?;
    use std::os::windows::fs::MetadataExt;
    let attrs = file.metadata().map_err(|e| e.to_string())?.file_attributes();
    if attrs & 0x410 != 0x10 { return Err("linked or changed output directory".into()); }
    Ok(file)
}

pub(crate) fn open_checksum_child(parent: &File, leaf: &OsStr, directory: bool) -> Result<File, String> {
    let mode = if directory { ChildAccess::ChecksumDirectory } else { ChildAccess::ChecksumFile };
    open_private_child(parent, leaf, mode)
}

enum ChildAccess {
    ChecksumDirectory,
    ChecksumFile,
    ArchiveReference,
    OpenDirectory,
    Metadata,
    CreateFile,
    CreateDirectory,
    CreateJournalDirectory,
    #[cfg(target_pointer_width = "64")]
    Witness(bool),
}

fn open_private_child(parent: &File, leaf: &OsStr, mode: ChildAccess) -> Result<File, String> {
    let mut wide: Vec<u16> = leaf.encode_wide().collect();
    if wide.is_empty()
        || wide.len() > 32767
        || wide.iter().any(|c| matches!(*c, 0 | 47 | 58 | 92))
        || wide == [46]
        || wide == [46, 46]
    {
        return Err("invalid private stage leaf".into());
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
        attributes: if matches!(mode, ChildAccess::ChecksumDirectory | ChildAccess::ChecksumFile) { 0x1040 } else { 0x40 },
        descriptor: std::ptr::null_mut(),
        qos: std::ptr::null_mut(),
    };
    let mut status = IoStatus {
        status: 0,
        information: 0,
    };
    let mut raw = std::ptr::null_mut();
    let (access, file_attributes, sharing, disposition, options, operation) = match mode {
        // Never follow reparse leafs or admit writers/renames. For files ask
        // for GENERIC_READ, just as File::options().read(true) does.
        ChildAccess::ChecksumDirectory => (0x100081, 0, 1, 1, 0x200021, "pin checksum directory"),
        ChildAccess::ChecksumFile => (0x80100000, 0, 1, 1, 0x200060, "open checksum file"),
        ChildAccess::ArchiveReference => (0x130080,0x80,0,2,0x201060,"create ordinary ZIP ACL reference"),
        #[cfg(target_pointer_width = "64")]
        ChildAccess::Witness(is_dir) => (0x170081, if is_dir { 0x10 } else { 0x80 },
            0, 2, if is_dir { 0x201021 } else { 0x201060 }, "create inheritance witness"),
        ChildAccess::Metadata => (0x100081, 0, 1, 1, 0x200020, "read private stage metadata"),
        // WRITE_DATA | READ/WRITE_ATTRIBUTES | SYNCHRONIZE, FILE_CREATE and
        // FILE_NON_DIRECTORY_FILE. Existing files, links and reparse leaves
        // cannot be opened or truncated.
        ChildAccess::CreateFile => (0x100182, 0x80, 1, 2, 0x200060, "create private stage file"),
        ChildAccess::OpenDirectory => (0x100081, 0, 3, 1, 0x200021, "pin output directory"),
        ChildAccess::CreateJournalDirectory => (0x120180, 0, 7, 2, 0x200021, "create journaled output directory"),
        ChildAccess::CreateDirectory => (0x100080, 0, 7, 2, 0x200021, "create output directory"),
    };
    let result = unsafe {
        NtCreateFile(
            &mut raw,
            access,
            &attributes,
            &mut status,
            std::ptr::null(),
            file_attributes,
            sharing,
            disposition,
            options,
            std::ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(format!("{operation}: {}", io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32)));
    }
    Ok(unsafe { File::from_raw_handle(raw) })
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
    security_descriptor(file, 4)
}
fn security_descriptor(file: &File, mask: u32) -> Result<Allocation, String> {
    let mut descriptor = std::ptr::null_mut();
    let result = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            1,
            mask,
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
    ordinary_aces(descriptor)
}
fn ordinary_aces(descriptor: Handle) -> Result<bool, String> {
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
    derive_with(parent, container, token, 1, 0x500)
}
fn derive_with(parent: Handle, container: bool, token: &Token, flags: u32, inheritance_bits: u16) -> Result<PrivateDescriptor, String> {
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
            flags,
            token.0,
            &mut mapping,
        )
    } == 0
    {
        return Err(error("derive destination permissions"));
    }
    let computed = PrivateDescriptor(computed);
    // AR and AI together request automatic inheritance through the native
    // setter. A witnessed legacy transition must instead preserve cleared bits.
    if unsafe { SetSecurityDescriptorControl(computed.0, 0x500, inheritance_bits) } == 0 {
        return Err(error("prepare automatic destination permissions"));
    }
    Ok(computed)
}
#[cfg(test)]
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
#[cfg(test)]
fn apply_compatible(root: &File, computed: &PrivateDescriptor) -> Result<(), String> {
    match apply_native(root, computed) {
        Ok(()) => Ok(()),
        // Preserve the proven Win32 behavior on filesystems that reject the
        // native update. Both routes use the same held identity and descriptor.
        Err(native) => apply(root, computed).map_err(|fallback| format!("{native}; {fallback}")),
    }
}
#[cfg(test)]
pub(crate) struct PermissionNode {
    pub(crate) file: File,
    pub(crate) parent_index: Option<usize>,
    pub(crate) is_dir: bool,
}
/// Historical comparator used only by fixture tests. A whole-tree replay can
/// overwrite a concurrent revocation and must never be used by extraction.
/// Nodes are parent-before-child, pinned READ_DATA|READ_CONTROL|WRITE_DAC with read-only
/// sharing. Never depend on SetSecurityInfo's silent best-effort propagation:
/// explicitly update every pinned object, using cached sibling descriptors.
#[cfg(test)]
pub(crate) fn inherit_tree(
    root: &File,
    parent: &File,
    nodes: &[PermissionNode],
) -> Result<(), String> {
    inherit_tree_with(root, parent, nodes, apply_compatible)
}
#[cfg(test)]
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
#[cfg(test)]
struct PreparedPermissions {
    root: std::rc::Rc<PrivateDescriptor>,
    nodes: Vec<std::rc::Rc<PrivateDescriptor>>,
}
#[cfg(test)]
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

    #[test]
    fn new_child_is_bound_to_the_open_directory_and_preserves_existing_links() {
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("original");
        let moved = root.path().join("moved");
        let sentinel = root.path().join("sentinel.txt");
        fs::create_dir(&original).unwrap();
        fs::write(&sentinel, b"existing content").unwrap();
        fs::hard_link(&sentinel, original.join("occupied.txt")).unwrap();
        // Allow renaming in this fixture to distinguish an identity-relative
        // create from an accidental reopen by path.
        let directory = File::options().access_mode(0x81).share_mode(7)
            .custom_flags(0x02200000).open(&original).unwrap();
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        let mut created = create_child_file(&directory, OsStr::new("new.txt")).unwrap();
        created.write_all(b"new content").unwrap();
        drop(created);
        assert_eq!(fs::read(moved.join("new.txt")).unwrap(), b"new content");
        assert!(!original.join("new.txt").exists());
        for invalid in ["occupied.txt", "new.txt", ".", "..", "../sentinel.txt", "new.txt:stream"] {
            assert!(create_child_file(&directory, OsStr::new(invalid)).is_err(), "{invalid}");
        }
        assert_eq!(fs::read(&sentinel).unwrap(), b"existing content");
        assert_eq!(fs::read(moved.join("new.txt")).unwrap(), b"new content");
    }

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

#[cfg(target_pointer_width = "64")]
pub(crate) use file_publication::{publish_verified_file_journaled, supports_file_publication, FilePublication};

#[cfg(all(test, target_pointer_width = "64"))]
pub(crate) fn ordinary_file_publication_test_parent(parent: &File) {
    // Only a newly created fixture directory. A deterministic protected parent
    // gives its children ordinary automatic inheritance without depending on a
    // runner's TEMP DACL (which production deliberately may send to fallback).
    let sid = current_sid(&token().unwrap()).unwrap();
    let text: Vec<_> = format!("D:PAI(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)")
        .encode_utf16().chain(Some(0)).collect();
    let mut memory = std::ptr::null_mut();
    assert_ne!(unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text.as_ptr(), 1, &mut memory, std::ptr::null_mut()) }, 0);
    let descriptor = Allocation(memory);
    assert_ne!(unsafe { SetSecurityDescriptorControl(descriptor.0, 0x500, 0x500) }, 0);
    assert!(unsafe { NtSetSecurityObject(parent.as_raw_handle(), 4 | 0x8000_0000, descriptor.0) } >= 0);
    assert!(supports_inheritance(parent).unwrap());
}

#[cfg(target_pointer_width = "64")]
mod file_publication {
    use super::*;
    use std::time::SystemTime;

    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(file: Handle, info: *mut u32) -> i32;
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
        #[cfg(test)]
        fn SetFileInformationByHandle(
            file: Handle,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn GetFinalPathNameByHandleW(file: Handle, path: *mut u16, size: u32, flags: u32) -> u32;
    }

    /// Unavailable guarantees that the source retained its original permissions
    /// and no destination was changed. Committed must never enter a copy fallback.
    pub(crate) enum FilePublication {
        Unavailable,
        Committed(File),
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Step { BeforeCommit, AfterCommit, ApplyTarget }

    fn leaf(value: &OsStr) -> Result<Vec<u16>, String> {
        let name: Vec<_> = value.encode_wide().collect();
        if name.is_empty()
            || name.len() > 32767
            || name.contains(&0)
            || name.iter().any(|c| matches!(*c, 47 | 58 | 92))
            || name == [46]
            || name == [46, 46]
        {
            return Err("invalid publication leaf".into());
        }
        Ok(name)
    }

    fn info(file: &File) -> Result<[u32; 13], String> {
        let mut value = [0; 13];
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), value.as_mut_ptr()) } == 0 {
            return Err(error("query publication identity"));
        }
        Ok(value)
    }

    fn ntfs(file: &File) -> bool {
        let mut name = [0u16; 16];
        (unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                name.as_mut_ptr(),
                name.len() as u32,
            )
        }) != 0
            && name[..5] == [78, 84, 70, 83, 0]
    }

    pub(crate) fn supports_file_publication(parent: &File) -> Result<bool, String> {
        let attributes = info(parent)?[0];
        Ok(attributes & 0x410 == 0x10
            && attributes & !(0x10 | 1 | 2 | 4 | 0x20 | 0x80) == 0
            && ntfs(parent))
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileTime(file: Handle, created: *const u64, accessed: *const u64, modified: *const u64) -> i32;
    }
    fn open_source(parent: &File, mut name: Vec<u16>) -> Result<File, String> {
        let mut unicode = UnicodeString {
            length: (name.len() * 2) as u16,
            maximum_length: (name.len() * 2) as u16,
            buffer: name.as_mut_ptr(),
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
        let mut handle = std::ptr::null_mut();
        // Preserve the source directory pins; this additional leaf pin has
        // actual READ_DATA access, so a concurrent data writer cannot exist.
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                0x170181,
                &attributes,
                &mut status,
                std::ptr::null(),
                0,
                0,
                1,
                0x200060,
                std::ptr::null_mut(),
                0,
            )
        };
        if result < 0 {
            return Err(format!(
                "open publication source: {}",
                io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32)
            ));
        }
        let file = unsafe { File::from_raw_handle(handle) };
        // BackupRead verifies Zone.Identifier. Keep NTFS from materializing
        // that validation as a new last-access timestamp when this handle closes.
        if unsafe { SetFileTime(handle, std::ptr::null(), &u64::MAX, std::ptr::null()) } == 0 {
            return Err(error("retain verified file access time"));
        }
        Ok(file)
    }

    fn rename(file: &File, parent: &File, name: &[u16], replace: bool) -> Result<(), String> {
        let length = 22 + name.len() * 2;
        let mut buffer = vec![0u64; length.div_ceil(8)];
        let raw = buffer.as_mut_ptr().cast::<u8>();
        let mut status = [0usize; 2];
        let result = unsafe {
            *raw = u8::from(replace);
            std::ptr::write_unaligned(raw.add(8).cast::<Handle>(), parent.as_raw_handle());
            std::ptr::write_unaligned(raw.add(16).cast::<u32>(), (name.len() * 2) as u32);
            std::ptr::copy_nonoverlapping(name.as_ptr().cast::<u8>(), raw.add(20), name.len() * 2);
            NtSetInformationFile(
                file.as_raw_handle(),
                status.as_mut_ptr(),
                raw.cast(),
                length as u32,
                10,
            )
        };
        if result < 0 {
            return Err(format!(
                "rename verified file: {}",
                io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32)
            ));
        }
        Ok(())
    }

    fn pinned_path(file: &File) -> Result<String, String> {
        let mut path = vec![0u16; 32768];
        let len = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                path.as_mut_ptr(),
                path.len() as u32,
                0,
            )
        };
        if len == 0 || len as usize >= path.len() {
            return Err(error("resolve private recovery path"));
        }
        Ok(String::from_utf16_lossy(&path[..len as usize]))
    }

    /// Caller must own a verified, private staging tree and retain all source
    /// and destination ancestor identity guards. ADS and metadata normalization
    /// must be complete before calling. `target_parent` has READ_CONTROL and
    /// TRAVERSE, share READ|WRITE; the existing stronger output guard is retained.
    /// All content/ADS/metadata is finalized before commit. DACL inheritance
    /// is applied only AFTER rename: traversal privileges and independently
    /// shared ADS must never expose a precommit leaf under destination access.
    /// Postcommit permission failure is an error that retains the target; it
    /// must never trigger fallback, target deletion, or source archive recycling.
    #[cfg(test)]
    pub(crate) fn publish_verified_file(
        source_parent: &File,
        source_leaf: &OsStr,
        target_parent: &File,
        target_leaf: &OsStr,
        size: u64,
        modified: SystemTime,
        readonly: bool,
        replace: bool,
        check_cancel: impl FnMut() -> Result<(), String>,
    ) -> Result<FilePublication, String> {
        publish_verified_file_journaled(source_parent,source_leaf,target_parent,target_leaf,size,modified,readonly,replace,None,check_cancel)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_verified_file_journaled(
        source_parent: &File,
        source_leaf: &OsStr,
        target_parent: &File,
        target_leaf: &OsStr,
        size: u64,
        modified: SystemTime,
        readonly: bool,
        replace: bool,
        journal: Option<&crate::engine::PublicationJournal>,
        mut check_cancel: impl FnMut() -> Result<(), String>,
    ) -> Result<FilePublication, String> {
        publish_with(source_parent, source_leaf, target_parent, target_leaf,
            size, modified, readonly, replace, journal, &mut check_cancel, &mut |_| Ok(()))
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_with(
        source_parent: &File,
        source_leaf: &OsStr,
        target_parent: &File,
        target_leaf: &OsStr,
        size: u64,
        modified: SystemTime,
        readonly: bool,
        replace: bool,
        journal: Option<&crate::engine::PublicationJournal>,
        check_cancel: &mut dyn FnMut() -> Result<(), String>,
        step: &mut dyn FnMut(Step) -> Result<(), String>,
    ) -> Result<FilePublication, String> {
        check_cancel()?;
        let source_name = leaf(source_leaf)?;
        let target_name = leaf(target_leaf)?;
        let parent_info = info(target_parent)?;
        if parent_info[0] & 0x410 != 0x10 {
            return Err("changed publication parent".into());
        }
        // Rename does not adopt compression/EFS/storage policy as a newly
        // created file does. Such destinations preserve the copy path.
        if parent_info[0] & !(0x10 | 1 | 2 | 4 | 0x20 | 0x80) != 0 || !ntfs(target_parent)
        {
            return Ok(FilePublication::Unavailable);
        }
        let file = match open_source(source_parent, source_name.clone()) {
            Ok(file) => file,
            Err(_) => return Ok(FilePublication::Unavailable),
        };
        let source_info = info(&file)?;
        if source_info[0] & 0x410 != 0
            || source_info[10] != 1
            || ((source_info[8] as u64) << 32 | source_info[9] as u64) != size
        {
            return Err("changed publication source".into());
        }
        if source_info[7] != parent_info[7]
            || source_info[0] != (0x20 | u32::from(readonly))
            || file.metadata().map_err(|e| e.to_string())?.modified().ok() != Some(modified)
        {
            return Ok(FilePublication::Unavailable);
        }
        let policy = match descriptor(target_parent) {
            Ok(policy) => policy,
            Err(_) => return Ok(FilePublication::Unavailable),
        };
        if !supports_descriptor(policy.0)? {
            return Ok(FilePublication::Unavailable);
        }
        let permissions = match token().and_then(|token| derive(policy.0, false, &token)) {
            Ok(value) => value,
            Err(_) => return Ok(FilePublication::Unavailable),
        };
        let path = PathBuf::from(pinned_path(target_parent)?).join(target_leaf);
        // Eligibility only; the final handle-relative rename still enforces
        // collision and replacement semantics against concurrent changes.
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if !replace || meta.is_dir() || meta.permissions().readonly()
                || meta.file_type().is_symlink() => return Ok(FilePublication::Unavailable),
            Err(e) if e.kind()!=io::ErrorKind::NotFound => return Ok(FilePublication::Unavailable),
            _ => {}
        }
        let private = descriptor(&file)?;
        let original_flags = if output::control(private.0)? & 0x1000 != 0 { 0x80000000 } else { 0x20000000 };
        let marker = output::output_marker(&file)?;
        let access_time = u64::from(source_info[3]) | (u64::from(source_info[4]) << 32);
        let committed = output::metadata_handle(&file, 5)?;
        let name = output::private_name()?;
        let scratch = leaf(OsStr::new(&name))?;
        check_cancel()?;
        if step(Step::BeforeCommit).is_err() { return Ok(FilePublication::Unavailable); }
        // First cross directories with a protected private DACL. The final
        // inheritance is applied only after the file is in its *actual*
        // destination parent, so changes there can reach the prepared file
        // even between the final validation and the no-replace rename.
        let set_original = |flags: u32| -> Result<(),String> {
            let status=unsafe { NtSetSecurityObject(file.as_raw_handle(),4 | flags,private.0) };
            if status<0 {return Err("could not restore staged file permissions".into());}
            Ok(())
        };
        set_original(0x80000000)?;
        if rename(&file, target_parent, &scratch, false).is_err() {
            set_original(original_flags)?;
            return Ok(FilePublication::Unavailable);
        }
        let mut pending = None;
        let transition = (|| -> Result<Result<(),String>,String> {
            let current = descriptor(target_parent)?;
            if !supports_descriptor(current.0)? || !output::same_file_permissions(permissions.0, derive(current.0, false, &token()?)?.0)? {
                return Err("destination permissions changed; verified file was not published".into());
            }
            step(Step::ApplyTarget)?;
            apply_native(&file, &permissions)?;
            let fresh=descriptor(target_parent)?;
            if !supports_descriptor(fresh.0)? || !output::same_file_permissions(permissions.0, derive(fresh.0, false, &token()?)?.0)? {
                return Err("destination permissions changed; verified file was not published".into());
            }
            if info(&file)?[10] != 1 || output::output_marker(&file)? != marker {
                return Err("verified file links or streams changed".into());
            }
            pending=journal.map(|_|OutputReceipt::capture(&path,&file)).transpose()?;
            if unsafe { SetFileTime(file.as_raw_handle(), std::ptr::null(), &access_time, std::ptr::null()) } == 0 {
                return Err(error("restore verified file access time"));
            }
            check_cancel()?;
            Ok(rename(&file, target_parent, &target_name, replace))
        })();
        if !matches!(transition, Ok(Ok(()))) {
            let reason = match &transition {Err(e)|Ok(Err(e))=>e.as_str(),_=>unreachable!()};
            // No final name exists. Reprotect before moving back, restore the
            // exact previous source policy in its own parent, and never replace
            // an unexpected entry. Recovery failure is a hard error.
            set_original(0x80000000)?;
            if let Err(restore) = rename(&file, source_parent, &source_name, false) {
                // The strongly pinned stage parent may reject a move back.
                // Discard only our known private derivative by its handle.
                // The archive and any competing destination remain untouched.
                let scratch_path = path.with_file_name(&name);
                output::discard_private_handle(&file,&scratch_path)?;
                return Err(format!("verified file was not published: {reason}; private parked copy discarded: {restore}"));
            }
            set_original(original_flags)?;
            if unsafe { SetFileTime(file.as_raw_handle(), std::ptr::null(), &access_time, std::ptr::null()) } == 0 {
                return Err(error("restore staged file access time"));
            }
            if let Err(error)=transition {return Err(error);}
            return Ok(FilePublication::Unavailable);
        }
        if let (Some(journal),Some(receipt))=(journal,pending) {
            journal.record(receipt);
            step(Step::AfterCommit).map_err(|e|format!("verified file was committed: {}: {e}",path.display()))?;
            return Ok(FilePublication::Committed(committed));
        }
        drop(file);
        step(Step::AfterCommit).map_err(|e|format!("verified file was committed: {}: {e}",path.display()))?;
        // Reopen by the retained identity, never by the public name, to carry
        // the original sharing contract until the caller releases its handle.
        let final_pin=output::reopen(&committed,0x20081,1).map_err(|e|format!("verified file was committed: {}: {e}",path.display()))?;
        Ok(FilePublication::Committed(final_pin))
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            fs,
            os::windows::fs::OpenOptionsExt,
            time::{Duration, UNIX_EPOCH},
        };

        fn open(path: &Path, access: u32, share: u32) -> File {
            fs::OpenOptions::new()
                .access_mode(access)
                .share_mode(share)
                .custom_flags(0x02200000)
                .open(path)
                .unwrap()
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
            let owned = Allocation(raw);
            let len = unsafe { GetSecurityDescriptorLength(owned.0) } as usize;
            unsafe { std::slice::from_raw_parts(owned.0.cast::<u8>(), len).to_vec() }
        }

        fn attrs(file: &File, value: u32) {
            let mut basic = [0i64; 5];
            basic[4] = value as i64;
            assert_ne!(
                unsafe {
                    SetFileInformationByHandle(file.as_raw_handle(), 0, basic.as_ptr().cast(), 40)
                },
                0
            );
        }
        fn policy(file: &File, no_propagate: bool) {
            let sid = current_sid(&token().unwrap()).unwrap();
            let flags = if no_propagate { "OICINP" } else { "OICI" };
            let text: Vec<_> = format!(
                "D:AI(A;{flags};FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;BU)"
            )
            .encode_utf16()
            .chain(Some(0))
            .collect();
            let mut raw = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        text.as_ptr(),
                        1,
                        &mut raw,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            let owned = Allocation(raw);
            assert_ne!(
                unsafe { SetSecurityDescriptorControl(owned.0, 0x500, 0x500) },
                0
            );
            assert!(
                unsafe { NtSetSecurityObject(file.as_raw_handle(), 4 | 0x2000_0000, owned.0) } >= 0
            );
        }
        struct Fixture {
            temp: tempfile::TempDir,
            stage: Option<PrivateStage>,
            pins: Vec<File>,
            source: PathBuf,
            target: PathBuf,
            modified: SystemTime,
            original_security: Vec<u8>,
            expected_security: Vec<u8>,
            identity: [u32; 13],
            readonly: bool,
        }
        impl Fixture {
            fn new(readonly: bool) -> Self {
                let temp = tempfile::tempdir().unwrap();
                let parent = open(temp.path(), 0x20081, 3);
                let stage = create(temp.path(), &parent).unwrap();
                let mut pins = vec![parent, stage.root().try_clone().unwrap()];
                let sub = stage.path().join("nested");
                fs::create_dir(&sub).unwrap();
                pins.push(open(&sub, 0x81, 1));
                let source = sub.join("source.bin");
                fs::write(&source, b"verified bytes").unwrap();
                fs::write(
                    format!("{}:Zone.Identifier", source.display()),
                    b"[ZoneTransfer]\r\nZoneId=3\r\n",
                )
                .unwrap();
                let modified = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_700);
                let file = open(&source, 0x20180, 7);
                file.set_times(fs::FileTimes::new().set_modified(modified))
                    .unwrap();
                attrs(&file, 0x20 | u32::from(readonly));
                let original_security = security(&file);
                let identity = info(&file).unwrap();
                drop(file);
                let out = temp.path().join("out");
                fs::create_dir(&out).unwrap();
                policy(&open(&out, 0x60081, 3), false);
                let control = out.join("ordinary");
                fs::write(&control, b"control").unwrap();
                let expected_security = security(&open(&control, 0x20080, 7));
                pins.push(open(&out, 0x81, 3));
                pins.push(open(&out, 0x200a0, 3));
                fs::write(
                    temp.path().join("original.archive"),
                    b"original archive sentinel",
                )
                .unwrap();
                Self {
                    temp,
                    stage: Some(stage),
                    pins,
                    source,
                    target: out.join("target.bin"),
                    modified,
                    original_security,
                    expected_security,
                    identity,
                    readonly,
                }
            }
            fn run(
                &self,
                replace: bool,
                cancel: &mut dyn FnMut() -> Result<(), String>,
                step: &mut dyn FnMut(Step) -> Result<(), String>,
            ) -> Result<FilePublication, String> {
                publish_with(
                    &self.pins[2],
                    OsStr::new("source.bin"),
                    &self.pins[4],
                    OsStr::new("target.bin"),
                    14,
                    self.modified,
                    self.readonly,
                    replace,
                    None,
                    cancel,
                    step,
                )
            }
            fn unchanged(&self) {
                assert_eq!(fs::read(&self.source).unwrap(), b"verified bytes");
                assert_eq!(
                    security(&open(&self.source, 0x20080, 7)),
                    self.original_security
                );
                assert_eq!(
                    fs::metadata(&self.source).unwrap().modified().unwrap(),
                    self.modified
                );
                assert_eq!(
                    info(&open(&self.source, 0x80, 7)).unwrap()[0],
                    0x20 | u32::from(self.readonly)
                );
                self.archive_intact();
            }
            fn archive_intact(&self) {
                assert_eq!(
                    fs::read(self.temp.path().join("original.archive")).unwrap(),
                    b"original archive sentinel"
                );
            }
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                for path in [&self.source, &self.target] {
                    if let Ok(file) = fs::OpenOptions::new()
                        .access_mode(0x100)
                        .share_mode(7)
                        .custom_flags(0x00200000)
                        .open(path)
                    {
                        attrs(&file, 0x20);
                    }
                }
                self.pins.clear();
                self.stage.take();
            }
        }

        #[test]
        fn file_publication_retains_strong_pins_identity_acl_ads_and_exact_metadata() {
            for readonly in [false, true] {
                let f = Fixture::new(readonly);
                let super::super::FilePublication::Committed(file) = super::publish_verified_file(
                    &f.pins[2], OsStr::new("source.bin"), &f.pins[4], OsStr::new("target.bin"),
                    14, f.modified, f.readonly, false, || Ok(())
                ).unwrap()
                else {
                    panic!("expected NTFS same-volume commit")
                };
                assert!(!f.source.exists());
                let after = info(&file).unwrap();
                // Creation/access/write FILETIMEs are retained; the old copy
                // route needlessly replaced creation/access with temp values.
                assert_eq!(&f.identity[1..7], &after[1..7]);
                assert_eq!(fs::read(&f.target).unwrap(), b"verified bytes");
                assert_eq!(security(&file), f.expected_security);
                for index in [7, 11, 12] {
                    assert_eq!(f.identity[index], after[index]);
                }
                assert_eq!(after[0], 0x20 | u32::from(readonly));
                assert_eq!(file.metadata().unwrap().modified().unwrap(), f.modified);
                assert_eq!(
                    fs::read(format!("{}:Zone.Identifier", f.target.display())).unwrap(),
                    b"[ZoneTransfer]\r\nZoneId=3\r\n"
                );
                assert!(fs::OpenOptions::new()
                    .write(true)
                    .share_mode(7)
                    .open(&f.target)
                    .is_err());
                f.archive_intact();
                drop(file);
            }
        }

        #[test]
        fn file_publication_collision_and_readonly_target_preserve_old_output() {
            for (replace, readonly_target) in [(false, false), (true, true), (true, false)] {
                let f = Fixture::new(false);
                fs::write(&f.target, b"old target sentinel").unwrap();
                if readonly_target {
                    attrs(&open(&f.target, 0x100, 7), 0x21);
                }
                let result = f
                    .run(replace, &mut || Ok(()), &mut |_| Ok(()))
                    .unwrap();
                if replace && !readonly_target {
                    assert!(matches!(result, FilePublication::Committed(_)));
                    assert_eq!(fs::read(&f.target).unwrap(), b"verified bytes");
                } else {
                    assert!(matches!(result, FilePublication::Unavailable));
                    assert_eq!(fs::read(&f.target).unwrap(), b"old target sentinel");
                    f.unchanged();
                }
            }
        }

        #[test]
        fn file_publication_before_commit_failure_and_cancel_never_change_permissions() {
            let f = Fixture::new(true);
            assert!(matches!(f.run(false, &mut || Ok(()), &mut |step| {
                assert_eq!(step, Step::BeforeCommit);
                assert_eq!(security(&open(&f.source, 0x20080, 7)), f.original_security);
                assert!(!f.target.exists());
                Err("injected rename unavailable".into())
            }).unwrap(), FilePublication::Unavailable));
            f.unchanged(); assert!(!f.target.exists());
            for cancel_on in [1, 2] {
                let f = Fixture::new(true); let mut calls = 0;
                let result = f.run(false, &mut || { calls += 1;
                    if calls == cancel_on { Err("cancelled".into()) } else { Ok(()) }
                }, &mut |_| Ok(()));
                assert!(matches!(result, Err(message) if message == "cancelled"));
                f.unchanged(); assert!(!f.target.exists());
            }
        }

        #[test]
        fn file_publication_retains_private_dacl_until_commit_then_finalizes() {
            for readonly in [false, true] {
                let f = Fixture::new(readonly); let mut events = Vec::new();
                let FilePublication::Committed(file) = f.run(false, &mut || Ok(()), &mut |step| {
                    events.push(step);
                    match step {
                        Step::BeforeCommit => {
                            assert_eq!(security(&open(&f.source,0x20080,7)),f.original_security);
                            assert!(!f.target.exists());
                        }
                        Step::ApplyTarget => {
                            assert!(!f.source.exists() && !f.target.exists());
                            let scratch = fs::read_dir(f.target.parent().unwrap()).unwrap()
                                .map(|e|e.unwrap().path()).find(|p|p.file_name().unwrap().to_string_lossy().starts_with(".qing-zip-output-")).unwrap();
                            assert!(File::open(scratch).is_err());
                        }
                        Step::AfterCommit => assert_eq!(security(&open(&f.target,0x20080,7)),f.expected_security),
                    }
                    Ok(())
                }).unwrap() else { panic!("must commit"); };
                assert_eq!(events, [Step::BeforeCommit, Step::ApplyTarget, Step::AfterCommit]);
                assert_eq!(security(&file), f.expected_security);
                assert_eq!(file.metadata().unwrap().modified().unwrap(), f.modified);
                assert_eq!(fs::read(format!("{}:Zone.Identifier", f.target.display())).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
                f.archive_intact();
            }
        }

        #[test]
        fn file_publication_postcommit_failure_never_falls_back_or_deletes_target() {
            for failure in [Step::AfterCommit, Step::ApplyTarget] {
              for replace in [false, true] {
                let f = Fixture::new(true);
                if replace { fs::write(&f.target, b"old output must not be restored after commit").unwrap(); }
                let result = f.run(replace, &mut || Ok(()), &mut |step| {
                    if step == failure { Err("injected postcommit permission failure".into()) } else { Ok(()) }
                });
                let message = match result { Err(message) => message, _ => panic!("must report committed failure") };
                if failure==Step::AfterCommit {
                    assert!(message.contains("was committed"));
                    assert!(message.contains("target.bin"));
                    assert!(!f.source.exists());
                    assert_eq!(fs::read(&f.target).unwrap(), b"verified bytes");
                    assert_eq!(security(&open(&f.target,0x20080,7)),f.expected_security);
                    assert_eq!(fs::metadata(&f.target).unwrap().modified().unwrap(), f.modified);
                    assert_eq!(fs::read(format!("{}:Zone.Identifier", f.target.display())).unwrap(), b"[ZoneTransfer]\r\nZoneId=3\r\n");
                } else if replace {
                    assert_eq!(fs::read(&f.target).unwrap(), b"old output must not be restored after commit");
                } else { assert!(!f.target.exists()); }
                assert!(!fs::read_dir(f.target.parent().unwrap()).unwrap().any(|e|e.unwrap().file_name().to_string_lossy().starts_with(".qing-zip-output-")));
                f.archive_intact();
              }
            }
        }

        #[test]
        fn file_publication_cancellation_after_commit_does_not_skip_permission_finalize() {
            use std::cell::Cell;
            let f = Fixture::new(false); let cancelled = Cell::new(false);
            let FilePublication::Committed(file) = f.run(false, &mut || {
                if cancelled.get() { Err("cancelled".into()) } else { Ok(()) }
            }, &mut |step| { if step == Step::AfterCommit { cancelled.set(true); } Ok(()) }).unwrap()
                else { panic!("commit must finish permissions"); };
            assert_eq!(security(&file), f.expected_security); f.archive_intact();
        }

        #[test]
        fn file_publication_rejects_hardlink_and_preserves_unsupported_acl_fallback() {
            let f = Fixture::new(false);
            let alias = f.temp.path().join("alias");
            fs::hard_link(&f.source, &alias).unwrap();
            assert!(
                matches!(f.run(false, &mut || Ok(()), &mut |_| Ok(())), Err(message) if message == "changed publication source")
            );
            f.unchanged();
            assert!(!f.target.exists());
            fs::remove_file(alias).unwrap();
            policy(&open(f.target.parent().unwrap(), 0x60081, 3), true);
            assert!(matches!(
                f.run(false, &mut || Ok(()), &mut |_| Ok(()))
                    .unwrap(),
                FilePublication::Unavailable
            ));
            f.unchanged();
            assert!(!f.target.exists());
            assert!(leaf(OsStr::new("../escape")).is_err());
            assert!(leaf(OsStr::new("target:stream")).is_err());
        }

        #[test]
        fn file_publication_keeps_not_content_indexed_destination_on_copy_route() {
            let f = Fixture::new(false);
            let parent = open(f.target.parent().unwrap(), 0x100, 7);
            attrs(&parent, 0x2010);
            assert!(!supports_file_publication(&f.pins[4]).unwrap());
            assert!(matches!(f.run(false, &mut || Ok(()), &mut |_| Ok(())).unwrap(), FilePublication::Unavailable));
            f.unchanged(); assert!(!f.target.exists());
            attrs(&parent, 0x10);
        }
    }
}
