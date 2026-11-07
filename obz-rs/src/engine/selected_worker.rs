//! Selected extraction for split Copy or LZMA2 7z archives. A fresh
//! child reads fixed source handles and writes parent-created private files.
use super::*;
use sha2::{Digest, Sha256};
use std::ffi::{c_void, OsStr};
use std::io::Write;
use std::os::windows::{
    ffi::OsStrExt,
    fs::OpenOptionsExt,
    io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

const LIMIT: usize = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 100_000;
const MAX_NAME_UNITS: usize = 8 * 1024 * 1024;
const MAX_OUTPUT_HANDLES: usize = 512;
const PREFIX: &str = "NanaZip.Universal.Console.exe";
type Handle = *mut c_void;
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn DuplicateHandle(
        source_process: Handle,
        source: Handle,
        target_process: Handle,
        target: *mut Handle,
        access: u32,
        inherit: i32,
        options: u32,
    ) -> i32;
    fn SetFileTime(
        file: Handle,
        created: *const u64,
        accessed: *const u64,
        modified: *const u64,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
    fn CancelSynchronousIo(thread: Handle) -> i32;
}
#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(algorithm: Handle, data: *mut u8, count: u32, flags: u32) -> i32;
}
extern "C" {
    fn qing_selected_worker_main(runtime: *const u16) -> i32;
    fn qing_spawn_selected_worker(
        exe: *const u16,
        process: *mut Handle,
        input: *mut Handle,
        output: *mut Handle,
    ) -> u32;
}

fn runtime_dir() -> Result<Option<PathBuf>, String> {
    let exe = executable()?;
    if exe.file_name().and_then(OsStr::to_str) != Some(PREFIX) {
        return Ok(None);
    }
    let canonical = exe
        .parent()
        .ok_or("missing runtime directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let name = canonical.to_str().ok_or("invalid runtime directory")?;
    let local = name.strip_prefix(r"\\?\").unwrap_or(name);
    if local.as_bytes().get(1) != Some(&b':') {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(local)))
}

/// Hidden CLI entry point. The runtime is resolved from this installation, not
/// from control input. Handles are duplicated into this process only after spawn.
#[doc(hidden)]
pub fn run_child() -> u8 {
    let Ok(Some(runtime)) = runtime_dir() else {
        return 3;
    };
    let wide: Vec<_> = runtime.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { qing_selected_worker_main(wide.as_ptr()) as u8 }
}

fn pin_runtime(
    root: &Path,
    ctl: &JobControl,
) -> Result<Vec<crate::engine_stream::PinnedSource>, String> {
    #[derive(Deserialize)]
    struct Provenance {
        files: BTreeMap<String, String>,
    }
    let published: Provenance =
        serde_json::from_str(NANA_PROVENANCE)
            .map_err(|e| e.to_string())?;
    let mut pins = Vec::new();
    for (name, expected) in published
        .files
        .into_iter()
        .filter(|(n, _)| n.ends_with(".dll"))
    {
        let mut pin = crate::engine_stream::pin(&root.join(&name))?;
        let mut hash = Sha256::new();
        let mut bytes_read = 0_u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            ctl.check()?;
            let n = pin.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            bytes_read += n as u64;
        }
        let observed = crate::checksum::hex(&hash.finalize());
        if observed != expected {
            if PhaseTimer::enabled() {
                let size = pin.metadata().ok().map(|m| m.len());
                eprintln!("OA_RUNTIME_IDENTITY component={name} bytes={bytes_read} size={size:?} expected={expected} observed={observed}");
            }
            return Err(format!("bundled archive DLL identity mismatch: {name}"));
        }
        pins.push(pin);
    }
    Ok(pins)
}

