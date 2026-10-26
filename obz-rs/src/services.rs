//! Optional desktop services. These are independent implementations, not recovered
//! Bandisoft code. Operations which persist secrets or modify Explorer integration
//! run only when explicitly called by the user-facing command.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct VaultEntry {
    id: String,
    label: String,
    password: String,
}
#[derive(Default, Serialize, Deserialize)]
struct VaultData {
    entries: Vec<VaultEntry>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct VaultLabel {
    pub id: String,
    pub label: String,
}

pub struct Vault {
    path: PathBuf,
}
impl Vault {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        safe_ancestors(&path)?;
        Ok(Self { path })
    }
    pub fn default_path() -> Result<PathBuf, String> {
        Ok(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?)
                .join("OpenBandiZip/passwords.dpapi"),
        )
    }
    fn write_lock(&self) -> Result<fs::File, String> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        safe_ancestors(parent)?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let lock_path = self.path.with_extension("lock");
        safe_ancestors(&lock_path)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|e| e.to_string())?;
        lock.lock().map_err(|e| e.to_string())?;
        Ok(lock)
    }
    fn load(&self) -> Result<VaultData, String> {
        safe_ancestors(&self.path)?;
        let bytes = match fs::read(&self.path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(VaultData::default()),
            Err(e) => return Err(e.to_string()),
        };
        if !bytes.starts_with(b"OBZV1\0") {
            return Err("unrecognized vault format".into());
        }
        let mut plain = protect(&bytes[6..], false)?;
        let value =
            serde_json::from_slice(&plain).map_err(|_| "invalid vault contents".to_string());
        plain.fill(0);
        value
    }
    fn save(&self, data: &VaultData) -> Result<(), String> {
        let mut plain = serde_json::to_vec(data).map_err(|e| e.to_string())?;
        let encrypted = protect(&plain, true);
        plain.fill(0);
        let encrypted = encrypted?;
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        safe_ancestors(parent)?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(b"OBZV1\0")
            .and_then(|_| temp.write_all(&encrypted))
            .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        safe_ancestors(&self.path)?;
        temp.persist(&self.path).map_err(|e| e.error.to_string())?;
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<VaultLabel>, String> {
        Ok(self
            .load()?
            .entries
            .into_iter()
            .map(|e| VaultLabel {
                id: e.id,
                label: e.label,
            })
            .collect())
    }
    pub fn put(&self, id: &str, label: &str, password: &str) -> Result<(), String> {
        if id.is_empty() || id.len() > 256 || label.len() > 1024 || password.len() > 65536 {
            return Err("invalid vault entry size".into());
        }
        let _lock = self.write_lock()?;
        let mut data = self.load()?;
        data.entries.retain(|e| e.id != id);
        data.entries.push(VaultEntry {
            id: id.into(),
            label: label.into(),
            password: password.into(),
        });
        self.save(&data)
    }
    pub fn get(&self, id: &str) -> Result<String, String> {
        self.load()?
            .entries
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| e.password)
            .ok_or_else(|| "password entry not found".into())
    }
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let _lock = self.write_lock()?;
        let mut data = self.load()?;
        let before = data.entries.len();
        data.entries.retain(|e| e.id != id);
        if data.entries.len() == before {
            return Ok(false);
        }
        self.save(&data)?;
        Ok(true)
    }
}

