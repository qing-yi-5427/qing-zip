//! Seekable default Add for an already locked ordinary ZIP.
use super::*;

const HELLO: u8 = 57;
const REQUEST: u8 = 58;
const VERSION: [u8; 4] = 1u32.to_le_bytes();
const MAX_ROOTS: usize = 4096;
const REQUEST_LIMIT: usize = 3 * 1024 * 1024;

#[repr(C)]
struct Root {
    source: Handle,
    leaf: *const u16,
}

extern "C" {
    fn qz_append_zip(runtime: *const u16, owner: Handle, original: Handle, output: Handle,
        expected_count: u32, roots: *const Root, root_count: u32, error: *mut u8, capacity: usize) -> i32;
}

pub(in crate::engine) fn try_encode(
    program: &Path, original: &File, output: &edit_source::EditOutput,
    names: &[String], count: u16, ctl: &JobControl,
) -> Result<bool, String> {
    if names.is_empty() || names.len() > MAX_ROOTS { return Ok(false); }
    let _timer = PhaseTimer::new("zip-append-owned-worker");
    let helper = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("obz-worker.exe");
    let Ok(_image) = crate::engine_stream::pin(&helper) else { return Ok(false); };
    let Some(_runtime) = owned_update::verified_bundled_nana_console(program) else { return Ok(false); };

    // Keep full source name chains fixed. Directory traversal starts from
    // the granted handles and uses the native writer's relative child opens.
    let mut guards = SourceValidationGuards::new();
    let mut roots = Vec::with_capacity(names.len());
    let mut identities = Vec::with_capacity(names.len());
    let mut request_bytes = 32; // three handles, original member count, source count
    for name in names {
        ctl.check()?;
        let path = Path::new(name);
        if attribute_query_path(path).is_none() { return Ok(false); }
        let leaf = path.file_name().ok_or("missing ZIP source filename")?;
        let units = leaf.encode_wide().count();
        if !(1..=255).contains(&units) || leaf.to_str().is_none() { return Ok(false); }
        request_bytes += 12 + units * 2; // source handle + UTF-16 length and bytes
        if request_bytes > REQUEST_LIMIT { return Ok(false); }
        if path.is_dir() {
            for parent in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
                if !guards.pin_directory(parent)? { return Ok(false); }
            }
            let pin = &guards.held[path];
            if !create_zip::local_ntfs(path, pin) { return Ok(false); }
            roots.push((leaf, pin.as_raw_handle()));
        } else {
            let Ok(pin) = crate::engine_stream::pin(path) else { return Ok(false); };
            if !create_zip::local_ntfs(path, &pin) { return Ok(false); }
            roots.push((leaf, pin.as_raw_handle()));
            identities.push(pin);
        }
    }

    let Ok(mut session) = Session::start_mode(&helper, 5) else { return Ok(false); };
    match create_zip::create_handshake_frame(&session, ctl)? {
        None => { create_zip::legacy_create_exit(&session, ctl)?; return Ok(false); }
        Some((HELLO, version)) if version == VERSION => (),
        _ => return Err("incompatible ZIP update helper".into()),
    }
    let candidate = output.disposable_zip_candidate()?;
    let writer = candidate.writer()?;
    let mut request = Vec::with_capacity(request_bytes);
    for (handle, access) in [
        (unsafe { GetCurrentProcess() }, 0x100000),
        (original.as_raw_handle(), 0x100081),
        (writer.as_raw_handle(), 0x100082),
    ] {
        request.extend_from_slice(&duplicate(&session.child, handle, access)?.to_le_bytes());
    }
    request.extend_from_slice(&u32::from(count).to_le_bytes());
    request.extend_from_slice(&(roots.len() as u32).to_le_bytes());
    for (leaf, handle) in roots {
        request.extend_from_slice(&duplicate(&session.child, handle, 0x100081)?.to_le_bytes());
        wide(&mut request, leaf)?;
    }
    session.send(REQUEST, request, ctl)?;
    let mut began = false;
    let encoded = loop {
        let (kind, body) = session.next(ctl)?.ok_or("ZIP update worker exited without a result")?;
        match (kind, body.as_slice()) {
            (47, []) if !began => {
                began = true;
                if PhaseTimer::enabled() { eprintln!("OA_ZIP_APPEND status=encoding"); }
                ctl.set("updating", Some(0), "Updating a temporary archive copy");
            }
            (46, [percent]) if began && *percent <= 99 => {
                ctl.set("updating", Some(*percent), "Updating a temporary archive copy");
            }
            (41, []) if !began => break false,
            (43, []) if began => break true,
            (43, message) if !message.is_empty() => return Err(String::from_utf8_lossy(message).into_owned()),
            _ => return Err("invalid ZIP update worker result".into()),
        }
    };
    if session.next(ctl)?.is_some() || session.wait_exit(ctl)? != if encoded { 0 } else { 3 } {
        return Err("ZIP update worker status differs from its result".into());
    }
    drop(writer);
    if encoded { candidate.finish()?; }
    Ok(encoded)
}

fn run_worker() -> Result<bool, String> {
    let runtime = runtime_dir()?.ok_or("missing ZIP update runtime")?;
    let runtime: Vec<_> = runtime.as_os_str().encode_wide().chain(Some(0)).collect();
    let (kind, bytes) = read_frame(&mut std::io::stdin().lock(), REQUEST_LIMIT)?.ok_or("missing ZIP update request")?;
    if kind != REQUEST { return Err("invalid ZIP update request".into()); }
    let mut request = Fields(&bytes);
    let owner = request.u64()? as usize as Handle;
    let original = request.u64()? as usize as Handle;
    let output = request.u64()? as usize as Handle;
    let expected = request.u32()?;
    let count = request.u32()? as usize;
    if count == 0 || count > MAX_ROOTS { return Err("invalid ZIP update source count".into()); }
    let mut sources = Vec::with_capacity(count);
    for _ in 0..count {
        let handle = request.u64()? as usize as Handle;
        let leaf: Vec<_> = request.wide()?.encode_utf16().chain(Some(0)).collect();
        sources.push((handle, leaf));
    }
    request.end()?;
    let roots: Vec<_> = sources.iter().map(|(source, leaf)| Root { source: *source, leaf: leaf.as_ptr() }).collect();
    let mut error = [0; 256];
    let status = unsafe { qz_append_zip(runtime.as_ptr(), owner, original, output, expected, roots.as_ptr(),
        count as u32, error.as_mut_ptr(), error.len()) };
    match status {
        0 => Ok(true),
        1 => Ok(false),
        _ => {
            let len = error.iter().position(|&byte| byte == 0).unwrap_or(error.len());
            Err(format!("ZIP update worker failed {status:#x}: {}", String::from_utf8_lossy(&error[..len])))
        }
    }
}

fn reply(kind: u8, body: &[u8]) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(&[kind])?;
    out.write_all(&(body.len() as u32).to_le_bytes())?;
    out.write_all(body)?;
    out.flush()
}

#[doc(hidden)]
pub fn helper_worker() -> u8 {
    if reply(HELLO, &VERSION).is_err() { return 2; }
    let (kind, body, code) = match run_worker() {
        Ok(true) => (43, Vec::new(), 0),
        Ok(false) => (41, Vec::new(), 3),
        Err(error) => (43, error.into_bytes(), 1),
    };
    if reply(kind, &body).is_err() { 2 } else { code }
}
