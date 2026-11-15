//! Read-only, positioned official ZIP listing. This experimental path is not
//! enabled in production until metadata parity and process-boundary tests pass.
use super::*;
extern "C" {
    pub(super) fn qing_spawn_zip_list_worker(exe: *const u16, process: *mut Handle,
        input: *mut Handle, output: *mut Handle) -> u32;
}

#[derive(Debug)]
struct Property { kind: u32, number: u64, precision: u16, sub: u16, reserved: u16, text: String }
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
        Ok(Self {kind, number, precision, sub, reserved, text})
    }
    fn render(&self, ordinal: usize) -> Result<Option<String>, String> {
        if self.kind == 0 { return Ok(None); }
        let expected = match ordinal { 0|7|8|10|11 => 3, 1|5|12 => 2, 6 => 4, _ => 1 };
        if self.kind != expected { return Err("unexpected ZIP metadata field type".into()); }
        Ok(Some(match self.kind {
            1 if ordinal == 4 => format!("{:08X}", u32::try_from(self.number).map_err(|_| "invalid ZIP CRC")?),
            1 if ordinal == 9 => attributes(u32::try_from(self.number).map_err(|_| "invalid ZIP attributes")?),
            1 => self.number.to_string(),
            2 => if self.number == 1 { "+" } else { "-" }.into(),
            3 => self.text.replace(['\r','\n'], "_"),
            4 => timestamp(self)?,
            _ => unreachable!(),
        }))
    }
}

// Formatting follows the pinned official PropIDUtils.cpp/PropVariantConv.cpp;
// original sources are distributed under the accompanying 7-Zip LGPL license.
fn attributes(mut attrs: u32) -> String {
    let posix = (attrs & 0x8000 != 0).then_some(attrs >> 16);
    if posix.is_some() && attrs & 0xf0000000 != 0 { attrs &= 0x3fff; }
    let mut s = String::new();
    for (i, c) in b"RHS8DAdNTsLCOIEVvX.PU.M......B".iter().enumerate() {
        if attrs & (1 << i) != 0 && *c != b'.' { s.push(*c as char); attrs &= !(1 << i); }
    }
    if attrs != 0 { s.push_str(&format!(" {attrs:08X}")); }
    if let Some(p) = posix {
        let mut chars = vec![b"0pc3d5b7-9lBsDEF"[((p >> 12) & 15) as usize]];
        for i in 0..9 { chars.push(if p & (1 << (8-i)) != 0 { b"rwx"[i % 3] } else { b'-' }); }
        for (mask, x, at, yes, no) in [(0x800, 0x40, 3, b's', b'S'), (0x400, 8, 6, b's', b'S'), (0x200, 1, 9, b't', b'T')] {
            if p & mask != 0 { chars[at] = if p & x != 0 { yes } else { no }; }
        }
        s.push(' '); s.push_str(std::str::from_utf8(&chars).unwrap());
    }
    s
}
fn timestamp(p: &Property) -> Result<String, String> {
    #[repr(C)] struct SystemTime { year:u16, month:u16, weekday:u16, day:u16, hour:u16, minute:u16, second:u16, millis:u16 }
    #[link(name="kernel32")] extern "system" { fn FileTimeToSystemTime(file: *const u64, system: *mut SystemTime) -> i32; }
    if p.number == 0 && p.sub == 0 { return Ok(String::new()); }
    let mut system: SystemTime = unsafe { std::mem::zeroed() };
    if unsafe { FileTimeToSystemTime(&p.number, &mut system) } == 0 { return Ok(String::new()); }
    let digits = if p.precision != 0 && p.precision <= 25 && p.sub < 100 && p.reserved == 0 {
        match p.precision { 1|2 => 0, 3 => 9, 16..=25 => (p.precision-16).min(9), _ => 7 }
    } else { 7 };
    let mut value = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", system.year,system.month,system.day,system.hour,system.minute,system.second);
    if digits != 0 {
        let fractional = format!("{:07}{:02}", p.number % 10_000_000, p.sub % 100);
        value.push('.'); value.push_str(&fractional[..digits as usize]);
    }
    value.push('Z'); Ok(value)
}

