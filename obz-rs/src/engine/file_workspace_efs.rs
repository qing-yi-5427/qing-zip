//! Verified EFS local-file copies using native encrypted staging and guarded
//! publication. Never decrypt to an ordinary temp file.
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
    identity_kind(handle,false)
}
fn identity_kind(handle:Handle,directory:bool)->Result<[u32;3],String>{
    let mut info = [0u32; 13];
    if unsafe { GetFileInformationByHandle(handle, info.as_mut_ptr()) } == 0 {
        return Err(error("读取EFS对象身份"));
    }
    if info[0] & 0x410 != if directory {0x10}else{0} || info[10] != 1 {
        return Err(format!(
            "EFS ordinary-file identity required: attrs={:#x} links={}",
            info[0], info[10]
        ));
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
        if std::io::Error::last_os_error().raw_os_error()==Some(38) && file.metadata().map_err(|e|e.to_string())?.is_dir(){
            return Ok(Vec::new());
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
    #[cfg(test)]
    basic: [u64; 3],
    digest: [u8; 32],
    logical: [u8; 32],
    encrypted: bool,
    certificates: [Vec<Vec<u8>>;2],
    ea: Vec<u8>,
    _pins: Vec<File>,
}
fn snapshot(file: &File, ctl: &JobControl) -> Result<Snapshot, String> {
    snapshot_with_share(file, ctl, 1)
}
fn snapshot_with_share(file: &File, ctl: &JobControl, share: u32) -> Result<Snapshot, String> {
    let value=snapshot_any(file,ctl,share)?;
    if !value.encrypted{return Err("目标未保持EFS加密，拒绝发布".into());}
    Ok(value)
}
fn snapshot_any(file: &File, ctl: &JobControl, share: u32) -> Result<Snapshot, String> {
    ctl.check()?;
    let directory=file.metadata().map_err(|e|e.to_string())?.is_dir();
    let source_id=identity_kind(file.as_raw_handle(),directory)?;
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
            let raw = unsafe { ReOpenFile(opened.as_raw_handle(), 0x80000000, share, 0x00200000) };
            if raw as isize == -1 {
                return Err(error("固定EFS数据流"));
            }
            unsafe { File::from_raw_handle(raw) }
        };
        let stream_is_dir=pin.metadata().map_err(|e|e.to_string())?.is_dir();
        if identity_kind(pin.as_raw_handle(),stream_is_dir)? != source_id {
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
    // Access time can change because of our own reads and is not a stable
    // verification field. Preserve it when copying, like ordinary local copies.
    let basic = file.metadata().map_err(|e| e.to_string())?;
    hash.update(basic.creation_time().to_le_bytes());
    hash.update(basic.last_write_time().to_le_bytes());
    hash.update((basic.file_attributes() & 0x3127).to_le_bytes());
    let ea = ea(file)?;
    hash.update((ea.len() as u64).to_le_bytes());
    hash.update(&ea);
    let logical=hash.clone().finalize().into();
    hash.update([u8::from(encrypted)]);
    let mut certificates=[Vec::new(),Vec::new()];
    let path = path(file)?;
    for recovery in [false, true] {
        let keys = if encrypted {keys(&path, recovery)?} else {Vec::new()};
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
        if encrypted && !recovery && keys.is_empty() {
            return Err("EFS缺少授权用户证书".into());
        }
        certificates[usize::from(recovery)]=keys.clone();
        hash.update([u8::from(recovery)]);
        hash.update((keys.len() as u64).to_le_bytes());
        for key in keys {
            hash.update((key.len() as u64).to_le_bytes());
            hash.update(key);
        }
    }
    Ok(Snapshot {
        #[cfg(test)]
        basic: [
            basic.creation_time(),
            basic.last_write_time(),
            u64::from(basic.file_attributes() & 0x3127),
        ],
        digest: hash.finalize().into(),
        logical,encrypted,certificates,
        ea,
        _pins: pins,
    })
}

pub(super) fn verify_published(
    file: &File,
    expected: [u8; 32],
    ctl: &JobControl,
) -> Result<(), String> {
    if snapshot(file, ctl)?.digest != expected {
        return Err("published EFS complete proof changed".into());
    }
    Ok(())
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
    // CopyFileEx ADS callback handles can omit FILE_READ_ATTRIBUTES. Reopen
    // the exact stream object, never its pathname, for metadata-only checks.
    let metadata = unsafe { ReOpenFile(target, 0x20080, 7, 0x00200000) };
    if metadata as isize == -1 {
        state.failed = Some(error("EFS callback metadata reopen"));
        return 1;
    }
    let metadata = unsafe { File::from_raw_handle(metadata) };
    let id = match identity(metadata.as_raw_handle()) {
        Ok(id) => id,
        Err(error) => {
            state.failed = Some(format!(
                "EFS native target identity stream={_n} reason={_reason}: {error}"
            ));
            return 1;
        }
    };
    if let Err(error) = crate::private_stage::verify_native_private_child(state.parent, &metadata) {
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
struct NativeFile(Option<File>);
impl std::ops::Deref for NativeFile {
    type Target = File;
    fn deref(&self) -> &File {
        self.0.as_ref().expect("owned native EFS file")
    }
}
impl Drop for NativeFile {
    fn drop(&mut self) {
        // This handle belongs exclusively to our native-created private output.
        // Never apply this cleanup to the source or a path-resolved replacement.
        let Some(file) = self.0.as_ref() else {
            return;
        };
        if let Ok(metadata) = file.metadata() {
            if metadata.file_attributes() & 1 != 0 {
                let attrs = (metadata.file_attributes() & !1) as u64;
                let basic = [0u64, 0, 0, 0, if attrs == 0 { 0x80 } else { attrs }];
                unsafe {
                    SetFileInformationByHandle(file.as_raw_handle(), 0, basic.as_ptr().cast(), 40);
                }
            }
        }
        let delete = 1u8;
        unsafe {
            SetFileInformationByHandle(file.as_raw_handle(), 4, (&delete as *const u8).cast(), 1);
        }
    }
}
pub(crate) struct Prepared {
    file: NativeFile,
    reservation: Option<crate::private_stage::NativeOutputReservation>,
    stage: crate::private_stage::PrivateStage,
    source: crate::engine_stream::PinnedSource,
    expected: Snapshot,
    target_digest: [u8;32],
    #[cfg(test)]
    callback_seen: bool,
}
struct EfsProof<'a> {
    source: &'a File,
    digest: [u8; 32],
    target_digest: [u8;32],
    control: JobControl,
}
impl crate::private_stage::LocalOutputProof for EfsProof<'_> {
    fn verify(&self, file: &File) -> Result<Vec<u8>, String> {
        if snapshot_any(self.source, &self.control,1)?.digest != self.digest {
            return Err("EFS source changed before publication".into());
        }
        let actual = snapshot_with_share(file, &self.control, 5)?.digest;
        if actual != self.target_digest {
            return Err("EFS complete stream/certificate proof changed".into());
        }
        Ok(actual.to_vec())
    }
}
impl Prepared {
    pub(super) fn source(&self) -> &File {
        &self.source
    }
    pub(super) fn digest(&self) -> [u8; 32] {
        self.target_digest
    }
    pub(crate) fn publish(
        mut self,
        target: &Path,
        replace: bool,
        ctl: &JobControl,
    ) -> Result<File, String> {
        self.verify(ctl)?;
        let proof = EfsProof {
            source: &self.source,
            digest: self.expected.digest,
            target_digest:self.target_digest,
            control: ctl.clone(),
        };
        let old_path = path(&self.file)?;
        let file = self.file.0.take().ok_or("EFS output ownership lost")?;
        let mut output = self
            .reservation
            .take()
            .ok_or("EFS target policy lost")?
            .adopt(file, self.stage.root(), &old_path)?;
        ctl.check()?;
        output.commit_local_streams(target, replace, &proof)
    }
    pub(crate) fn verify(&self, ctl: &JobControl) -> Result<(), String> {
        crate::private_stage::verify_native_private_child(self.stage.root(), &self.file)?;
        let source = snapshot_any(&self.source, ctl,1)?;
        // Private target main handle owns DELETE for identity-bound cleanup.
        // Its ADS reads must share that existing right, never share WRITE.
        let target = snapshot_with_share(&self.file, ctl, 5)?;
        if source.digest != self.expected.digest || target.digest != self.target_digest {
            #[cfg(test)]
            eprintln!(
                "EFS proof mismatch source_same={} target_same={} basic expected={:?} source={:?} target={:?}",
                source.digest == self.expected.digest,
                target.digest == self.target_digest,
                self.expected.basic,
                source.basic,
                target.basic
            );
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
    prepare_bound(source, parent, parent_pin, ctl, None)
}
pub(super) fn prepare_bound(
    source: &Path,
    parent: &Path,
    parent_pin: &File,
    ctl: &JobControl,
    expected: Option<(u32, u64)>,
) -> Result<Prepared, String> {
    ctl.check()?;
    let source = crate::engine_stream::pin(source)?;
    let actual = identity(source.as_raw_handle())?;
    if expected.is_some_and(|old| {
        old != (
            actual[0],
            (u64::from(actual[1]) << 32) | u64::from(actual[2]),
        )
    }) {
        return Err("EFS source identity changed before copy".into());
    }
    let expected = snapshot_any(&source, ctl,1)?;
    if !expected.encrypted && parent_pin.metadata().map_err(|e|e.to_string())?.file_attributes()&0x4000==0 {
        return Err("EFS copy requires encrypted source or encrypted destination policy".into());
    }
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
    let reservation = crate::private_stage::reserve_native_output(parent, parent_pin)?;
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
    let file = NativeFile(Some(
        File::options()
            .access_mode(0x1f0199) // WRITE_OWNER restores our private owner before adoption
            .share_mode(0)
            .custom_flags(0x02200000)
            .open(&target)
            .map_err(|e| e.to_string())?,
    ));
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
    let metadata = source.metadata().map_err(|e| e.to_string())?;
    let attrs = metadata.file_attributes() & 0x3127;
    let basic = [
        metadata.creation_time(),
        metadata.last_access_time(),
        metadata.last_write_time(),
        0,
        u64::from(if attrs == 0 { 0x80 } else { attrs }),
    ];
    if unsafe { SetFileInformationByHandle(file.as_raw_handle(), 0, basic.as_ptr().cast(), 40) }
        == 0
    {
        return Err(error("restore encrypted copy basic metadata"));
    }
    #[cfg(test)]
    let callback_seen = state.destination.is_some();
    let target_state=snapshot_with_share(&file,ctl,5)?;
    if target_state.logical!=expected.logical || (expected.encrypted && target_state.certificates!=expected.certificates) {
        return Err("EFS native copy content or source certificate policy changed".into());
    }
    let prepared = Prepared {
        target_digest:target_state.digest,
        file,
        reservation: Some(reservation),
        stage,
        source,
        expected,
        #[cfg(test)]
        callback_seen,
    };
    prepared.verify(ctl)?;
    Ok(prepared)
}


type RawExport=unsafe extern "system" fn(*const u8,Handle,u32)->u32;
type RawImport=unsafe extern "system" fn(*mut u8,Handle,*mut u32)->u32;
#[link(name="advapi32")]
unsafe extern "system" {
    fn OpenEncryptedFileRawW(path:*const u16,flags:u32,context:*mut Handle)->u32;
    fn ReadEncryptedFileRaw(callback:Option<RawExport>,argument:Handle,context:Handle)->u32;
    fn WriteEncryptedFileRaw(callback:Option<RawImport>,argument:Handle,context:Handle)->u32;
    fn CloseEncryptedFileRaw(context:Handle);
}
struct RawLease(Handle);
impl Drop for RawLease{fn drop(&mut self){unsafe{CloseEncryptedFileRaw(self.0)}}}
fn raw_open(path:&Path,flags:u32)->Result<RawLease,String>{
    let mut handle=std::ptr::null_mut();
    let code=unsafe{OpenEncryptedFileRawW(wide(path).as_ptr(),flags,&mut handle)};
    if code!=0{return Err(format!("EFS directory raw open failed: {code}"));}
    if handle.is_null(){return Err("EFS raw context is null".into());}
    Ok(RawLease(handle))
}
struct RawIo<'a>{file:&'a mut File,ctl:&'a JobControl,failed:Option<String>}
unsafe extern "system" fn raw_export(data:*const u8,arg:Handle,len:u32)->u32{
    use std::io::Write;
    let ctx=unsafe{&mut *arg.cast::<RawIo<'_>>()};
    let result=(||{ctx.ctl.check()?;if data.is_null()&&len!=0{return Err("invalid EFS raw buffer".into());}
        if len!=0 {ctx.file.write_all(unsafe{std::slice::from_raw_parts(data,len as usize)}).map_err(|e|e.to_string())?;}Ok(())})();
    match result{Ok(())=>0,Err(e)=>{ctx.failed=Some(e);995}}
}
unsafe extern "system" fn raw_import(data:*mut u8,arg:Handle,len:*mut u32)->u32{
    use std::io::Read;
    let ctx=unsafe{&mut *arg.cast::<RawIo<'_>>()};
    let result=(||{ctx.ctl.check()?;if len.is_null(){return Err("invalid EFS raw count".into());}
        let count=unsafe{*len} as usize;if data.is_null()&&count!=0{return Err("invalid EFS raw buffer".into());}
        let actual=if count==0{0}else{ctx.file.read(unsafe{std::slice::from_raw_parts_mut(data,count)}).map_err(|e|e.to_string())?};
        unsafe{*len=actual as u32};Ok(())})();
    match result{Ok(())=>0,Err(e)=>{ctx.failed=Some(e);995}}
}
/// Per-directory object only. No recursive import and no mutation of an
/// existing public directory; publication uses the ordinary local proof gate.
pub(super) struct PreparedDirectory{
    output:crate::private_stage::PrivateOutputFile,
    source:crate::engine_stream::PinnedSource,
    expected:Snapshot,
}
impl PreparedDirectory{
    pub(super) fn source(&self)->&File{&self.source}
    pub(super) fn digest(&self)->[u8;32]{self.expected.digest}
    pub(super) fn publish(mut self,target:&Path,ctl:&JobControl)->Result<File,String>{
        struct Proof<'a>{source:&'a File,digest:[u8;32],ctl:&'a JobControl}
        impl crate::private_stage::LocalOutputProof for Proof<'_>{
            fn verify(&self,file:&File)->Result<Vec<u8>,String>{
                self.ctl.check()?;
                if snapshot(self.source,self.ctl)?.digest!=self.digest || snapshot_with_share(file,self.ctl,5)?.digest!=self.digest {
                    return Err("EFS directory complete source/target proof changed".into());
                }Ok(self.digest.to_vec())
            }
        }
        let proof=Proof{source:&self.source,digest:self.expected.digest,ctl};
        let result=self.output.commit_local_streams(target,false,&proof);
        let cleanup=self.output.abort();
        let output=match (result,cleanup){(Ok(file),Ok(()))=>file,(Err(e),Ok(()))=>return Err(e),(Ok(_),Err(e))=>return Err(e),(Err(e),Err(c))=>return Err(format!("{e}; {c}"))};
        let pin=File::options().access_mode(0x120188).share_mode(3).custom_flags(0x02200000)
            .open(path(&output)?).map_err(|e|e.to_string())?;
        if identity_kind(pin.as_raw_handle(),true)?!=identity_kind(output.as_raw_handle(),true)?{return Err("published EFS directory identity changed".into());}
        Ok(pin)
    }
}
pub(super) fn validate_directory(file:&File,ctl:&JobControl)->Result<(),String>{snapshot(file,ctl).map(|_|())}
pub(super) fn restore_directory_metadata(source:&File,target:&File)->Result<(),String>{
    identity_kind(source.as_raw_handle(),true)?;identity_kind(target.as_raw_handle(),true)?;
    if target.metadata().map_err(|e|e.to_string())?.file_attributes()&0x4000==0{return Err("EFS directory policy was lost".into());}
    let meta=source.metadata().map_err(|e|e.to_string())?;let attrs=meta.file_attributes()&0x3127;
    let basic=[meta.creation_time(),meta.last_access_time(),meta.last_write_time(),0,u64::from(if attrs==0{0x80}else{attrs})];
    if unsafe{SetFileInformationByHandle(target.as_raw_handle(),0,basic.as_ptr().cast(),40)}==0{return Err(error("restore EFS directory metadata"));}
    Ok(())
}
pub(super) fn prepare_directory(source_path:&Path,expected_id:Option<[u32;3]>,parent_path:&Path,parent:&File,ctl:&JobControl)->Result<PreparedDirectory,String>{
    use std::io::{Seek,SeekFrom};
    ctl.check()?;
    let source=crate::engine_stream::pin_directory_metadata(source_path)?;
    let id=identity_kind(source.as_raw_handle(),true)?;
    if expected_id.is_some_and(|expected|expected!=id){return Err("EFS source directory identity changed".into());}
    let mut expected=snapshot(&source,ctl)?;
    // Existing ADS pins conflict with native Raw. The metadata/ancestor pins
    // remain live; Raw holds existing stream locks. Re-enumerate and re-pin
    // every stream afterwards, rejecting new streams, EA or policy changes.
    expected._pins.clear();
    let stage=crate::private_stage::create(parent_path,parent)?;
    let mut spool=File::options().read(true).write(true).create_new(true)
        .open(stage.path().join("directory.raw")).map_err(|e|e.to_string())?;
    {
        let lease=raw_open(source_path,0)?;
        let mut io=RawIo{file:&mut spool,ctl,failed:None};
        let code=unsafe{ReadEncryptedFileRaw(Some(raw_export),(&mut io as *mut RawIo<'_>).cast(),lease.0)};
        if let Some(e)=io.failed{return Err(e);}
        if code!=0{return Err(format!("EFS directory raw export failed: {code}"));}
    }
    let rebound=snapshot(&source,ctl)?;
    if rebound.digest!=expected.digest{return Err("EFS source directory changed during raw export".into());}
    expected=rebound;
    spool.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
    let mut output=crate::private_stage::create_local_directory_output(parent_path,parent)?;
    output.native_directory(|_,name|{
        let lease=raw_open(name,3)?;
        let mut io=RawIo{file:&mut spool,ctl,failed:None};
        let code=unsafe{WriteEncryptedFileRaw(Some(raw_import),(&mut io as *mut RawIo<'_>).cast(),lease.0)};
        if let Some(e)=io.failed{return Err(e);}
        if code!=0{return Err(format!("EFS directory raw import failed: {code}"));}
        Ok(())
    })?;
    if !expected.ea.is_empty(){set_ea(output.as_file(),&expected.ea)?;}
    let meta=source.metadata().map_err(|e|e.to_string())?;
    let attrs=meta.file_attributes()&0x3127;
    let basic=[meta.creation_time(),meta.last_access_time(),meta.last_write_time(),0,u64::from(if attrs==0{0x80}else{attrs})];
    if unsafe{SetFileInformationByHandle(output.as_file().as_raw_handle(),0,basic.as_ptr().cast(),40)}==0{return Err(error("restore EFS directory times"));}
    if snapshot_with_share(output.as_file(),ctl,5)?.digest!=expected.digest || snapshot(&source,ctl)?.digest!=expected.digest{
        return Err("EFS directory content, attributes or certificate policy changed".into());
    }
    Ok(PreparedDirectory{output,source,expected})
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
    fn directory_fixture(root:&Path)->PathBuf{
        let source=root.join("encrypted-dir");std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("plain.txt"),b"plaintext existing child").unwrap();
        std::fs::create_dir(source.join("plain-empty")).unwrap();
        encrypt_ci_fixture(&source);
        std::fs::write(source.join("secret.txt"),b"encrypted child").unwrap();
        std::fs::write(format!("{}:alpha",source.display()),b"directory ADS").unwrap();
        let file=File::options().access_mode(0x20098).share_mode(7).custom_flags(0x02200000).open(&source).unwrap();
        set_ea(&file,&[0,0,0,0,0x80,1,3,0,b'A',0,1,2,3]).unwrap();source
    }
    #[test]
    #[ignore = "real EFS directory transfer only in disposable hosted runner"]
    fn ci_efs_directory_transfer_tree_and_cross_volume_retention(){
        for moving in [false,true]{
            let root=tempfile::tempdir().unwrap();let source=directory_fixture(root.path());
            let dest=tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
            if moving{
                let src=File::options().read(true).custom_flags(0x02200000).open(&source).unwrap();
                let mut info=[0u32;13];assert_ne!(unsafe{GetFileInformationByHandle(parent(dest.path()).as_raw_handle(),info.as_mut_ptr())},0);
                assert_ne!(identity_kind(src.as_raw_handle(),true).unwrap()[0],info[7],"actual cross-volume fixture required");
            }
            let result=super::super::file_workspace::transfer(&[source.display().to_string()],dest.path().to_str().unwrap(),moving,&super::super::EngineOptions::default()).unwrap();
            let target=dest.path().join("encrypted-dir");
            assert_eq!(std::fs::read(target.join("plain.txt")).unwrap(),b"plaintext existing child");
            assert_eq!(std::fs::read(target.join("secret.txt")).unwrap(),b"encrypted child");
            assert_eq!(std::fs::read(format!("{}:alpha",target.display())).unwrap(),b"directory ADS");
            assert_ne!(std::fs::metadata(&target).unwrap().file_attributes()&0x4000,0);
            assert_ne!(std::fs::metadata(target.join("plain.txt")).unwrap().file_attributes()&0x4000,0);
            assert!(source.join("plain-empty").is_dir());assert!(source.join("plain.txt").exists());assert!(source.join("secret.txt").exists());
            if moving{assert_eq!(result.moved,0);assert!(!result.retained_sources.is_empty());}
        }
    }
    #[test]
    #[ignore = "real EFS directory merges only in disposable hosted runner"]
    fn ci_efs_existing_directory_keeps_policy_and_honors_child_conflicts(){
        for mode in ["error","skip","rename","overwrite"]{
            let root=tempfile::tempdir().unwrap();let source=directory_fixture(root.path());
            let dest=root.path().join("dest");let target=dest.join("encrypted-dir");std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("secret.txt"),b"original target").unwrap();
            std::fs::write(format!("{}:alpha",target.display()),b"target ADS").unwrap();
            let opts=super::super::EngineOptions{overwrite:mode.into(),..Default::default()};
            let result=super::super::file_workspace::transfer(&[source.display().to_string()],dest.to_str().unwrap(),false,&opts);
            if mode=="error"{assert!(result.is_err());}else{assert!(!result.unwrap().metadata_not_merged.is_empty());}
            assert_eq!(std::fs::metadata(&target).unwrap().file_attributes()&0x4000,0);
            assert_eq!(std::fs::read(format!("{}:alpha",target.display())).unwrap(),b"target ADS");
            assert_eq!(std::fs::read(target.join("secret.txt")).unwrap(),if mode=="overwrite"{b"encrypted child".as_slice()}else{b"original target".as_slice()});
            if mode=="rename"{assert_eq!(std::fs::read(target.join("secret (1).txt")).unwrap(),b"encrypted child");}
            assert!(source.join("secret.txt").exists());
        }
    }
    #[test]
    #[ignore = "real EFS directory proof rejection only in disposable hosted runner"]
    fn ci_efs_directory_publication_rejects_cancel_new_stream_and_ea_change(){
        for mode in ["cancel","stream","ea"]{
            let root=tempfile::tempdir().unwrap();let source=directory_fixture(root.path());
            let dest=root.path().join("dest");std::fs::create_dir(&dest).unwrap();let ctl=JobControl::new();
            let prepared=prepare_directory(&source,None,&dest,&parent(&dest),&ctl).unwrap();
            match mode{
                "cancel"=>ctl.cancel(),
                "stream"=>std::fs::write(format!("{}:new",source.display()),b"late source stream").unwrap(),
                _=>{
                    let writer=File::options().access_mode(0x18).share_mode(7).custom_flags(0x02200000).open(&source).unwrap();
                    set_ea(&writer,&[0,0,0,0,0x80,1,1,0,b'B',0,9]).unwrap();
                }
            }
            assert!(prepared.publish(&dest.join("copied"),&ctl).is_err());
            assert_eq!(std::fs::read_dir(&dest).unwrap().count(),0);assert!(source.is_dir());
        }
    }
    #[test]
    #[ignore = "creates an EFS certificate only in disposable GitHub-hosted CI"]
    fn ci_efs_directory_raw_private_publication_preserves_policy_and_all_metadata(){
        for populated in [false,true]{
            let temp=tempfile::tempdir().unwrap();let source=temp.path().join("encrypted-dir");let dest=temp.path().join("dest");
            std::fs::create_dir(&source).unwrap();std::fs::create_dir(&dest).unwrap();
            if populated{std::fs::write(source.join("plain-before"),b"source plain child").unwrap();}
            encrypt_ci_fixture(&source);
            if populated{
                for name in ["alpha","beta","中文"]{std::fs::write(format!("{}:{name}",source.display()),name.as_bytes()).unwrap();}
                let h=File::options().access_mode(0x20098).share_mode(7).custom_flags(0x02200000).open(&source).unwrap();
                set_ea(&h,&[0,0,0,0,0x80,1,3,0,b'A',0,1,2,3]).unwrap();
            }
            let ctl=JobControl::new();
            let prepared=prepare_directory(&source,None,&dest,&parent(&dest),&ctl).unwrap();
            let digest=prepared.expected.digest;
            assert_eq!(snapshot(&prepared.source,&ctl).unwrap().digest,digest);
            let target=dest.join("copied");let output=prepared.publish(&target,&ctl).unwrap();drop(output);
            let check=File::options().read(true).share_mode(7).custom_flags(0x02200000).open(&target).unwrap();
            assert_eq!(snapshot(&check,&ctl).unwrap().digest,digest);drop(check);
            assert_eq!(std::fs::read_dir(&target).unwrap().count(),0,"raw directory import is not recursive");
            std::fs::write(target.join("new-child"),b"future encryption policy").unwrap();
            assert_ne!(std::fs::metadata(target.join("new-child")).unwrap().file_attributes()&0x4000,0);
            if populated{assert_eq!(std::fs::read(source.join("plain-before")).unwrap(),b"source plain child");
                assert_eq!(std::fs::metadata(source.join("plain-before")).unwrap().file_attributes()&0x4000,0);}
        }
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
            let held_path = path(&prepared.file).unwrap();
            assert!(std::fs::rename(&held_path, prepared.stage.path().join("replaced")).is_err());
            assert!(std::fs::write(&held_path, b"replacement").is_err());
            prepared.verify(&JobControl::new()).unwrap();
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
            let digest = prepared.digest();
            let output_path = dest.join("published");
            let output = prepared
                .publish(&output_path, false, &JobControl::new())
                .unwrap();
            drop(output);
            assert!(!private.exists());
            verify_published(
                &File::open(&output_path).unwrap(),
                digest,
                &JobControl::new(),
            )
            .unwrap();
            assert_eq!(
                std::fs::metadata(&output_path)
                    .unwrap()
                    .permissions()
                    .readonly(),
                readonly
            );
            if readonly {
                let mut p = std::fs::metadata(&output_path).unwrap().permissions();
                p.set_readonly(false);
                std::fs::set_permissions(&output_path, p).unwrap();
            }
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
    #[test]
    #[ignore = "real EFS transfer only on disposable hosted runner"]
    fn ci_efs_transfer_conflicts_complete_proof_and_native_cancel() {
        for mode in ["error", "skip", "rename", "overwrite"] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source.txt");
            let dest = temp.path().join("dest");
            std::fs::write(&source, b"efs content").unwrap();
            std::fs::write(format!("{}:custom", source.display()), b"all ads bytes").unwrap();
            encrypt_ci_fixture(&source);
            {
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(&source)
                    .unwrap();
                set_ea(&file, &[0, 0, 0, 0, 0x80, 1, 3, 0, b'A', 0, 1, 2, 3]).unwrap();
            }
            let before = snapshot(&File::open(&source).unwrap(), &JobControl::new())
                .unwrap()
                .digest;
            std::fs::create_dir(&dest).unwrap();
            std::fs::write(dest.join("source.txt"), b"old").unwrap();
            let opts = super::super::EngineOptions {
                overwrite: mode.into(),
                ..Default::default()
            };
            let result = super::super::file_workspace::transfer(
                &[source.display().to_string()],
                dest.to_str().unwrap(),
                false,
                &opts,
            );
            if mode == "error" {
                assert!(result.is_err());
            } else if mode == "skip" {
                assert_eq!(result.unwrap().skipped, 1);
            } else {
                assert_eq!(result.unwrap().copied, 1);
                let target = dest.join(if mode == "rename" {
                    "source (1).txt"
                } else {
                    "source.txt"
                });
                assert_eq!(
                    snapshot(&File::open(&target).unwrap(), &JobControl::new())
                        .unwrap()
                        .digest,
                    before
                );
            }
            if mode != "overwrite" {
                assert_eq!(std::fs::read(dest.join("source.txt")).unwrap(), b"old");
            }
            assert_eq!(
                snapshot(&File::open(&source).unwrap(), &JobControl::new())
                    .unwrap()
                    .digest,
                before
            );
            assert!(std::fs::read_dir(&dest).unwrap().all(|p| {
                !p.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".qing")
            }));
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("cancel");
        let dest = temp.path().join("dest");
        std::fs::write(&source, vec![1; 1024 * 1024]).unwrap();
        encrypt_ci_fixture(&source);
        std::fs::create_dir(&dest).unwrap();
        std::fs::write(dest.join("cancel"), b"old").unwrap();
        CANCEL_NATIVE.with(|v| v.set(true));
        let opts = super::super::EngineOptions {
            overwrite: "overwrite".into(),
            ..Default::default()
        };
        assert!(
            super::super::file_workspace::transfer(
                &[source.display().to_string()],
                dest.to_str().unwrap(),
                false,
                &opts
            )
            .is_err()
        );
        assert_eq!(std::fs::read(dest.join("cancel")).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dest).unwrap().count(), 1);
        assert_eq!(std::fs::metadata(source).unwrap().len(), 1024 * 1024);
    }
    #[test]
    #[ignore = "real EFS cross-volume copy only on disposable hosted runner"]
    fn ci_efs_cross_volume_move_keeps_source_and_reports_boundary() {
        let source_dir = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let source = source_dir.path().join("encrypted.txt");
        std::fs::write(&source, b"source retained").unwrap();
        encrypt_ci_fixture(&source);
        assert_ne!(
            identity(File::open(&source).unwrap().as_raw_handle()).unwrap()[0],
            {
                let mut info = [0u32; 13];
                assert_ne!(
                    unsafe {
                        GetFileInformationByHandle(
                            parent(dest.path()).as_raw_handle(),
                            info.as_mut_ptr(),
                        )
                    },
                    0
                );
                info[7]
            },
            "test requires actual distinct volumes"
        );
        let result = super::super::file_workspace::transfer(
            &[source.display().to_string()],
            dest.path().to_str().unwrap(),
            true,
            &super::super::EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(result.copied, 1);
        assert_eq!(result.moved, 0);
        assert!(!result.retained_sources.is_empty());
        assert!(!result.retained_reasons.is_empty());
        assert!(source.exists());
        assert_eq!(
            snapshot(&File::open(&source).unwrap(), &JobControl::new())
                .unwrap()
                .digest,
            snapshot(
                &File::open(dest.path().join("encrypted.txt")).unwrap(),
                &JobControl::new()
            )
            .unwrap()
            .digest
        );
    }

    #[test]
    #[ignore = "real EFS publication rejection gates only on disposable hosted runner"]
    fn ci_efs_publication_rejects_changed_source_cancel_and_parent_policy() {
        for gate in ["source", "cancel", "parent"] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let dest = temp.path().join("dest");
            std::fs::write(&source, b"original").unwrap();
            encrypt_ci_fixture(&source);
            std::fs::create_dir(&dest).unwrap();
            let target = dest.join("existing");
            std::fs::write(&target, b"old target").unwrap();
            let ctl = JobControl::new();
            let prepared = prepare(&source, &dest, &parent(&dest), &ctl).unwrap();
            let private = prepared.stage.path().to_owned();
            if gate == "source" {
                let ea_writer = File::options()
                    .access_mode(0x18)
                    .share_mode(7)
                    .open(&source)
                    .unwrap();
                set_ea(&ea_writer, &[0, 0, 0, 0, 0, 1, 1, 0, b'B', 0, 9]).unwrap();
            } else if gate == "cancel" {
                ctl.cancel();
            } else {
                // Revoke inherited read on this disposable parent.
                // The prepared snapshot must be rejected, never replayed.
                let status = std::process::Command::new("icacls")
                    .arg(&dest)
                    .args(["/deny", "*S-1-1-0:(OI)(CI)R"])
                    .output()
                    .unwrap();
                assert!(status.status.success(), "parent fixture ACL change failed");
            }
            let refused = prepared.publish(&target, true, &ctl).is_err();
            if gate == "parent" {
                let restored = std::process::Command::new("icacls")
                    .arg(&dest)
                    .args(["/remove:d", "*S-1-1-0"])
                    .output()
                    .unwrap();
                assert!(
                    restored.status.success(),
                    "disposable parent ACL restore failed"
                );
            }
            assert!(refused, "{gate}");
            assert!(!private.exists());
            assert_eq!(std::fs::read(&target).unwrap(), b"old target");
            assert!(source.exists());
            assert_eq!(std::fs::read_dir(dest).unwrap().count(), 1);
        }
    }
    #[test]
    #[ignore = "plaintext to EFS policy requires disposable hosted runner"]
    fn ci_efs_plain_source_respects_encrypted_target_policy_and_keeps_source_plain() {
        for mode in ["error","skip","rename","overwrite"] {
            let temp=tempfile::tempdir().unwrap();let source=temp.path().join("source.txt");let dest=temp.path().join("dest");
            std::fs::write(&source,b"plain source content").unwrap();std::fs::write(format!("{}:named",source.display()),b"plain named stream").unwrap();
            {let file=File::options().read(true).write(true).open(&source).unwrap();set_ea(&file,&[0,0,0,0,0x80,1,1,0,b'A',0,7]).unwrap();}
            std::fs::create_dir(&dest).unwrap();encrypt_ci_fixture(&dest);
            let old=dest.join("source.txt");std::fs::write(&old,b"old destination").unwrap();
            let before=snapshot_any(&File::open(&source).unwrap(),&JobControl::new(),1).unwrap();assert!(!before.encrypted);
            let options=super::super::EngineOptions{overwrite:mode.into(),..Default::default()};
            let copied=super::super::file_workspace::transfer(&[source.display().to_string()],dest.to_str().unwrap(),false,&options);
            if mode=="error"{assert!(copied.is_err());}
            else if mode=="skip"{assert_eq!(copied.unwrap().skipped,1);}
            else{
                assert_eq!(copied.unwrap().copied,1);
                let target=dest.join(if mode=="rename"{"source (1).txt"}else{"source.txt"});
                let actual=snapshot(&File::open(target).unwrap(),&JobControl::new()).unwrap();
                assert!(actual.encrypted);assert!(!actual.certificates[0].is_empty());assert_eq!(actual.logical,before.logical);
                assert_ne!(actual.digest,before.digest,"encryption-state difference must remain explicit");
            }
            if mode!="overwrite"{assert_eq!(std::fs::read(old).unwrap(),b"old destination");}
            assert_eq!(snapshot_any(&File::open(&source).unwrap(),&JobControl::new(),1).unwrap().digest,before.digest);
        }
    }

}