fn safe_ancestors(path: &Path) -> Result<(), String> {
    for path in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        match fs::symlink_metadata(path) {
            Ok(m) => {
                let mut linked = m.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    linked |= m.file_attributes() & 0x400 != 0;
                }
                if linked {
                    return Err("linked service output path is not permitted".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn protect(input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    #[repr(C)]
    struct Blob {
        len: u32,
        data: *mut u8,
    }
    #[link(name = "Crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "Kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    let len = u32::try_from(input.len()).map_err(|_| "vault too large")?;
    let source = Blob {
        len,
        data: input.as_ptr() as *mut u8,
    };
    let mut output = Blob {
        len: 0,
        data: std::ptr::null_mut(),
    };
    // UI_FORBIDDEN; no LOCAL_MACHINE flag: protected to the current Windows user.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &source,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &source,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(format!(
            "Windows DPAPI failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = unsafe { std::slice::from_raw_parts(output.data, output.len as usize).to_vec() };
    unsafe {
        for i in 0..output.len as usize {
            std::ptr::write_volatile(output.data.add(i), 0);
        }
        LocalFree(output.data.cast());
    }
    Ok(result)
}
#[cfg(not(windows))]
fn protect(_: &[u8], _: bool) -> Result<Vec<u8>, String> {
    Err("password vault requires Windows DPAPI".into())
}

#[derive(Serialize, Deserialize)]
pub struct RepairReport {
    pub recovered: Vec<String>,
    pub skipped: Vec<String>,
    pub output: String,
    pub complete: bool,
}

// PKWARE APPNOTE 6.3.10, sections 4.3.9 and 4.5.3. These readers do not
// depend on a surviving central directory. No data is published until verified.
fn repair_crc(state: u32, bytes: &[u8]) -> u32 {
    const fn table() -> [u32; 256] {
        let mut out = [0; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut j = 0;
            while j < 8 {
                c = if c & 1 != 0 {
                    0xedb88320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                j += 1;
            }
            out[i] = c;
            i += 1;
        }
        out
    }
    const TABLE: [u32; 256] = table();
    bytes.iter().fold(state, |c, b| {
        TABLE[((c ^ *b as u32) & 255) as usize] ^ (c >> 8)
    })
}
fn repair_charge(work: &mut u64, count: u64) -> Result<(), String> {
    if count > *work {
        *work = 0;
        return Err("repair verification work limit exceeded".into());
    }
    *work -= count;
    Ok(())
}
/// Return exact compressed bytes consumed, uncompressed length, and CRC without
/// allocating the uncompressed file. StreamEnd is required, never merely EOF.
fn repair_inflate(data: &[u8], limit: u64, work: &mut u64) -> Result<(usize, u64, u32), String> {
    use miniz_oxide::{
        inflate::stream::{inflate, InflateState},
        DataFormat, MZFlush, MZStatus,
    };
    let mut state = InflateState::new_boxed(DataFormat::Raw);
    let mut input = 0;
    let mut output = 0u64;
    let mut crc = u32::MAX;
    let mut chunk = [0u8; 65536];
    loop {
        let available = usize::try_from(limit.saturating_sub(output).saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(chunk.len());
        let result = inflate(
            &mut state,
            &data[input..],
            &mut chunk[..available],
            MZFlush::None,
        );
        repair_charge(work, (result.bytes_consumed + result.bytes_written) as u64)?;
        input += result.bytes_consumed;
        output = output
            .checked_add(result.bytes_written as u64)
            .filter(|n| *n <= limit)
            .ok_or("repair output exceeds byte limit")?;
        crc = repair_crc(crc, &chunk[..result.bytes_written]);
        match result.status {
            Ok(MZStatus::StreamEnd) => return Ok((input, output, !crc)),
            Ok(_) => {}
            Err(_) => return Err("invalid or truncated deflate stream".into()),
        }
        if result.bytes_consumed == 0 && result.bytes_written == 0 {
            return Err("deflate stream made no progress".into());
        }
    }
}
fn repair_descriptor(data: &[u8], zip64: bool, crc: u32, packed: u64, size: u64) -> Option<usize> {
    // CRC can itself equal the optional marker; test both layouts.
    for prefix in [4, 0] {
        if prefix == 4 && !data.starts_with(b"PK\x07\x08") {
            continue;
        }
        let count = if zip64 { 20 } else { 12 };
        let Some(d) = data.get(prefix..prefix + count) else {
            continue;
        };
        if u32::from_le_bytes(d[..4].try_into().ok()?) != crc {
            continue;
        }
        let (p, s) = if zip64 {
            (
                u64::from_le_bytes(d[4..12].try_into().ok()?),
                u64::from_le_bytes(d[12..20].try_into().ok()?),
            )
        } else {
            (
                u32::from_le_bytes(d[4..8].try_into().ok()?) as u64,
                u32::from_le_bytes(d[8..12].try_into().ok()?) as u64,
            )
        };
        if p == packed && s == size {
            return Some(prefix + count);
        }
    }
    None
}
fn repair_boundary(data: &[u8]) -> bool {
    data.is_empty()
        || [
            b"PK\x03\x04",
            b"PK\x01\x02",
            b"PK\x05\x06",
            b"PK\x06\x06",
            b"PK\x06\x07",
        ]
        .iter()
        .any(|s| data.starts_with(*s))
}
fn repair_stored_descriptor(
    data: &[u8],
    original: &[u8],
    packed_prefix: u64,
    zip64: bool,
    limit: u64,
    work: &mut u64,
) -> Result<(usize, u64, u32, usize), String> {
    let mut crc = u32::MAX;
    for length in 0..=data.len().min(usize::try_from(limit).unwrap_or(usize::MAX)) {
        repair_charge(work, 1)?;
        if let Some(n) = repair_descriptor(
            &original[length..],
            zip64,
            !crc,
            length as u64 + packed_prefix,
            length as u64,
        ) {
            if repair_boundary(&original[length + n..]) {
                return Ok((length, length as u64, !crc, n));
            }
        }
        if length < data.len() {
            crc = repair_crc(crc, &data[length..length + 1]);
        }
    }
    Err("no CRC-valid bounded Store data descriptor".into())
}
/// UTF-8 flag is authoritative. Otherwise use a CRC-bound Unicode Path extra
/// field if present, then explicit codepage or ZIP's CP437 legacy default.
fn repair_filename(
    raw: &[u8],
    flags: u16,
    extra: &[u8],
    codepage: Option<u32>,
) -> Result<String, String> {
    if flags & 0x800 != 0 {
        return std::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| "invalid UTF-8 flagged ZIP filename".into());
    }
    let mut fields = crate::util::Reader::new(extra);
    let mut unicode = None;
    while fields.bytes_left() > 0 {
        let tag = fields.u16()?;
        let n = fields.u16()? as usize;
        let value = fields.read(n)?;
        if tag == 0x7075
            && value.len() >= 5
            && value[0] == 1
            && u32::from_le_bytes(value[1..5].try_into().unwrap()) == crate::util::crc32(raw)
        {
            let name =
                std::str::from_utf8(&value[5..]).map_err(|_| "invalid Unicode ZIP path extra")?;
            if unicode.as_deref().is_some_and(|previous| previous != name) {
                return Err("conflicting Unicode ZIP path extras".into());
            }
            unicode = Some(name.to_owned());
        }
    }
    if let Some(name) = unicode {
        return Ok(name);
    }
    let cp = codepage.unwrap_or(437);
    if cp == 65001 {
        return std::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| "invalid UTF-8 ZIP filename".into());
    }
    #[cfg(windows)]
    {
        #[link(name = "Kernel32")]
        extern "system" {
            fn MultiByteToWideChar(
                cp: u32,
                flags: u32,
                src: *const u8,
                n: i32,
                dest: *mut u16,
                capacity: i32,
            ) -> i32;
        }
        if cp == 0 || cp == 1 || cp == 2 || cp == 3 {
            return Err(
                "repair codepage must be an explicit numeric encoding (e.g. 437,936,65001)".into(),
            );
        }
        let n = i32::try_from(raw.len()).map_err(|_| "ZIP filename too long")?;
        let count = unsafe { MultiByteToWideChar(cp, 8, raw.as_ptr(), n, std::ptr::null_mut(), 0) };
        if count == 0 {
            return Err(format!("invalid ZIP filename for codepage {cp}"));
        }
        let mut wide = vec![0u16; count as usize];
        let actual =
            unsafe { MultiByteToWideChar(cp, 8, raw.as_ptr(), n, wide.as_mut_ptr(), count) };
        if actual != count {
            return Err("ZIP filename conversion failed".into());
        }
        String::from_utf16(&wide).map_err(|_| "invalid decoded ZIP filename".into())
    }
    #[cfg(not(windows))]
    {
        if raw.is_ascii() {
            Ok(String::from_utf8(raw.to_vec()).unwrap())
        } else {
            Err(format!("legacy codepage {cp} repair requires Windows"))
        }
    }
}
fn repair_single_zip(
    local: &[u8],
    h: &[u8],
    name: &[u8],
    extra: &[u8],
    packed: u64,
    size: u64,
    crc: u32,
) -> Result<Vec<u8>, String> {
    if local.len() >= u32::MAX as usize || packed >= u32::MAX as u64 || size >= u32::MAX as u64 {
        return Err("repair candidate exceeds classic output bound".into());
    }
    let mut output = local.to_vec();
    let offset = output.len() as u32;
    let mut c = vec![0u8; 46];
    c[..4].copy_from_slice(b"PK\x01\x02");
    c[4..6].copy_from_slice(&20u16.to_le_bytes());
    c[6..30].copy_from_slice(&h[4..28]);
    c[16..20].copy_from_slice(&crc.to_le_bytes());
    c[20..24].copy_from_slice(&(packed as u32).to_le_bytes());
    c[24..28].copy_from_slice(&(size as u32).to_le_bytes());
    c[30..32].copy_from_slice(&(extra.len() as u16).to_le_bytes());
    output.extend_from_slice(&c);
    output.extend_from_slice(name);
    output.extend_from_slice(extra);
    let length = output.len() as u32 - offset;
    output.extend_from_slice(b"PK\x05\x06");
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&1u16.to_le_bytes());
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(&offset.to_le_bytes());
    output.extend_from_slice(&[0; 2]);
    Ok(output)
}
fn repair_verify_aes(
    candidate: Vec<u8>,
    password: &str,
    size: u64,
    crc: u32,
    version: u16,
    work: &mut u64,
) -> Result<(), String> {
    repair_charge(work, candidate.len() as u64)?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(candidate))
        .map_err(|e| format!("AES candidate rejected: {e}"))?;
    let mut member = archive
        .by_index_decrypt(0, password.as_bytes())
        .map_err(|_| "AES password or metadata mismatch")?;
    let mut buffer = RepairSecret(vec![0u8; 65536]);
    let mut actual = 0u64;
    let mut actual_crc = u32::MAX;
    loop {
        let available = usize::try_from(size.saturating_sub(actual).saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(buffer.0.len());
        let n = member
            .read(&mut buffer.0[..available])
            .map_err(|_| "AES authentication, CRC, or compressed data verification failed")?;
        if n == 0 {
            break;
        }
        repair_charge(work, n as u64)?;
        actual = actual
            .checked_add(n as u64)
            .filter(|n| *n <= size)
            .ok_or("AES output exceeds declared limit")?;
        actual_crc = repair_crc(actual_crc, &buffer.0[..n]);
    }
    // zip 9.1 finalizes the AES reader at decompressor EOF, including any
    // unread ciphertext, before returning EOF. AE-2 mandates CRC=0 and uses MAC.
    if actual != size || (version == 1 && (!actual_crc) != crc) {
        return Err("AES size or CRC mismatch".into());
    }
    Ok(())
}
fn repair_aes(
    local_prefix: &[u8],
    h: &[u8],
    name: &[u8],
    extra: &[u8],
    payload: &[u8],
    zip64: bool,
    descriptor: bool,
    packed: u64,
    size: u64,
    crc: u32,
    limit: u64,
    password: &str,
    work: &mut u64,
) -> Result<(usize, u64, u32, usize), String> {
    let mut extras = crate::util::Reader::new(extra);
    let mut version = None;
    while extras.bytes_left() > 0 {
        let tag = extras.u16()?;
        let n = extras.u16()? as usize;
        let value = extras.read(n)?;
        if tag == 0x9901 {
            if version.is_some()
                || value.len() != 7
                || &value[2..4] != b"AE"
                || !matches!(value[4], 1..=3)
            {
                return Err("invalid AES extra field".into());
            }
            let v = u16::from_le_bytes(value[..2].try_into().unwrap());
            let method = u16::from_le_bytes(value[5..7].try_into().unwrap());
            if !matches!(v, 1 | 2) || !matches!(method, 0 | 8) {
                return Err("unsupported AES version or compression method".into());
            }
            version = Some(v);
        }
    }
    let version = version.ok_or("missing AES metadata")?;
    if descriptor {
        repair_charge(work, payload.len() as u64)?;
    }
    let mut verify =
        |length: usize, size: u64, crc: u32, descriptor_len: usize| -> Result<(), String> {
            if size > limit {
                return Err("AES output exceeds repair byte limit".into());
            }
            if version == 2 && crc != 0 {
                return Err("AE-2 CRC must be zero".into());
            }
            let data = payload
                .get(..length + descriptor_len)
                .ok_or("truncated AES payload")?;
            let mut local = local_prefix.to_vec();
            local.extend_from_slice(data);
            let candidate = repair_single_zip(&local, h, name, extra, length as u64, size, crc)?;
            repair_verify_aes(candidate, password, size, crc, version, work)
        };
    if !descriptor {
        let length = usize::try_from(packed).map_err(|_| "packed size overflow")?;
        verify(length, size, crc, 0)?;
        return Ok((length, size, crc, 0));
    }
    let mut trials = 0;
    // AES ciphertext has no self-delimiting compression boundary. Only consider
    // descriptors whose packed length matches their exact position, then require
    // full password+MAC+size verification before accepting a candidate.
    for length in 20..payload.len() {
        for prefix in [4, 0] {
            if prefix == 4 && !payload[length..].starts_with(b"PK\x07\x08") {
                continue;
            }
            let width = if zip64 { 20 } else { 12 };
            let Some(d) = payload.get(length + prefix..length + prefix + width) else {
                continue;
            };
            let candidate_crc = u32::from_le_bytes(d[..4].try_into().unwrap());
            let (p, s) = if zip64 {
                (
                    u64::from_le_bytes(d[4..12].try_into().unwrap()),
                    u64::from_le_bytes(d[12..20].try_into().unwrap()),
                )
            } else {
                (
                    u32::from_le_bytes(d[4..8].try_into().unwrap()) as u64,
                    u32::from_le_bytes(d[8..12].try_into().unwrap()) as u64,
                )
            };
            if p != length as u64
                || s > limit
                || !repair_boundary(&payload[length + prefix + width..])
            {
                continue;
            }
            trials += 1;
            if trials > 64 {
                return Err("AES descriptor candidate limit exceeded".into());
            }
            if verify(length, s, candidate_crc, prefix + width).is_ok() {
                return Ok((length, s, candidate_crc, prefix + width));
            }
        }
    }
    Err("no authenticated AES data descriptor".into())
}
// Traditional PKZIP encryption per APPNOTE 6.1. CRC/size validation below
// is mandatory: the single-byte password header check alone is not sufficient.
struct RepairSecret(Vec<u8>);
impl Drop for RepairSecret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
fn repair_zipcrypto(
    data: &[u8],
    password: &str,
    check_byte: u8,
    work: &mut u64,
) -> Result<RepairSecret, String> {
    if data.len() < 12 {
        return Err("truncated ZipCrypto header".into());
    }
    repair_charge(work, data.len() as u64)?;
    let mut keys = [0x12345678u32, 0x23456789, 0x34567890];
    fn update(keys: &mut [u32; 3], byte: u8) {
        keys[0] = repair_crc(keys[0], &[byte]);
        keys[1] = keys[1]
            .wrapping_add(keys[0] & 255)
            .wrapping_mul(134775813)
            .wrapping_add(1);
        keys[2] = repair_crc(keys[2], &[(keys[1] >> 24) as u8]);
    }
    for b in password.bytes() {
        update(&mut keys, b);
    }
    let mut output = RepairSecret(Vec::with_capacity(data.len()));
    for (i, &encrypted) in data.iter().enumerate() {
        let t = (keys[2] | 2) & 0xffff;
        let byte = encrypted ^ ((t.wrapping_mul(t ^ 1) >> 8) as u8);
        update(&mut keys, byte);
        output.0.push(byte);
        if i == 11 && byte != check_byte {
            keys.fill(0);
            return Err("ZipCrypto password header mismatch".into());
        }
    }
    keys.fill(0);
    Ok(output)
}
/// Salvage CRC-valid Store/Deflate records, including ZIP64 local sizes and
/// signed/unsigned data descriptors. Output is a new classic ZIP (under 4 GiB
/// and 65535 members); ZIP64 input is supported within that bounded output size.
pub fn repair_zip(input: &str, output: &str, max_bytes: u64) -> Result<RepairReport, String> {
    repair_zip_with_password(input, output, max_bytes, None)
}
/// Password is used only to verify encrypted content; recovered members retain
/// their original encryption. Never logs or stores decrypted payloads.
pub fn repair_zip_with_password(
    input: &str,
    output: &str,
    max_bytes: u64,
    password: Option<&str>,
) -> Result<RepairReport, String> {
    repair_zip_with_options(input, output, max_bytes, password, None)
}
/// Explicit legacy filename codepage; UTF-8 flag and valid Unicode Path extras
/// take precedence. None uses CP437, not the machine's changing OEM locale.
pub fn repair_zip_with_options(
    input: &str,
    output: &str,
    max_bytes: u64,
    password: Option<&str>,
    codepage: Option<u32>,
) -> Result<RepairReport, String> {
    if max_bytes == 0 {
        return Err("repair byte limit must be positive".into());
    }
    let source = fs::File::open(input).map_err(|e| e.to_string())?;
    if source.metadata().map_err(|e| e.to_string())?.len() > max_bytes {
        return Err("repair input exceeds byte limit".into());
    }
    if Path::new(output).exists() {
        return Err(
            "repair requires a new output path; existing files are never overwritten".into(),
        );
    }
    safe_ancestors(Path::new(output))?;
    let mut bytes = Vec::new();
    source
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err("repair input exceeds byte limit".into());
    }
    let mut locals = Vec::new();
    let mut central = Vec::new();
    let mut recovered = Vec::new();
    let mut skipped = Vec::new();
    let mut pos = 0;
    let mut unpacked = 0u64;
    let mut names = std::collections::HashSet::new();
    let mut files = std::collections::HashSet::new();
    let mut parents = std::collections::HashSet::new();
    let mut work = max_bytes.saturating_mul(4);
    let mut candidates = 0;
    while pos + 4 <= bytes.len() {
        if &bytes[pos..pos + 4] != b"PK\x03\x04" {
            pos += 1;
            continue;
        }
        candidates += 1;
        if candidates > 100000 {
            return Err("repair local record candidate limit exceeded".into());
        }
        let record = pos;
        let attempt = (|| -> Result<usize, String> {
            let h = bytes.get(pos..pos + 30).ok_or("truncated local header")?;
            let u16at = |i| u16::from_le_bytes([h[i], h[i + 1]]);
            let u32at = |i| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
            let flags = u16at(6);
            let method = u16at(8);
            let expected_crc = u32at(14);
            let mut packed = u32at(18) as u64;
            let mut size = u32at(22) as u64;
            let name_len = u16at(26) as usize;
            let extra_len = u16at(28) as usize;
            if flags & (64 | 8192) != 0 {
                return Err("strong ZIP encryption repair unsupported".into());
            }
            let encrypted = flags & 1 != 0;
            if encrypted && password.is_none() {
                return Err("encrypted local record requires password-aware repair".into());
            }
            if !matches!(method, 0 | 8 | 99) {
                return Err(format!("compression method {method} unsupported"));
            }
            if method == 99 && !encrypted {
                return Err("AES entry lacks encryption flag".into());
            }
            let start = pos
                .checked_add(30 + name_len + extra_len)
                .ok_or("record overflow")?;
            let extra = bytes
                .get(pos + 30 + name_len..start)
                .ok_or("truncated local extra field")?;
            let raw_name = bytes
                .get(pos + 30..pos + 30 + name_len)
                .ok_or("truncated filename")?;
            let name = repair_filename(raw_name, flags, extra, codepage)?;
            crate::extract::relative_path(&name)?;
            let output_name_len =
                u16::try_from(name.len()).map_err(|_| "UTF-8 ZIP filename too long")?;
            let normalized = name.replace('\\', "/").trim_end_matches('/').to_lowercase();
            if names.contains(&normalized) {
                return Err("duplicate archive name".into());
            }
            for (i, _) in normalized.match_indices('/') {
                if files.contains(&normalized[..i]) {
                    return Err("file/directory name collision".into());
                }
            }
            if !name.ends_with('/') && parents.contains(&normalized) {
                return Err("file/directory name collision".into());
            }
            let mut cursor = crate::util::Reader::new(extra);
            let mut zip64 = false;
            let mut retained_extra = Vec::new();
            while cursor.bytes_left() > 0 {
                let kind = cursor.u16()?;
                let n = cursor.u16()? as usize;
                let value = cursor.read(n)?;
                if kind != 0x7075 {
                    retained_extra.extend_from_slice(&kind.to_le_bytes());
                    retained_extra.extend_from_slice(&(n as u16).to_le_bytes());
                    retained_extra.extend_from_slice(value);
                }
                if kind == 1 {
                    if zip64 {
                        return Err("duplicate ZIP64 extra field".into());
                    }
                    zip64 = true;
                    let mut z = crate::util::Reader::new(value);
                    if size == u32::MAX as u64 {
                        size = z.u64()?;
                    }
                    if packed == u32::MAX as u64 {
                        packed = z.u64()?;
                    }
                }
            }
            if !zip64 && (size == u32::MAX as u64 || packed == u32::MAX as u64) {
                return Err("missing ZIP64 size extra field".into());
            }
            let remaining = max_bytes
                .checked_sub(unpacked)
                .ok_or("repair output exceeds byte limit")?;
            let original_payload = bytes.get(start..).ok_or("truncated content")?;
            let secret;
            let packed_prefix = if encrypted { 12 } else { 0 };
            let payload = if encrypted && method != 99 {
                let encrypted_data = if flags & 8 == 0 {
                    original_payload
                        .get(..usize::try_from(packed).map_err(|_| "packed size overflow")?)
                        .ok_or("truncated encrypted content")?
                } else {
                    original_payload
                };
                secret = repair_zipcrypto(
                    encrypted_data,
                    password.unwrap(),
                    if flags & 8 != 0 {
                        h[11]
                    } else {
                        (expected_crc >> 24) as u8
                    },
                    &mut work,
                )?;
                &secret.0[12..]
            } else {
                original_payload
            };
            let (length, size, crc, descriptor) = if method == 99 {
                repair_aes(
                    &bytes[record..start],
                    h,
                    raw_name,
                    extra,
                    original_payload,
                    zip64,
                    flags & 8 != 0,
                    packed,
                    size,
                    expected_crc,
                    remaining,
                    password.unwrap(),
                    &mut work,
                )?
            } else if flags & 8 != 0 {
                if method == 8 {
                    let (length, size, crc) = repair_inflate(payload, remaining, &mut work)?;
                    let descriptor = repair_descriptor(
                        &original_payload[length + packed_prefix..],
                        zip64,
                        crc,
                        (length + packed_prefix) as u64,
                        size,
                    )
                    .ok_or("missing or mismatched data descriptor")?;
                    (length + packed_prefix, size, crc, descriptor)
                } else {
                    let (length, size, crc, descriptor) = repair_stored_descriptor(
                        payload,
                        &original_payload[packed_prefix..],
                        packed_prefix as u64,
                        zip64,
                        remaining,
                        &mut work,
                    )?;
                    (length + packed_prefix, size, crc, descriptor)
                }
            } else {
                if size > remaining {
                    return Err("repair output exceeds byte limit".into());
                }
                let length = usize::try_from(packed)
                    .map_err(|_| "packed size overflow")?
                    .checked_sub(packed_prefix)
                    .ok_or("truncated encrypted header")?;
                let comp = payload
                    .get(..length)
                    .ok_or("truncated compressed content")?;
                let (consumed, actual, crc) = if method == 8 {
                    repair_inflate(comp, size, &mut work)?
                } else {
                    repair_charge(&mut work, packed)?;
                    (length, length as u64, crate::util::crc32(comp))
                };
                if consumed != length || actual != size || crc != expected_crc {
                    return Err("size or CRC mismatch".into());
                }
                (length + packed_prefix, size, crc, 0)
            };
            if name.ends_with('/') && size != 0 {
                return Err("directory has nonempty content".into());
            }
            let end = start
                .checked_add(length + descriptor)
                .ok_or("record overflow")?;
            let comp = &bytes[start..start + length];
            if recovered.len() >= 65534
                || size >= u32::MAX as u64
                || length >= u32::MAX as usize
                || locals
                    .len()
                    .checked_add(30 + name.len() + retained_extra.len() + length + descriptor)
                    .filter(|n| *n < u32::MAX as usize)
                    .is_none()
            {
                return Err("repaired ZIP exceeds classic ZIP output limit".into());
            }
            let offset = locals.len() as u32;
            let mut header = h.to_vec();
            header[4..6].copy_from_slice(&20u16.to_le_bytes());
            header[6..8].copy_from_slice(&0x0800u16.to_le_bytes());
            header[14..18].copy_from_slice(&crc.to_le_bytes());
            header[18..22].copy_from_slice(&(length as u32).to_le_bytes());
            header[22..26].copy_from_slice(&(size as u32).to_le_bytes());
            header[26..28].copy_from_slice(&output_name_len.to_le_bytes());
            header[28..30].copy_from_slice(&0u16.to_le_bytes());
            if encrypted {
                // Keep flag 3 and descriptor: ZipCrypto uses modification time for
                // its header check when this flag is set. Clearing it breaks keys.
                let mut encrypted_header = h.to_vec();
                encrypted_header[6..8].copy_from_slice(&(flags | 0x800).to_le_bytes());
                encrypted_header[26..28].copy_from_slice(&output_name_len.to_le_bytes());
                encrypted_header[28..30]
                    .copy_from_slice(&(retained_extra.len() as u16).to_le_bytes());
                locals.extend_from_slice(&encrypted_header);
                locals.extend_from_slice(name.as_bytes());
                locals.extend_from_slice(&retained_extra);
                locals.extend_from_slice(&bytes[start..end]);
            } else {
                locals.extend_from_slice(&header);
                locals.extend_from_slice(name.as_bytes());
                locals.extend_from_slice(comp);
            }
            let mut c = vec![0u8; 46];
            c[..4].copy_from_slice(b"PK\x01\x02");
            c[4..6].copy_from_slice(&20u16.to_le_bytes());
            c[6..30].copy_from_slice(&header[4..28]);
            c[42..46].copy_from_slice(&offset.to_le_bytes());
            if encrypted {
                c[6..14].copy_from_slice(&h[4..12]);
                c[8..10].copy_from_slice(&(flags | 0x800).to_le_bytes());
                c[30..32].copy_from_slice(&(retained_extra.len() as u16).to_le_bytes());
            }
            if name.ends_with('/') {
                c[38..42].copy_from_slice(&0x10u32.to_le_bytes());
            } else {
                files.insert(normalized.clone());
            }
            central.extend_from_slice(&c);
            central.extend_from_slice(name.as_bytes());
            if encrypted {
                central.extend_from_slice(&retained_extra);
            }
            for (i, _) in normalized.match_indices('/') {
                parents.insert(normalized[..i].to_owned());
            }
            names.insert(normalized);
            recovered.push(name.to_owned());
            unpacked += size;
            Ok(end)
        })();
        match attempt {
            Ok(end) => pos = end,
            Err(why) => {
                if skipped.len() < 1000 {
                    skipped.push(format!("offset {record}: {why}"));
                }
                pos += 4;
            }
        }
        if work == 0 {
            return Err("repair verification work limit exceeded".into());
        }
    }
    if recovered.is_empty() {
        return Err(format!(
            "no CRC-valid recoverable local records ({} candidates rejected)",
            skipped.len()
        ));
    }
    let count = recovered.len() as u16;
    let cd_offset = locals.len() as u32;
    let cd_size = u32::try_from(central.len()).map_err(|_| "central directory too large")?;
    locals.extend_from_slice(&central);
    locals.extend_from_slice(b"PK\x05\x06");
    locals.extend_from_slice(&[0; 4]);
    locals.extend_from_slice(&count.to_le_bytes());
    locals.extend_from_slice(&count.to_le_bytes());
    locals.extend_from_slice(&cd_size.to_le_bytes());
    locals.extend_from_slice(&cd_offset.to_le_bytes());
    locals.extend_from_slice(&[0; 2]);
    let parent = Path::new(output)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&locals).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    safe_ancestors(Path::new(output))?;
    temp.persist_noclobber(output)
        .map_err(|e| e.error.to_string())?;
    Ok(RepairReport {
        recovered,
        skipped,
        output: output.into(),
        complete: false,
    })
}

#[derive(Serialize, Deserialize)]
pub struct RecoveryReport {
    pub password: Option<String>,
    pub attempts: usize,
    pub status: String,
}

/// Checks only the explicitly supplied bounded candidate set. The checker must
/// test encrypted content (a nonencrypted archive cannot validate a password).
pub fn recover_password<I, F>(
    candidates: I,
    max_attempts: usize,
    cancel: &AtomicBool,
    mut check: F,
) -> Result<RecoveryReport, String>
where
    I: IntoIterator<Item = String>,
    F: FnMut(&str) -> Result<bool, String>,
{
    if max_attempts == 0 || max_attempts > 1_000_000 {
        return Err("password recovery limit must be 1..=1000000".into());
    }
    let mut attempts = 0;
    for candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Ok(RecoveryReport {
                password: None,
                attempts,
                status: "cancelled".into(),
            });
        }
        if attempts == max_attempts {
            return Ok(RecoveryReport {
                password: None,
                attempts,
                status: "limit_reached".into(),
            });
        }
        attempts += 1;
        if check(&candidate)? {
            return Ok(RecoveryReport {
                password: Some(candidate),
                attempts,
                status: "found".into(),
            });
        }
    }
    Ok(RecoveryReport {
        password: None,
        attempts,
        status: "exhausted".into(),
    })
}

