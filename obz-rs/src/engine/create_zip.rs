use super::*;

extern "C" {
    fn qz_create_snapshot(runtime:*const u16,owner:Handle,root:Handle,name:*const u16,output:Handle,
        level:u32,threads:u32,explicit:u32,dictionary:u32,solid:u32,seven:u32,times:u32,snapshot:*const u8,snapshot_size:usize,latest:*mut u64,error:*mut u8,capacity:usize)->i32;
    fn qz_create_timed(runtime:*const u16,owner:Handle,root:Handle,name:*const u16,output:Handle,
        level:u32,threads:u32,explicit:u32,dictionary:u32,solid:u32,seven:u32,times:u32,latest:*mut u64,error:*mut u8,capacity:usize)->i32;
    pub(super) fn qing_spawn_create_worker(
        exe: *const u16,
        process: *mut Handle,
        input: *mut Handle,
        output: *mut Handle,
    ) -> u32;
    fn qz_create_7z(runtime: *const u16, owner: Handle, directory: Handle,
        root_name: *const u16, output: Handle, level: u32, threads: u32,
        explicit: u32, dictionary: u32, solid: u32, error: *mut u8, capacity: usize) -> i32;
    fn qz_create_zip(
        runtime: *const u16,
        owner: Handle,
        directory: Handle,
        root_name: *const u16,
        output: Handle,
        level: u32,
        threads: u32,
        explicit: u32,
        error: *mut u8,
        capacity: usize,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn GetVolumeInformationByHandleW(
        file: Handle,
        name: *mut u16,
        n: u32,
        serial: *mut u32,
        component: *mut u32,
        flags: *mut u32,
        fs: *mut u16,
        size: u32,
    ) -> i32;
}

fn local_disk(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(p)) = path.components().next() else {
        return false;
    };
    let Prefix::Disk(letter) = p.kind() else {
        return false;
    };
    let root = [u16::from(letter), 58, 92, 0];
    attribute_query_path(path).is_some() && unsafe { GetDriveTypeW(root.as_ptr()) } == 3
}
fn ordinary_ntfs(file: &File) -> bool {
    let mut fs = [0u16; 16];
    unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            16,
        ) != 0
            && fs[..5] == [78, 84, 70, 83, 0]
    }
}

pub(in crate::engine) fn eligible(format: &str, names: &[String], opts: &EngineOptions) -> bool {
    format == "zip"
        && names.len() == 1
        && opts
            .method
            .as_deref()
            .is_none_or(|m| m.trim().eq_ignore_ascii_case("deflate"))
        && opts.level.is_none_or(|v| (1..=9).contains(&v))
        && opts.threads.is_none_or(|v| (1..=64).contains(&v))
        && opts.password.is_none()
        && opts.codepage.is_none()
        && opts.encryption.is_none()
        && opts.dictionary_bytes.is_none()
        && opts.word_size.is_none()
        && opts.solid_block_bytes.is_none()
        && opts.solid.is_none()
        && opts.header_encryption.is_none()
        && opts.volume_bytes.is_none()
        && opts.selected.is_empty()
        && opts.overwrite == "error"
        && opts.zone_identifier.is_none()
}

// Initial 7z route: one local ordinary directory, single volume and no
// encryption. Unsupported options retain the complete console source preflight.
pub(in crate::engine) fn eligible_7z(format: &str, names: &[String], opts: &EngineOptions) -> bool {
    if format != "7z" || !opts.method.as_deref().is_none_or(|m| m.trim().eq_ignore_ascii_case("lzma2"))
        || !opts.dictionary_bytes.is_none_or(|n| (4096..=1024*1024*1024).contains(&n)) { return false; }
    let mut common=opts.clone(); common.method=None;common.dictionary_bytes=None;common.solid=None;
    eligible("zip", names, &common)
}
pub(in crate::engine) fn try_create_format(out: &str, names: &[String], format: &str, opts: &EngineOptions, ctl: &JobControl) -> Result<bool,String> {
    if format == "zip" { try_create(out,names,opts,ctl) }
    else if eligible_7z(format,names,opts) { try_create_impl(out,names,opts,ctl,true) }
    else { Ok(false) }
}

// Routing samples a limited number of directory listings. The actual native
// manifest or the ordinary source preflight always rechecks the tree independently.
const MIN_WORKER_FILES: usize = 2048;
const PROBE_DIRECTORIES: usize = 32;
const PROBE_ENTRIES: usize = MIN_WORKER_FILES + 256;

