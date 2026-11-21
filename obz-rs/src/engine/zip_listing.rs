//! Read-only, positioned official ZIP listing for proven large-prefix inputs.
use super::*;
const ZIP_LIST_VERSION: u32 = 2;
pub(super) const ITEM_PROPERTIES: [&str; 16] = [
    "Path",
    "Folder",
    "Size",
    "Packed Size",
    "CRC",
    "Encrypted",
    "Modified",
    "Method",
    "Comment",
    "Attributes",
    "Symbolic Link",
    "Hard Link",
    "Alternate Stream",
    "Posix Attributes",
    "Created",
    "Accessed",
];
extern "C" {
    pub(super) fn qing_spawn_zip_list_worker(
        exe: *const u16,
        process: *mut Handle,
        input: *mut Handle,
        output: *mut Handle,
    ) -> u32;
}

#[derive(Debug, Default)]
pub(super) struct Property {
    pub(super) kind: u32,
    pub(super) number: u64,
    pub(super) precision: u16,
    pub(super) sub: u16,
    pub(super) reserved: u16,
    pub(super) text: String,
}
impl Property {
    fn read(f: &mut Fields) -> Result<Self, String> {
        let kind = f.u32()?;
        let number = f.u64()?;
        let precision = u16::from_le_bytes(f.take(2)?.try_into().unwrap());
        let sub = u16::from_le_bytes(f.take(2)?.try_into().unwrap());
        let reserved = u16::from_le_bytes(f.take(2)?.try_into().unwrap());
        let text = f.wide()?;
        if kind > 4 || (kind != 3 && !text.is_empty()) || (kind == 2 && number > 1) {
            return Err("invalid ZIP metadata variant".into());
        }
        Ok(Self {
            kind,
            number,
            precision,
            sub,
            reserved,
            text,
        })
    }
    pub(super) fn check_type(&self, ordinal: usize) -> Result<(), String> {
        if ordinal >= ITEM_PROPERTIES.len() {
            return Err("unknown ZIP metadata field".into());
        }
        if self.kind == 0 {
            return Ok(());
        }
        let expected = match ordinal {
            0 | 7 | 8 | 10 | 11 => 3,
            1 | 5 | 12 => 2,
            6 | 14 | 15 => 4,
            _ => 1,
        };
        if self.kind != expected {
            return Err("unexpected archive metadata field type".into());
        }
        Ok(())
    }

    fn render(&self, ordinal: usize) -> Result<Option<String>, String> {
        self.check_type(ordinal)?;
        if self.kind == 0 {
            return Ok(None);
        }
        Ok(Some(match self.kind {
            1 if ordinal == 4 => format!(
                "{:08X}",
                u32::try_from(self.number).map_err(|_| "invalid ZIP CRC")?
            ),
            1 if ordinal == 9 => {
                attributes(u32::try_from(self.number).map_err(|_| "invalid ZIP attributes")?)
            }
            1 => self.number.to_string(),
            2 => if self.number == 1 { "+" } else { "-" }.into(),
            3 => self.text.replace(['\r', '\n'], "_"),
            4 => timestamp(self)?,
            _ => unreachable!(),
        }))
    }
}

fn listing_member(
    properties: &[Property],
    seen: &mut HashSet<String>,
) -> Result<Option<Entry>, String> {
    if properties.len() != ITEM_PROPERTIES.len() {
        return Err("incomplete ZIP item metadata".into());
    }
    if properties[13].kind != 0 && ((properties[13].number >> 12) & 15) == 10 {
        return Err("archive symbolic link unsupported".into());
    }
    let values: Vec<Option<String>> = properties
        .iter()
        .enumerate()
        .map(|(i, p)| p.render(i))
        .collect::<Result<_, _>>()?;
    let mut props = listing_properties::Properties::default();
    for (key, value) in ITEM_PROPERTIES.iter().zip(&values) {
        if let Some(value) = value {
            props.insert(key, value);
        }
    }
    let name = values[0].as_deref().ok_or("missing ZIP member name")?;
    listing_entry(name, &props, seen)
}