/// Lazy, deterministic character-set search. Work is bounded by max_attempts;
/// the Cartesian product is never materialized in memory.
pub fn recover_charset_password<F>(
    charset: &str,
    min_len: usize,
    max_len: usize,
    max_attempts: usize,
    cancel: &AtomicBool,
    check: F,
) -> Result<RecoveryReport, String>
where
    F: FnMut(&str) -> Result<bool, String>,
{
    let mut chars = Vec::new();
    for c in charset.chars() {
        if c.is_control() {
            return Err("recovery charset cannot contain control characters".into());
        }
        if !chars.contains(&c) {
            chars.push(c);
        }
    }
    if chars.is_empty() || chars.len() > 256 || min_len == 0 || min_len > max_len || max_len > 64 {
        return Err("charset recovery requires 1..256 unique characters and lengths 1..64".into());
    }
    struct Candidates {
        chars: Vec<char>,
        digits: Vec<usize>,
        max: usize,
        finished: bool,
    }
    impl Iterator for Candidates {
        type Item = String;
        fn next(&mut self) -> Option<String> {
            if self.finished {
                return None;
            }
            let word = self.digits.iter().map(|&i| self.chars[i]).collect();
            let mut carry = true;
            for i in (0..self.digits.len()).rev() {
                self.digits[i] += 1;
                if self.digits[i] < self.chars.len() {
                    carry = false;
                    break;
                }
                self.digits[i] = 0;
            }
            if carry {
                if self.digits.len() == self.max {
                    self.finished = true;
                } else {
                    self.digits.push(0);
                }
            }
            Some(word)
        }
    }
    recover_password(
        Candidates {
            chars,
            digits: vec![0; min_len],
            max: max_len,
            finished: false,
        },
        max_attempts,
        cancel,
        check,
    )
}