fn large_directory_hint(root: &Path, ctl: &JobControl) -> Result<bool, String> {
    use std::collections::VecDeque;
    let mut pending = VecDeque::from([root.to_owned()]);
    let mut probe_pins = SourceValidationGuards::new();
    let (mut visited, mut entries, mut files) = (0, 0, 0);
    while let Some(dir) = pending.pop_front() {
        // Root and its ancestors are pinned by the caller. Each additional
        // directory is opened without following reparse points; listed types
        // select work only and cannot authorize a path or file.
        if dir != root && !matches!(probe_pins.pin_directory(&dir), Ok(true)) {
            return Ok(false);
        }
        visited += 1;
        let Ok(iter) = fs::read_dir(dir) else {
            return Ok(false);
        };
        for entry in iter {
            ctl.check()?;
            if entries == PROBE_ENTRIES {
                return Ok(false);
            }
            entries += 1;
            let Ok(entry) = entry else { return Ok(false) };
            let Ok(kind) = entry.file_type() else {
                return Ok(false);
            };
            if kind.is_symlink() {
                return Ok(false);
            }
            if kind.is_file() {
                files += 1;
                if files == MIN_WORKER_FILES {
                    return Ok(true);
                }
            } else if kind.is_dir() {
                if visited + pending.len() < PROBE_DIRECTORIES {
                    pending.push_back(entry.path());
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(false)
}

/// false is possible only before UpdateItems starts; the caller must then redo
/// the complete ordinary source validation. Security, cancellation, protocol and
/// publication errors never turn into a second encode.
pub(in crate::engine) fn try_create(
    out: &str,
    names: &[String],
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<bool, String> {
    try_create_impl(out,names,opts,ctl,false)
}
fn try_create_impl(out: &str, names: &[String], opts: &EngineOptions, ctl: &JobControl, seven: bool) -> Result<bool,String> {
    // Console preflight uses atime-suppressed enumeration. A native attempt can
    // enumerate before declining; that would lose the original directory atime.
    if opts.store_atime==Some(true) {return Ok(false);}
    let policy = crate::machine_policy::scope();
    ctl.check()?;
    if policy.restricted() {
        return Ok(false);
    }
    let src = absolute(&names[0])?;
    let target = absolute(out)?;
    if !local_disk(&src)
        || !local_disk(&target)
        || !target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(if seven {"7z"} else {"zip"}))
    {
        return Ok(false);
    }
    let parent = target.parent().ok_or("output has no parent")?;
    if !parent.is_dir() || !src.is_dir() {
        return Ok(false);
    }
    let mut source_guards = SourceValidationGuards::new();
    for a in src.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if !source_guards.pin_directory(a)? {
            return Ok(false);
        }
    }
    let large = {
        let _timer = PhaseTimer::new("zip-create-size-probe");
        large_directory_hint(&src, ctl)?
    };
    if !large {
        return Ok(false);
    }
    let inputs = checked_input_roots(names)?;
    no_links(&target)?;
    reject_output_in_inputs(parent, &inputs)?;
    if target.exists() {
        return Err("output archive already exists; use update to modify it".into());
    }
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("obz.exe");
    if !exe.is_file() {
        return Ok(false);
    }
    let Some(runtime) = runtime_dir()? else {
        return Ok(false);
    };
    let mut guards = CheckedDirectories::default();
    guards.check(parent)?;
    let Some((destination, _)) = guards.held.get(parent) else {
        return Ok(false);
    };
    if !ordinary_ntfs(destination) || !ordinary_ntfs(&source_guards.held[&src]) {
        return Ok(false);
    }
    let mut dest_ids = HashSet::new();
    for a in parent.ancestors() {
        let Some((f, _)) = guards.held.get(a) else {
            return Ok(false);
        };
        dest_ids.insert(directory_identity(f)?);
    }
    if dest_ids.contains(&directory_identity(&source_guards.held[&src])?) {
        return Err("target inside source directory".into());
    }
    let shared = source_guards
        .held
        .iter()
        .map(|(p, f)| Ok((p.clone(), dest_ids.contains(&directory_identity(f)?))))
        .collect::<Result<Vec<_>, String>>()?;
    let _runtime = pin_runtime(&runtime, ctl)?;
    let _image = crate::engine_stream::pin(&exe)?;
    let parent_pin = match File::options()
        .access_mode(0x200a0)
        .share_mode(3)
        .custom_flags(0x02200000)
        .open(parent)
    {
        Ok(file) => file,
        Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if directory_identity(&parent_pin)? != directory_identity(destination)?
        || directory_attributes(&parent_pin)? & 0x410 != 0x10
    {
        return Err("ZIP destination directory identity changed".into());
    }
    let Some(mut output) = crate::private_stage::ArchiveOutput::try_new(parent, &parent_pin)?
    else {
        return Ok(false);
    };
    let encode = encode(
        &exe,
        &src,
        &source_guards.held[&src],
        output.file(),
        seven,
        opts,
        ctl,
        None,
    );
    for (p, overlap) in shared {
        if overlap {
            source_guards.held.remove(&p);
        }
    }
    let result = encode.and_then(|(encoded, latest)| {
        ctl.check()?;
        policy.unchanged()?;
        if !encoded {
            return Ok(false);
        }
        guards.check_prepared(parent)?;
        reject_output_in_inputs(parent, &inputs)?;
        output.file().sync_all().map_err(|e| e.to_string())?;
        if let Some(value)=latest {output.set_modified_filetime(value)?;}
        ctl.check()?;
        policy.unchanged()?;
        output.publish(target.file_name().ok_or("missing output name")?, || {
            guards.check_prepared(parent)?;
            policy.unchanged()?;
            ctl.check()
        })?;
        Ok(true)
    });
    match (result, output.cleanup()) {
        (Err(error), Err(cleanup)) => Err(format!("{error}; staging cleanup failed: {cleanup}")),
        (Ok(true), Err(cleanup)) => Err(format!(
            "archive was committed but staging cleanup failed: {cleanup}"
        )),
        (Ok(false), Err(cleanup)) => Err(cleanup),
        (result, Ok(())) => result,
    }
}

// Snapshot mode is mandatory once requested: unsupported cannot fall through
// to a filesystem-atime encoder after the source has already been traversed.
pub(in crate::engine) fn create_from_snapshot(target:&Path, prepared:&create_input::Prepared,
    format:&str, opts:&EngineOptions, ctl:&JobControl)->Result<(),String>{
    let mut qualified=opts.clone();qualified.store_atime=None;
    let names=vec![prepared.container().display().to_string()];
    if !eligible(format,&names,&qualified) && !eligible_7z(format,&names,&qualified) {
        return Err("snapshot creation options are not implemented by the current native candidate".into());
    }
    let policy=crate::machine_policy::scope();
    if policy.restricted(){return Err("machine policy rejects native snapshot writer".into());}
    let manifest=prepared.snapshot_manifest()?;
    prepared.verify(ctl)?;
    let source=File::options().access_mode(0x100081).share_mode(1).custom_flags(0x02200000)
        .open(prepared.container()).map_err(|e|e.to_string())?;
    no_links(target)?;
    if target.exists(){return Err("snapshot archive already exists".into());}
    let parent=target.parent().ok_or("output has no parent")?;
    let mut guards=CheckedDirectories::default();guards.check(parent)?;
    let parent_pin=File::options().access_mode(0x200a0).share_mode(3).custom_flags(0x02200000)
        .open(parent).map_err(|e|e.to_string())?;
    let expected=&guards.held.get(parent).ok_or("missing output parent pin")?.0;
    if directory_identity(&parent_pin)?!=directory_identity(expected)? {return Err("output parent changed".into());}
    let runtime=runtime_dir()?.ok_or("missing native runtime")?;
    let _runtime=pin_runtime(&runtime,ctl)?;
    let exe=std::env::current_exe().map_err(|e|e.to_string())?.with_file_name("obz.exe");
    let _image=crate::engine_stream::pin(&exe)?;
    let mut output=crate::private_stage::ArchiveOutput::try_new(parent,&parent_pin)?.ok_or("private snapshot output unavailable")?;
    let result=(||{
        let (encoded,latest)=encode(&exe,prepared.container(),&source,output.file(),format=="7z",opts,ctl,Some(&manifest))?;
        if !encoded {return Err("native snapshot writer declined verified input".into());}
        prepared.verify(ctl)?;policy.unchanged()?;ctl.check()?;
        output.file().sync_all().map_err(|e|e.to_string())?;
        if let Some(value)=latest {output.set_modified_filetime(value)?;}
        output.publish(target.file_name().ok_or("output name missing")?,||{guards.check_prepared(parent)?;policy.unchanged()?;ctl.check()})?;
        if PhaseTimer::enabled(){eprintln!("OA_CREATE_SNAPSHOT status=complete");}
        Ok(())
    })();
    match (result,output.cleanup()) {(Err(e),Err(c))=>Err(format!("{e}; cleanup: {c}")),(Err(e),_)=>Err(e),(_,Err(e))=>Err(e),_=>Ok(())}
}

// The optional small image negotiates before receiving ANY privileged handles.
// Only an older image's clean unsupported-entry exit is a compatibility fallback.
const CREATE_HELLO: u8 = 49;
const CREATE_CHALLENGE: u8 = 50;
const CREATE_ACK: u8 = 51;
const CREATE_VERSION: u32 = 1;
const CREATE_CAPABILITIES: u32 = 15; // ZIP + single-volume LZMA2 7z + typed TIMES
const CREATE_TIMES: u32 = 4;

struct CreateSession {
    // Drop/reap the child before releasing its pinned executable image.
    session: Session,
    _image: Option<crate::engine_stream::PinnedSource>,
}
fn check_create_hello(kind: u8, body: &[u8]) -> Result<u32, String> {
    if kind != CREATE_HELLO || body.len()!=8 || body[..4]!=CREATE_VERSION.to_le_bytes() {
        return Err("incompatible create helper handshake".into());
    }
    let capabilities=u32::from_le_bytes(body[4..].try_into().unwrap());
    if !matches!(capabilities,3|7|15) {return Err("incompatible create helper capabilities".into());}
    Ok(capabilities)
}
fn check_create_ack(kind: u8, body: &[u8], nonce: &[u8; 16]) -> Result<(), String> {
    if kind != CREATE_ACK || body != nonce { return Err("create helper response identity mismatch".into()); }
    Ok(())
}
fn create_handshake_frame(session: &Session, ctl: &JobControl) -> Result<Frame, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        ctl.check()?;
        if std::time::Instant::now() >= deadline { return Err("create helper handshake timed out".into()); }
        match session.receiver.as_ref().ok_or("create helper receiver closed")?.recv_timeout(Duration::from_millis(25)) {
            Ok(frame) => return frame,
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(_) => return Err("create helper control channel failed".into()),
        }
    }
}
fn legacy_create_exit(session: &Session, ctl: &JobControl) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        ctl.check()?;
        if let Some(code) = session.exit_code()? {
            return if code == 3 { Ok(()) } else { Err("create helper exited without a handshake".into()) };
        }
        if std::time::Instant::now() >= deadline { return Err("create helper handshake timed out".into()); }
        thread::sleep(Duration::from_millis(1));
    }
}
fn create_session(exe: &Path, ctl: &JobControl, need_times: bool, need_snapshot: bool) -> Result<CreateSession, String> {
    let helper = exe.with_file_name("obz-worker.exe");
    match fs::symlink_metadata(&helper) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {
            let image = crate::engine_stream::pin(&helper)?;
            let mut session = Session::start(&helper, true)?;
            match create_handshake_frame(&session, ctl)? {
                None => {
                    legacy_create_exit(&session, ctl)?;
                    // No frame, no handles granted, exact legacy unsupported exit.
                    // Drop joins the reader and reaps before starting the old entry.
                    drop(session);
                }
                Some((kind, body)) => {
                    let capabilities=check_create_hello(kind, &body)?;
                    if (need_times && capabilities & CREATE_TIMES == 0) || (need_snapshot && capabilities & 8 == 0) {
                        // Recognized legacy protocol, but no TIMES capability. No
                        // nonce or privileged source/output handles were granted.
                        drop(session);
                        drop(image);
                        return Ok(CreateSession {session:Session::start(exe,true)?,_image:None});
                    }
                    let mut nonce = [0u8;16];
                    if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 16, 2) } < 0 {
                        return Err("create helper nonce failed".into());
                    }
                    session.send(CREATE_CHALLENGE, nonce.to_vec(), ctl)?;
                    let (kind, body) = create_handshake_frame(&session, ctl)?.ok_or("create helper exited before identity acknowledgement")?;
                    check_create_ack(kind, &body, &nonce)?;
                    return Ok(CreateSession { session, _image: Some(image) });
                }
            }
        }
    }
    Ok(CreateSession { session: Session::start(exe, true)?, _image: None })
}

