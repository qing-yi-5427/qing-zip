//! Batched directory records from the bundled, pinned NanaZip RAR handlers.
use super::{
    zip_listing::{attributes, timestamp, Property, ITEM_PROPERTIES},
    *,
};

const RAR_LIST_VERSION: u32 = 1;
const MAX_ITEMS: u32 = 512_000;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MISSING_VOLUME: u32 = 0x8007_0002;
const BUFFER_OVERFLOW: u32 = 0x8007_006f;

extern "C" {
    pub(super) fn qing_spawn_rar_list_worker(
        exe: *const u16,
        process: *mut Handle,
        input: *mut Handle,
        output: *mut Handle,
    ) -> u32;
}

fn read_property(f: &mut Fields<'_>) -> Result<Property, String> {
    let mut prop = Property {
        kind: u32::from(f.take(1)?[0]),
        ..Property::default()
    };
    match prop.kind {
        0 => {}
        1 => prop.number = f.u64()?,
        2 => {
            prop.number = u64::from(f.take(1)?[0]);
            if prop.number > 1 {
                return Err("invalid RAR metadata flag".into());
            }
        }
        3 => {
            prop.text = f.wide()?;
            if prop.text.contains('\0') {
                return Err("embedded NUL in RAR metadata".into());
            }
        }
        4 => {
            prop.number = f.u64()?;
            prop.precision = u16::from_le_bytes(f.take(2)?.try_into().unwrap());
            prop.sub = u16::from_le_bytes(f.take(2)?.try_into().unwrap());
            prop.reserved = u16::from_le_bytes(f.take(2)?.try_into().unwrap());
        }
        _ => return Err("invalid RAR metadata variant".into()),
    }
    Ok(prop)
}

fn read_batch(
    f: &mut Fields<'_>,
    seen: &mut HashSet<String>,
    entries: &mut Vec<Entry>,
    total: &mut u32,
) -> Result<(), String> {
    let rows = f.u32()?;
    if rows == 0 || rows > MAX_ITEMS - *total {
        return Err("RAR directory exceeds supported entry limit".into());
    }
    let mut props: [Property; ITEM_PROPERTIES.len()] = std::array::from_fn(|_| Property::default());
    for _ in 0..rows {
        for (ordinal, property) in props.iter_mut().enumerate() {
            *property = read_property(f)?;
            property.check_type(ordinal)?;
        }
        if let Some(entry) = rar_entry(&mut props, seen)? {
            entries.push(entry);
        }
    }
    *total += rows;
    f.end()?;
    Ok(())
}

fn rar_entry(
    props: &mut [Property; ITEM_PROPERTIES.len()],
    seen: &mut HashSet<String>,
) -> Result<Option<Entry>, String> {
    let [path, folder, size, packed, crc, encrypted, modified, method, comment, attrs, symbolic_link, hard_link, alternate_stream, posix, created, accessed] =
        props;
    if [&symbolic_link.text, &hard_link.text]
        .iter()
        .any(|link| !link.is_empty() && *link != "-")
        || alternate_stream.number != 0
    {
        return Err("archive links or alternate streams are unsupported".into());
    }
    if (posix.number >> 12) & 15 == 10 {
        return Err("archive symbolic link unsupported".into());
    }
    if path.kind == 0 || path.text.contains(['\r', '\n']) {
        return Err("invalid archive entry name".into());
    }
    let attributes = if attrs.kind == 0 {
        String::new()
    } else {
        attributes(u32::try_from(attrs.number).map_err(|_| "invalid RAR attributes")?)
    };
    let Some(is_dir) = validate_listing_member(&path.text, folder.number != 0, &attributes, seen)?
    else {
        return Ok(None);
    };
    for value in [&mut method.text, &mut comment.text] {
        if value.contains(['\r', '\n']) {
            *value = value.replace(['\r', '\n'], "_");
        }
    }
    Ok(Some(Entry {
        name: std::mem::take(&mut path.text),
        size_known: is_dir || size.kind != 0,
        size: size.number,
        packed_size: packed.number,
        is_dir,
        crc: if crc.kind == 0 {
            String::new()
        } else {
            format!(
                "{:08X}",
                u32::try_from(crc.number).map_err(|_| "invalid RAR CRC")?
            )
        },
        encrypted: encrypted.number != 0,
        modified: timestamp(modified)?,
        method: std::mem::take(&mut method.text),
        comment: std::mem::take(&mut comment.text),
        attributes,
        created: timestamp(created)?,
        accessed: timestamp(accessed)?,
    }))
}