/// Create an ISO/Joliet image using Windows' IMAPI2 filesystem authoring API.
/// This does not burn a disc, mount an image, or change system configuration.
pub fn create_iso(inputs: &[String], output: &str) -> Result<String, String> {
    create_iso_with_cancel(inputs, output, &AtomicBool::new(false))
}

pub fn create_iso_with_cancel(
    inputs: &[String],
    output: &str,
    cancel: &AtomicBool,
) -> Result<String, String> {
    if !cfg!(windows) {
        return Err("ISO creation requires Windows IMAPI2FS".into());
    }
    if inputs.is_empty() {
        return Err("ISO inputs cannot be empty".into());
    }
    let output = Path::new(output);
    if output.exists() {
        return Err("ISO output already exists".into());
    }
    safe_ancestors(output)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    // Keep staging outside the output/source tree so creating an ISO inside an
    // input directory cannot recursively copy the staging directory into itself.
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let staging = temp.path().join("content");
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    fn copy_input(source: &Path, target: &Path, cancel: &AtomicBool) -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("operation cancelled".into());
        }
        safe_ancestors(source)?;
        let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            fs::create_dir(target).map_err(|e| e.to_string())?;
            for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                copy_input(&entry.path(), &target.join(entry.file_name()), cancel)?;
            }
        } else if metadata.is_file() {
            let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)
                .map_err(|e| e.to_string())?;
            let mut buf = vec![0; 1024 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("operation cancelled".into());
                }
                let n = input.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                output.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        } else {
            return Err("non-regular ISO input rejected".into());
        }
        Ok(())
    }
    for input in inputs {
        let source = Path::new(input);
        let canonical_source = fs::canonicalize(source).map_err(|e| e.to_string())?;
        if source.is_dir()
            && fs::canonicalize(temp.path())
                .map_err(|e| e.to_string())?
                .starts_with(&canonical_source)
        {
            return Err("ISO source contains the temporary staging directory".into());
        }
        let name = source.file_name().ok_or("ISO input must have a filename")?;
        let name_text = name.to_str().ok_or("ISO filename is not Unicode")?;
        crate::extract::relative_path(name_text)?;
        copy_input(source, &staging.join(name), cancel)?;
    }
    let script = temp.path().join("create-image.ps1");
    fs::write(&script, include_str!("services_iso.ps1")).map_err(|e| e.to_string())?;
    let image = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| e.to_string())?
        .into_temp_path();
    let stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot unavailable")?)
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut cmd = Command::new(powershell);
    hidden(&mut cmd);
    let mut child = cmd
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-Source")
        .arg(fs::canonicalize(&staging).map_err(|e| e.to_string())?)
        .arg("-Output")
        .arg(fs::canonicalize(&image).map_err(|e| e.to_string())?)
        .stdout(Stdio::from(stdout.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(stdout.try_clone().map_err(|e| e.to_string())?))
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || started.elapsed() > Duration::from_secs(3600) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("ISO creation cancelled or exceeded one hour".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        use std::io::{Seek, SeekFrom};
        let mut stdout = stdout;
        stdout.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        stdout
            .take(8192)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        return Err(format!(
            "IMAPI ISO creation failed: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    safe_ancestors(output)?;
    image
        .persist_noclobber(output)
        .map_err(|e| e.error.to_string())?;
    Ok(output.to_string_lossy().into_owned())
}

fn hidden(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.stdin(Stdio::null());
}

#[derive(Serialize, Deserialize)]
pub struct ScanReport {
    pub scanner: String,
    pub exit_code: Option<i32>,
    pub status: String,
    pub details: String,
}

pub fn defender_path() -> Result<PathBuf, String> {
    let base = PathBuf::from(std::env::var_os("ProgramData").ok_or("ProgramData unavailable")?)
        .join("Microsoft/Windows Defender/Platform");
    let mut candidates: Vec<_> = fs::read_dir(base)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path().join("MpCmdRun.exe"))
        .filter(|p| p.is_file())
        .collect();
    candidates.sort();
    candidates
        .pop()
        .or_else(|| {
            std::env::var_os("ProgramFiles")
                .map(PathBuf::from)
                .map(|p| p.join("Windows Defender/MpCmdRun.exe"))
                .filter(|p| p.is_file())
        })
        .ok_or_else(|| "Microsoft Defender command-line scanner unavailable".into())
}

/// Microsoft custom scan with remediation explicitly disabled. Exit 2 is kept
/// ambiguous (threat or operational error), never reported as a clean result.
pub fn scan_defender(path: &str) -> Result<ScanReport, String> {
    let target = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let scanner = defender_path()?;
    let log = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&scanner);
    hidden(&mut cmd);
    let mut child = cmd
        .args([
            "-Scan",
            "-ScanType",
            "3",
            "-DisableRemediation",
            "-Timeout",
            "1",
            "-File",
        ])
        .arg(target)
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(300) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Defender scan exceeded 5 minute limit; result unknown".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    use std::io::{Seek, SeekFrom};
    let mut log = log;
    log.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    log.take(16384)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(ScanReport { scanner: scanner.to_string_lossy().into_owned(), exit_code: status.code(), status: if status.success() { "no_threat_reported" } else { "threat_or_scan_error" }.into(), details: format!("Custom scan with remediation disabled. Nonzero results require review in Windows Security; encrypted archive contents may be inaccessible to the scanner.\n{}",String::from_utf8_lossy(&bytes)) })
}

/// Explorer cannot launch extended-length device paths in a shell command.
fn explorer_executable_path(raw: &str) -> Result<String, String> {
    let path = if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else { raw.strip_prefix(r"\\?\").unwrap_or(raw).to_owned() };
    if path.contains(['"', '\0', '\r', '\n']) || path.starts_with(r"\\.\") {
        return Err("invalid executable path".into());
    }
    Ok(path)
}

/// Generates the exact owned Explorer registration; useful to review or test
/// without modifying the registry. No default-association keys are touched.
pub fn shell_menu_plan(exe: &str) -> Result<Vec<Vec<String>>, String> {
    let path = fs::canonicalize(exe).map_err(|e| e.to_string())?;
    if !path.is_file() { return Err("请选择可执行程序文件".into()); }
    let raw = path.to_str().ok_or("executable path is not Unicode")?;
    let exe = explorer_executable_path(raw)?;
    let key = r"HKCU\Software\Classes\*\shell\qing-zip";
    Ok(vec![
        vec![
            "add".into(),
            key.into(),
            "/ve".into(),
            "/t".into(),
            "REG_SZ".into(),
            "/d".into(),
            "使用 qing-zip 打开".into(),
            "/f".into(),
        ],
        vec![
            "add".into(),
            key.into(),
            "/v".into(),
            "Icon".into(),
            "/t".into(),
            "REG_SZ".into(),
            "/d".into(),
            format!("\"{exe}\",0"),
            "/f".into(),
        ],
        vec![
            "add".into(),
            format!("{key}\\command"),
            "/ve".into(),
            "/t".into(),
            "REG_SZ".into(),
            "/d".into(),
            format!("\"{exe}\" \"%1\""),
            "/f".into(),
        ],
    ])
}

pub fn shell_menu(enable: bool, exe: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("Explorer menu integration requires Windows".into());
    }
    let operations = if enable {
        shell_menu_plan(exe)?
    } else {
        vec![vec![
            "delete".into(),
            r"HKCU\Software\Classes\*\shell\qing-zip".into(),
            "/f".into(),
        ]]
    };
    let reg = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot unavailable")?)
        .join("System32/reg.exe");
    // Retire the previous product-owned key without touching other shell handlers.
    for legacy in [r"HKCU\Software\Classes\*\shell\OpenBandiZip", r"HKCU\Software\Classes\*\shell\OpenArchive"] {
    let mut query = Command::new(&reg); hidden(&mut query);
    if query.args(["query", legacy]).output().map_err(|e| e.to_string())?.status.success() {
        let mut remove = Command::new(&reg); hidden(&mut remove);
        if !remove.args(["delete", legacy, "/f"]).output().map_err(|e| e.to_string())?.status.success() {
            return Err("无法移除旧版右键入口".into());
        }
    }
    }
    if !enable {
        let mut query = Command::new(&reg); hidden(&mut query);
        if !query.args(["query", r"HKCU\Software\Classes\*\shell\qing-zip"]).output().map_err(|e|e.to_string())?.status.success() {return Ok(());}
    }
    for args in operations {
        let mut cmd = Command::new(&reg);
        hidden(&mut cmd);
        let result = cmd.args(args).output().map_err(|e| e.to_string())?;
        if !result.status.success() {
            return Err(
                "Explorer menu registration failed; no default associations were changed".into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn vault_roundtrip_never_persists_plaintext() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vault.dat");
        let vault = Vault::open(&path).unwrap();
        vault
            .put("example", "test", "unique-secret-123中文")
            .unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.windows(13).any(|w| w == b"unique-secret"));
        assert_eq!(vault.get("example").unwrap(), "unique-secret-123中文");
        vault.put("example", "updated", "new-secret").unwrap();
        assert_eq!(vault.list().unwrap().len(), 1);
        assert_eq!(vault.get("example").unwrap(), "new-secret");
        assert!(vault.remove("example").unwrap());
        assert!(vault.get("example").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn concurrent_vault_writes_keep_all_entries() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("vault.dat");
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    Vault::open(path)
                        .unwrap()
                        .put(&format!("entry-{i}"), "test", "secret")
                        .unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(Vault::open(path).unwrap().list().unwrap().len(), 8);
    }
    #[test]
    fn repair_truncated_directory_preserves_original() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("original.txt");
        fs::write(&src, b"recoverable bytes").unwrap();
        let zip = temp.path().join("damaged.zip");
        crate::zipfmt::create_zip(
            zip.to_str().unwrap(),
            &[src.to_str().unwrap().into()],
            false,
        )
        .unwrap();
        let mut bytes = fs::read(&zip).unwrap();
        let cd = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes.truncate(cd);
        fs::write(&zip, &bytes).unwrap();
        let out = temp.path().join("fixed.zip");
        let report = repair_zip(zip.to_str().unwrap(), out.to_str().unwrap(), 1024 * 1024).unwrap();
        assert_eq!(report.recovered, ["original.txt"]);
        assert!(!report.complete);
        assert_eq!(fs::read(&zip).unwrap(), bytes);
        let fixed = fs::read(&out).unwrap();
        let list = crate::zipfmt::read_central_dir(&fixed, 0, fixed.len()).unwrap();
        assert_eq!(
            crate::zipfmt::extract_entry(&fixed, 0, &list[0]).unwrap(),
            b"recoverable bytes"
        );
        assert!(repair_zip(zip.to_str().unwrap(), out.to_str().unwrap(), 1024 * 1024).is_err());
    }
    #[test]
    fn recovery_is_bounded_and_cancellable() {
        let cancelled = AtomicBool::new(false);
        let r = recover_password(["wrong".into(), "correct".into()], 10, &cancelled, |p| {
            Ok(p == "correct")
        })
        .unwrap();
        assert_eq!(r.status, "found");
        assert_eq!(r.attempts, 2);
        let r = recover_password(["a".into(), "b".into()], 1, &cancelled, |_| Ok(false)).unwrap();
        assert_eq!(r.status, "limit_reached");
        assert_eq!(r.attempts, 1);
        let r = recover_password(["a".into()], 5, &cancelled, |_| Ok(false)).unwrap();
        assert_eq!(r.status, "exhausted");
        cancelled.store(true, Ordering::Relaxed);
        let r =
            recover_password(["a".into()], 1, &cancelled, |_| panic!("must not check")).unwrap();
        assert_eq!(r.status, "cancelled");
        assert_eq!(r.attempts, 0);
    }
    #[test]
    fn charset_recovery_is_lazy_and_bounded() {
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let report = recover_charset_password("aba", 1, 2, 20, &cancel, |p| {
            seen.push(p.to_owned());
            Ok(p == "ba")
        })
        .unwrap();
        assert_eq!(seen, ["a", "b", "aa", "ab", "ba"]);
        assert_eq!(report.password.as_deref(), Some("ba"));
        let report = recover_charset_password("abc", 1, 64, 2, &cancel, |_| Ok(false)).unwrap();
        assert_eq!(report.status, "limit_reached");
        assert_eq!(report.attempts, 2);
        assert!(recover_charset_password("", 1, 5, 2, &cancel, |_| Ok(false)).is_err());
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "Windows IMAPI2FS and external 7-Zip interoperability smoke test"]
    fn iso_creation_roundtrip_with_chinese_name() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("中文.txt");
        fs::write(&input, b"ISO interoperability").unwrap();
        let output = temp.path().join("created.iso");
        create_iso(
            &[input.to_string_lossy().into_owned()],
            output.to_str().unwrap(),
        )
        .unwrap();
        let opts = crate::engine::EngineOptions::default();
        let entries = crate::engine::list(output.to_str().unwrap(), &opts).unwrap();
        assert!(entries.iter().any(|e| e.name == "中文.txt"));
        let dest = temp.path().join("out");
        crate::engine::extract(output.to_str().unwrap(), dest.to_str().unwrap(), &opts).unwrap();
        assert_eq!(
            fs::read(dest.join("中文.txt")).unwrap(),
            b"ISO interoperability"
        );
        assert!(create_iso(
            &[input.to_string_lossy().into_owned()],
            output.to_str().unwrap()
        )
        .is_err());
    }
    #[test]
    fn shell_plan_only_uses_owned_per_user_keys() {
        let commands = shell_menu_plan(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
        assert_eq!(commands.len(), 3);
        for command in commands {
            assert!(command[1].starts_with(r"HKCU\Software\Classes\*\shell\qing-zip"));
            assert!(!command.iter().any(|s| s.contains("UserChoice")));
        }
    }
    #[test]
    fn explorer_commands_use_shell_compatible_paths() {
        assert_eq!(explorer_executable_path(r"\\?\C:\中文 folder\qing-zip.exe").unwrap(), r"C:\中文 folder\qing-zip.exe");
        assert_eq!(explorer_executable_path(r"\\?\UNC\server\share\qing-zip.exe").unwrap(), r"\\server\share\qing-zip.exe");
        assert!(explorer_executable_path("bad\"path.exe").is_err());
        let commands=shell_menu_plan(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
        assert!(!commands[2][6].contains(r"\\?\"));
        assert!(commands[2][6].ends_with("\" \"%1\""));
        assert!(shell_menu_plan(std::env::temp_dir().to_str().unwrap()).is_err());
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "requires locally enabled Microsoft Defender; bounded real-service smoke test"]
    fn defender_smoke_scan_benign_fixture() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("benign.txt");
        fs::write(&file, b"OpenBandiZip benign scan fixture").unwrap();
        let report = scan_defender(file.to_str().unwrap()).unwrap();
        println!("{}", serde_json::to_string(&report).unwrap());
        assert!(matches!(
            report.status.as_str(),
            "no_threat_reported" | "threat_or_scan_error"
        ));
        assert_eq!(fs::read(file).unwrap(), b"OpenBandiZip benign scan fixture");
    }
}
