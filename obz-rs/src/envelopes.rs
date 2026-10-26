//! AES Crypt, lzip, UUencode and XXencode containers with private verified staging.
use crate::engine::{self, EngineOptions, Entry};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 512 * 1024 * 1024;
fn check(opts: &EngineOptions) -> Result<(), String> {
    if let Some(c) = &opts.control {
        c.check()?;
    }
    if !["error", "skip", "rename", "overwrite"].contains(&opts.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    Ok(())
}
struct Checked<T> {
    inner: T,
    opts: EngineOptions,
    written: u64,
}
impl<T: Read> Read for Checked<T> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        check(&self.opts).map_err(std::io::Error::other)?;
        self.inner.read(b)
    }
}
impl<T: Seek> Seek for Checked<T> {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        check(&self.opts).map_err(std::io::Error::other)?;
        self.inner.seek(p)
    }
}
impl<T: Write> Write for Checked<T> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        check(&self.opts).map_err(std::io::Error::other)?;
        if b.len() as u64 > LIMIT.saturating_sub(self.written) {
            return Err(std::io::Error::other(
                "decoded envelope exceeds512MiB limit",
            ));
        }
        let n = self.inner.write(b)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
fn kind(path: &str) -> Result<&'static str, String> {
    let f = File::open(path).map_err(|e| e.to_string())?;
    let mut b = Vec::new();
    f.take(65536)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.starts_with(b"AES") && b.get(3).is_some_and(|v| *v <= 3) {
        Ok("AES Crypt")
    } else if b.starts_with(b"LZIP") {
        Ok("lzip")
    } else if b.split(|c| *c == b'\n').any(|l| l.starts_with(b"begin ")) {
        Ok("UU/XXencode")
    } else {
        Err("unsupported envelope format".into())
    }
}
pub fn is_supported(path: &str) -> bool {
    kind(path).is_ok()
}
fn name(path: &str, kind: &str) -> Result<String, String> {
    let path = Path::new(path);
    let mut name = path
        .file_stem()
        .ok_or("missing filename")?
        .to_string_lossy()
        .to_string();
    if kind == "lzip"
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("tlz"))
    {
        name.push_str(".tar")
    };
    engine::validate_name(&name)?;
    Ok(name)
}
fn entry(name: String, size: u64, packed: u64, kind: &str) -> Entry {
    Entry {
        name,
        size,
        packed_size: packed,
        is_dir: false,
        crc: String::new(),
        encrypted: kind == "AES Crypt",
        modified: String::new(),
        method: kind.into(),
        comment: String::new(),
        attributes: String::new(),
    }
}
// Cap unauthenticated extension/KDF values before entering the crypto routine.
fn aes_preflight(file: &mut File) -> Result<(), String> {
    let mut head = [0; 5];
    file.read_exact(&mut head).map_err(|e| e.to_string())?;
    if head[3] >= 2 {
        let mut extensions = 0u64;
        loop {
            let mut n = [0; 2];
            file.read_exact(&mut n).map_err(|e| e.to_string())?;
            let len = u16::from_be_bytes(n) as u64;
            if len == 0 {
                break;
            }
            extensions = extensions
                .checked_add(len)
                .filter(|n| *n <= 1024 * 1024)
                .ok_or("AES extensions exceed1MiB limit")?;
            file.seek(SeekFrom::Current(len as i64))
                .map_err(|e| e.to_string())?;
        }
        if head[3] == 3 {
            let mut n = [0; 4];
            file.read_exact(&mut n).map_err(|e| e.to_string())?;
            if u32::from_be_bytes(n) > 2_000_000 {
                return Err("AES KDF exceeds2000000 iteration budget".into());
            }
        }
    }
    file.rewind().map_err(|e| e.to_string())?;
    Ok(())
}
fn encoded_text(bytes: &[u8], opts: &EngineOptions) -> Result<String, String> {
    if opts.codepage.is_none() {
        if let Ok(s) = std::str::from_utf8(bytes) {
            return Ok(s.to_owned());
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MultiByteToWideChar(
                cp: u32,
                flags: u32,
                input: *const u8,
                len: i32,
                out: *mut u16,
                capacity: i32,
            ) -> i32;
            fn GetACP() -> u32;
        }
        let cp = opts.codepage.unwrap_or_else(|| unsafe { GetACP() });
        let len = i32::try_from(bytes.len()).map_err(|_| "encoded input too large")?;
        let size =
            unsafe { MultiByteToWideChar(cp, 8, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
        if size == 0 {
            return Err("encoded filename cannot be decoded with selected codepage".into());
        }
        let mut out = vec![0u16; size as usize];
        let n = unsafe { MultiByteToWideChar(cp, 8, bytes.as_ptr(), len, out.as_mut_ptr(), size) };
        if n != size {
            return Err("filename decoding failed".into());
        }
        String::from_utf16(&out).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        std::str::from_utf8(bytes)
            .map(|s| s.to_owned())
            .map_err(|_| "only UTF8 encoded filenames are supported on this platform".into())
    }
}
fn text_decode(text: &str, xx: bool) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut lines = text.lines();
    let mut out = Vec::new();
    let mut total = 0u64;
    while let Some(line) = lines.next() {
        if !line.starts_with("begin ") {
            continue;
        }
        let mut parts = line.splitn(3, ' ');
        parts.next();
        let mode = parts.next().ok_or("missing encoded mode")?;
        if mode.len() != 3 || !mode.bytes().all(|b| matches!(b, b'0'..=b'7')) {
            return Err("invalid encoded file mode".into());
        }
        let name = parts.next().ok_or("missing encoded filename")?.to_string();
        engine::validate_name(&name)?;
        let mut data = Vec::new();
        let mut ended = false;
        let mut zero = false;
        for line in lines.by_ref() {
            if line == "end" {
                ended = true;
                break;
            }
            let b = line.as_bytes();
            if b.is_empty() {
                return Err("empty encoded data line".into());
            }
            let value = |c: u8| -> Result<u8, String> {
                if xx {
                    b"+-0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
                        .iter()
                        .position(|v| *v == c)
                        .map(|n| n as u8)
                        .ok_or("invalid XXencode character".into())
                } else if (32..=96).contains(&c) {
                    Ok((c.wrapping_sub(32)) & 63)
                } else {
                    Err("invalid UUencode character".into())
                }
            };
            let n = value(b[0])? as usize;
            if n > 45 {
                return Err("encoded line exceeds45 bytes".into());
            }
            if n == 0 {
                zero = true;
                continue;
            }
            if zero {
                return Err("data follows encoded terminator".into());
            }
            let encoded = n.div_ceil(3) * 4;
            if b.len() != encoded + 1 {
                return Err("encoded line length mismatch".into());
            }
            let mut row = Vec::new();
            for chunk in b[1..].chunks_exact(4) {
                let a = value(chunk[0])?;
                let c = value(chunk[1])?;
                let d = value(chunk[2])?;
                let e = value(chunk[3])?;
                row.extend_from_slice(&[(a << 2) | (c >> 4), (c << 4) | (d >> 2), (d << 6) | e]);
            }
            data.extend_from_slice(&row[..n]);
            total += n as u64;
            if total > LIMIT {
                return Err("decoded envelope exceeds512MiB".into());
            }
        }
        if !ended || !zero {
            return Err("encoded envelope is truncated".into());
        }
        out.push((name, data));
        if out.len() > 100000 {
            return Err("too many encoded entries".into());
        }
    }
    if out.is_empty() {
        Err("no encoded files".into())
    } else {
        Ok(out)
    }
}
struct Prepared {
    _temp: tempfile::TempDir,
    items: Vec<(Entry, PathBuf)>,
}
fn prepare(path: &str, opts: &EngineOptions) -> Result<Prepared, String> {
    check(opts)?;
    let kind = kind(path)?;
    let f = File::open(path).map_err(|e| e.to_string())?;
    let packed = f.metadata().map_err(|e| e.to_string())?.len();
    if packed > LIMIT {
        return Err("envelope input exceeds512MiB limit".into());
    }
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    if kind == "UU/XXencode" {
        let mut bytes = Vec::new();
        f.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("envelope input grew beyond limit".into());
        }
        let text = encoded_text(&bytes, opts)?;
        let rows = text_decode(&text, false).or_else(|_| text_decode(&text, true))?;
        let mut names = HashSet::new();
        for (n, data) in rows {
            check(opts)?;
            let key = engine::validate_name(&n)?.to_string_lossy().to_lowercase();
            if !names.insert(key) {
                return Err("duplicate encoded filenames".into());
            }
            let out = temp.path().join(format!("{}.bin", items.len()));
            fs::write(&out, &data).map_err(|e| e.to_string())?;
            items.push((entry(n, data.len() as u64, packed, kind), out));
        }
    } else {
        let out = temp.path().join("content.bin");
        let target = File::create(&out).map_err(|e| e.to_string())?;
        let mut writer = Checked {
            inner: target,
            opts: opts.clone(),
            written: 0,
        };
        if kind == "lzip" {
            let mut decoder = lzma_rust2::LzipReader::new(Checked {
                inner: f,
                opts: opts.clone(),
                written: 0,
            });
            std::io::copy(&mut decoder, &mut writer).map_err(|e| e.to_string())?;
        } else {
            let mut f = f;
            aes_preflight(&mut f)?;
            let password = opts
                .password
                .as_deref()
                .ok_or("AES Crypt password required")?;
            aescrypt_rs::decrypt_verified(
                Checked {
                    inner: f,
                    opts: opts.clone(),
                    written: 0,
                },
                &mut writer,
                password,
            )
            .map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
        items.push((entry(name(path, kind)?, writer.written, packed, kind), out));
    }
    let names: Vec<_> = items
        .iter()
        .map(|(e, _)| {
            engine::validate_name(&e.name)
                .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        })
        .collect::<Result<_, _>>()?;
    let files: HashSet<_> = names.iter().map(|s| s.as_str()).collect();
    for name in &names {
        for (i, _) in name.match_indices('/') {
            if files.contains(&name[..i]) {
                return Err("encoded file/directory conflict".into());
            }
        }
    }
    check(opts)?;
    Ok(Prepared { _temp: temp, items })
}
pub fn list(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    check(opts)?;
    if kind(path)? == "AES Crypt" && opts.password.is_none() {
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        aes_preflight(&mut f)?;
        let mut e = entry(
            name(path, "AES Crypt")?,
            0,
            f.metadata().map_err(|e| e.to_string())?.len(),
            "AES Crypt",
        );
        e.comment = "Original size is available after entering the password".into();
        return Ok(vec![e]);
    }
    Ok(prepare(path, opts)?
        .items
        .into_iter()
        .map(|(e, _)| e)
        .collect())
}
pub fn test(path: &str, opts: &EngineOptions) -> Result<(), String> {
    prepare(path, opts).map(|_| ())
}
pub fn preview(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    let p = prepare(path, opts)?;
    let (e, file) = p
        .items
        .iter()
        .find(|(e, _)| e.name == name)
        .ok_or("entry not found")?;
    if e.size > 32 * 1024 * 1024 {
        return Err("preview exceeds32MiB".into());
    }
    fs::read(file).map_err(|e| e.to_string())
}
pub fn extract(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    let p = prepare(path, opts)?;
    let chosen: Vec<_> = p
        .items
        .iter()
        .filter(|(e, _)| engine::selected(&e.name, opts))
        .collect();
    if !opts.selected.is_empty() && chosen.is_empty() {
        return Err("selected entries not found".into());
    }
    let dest = std::path::absolute(dest).map_err(|e| e.to_string())?;
    engine::no_links(&dest)?;
    for (e, _) in &chosen {
        let target = dest.join(engine::validate_name(&e.name)?);
        engine::no_links(&target)?;
        if opts.overwrite == "error" && target.exists() {
            return Err("destination exists".into());
        }
    }
    let mut count = 0;
    for (e, source) in chosen {
        check(opts)?;
        let Some(target) =
            engine::choose_target(dest.join(engine::validate_name(&e.name)?), &opts.overwrite)?
        else {
            continue;
        };
        let parent = target.parent().ok_or("missing output parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        engine::no_links(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        std::io::copy(
            &mut File::open(source).map_err(|e| e.to_string())?,
            &mut temp,
        )
        .map_err(|e| e.to_string())?;
        check(opts)?;
        engine::no_links(&target)?;
        if opts.overwrite == "overwrite" {
            temp.persist(target).map_err(|e| e.error.to_string())?;
        } else {
            temp.persist_noclobber(target)
                .map_err(|e| e.error.to_string())?;
        }
        count += 1;
    }
    Ok(count)
}