// These hints only choose whether to capture sibling handles. Open, the
// volume proof and all directory interpretation still belong to NanaZip.
fn read_vint(input: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..=63).step_by(7) {
        let (&byte, rest) = input.split_first()?;
        *input = rest;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn rar5_may_have_volumes(mut header: &[u8]) -> Option<bool> {
    // Four CRC bytes precede the sized main header. An encryption header
    // does not disclose the archive flags until after password processing.
    header = header.get(4..)?;
    let _size = read_vint(&mut header)?;
    if read_vint(&mut header)? != 1 {
        return None;
    }
    let flags = read_vint(&mut header)?;
    if flags & 1 != 0 {
        read_vint(&mut header)?;
    }
    if flags & 2 != 0 {
        read_vint(&mut header)?;
    }
    Some(read_vint(&mut header)? & 1 != 0)
}

fn rar_format(source: &mut File) -> Result<Option<(u32, bool)>, String> {
    let mut header = [0u8; 80];
    source.rewind().map_err(|e| e.to_string())?;
    let mut size = 0;
    while size < header.len() {
        let read = source
            .read(&mut header[size..])
            .map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        size += read;
    }
    let header = &header[..size];
    Ok(if header.starts_with(b"Rar!\x1a\x07\x01\0") {
        Some((5, rar5_may_have_volumes(&header[8..]).unwrap_or(true)))
    } else if header.starts_with(b"Rar!\x1a\x07\0") {
        Some((
            4,
            header
                .get(9..=10)
                .is_none_or(|h| h[0] != 0x73 || h[1] & 1 != 0),
        ))
    } else {
        None
    })
}

fn rar_leaf(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    let ext = ext.as_bytes();
    ext.eq_ignore_ascii_case(b"rar")
        || (ext.len() == 3
            && matches!(ext[0].to_ascii_lowercase(), b'r'..=b'z')
            && ext[1..].iter().all(u8::is_ascii_digit))
}

type Pins = Vec<crate::engine_stream::PinnedSource>;
pub(super) fn direct_volumes(
    path: &Path, source: &mut File, ctl: &JobControl,
) -> Result<Option<(u32, NamedPins)>, String> {
    let Some((format, may_have_volumes)) = rar_format(source)? else { return Ok(None) };
    let Some(leaf) = path.file_name().and_then(OsStr::to_str) else { return Ok(None) };
    if may_have_volumes && !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("rar")) {
        return Ok(None);
    }
    if matches!(crate::deep_volume_plan::classify(leaf, &[]), crate::deep_volume_plan::Member::Later { .. }) {
        return Ok(None);
    }
    if may_have_volumes {
        capture_siblings(path, leaf, ctl).map(|parts| parts.map(|parts| (format, parts)))
    } else {
        Ok(Some((format, Vec::new())))
    }
}
pub(super) type NamedPins = Vec<(String, crate::engine_stream::PinnedSource)>;
fn capture_siblings(
    path: &Path,
    leaf: &str,
    ctl: &JobControl,
) -> Result<Option<NamedPins>, String> {
    let parent = path.parent().ok_or("RAR archive has no parent directory")?;
    let Ok(directory) = std::fs::read_dir(parent) else {
        return Ok(None);
    };
    let mut candidates = Vec::new();
    for (index, item) in directory.enumerate() {
        if index % 256 == 0 {
            ctl.check()?;
        }
        if index == 65_536 {
            return Ok(None);
        }
        let Ok(item) = item else {
            return Ok(None);
        };
        let Ok(name) = item.file_name().into_string() else {
            continue;
        };
        if rar_leaf(&name) {
            candidates.push(name);
        }
    }
    let mut parts = Vec::new();
    for name in crate::deep_volume_plan::root_family(leaf, &candidates) {
        if name.eq_ignore_ascii_case(leaf) {
            if name != leaf && candidates.iter().any(|candidate| candidate == leaf) {
                return Ok(None);
            }
            continue;
        }
        if parts.len() == 1023 {
            return Ok(None);
        }
        ctl.check()?;
        let Ok(pin) = crate::engine_stream::pin(&parent.join(&name)) else {
            return Ok(None);
        };
        parts.push((name, pin));
    }
    Ok(Some(parts))
}