// Formatting follows the pinned official PropIDUtils.cpp/PropVariantConv.cpp;
// original sources are distributed under the accompanying 7-Zip LGPL license.
pub(super) fn attributes(mut attrs: u32) -> String {
    let posix = (attrs & 0x8000 != 0).then_some(attrs >> 16);
    if posix.is_some() && attrs & 0xf0000000 != 0 {
        attrs &= 0x3fff;
    }
    let mut s = String::new();
    for (i, c) in b"RHS8DAdNTsLCOIEVvX.PU.M......B".iter().enumerate() {
        if attrs & (1 << i) != 0 && *c != b'.' {
            s.push(*c as char);
            attrs &= !(1 << i);
        }
    }
    if attrs != 0 {
        s.push_str(&format!(" {attrs:08X}"));
    }
    if let Some(p) = posix {
        let mut chars = vec![b"0pc3d5b7-9lBsDEF"[((p >> 12) & 15) as usize]];
        for i in 0..9 {
            chars.push(if p & (1 << (8 - i)) != 0 {
                b"rwx"[i % 3]
            } else {
                b'-'
            });
        }
        for (mask, x, at, yes, no) in [
            (0x800, 0x40, 3, b's', b'S'),
            (0x400, 8, 6, b's', b'S'),
            (0x200, 1, 9, b't', b'T'),
        ] {
            if p & mask != 0 {
                chars[at] = if p & x != 0 { yes } else { no };
            }
        }
        s.push(' ');
        s.push_str(std::str::from_utf8(&chars).unwrap());
    }
    s
}
pub(super) fn timestamp(p: &Property) -> Result<String, String> {
    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        weekday: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn FileTimeToSystemTime(file: *const u64, system: *mut SystemTime) -> i32;
    }
    let ns_valid = p.precision != 0 && p.precision <= 25 && p.sub < 100 && p.reserved == 0;
    let ns = if ns_valid { p.sub } else { 0 };
    if p.number == 0 && ns == 0 {
        return Ok(String::new());
    }
    let mut system: SystemTime = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToSystemTime(&p.number, &mut system) } == 0 {
        return Ok(String::new());
    }
    let digits = if ns_valid {
        match p.precision {
            1 | 2 => 0,
            3 => 9,
            16..=25 => (p.precision - 16).min(9),
            _ => 7,
        }
    } else {
        7
    };
    let mut value = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        system.year, system.month, system.day, system.hour, system.minute, system.second
    );
    if digits != 0 {
        let fractional = format!("{:07}{:02}", p.number % 10_000_000, ns);
        value.push('.');
        value.push_str(&fractional[..digits as usize]);
    }
    value.push('Z');
    Ok(value)
}

