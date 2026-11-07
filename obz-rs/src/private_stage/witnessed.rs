use super::*;
use std::os::windows::fs::MetadataExt;

// The unprotected legacy NTFS model is eligible only if normal subdirectories
// reproduce their parent's compared security fields. One real directory and one real
// file then cover any depth: a child's parent is in that same observed state.
// Owner/group, DACL, labels, resource attributes, scope and access filters.
// SACL_SECURITY_INFORMATION (0x8) requires audit privilege. Creation under
// the same pinned parent inherits audit ACEs natively; we only change the DACL.
const ACCESS_POLICY_FIELDS: u32 = 0x1f7;
const EXCEPT_DACL: u32 = ACCESS_POLICY_FIELDS & !4;

pub(crate) struct WitnessedInheritance {
    parent_sd: Vec<u8>,
    file_sd: Vec<u8>,
    owner_group: Vec<u8>,
    directory: PrivateDescriptor,
    file: PrivateDescriptor,
}

fn bytes(sd: &Allocation) -> Result<Vec<u8>, String> {
    let size = unsafe { GetSecurityDescriptorLength(sd.0) } as usize;
    if !(20..=1024 * 1024).contains(&size) {
        return Err("invalid security descriptor size".into());
    }
    Ok(unsafe { std::slice::from_raw_parts(sd.0.cast::<u8>(), size) }.to_vec())
}

fn snapshot(file: &File, mask: u32) -> Result<Vec<u8>, String> {
    bytes(&security_descriptor(file, mask)?)
}

fn stable_policy(sd: Handle, user: &str) -> Result<bool, String> {
    let (mut control, mut revision) = (0, 0);
    if unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } == 0 {
        return Err(error("read legacy security controls"));
    }
    // Unprotected, non-null, non-defaulted DACL, no AI/AR. Reject opaque ACEs,
    // no-propagate, and security owners outside our trust boundary.
    if control != 0x8004 || revision != 1 || !ordinary_aces(sd)? {
        return Ok(false);
    }
    #[link(name = "Advapi32")]
    extern "system" {
        fn GetSecurityDescriptorOwner(sd: Handle, owner: *mut Handle, defaulted: *mut i32) -> i32;
    }
    let trusted = |sid: &str| sid == user || matches!(sid, "S-1-5-18" | "S-1-5-32-544");
    let (mut owner, mut defaulted) = (std::ptr::null_mut(), 0);
    if unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted) } == 0
        || owner.is_null()
        || !trusted(&sid_text(owner)?)
    {
        return Ok(false);
    }
    let (mut acl, mut present) = (std::ptr::null_mut(), 0);
    if unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted) } == 0
        || acl.is_null()
    {
        return Ok(false);
    }
    let count = unsafe { std::ptr::read_unaligned((acl as *const u8).add(4).cast::<u16>()) };
    let mut inherited = false;
    for index in 0..count {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(acl, index.into(), &mut ace) } == 0 {
            return Err(error("read legacy ACE"));
        }
        let p = ace.cast::<u8>();
        let size = unsafe { std::ptr::read_unaligned(p.add(2).cast::<u16>()) };
        let flags = unsafe { *p.add(1) };
        inherited |= flags & 0x10 != 0;
        if size < 16 || (flags & 0x10 == 0 && flags != 0) {
            return Ok(false);
        }
        let access = unsafe { std::ptr::read_unaligned(p.add(4).cast::<u32>()) };
        // A shared writer must not be able to rewrite the witness permissions.
        if unsafe { *p } == 0
            && access & (0x10000000 | 0x40000 | 0x80000) != 0
            && !trusted(&sid_text(unsafe { p.add(8).cast() })?)
        {
            return Ok(false);
        }
    }
    Ok(inherited)
}

fn ordinary_storage(handle: &File, directory: bool) -> Result<bool, String> {
    let attributes = handle
        .metadata()
        .map_err(|e| e.to_string())?
        .file_attributes();
    // Never turn compressed, sparse, encrypted, recalled or linked objects
    // into files with different storage semantics through an in-volume rename.
    if (attributes & 0x10 != 0) != directory || attributes & !0xb7 != 0 {
        return Ok(false);
    }
    if !directory {
        return Ok(true);
    }
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            file: Handle,
            class: i32,
            info: *mut u32,
            length: u32,
        ) -> i32;
    }
    let mut flags = 0u32;
    let ok = unsafe { GetFileInformationByHandleEx(handle.as_raw_handle(), 23, &mut flags, 4) };
    Ok(ok != 0 && flags == 0)
}

enum ProbeError {
    Unavailable,
    Occupied(PathBuf),
}
impl From<String> for ProbeError {
    fn from(_: String) -> Self {
        Self::Unavailable
    }
}

fn pending_witness(file: &File, path: &Path) -> Result<(), ProbeError> {
    let mut io = IoStatus {
        status: 0,
        information: 0,
    };
    let remove = 1u8;
    // FILE_DELETE_ON_CLOSE alone still permits new children until close.
    // FileDispositionInformation marks this identity delete-pending now.
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            (&mut io as *mut IoStatus).cast(),
            (&remove as *const u8).cast(),
            1,
            13,
        )
    };
    if status >= 0 {
        return Ok(());
    }
    if unsafe { RtlNtStatusToDosError(status) } == 145 {
        // Nothing in this directory is ours. Never enumerate or delete it.
        return Err(ProbeError::Occupied(path.to_owned()));
    }
    Err(ProbeError::Unavailable)
}