pub(crate) fn try_list(
    path: &str,
    source: &mut File,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<Option<(Vec<Entry>, Pins)>, String> {
    let policy = crate::machine_policy::scope();
    if policy.restricted() || opts.codepage.is_some() {
        return Ok(None);
    }
    let Some((format, may_have_volumes)) = rar_format(source)? else {
        return Ok(None);
    };
    let path = absolute(path)?;
    let Some(leaf) = path.file_name().and_then(OsStr::to_str) else {
        return Ok(None);
    };
    if may_have_volumes
        && !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rar"))
    {
        return Ok(None);
    }
    if matches!(
        crate::deep_volume_plan::classify(leaf, &[]),
        crate::deep_volume_plan::Member::Later { .. }
    ) {
        return Ok(None);
    }
    let Some(runtime) = runtime_dir()? else {
        return Ok(None);
    };
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("obz.exe");
    #[cfg(test)]
    let exe = if exe.is_file() {
        exe
    } else {
        exe.parent().unwrap().parent().unwrap().join("obz.exe")
    };
    if !exe.is_file() {
        return Ok(None);
    }
    let parts = if may_have_volumes {
        let Some(parts) = capture_siblings(&path, leaf, ctl)? else {
            return Ok(None);
        };
        parts
    } else {
        Vec::new()
    };
    if let Some(password) = &opts.password {
        crate::engine_password::validate(password)?;
    }
    let _timer = PhaseTimer::new("list-rar-structured");
    let _runtime = pin_runtime(&runtime, ctl)?;
    let _image = crate::engine_stream::pin(&exe)?;
    let mut nonce = [0u8; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 16, 2) } < 0 {
        return Err("RAR worker nonce failed".into());
    }
    let mut session = Session::start_mode(&exe, 4)?;
    let mut frame = nonce.to_vec();
    wide(&mut frame, runtime.as_os_str())?;
    frame.extend_from_slice(
        &duplicate(&session.child, unsafe { GetCurrentProcess() }, 0x100000)?.to_le_bytes(),
    );
    frame.extend_from_slice(&((parts.len() + 1) as u32).to_le_bytes());
    for (name, file) in std::iter::once((leaf, &*source))
        .chain(parts.iter().map(|(name, file)| (name.as_str(), &**file)))
    {
        frame.extend_from_slice(
            &duplicate(&session.child, file.as_raw_handle(), 0x100081)?.to_le_bytes(),
        );
        frame.extend_from_slice(
            &file
                .metadata()
                .map_err(|e| e.to_string())?
                .len()
                .to_le_bytes(),
        );
        wide(&mut frame, OsStr::new(name))?;
    }
    let password = opts.password.as_deref().unwrap_or_default();
    let units = password.encode_utf16().count();
    if units > MAX_PASSWORD_UNITS {
        return Err("password input length exceeds limit".into());
    }
    frame.extend_from_slice(&(units as u32).to_le_bytes());
    session.send(12, frame, ctl)?;
    if units != 0 {
        session.send(6, SecretFrame::new(password), ctl)?;
    }
    let mut request = nonce.to_vec();
    request.extend_from_slice(&RAR_LIST_VERSION.to_le_bytes());
    request.extend_from_slice(&format.to_le_bytes());
    session.send(10, request, ctl)?;

    let mut bytes = 0usize;
    let mut count = 0u32;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut received = false;
    let mut total = None;
    let mut previous_percent = 0;
    let mut metadata_phase = None;
    while let Some((kind, body)) = session.next(ctl)? {
        let mut f = Fields(&body);
        if f.take(16)? != nonce || received {
            return Err("invalid RAR worker identity/state".into());
        }
        match kind {
            12 if total.is_none() => {
                let announced = f.u32()?;
                f.end()?;
                if announced > MAX_ITEMS {
                    return Err("RAR directory exceeds supported entry limit".into());
                }
                total = Some(announced);
                metadata_phase = Some(PhaseTimer::new("list-rar-entries"));
                entries.reserve(announced as usize);
                ctl.set("listing", Some(0), "Reading archive entries");
            }
            9 => {
                let announced = total.ok_or("missing RAR item total")?;
                bytes += body.len();
                if bytes > MAX_BYTES {
                    return Err("RAR directory exceeds supported metadata limit".into());
                }
                read_batch(&mut f, &mut seen, &mut entries, &mut count)?;
                if count > announced {
                    return Err("unexpected RAR metadata rows".into());
                }
                let percent = (count * 100 / announced).min(99) as u8;
                if percent > previous_percent {
                    previous_percent = percent;
                    ctl.set("listing", Some(percent), "Reading archive entries");
                }
            }
            11 => {
                let status = f.u32()?;
                let reported = f.u32()?;
                let phase = f.u32()?;
                f.end()?;
                if status != 0 && PhaseTimer::enabled() {
                    eprintln!("OA_RAR_LIST status={status:08x} phase={phase}");
                }
                policy.unchanged()?;
                match status {
                    0 if reported == count && total == Some(count) => received = true,
                    0 => return Err("incomplete RAR metadata receipt".into()),
                    MISSING_VOLUME => return Err("RAR archive is missing a required volume".into()),
                    BUFFER_OVERFLOW => {
                        return Err("RAR directory exceeds supported metadata limit".into())
                    }
                    _ => {
                        ctl.set("listing", None, "Reading archive directory");
                        return Ok(None); // Preserve official diagnostics, including password prompts.
                    }
                }
            }
            _ => return Err("unexpected RAR worker frame".into()),
        }
    }
    if !received || session.wait_exit(ctl)? != 0 {
        return Err("RAR metadata worker failed".into());
    }
    drop(metadata_phase);
    ctl.check()?;
    policy.unchanged()?;
    Ok(Some((
        entries,
        parts.into_iter().map(|(_, file)| file).collect(),
    )))
}