fn manifest_fits(entries: &[Entry]) -> bool {
    if entries.len() > MAX_ITEMS {
        return false;
    }
    let mut units = 0usize;
    let mut bytes = 20usize; // nonce and count, then 68 fixed bytes plus each UTF-16 name
    for e in entries {
        let n = e.name.encode_utf16().count();
        if n > 32767 {
            return false;
        }
        let Some(next) = units.checked_add(n) else {
            return false;
        };
        units = next;
        let Some(next) = bytes.checked_add(68 + 2 * n) else {
            return false;
        };
        bytes = next;
        if units > MAX_NAME_UNITS || bytes > LIMIT {
            return false;
        }
    }
    true
}

// Match a single LZMA2 coder in the official technical listing. The worker
// independently checks the same-object manifest before granting output handles.
fn single_lzma2(method: &str) -> bool {
    let Some(dictionary) = method.strip_prefix("LZMA2:") else {
        return false;
    };
    let digits = dictionary.strip_suffix(['k', 'm']).unwrap_or(dictionary);
    dictionary.len() <= 5 && !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit())
}

// A separate anonymous input frame carries only the password. The sender
// owns and wipes its single, exactly sized UTF-16 transport copy on every exit.
const MAX_PASSWORD_UNITS: usize = 32767;

struct SecretFrame(Vec<u8>);
impl SecretFrame {
    fn new(password: &str) -> Self {
        let mut body = Vec::with_capacity(password.encode_utf16().count() * 2);
        for unit in password.encode_utf16() {
            body.extend_from_slice(&unit.to_le_bytes());
        }
        Self(body)
    }
}
impl AsRef<[u8]> for SecretFrame {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl Drop for SecretFrame {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

fn encrypted_lzma2(method: &str) -> bool {
    let Some((coder, rounds)) = method.split_once(" 7zAES:") else {
        return false;
    };
    single_lzma2(coder)
        && !rounds.is_empty()
        && rounds.len() <= 2
        && rounds.bytes().all(|c| c.is_ascii_digit())
}

fn eligible(
    first: &File,
    parts: &[File],
    entries: &[Entry],
    opts: &EngineOptions,
    leaf: &str,
) -> Result<bool, String> {
    if let Some(password) = &opts.password {
        crate::engine_password::validate(password)?;
    }
    if !leaf
        .get(leaf.len().saturating_sub(7)..)
        .is_some_and(|s| s.eq_ignore_ascii_case(".7z.001"))
        || parts.is_empty()
        || parts.len() >= 1024
        || opts.selected.is_empty()
        || opts
            .password
            .as_ref()
            .is_some_and(|p| p.encode_utf16().count() > MAX_PASSWORD_UNITS)
        || opts.password.is_some() != entries.iter().any(|e| e.encrypted)
        || opts.codepage.is_some()
        || opts.threads.is_some()
        || entries.is_empty()
        || !manifest_fits(entries)
        || !entries.iter().all(|entry| {
            if !entry.size_known {
                return false;
            }
            if entry.encrypted {
                return opts.password.is_some() && encrypted_lzma2(&entry.method);
            }
            if entry.is_dir || entry.size == 0 {
                return true;
            }
            opts.password.is_none()
                && (single_lzma2(&entry.method)
                    || (entry.method == "Copy" && entry.packed_size == entry.size))
        })
        || entries
            .iter()
            .filter(|e| !e.is_dir && selected(&e.name, opts))
            .count()
            > MAX_OUTPUT_HANDLES
    {
        return Ok(false);
    }
    let mut source = first.try_clone().map_err(|e| e.to_string())?;
    let mut start = [0; 32];
    source.rewind().map_err(|e| e.to_string())?;
    if source.metadata().map_err(|e| e.to_string())?.len() < start.len() as u64 {
        return Ok(false);
    }
    source.read_exact(&mut start).map_err(|e| e.to_string())?;
    if &start[..6] != b"7z\xbc\xaf'\x1c" || start[6] != 0 {
        return Ok(false);
    }
    if crc32fast::hash(&start[12..]) != u32::from_le_bytes(start[8..12].try_into().unwrap()) {
        return Err("7z start header CRC mismatch".into());
    }
    let offset = u64::from_le_bytes(start[12..20].try_into().unwrap());
    let size = u64::from_le_bytes(start[20..28].try_into().unwrap());
    let end = 32u64
        .checked_add(offset)
        .and_then(|v| v.checked_add(size))
        .ok_or("7z range overflow")?;
    let total = std::iter::once(first)
        .chain(parts.iter())
        .try_fold(0u64, |sum, file| {
            sum.checked_add(file.metadata().map_err(|e| e.to_string())?.len())
                .ok_or_else(|| "volume size overflow".to_owned())
        })?;
    if end > total {
        return Err("the fixed set of 7z volumes is incomplete".into());
    }
    Ok(end == total)
}

fn duplicate(child: &OwnedHandle, source: Handle, access: u32) -> Result<u64, String> {
    let mut remote = std::ptr::null_mut();
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            child.as_raw_handle(),
            &mut remote,
            access,
            0,
            0,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(remote as usize as u64)
    }
}

fn wide(bytes: &mut Vec<u8>, s: &OsStr) -> Result<(), String> {
    let units: Vec<_> = s.encode_wide().collect();
    if units.len() > 32767 {
        return Err("worker string limit exceeded".into());
    }
    bytes.extend_from_slice(&(units.len() as u32).to_le_bytes());
    for u in units {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    Ok(())
}
struct Fields<'a>(&'a [u8]);
impl<'a> Fields<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let (head, tail) = self
            .0
            .split_at_checked(n)
            .ok_or("truncated selected worker frame")?;
        self.0 = tail;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bit(&mut self) -> Result<bool, String> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("invalid worker flag".into()),
        }
    }
    fn wide(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        if n > 32767 {
            return Err("worker name limit exceeded".into());
        }
        let units: Vec<_> = self
            .take(n * 2)?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16(&units).map_err(|_| "invalid UTF-16 archive name".into())
    }
    fn time(&mut self) -> Result<Option<u64>, String> {
        let present = self.bit()?;
        let ticks = self.u64()?;
        Ok(present.then_some(ticks))
    }
    fn end(&self) -> Result<(), String> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err("unexpected worker frame data".into())
        }
    }
}