/// Offset is derived from a complete single-disk
/// central-directory proof using the same pinned source handle duplicated below.
pub(crate) fn try_list(
    source: &mut File,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<Option<Vec<Entry>>, String> {
    ctl.check()?;
    let policy = crate::machine_policy::scope();
    if opts.codepage.is_some() || policy.restricted() {
        return Ok(None);
    }
    let Some(layout) = zip_input_probe::list_layout(source, ctl)? else {
        return Ok(None);
    };
    let offset = layout.first_local;
    if layout.count > 65536 {
        return Ok(None);
    }
    if offset == 0 {
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
    let _runtime = pin_runtime(&runtime, ctl)?;
    let _image = crate::engine_stream::pin(&exe)?;
    let mut nonce = [0u8; 16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), nonce.as_mut_ptr(), 16, 2) } < 0 {
        return Err("ZIP worker nonce failed".into());
    }
    let mut session = Session::start_mode(&exe, 2)?;
    let mut frame = nonce.to_vec();
    wide(&mut frame, runtime.as_os_str())?;
    frame.extend_from_slice(
        &duplicate(&session.child, unsafe { GetCurrentProcess() }, 0x100000)?.to_le_bytes(),
    );
    frame.extend_from_slice(&1u32.to_le_bytes());
    frame.extend_from_slice(
        &duplicate(&session.child, source.as_raw_handle(), 0x100081)?.to_le_bytes(),
    );
    frame.extend_from_slice(
        &source
            .metadata()
            .map_err(|e| e.to_string())?
            .len()
            .to_le_bytes(),
    );
    wide(&mut frame, OsStr::new("input.zip"))?;
    frame.extend_from_slice(&0u32.to_le_bytes());
    session.send(8, frame, ctl)?;
    let mut position = nonce.to_vec();
    position.extend_from_slice(&ZIP_LIST_VERSION.to_le_bytes());
    position.extend_from_slice(&offset.to_le_bytes());
    session.send(10, position, ctl)?;
    let mut properties: Vec<Property> = Vec::new();
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut archive_count = 0;
    let mut entry_count = 0u32;
    let mut units = 0usize;
    let mut terminal = None;
    while let Some((kind, body)) = session.next(ctl)? {
        let mut f = Fields(&body);
        if f.take(16)? != nonce || terminal.is_some() {
            return Err("invalid ZIP worker identity/state".into());
        }
        match kind {
            9 => {
                #[cfg(test)]
                tests::ON_FRAME.with(|hook| {
                    if let Some(callback) = hook.borrow_mut().take() {
                        callback();
                    }
                });
                ctl.check()?;
                let index = f.u32()?;
                let ordinal = f.u32()? as usize;
                let prop = Property::read(&mut f)?;
                f.end()?;
                units = units
                    .checked_add(prop.text.len())
                    .ok_or("ZIP metadata limit")?;
                if units > 16 * 1024 * 1024 {
                    return Err("ZIP metadata limit".into());
                }
                if index == u32::MAX {
                    if entry_count != 0
                        || !properties.is_empty()
                        || ordinal != archive_count
                        || ordinal >= 8
                    {
                        return Err("invalid ZIP archive metadata ordering".into());
                    }
                    if prop.kind != 0 && prop.kind != if (3..=4).contains(&ordinal) { 3 } else { 1 }
                    {
                        return Err("unexpected ZIP archive metadata type".into());
                    }
                    // Nonzero warnings/errors stay on the CLI path. Native offsets
                    // and extent must independently agree with the source proof.
                    if ordinal < 5 && (prop.number != 0 || !prop.text.is_empty()) {
                        return Ok(None);
                    }
                    match ordinal {
                        5 if prop.kind != 1 || prop.number != layout.start => return Ok(None),
                        6 if prop.kind != 1 || prop.number != layout.end - layout.start => {
                            return Ok(None)
                        }
                        7 if prop.kind != 0
                            && (prop.kind != 1
                                || prop.number
                                    != source.metadata().map_err(|e| e.to_string())?.len()
                                        - layout.end) =>
                        {
                            return Ok(None)
                        }
                        _ => {}
                    }
                    archive_count += 1;
                } else {
                    if archive_count != 8
                        || index != entry_count
                        || ordinal != properties.len()
                        || index >= 65536
                    {
                        return Err("invalid ZIP item ordering".into());
                    }
                    if ordinal == 0 && prop.text.contains(['\r', '\n']) {
                        return Ok(None);
                    }
                    properties.push(prop);
                    if properties.len() == ITEM_PROPERTIES.len() {
                        if let Some(entry) = listing_member(&properties, &mut seen)? {
                            entries.push(entry);
                        }
                        properties.clear();
                        entry_count += 1;
                    }
                }
            }
            11 => {
                let hr = f.u32()?;
                let count = f.u32()?;
                f.end()?;
                if hr != 0 {
                    return Ok(None);
                }
                if archive_count != 8
                    || count != entry_count
                    || u64::from(count) != layout.count
                    || !properties.is_empty()
                {
                    return Err("incomplete ZIP metadata receipt".into());
                }
                terminal = Some(());
            }
            _ => return Err("unexpected ZIP worker frame".into()),
        }
    }
    while session.exit_code()?.is_none() {
        ctl.check()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    if terminal.is_none() || session.exit_code()? != Some(0) {
        return Err("ZIP metadata worker failed".into());
    }
    ctl.check()?;
    policy.unchanged()?;
    Ok(Some(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    thread_local! { pub(super) static ON_FRAME: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) }; }

    // These tests only use in-memory properties/frames; no worker or source file.
    fn times_properties() -> Vec<Property> {
        let mut properties: Vec<_> = (0..ITEM_PROPERTIES.len())
            .map(|_| Property {
                kind: 0,
                number: 0,
                precision: 0,
                sub: 0,
                reserved: 0,
                text: String::new(),
            })
            .collect();
        properties[0].kind = 3;
        properties[0].text = "member.txt".into();
        properties[2].kind = 1;
        properties[2].number = 3;
        properties
    }

    #[test]
    fn zip_member_times_distinct_values_and_precision() {
        let mut properties = times_properties();
        for (ordinal, ticks, precision, sub) in [
            (6, 133801234567890123, 23, 0),
            (14, 133801234577890124, 25, 56),
            (15, 133801234587890125, 2, 0),
        ] {
            properties[ordinal] = Property {
                kind: 4,
                number: ticks,
                precision,
                sub,
                reserved: 0,
                text: String::new(),
            };
        }
        let entry = listing_member(&properties, &mut HashSet::new())
            .unwrap()
            .unwrap();
        assert_eq!(entry.modified, "2024-12-31 12:57:36.7890123Z");
        assert_eq!(entry.created, "2024-12-31 12:57:37.789012456Z");
        assert_eq!(entry.accessed, "2024-12-31 12:57:38Z");
        assert!(entry.size_known);
        assert_eq!(entry.size, 3);
    }

    #[test]
    fn zip_member_times_absent_values_stay_empty() {
        let properties = times_properties();
        let entry = listing_member(&properties, &mut HashSet::new())
            .unwrap()
            .unwrap();
        assert!(entry.modified.is_empty() && entry.created.is_empty() && entry.accessed.is_empty());
        let mut properties = properties;
        properties[14].kind = 4; // Official zero FILETIME also remains empty.
        properties[15].kind = 4;
        let entry = listing_member(&properties, &mut HashSet::new())
            .unwrap()
            .unwrap();
        assert!(entry.created.is_empty() && entry.accessed.is_empty());
    }

    #[test]
    fn zip_member_times_reject_wrong_types_counts_and_existing_unsafe_metadata() {
        let mut properties = times_properties();
        assert!(listing_member(&properties[..14], &mut HashSet::new()).is_err());
        assert!(listing_member(&properties[..15], &mut HashSet::new()).is_err());
        properties.push(Property {
            kind: 0,
            number: 0,
            precision: 0,
            sub: 0,
            reserved: 0,
            text: String::new(),
        });
        assert!(listing_member(&properties, &mut HashSet::new()).is_err());
        let unknown = properties.pop().unwrap();
        assert!(unknown.render(16).is_err());
        for ordinal in [6, 14, 15] {
            for kind in [1, 2, 3] {
                properties[ordinal].kind = kind;
                assert!(listing_member(&properties, &mut HashSet::new()).is_err());
            }
            properties[ordinal].kind = 0;
        }
        let mut seen = HashSet::new();
        listing_member(&properties, &mut seen).unwrap();
        assert!(listing_member(&properties, &mut seen)
            .unwrap_err()
            .contains("duplicate"));
        properties[13].kind = 1;
        properties[13].number = 0xa000;
        assert!(listing_member(&properties, &mut HashSet::new())
            .unwrap_err()
            .contains("symbolic link"));
    }

    #[test]
    fn zip_member_times_protocol_v2_is_synchronized_and_rejects_v1() {
        // Statically inspect the native request guards rather than launching a
        // deliberately incompatible worker on this machine.
        let contract = include_str!("../../native/nana/bridge_contract.h");
        let worker = include_str!("../../native/nana/zip_list_worker.inc");
        let bridge = include_str!("../../native/nana/zip_list.inc");
        assert_eq!(ZIP_LIST_VERSION, 2);
        assert_ne!(ZIP_LIST_VERSION, 1);
        assert!(contract.contains("QZ_ZIP_LIST_VERSION = 2"));
        assert!(worker.contains("position.get<uint32_t>() != QZ_ZIP_LIST_VERSION) return 3;"));
        assert!(worker.contains("QzZipListHost host{QZ_ZIP_LIST_VERSION,"));
        assert!(worker.contains("QzZipListRequest request{QZ_ZIP_LIST_VERSION,"));
        assert!(bridge.contains("request->abi_version != QZ_ZIP_LIST_VERSION"));
        assert!(bridge.contains("host.abi_version != QZ_ZIP_LIST_VERSION"));
        assert!(
            bridge.contains("kpidHardLink, kpidIsAltStream, kpidPosixAttrib, kpidCTime, kpidATime")
        );
    }

    fn small_input(names: &[&str]) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source.zip");
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in names {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"fixture").unwrap();
        }
        let bytes = z.finish().unwrap().into_inner();
        let mut file = File::create(&path).unwrap();
        file.write_all(&vec![0; 8192]).unwrap();
        file.write_all(&bytes).unwrap();
        (temp, path)
    }
    #[test]
    fn native_zip_cancel_after_real_worker_metadata_keeps_pins_and_reaps_child() {
        let (temp, path) = small_input(&["safe.txt"]);
        let original = fs::read(&path).unwrap();
        let mut source = crate::engine_stream::pin(&path).unwrap();
        let ctl = JobControl::new();
        let trigger = ctl.clone();
        let check_path = path.clone();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hit = called.clone();
        ON_FRAME.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                assert_eq!(
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&check_path)
                        .unwrap_err()
                        .raw_os_error(),
                    Some(32)
                );
                assert_eq!(
                    fs::rename(&check_path, check_path.with_extension("moved"))
                        .unwrap_err()
                        .raw_os_error(),
                    Some(32)
                );
                hit.store(true, Ordering::SeqCst);
                trigger.cancel();
            }))
        });
        assert!(try_list(&mut source, &EngineOptions::default(), &ctl)
            .unwrap_err()
            .contains("cancelled"));
        assert!(called.load(Ordering::SeqCst));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        drop(source);
        assert!(fs::OpenOptions::new().write(true).open(&path).is_ok());
    }
    #[test]
    fn native_zip_policy_and_codepage_defer_without_worker() {
        let (_temp, path) = small_input(&["safe.txt"]);
        let mut source = crate::engine_stream::pin(&path).unwrap();
        crate::machine_policy::with_rules([&["zip"], &[], &[], &[]], || {
            assert!(
                try_list(&mut source, &EngineOptions::default(), &JobControl::new())
                    .unwrap()
                    .is_none()
            );
        });
        let options = EngineOptions {
            codepage: Some(936),
            ..Default::default()
        };
        assert!(try_list(&mut source, &options, &JobControl::new())
            .unwrap()
            .is_none());
    }
    #[test]
    fn native_zip_rejects_unselected_traversal_and_case_collisions() {
        for names in [vec!["safe.txt", "../escape"], vec!["a.txt", "A.txt"]] {
            let (_temp, path) = small_input(&names);
            let mut source = crate::engine_stream::pin(&path).unwrap();
            let options = EngineOptions {
                selected: vec!["safe.txt".into()],
                ..Default::default()
            };
            assert!(try_list(&mut source, &options, &JobControl::new()).is_err());
        }
    }
    #[test]
    fn native_zip_multidisk_candidate_defers_and_cancelled_probe_stops() {
        let (_temp, path) = small_input(&["safe.txt"]);
        let mut bytes = fs::read(&path).unwrap();
        let at = bytes.windows(4).position(|p| p == b"PK\x01\x02").unwrap();
        bytes[at + 34] = 1;
        fs::write(&path, bytes).unwrap();
        let mut source = crate::engine_stream::pin(&path).unwrap();
        assert!(
            try_list(&mut source, &EngineOptions::default(), &JobControl::new())
                .unwrap()
                .is_none()
        );
        let ctl = JobControl::new();
        ctl.cancel();
        assert!(try_list(&mut source, &EngineOptions::default(), &ctl)
            .unwrap_err()
            .contains("cancelled"));
    }
    #[test]
    fn native_zip_unknown_size_and_invalid_property_types() {
        let absent = Property {
            kind: 0,
            number: 0,
            precision: 0,
            sub: 0,
            reserved: 0,
            text: String::new(),
        };
        assert_eq!(absent.render(2).unwrap(), None);
        let mut props = listing_properties::Properties::default();
        props.insert("Path", "unknown");
        let entry = listing_entry("unknown", &props, &mut HashSet::new())
            .unwrap()
            .unwrap();
        assert!(!entry.size_known);
        let wrong = Property {
            kind: 3,
            text: "123".into(),
            ..absent
        };
        assert!(wrong.render(2).is_err());
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&[0; 10]);
        assert!(Property::read(&mut Fields(&bytes)).is_err());
    }
    #[test]
    fn positioned_zip_metadata_matches_console() {
        for mode in 0..6 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("embedded.zip");
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let mut options = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o640)
                .with_file_comment("first line\nsecond line");
            if mode == 1 {
                options =
                    options.with_aes_encryption(zip::AesMode::Aes256, "known-fixture-password");
            }
            if mode == 2 {
                options = options.large_file(true);
            }
            if mode == 3 {
                let ticks = 133801234567890123u64;
                let mut extra = vec![0u8; 4];
                extra.extend_from_slice(&1u16.to_le_bytes());
                extra.extend_from_slice(&24u16.to_le_bytes());
                for _ in 0..3 {
                    extra.extend_from_slice(&ticks.to_le_bytes());
                }
                options.add_extra_field(0x000a, extra, false).unwrap();
            }
            zip.add_directory("folder/", options.clone()).unwrap();
            zip.start_file("folder/hello.txt", options.clone()).unwrap();
            zip.write_all(b"hello native directory").unwrap();
            zip.start_file("empty.bin", options).unwrap();
            let mut bytes = zip.finish().unwrap().into_inner();
            if mode == 4 {
                let at = bytes.windows(4).rposition(|p| p == b"PK\x05\x06").unwrap();
                let count = u16::from_le_bytes(bytes[at + 10..at + 12].try_into().unwrap()) as u64;
                let size = u32::from_le_bytes(bytes[at + 12..at + 16].try_into().unwrap()) as u64;
                let offset = u32::from_le_bytes(bytes[at + 16..at + 20].try_into().unwrap()) as u64;
                let mut record = b"PK\x06\x06".to_vec();
                record.extend_from_slice(&44u64.to_le_bytes());
                record.extend_from_slice(&45u16.to_le_bytes());
                record.extend_from_slice(&45u16.to_le_bytes());
                record.extend_from_slice(&[0; 8]);
                record.extend_from_slice(&count.to_le_bytes());
                record.extend_from_slice(&count.to_le_bytes());
                record.extend_from_slice(&size.to_le_bytes());
                record.extend_from_slice(&offset.to_le_bytes());
                record.extend_from_slice(b"PK\x06\x07");
                record.extend_from_slice(&0u32.to_le_bytes());
                record.extend_from_slice(&(at as u64).to_le_bytes());
                record.extend_from_slice(&1u32.to_le_bytes());
                bytes.splice(at..at, record);
            }
            let mut file = File::create(&path).unwrap();
            file.write_all(&vec![0x5a; 9 * 1024 * 1024]).unwrap();
            file.write_all(&bytes).unwrap();
            file.write_all(b"harmless trailing data").unwrap();
            if mode == 5 {
                file.write_all(&vec![b'T'; 3 * 1024 * 1024]).unwrap();
            }
            drop(file);
            let opts = EngineOptions::default();
            let ctl = JobControl::new();
            let output = std::process::Command::new(executable().unwrap())
                .args([
                    "l",
                    "-slt",
                    "-slmu",
                    "-ba",
                    "-sccUTF-8",
                    "-tzip:s10m",
                    "--",
                    path.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            let expected = parse_listing(&output.stdout).unwrap();
            let mut source = crate::engine_stream::pin(&path).unwrap();
            let actual = try_list(&mut source, &opts, &ctl)
                .unwrap()
                .unwrap_or_else(|| panic!("native route must run for mode {mode}"));
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "mode {mode}"
            );
        }
    }
}