impl WitnessedInheritance {
    pub(crate) fn qualify(parent_path: &Path, parent: &File) -> Result<Option<Self>, String> {
        match Self::try_qualify(parent_path, parent) {
            Ok(result) => Ok(result),
            Err(ProbeError::Unavailable) => Ok(None),
            Err(ProbeError::Occupied(path)) => Err(format!(
                "another process added content to the inheritance witness; directory preserved: {}",
                path.display()
            )),
        }
    }
    fn try_qualify(parent_path: &Path, parent: &File) -> Result<Option<Self>, ProbeError> {
        let token = token()?;
        let user = current_sid(&token)?;
        let source = security_descriptor(parent, ACCESS_POLICY_FIELDS)?;
        if !supports_file_publication(parent)?
            || !ordinary_storage(parent, true)?
            || !stable_policy(source.0, &user)?
        {
            return Ok(None);
        }
        let parent_sd = bytes(&source)?;
        // Leave labeled/conditional policies on ordinary creation.
        if snapshot(parent, 7)? != parent_sd {
            return Ok(None);
        }
        let protected = protected_descriptor(&user)?;
        let owner_group = snapshot(parent, EXCEPT_DACL)?;

        let probe = |directory| -> Result<Option<(PrivateDescriptor, Vec<u8>)>, ProbeError> {
            let mut nonce = [0u8; 16];
            if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 16, 2) } < 0 {
                return Err(ProbeError::Unavailable);
            }
            let leaf = format!(".qing-zip-acl-{:032x}", u128::from_ne_bytes(nonce));
            let witness =
                open_private_child(parent, OsStr::new(&leaf), ChildAccess::Witness(directory))?;
            pending_witness(&witness, &parent_path.join(leaf))?;
            if !ordinary_storage(&witness, directory)?
                || snapshot(&witness, EXCEPT_DACL)? != owner_group
            {
                return Ok(None);
            }
            let normal = snapshot(&witness, ACCESS_POLICY_FIELDS)?;
            if directory && normal != parent_sd {
                return Ok(None);
            }
            // Different volumes materialize effective ACEs with or without
            // INHERITED_ACE. Prove an exact protected-to-ordinary transition
            // on the empty witness. No sampled DACL is applied to archive data.
            for mode in [1, 0] {
                let computed = derive_with(source.0, directory, &token, mode, 0)?;
                if unsafe {
                    NtSetSecurityObject(witness.as_raw_handle(), 4 | 0x80000000, protected.0)
                } < 0
                {
                    return Ok(None);
                }
                apply_native(&witness, &computed)?;
                if snapshot(&witness, ACCESS_POLICY_FIELDS)? == normal {
                    return Ok(Some((computed, normal)));
                }
            }
            Ok(None)
        };
        let Some((directory, _)) = probe(true)? else {
            return Ok(None);
        };
        let Some((file, file_sd)) = probe(false)? else {
            return Ok(None);
        };
        let result = Self {
            parent_sd,
            file_sd,
            owner_group,
            directory,
            file,
        };
        Ok(result.parent_matches(parent)?.then_some(result))
    }
    fn parent_matches(&self, parent: &File) -> Result<bool, String> {
        Ok(ordinary_storage(parent, true)?
            && snapshot(parent, ACCESS_POLICY_FIELDS)? == self.parent_sd)
    }
    pub(crate) fn compatible(
        &self,
        parent: &File,
        root: &File,
        nodes: &[PermissionNode],
    ) -> Result<bool, String> {
        if !self.parent_matches(parent)?
            || !ordinary_storage(root, true)?
            || snapshot(root, EXCEPT_DACL)? != self.owner_group
        {
            return Ok(false);
        }
        for node in nodes {
            if !ordinary_storage(&node.file, node.is_dir)?
                || snapshot(&node.file, EXCEPT_DACL)? != self.owner_group
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn restore(
        &self,
        parent: &File,
        root: &File,
        nodes: &[PermissionNode],
    ) -> Result<(), String> {
        if !self.compatible(parent, root, nodes)? {
            return Err("legacy destination policy or staged identity changed".into());
        }
        let apply = |target: &File, is_dir| -> Result<(), String> {
            let (computed, expected) = if is_dir {
                (&self.directory, &self.parent_sd)
            } else {
                (&self.file, &self.file_sd)
            };
            apply_native(target, computed)?;
            if snapshot(target, ACCESS_POLICY_FIELDS)? != *expected {
                return Err("restored legacy permissions differ from normal creation".into());
            }
            Ok(())
        };
        // NTFS can protect a nonmatching child DACL when its parent's
        // permissions still differ. All verified identities are pinned; restore
        // in the same parent-before-child order as ordinary file creation.
        apply(root, true)?;
        for node in nodes {
            apply(&node.file, node.is_dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::windows::fs::OpenOptionsExt};

    fn pin(path: &Path) -> File {
        File::options()
            .access_mode(0x70081)
            .share_mode(7)
            .custom_flags(0x2200000)
            .open(path)
            .unwrap()
    }
    fn policy(dir: &File, text: &str) {
        let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
        let mut sd = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    1,
                    &mut sd,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let sd = Allocation(sd);
        assert!(unsafe { NtSetSecurityObject(dir.as_raw_handle(), 0x20000004, sd.0) } >= 0);
    }
    #[test]
    fn pending_witness_never_owns_public_children() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("witness");
        let parent = pin(t.path());
        let witness =
            open_private_child(&parent, OsStr::new("witness"), ChildAccess::Witness(true)).unwrap();
        assert!(pending_witness(&witness, &path).is_ok());
        assert!(fs::write(path.join("foreign"), b"foreign").is_err());
        assert!(open_private_child(&witness, OsStr::new("relative-foreign"), ChildAccess::CreateFile).is_err());
        drop(witness);
        assert!(!path.exists());

        // An untrusted creator can win the tiny interval between CREATE and
        // disposition. Preserve its new file rather than cleaning by path.
        let witness =
            open_private_child(&parent, OsStr::new("witness"), ChildAccess::Witness(true)).unwrap();
        fs::write(path.join("foreign"), b"keep").unwrap();
        assert!(
            matches!(pending_witness(&witness, &path), Err(ProbeError::Occupied(ref p)) if p == &path)
        );
        drop(witness);
        assert_eq!(fs::read(path.join("foreign")).unwrap(), b"keep");
    }
    const LEGACY: &str = "D:(A;ID;FA;;;BA)(A;OICIIOID;GA;;;BA)(A;ID;FA;;;SY)(A;OICIIOID;GA;;;SY)(A;ID;0x1301bf;;;AU)(A;OICIIOID;SDGXGWGR;;;AU)(A;ID;0x1200a9;;;BU)(A;OICIIOID;GXGR;;;BU)";
    #[test]
    fn witnessed_tree_matches_normal_metadata_and_future_inheritance() {
        let t = tempfile::tempdir().unwrap();
        // This owner-created fixture alone receives the test policy.
        let policy_root = t.path().join("model");
        fs::create_dir(&policy_root).unwrap();
        policy(&pin(&policy_root), LEGACY);
        let path = policy_root.join("parent");
        fs::create_dir(&path).unwrap();
        let parent = pin(&path);
        assert!(!supports_inheritance(&parent).unwrap());
        let w = WitnessedInheritance::qualify(&path, &parent)
            .unwrap()
            .expect("fixed-point legacy ACL");
        assert_eq!(fs::read_dir(&path).unwrap().count(), 0);
        let normal = path.join("normal");
        let stage = create(&path, &parent).unwrap();
        for base in [&normal, stage.path()] {
            fs::create_dir_all(base.join("a/b")).unwrap();
            fs::write(base.join("a/b/file"), b"data").unwrap();
        }
        let leaf = pin(&stage.path().join("a/b/file"));
        let fixed = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        File::options()
            .write(true)
            .open(stage.path().join("a/b/file"))
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(fixed))
            .unwrap();
        drop(leaf);
        let mut expected = std::collections::BTreeMap::new();
        for (rel, dir, len) in [("a", true, 0), ("a/b", true, 0), ("a/b/file", false, 4)] {
            expected.insert(rel.to_string(), (dir, len));
        }
        let nodes = crate::engine::pin_committed_tree(stage.path(), &expected).unwrap();
        assert!(w.compatible(&parent, stage.root(), &nodes).unwrap());
        w.restore(&parent, stage.root(), &nodes).unwrap();
        drop(nodes);
        for rel in ["a", "a/b", "a/b/file"] {
            let staged = pin(&stage.path().join(rel));
            let ordinary = pin(&normal.join(rel));
            assert_eq!(
                snapshot(&staged, 7).unwrap(),
                snapshot(&ordinary, 7).unwrap(),
                "{rel}"
            );
            assert_eq!(
                staged.metadata().unwrap().file_attributes(),
                ordinary.metadata().unwrap().file_attributes()
            );
        }
        assert_eq!(
            fs::metadata(stage.path().join("a/b/file"))
                .unwrap()
                .modified()
                .unwrap(),
            fixed
        );
        for base in [&normal, stage.path()] {
            fs::create_dir(base.join("a/b/future")).unwrap();
            fs::write(base.join("a/b/future/child"), b"").unwrap();
        }
        for rel in ["a/b/future", "a/b/future/child"] {
            assert_eq!(
                snapshot(&pin(&stage.path().join(rel)), 7).unwrap(),
                snapshot(&pin(&normal.join(rel)), 7).unwrap()
            );
        }
        policy(&parent, "D:(A;ID;FA;;;WD)(A;OICIIOID;GA;;;WD)");
        assert!(!w.parent_matches(&parent).unwrap());
        assert!(WitnessedInheritance::qualify(&path, &parent)
            .unwrap()
            .is_none());
    }
}