type Frame = Option<(u8, Vec<u8>)>;
fn read_frame(stdout: &mut impl std::io::Read) -> Result<Frame, String> {
    let mut kind = [0];
    let mut n = [0; 4];
    if stdout.read(&mut kind).map_err(|e| e.to_string())? == 0 {
        return Ok(None);
    }
    stdout.read_exact(&mut n).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(n) as usize;
    if length > LIMIT {
        return Err("selected worker frame exceeds size limit".into());
    }
    let mut body = vec![0; length];
    stdout.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Some((kind[0], body)))
}
struct Session {
    child: OwnedHandle,
    input: Option<File>,
    receiver: Option<Receiver<Result<Frame, String>>>,
    reader: Option<JoinHandle<()>>,
}
impl Session {
    fn start(exe: &Path) -> Result<Self, String> {
        let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let (mut process, mut input, mut output) = (
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        let error = unsafe {
            qing_spawn_selected_worker(wide.as_ptr(), &mut process, &mut input, &mut output)
        };
        if error != 0 {
            return Err(std::io::Error::from_raw_os_error(error as i32).to_string());
        }
        let child = unsafe { OwnedHandle::from_raw_handle(process) };
        let input = Some(unsafe { File::from_raw_handle(input) });
        let mut stdout = unsafe { File::from_raw_handle(output) };
        let (tx, rx) = mpsc::sync_channel(2);
        let mut s = Self {
            child,
            input,
            receiver: Some(rx),
            reader: None,
        };
        s.reader = Some(
            std::thread::Builder::new()
                .name("selected-worker-output".into())
                .spawn(move || loop {
                    let frame = read_frame(&mut stdout);
                    let last = !matches!(&frame, Ok(Some(_)));
                    if tx.send(frame).is_err() || last {
                        break;
                    }
                })
                .map_err(|e| e.to_string())?,
        );
        Ok(s)
    }
    fn send(
        &mut self,
        kind: u8,
        body: impl AsRef<[u8]> + Send + 'static,
        ctl: &JobControl,
    ) -> Result<(), String> {
        if body.as_ref().len() > LIMIT {
            return Err("worker request exceeds size limit".into());
        }
        let mut pipe = self
            .input
            .as_ref()
            .ok_or("worker input closed")?
            .try_clone()
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(1);
        let writer = std::thread::Builder::new()
            .name("selected-worker-input".into())
            .spawn(move || {
                let bytes = body.as_ref();
                let result = pipe
                    .write_all(&[kind])
                    .and_then(|_| pipe.write_all(&(bytes.len() as u32).to_le_bytes()))
                    .and_then(|_| pipe.write_all(bytes))
                    .and_then(|_| pipe.flush());
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        let result = loop {
            if ctl.check().is_err() {
                // Closing every worker pipe reader releases a blocked write. Also
                // interrupt the synchronous I/O in case another OS component is stalled.
                unsafe {
                    TerminateProcess(self.child.as_raw_handle(), 2);
                    CancelSynchronousIo(writer.as_raw_handle());
                }
            }
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => break result.map_err(|e| e.to_string()),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break Err("selected worker input channel closed".into()),
            }
        };
        writer
            .join()
            .map_err(|_| "selected worker input thread failed")?;
        ctl.check()?;
        result
    }
    fn next(&self, ctl: &JobControl) -> Result<Frame, String> {
        loop {
            ctl.check()?;
            match self
                .receiver
                .as_ref()
                .unwrap()
                .recv_timeout(Duration::from_millis(25))
            {
                Ok(value) => return value,
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(_) => return Err("selected worker control channel closed".into()),
            }
        }
    }
    fn exit_code(&self) -> Result<Option<u32>, String> {
        match unsafe { WaitForSingleObject(self.child.as_raw_handle(), 0) } {
            258 => Ok(None),
            0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.child.as_raw_handle(), &mut code) } == 0 {
                    Err(std::io::Error::last_os_error().to_string())
                } else {
                    Ok(Some(code))
                }
            }
            _ => Err(std::io::Error::last_os_error().to_string()),
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.input.take();
        self.receiver.take();
        unsafe {
            if WaitForSingleObject(self.child.as_raw_handle(), 0) != 0 {
                TerminateProcess(self.child.as_raw_handle(), 2);
            }
            WaitForSingleObject(self.child.as_raw_handle(), u32::MAX);
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[derive(Default)]
struct Stats {
    decoded: u64,
    selected: u64,
    completed: u32,
    item_errors: u32,
    block_errors: u32,
    code: i32,
}
struct Member {
    entry: usize,
    chosen: bool,
    times: [Option<u64>; 3],
}
fn system_time(ticks: u64) -> Option<std::time::SystemTime> {
    std::time::UNIX_EPOCH
        .checked_sub(Duration::from_secs(11_644_473_600))?
        .checked_add(Duration::new(
            ticks / 10_000_000,
            (ticks % 10_000_000) as u32 * 100,
        ))
}
fn acknowledge(
    data: &mut Fields,
    entries: &[Entry],
    opts: &EngineOptions,
) -> Result<Vec<Member>, String> {
    if data.u32()? as usize != entries.len() {
        return Err("archive manifest changed before extraction".into());
    }
    let mut available = std::collections::HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        available.insert(entry.name.replace('\\', "/"), i);
    }
    let mut members = Vec::with_capacity(entries.len());
    for _ in entries {
        let name = data.wide()?.replace('\\', "/");
        validate_name(&name)?;
        let i = available
            .remove(&name)
            .ok_or("archive name changed before extraction")?;
        let entry = &entries[i];
        let size = data.u64()?;
        let directory = data.bit()?;
        let attrs_present = data.bit()?;
        let attrs = data.u32()?;
        let has_crc = data.bit()?;
        let crc = data.u32()?;
        let times = [data.time()?, data.time()?, data.time()?];
        let kind = (attrs >> 16) & 0xf000;
        if entry.size != size
            || entry.is_dir != directory
            || attrs & 0x400 != 0
            || !matches!(kind, 0 | 0x4000 | 0x8000)
            || (kind != 0 && (kind == 0x4000) != directory)
            || (attrs_present && (attrs & 1 != 0) != entry.attributes.contains('R'))
            || has_crc.then_some(crc) != u32::from_str_radix(&entry.crc, 16).ok()
            || times[2].and_then(system_time) != archive_modified_time(&entry.modified)
        {
            return Err("archive item metadata differs from the verified listing".into());
        }
        members.push(Member {
            entry: i,
            chosen: selected(&entry.name, opts),
            times,
        });
    }
    data.end()?;
    if !available.is_empty() {
        return Err("incomplete archive manifest".into());
    }
    Ok(members)
}

fn set_times(file: &File, times: &[Option<u64>; 3]) -> Result<(), String> {
    if times.iter().all(Option::is_none) {
        return Ok(());
    }
    let p = |value: &Option<u64>| value.as_ref().map_or(std::ptr::null(), |v| v as *const u64);
    let ok = unsafe {
        SetFileTime(
            file.as_raw_handle(),
            p(&times[0]),
            p(&times[1]),
            p(&times[2]),
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    exe: &Path,
    runtime: &Path,
    leaf: &str,
    first: &File,
    parts: &[File],
    entries: &[Entry],
    opts: &EngineOptions,
    root: &Path,
    guards: &mut CheckedDirectories,
    ctl: &JobControl,
    stats: &mut Stats,
) -> Result<bool, String> {
    // Source handles and their parent namespace are already pinned from listing.
    // Runtime pins outlive the session. The session always reaps its child.
    let _runtime = pin_runtime(runtime, ctl)?;
    let _worker_image = crate::engine_stream::pin(exe)?;
    let mut nonce = [0u8; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 16, 2) } < 0 {
        return Err("worker job identity generation failed".into());
    }
    let mut session = Session::start(exe)?;
    let mut request = nonce.to_vec();
    wide(&mut request, runtime.as_os_str())?;
    request.extend_from_slice(
        &duplicate(&session.child, unsafe { GetCurrentProcess() }, 0x100000)?.to_le_bytes(),
    );
    request.extend_from_slice(&((parts.len() + 1) as u32).to_le_bytes());
    let prefix = &leaf[..leaf.len() - 3];
    for (i, file) in std::iter::once(first).chain(parts.iter()).enumerate() {
        request.extend_from_slice(
            &duplicate(&session.child, file.as_raw_handle(), 0x100081)?.to_le_bytes(),
        );
        request.extend_from_slice(
            &file
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
                .to_le_bytes(),
        );
        wide(&mut request, OsStr::new(&format!("{prefix}{:03}", i + 1)))?;
    }
    let password = opts.password.as_deref();
    let units = password.map_or(0, |p| p.encode_utf16().count() as u32);
    request.extend_from_slice(&units.to_le_bytes());
    session.send(1, request, ctl)?;
    if let Some(password) = password {
        session.send(6, SecretFrame::new(password), ctl)?;
    }
    let full_bytes = entries
        .iter()
        .try_fold(0u64, |n, e| n.checked_add(e.size))
        .ok_or("archive length overflow")?;
    let mut accepted: Option<Vec<Member>> = None;
    let mut files = BTreeMap::new();
    let mut terminal: Option<Result<bool, String>> = None;
    while let Some((kind, body)) = session.next(ctl)? {
        let mut data = Fields(&body);
        if data.take(16)? != nonce || terminal.is_some() {
            return Err("invalid selected worker response".into());
        }
        match kind {
            2 if accepted.is_none() => {
                let members = acknowledge(&mut data, entries, opts)?;
                let mut ack = nonce.to_vec();
                ack.extend_from_slice(&(members.len() as u32).to_le_bytes());
                for member in &members {
                    ctl.check()?;
                    let e = &entries[member.entry];
                    let mut remote = 0u64;
                    if member.chosen {
                        let path = root.join(validate_name(&e.name)?);
                        if e.is_dir {
                            guards.create_dirs(&path)?;
                        } else {
                            let file = create_staged_output(&path, guards)?;
                            remote = duplicate(&session.child, file.as_raw_handle(), 0x100082)?;
                            files.insert(member.entry, file);
                        }
                    }
                    ack.push(u8::from(member.chosen));
                    ack.extend_from_slice(&remote.to_le_bytes());
                }
                session.send(3, ack, ctl)?;
                accepted = Some(members);
            }
            7 if password.is_some() && accepted.is_none() => {
                data.end()?;
                ctl.set("decoding", None, "Opening encrypted archive header");
            }
            4 => {
                let decoded = data.u64()?;
                let total = data.u64()?;
                data.end()?;
                if total != full_bytes || decoded > total {
                    return Err("invalid selected worker progress".into());
                }
                ctl.set(
                    "decoding",
                    Some((decoded.saturating_mul(100) / total.max(1)) as u8),
                    "Verifying all archive entries",
                );
            }
            5 => {
                let hr = data.u32()?;
                let outcome = data.u32()?;
                let count = data.u32()?;
                stats.completed = data.u32()?;
                stats.item_errors = data.u32()?;
                stats.block_errors = data.u32()?;
                let open = data.u32()?;
                stats.decoded = data.u64()?;
                stats.selected = data.u64()?;
                stats.code = data.u32()? as i32;
                let length = data.u32()? as usize;
                if length > 192 {
                    return Err("invalid worker reason".into());
                }
                let reason =
                    std::str::from_utf8(data.take(length)?).map_err(|_| "invalid worker reason")?;
                data.end()?;
                let selected_bytes = entries
                    .iter()
                    .filter(|e| selected(&e.name, opts))
                    .map(|e| e.size)
                    .sum::<u64>();
                let valid = accepted.is_some()
                    && hr == 0
                    && outcome == 2
                    && count as usize == entries.len()
                    && stats.completed == count
                    && stats.item_errors == 0
                    && stats.block_errors == 0
                    && stats.code == -1
                    && open == 0
                    && stats.decoded == full_bytes
                    && stats.selected == selected_bytes;
                // A positively identified valid format feature outside the supported
                // subset can use the original decoder only before any stage grant.
                let ineligible = outcome == 3
                    && hr == 0
                    && accepted.is_none()
                    && files.is_empty()
                    && count == 0
                    && stats.completed == 0
                    && stats.decoded == 0
                    && stats.selected == 0
                    && open == 0
                    && stats.item_errors == 0
                    && stats.block_errors == 0
                    && stats.code == -1;
                terminal = Some(if valid {
                    Ok(true)
                } else if ineligible {
                    Ok(false)
                } else if stats.code == 9 || (password.is_some() && matches!(stats.code, 2 | 3)) {
                    Err("incorrect password or damaged encrypted archive".into())
                } else if stats.code == 3 {
                    Err("7z CRC verification failed".into())
                } else {
                    Err(format!("selected worker failed ({hr:#010x}): {reason}"))
                });
            }
            _ => return Err("unexpected selected worker frame".into()),
        }
    }
    let status = loop {
        ctl.check()?;
        if let Some(status) = session.exit_code()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let used = terminal.ok_or_else(|| {
        format!("selected worker exited without a terminal receipt (exit {status:#010x})")
    })??;
    if status != 0 {
        return Err("selected worker exit status disagrees with its receipt".into());
    }
    if !used {
        return Ok(false);
    }
    for member in accepted.unwrap().iter().rev().filter(|m| m.chosen) {
        ctl.check()?;
        if let Some(file) = files.get(&member.entry) {
            set_times(file, &member.times)?;
        } else {
            let path = root.join(validate_name(&entries[member.entry].name)?);
            guards.check(&path)?;
            let file = File::options()
                .access_mode(0x100)
                .custom_flags(0x02200000)
                .open(path)
                .map_err(|e| e.to_string())?;
            set_times(&file, &member.times)?;
        }
    }
    Ok(true)
}

pub(super) struct WorkerPlan<'a> {
    leaf: &'a str,
    exe: PathBuf,
    runtime: PathBuf,
}

pub(super) fn plan<'a>(path: &'a str, first: &File, parts: &[File], entries: &[Entry],
    opts: &EngineOptions) -> Result<Option<WorkerPlan<'a>>, String> {
    let Some(leaf) = Path::new(path).file_name().and_then(OsStr::to_str) else {
        return Ok(None);
    };
    if !eligible(first, parts, entries, opts, leaf)? {
        return Ok(None);
    }
    let Some(runtime) = runtime_dir()? else { return Ok(None); };
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = current.with_file_name("obz.exe");
    Ok(exe.is_file().then_some(WorkerPlan { leaf, exe, runtime }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn decode(
    plan: WorkerPlan<'_>,
    first: &crate::engine_stream::PinnedSource,
    parts: &[File],
    entries: &[Entry],
    opts: &EngineOptions,
    root: &Path,
    guards: &mut CheckedDirectories,
    ctl: &JobControl,
) -> Result<bool, String> {
    let WorkerPlan { leaf, exe, runtime } = plan;
    let mut stats = Stats {
        code: -1,
        ..Default::default()
    };
    let result = run(
        &exe, &runtime, leaf, first, parts, entries, opts, root, guards, ctl, &mut stats,
    );
    if PhaseTimer::enabled() {
        eprintln!("OA_SELECTED_WORKER status={} volumes={} decoded={} selected={} discarded={} completed={} item_errors={} block_errors={} code={}",
            match result { Ok(true) => "ok", Ok(false) => "ineligible", Err(_) => "error" }, parts.len()+1, stats.decoded, stats.selected,
            stats.decoded.saturating_sub(stats.selected), stats.completed, stats.item_errors, stats.block_errors, stats.code);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: String) -> Entry {
        Entry {
            name,
            size: 1,
            size_known: true,
            packed_size: 1,
            is_dir: false,
            crc: "12345678".into(),
            encrypted: false,
            modified: String::new(),
            method: "Copy".into(),
            comment: String::new(),
            attributes: String::new(),
        }
    }

    #[test]
    fn native_manifest_capacity_uses_utf16_and_total_wire_size() {
        let long = entry("a".repeat(32767));
        assert!(manifest_fits(&vec![long.clone(); 255]));
        assert!(!manifest_fits(&vec![long; 256]));
        // Each supplementary character occupies two UTF-16 code units.
        assert!(!manifest_fits(&[entry("馃梽".repeat(16384))]));
    }

    #[test]
    fn declared_end_cannot_acquire_a_late_volume() {
        let mut first = tempfile::tempfile().unwrap();
        let part = tempfile::tempfile().unwrap();
        part.set_len(32).unwrap();
        let parts = [part];
        let opts = EngineOptions {
            selected: vec!["a".into()],
            ..Default::default()
        };
        let entries = [entry("a".into())];
        for (end, expected) in [(64u64, Some(true)), (63, Some(false)), (65, None)] {
            let mut start = [0; 32];
            start[..6].copy_from_slice(b"7z\xbc\xaf'\x1c");
            start[7] = 4;
            start[12..20].copy_from_slice(&(end - 32).to_le_bytes());
            let crc = crc32fast::hash(&start[12..]);
            start[8..12].copy_from_slice(&crc.to_le_bytes());
            first.rewind().unwrap();
            first.write_all(&start).unwrap();
            let actual = eligible(&first, &parts, &entries, &opts, "Upper.7Z.001");
            if let Some(value) = expected {
                assert_eq!(actual.unwrap(), value);
            } else {
                assert!(actual.unwrap_err().contains("incomplete"));
            }
        }
    }
}
