//! Isolated EFS candidate. Not routed from file-workspace until publication and
//! Windows CI acceptance are complete. Never decrypt to an ordinary temp file.
use super::{JobControl, file_workspace_compression, file_workspace_ea};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::c_void,
    fs::File,
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
};
type Handle = *mut c_void;
#[repr(C)]
struct Status {
    status: usize,
    information: usize,
}
#[repr(C)]
struct HashBlob {
    size: u32,
    data: *const u8,
}
#[repr(C)]
struct CertificateHash {
    size: u32,
    sid: Handle,
    hash: *const HashBlob,
    display: *const u16,
}
#[repr(C)]
struct HashList {
    count: u32,
    items: *const *const CertificateHash,
}
type Progress =
    unsafe extern "system" fn(i64, i64, i64, i64, u32, u32, Handle, Handle, Handle) -> u32;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandle(file: Handle, info: *mut u32) -> i32;
    fn GetFileInformationByHandleEx(file: Handle, class: i32, data: *mut u32, size: u32) -> i32;
    fn SetFileInformationByHandle(file: Handle, class: i32, data: *const c_void, size: u32) -> i32;
    fn GetFinalPathNameByHandleW(file: Handle, path: *mut u16, size: u32, flags: u32) -> u32;
    fn ReOpenFile(file: Handle, access: u32, share: u32, flags: u32) -> Handle;
    fn CopyFileExW(
        source: *const u16,
        target: *const u16,
        progress: Option<Progress>,
        context: Handle,
        cancel: *mut i32,
        flags: u32,
    ) -> i32;
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
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn QueryUsersOnEncryptedFile(path: *const u16, list: *mut *mut HashList) -> u32;
    fn QueryRecoveryAgentsOnEncryptedFile(path: *const u16, list: *mut *mut HashList) -> u32;
    fn FreeEncryptionCertificateHashList(list: *mut HashList);
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryEaFile(
        file: Handle,
        status: *mut Status,
        buffer: *mut u8,
        size: u32,
        single: u8,
        names: *const u8,
        names_size: u32,
        index: *const u32,
        restart: u8,
    ) -> i32;
    fn NtSetEaFile(file: Handle, status: *mut Status, buffer: *const u8, size: u32) -> i32;
}
fn error(context: &str) -> String {
    format!("{context}：{}", std::io::Error::last_os_error())
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn identity(handle: Handle) -> Result<[u32; 3], String> {
    let mut info = [0u32; 13];
    if unsafe { GetFileInformationByHandle(handle, info.as_mut_ptr()) } == 0 {
        return Err(error("读取EFS对象身份"));
    }
    if info[0] & 0x410 != 0 || info[10] != 1 {
        return Err("EFS候选仅支持无链接普通文件".into());
    }
    Ok([info[7], info[11], info[12]])
}
fn path(file: &File) -> Result<PathBuf, String> {
    let mut text = vec![0u16; 32768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            text.as_mut_ptr(),
            text.len() as u32,
            0,
        )
    };
    if count == 0 || count as usize >= text.len() {
        return Err(error("读取EFS固定路径"));
    }
    Ok(String::from_utf16(&text[..count as usize])
        .map_err(|_| "EFS路径编码无效")?
        .into())
}
fn keys(path: &Path, recovery: bool) -> Result<Vec<Vec<u8>>, String> {
    let path = wide(path);
    let mut raw = std::ptr::null_mut();
    let status = unsafe {
        if recovery {
            QueryRecoveryAgentsOnEncryptedFile(path.as_ptr(), &mut raw)
        } else {
            QueryUsersOnEncryptedFile(path.as_ptr(), &mut raw)
        }
    };
    if status != 0 {
        return Err(format!("读取EFS授权证书失败：{status}"));
    }
    struct Free(*mut HashList);
    impl Drop for Free {
        fn drop(&mut self) {
            unsafe { FreeEncryptionCertificateHashList(self.0) }
        }
    }
    let _free = Free(raw);
    if raw.is_null() {
        return Err("EFS证书列表为空指针".into());
    }
    let list = unsafe { &*raw };
    if list.count > 1024 || (list.count != 0 && list.items.is_null()) {
        return Err("EFS证书列表无效".into());
    }
    let mut result = Vec::new();
    for index in 0..list.count as usize {
        let item = unsafe { *list.items.add(index) };
        if item.is_null() || unsafe { (*item).hash.is_null() } {
            return Err("EFS证书数据无效".into());
        }
        let hash = unsafe { &*(*item).hash };
        if hash.size == 0 || hash.size > 1024 || hash.data.is_null() {
            return Err("EFS证书哈希无效".into());
        }
        result.push(unsafe { std::slice::from_raw_parts(hash.data, hash.size as usize) }.to_vec());
    }
    result.sort();
    Ok(result)
}
fn ea(file: &File) -> Result<Vec<u8>, String> {
    let mut words = vec![0u64; file_workspace_ea::MAX_EA_BYTES / 8];
    let mut status = Status {
        status: 0,
        information: 0,
    };
    let code = unsafe {
        NtQueryEaFile(
            file.as_raw_handle(),
            &mut status,
            words.as_mut_ptr().cast(),
            (words.len() * 8) as u32,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            1,
        )
    };
    if code as u32 == 0xc0000052 {
        return Ok(Vec::new());
    }
    if code != 0 || status.information > words.len() * 8 {
        return Err(format!("读取EFS扩展属性失败：{code:#x}"));
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), status.information) }
            .to_vec();
    file_workspace_ea::validate(0, 0, &bytes, || Ok(()))?;
    Ok(bytes)
}
fn set_ea(file: &File, bytes: &[u8]) -> Result<(), String> {
    file_workspace_ea::validate(0, 0, bytes, || Ok(()))?;
    let mut aligned = vec![0u32; bytes.len().div_ceil(4)];
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            aligned.as_mut_ptr().cast::<u8>(),
            bytes.len(),
        );
    }
    let mut status = Status {
        status: 0,
        information: 0,
    };
    let code = unsafe {
        NtSetEaFile(
            file.as_raw_handle(),
            &mut status,
            aligned.as_ptr().cast(),
            bytes.len() as u32,
        )
    };
    if code != 0 {
        return Err(format!("EFS私有目标扩展属性恢复失败：{code:#x}"));
    }
    Ok(())
}
fn stream_names(file: &File) -> Result<Vec<(Vec<u16>, u64)>, String> {
    let mut words = vec![0u64; 512];
    loop {
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                7,
                words.as_mut_ptr().cast(),
                (words.len() * 8) as u32,
            )
        } != 0
        {
            break;
        }
        if !matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(122 | 234)
        ) || words.len() >= 2 * 1024 * 1024
        {
            return Err(error("枚举EFS全部数据流"));
        }
        words.resize(words.len() * 2, 0);
    }
    let bytes = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * 8) };
    let mut offset = 0;
    let mut names = BTreeSet::new();
    let mut result = Vec::new();
    loop {
        let head = bytes.get(offset..offset + 24).ok_or("EFS数据流记录截断")?;
        let next = u32::from_le_bytes(head[..4].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(head[8..16].try_into().unwrap());
        if length == 0 || length % 2 != 0 || size > i64::MAX as u64 {
            return Err("EFS数据流记录长度无效".into());
        }
        let name = bytes
            .get(offset + 24..offset + 24 + length)
            .ok_or("EFS数据流名称截断")?
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect::<Vec<_>>();
        let text = String::from_utf16(&name).map_err(|_| "EFS数据流名称编码无效")?;
        let inner = text
            .strip_prefix(':')
            .and_then(|v| v.strip_suffix(":$DATA"))
            .ok_or("EFS含不支持的数据流类型")?;
        if inner.chars().any(|c| matches!(c, ':' | '/' | '\\' | '\0')) || !names.insert(text) {
            return Err("EFS数据流名称重复或无效".into());
        }
        result.push((name, size));
        if result.len() > 4096 {
            return Err("EFS数据流过多".into());
        }
        if next == 0 {
            break;
        }
        if next < 24 + length || next % 8 != 0 || next > bytes.len() - offset {
            return Err("EFS数据流链无效".into());
        }
        offset += next;
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}
struct Snapshot {
    digest: [u8; 32],
    ea: Vec<u8>,
    _pins: Vec<File>,
}
fn snapshot(file: &File, ctl: &JobControl) -> Result<Snapshot, String> {
    ctl.check()?;
    identity(file.as_raw_handle())?;
    let encrypted = file
        .metadata()
        .map_err(|e| e.to_string())?
        .file_attributes()
        & 0x4000
        != 0;
    #[cfg(test)]
    let encrypted = encrypted
        && !(TARGET_STATE_FAULT.with(|v| v.get()) == 1
            && path(file)?
                .file_name()
                .is_some_and(|n| n == "encrypted-native-output"));
    if !encrypted {
        return Err("目标未保持EFS加密，拒绝发布".into());
    }
    let names = stream_names(file)?;
    let mut pins = Vec::new();
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    for (name, size) in &names {
        ctl.check()?;
        let pin = if name == &"::$DATA".encode_utf16().collect::<Vec<_>>() {
            file.try_clone().map_err(|e| e.to_string())?
        } else {
            let opened = file_workspace_compression::stream(file, name, false)?;
            let raw = unsafe { ReOpenFile(opened.as_raw_handle(), 0x80000000, 1, 0x00200000) };
            if raw as isize == -1 {
                return Err(error("固定EFS数据流"));
            }
            unsafe { File::from_raw_handle(raw) }
        };
        if identity(pin.as_raw_handle())? != identity(file.as_raw_handle())? {
            return Err("EFS数据流对象身份不一致".into());
        }
        hash.update((name.len() as u64).to_le_bytes());
        for unit in name {
            hash.update(unit.to_le_bytes());
        }
        hash.update(size.to_le_bytes());
        let mut offset = 0;
        while offset < *size {
            ctl.check()?;
            let amount = (*size - offset).min(buffer.len() as u64) as usize;
            let count = pin
                .seek_read(&mut buffer[..amount], offset)
                .map_err(|e| e.to_string())?;
            if count == 0 {
                return Err("EFS数据流提前结束".into());
            }
            hash.update(&buffer[..count]);
            offset += count as u64;
        }
        if pin.metadata().map_err(|e| e.to_string())?.len() != *size {
            return Err("EFS数据流长度变化".into());
        }
        pins.push(pin);
    }
    if stream_names(file)? != names {
        return Err("EFS数据流清单变化".into());
    }
    let ea = ea(file)?;
    hash.update((ea.len() as u64).to_le_bytes());
    hash.update(&ea);
    let path = path(file)?;
    for recovery in [false, true] {
        let keys = keys(&path, recovery)?;
        #[cfg(test)]
        let keys = if !recovery
            && TARGET_STATE_FAULT.with(|v| v.get()) == 2
            && path
                .file_name()
                .is_some_and(|n| n == "encrypted-native-output")
        {
            Vec::new()
        } else {
            keys
        };
        if !recovery && keys.is_empty() {
            return Err("EFS缺少授权用户证书".into());
        }
        hash.update([u8::from(recovery)]);
        hash.update((keys.len() as u64).to_le_bytes());
        for key in keys {
            hash.update((key.len() as u64).to_le_bytes());
            hash.update(key);
        }
    }
    Ok(Snapshot {
        digest: hash.finalize().into(),
        ea,
        _pins: pins,
    })
}

struct CopyContext<'a> {
    source: [u32; 3],
    destination: Option<[u32; 3]>,
    parent: &'a File,
    ctl: &'a JobControl,
    failed: Option<String>,
}
unsafe extern "system" fn progress(
    _a: i64,
    _b: i64,
    _c: i64,
    _d: i64,
    _n: u32,
    _reason: u32,
    source: Handle,
    target: Handle,
    context: Handle,
) -> u32 {
    let state = unsafe { &mut *context.cast::<CopyContext<'_>>() };
    #[cfg(test)]
    CANCEL_NATIVE.with(|cancel| {
        if cancel.replace(false) {
            state.ctl.cancel()
        }
    });
    if state.ctl.check().is_err() || identity(source).ok() != Some(state.source) {
        state.failed = Some("EFS复制取消或源身份变化".into());
        return 1;
    }
    let Ok(id) = identity(target) else {
        state.failed = Some("EFS原生目标身份无效".into());
        return 1;
    };
    let borrowed = std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(target) });
    if let Err(error) = crate::private_stage::verify_native_private_child(state.parent, &borrowed) {
        state.failed = Some(error);
        return 1;
    }
    if state.destination.is_some_and(|old| old != id) {
        state.failed = Some("EFS原生目标身份变化".into());
        return 1;
    }
    state.destination = Some(id);
    0
}