// New optional-image entry. The original obz.exe entry deliberately keeps its
// original protocol, so a missing/older helper can still use it safely.
pub fn helper_worker() -> u8 {
    fn handshake() -> Result<(), String> {
        let hello = [CREATE_VERSION.to_le_bytes(), CREATE_CAPABILITIES.to_le_bytes()].concat();
        let mut output = std::io::stdout().lock();
        output.write_all(&[CREATE_HELLO]).and_then(|_| output.write_all(&(hello.len() as u32).to_le_bytes()))
            .and_then(|_| output.write_all(&hello)).and_then(|_| output.flush()).map_err(|e| e.to_string())?;
        let (kind, nonce) = read_frame(&mut std::io::stdin().lock(), 64*1024*1024+1024)?.ok_or("missing create challenge")?;
        if kind != CREATE_CHALLENGE || nonce.len() != 16 { return Err("invalid create challenge".into()); }
        output.write_all(&[CREATE_ACK]).and_then(|_| output.write_all(&16u32.to_le_bytes()))
            .and_then(|_| output.write_all(&nonce)).and_then(|_| output.flush()).map_err(|e| e.to_string())
    }
    if handshake().is_err() { return 2; }
    worker()
}

fn encode(
    exe: &Path,
    src: &Path,
    root: &File,
    output: &File,
    seven: bool,
    opts: &EngineOptions,
    ctl: &JobControl,
    snapshot: Option<&[u8]>,
) -> Result<(bool, Option<u64>), String> {
    let times=compression_times::native_flags(opts);
    let mut prepared = create_session(exe, ctl, times!=0, snapshot.is_some())?;
    let session = &mut prepared.session;
    let mut data = Vec::new();
    if times!=0 || snapshot.is_some() {data.extend_from_slice(&u32::from(seven).to_le_bytes());}
    for (handle, access) in [
        (unsafe { GetCurrentProcess() }, 0x100000),
        (root.as_raw_handle(), 0x100081),
        (output.as_raw_handle(), 0x100082),
    ] {
        data.extend_from_slice(&duplicate(&session.child, handle, access)?.to_le_bytes());
    }
    wide(&mut data, src.file_name().ok_or("source name missing")?)?;
    data.extend_from_slice(&opts.level.map(u32::from).unwrap_or(u32::MAX).to_le_bytes());
    data.extend_from_slice(&opts.threads.unwrap_or(0).to_le_bytes());
    data.extend_from_slice(&u32::from(opts.method.is_some()).to_le_bytes());
    if seven {
        data.extend_from_slice(&(opts.dictionary_bytes.unwrap_or(0) as u32).to_le_bytes());
        data.extend_from_slice(&opts.solid.map(|s|u32::from(s)+1).unwrap_or(0).to_le_bytes());
    }
    if times!=0 {data.extend_from_slice(&times.to_le_bytes());}
    if let Some(snapshot)=snapshot {data.extend_from_slice(snapshot);}
    session.send(if snapshot.is_some() {54}else if times!=0 {52}else if seven {48} else {42}, data, ctl)?;
    let mut began = false;
    let mut latest=None;
    let encoded = loop {
        let (kind, body) = session
            .next(ctl)?
            .ok_or("ZIP worker exited without terminal receipt")?;
        match (kind, body.as_slice()) {
            (47, []) if !began => {
                began = true;
                if PhaseTimer::enabled() {
                    eprintln!("OA_CREATE_WORKER status=encoding");
                }
                ctl.set("compressing", Some(0), "Creating archive");
            }
            (46, [percent]) if began && *percent <= 99 => {
                ctl.set("compressing", Some(*percent), "Creating archive")
            }
            (53, value) if began && times&64!=0 && latest.is_none() && value.len()==8 => { latest=Some(u64::from_le_bytes(value.try_into().unwrap())); }
            (41, []) if !began => break false,
            (43, []) if began => break true,
            (43, msg) if !msg.is_empty() => return Err(String::from_utf8_lossy(msg).into_owned()),
            _ => return Err("invalid ZIP worker receipt".into()),
        }
    };
    if session.next(ctl)?.is_some() {
        return Err("trailing ZIP worker receipt".into());
    }
    loop {
        match session.exit_code()? {
            Some(0) if encoded => break,
            Some(3) if !encoded => break,
            Some(_) => return Err("ZIP worker status differs from its receipt".into()),
            None => {
                ctl.check()?;
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    if encoded && (times&64!=0)!=latest.is_some() {return Err("missing archive timestamp receipt".into());}
    Ok((encoded,latest))
}

fn run_worker() -> Result<(bool, Option<u64>), String> {
    let runtime = runtime_dir()?.ok_or("missing ZIP runtime")?;
    let runtime: Vec<_> = runtime.as_os_str().encode_wide().chain(Some(0)).collect();
    let (kind, bytes) = read_frame(&mut std::io::stdin().lock(), 64*1024*1024+1024)?.ok_or("no ZIP request")?;
    if kind != 42 && kind != 48 && kind != 52 && kind != 54 {
        return Err("invalid ZIP request".into());
    }
    let mut frame = Fields(&bytes);
    let seven=if kind==52 || kind==54 {frame.bit()?}else{kind==48};
    let owner = frame.u64()? as usize as Handle;
    let root = frame.u64()? as usize as Handle;
    let output = frame.u64()? as usize as Handle;
    let name: Vec<_> = frame.wide()?.encode_utf16().chain(Some(0)).collect();
    let level = frame.u32()?;
    let threads = frame.u32()?;
    let explicit = frame.bit()?;
    let (dictionary,solid) = if seven {(frame.u32()?,frame.u32()?)} else {(0,0)};
    if seven && (solid>2 || (dictionary!=0 && !(4096..=1024*1024*1024).contains(&dictionary))) { return Err("invalid 7z create options".into()); }
    let times=if kind==52 || kind==54 {frame.u32()?}else{0};
    if !compression_times::valid_native(times) {return Err("invalid timestamp options".into());}
    let snapshot=if kind==54 {let remaining=frame.0.len();frame.take(remaining)?}else{&[]};
    frame.end()?;
    let mut latest=0u64;
    let mut error = [0u8; 256];
    let status = unsafe {
        if kind==54 {qz_create_snapshot(runtime.as_ptr(),owner,root,name.as_ptr(),output,level,threads,u32::from(explicit),dictionary,solid,u32::from(seven),times,snapshot.as_ptr(),snapshot.len(),&mut latest,error.as_mut_ptr(),error.len())}
        else if kind==52 {qz_create_timed(runtime.as_ptr(),owner,root,name.as_ptr(),output,level,threads,u32::from(explicit),dictionary,solid,u32::from(seven),times,&mut latest,error.as_mut_ptr(),error.len())}
        else if seven { qz_create_7z(runtime.as_ptr(),owner,root,name.as_ptr(),output,level,threads,u32::from(explicit),dictionary,solid,error.as_mut_ptr(),error.len()) }
        else { qz_create_zip(
            runtime.as_ptr(),
            owner,
            root,
            name.as_ptr(),
            output,
            level,
            threads,
            u32::from(explicit),
            error.as_mut_ptr(),
            error.len(),
        ) }
    };
    if status == 1 {
        return Ok((false,None));
    }
    if status != 0 {
        let n = error.iter().position(|b| *b == 0).unwrap_or(error.len());
        return Err(format!(
            "ZIP worker failed {status:#x}: {}",
            String::from_utf8_lossy(&error[..n])
        ));
    }
    Ok((true,if times&64!=0 {Some(latest)}else{None}))
}

pub fn worker() -> u8 {
    let r = run_worker();
    let err = r.as_ref().err().map(String::as_bytes).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    let unsupported = matches!(r, Ok((false,_)));
    if let Ok((true,Some(value)))=&r {
        if out.write_all(&[53]).and_then(|_|out.write_all(&8u32.to_le_bytes())).and_then(|_|out.write_all(&value.to_le_bytes())).is_err(){return 2;}
    }
    if out
        .write_all(&[if unsupported { 41 } else { 43 }])
        .and_then(|_| out.write_all(&(err.len() as u32).to_le_bytes()))
        .and_then(|_| out.write_all(err))
        .is_err()
    {
        return 2;
    }
    if unsupported {
        3
    } else {
        u8::from(r.is_err())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_helper_identity_requires_exact_version_capabilities_and_nonce() {
        let hello = [1u32.to_le_bytes(), 3u32.to_le_bytes()].concat();
        assert_eq!(check_create_hello(CREATE_HELLO, &hello).unwrap(),3);
        let timed=[1u32.to_le_bytes(),7u32.to_le_bytes()].concat();
        assert_eq!(check_create_hello(CREATE_HELLO,&timed).unwrap(),7);
        assert!(check_create_hello(CREATE_HELLO,&[1u32.to_le_bytes(),15u32.to_le_bytes()].concat()).is_err());
        for kind in [0, CREATE_ACK, 41, 43, 47] { assert!(check_create_hello(kind, &hello).is_err()); }
        for body in [vec![], [2u32.to_le_bytes(), 3u32.to_le_bytes()].concat(),
            [1u32.to_le_bytes(), 1u32.to_le_bytes()].concat(), [hello.as_slice(), &[0]].concat()] {
            assert!(check_create_hello(CREATE_HELLO, &body).is_err());
        }
        let nonce = [42u8;16];
        assert!(check_create_ack(CREATE_ACK, &nonce, &nonce).is_ok());
        assert!(check_create_ack(CREATE_HELLO, &nonce, &nonce).is_err());
        assert!(check_create_ack(CREATE_ACK, &[41u8;16], &nonce).is_err());
        assert!(check_create_ack(CREATE_ACK, &nonce[..15], &nonce).is_err());
    }
    #[test]
    fn only_supported_7z_options_enter_the_pinned_create_worker() {
        let inputs=["source".to_owned()];let normal=EngineOptions::default();
        assert!(eligible_7z("7z",&inputs,&normal));
        for solid in [None,Some(true),Some(false)] {
            assert!(eligible_7z("7z",&inputs,&EngineOptions {method:Some("LZMA2".into()),dictionary_bytes:Some(16*1024*1024),threads:Some(4),level:Some(5),solid,..normal.clone()}));
        }
        for opts in [
            EngineOptions {method:Some("LZMA".into()),..normal.clone()},
            EngineOptions {password:Some("secret".into()),..normal.clone()},
            EngineOptions {dictionary_bytes:Some(4095),..normal.clone()},
            EngineOptions {dictionary_bytes:Some(1024*1024*1024+1),..normal.clone()},
            EngineOptions {volume_bytes:Some(1024),..normal.clone()},
            EngineOptions {word_size:Some(32),..normal.clone()},
            EngineOptions {solid_block_bytes:Some(1024),..normal.clone()},
            EngineOptions {header_encryption:Some(false),..normal.clone()},
        ] { assert!(!eligible_7z("7z",&inputs,&opts)); }
        assert!(!eligible_7z("zip",&inputs,&normal));
        assert!(!eligible_7z("7z",&[],&normal));
    }
    #[test]
    fn only_matching_zip_create_options_use_the_worker() {
        let inputs = ["source".to_owned()];
        let normal = EngineOptions::default();
        assert!(eligible("zip", &inputs, &normal));
        assert!(!eligible("7z", &inputs, &normal));
        assert!(!eligible("zip", &[], &normal));
        for options in [
            EngineOptions { word_size: Some(32), ..normal.clone() },
            EngineOptions { solid_block_bytes: Some(1024), ..normal.clone() },
            EngineOptions {
                password: Some("secret".into()),
                ..normal.clone()
            },
            EngineOptions {
                method: Some("store".into()),
                ..normal.clone()
            },
            EngineOptions {
                level: Some(0),
                ..normal.clone()
            },
            EngineOptions {
                volume_bytes: Some(1234),
                ..normal.clone()
            },
            EngineOptions {
                threads: Some(0),
                ..normal.clone()
            },
            EngineOptions {
                threads: Some(65),
                ..normal.clone()
            },
            EngineOptions {
                codepage: Some(932),
                ..normal.clone()
            },
            EngineOptions {
                encryption: Some("aes256".into()),
                ..normal.clone()
            },
            EngineOptions {
                dictionary_bytes: Some(4096),
                ..normal.clone()
            },
            EngineOptions {
                header_encryption: Some(false),
                ..normal.clone()
            },
            EngineOptions {
                solid: Some(false),
                ..normal.clone()
            },
        ] {
            assert!(!eligible("zip", &inputs, &options));
        }
        crate::machine_policy::with_rules([&[], &[], &[], &["blocked codec"]], || {
            assert!(!try_create("missing.zip", &inputs, &normal, &JobControl::new()).unwrap());
        });
    }
}
