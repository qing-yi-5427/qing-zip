//! Optional local NTFS transaction leases for verified cross-volume moves.
//!
//! Unlike oplocks, TxF rejects even native FILE_COMPLETE_IF_OPLOCKED EA writers.
//! It is unavailable on some volumes/configurations: callers MUST retain sources
//! when beginning or acquiring a lease fails. Never enable TxF or change policy.
//! https://learn.microsoft.com/windows/win32/fileio/programming-considerations-for-transacted-fileio-
use super::{directory_identity, File, Path};
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
    io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use std::{ffi::c_void, marker::PhantomData};
type Handle = *mut c_void;
#[link(name = "ktmw32")]
unsafe extern "system" {
    fn CreateTransaction(
        attrs: Handle,
        guid: Handle,
        options: u32,
        isolation: u32,
        flags: u32,
        timeout: u32,
        description: *const u16,
    ) -> Handle;
    fn CommitTransaction(transaction: Handle) -> i32;
    fn RollbackTransaction(transaction: Handle) -> i32;
    fn GetTransactionInformation(
        transaction: Handle,
        outcome: *mut u32,
        isolation: *mut u32,
        flags: *mut u32,
        timeout: *mut u32,
        description_size: u32,
        description: *mut u16,
    ) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateFileTransactedW(
        name: *const u16,
        access: u32,
        share: u32,
        security: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
        transaction: Handle,
        version: Handle,
        extended: Handle,
    ) -> Handle;
    fn SetFileInformationByHandle(file: Handle, class: i32, data: *const c_void, size: u32) -> i32;
}
fn error(action: &str) -> String {
    format!("{action}：{}", std::io::Error::last_os_error())
}

pub(super) struct Transaction {
    handle: OwnedHandle,
    finished: bool,
}
impl Transaction {
    pub(super) fn begin() -> Result<Self, String> {
        let description: Vec<u16> = "qing-zip verified local move"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let handle = unsafe {
            CreateTransaction(
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                description.as_ptr(),
            )
        };
        if handle.is_null() || handle as isize == -1 {
            return Err(error("无法建立文件事务，保留来源"));
        }
        Ok(Self {
            handle: unsafe { OwnedHandle::from_raw_handle(handle) },
            finished: false,
        })
    }

