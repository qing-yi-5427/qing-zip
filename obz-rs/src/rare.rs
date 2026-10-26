//! Isolated uncommon-format readers; paths are validated before publication.
use crate::engine::{self, EngineOptions, Entry};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
fn check(o: &EngineOptions) -> Result<(), String> {
    if let Some(c) = &o.control {
        c.check()?;
    }
    if !["error", "overwrite", "skip", "rename"].contains(&o.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    Ok(())
}
fn entry(name: String, size: u64, method: &str) -> Entry {
    Entry {
        name,
        size,
        packed_size: 0,
        is_dir: false,
        crc: String::new(),
        encrypted: false,
        modified: String::new(),
        method: method.into(),
        comment: String::new(),
        attributes: String::new(),
    }
}
pub fn supports(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "pma" | "zpaq" | "daa" | "isz"
    )
}
fn validate(entries: &[Entry]) -> Result<(), String> {
    let mut tree = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut total = 0u64;
    if entries.len() > 100000 {
        return Err("too many archive members".into());
    }
    for e in entries {
        let rel = engine::validate_name(&e.name)?;
        let key = rel.to_string_lossy().to_lowercase();
        if !seen.insert(key.clone()) {
            return Err("duplicate archive path".into());
        }
        if tree.insert(key, e.is_dir).is_some_and(|v| !v || !e.is_dir) {
            return Err("conflicting archive path".into());
        }
        for p in rel
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            if tree.insert(p.to_string_lossy().to_lowercase(), true) == Some(false) {
                return Err("archive file is a parent directory".into());
            }
        }
        total = total.checked_add(e.size).ok_or("size overflow")?;
        if e.size > MAX_FILE || total > MAX_TOTAL {
            return Err("uncommon archive size limit exceeded".into());
        }
    }
    Ok(())
}
fn runtime(name: &str) -> Result<PathBuf, String> {
    let rel = PathBuf::from("runtime").join(name);
    let mut roots = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(&rel)];
    if let Ok(exe) = std::env::current_exe() {
        for p in exe.ancestors().skip(1).take(4) {
            roots.insert(0, p.join(&rel));
        }
    }
    roots
        .into_iter()
        .find(|p| p.is_file())
        .ok_or(format!("bundled runtime {name} missing"))
}
fn run(program: &Path, args: &[String], o: &EngineOptions) -> Result<String, String> {
    check(o)?;
    let mut logs = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut errors = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(logs.try_clone().map_err(|e| e.to_string())?)
        .stderr(errors.try_clone().map_err(|e| e.to_string())?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let _job = match crate::legacy_worker::job::Job::attach(&child) {
        Ok(j) => j,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let started = Instant::now();
    let status = loop {
        if check(o).is_err()
            || started.elapsed() > Duration::from_secs(600)
            || logs.metadata().map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024
        {
            let _ = child.kill();
            let _ = child.wait();
            check(o)?;
            return Err("decoder time/output limit exceeded".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    };
    if !status.success() {
        errors.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut message = String::new();
        errors.take(4096).read_to_string(&mut message).ok();
        if let Some(p) = &o.password {
            message = message.replace(p, "[redacted]");
        }
        return Err(format!(
            "archive decoder failed ({:?}): {}",
            status.code(),
            message.trim()
        ));
    }
    logs.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut text = String::new();
    logs.take(32 * 1024 * 1024)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Ok(text)
}
fn zpaq_args(action: &str, path: &str, o: &EngineOptions) -> Result<Vec<String>, String> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let mut a = vec![
        action.into(),
        path.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned(),
        "-threads".into(),
        "1".into(),
    ];
    if let Some(p) = &o.password {
        if p.is_empty() || p.chars().any(|c| c.is_control()) {
            return Err("invalid password".into());
        }
        a.extend(["-key".into(), p.clone()]);
    }
    Ok(a)
}
fn zpaq_list(path: &str, o: &EngineOptions) -> Result<Vec<Entry>, String> {
    let text = run(&runtime("zpaq/obz-zpaq.exe")?, &zpaq_args("l", path, o)?, o)?;
    let mut result = Vec::new();
    for line in text.lines().filter(|l| l.starts_with("OBZ1 ")) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 5 {
            return Err("invalid ZPAQ directory record".into());
        }
        let size = fields[1].parse::<u64>().map_err(|_| "invalid ZPAQ size")?;
        let attr = fields[2]
            .parse::<u64>()
            .map_err(|_| "invalid ZPAQ attributes")?;
        if (attr & 255 == b'w' as u64 && (attr >> 8) & 0x400 != 0)
            || (attr & 255 == b'u' as u64 && ![0, 4, 8].contains(&((attr >> 20) & 15)))
        {
            return Err("ZPAQ links/special files rejected".into());
        }
        let h = fields[4];
        if h.len() % 2 != 0 {
            return Err("bad hex name".into());
        }
        let bytes = (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| "bad hex name"))
            .collect::<Result<Vec<_>, _>>()?;
        let name = String::from_utf8(bytes).map_err(|_| "non-UTF8 ZPAQ name")?;
        let mut e = entry(name, size, "ZPAQ");
        e.is_dir = e.name.ends_with('/');
        e.encrypted = o.password.is_some();
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(fields[3], "%Y%m%d%H%M%S") {
            e.modified = d.format("%Y-%m-%d %H:%M:%S").to_string();
        }
        result.push(e);
    }
    validate(&result)?;
    Ok(result)
}
fn pma(path: &str, stage: Option<&Path>, o: &EngineOptions) -> Result<Vec<Entry>, String> {
    check(o)?;
    if o.password.is_some() {
        return Err("PMA encryption is unsupported by format".into());
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = delharc::LhaDecodeReader::new(file).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    let mut total_size = 0u64;
    let mut seen = HashSet::new();
    loop {
        check(o)?;
        if reader.is_absent() {
            break;
        }
        let h = reader.header();
        if h.level != 0 || !matches!(&h.compression, b"-pm0-" | b"-pm1-" | b"-pm2-") {
            return Err("unsupported PMA header or compression".into());
        }
        let name = String::from_utf8(h.filename.to_vec())
            .map_err(|_| "PMA filename must be ASCII/UTF8")?;
        // CP/M stores a terminal dot for an empty 8.3 extension.
        let name = name.strip_suffix('.').unwrap_or(&name).to_owned();
        let mut e = entry(
            name,
            h.original_size,
            &String::from_utf8_lossy(&h.compression),
        );
        e.packed_size = h.compressed_size;
        e.crc = format!("{:04X}", h.file_crc);
        let key = engine::validate_name(&e.name)?
            .to_string_lossy()
            .to_lowercase();
        total_size = total_size
            .checked_add(e.size)
            .ok_or("PMA total size overflow")?;
        if !seen.insert(key)
            || entries.len() >= 100000
            || e.size > MAX_FILE
            || total_size > MAX_TOTAL
        {
            return Err("PMA duplicate or size/count limit exceeded".into());
        }
        entries.push(e);
        let mut out = if let Some(root) = stage {
            let dest = root.join(engine::validate_name(&entries.last().unwrap().name)?);
            fs::create_dir_all(dest.parent().ok_or("bad PMA path")?).map_err(|e| e.to_string())?;
            Some(File::create(dest).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let mut total = 0u64;
        let mut buf = vec![0u8; 65536];
        loop {
            check(o)?;
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > entries.last().unwrap().size {
                return Err("PMA output exceeds declared size".into());
            }
            if let Some(out) = &mut out {
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        }
        if total != entries.last().unwrap().size {
            return Err("PMA size mismatch".into());
        }
        reader.crc_check().map_err(|e| e.to_string())?;
        if !reader.next_file().map_err(|e| e.to_string())? {
            break;
        }
    }
    validate(&entries)?;
    Ok(entries)
}
pub fn list(path: &str, o: &EngineOptions) -> Result<Vec<Entry>, String> {
    if is_disc(path) {
        let (_temp, iso) = disc_iso(path, o)?;
        return engine::list(&iso, o);
    }

    check(o)?;
    if path.to_ascii_lowercase().ends_with(".pma") {
        pma(path, None, o)
    } else if path.to_ascii_lowercase().ends_with(".zpaq") {
        zpaq_list(path, o)
    } else {
        Err("unsupported uncommon format".into())
    }
}
fn materialize(path: &str, o: &EngineOptions) -> Result<(tempfile::TempDir, Vec<Entry>), String> {
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    let entries = if path.to_ascii_lowercase().ends_with(".pma") {
        pma(path, Some(stage.path()), o)?
    } else {
        let entries = zpaq_list(path, o)?;
        let mut args = zpaq_args("x", path, o)?;
        args.extend([
            "-to".into(),
            format!("{}/", stage.path().display()),
            "-noattributes".into(),
        ]);
        run(&runtime("zpaq/obz-zpaq.exe")?, &args, o)?;
        entries
    };
    let mut allowed = HashSet::new();
    for e in &entries {
        let rel = engine::validate_name(&e.name)?;
        for p in rel.ancestors().filter(|p| !p.as_os_str().is_empty()) {
            allowed.insert(p.to_string_lossy().to_lowercase());
        }
        let p = stage.path().join(rel);
        engine::no_links(&p)?;
        let m = fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
        if m.is_dir() != e.is_dir || (!e.is_dir && (!m.is_file() || m.len() != e.size)) {
            return Err("decoded manifest mismatch".into());
        }
    }
    let mut stack = vec![stage.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        check(o)?;
        for item in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let p = item.map_err(|e| e.to_string())?.path();
            engine::no_links(&p)?;
            if !allowed.contains(
                &p.strip_prefix(stage.path())
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .to_lowercase(),
            ) {
                return Err("unexpected decoded output".into());
            }
            if p.is_dir() {
                stack.push(p);
            }
        }
    }
    Ok((stage, entries))
}
pub fn test(path: &str, o: &EngineOptions) -> Result<(), String> {
    if is_disc(path) {
        let (_temp, iso) = disc_iso(path, o)?;
        return engine::test(&iso, o);
    }

    materialize(path, o).map(|_| ())
}
pub fn extract(path: &str, dest: &str, o: &EngineOptions) -> Result<usize, String> {
    if is_disc(path) {
        let (_temp, iso) = disc_iso(path, o)?;
        return engine::extract(&iso, dest, o);
    }

    let (stage, entries) = materialize(path, o)?;
    let dest = Path::new(dest);
    engine::no_links(dest)?;
    let entries: Vec<_> = entries
        .iter()
        .filter(|e| engine::selected(&e.name, o))
        .collect();
    if !o.selected.is_empty() && entries.is_empty() {
        return Err("selected members not found".into());
    }
    for e in &entries {
        let p = dest.join(engine::validate_name(&e.name)?);
        engine::no_links(&p)?;
        if p.exists() && (!e.is_dir && o.overwrite == "error" || p.is_dir() != e.is_dir) {
            return Err("destination exists or conflicts".into());
        }
    }
    let mut count = 0;
    for e in &entries {
        check(o)?;
        let rel = engine::validate_name(&e.name)?;
        let p = dest.join(&rel);
        if e.is_dir {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
            continue;
        }
        let Some(p) = engine::choose_target(p, &o.overwrite)? else {
            continue;
        };
        let parent = p.parent().ok_or("missing parent")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        engine::no_links(parent)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        let mut input = File::open(stage.path().join(rel)).map_err(|e| e.to_string())?;
        let mut buf = vec![0; 1024 * 1024];
        loop {
            check(o)?;
            let n = input.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            tmp.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        }
        check(o)?;
        engine::no_links(&p)?;
        if o.overwrite == "overwrite" {
            tmp.persist(&p).map_err(|e| e.error.to_string())?;
        } else {
            tmp.persist_noclobber(&p).map_err(|e| e.error.to_string())?;
        }
        engine::restore_metadata(&p, e)?;
        count += 1;
    }
    Ok(count)
}
pub fn preview(path: &str, name: &str, o: &EngineOptions) -> Result<Vec<u8>, String> {
    if is_disc(path) {
        let (_temp, iso) = disc_iso(path, o)?;
        return engine::preview(&iso, name, o);
    }

    let (stage, entries) = materialize(path, o)?;
    let e = entries
        .iter()
        .find(|e| e.name == name && !e.is_dir)
        .ok_or("member not found")?;
    if e.size > 32 * 1024 * 1024 {
        return Err("preview exceeds 32 MiB".into());
    }
    fs::read(stage.path().join(engine::validate_name(name)?)).map_err(|e| e.to_string())
}

fn is_disc(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".daa") || p.ends_with(".isz")
}
fn disc_iso(path: &str, o: &EngineOptions) -> Result<(tempfile::TempDir, String), String> {
    check(o)?;
    if o.password.is_some() {
        return Err("encrypted DAA/ISZ decoding is not yet supported".into());
    }
    let stage = tempfile::tempdir().map_err(|e| e.to_string())?;
    let iso = stage.path().join("image.iso");
    let input = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let args = vec![
        "-I".into(),
        runtime("python/disc_worker.py")?
            .to_string_lossy()
            .into_owned(),
        input.to_string_lossy().into_owned(),
        iso.to_string_lossy().into_owned(),
    ];
    run(&runtime("python/python.exe")?, &args, o)?;
    if fs::metadata(&iso).map_err(|e| e.to_string())?.len() > MAX_TOTAL {
        return Err("decoded disc too large".into());
    }
    Ok((stage, iso.to_string_lossy().into_owned()))
}