/// Only tests currently call this. Offset is derived from a complete single-disk
/// central-directory proof using the same pinned source handle duplicated below.
pub(super) fn try_list(source: &mut File, opts: &EngineOptions, ctl: &JobControl) -> Result<Option<Vec<Entry>>, String> {
    if opts.codepage.is_some() || crate::machine_policy::scope().restricted() { return Ok(None); }
    let Some(limit) = zip_input_probe::zip_search_limit(source, ctl, false)? else { return Ok(None); };
    let offset = limit.checked_sub(1).ok_or("invalid ZIP proof offset")?;
    if offset == 0 { return Ok(None); }
    let Some(runtime) = runtime_dir()? else { return Ok(None); };
    let exe = std::env::current_exe().map_err(|e|e.to_string())?.with_file_name("obz.exe");
    #[cfg(test)]
    let exe = if exe.is_file() {exe} else { exe.parent().unwrap().parent().unwrap().join("obz.exe") };
    if !exe.is_file() { return Ok(None); }
    let _runtime = pin_runtime(&runtime, ctl)?;
    let _image = crate::engine_stream::pin(&exe)?;
    let mut nonce = [0u8;16];
    if unsafe { BCryptGenRandom(std::ptr::null_mut(),nonce.as_mut_ptr(),16,2) } < 0 { return Err("ZIP worker nonce failed".into()); }
    let mut session = Session::start_mode(&exe, 2)?;
    let mut frame = nonce.to_vec(); wide(&mut frame, runtime.as_os_str())?;
    frame.extend_from_slice(&duplicate(&session.child,unsafe{GetCurrentProcess()},0x100000)?.to_le_bytes());
    frame.extend_from_slice(&1u32.to_le_bytes());
    frame.extend_from_slice(&duplicate(&session.child,source.as_raw_handle(),0x100081)?.to_le_bytes());
    frame.extend_from_slice(&source.metadata().map_err(|e|e.to_string())?.len().to_le_bytes());
    wide(&mut frame,OsStr::new("input.zip"))?;
    frame.extend_from_slice(&0u32.to_le_bytes()); session.send(8,frame,ctl)?;
    let mut position = nonce.to_vec(); position.extend_from_slice(&1u32.to_le_bytes());position.extend_from_slice(&offset.to_le_bytes());
    session.send(10,position,ctl)?;
    let mut properties:Vec<Property> = Vec::new();
    let mut entries = Vec::new(); let mut seen = HashSet::new();
    let mut archive_count = 0; let mut entry_count = 0u32; let mut units = 0usize; let mut terminal = None;
    while let Some((kind,body)) = session.next(ctl)? {
        let mut f = Fields(&body);
        if f.take(16)? != nonce || terminal.is_some() { return Err("invalid ZIP worker identity/state".into()); }
        match kind {
            9 => {
                let index = f.u32()?; let ordinal = f.u32()? as usize;
                let prop = Property::read(&mut f)?; f.end()?;
                units = units.checked_add(prop.text.len()).ok_or("ZIP metadata limit")?;
                if units > 16*1024*1024 { return Err("ZIP metadata limit".into()); }
                if index == u32::MAX {
                    if entry_count != 0 || !properties.is_empty() || ordinal != archive_count || ordinal >= 8 { return Err("invalid ZIP archive metadata ordering".into()); }
                    // Tail warnings are left to the CLI until a proof carries the
                    // exact archive extent. No warning is silently discarded.
                    if ordinal < 5 && (prop.number != 0 || !prop.text.is_empty()) { return Ok(None); }
                    archive_count += 1;
                } else {
                    if archive_count != 8 || index != entry_count || ordinal != properties.len() || index >= 65536 { return Err("invalid ZIP item ordering".into()); }
                    properties.push(prop);
                    if properties.len() == 14 {
                        if properties[13].kind != 0 && ((properties[13].number >> 12) & 15) == 10 { return Err("archive symbolic link unsupported".into()); }
                        const NAMES:[&str;14]=["Path","Folder","Size","Packed Size","CRC","Encrypted","Modified","Method","Comment","Attributes","Symbolic Link","Hard Link","Alternate Stream","Posix Attributes"];
                        let values:Vec<Option<String>>=properties.iter().enumerate().map(|(i,p)|p.render(i)).collect::<Result<_,_>>()?;
                        let mut props=listing_properties::Properties::default();
                        for (key,value) in NAMES.iter().zip(&values) { if let Some(value)=value { props.insert(key,value); } }
                        let name=values[0].as_deref().ok_or("missing ZIP member name")?;
                        if let Some(entry)=listing_entry(name,&props,&mut seen)? { entries.push(entry); }
                        properties.clear();entry_count+=1;
                    }
                }
            }
            11 => { let hr=f.u32()?; let count=f.u32()?; f.end()?;
                if hr != 0 { return Ok(None); }
                if archive_count != 8 || count != entry_count || !properties.is_empty() { return Err("incomplete ZIP metadata receipt".into()); }
                terminal=Some(());
            }
            _ => return Err("unexpected ZIP worker frame".into()),
        }
    }
    while session.exit_code()?.is_none() { ctl.check()?; std::thread::sleep(Duration::from_millis(5)); }
    if terminal.is_none() || session.exit_code()? != Some(0) { return Err("ZIP metadata worker failed".into()); }
    ctl.check()?; crate::machine_policy::scope().unchanged()?;
    Ok(Some(entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positioned_zip_metadata_matches_console() {
      for mode in 0..4 {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("embedded.zip");
        let mut zip=zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let mut options=zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
            .compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o640)
            .with_file_comment("first line\nsecond line");
        if mode==1 { options=options.with_aes_encryption(zip::AesMode::Aes256,"known-fixture-password"); }
        if mode==2 { options=options.large_file(true); }
        if mode==3 {
            let ticks=133801234567890123u64;
            let mut extra=vec![0u8;4];extra.extend_from_slice(&1u16.to_le_bytes());extra.extend_from_slice(&24u16.to_le_bytes());
            for _ in 0..3 {extra.extend_from_slice(&ticks.to_le_bytes());}
            options.add_extra_field(0x000a,extra,false).unwrap();
        }
        zip.add_directory("folder/",options.clone()).unwrap();
        zip.start_file("folder/hello.txt",options.clone()).unwrap();
        zip.write_all(b"hello native directory").unwrap();
        zip.start_file("empty.bin",options).unwrap();
        let bytes=zip.finish().unwrap().into_inner();
        let mut file=File::create(&path).unwrap();file.write_all(&vec![0x5a;9*1024*1024]).unwrap();file.write_all(&bytes).unwrap();
        file.write_all(b"harmless trailing data").unwrap();drop(file);
        let opts=EngineOptions::default();let ctl=JobControl::new();
        let expected=list(path.to_str().unwrap(),&opts).unwrap();
        let mut source=crate::engine_stream::pin(&path).unwrap();
        let actual=try_list(&mut source,&opts,&ctl).unwrap().expect("native route must run");
        assert_eq!(serde_json::to_value(actual).unwrap(),serde_json::to_value(expected).unwrap(),"mode {mode}");
      }
    }
}