/// Prepared object stays within its protected stage. No publication API yet.
struct NativeFile(File);
impl std::ops::Deref for NativeFile {
    type Target = File;
    fn deref(&self) -> &File {
        &self.0
    }
}
impl Drop for NativeFile {
    fn drop(&mut self) {
        // This handle belongs exclusively to our native-created private output.
        // Never apply this cleanup to the source or a path-resolved replacement.
        if let Ok(metadata) = self.0.metadata() {
            if metadata.file_attributes() & 1 != 0 {
                let attrs = (metadata.file_attributes() & !1) as u64;
                let basic = [0u64, 0, 0, 0, if attrs == 0 { 0x80 } else { attrs }];
                unsafe {
                    SetFileInformationByHandle(
                        self.0.as_raw_handle(),
                        0,
                        basic.as_ptr().cast(),
                        40,
                    );
                }
            }
        }
        let delete = 1u8;
        unsafe {
            SetFileInformationByHandle(self.0.as_raw_handle(), 4, (&delete as *const u8).cast(), 1);
        }
    }
}
pub(crate) struct Prepared {
    file: NativeFile,
    stage: crate::private_stage::PrivateStage,
    source: crate::engine_stream::PinnedSource,
    expected: Snapshot,
    #[cfg(test)]
    callback_seen: bool,
}
impl Prepared {
    pub(crate) fn verify(&self, ctl: &JobControl) -> Result<(), String> {
        crate::private_stage::verify_native_private_child(self.stage.root(), &self.file)?;
        if snapshot(&self.source, ctl)?.digest != self.expected.digest
            || snapshot(&self.file, ctl)?.digest != self.expected.digest
        {
            return Err("EFS源或私有目标发生变化".into());
        }
        Ok(())
    }
}
pub(crate) fn prepare(
    source: &Path,
    parent: &Path,
    parent_pin: &File,
    ctl: &JobControl,
) -> Result<Prepared, String> {
    ctl.check()?;
    let source = crate::engine_stream::pin(source)?;
    let expected = snapshot(&source, ctl)?;
    let mut flags = 0;
    let callback = Some(progress as Progress);
    #[cfg(test)]
    let callback = if NO_PROGRESS.with(|v| v.get()) {
        None
    } else {
        callback
    };
    if unsafe {
        GetVolumeInformationByHandleW(
            parent_pin.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut flags,
            std::ptr::null_mut(),
            0,
        )
    } == 0
        || flags & 0x20000 == 0
    {
        return Err("目标卷不支持EFS加密，源保持不变".into());
    }
    let stage = crate::private_stage::create(parent, parent_pin)?;
    let target = stage.path().join("encrypted-native-output");
    let source_path = wide(&path(&source)?);
    let target_path = wide(&target);
    let mut state = CopyContext {
        source: identity(source.as_raw_handle())?,
        destination: None,
        parent: stage.root(),
        ctl,
        failed: None,
    };
    let mut cancelled = 0;
    if unsafe {
        CopyFileExW(
            source_path.as_ptr(),
            target_path.as_ptr(),
            callback,
            (&mut state as *mut CopyContext<'_>).cast(),
            &mut cancelled,
            1,
        )
    } == 0
        || state.failed.is_some()
    {
        return Err(state
            .failed
            .unwrap_or_else(|| error("EFS原生复制失败，源保持不变")));
    }
    ctl.check()?;
    let file = NativeFile(
        File::options()
            .access_mode(0x170199)
            .share_mode(0)
            .custom_flags(0x02200000)
            .open(&target)
            .map_err(|e| e.to_string())?,
    );
    crate::private_stage::verify_native_private_child(stage.root(), &file)?;
    if state
        .destination
        .is_some_and(|expected| identity(file.as_raw_handle()).ok() != Some(expected))
    {
        return Err("EFS原生目标身份变化".into());
    }
    if !expected.ea.is_empty() {
        set_ea(&file, &expected.ea)?;
    }
    #[cfg(test)]
    let callback_seen=state.destination.is_some();
    let prepared = Prepared {
        file,
        stage,
        source,
        expected,
        #[cfg(test)]
        callback_seen,
    };
    prepared.verify(ctl)?;
    Ok(prepared)
}

#[cfg(test)]
thread_local! {
    static CANCEL_NATIVE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
    static TARGET_STATE_FAULT:std::cell::Cell<u8>=const{std::cell::Cell::new(0)};
    static NO_PROGRESS:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parent(path: &Path) -> File {
        File::options()
            .access_mode(0x200a0)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(path)
            .unwrap()
    }
    #[test]
    fn plaintext_is_not_routed_to_efs_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");
        std::fs::write(&source, b"plain").unwrap();
        std::fs::create_dir(&dest).unwrap();
        assert!(prepare(&source, &dest, &parent(&dest), &JobControl::new()).is_err());
        assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
    }
    fn encrypt_ci_fixture(path: &Path) {
        assert_eq!(
            std::env::var("GITHUB_ACTIONS").as_deref(),
            Ok("true"),
            "EFS certificates are forbidden on a developer machine"
        );
        assert_eq!(
            std::env::var("RUNNER_ENVIRONMENT").as_deref(),
            Ok("github-hosted")
        );
        assert_eq!(
            std::env::var("GITHUB_REPOSITORY").as_deref(),
            Ok("qing-yi-5427/qing-zip")
        );
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn EncryptFileW(path: *const u16) -> i32;
        }
        assert_ne!(
            unsafe { EncryptFileW(wide(path).as_ptr()) },
            0,
            "{}",
            error("CI EFS unsupported")
        );
    }
    #[test]
    #[ignore = "creates an EFS certificate only in disposable GitHub-hosted CI"]
    fn ci_efs_candidate_preserves_complete_streams_ea_keys_and_private_identity() {
        for size in [0, 1024 * 1024] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let dest = temp.path().join("dest");
            std::fs::write(&source, vec![0x51; size]).unwrap();
            std::fs::create_dir(&dest).unwrap();
            for name in ["alpha", "beta", "Zone.Identifier", "中文"] {
                std::fs::write(format!("{}:{name}", source.display()), name.as_bytes()).unwrap();
            }
            encrypt_ci_fixture(&source);
            let expected_ea = {
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(&source)
                    .unwrap();
                let payload = [0, 0, 0, 0, 0x80, 1, 3, 0, b'A', 0, 1, 2, 3];
                set_ea(&file, &payload).unwrap();
                ea(&file).unwrap()
            };
            let prepared = prepare(&source, &dest, &parent(&dest), &JobControl::new()).unwrap();
            prepared.verify(&JobControl::new()).unwrap();
            assert_eq!(ea(&prepared.file).unwrap(), expected_ea);
            assert_eq!(stream_names(&prepared.file).unwrap().len(), 5);
            let private_path = prepared.stage.path().to_owned();
            assert!(private_path.is_dir());
            drop(prepared);
            assert!(!private_path.exists());
            assert!(source.exists());
        }
    }
    #[test]
    #[ignore = "creates an EFS certificate only in disposable GitHub-hosted CI"]
    fn ci_efs_native_cancel_keeps_source_and_no_partial_output() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let dest = temp.path().join("dest");
        std::fs::write(&source, vec![0x51; 1024 * 1024]).unwrap();
        std::fs::create_dir(&dest).unwrap();
        encrypt_ci_fixture(&source);
        CANCEL_NATIVE.with(|value| value.set(true));
        assert!(prepare(&source, &dest, &parent(&dest), &JobControl::new()).is_err());
        assert!(
            !CANCEL_NATIVE.with(|value| value.replace(false)),
            "native callback must actually run"
        );
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        assert_eq!(std::fs::read(&source).unwrap(), vec![0x51; 1024 * 1024]);
    }
    #[test]
    #[ignore = "creates EFS only in disposable CI; encryption/key failure uses explicit fault injection"]
    fn ci_efs_target_encryption_and_key_rejection_discards_private_output() {
        for fault in [1, 2] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let dest = temp.path().join("dest");
            std::fs::write(&source, b"real encrypted source").unwrap();
            std::fs::create_dir(&dest).unwrap();
            encrypt_ci_fixture(&source);
            TARGET_STATE_FAULT.with(|v| v.set(fault));
            let result = prepare(&source, &dest, &parent(&dest), &JobControl::new());
            TARGET_STATE_FAULT.with(|v| v.set(0));
            let error = match result {
                Ok(_) => panic!("injected EFS downgrade was accepted"),
                Err(e) => e,
            };
            assert!(
                error.contains(if fault == 1 {
                    "未保持EFS加密"
                } else {
                    "缺少授权用户证书"
                }),
                "{error}"
            );
            assert_eq!(std::fs::read_dir(dest).unwrap().count(), 0);
            assert_eq!(std::fs::read(source).unwrap(), b"real encrypted source");
        }
    }
    #[test]
    #[ignore = "creates EFS only in disposable CI; verifies readonly and no-callback ownership"]
    fn ci_efs_readonly_and_completely_empty_copy_without_progress() {
        for (readonly, disable_callback) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let dest = temp.path().join("dest");
            let contents = if readonly {
                b"readonly encrypted source".as_slice()
            } else {
                b"".as_slice()
            };
            std::fs::write(&source, contents).unwrap();
            std::fs::create_dir(&dest).unwrap();
            if readonly {
                std::fs::write(format!("{}:custom", source.display()), b"readonly ADS").unwrap();
            }
            encrypt_ci_fixture(&source);
            if readonly {
                let mut permissions = std::fs::metadata(&source).unwrap().permissions();
                permissions.set_readonly(true);
                std::fs::set_permissions(&source, permissions).unwrap();
            }
            NO_PROGRESS.with(|value| value.set(disable_callback));
            let result = prepare(&source, &dest, &parent(&dest), &JobControl::new());
            NO_PROGRESS.with(|value| value.set(false));
            let prepared = result.unwrap();
            prepared.verify(&JobControl::new()).unwrap();
            if disable_callback {
                assert!(
                    !prepared.callback_seen,
                    "test must genuinely omit native callbacks"
                );
            }
            assert_eq!(
                prepared.file.metadata().unwrap().permissions().readonly(),
                readonly
            );
            let private = prepared.stage.path().to_owned();
            drop(prepared);
            assert!(!private.exists());
            assert_eq!(std::fs::read(&source).unwrap(), contents);
            assert_eq!(
                std::fs::metadata(&source).unwrap().permissions().readonly(),
                readonly
            );
            if readonly {
                let mut permissions = std::fs::metadata(&source).unwrap().permissions();
                permissions.set_readonly(false);
                std::fs::set_permissions(&source, permissions).unwrap();
            }
        }
    }
}