    /// Ancestors must remain independently pinned by the caller. The expected
    /// identity comes from the original verified source / published target pin.
    pub(super) fn lock<'a>(
        &'a self,
        path: &Path,
        expected: (u32, u64),
    ) -> Result<Locked<'a>, String> {
        self.lock_access(path, expected, true)
    }
    pub(super) fn lock_target<'a>(
        &'a self,
        path: &Path,
        expected: (u32, u64),
    ) -> Result<Locked<'a>, String> {
        self.lock_access(path, expected, false)
    }
    fn lock_access<'a>(
        &'a self,
        path: &Path,
        expected: (u32, u64),
        deletable: bool,
    ) -> Result<Locked<'a>, String> {
        if !path.is_absolute() {
            return Err("文件事务需要绝对路径".into());
        }
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err("文件路径含无效字符".into());
        }
        wide.push(0);
        let raw = unsafe {
            CreateFileTransactedW(
                wide.as_ptr(),
                0x80000100 | if deletable { 0x10000 } else { 0 },
                7,
                std::ptr::null_mut(),
                3,
                0x02200000,
                std::ptr::null_mut(),
                self.handle.as_raw_handle(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if raw.is_null() || raw as isize == -1 {
            return Err(error("无法固定来源或目标的事务视图，保留来源"));
        }
        let transactional = unsafe { File::from_raw_handle(raw) };
        let meta = crate::private_stage::checked_metadata(&transactional)?;
        if meta.file_attributes() & 0x4400 != 0
            || (!meta.is_file() && !meta.is_dir())
            || directory_identity(&transactional)? != expected
        {
            return Err("事务对象身份或类型改变，保留来源".into());
        }
        // BackupRead on a transacted handle silently omits BACKUP_EA_DATA.
        // The transaction's write lease blocks external writers, so read the
        // unchanged committed view with a second, identity-checked read handle.
        let reader = File::options()
            .read(true)
            .share_mode(5)
            .custom_flags(0x02200000)
            .open(path)
            .map_err(|e| format!("无法读取已固定的事务对象：{e}"))?;
        let read_meta = crate::private_stage::checked_metadata(&reader)?;
        if directory_identity(&reader)? != expected
            || read_meta.is_dir() != meta.is_dir()
            || read_meta.file_attributes() & 0x4400 != 0
        {
            return Err("事务读取对象被替换，保留来源".into());
        }
        Ok(Locked {
            transactional,
            reader,
            deletable,
            _transaction: PhantomData,
        })
    }

    fn outcome(&self) -> Option<u32> {
        let (mut outcome, mut isolation, mut flags, mut timeout) = (0, 0, 0, 0);
        let ok = unsafe {
            GetTransactionInformation(
                self.handle.as_raw_handle(),
                &mut outcome,
                &mut isolation,
                &mut flags,
                &mut timeout,
                0,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(outcome)
    }
    pub(super) fn commit(mut self) -> Result<(), String> {
        // Locked<'_> borrows self: all leased file handles must have been closed
        // before this consuming call. TxF retains write exclusion until commit.
        let committed = unsafe { CommitTransaction(self.handle.as_raw_handle()) } != 0;
        let failure = error("文件事务提交失败");
        if committed || self.outcome() == Some(2) {
            self.finished = true;
            return Ok(());
        }
        self.rollback_inner()
            .map_err(|rollback| format!("{failure}；{rollback}"))?;
        Err(format!("{failure}；删除已回滚，来源仍保留"))
    }
    fn rollback_inner(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }
        let rolled_back = unsafe { RollbackTransaction(self.handle.as_raw_handle()) } != 0;
        let failure = error("无法确认事务回滚，请核查来源和目标");
        if rolled_back || self.outcome() == Some(3) {
            self.finished = true;
            Ok(())
        } else {
            Err(failure)
        }
    }
    pub(super) fn rollback(mut self) -> Result<(), String> {
        self.rollback_inner()
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback_inner();
        }
    }
}

pub(super) struct Locked<'a> {
    transactional: File,
    reader: File,
    deletable: bool,
    _transaction: PhantomData<&'a Transaction>,
}
impl Locked<'_> {
    pub(super) fn reader(&self) -> &File {
        &self.reader
    }

    /// The exact leased object is marked inside the transaction. No pathname is
    /// reopened for deletion; nonempty directories fail without removing a child.
    pub(super) fn delete(self) -> Result<(), String> {
        if !self.deletable {
            return Err("目标租约不能用于删除".into());
        }
        let meta = self.transactional.metadata().map_err(|e| e.to_string())?;
        if meta.file_attributes() & 1 != 0 {
            // Readonly is cleared only in the private transactional view. Abort
            // restores it; neither the source ACL nor the published target changes.
            let attributes = meta.file_attributes() & 0x3126;
            let basic = [
                0u64,
                0,
                0,
                0,
                u64::from(if attributes == 0 { 0x80 } else { attributes }),
            ];
            if unsafe {
                SetFileInformationByHandle(
                    self.transactional.as_raw_handle(),
                    0,
                    basic.as_ptr().cast(),
                    40,
                )
            } == 0
            {
                return Err(error("无法在事务内处理只读来源"));
            }
        }
        let delete = 1u32;
        if unsafe {
            SetFileInformationByHandle(
                self.transactional.as_raw_handle(),
                4,
                (&delete as *const u32).cast(),
                4,
            )
        } == 0
        {
            return Err(error("无法在事务内删除已验证对象，保留来源"));
        }
        // Both handles close here. This makes the deletion eligible for commit,
        // but is reversible until Transaction::commit finishes successfully.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read};
    fn identity(path: &Path) -> (u32, u64) {
        let f = File::options()
            .read(true)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(path)
            .unwrap();
        directory_identity(&f).unwrap()
    }
    #[test]
    fn committed_delete_and_rollback_bind_exact_original_file() {
        for commit in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("原件.txt");
            fs::write(&path, b"original").unwrap();
            let tx = Transaction::begin().unwrap();
            let locked = tx.lock(&path, identity(&path)).unwrap();
            let mut bytes = Vec::new();
            (&*locked.reader()).read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, b"original");
            locked.delete().unwrap();
            assert!(path.exists());
            if commit {
                tx.commit().unwrap()
            } else {
                tx.rollback().unwrap()
            }
            assert_eq!(path.exists(), !commit);
            if !commit {
                assert_eq!(fs::read(path).unwrap(), b"original")
            }
        }
    }
    #[test]
    fn mismatched_identity_and_existing_ea_writer_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, b"original").unwrap();
        let id = identity(&path);
        let tx = Transaction::begin().unwrap();
        assert!(tx.lock(&path, (id.0, id.1 + 1)).is_err());
        tx.rollback().unwrap();
        let writer = File::options()
            .access_mode(0x18)
            .share_mode(7)
            .open(&path)
            .unwrap();
        let tx = Transaction::begin().unwrap();
        assert!(tx.lock(&path, id).is_err());
        tx.rollback().unwrap();
        drop(writer);
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
    #[test]
    fn lease_excludes_writers_after_file_handles_close_until_transaction_end() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, b"original").unwrap();
        let tx = Transaction::begin().unwrap();
        let locked = tx.lock(&path, identity(&path)).unwrap();
        for access in [0x18, 0x100, 0x40000000] {
            assert!(File::options()
                .access_mode(access)
                .share_mode(7)
                .open(&path)
                .is_err());
        }
        drop(locked);
        assert!(File::options()
            .access_mode(0x18)
            .share_mode(7)
            .open(&path)
            .is_err());
        tx.rollback().unwrap();
        assert!(File::options()
            .access_mode(0x18)
            .share_mode(7)
            .open(&path)
            .is_ok());
    }
    #[test]
    fn target_lease_coexists_with_strong_directory_pin_but_excludes_ea_writers() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("directory");
        fs::create_dir(&path).unwrap();
        let parent_pin = File::options()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(&path)
            .unwrap();
        let tx = Transaction::begin().unwrap();
        let locked = tx
            .lock_target(&path, directory_identity(&parent_pin).unwrap())
            .unwrap();
        assert_eq!(
            directory_identity(&locked.transactional).unwrap(),
            directory_identity(locked.reader()).unwrap()
        );
        assert!(File::options()
            .access_mode(0x18)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(&path)
            .is_err());
        drop(locked);
        assert!(File::options()
            .access_mode(0x18)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(&path)
            .is_err());
        tx.rollback().unwrap();
    }
    #[test]
    fn native_complete_if_oplocked_does_not_bypass_transaction() {
        #[repr(C)]
        struct Unicode {
            length: u16,
            max: u16,
            text: *mut u16,
        }
        #[repr(C)]
        struct Attributes {
            length: u32,
            root: Handle,
            name: *mut Unicode,
            flags: u32,
            sd: Handle,
            qos: Handle,
        }
        #[repr(C)]
        struct Status {
            status: usize,
            information: usize,
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtCreateFile(
                out: *mut Handle,
                access: u32,
                attrs: *mut Attributes,
                status: *mut Status,
                size: *const i64,
                attributes: u32,
                share: u32,
                disposition: u32,
                options: u32,
                ea: Handle,
                ea_len: u32,
            ) -> i32;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, b"original").unwrap();
        let tx = Transaction::begin().unwrap();
        let locked = tx.lock(&path, identity(&path)).unwrap();
        let text = format!("\\??\\{}", path.display());
        let mut text = text.encode_utf16().collect::<Vec<_>>();
        let mut name = Unicode {
            length: (text.len() * 2) as u16,
            max: (text.len() * 2) as u16,
            text: text.as_mut_ptr(),
        };
        let mut attrs = Attributes {
            length: std::mem::size_of::<Attributes>() as u32,
            root: std::ptr::null_mut(),
            name: &mut name,
            flags: 0,
            sd: std::ptr::null_mut(),
            qos: std::ptr::null_mut(),
        };
        for access in [0x18, 0x100, 2] {
            let mut status = Status {
                status: 0,
                information: 0,
            };
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                NtCreateFile(
                    &mut handle,
                    access,
                    &mut attrs,
                    &mut status,
                    std::ptr::null(),
                    0,
                    7,
                    1,
                    0x4100,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if result >= 0 && !handle.is_null() {
                drop(unsafe { OwnedHandle::from_raw_handle(handle) });
            }
            assert!(
                result < 0,
                "native access {access:#x} must not bypass transaction"
            );
        }
        drop(locked);
        tx.rollback().unwrap();
        assert_eq!(fs::read(path).unwrap(), b"original");
    }
    #[test]
    fn existing_writable_mapping_without_file_handle_blocks_transaction() {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateFileMappingW(
                file: Handle,
                attrs: Handle,
                protect: u32,
                high: u32,
                low: u32,
                name: *const u16,
            ) -> Handle;
            fn MapViewOfFile(
                mapping: Handle,
                access: u32,
                high: u32,
                low: u32,
                size: usize,
            ) -> Handle;
            fn UnmapViewOfFile(view: *const c_void) -> i32;
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, b"original").unwrap();
        let id = identity(&path);
        let writer = File::options()
            .read(true)
            .write(true)
            .share_mode(7)
            .open(&path)
            .unwrap();
        let raw = unsafe {
            CreateFileMappingW(
                writer.as_raw_handle(),
                std::ptr::null_mut(),
                4,
                0,
                0,
                std::ptr::null(),
            )
        };
        assert!(!raw.is_null());
        let mapping = unsafe { OwnedHandle::from_raw_handle(raw) };
        let view = unsafe { MapViewOfFile(mapping.as_raw_handle(), 2, 0, 0, 0) };
        assert!(!view.is_null());
        struct View(Handle);
        impl Drop for View {
            fn drop(&mut self) {
                unsafe {
                    UnmapViewOfFile(self.0);
                }
            }
        }
        let view = View(view);
        drop(writer);
        drop(mapping);
        let tx = Transaction::begin().unwrap();
        assert!(tx.lock(&path, id).is_err());
        tx.rollback().unwrap();
        unsafe {
            std::ptr::copy_nonoverlapping(b"changed!".as_ptr(), view.0.cast(), 8);
        }
        drop(view);
        assert_eq!(fs::read(path).unwrap(), b"changed!");
    }
    #[test]
    fn readonly_delete_is_transactional_and_drop_rolls_back() {
        for commit in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("readonly");
            fs::write(&path, b"original").unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions).unwrap();
            let tx = Transaction::begin().unwrap();
            let locked = tx.lock(&path, identity(&path)).unwrap();
            locked.delete().unwrap();
            if commit {
                tx.commit().unwrap()
            } else {
                drop(tx)
            }
            assert_eq!(path.exists(), !commit);
            if !commit {
                assert!(fs::metadata(&path).unwrap().permissions().readonly());
                let mut p = fs::metadata(&path).unwrap().permissions();
                p.set_readonly(false);
                fs::set_permissions(&path, p).unwrap();
            }
        }
    }
    #[test]
    fn directory_delete_never_removes_late_children() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("directory");
        fs::create_dir(&path).unwrap();
        let tx = Transaction::begin().unwrap();
        let locked = tx.lock(&path, identity(&path)).unwrap();
        fs::write(path.join("late"), b"foreign").unwrap();
        assert!(locked.delete().is_err());
        tx.rollback().unwrap();
        assert_eq!(fs::read(path.join("late")).unwrap(), b"foreign");
        fs::remove_file(path.join("late")).unwrap();
        let tx = Transaction::begin().unwrap();
        tx.lock(&path, identity(&path)).unwrap().delete().unwrap();
        assert!(fs::write(path.join("later"), b"foreign").is_err());
        tx.rollback().unwrap();
        assert!(path.is_dir());
    }
}
