//! Multi-format operations through the redistributable 7-Zip engine.
//! No shell is used. Extraction stages preflighted archives before publishing files.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

// Opt-in diagnostic timing; no paths, passwords or archive names are logged.
struct PhaseTimer(&'static str, Option<std::time::Instant>);
impl PhaseTimer {
    fn new(label: &'static str) -> Self {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        Self(label, (*ENABLED.get_or_init(||std::env::var_os("OPEN_ARCHIVE_PROFILE").is_some_and(|v|v=="1"))).then(std::time::Instant::now))
    }
}
impl Drop for PhaseTimer {
    fn drop(&mut self) { if let Some(start)=self.1 {eprintln!("OA_PHASE {} {:.6}",self.0,start.elapsed().as_secs_f64());} }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineOptions {
    pub password: Option<String>,
    pub codepage: Option<u32>,
    pub threads: Option<u32>,
    pub level: Option<u8>,
    pub method: Option<String>,
    pub encryption: Option<String>,
    pub volume_bytes: Option<u64>,
    pub selected: Vec<String>,
    /// error (default), overwrite, skip, rename.
    pub overwrite: String,
    #[serde(skip)]
    pub control: Option<JobControl>,
}
impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            password: None,
            codepage: None,
            threads: None,
            level: None,
            method: None,
            encryption: None,
            volume_bytes: None,
            selected: Vec::new(),
            overwrite: "error".into(),
            control: None,
        }
    }
}
#[derive(Clone, Default, Serialize)]
pub struct Progress {
    pub phase: String,
    pub percent: Option<u8>,
    pub message: String,
}
#[derive(Clone, Default)]
pub struct JobControl {
    pub cancelled: Arc<AtomicBool>,
    pub progress: Arc<Mutex<Progress>>,
}
impl JobControl {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn snapshot(&self) -> Progress {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err("operation cancelled".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn set(&self, phase: &str, percent: Option<u8>, message: &str) {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = Progress {
            phase: phase.into(),
            percent,
            message: message.into(),
        };
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub packed_size: u64,
    pub is_dir: bool,
    pub crc: String,
    pub encrypted: bool,
    pub modified: String,
    pub method: String,
    pub comment: String,
    pub attributes: String,
}
#[derive(Serialize)]
pub struct Capabilities {
    pub engine: String,
    pub available: bool,
    pub read_formats: Vec<String>,
    pub write_formats: Vec<String>,
    pub features: Vec<String>,
    pub limitations: Vec<String>,
}

pub fn executable() -> Result<PathBuf, String> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("runtime/7zip/7z.exe"));
        }
    }
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/7zip/7z.exe"));
    if let Some(p) = std::env::var_os("ProgramFiles") {
        paths.push(PathBuf::from(p).join("7-Zip/7z.exe"));
    }
    if cfg!(windows) {
        paths.push(PathBuf::from("C:/Program Files/7-Zip/7z.exe"));
    }
    paths
        .into_iter()
        .find(|p| p.is_file())
        .ok_or("7-Zip engine not found; install 7-Zip or restore runtime/7zip".into())
}
pub fn capabilities() -> Capabilities {
    Capabilities { engine:executable().map(|p|p.display().to_string()).unwrap_or_default(), available:executable().is_ok(),
      read_formats:["7z","zip","zipx (supported methods)","rar","rar5","tar","gzip","bzip2","xz","zstd","iso","udf","cab","wim","msi","nsis","arj","lzh","deb","rpm","dmg","vhd","vhdx","sfx","split"].iter().map(|s|s.to_string()).collect(),
      write_formats:["zip","7z","tar","gzip","bzip2","xz","wim","sfx","tgz","tbz2","txz","zipx"].iter().map(|s|s.to_string()).collect(),
      features:["AES-256 ZIP/7z","encrypted 7z headers","split volumes","ZIP64","multithreaded compression","selected extraction","archive integrity test","add/delete/rename","bounded preview","cancel/progress","codepage override","overwrite policies"].iter().map(|s|s.to_string()).collect(),
      limitations:["RAR/LZH creation, ALZ/EGG/ACE and arbitrary ZIPX codecs are not supported by this engine","Extraction verifies the complete archive before publishing selected files; eligible small ZIPs and simple solid 7z use bounded memory; full ZIP/7z extraction to a new directory under a private existing parent can stage on the target volume and commit the verified tree atomically; other cases require temporary disk staging","Archive symbolic links, hard links and alternate streams are rejected","Passwords passed to 7-Zip are visible to same-user process inspection; never included in application logs"].iter().map(|s|s.to_string()).collect() }
}
fn absolute(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    Ok(if p.is_absolute() {
        p
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(p)
    })
}
fn control(opts: &EngineOptions) -> JobControl {
    opts.control.clone().unwrap_or_default()
}
fn base(command: &str, opts: &EngineOptions) -> Result<Vec<String>, String> {
    if opts.level.is_some_and(|v| v > 9) || opts.threads == Some(0) || opts.volume_bytes == Some(0)
    {
        return Err("invalid compression settings".into());
    }
    if !["error", "overwrite", "skip", "rename"].contains(&opts.overwrite.as_str()) {
        return Err("invalid overwrite policy".into());
    }
    let mut a = vec![
        command.into(),
        "-y".into(),
        "-sccUTF-8".into(),
        "-spd".into(),
        "-bsp2".into(),
    ];
    if let Some(p) = &opts.password {
        if p.contains('\0') || p.contains('\n') || p.contains('\r') {
            return Err("password contains invalid characters".into());
        }
        if p.is_empty() {
            return Err("password must not be empty".into());
        }
        a.push(format!("-p{p}"));
    }
    if let Some(cp) = opts.codepage {
        a.push(format!("-mcp={cp}"));
    }
    Ok(a)
}
fn scrub(text: &str, opts: &EngineOptions) -> String {
    let mut s = text.to_owned();
    if let Some(p) = &opts.password {
        s = s.replace(p, "[redacted]");
    }
    s
}
fn run(
    args: &[String],
    cwd: Option<&Path>,
    opts: &EngineOptions,
    output: Option<File>,
) -> Result<Vec<u8>, String> {
    run_program(executable()?, args, cwd, opts, output)
}
/// Wait for exit readiness without imposing a fixed delay on a completed process.
/// The timeout still bounds progress updates and cancellation checks.
pub fn wait_for_process(child: &std::process::Child, timeout: Duration) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "Kernel32")]
        extern "system" { fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32; }
        // Child owns the valid process handle for the duration of this wait.
        let result = unsafe { WaitForSingleObject(child.as_raw_handle(), timeout.as_millis().min(u32::MAX as u128-1) as u32) };
        match result { 0 | 258 => Ok(()), _ => Err(std::io::Error::last_os_error()) }
    }
    #[cfg(not(windows))]
    { let _ = child; thread::sleep(timeout); Ok(()) }
}

fn run_program(
    program: PathBuf,
    args: &[String],
    cwd: Option<&Path>,
    opts: &EngineOptions,
    output: Option<File>,
) -> Result<Vec<u8>, String> {
    let _timer = PhaseTimer::new("engine-process");
    let ctl = control(opts);
    ctl.check()?;
    let logs = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let errors = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let capture = output.is_none();
    let stdout = match output {
        Some(f) => f,
        None => logs.reopen().map_err(|e| e.to_string())?,
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(errors.reopen().map_err(|e| e.to_string())?));
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot launch archive engine: {e}"))?;
    let mut progress_reader = File::open(errors.path()).ok();
    let status = loop {
        if ctl.check().is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err("operation cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {
                use std::io::{Seek, SeekFrom};
                if let Some(progress) = progress_reader.as_mut() {
                    if let Ok(length) = progress.metadata().map(|m| m.len()) {
                        let _ = progress.seek(SeekFrom::Start(length.saturating_sub(256)));
                        let mut tail = String::new();
                        let _ = progress.read_to_string(&mut tail);
                        if let Some(pos) = tail.rfind('%') {
                            let digits = tail[..pos]
                                .chars()
                                .rev()
                                .take_while(|c| c.is_ascii_digit())
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect::<String>();
                            if let Ok(n) = digits.parse::<u8>() {
                                let mut state =
                                    ctl.progress.lock().unwrap_or_else(|e| e.into_inner());
                                if state.phase != "extracting" {
                                    state.percent = Some(n.min(100));
                                }
                            }
                        }
                    }
                }
                if let Err(e) = wait_for_process(&child, Duration::from_millis(40)) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(e.to_string());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    use std::io::{Seek, SeekFrom};
    let mut logs = logs.reopen().map_err(|e| e.to_string())?;
    logs.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    logs.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("engine listing exceeds supported limit".into());
    }
    if !status.success() {
        let mut errors = errors.reopen().map_err(|e| e.to_string())?;
        errors.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut err = String::new();
        errors.take(8192).read_to_string(&mut err).ok();
        return Err(scrub(
            &format!(
                "archive engine failed ({}): {} {}",
                status.code().unwrap_or(-1),
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(4096)
                    .collect::<String>(),
                err
            ),
            opts,
        ));
    }
    if capture {
        Ok(bytes)
    } else {
        Ok(Vec::new())
    }
}
pub(crate) fn validate_name(name: &str) -> Result<PathBuf, String> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') {
        return Err("absolute archive path rejected".into());
    }
    let normalized = name
        .split('/')
        .filter(|part| *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    crate::extract::relative_path(&normalized)
}
fn parse_listing(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| "engine returned invalid UTF-8")?
        .replace("\r\n", "\n");
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for block in text.split("\n\n").filter(|s| !s.trim().is_empty()) {
        let mut props = BTreeMap::new();
        for line in block.lines() {
            if let Some((k, v)) = line.split_once(" = ") {
                if props.insert(k, v).is_some() {
                    return Err("ambiguous archive entry metadata".into());
                }
            } else if !line.trim().is_empty() {
                return Err("unsupported archive filename or metadata".into());
            }
        }
        let Some(name) = props.get("Path") else {
            continue;
        };
        if ["Symbolic Link", "Hard Link", "Alternate Stream"]
            .iter()
            .any(|k| props.get(k).is_some_and(|s| !s.is_empty() && *s != "-"))
        {
            return Err("archive links or alternate streams are unsupported".into());
        }
        let attrs = props.get("Attributes").copied().unwrap_or("");
        if attrs.starts_with('l') || attrs.contains(" lrwx") {
            return Err("archive symbolic link unsupported".into());
        }
        let is_dir = props.get("Folder") == Some(&"+")
            || attrs.starts_with('D')
            || attrs.starts_with('d')
            || name.ends_with('/');
        if matches!(*name, "." | "./") {
            if !is_dir {
                return Err("archive root must be a directory".into());
            }
            continue;
        }
        if !seen.insert(path_key(&validate_name(name)?)) {
            return Err("duplicate or case-colliding archive names".into());
        }
        let size = props.get("Size").and_then(|s| s.parse().ok()).unwrap_or(0);
        result.push(Entry {
            name: (*name).into(),
            size,
            packed_size: props
                .get("Packed Size")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            is_dir,
            crc: props.get("CRC").unwrap_or(&"").to_string(),
            encrypted: props.get("Encrypted") == Some(&"+"),
            modified: props.get("Modified").unwrap_or(&"").to_string(),
            method: props.get("Method").unwrap_or(&"").to_string(),
            comment: props.get("Comment").unwrap_or(&"").to_string(),
            attributes: attrs.to_string(),
        });
    }
    Ok(result)
}
fn list_raw(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    let _timer = PhaseTimer::new("list-total");
    let ctl = control(opts);
    ctl.set("listing", None, "Reading archive directory");
    let mut args = base("l", opts)?;
    args.extend([
        "-slt".into(),
        "-ba".into(),
        "--".into(),
        absolute(path)?.display().to_string(),
    ]);
    let listing = run(&args, None, opts, None)?;
    let mut out = parse_listing(&listing)?;
    // Bzip2 and some raw streams have no stored name or uncompressed size.
    // 7-Zip emits an anonymous Size block; measure it rather than report empty.
    if out.is_empty() && String::from_utf8_lossy(&listing).starts_with("Size =") {
        let temp = tempfile::tempfile().map_err(|e| e.to_string())?;
        let mut decode = base("x", opts)?;
        decode.extend([
            "-so".into(),
            "--".into(),
            absolute(path)?.display().to_string(),
        ]);
        run(
            &decode,
            None,
            opts,
            Some(temp.try_clone().map_err(|e| e.to_string())?),
        )?;
        let name = Path::new(path)
            .file_stem()
            .ok_or("stream has no name")?
            .to_string_lossy()
            .into_owned();
        validate_name(&name)?;
        out.push(Entry {
            name,
            size: temp.metadata().map_err(|e| e.to_string())?.len(),
            packed_size: fs::metadata(path).map_err(|e| e.to_string())?.len(),
            is_dir: false,
            crc: String::new(),
            encrypted: false,
            modified: String::new(),
            method: "raw stream".into(),
            comment: String::new(),
            attributes: String::new(),
        });
    }
    ctl.set("complete", Some(100), "Archive directory ready");
    Ok(out)
}
pub fn list_with_control(
    path: &str,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<Vec<Entry>, String> {
    let mut o = opts.clone();
    o.control = Some(ctl.clone());
    list(path, &o)
}
pub(crate) fn selected(name: &str, opts: &EngineOptions) -> bool {
    opts.selected.is_empty()
        || opts.selected.iter().any(|s| {
            name.replace('\\', "/") == s.replace('\\', "/")
                || name
                    .replace('\\', "/")
                    .starts_with(&(s.replace('\\', "/").trim_end_matches('/').to_owned() + "/"))
        })
}
// Attribute queries avoid opening and closing a handle for every ancestor. Keep
// ambiguous namespaces and DOS normalization cases on std's metadata path.
#[cfg(windows)]
fn attribute_query_path(path: &Path) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};
    let verbatim = match path.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(_) if path.is_absolute() => false,
            Prefix::VerbatimDisk(_) if path.is_absolute() => true,
            _ => return None,
        },
        _ => return None,
    };
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) || (!verbatim && wide.len() + 1 >= 248) {
        return None;
    }
    // Do not invent a verbatim prefix or normalize the caller's spelling.
    // Conservative component screening also excludes device names and streams.
    let start = if verbatim { 7 } else { 3 };
    if wide.len() < start || wide[start - 1] != b'\\' as u16 {
        return None;
    }
    for part in wide[start..].split(|c| *c == b'\\' as u16) {
        if part.is_empty() {
            continue;
        }
        if part.iter().any(|c| matches!(*c, 47 | 58 | 42 | 63))
            || matches!(part.last(), Some(32 | 46))
        {
            return None;
        }
        let stem = part.split(|c| *c == 46).next().unwrap();
        let name = String::from_utf16_lossy(stem).to_ascii_uppercase();
        if matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((name.starts_with("COM") || name.starts_with("LPT"))
                && name.len() == 4
                && name.as_bytes()[3].is_ascii_digit())
        {
            return None;
        }
    }
    wide.push(0);
    Some(wide)
}

#[cfg(windows)]
fn fresh_path_attributes(path: &Path) -> Option<u32> {
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileAttributesW(path: *const u16) -> u32;
    }
    let wide = attribute_query_path(path)?;
    // SAFETY: wide is NUL-terminated with no interior NUL and remains alive.
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attributes == u32::MAX {
        // Let std retain its long-path, missing-file, sharing and error handling.
        None
    } else {
        Some(attributes)
    }
}

#[cfg(windows)]
fn fast_reparse_attribute(path: &Path) -> Option<bool> {
    fresh_path_attributes(path).map(|attributes| attributes & 0x400 != 0)
}

pub(crate) fn no_links(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        #[cfg(windows)]
        if let Some(link) = fast_reparse_attribute(p) {
            if link {
                return Err(format!("linked output path rejected: {}", p.display()));
            }
            continue;
        }
        match fs::symlink_metadata(p) {
            Ok(m) => {
                let mut link = m.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    link |= m.file_attributes() & 0x400 != 0;
                }
                if link {
                    return Err(format!("linked output path rejected: {}", p.display()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
#[cfg(windows)]
fn directory_identity(file: &File) -> Result<(u32, u64), String> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut u32) -> i32;
    }
    // BY_HANDLE_FILE_INFORMATION consists of 13 DWORDs, including three FILETIMEs.
    let mut info = [0u32; 13];
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok((info[7], ((info[11] as u64) << 32) | info[12] as u64))
}
#[cfg(windows)]
fn directory_attributes(file: &File) -> Result<u32, String> {
    use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *mut u32,
            size: u32,
        ) -> i32;
    }
    let mut info = [0u32; 2]; // FILE_ATTRIBUTE_TAG_INFO
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), 9, info.as_mut_ptr(), 8) } != 0 {
        Ok(info[0])
    } else {
        file.metadata()
            .map(|m| m.file_attributes())
            .map_err(|e| e.to_string())
    }
}
// Only operation-owned directory handles deny write/delete sharing: their checked reparse
// identity cannot change. Only direct parents of child-file publication are
// weakened to WRITE sharing; those handles re-query attributes on every check.
// Ancestors are checked root-first and file/missing leaves are never cached.
#[derive(Default)]
struct CheckedDirectories {
    #[cfg(windows)]
    held: std::collections::HashMap<PathBuf, (File, bool)>,
    #[cfg(windows)]
    publication_parents: HashSet<PathBuf>,
    #[cfg(windows)]
    owned_roots: Vec<PathBuf>,
    #[cfg(windows)]
    owned_dirs: HashSet<PathBuf>,
}
impl CheckedDirectories {
    fn owned_root(root: &Path) -> Self {
        let mut guard = Self::default();
        #[cfg(windows)]
        guard.owned_roots.push(root.to_path_buf());
        #[cfg(not(windows))]
        let _ = root;
        guard
    }
    fn create_dirs(&mut self, path: &Path) -> Result<(), String> {
        for component in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            self.check(component)?;
            if self.pinned(component) {
                continue;
            }
            match fs::create_dir(component) {
                Ok(()) => {
                    #[cfg(windows)]
                    self.owned_dirs.insert(component.to_path_buf());
                    self.check(component)?;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists && component.is_dir() =>
                {
                    self.check(component)?
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }
    fn pinned(&self, path: &Path) -> bool {
        #[cfg(windows)]
        {
            self.held.contains_key(path)
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            false
        }
    }
    fn allow_publication(&mut self, parent: &Path) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            self.publication_parents.insert(parent.to_path_buf());
            let parent_key = path_key(parent);
            let aliases = self
                .held
                .iter()
                .filter(|(path, (_, weak))| !weak && path_key(path) == parent_key)
                .map(|(path, (file, _))| Ok((path.clone(), directory_identity(file)?)))
                .collect::<Result<Vec<_>, String>>()?;
            if !aliases.is_empty() {
                // Open while all strong aliases are still held. Compare identities
                // before replacing them; case folding alone never proves identity.
                match fs::OpenOptions::new()
                    .access_mode(0x81)
                    .share_mode(3)
                    .custom_flags(0x02200000)
                    .open(parent)
                {
                    Ok(handle) => {
                        let meta = handle.metadata().map_err(|e| e.to_string())?;
                        let identity = directory_identity(&handle)?;
                        if meta.file_attributes() & 0x400 != 0 || !meta.is_dir() {
                            return Err(format!(
                                "linked or changed output directory rejected: {}",
                                parent.display()
                            ));
                        }
                        if let Some((old, _)) = self.held.get(parent) {
                            if directory_identity(old)? != identity {
                                return Err("output directory identity changed".into());
                            }
                        }
                        for (alias, old_identity) in aliases {
                            if old_identity == identity {
                                self.publication_parents.insert(alias.clone());
                                self.held.insert(
                                    alias,
                                    (handle.try_clone().map_err(|e| e.to_string())?, true),
                                );
                            }
                        }
                        self.held.insert(parent.to_path_buf(), (handle, true));
                    }
                    Err(_) => {
                        // No incomplete alias transition: retain safety through
                        // fresh path checks; publication may correctly report the
                        // original sharing/permission failure.
                        self.held.remove(parent);
                        no_links(parent)?;
                    }
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = parent;
        }
        Ok(())
    }
    // The parallel publisher prepares every directory first, then shares this
    // immutable handle table. Each caller still queries current weak attributes;
    // there is no metadata cache and no mutex around filesystem operations.
    fn check_prepared(&self, path: &Path) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if attribute_query_path(path).is_some() {
                for part in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
                    if let Some((handle, weak)) = self.held.get(part) {
                        if *weak {
                            let attributes = directory_attributes(handle)?;
                            if attributes & 0x400 != 0 || attributes & 0x10 == 0 {
                                return Err(format!(
                                    "linked or changed output directory rejected: {}",
                                    part.display()
                                ));
                            }
                        }
                    } else {
                        match fs::symlink_metadata(part) {
                            Ok(meta) if meta.file_attributes() & 0x400 != 0 => {
                                return Err(format!(
                                    "linked output path rejected: {}",
                                    part.display()
                                ))
                            }
                            Ok(_) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                }
                return Ok(());
            }
        }
        no_links(path)
    }
    fn check(&mut self, path: &Path) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            // Preserve std's handling of uncommon namespaces and ambiguous paths.
            if attribute_query_path(path).is_some() {
                for part in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
                    if let Some((handle, publication_parent)) = self.held.get(part) {
                        if *publication_parent {
                            let attributes = directory_attributes(handle)?;
                            if attributes & 0x400 != 0 || attributes & 0x10 == 0 {
                                return Err(format!(
                                    "linked or changed output directory rejected: {}",
                                    part.display()
                                ));
                            }
                        }
                        continue;
                    }
                    match fs::symlink_metadata(part) {
                        Ok(meta) => {
                            if meta.file_attributes() & 0x400 != 0 {
                                return Err(format!(
                                    "linked output path rejected: {}",
                                    part.display()
                                ));
                            }
                            if meta.is_dir() && self.held.len() < 4096 {
                                let owned = self.owned_dirs.contains(part)
                                    || self.owned_roots.iter().any(|root| part.starts_with(root));
                                // Shared ancestors and pre-existing output directories
                                // must not block unrelated applications' child renames.
                                let publication_parent =
                                    !owned || self.publication_parents.contains(part);
                                let lock = fs::OpenOptions::new()
                                    .access_mode(0x81) // FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES
                                    .share_mode(if publication_parent { 3 } else { 1 })
                                    .custom_flags(0x02200000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
                                    .open(part);
                                match lock {
                                    Ok(lock) => {
                                        let pinned = lock.metadata().map_err(|e| e.to_string())?;
                                        if pinned.file_attributes() & 0x400 != 0 || !pinned.is_dir()
                                        {
                                            return Err(format!(
                                                "linked or changed output directory rejected: {}",
                                                part.display()
                                            ));
                                        }
                                        self.held
                                            .insert(part.to_path_buf(), (lock, publication_parent));
                                    }
                                    // Existing applications may hold incompatible sharing handles.
                                    // Retain the original checks instead of weakening access.
                                    Err(_) => return no_links(path),
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.to_string()),
                    }
                }
                return Ok(());
            }
        }
        no_links(path)
    }
}

pub(crate) fn choose_target(path: PathBuf, policy: &str) -> Result<Option<PathBuf>, String> {
    choose_target_checked(path, policy, &mut no_links)
}
fn choose_target_checked(
    path: PathBuf,
    policy: &str,
    check: &mut impl FnMut(&Path) -> Result<(), String>,
) -> Result<Option<PathBuf>, String> {
    check(&path)?;
    if !path.exists() {
        return Ok(Some(path));
    }
    match policy {
        "skip" => Ok(None),
        "overwrite" => {
            if path.is_dir() {
                Err("cannot overwrite directory with file".into())
            } else {
                Ok(Some(path))
            }
        }
        "rename" => {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let ext = path
                .extension()
                .map(|s| format!(".{}", s.to_string_lossy()))
                .unwrap_or_default();
            for i in 1..10000 {
                let p = path.with_file_name(format!("{stem} ({i}){ext}"));
                if !p.exists() {
                    check(&p)?;
                    return Ok(Some(p));
                }
            }
            Err("no free output name".into())
        }
        _ => Err(format!("destination exists: {}", path.display())),
    }
}
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

// Include implicit directories and reject file/directory aliases before allowing
// the decoder to create anything, even when the conflicting entry is unselected.
fn expected_tree(entries: &[Entry]) -> Result<BTreeMap<String, (bool, u64)>, String> {
    let mut tree = BTreeMap::new();
    for e in entries {
        let rel = validate_name(&e.name)?;
        let key = path_key(&rel);
        if let Some((dir, _)) = tree.insert(key, (e.is_dir, e.size)) {
            if !dir || !e.is_dir {
                return Err("conflicting archive paths".into());
            }
        }
        for parent in rel
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
        {
            if tree
                .insert(path_key(parent), (true, 0))
                .is_some_and(|(dir, _)| !dir)
            {
                return Err("archive file is also a directory".into());
            }
        }
    }
    Ok(tree)
}

#[cfg(test)]
fn verify_stage(
    root: &Path,
    expected: &BTreeMap<String, (bool, u64)>,
    ctl: &JobControl,
) -> Result<(), String> {
    let mut guards = CheckedDirectories::default();
    verify_stage_checked(root, expected, ctl, &mut guards)
}
fn verify_stage_checked(
    root: &Path,
    expected: &BTreeMap<String, (bool, u64)>,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
) -> Result<(), String> {
    verify_stage_with_metadata(root, expected, ctl, guards, None)
}
fn checked_staged_metadata(
    path: &Path,
    guards: &mut CheckedDirectories,
) -> Result<fs::Metadata, String> {
    guards.check(path.parent().ok_or("missing staged parent")?)?;
    // Always query the current leaf; DirEntry metadata may be an older Windows
    // enumeration snapshot and must not be used as reparse/size evidence.
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let mut linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        linked |= meta.file_attributes() & 0x400 != 0;
    }
    if linked {
        return Err("linked decoded path rejected".into());
    }
    Ok(meta)
}
fn verify_stage_with_metadata(
    root: &Path,
    expected: &BTreeMap<String, (bool, u64)>,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
    entries: Option<&[Entry]>,
) -> Result<(), String> {
    let _timer = PhaseTimer::new("verify-and-metadata");
    let mut files = std::collections::HashMap::new();
    if let Some(entries) = entries {
        for entry in entries.iter().filter(|e| !e.is_dir) {
            files.insert(path_key(&validate_name(&entry.name)?), entry);
        }
    }
    let mut pending = vec![root.to_path_buf()];
    let mut found = HashSet::new();
    while let Some(dir) = pending.pop() {
        ctl.check()?;
        guards.check(&dir)?;
        for item in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            ctl.check()?;
            let path = item.map_err(|e| e.to_string())?.path();
            let meta = checked_staged_metadata(&path, guards)?;
            let key = path_key(path.strip_prefix(root).map_err(|e| e.to_string())?);
            let &(is_dir, size) = expected
                .get(&key)
                .ok_or_else(|| format!("unexpected decoded path {key}"))?;
            if !found.insert(key.clone())
                || meta.is_dir() != is_dir
                || (!is_dir && (!meta.is_file() || meta.len() != size))
            {
                return Err("decoded tree does not match archive directory".into());
            }
            if is_dir {
                pending.push(path);
            } else if let Some(entry) = files.get(&key) {
                normalize_staged_entry(&path, entry, &meta, ctl, guards)?;
            }
        }
    }
    if found.len() != expected.len() {
        return Err("archive entries missing after decoding".into());
    }
    // Keep original reverse-listing semantics, including duplicate directory
    // metadata and implicit directories without an archive entry.
    if let Some(entries) = entries {
        for entry in entries.iter().rev().filter(|e| e.is_dir) {
            ctl.check()?;
            let path = root.join(validate_name(&entry.name)?);
            guards.check(&path)?;
            let meta = checked_staged_metadata(&path, guards)?;
            normalize_staged_entry(&path, entry, &meta, ctl, guards)?;
        }
    }
    Ok(())
}

const MEMORY_ZIP_LIMIT: u64 = 64 * 1024 * 1024;
const MEMORY_ZIP_FILE_LIMIT: u64 = 8 * 1024 * 1024;
const MEMORY_ZIP_ENTRY_LIMIT: usize = 16_384;

// Hold this source across the independent listing and decoding. Failed locking
// or an ineligible container keeps the original decoder path.
fn memory_archive_source(path: &str, opts: &EngineOptions) -> Option<File> {
    if opts.password.is_some()
        || opts.codepage.is_some()
        || !["zip", "7z"].iter().any(|ext| {
            Path::new(path)
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case(ext))
        })
    {
        return None;
    }
    no_links(&absolute(path).ok()?).ok()?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000); // no WRITE/DELETE; open reparse itself
    }
    let file = options.open(path).ok()?;
    let meta = file.metadata().ok()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return None;
        }
    }
    if !meta.is_file() || meta.len() > MEMORY_ZIP_LIMIT {
        return None;
    }
    Some(file)
}
fn zip_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn zip_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn ordinary_zip_extra(mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        if bytes.len() < 4 {
            return false;
        }
        let id = zip_u16(bytes, 0);
        let len = zip_u16(bytes, 2) as usize;
        if bytes.len() < len + 4 {
            return false;
        }
        // Only known timestamp/uid metadata. ZIP64, Unicode path overrides,
        // links, AES and all unknown extensions stay with the original engine.
        let payload = &bytes[4..len + 4];
        let known = match id {
            // Reserved DWORD followed by the three NTFS FILETIMEs, no unknown subtag.
            0x000a => {
                len == 32
                    && payload[..4] == [0, 0, 0, 0]
                    && zip_u16(payload, 4) == 1
                    && zip_u16(payload, 6) == 24
            }
            0x5455 => {
                len >= 1 && payload[0] & !7 == 0 && len == 1 + payload[0].count_ones() as usize * 4
            }
            0x7875 => {
                len >= 5
                    && payload[0] == 1
                    && (1..=8).contains(&payload[1])
                    && (2 + payload[1] as usize) < len
                    && (1..=8).contains(&payload[2 + payload[1] as usize])
                    && len == 3 + payload[1] as usize + payload[2 + payload[1] as usize] as usize
            }
            _ => false,
        };
        if !known {
            return false;
        }
        bytes = &bytes[len + 4..];
    }
    true
}
// Deliberately narrower than ZIP validity. Unknown layouts are not rejected;
// they simply use the existing engine. This check precedes ZipArchive allocation.
fn ordinary_zip_layout(bytes: &[u8], entries: &[Entry]) -> bool {
    if bytes.len() < 22 || entries.len() > MEMORY_ZIP_ENTRY_LIMIT {
        return false;
    }
    let Some(end) = (bytes.len().saturating_sub(65557)..=bytes.len() - 22)
        .rev()
        .find(|&i| {
            bytes[i..i + 4] == *b"PK\x05\x06"
                && i + 22 + zip_u16(bytes, i + 20) as usize == bytes.len()
        })
    else {
        return false;
    };
    let eocd = &bytes[end..];
    if zip_u16(eocd, 4) != 0
        || zip_u16(eocd, 6) != 0
        || zip_u16(eocd, 8) as usize != entries.len()
        || zip_u16(eocd, 10) as usize != entries.len()
    {
        return false;
    }
    let central = zip_u32(eocd, 16) as usize;
    let length = zip_u32(eocd, 12) as usize;
    if length > 8 * 1024 * 1024 || central.checked_add(length) != Some(end) {
        return false;
    }
    let mut cursor = central;
    let mut ranges = Vec::new();
    for _ in entries {
        if cursor + 46 > end || bytes[cursor..cursor + 4] != *b"PK\x01\x02" {
            return false;
        }
        let c = &bytes[cursor..end];
        let flags = zip_u16(c, 8);
        let method = zip_u16(c, 10);
        let name_len = zip_u16(c, 28) as usize;
        let extra_len = zip_u16(c, 30) as usize;
        let comment_len = zip_u16(c, 32) as usize;
        let next = cursor + 46 + name_len + extra_len + comment_len;
        if next > end
            || flags & !0x0806 != 0
            || !matches!(method, 0 | 8)
            || zip_u16(c, 34) != 0
            || zip_u32(c, 20) == u32::MAX
            || zip_u32(c, 24) == u32::MAX
        {
            return false;
        }
        let local = zip_u32(c, 42) as usize;
        if local + 30 > central || bytes[local..local + 4] != *b"PK\x03\x04" {
            return false;
        }
        let l = &bytes[local..central];
        let local_name = zip_u16(l, 26) as usize;
        let local_extra = zip_u16(l, 28) as usize;
        let data = local + 30 + local_name + local_extra;
        let data_end = data.checked_add(zip_u32(c, 20) as usize);
        if data > central
            || data_end.is_none_or(|n| n > central)
            || c[6..28] != l[4..26]
            || name_len != local_name
            || c[46..46 + name_len] != l[30..30 + local_name]
            || !ordinary_zip_extra(&c[46 + name_len..46 + name_len + extra_len])
            || !ordinary_zip_extra(&l[30 + local_name..30 + local_name + local_extra])
        {
            return false;
        }
        ranges.push((local, data_end.unwrap()));
        cursor = next;
    }
    if cursor != end {
        return false;
    }
    ranges.sort_unstable();
    let mut previous = 0;
    for (start, end) in ranges {
        if start != previous {
            return false;
        }
        previous = end;
    }
    previous == central
}

#[derive(Debug)]
enum MemoryChunk {
    Owned(Vec<u8>),
    Shared {
        bytes: Arc<Vec<u8>>,
        start: usize,
        end: usize,
    },
}
impl MemoryChunk {
    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Shared { bytes, start, end } => &bytes[*start..*end],
        }
    }
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }
    #[cfg(test)]
    fn capacity(&self) -> usize {
        match self {
            Self::Owned(bytes) => bytes.capacity(),
            Self::Shared { bytes, .. } => bytes.capacity(),
        }
    }
}
type MemoryFiles = std::collections::HashMap<String, MemoryChunk>;
fn decode_memory_zip(
    source: Option<File>,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<Option<MemoryFiles>, String> {
    let Some(mut source) = source else {
        return Ok(None);
    };
    let files = entries.iter().filter(|e| !e.is_dir).count();
    let size = entries
        .iter()
        .try_fold(0u64, |sum, e| sum.checked_add(e.size));
    if files < 128
        || entries.len() > MEMORY_ZIP_ENTRY_LIMIT
        || size.is_none_or(|n| n > MEMORY_ZIP_LIMIT)
        || entries
            .iter()
            .any(|e| e.encrypted || e.size > MEMORY_ZIP_FILE_LIMIT || (e.is_dir && e.size != 0))
    {
        return Ok(None);
    }
    ctl.check()?;
    let mut bytes = Vec::new();
    let length = source.metadata().map_err(|e| e.to_string())?.len();
    if length > MEMORY_ZIP_LIMIT {
        return Ok(None);
    }
    bytes
        .try_reserve_exact(length as usize)
        .map_err(|e| e.to_string())?;
    if bytes.capacity() as u64 > MEMORY_ZIP_LIMIT {
        return Ok(None);
    }
    bytes.resize(length as usize, 0);
    for chunk in bytes.chunks_mut(65536) {
        ctl.check()?;
        source.read_exact(chunk).map_err(|e| e.to_string())?;
    }
    ctl.check()?;
    if source.read(&mut [0u8; 1]).map_err(|e| e.to_string())? != 0
        || !ordinary_zip_layout(&bytes, entries)
    {
        return Ok(None);
    }
    let mut archive = match zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
        Ok(a) => a,
        Err(_) => return Ok(None),
    };
    if archive.offset() != 0
        || archive.len() != entries.len()
        || archive.has_duplicate_names()
        || archive.validate_header_consistency().is_err()
    {
        return Ok(None);
    }
    let expected = entries
        .iter()
        .map(|e| Ok((path_key(&validate_name(&e.name)?), e)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
    let mut names = Vec::with_capacity(entries.len());
    let mut seen = HashSet::new();
    for i in 0..archive.len() {
        ctl.check()?;
        let member = archive.by_index_data(i).map_err(|e| e.to_string())?;
        let Ok(name) = std::str::from_utf8(member.name_raw()) else {
            return Ok(None);
        };
        let key = path_key(&validate_name(name)?);
        let Some(entry) = expected.get(&key) else {
            return Ok(None);
        };
        let kind = member.unix_mode().map(|m| m & 0xf000).unwrap_or(0);
        if !seen.insert(key.clone())
            || member.encrypted()
            || member.is_symlink()
            || !matches!(kind, 0 | 0x4000 | 0x8000)
            || (kind == 0x4000 && !entry.is_dir)
            || (kind == 0x8000 && entry.is_dir)
            || member.is_dir() != entry.is_dir
            || member.size() != entry.size
            || !matches!(
                member.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            )
            || member.external_attributes() & 0x400 != 0
            || (!entry.is_dir && u32::from_str_radix(&entry.crc, 16).ok() != Some(member.crc32()))
        {
            return Ok(None);
        }
        names.push(key);
    }
    if seen.len() != entries.len() {
        return Ok(None);
    }
    ctl.set(
        "decoding",
        Some(0),
        "Verifying complete archive in bounded memory",
    );
    let mut contents = MemoryFiles::with_capacity(files);
    let mut allocated = 0u64;
    for (i, key) in names.into_iter().enumerate() {
        ctl.check()?;
        let entry = expected[&key];
        let mut member = archive
            .by_index(i)
            .map_err(|e| format!("ZIP decode: {e}"))?;
        let mut data = Vec::new();
        data.try_reserve_exact(entry.size as usize)
            .map_err(|e| e.to_string())?;
        allocated = allocated
            .checked_add(data.capacity() as u64)
            .ok_or("ZIP memory budget exceeded")?;
        if allocated > MEMORY_ZIP_LIMIT {
            return Err("ZIP memory budget exceeded".into());
        }
        data.resize(entry.size as usize, 0);
        let mut filled = 0;
        while filled < data.len() {
            ctl.check()?;
            let limit = (filled + 65536).min(data.len());
            let count = member
                .read(&mut data[filled..limit])
                .map_err(|e| format!("ZIP integrity: {e}"))?;
            if count == 0 {
                return Err("ZIP decoded length mismatch".into());
            }
            filled += count;
        }
        // A nonempty read to EOF is mandatory, including empty entries: it
        // finalizes the decoder and CRC rather than stopping at advertised size.
        if member
            .read(&mut [0u8; 1])
            .map_err(|e| format!("ZIP integrity: {e}"))?
            != 0
            || crc32fast::hash(&data) != member.crc32()
        {
            return Err("ZIP size or CRC mismatch".into());
        }
        if !entry.is_dir {
            contents.insert(key, MemoryChunk::Owned(data));
        }
    }
    ctl.check()?;
    Ok(Some(contents))
}

fn decode_memory_sevenz(
    source: Option<File>,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<Option<MemoryFiles>, String> {
    let Some(mut source) = source else {
        return Ok(None);
    };
    let count = entries.iter().filter(|e| !e.is_dir).count();
    let total = entries
        .iter()
        .try_fold(0u64, |sum, e| sum.checked_add(e.size));
    if count < 128
        || entries.len() > MEMORY_ZIP_ENTRY_LIMIT
        || total.is_none_or(|n| n > MEMORY_ZIP_LIMIT)
        || entries
            .iter()
            .any(|e| e.encrypted || e.size > MEMORY_ZIP_FILE_LIMIT || (e.is_dir && e.size != 0))
    {
        return Ok(None);
    }
    ctl.check()?;
    let length = source.metadata().map_err(|e| e.to_string())?.len();
    if length > MEMORY_ZIP_LIMIT {
        return Ok(None);
    }
    let mut data = Vec::new();
    data.try_reserve_exact(length as usize)
        .map_err(|e| e.to_string())?;
    if data.capacity() as u64 > MEMORY_ZIP_LIMIT {
        return Ok(None);
    }
    data.resize(length as usize, 0);
    for chunk in data.chunks_mut(65536) {
        ctl.check()?;
        source.read_exact(chunk).map_err(|e| e.to_string())?;
    }
    if source.read(&mut [0u8; 1]).map_err(|e| e.to_string())? != 0 {
        return Ok(None);
    }
    let Some(parsed) = crate::sevenz::parse_memory_archive(&data, &|| ctl.check())? else {
        return Ok(None);
    };
    if parsed.archive.entries.len() != entries.len() {
        return Ok(None);
    }
    let expected = entries
        .iter()
        .map(|e| Ok((path_key(&validate_name(&e.name)?), e)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for member in &parsed.archive.entries {
        ctl.check()?;
        let key = path_key(&validate_name(&member.name)?);
        let Some(entry) = expected.get(&key) else {
            return Ok(None);
        };
        let kind = (member.attrs >> 16) & 0xf000;
        if !seen.insert(key.clone())
            || member.is_dir != entry.is_dir
            || member.size != entry.size
            || member.attrs & 0x400 != 0
            || !matches!(kind, 0 | 0x4000 | 0x8000)
            || (kind == 0x4000 && !entry.is_dir)
            || (kind == 0x8000 && entry.is_dir)
            || (entry.size != 0 && member.crc != u32::from_str_radix(&entry.crc, 16).ok())
            || (entry.size != 0 && member.crc.is_none())
        {
            return Ok(None);
        }
        names.push(key);
    }
    if seen.len() != entries.len() {
        return Ok(None);
    }
    ctl.set(
        "decoding",
        Some(0),
        "Verifying complete solid archive in bounded memory",
    );
    let bytes = Arc::new(parsed.decode(&data, &|| ctl.check())?);
    if bytes.capacity() as u64 > MEMORY_ZIP_LIMIT {
        return Err("7z memory capacity budget exceeded".into());
    }
    let mut output = MemoryFiles::with_capacity(count);
    for (key, member) in names.into_iter().zip(&parsed.archive.entries) {
        ctl.check()?;
        let start = usize::try_from(member.offset_in_stream).map_err(|_| "7z offset overflow")?;
        let end = start
            .checked_add(member.size as usize)
            .ok_or("7z range overflow")?;
        let chunk = bytes
            .get(start..end)
            .ok_or("7z entry exceeds decoded stream")?;
        let entry = expected[&key];
        let crc = crc32fast::hash(chunk);
        if member.crc.is_some_and(|want| crc != want)
            || u32::from_str_radix(&entry.crc, 16)
                .ok()
                .is_some_and(|want| crc != want)
        {
            return Err("7z entry CRC mismatch".into());
        }
        if !entry.is_dir {
            output.insert(
                key,
                MemoryChunk::Shared {
                    bytes: bytes.clone(),
                    start,
                    end,
                },
            );
        }
    }
    ctl.check()?;
    Ok(Some(output))
}
#[derive(Clone, Copy)]
enum StagedContents<'a> {
    Disk(&'a Path),
    Memory(&'a MemoryFiles),
}
impl StagedContents<'_> {
    fn reader(&self, relative: &Path) -> Result<Box<dyn Read + '_>, String> {
        match self {
            Self::Disk(root) => Ok(Box::new(
                File::open(root.join(relative)).map_err(|e| e.to_string())?,
            )),
            Self::Memory(files) => Ok(Box::new(std::io::Cursor::new(
                files
                    .get(&path_key(relative))
                    .ok_or("verified memory entry missing")?
                    .as_slice(),
            ))),
        }
    }
}

fn preflight_output_targets(
    entries: &[&Entry],
    dest: &Path,
    overwrite: &str,
    output_guards: &mut CheckedDirectories,
) -> Result<(), String> {
    for e in entries {
        let target = dest.join(validate_name(&e.name)?);
        output_guards.check(&target)?;
        if target.exists() && (e.is_dir != target.is_dir() || (!e.is_dir && overwrite == "error")) {
            return Err(format!("destination conflict: {}", target.display()));
        }
        for parent in target.ancestors().skip(1) {
            if !output_guards.pinned(parent) && parent.exists() && !parent.is_dir() {
                return Err("output parent is a file".into());
            }
        }
    }
    Ok(())
}
fn extract_raw(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    let ctl = control(opts);
    let mut memory_source = memory_archive_source(path, opts);
    let all_entries = list_raw(path, opts)?;
    #[cfg(windows)]
    if let Some(source) = memory_source.as_ref() {
        let same = memory_archive_source(path, opts).is_some_and(|current| {
            directory_identity(source)
                .ok()
                .zip(directory_identity(&current).ok())
                .is_some_and(|(before, after)| before == after)
        });
        if !same {
            memory_source = None;
        }
    }
    let expected = expected_tree(&all_entries)?;
    let entries: Vec<_> = all_entries
        .iter()
        .filter(|e| selected(&e.name, opts))
        .collect();
    if !opts.selected.is_empty() && entries.is_empty() {
        return Err("selected archive entries were not found".into());
    }
    let dest = absolute(dest)?;
    // Declaration order releases all output locks before TempDir cleanup, also
    // on early errors (the output may share TEMP ancestors).
    let mut stage;
    let mut output_guards = CheckedDirectories::default();
    output_guards.check(&dest)?;
    let defer_preflight = opts.selected.is_empty()
        && !dest.try_exists().map_err(|e| e.to_string())?
        && dest.parent().is_some_and(Path::is_dir);
    if !defer_preflight {
        preflight_output_targets(&entries, &dest, &opts.overwrite, &mut output_guards)?;
    }
    ctl.check()?;
    let memory = if Path::new(path)
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("7z"))
    {
        decode_memory_sevenz(memory_source, &all_entries, &ctl)?
    } else {
        decode_memory_zip(memory_source, &all_entries, &ctl)?
    };
    let mut transaction = if memory.is_none() {
        new_directory_stage(path, &dest, opts, &mut output_guards)?
    } else {
        None
    };
    stage = if let Some((temp, _)) = transaction.as_mut() {
        temp.take()
    } else if memory.is_none() {
        Some(tempfile::tempdir().map_err(|e| e.to_string())?)
    } else {
        None
    };
    if defer_preflight && transaction.is_none() {
        preflight_output_targets(&entries, &dest, &opts.overwrite, &mut output_guards)?;
    }
    let mut stage_guards = stage
        .as_ref()
        .map(|s| CheckedDirectories::owned_root(s.path()));
    #[cfg(windows)]
    if let (Some((_, root)), Some(temp), Some(guards)) =
        (&transaction, &stage, stage_guards.as_mut())
    {
        // Register the dedicated DELETE handle rather than reopening root with
        // incompatible sharing and accidentally skipping all descendant pins.
        guards.held.insert(
            temp.path().to_path_buf(),
            (root.try_clone().map_err(|e| e.to_string())?, false),
        );
    }
    if let Some(stage) = &stage {
        let stage_guards = stage_guards.as_mut().unwrap();
        stage_guards.check(stage.path())?;
        ctl.set(
            "decoding",
            Some(0),
            "Decoding archive into temporary storage",
        );
        let mut args = base("x", opts)?;
        if all_entries.len() == 1 && !all_entries[0].is_dir {
            // Raw compressors may synthesize different disk and listing names.
            // One stdout decode is unambiguous and retains the advertised name.
            let target = stage.path().join(validate_name(&all_entries[0].name)?);
            fs::create_dir_all(target.parent().ok_or("missing staged parent")?)
                .map_err(|e| e.to_string())?;
            args.extend([
                "-so".into(),
                "--".into(),
                absolute(path)?.display().to_string(),
            ]);
            run(
                &args,
                None,
                opts,
                Some(File::create(target).map_err(|e| e.to_string())?),
            )?;
        } else {
            args.extend([
                format!("-o{}", stage.path().display()),
                "--".into(),
                absolute(path)?.display().to_string(),
            ]);
            run(&args, None, opts, None)?;
        }
        verify_stage_with_metadata(
            stage.path(),
            &expected,
            &ctl,
            stage_guards,
            transaction.as_ref().map(|_| all_entries.as_slice()),
        )?;
    }
    if let Some((_, root)) = transaction.as_ref() {
        let temp = stage.as_ref().unwrap();
        ctl.check()?;
        output_guards.check(&dest)?;
        output_guards.allow_publication(dest.parent().ok_or("missing output parent")?)?;
        output_guards.check(&dest)?;
        validate_transaction_root(root, temp.path())?;
        // Child handles deny directory renames. Root stays pinned through the
        // single handle-based no-replace commit; parent handles remain held.
        drop(stage_guards.take());
        ctl.set(
            "extracting",
            Some(99),
            "Committing verified archive directory",
        );
        ctl.check()?;
        if rename_directory_noclobber(root, &dest)? {
            let _published_path = stage.take().unwrap().keep();
            ctl.set("complete", Some(100), "Extraction complete");
            return Ok(entries.iter().filter(|e| !e.is_dir).count());
        }
        if dest.try_exists().map_err(|e| e.to_string())? {
            return Err("destination appeared during directory commit".into());
        }
        // Re-establish every descendant guard before old per-file publication.
        // Keep the dedicated root pin continuously across this transition.
        let mut guards = CheckedDirectories::owned_root(temp.path());
        #[cfg(windows)]
        guards.held.insert(
            temp.path().to_path_buf(),
            (root.try_clone().map_err(|e| e.to_string())?, false),
        );
        verify_stage_checked(temp.path(), &expected, &ctl, &mut guards)?;
        // An unsupported whole-directory rename may use old publication only
        // after atomically claiming the still-absent destination ourselves.
        // Never merge into a competitor created during fallback verification.
        ctl.check()?;
        output_guards.check(&dest)?;
        fs::create_dir(&dest).map_err(|e| format!("claim fallback destination: {e}"))?;
        #[cfg(windows)]
        output_guards.owned_dirs.insert(dest.clone());
        output_guards.check(&dest)?;
        stage_guards = Some(guards);
    }
    drop(transaction.take());
    let staged = match &memory {
        Some(files) => StagedContents::Memory(files),
        None => StagedContents::Disk(stage.as_ref().unwrap().path()),
    };
    let file_count = entries.iter().filter(|e| !e.is_dir).count();
    let count = if opts.overwrite == "error"
        && opts.selected.is_empty()
        && file_count >= 128
        && entries
            .iter()
            .all(|e| e.is_dir || e.size <= 8 * 1024 * 1024)
    {
        publish_staged_files_parallel(&entries, &dest, staged, &ctl, &mut output_guards)?
    } else {
        let mut count = 0;
        for (i, e) in entries.iter().enumerate() {
            ctl.check()?;
            ctl.set(
                "extracting",
                Some((i * 100 / entries.len().max(1)) as u8),
                &e.name,
            );
            let rel = validate_name(&e.name)?;
            let target = dest.join(&rel);
            output_guards.check(&target)?;
            if e.is_dir {
                output_guards.create_dirs(&target)?;
                continue;
            }
            let Some(target) =
                choose_target_checked(target, &opts.overwrite, &mut |p| output_guards.check(p))?
            else {
                continue;
            };
            let parent = target.parent().ok_or("missing output directory")?;
            if !output_guards.pinned(parent) {
                output_guards.create_dirs(parent)?;
            }
            output_guards.check(parent)?;
            let mut tmp = output_temp(parent)?;
            let mut source = staged.reader(&rel)?;
            let mut buf = vec![0; 1024 * 1024];
            loop {
                ctl.check()?;
                let n = source.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                use std::io::Write;
                tmp.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
            ctl.check()?;
            output_guards.check(&target)?;
            output_guards.allow_publication(parent)?;
            output_guards.check(&target)?;
            let published = publish_output(tmp, &target, opts.overwrite == "overwrite")?;
            restore_open_file_metadata(&published, e).map_err(|e| format!("file metadata: {e}"))?;
            count += 1;
        }
        count
    };
    for e in entries.iter().rev().filter(|e| e.is_dir) {
        restore_metadata(&dest.join(validate_name(&e.name)?), e)
            .map_err(|e| format!("directory metadata: {e}"))?;
    }
    drop(stage_guards);
    ctl.set("complete", Some(100), "Extraction complete");
    Ok(count)
}

// Inspect the actual inherited DACL before writing any archive data. Shared
// destinations retain private system-TEMP staging; no ACL propagation shortcuts.
#[cfg(windows)]
fn private_stage_acl(path: &Path) -> Result<bool, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let handle = match fs::OpenOptions::new()
        .access_mode(0x20080)
        .share_mode(3)
        .custom_flags(0x02200000)
        .open(path)
    {
        Ok(h) => h,
        Err(_) => return Ok(false),
    };
    if directory_attributes(&handle)? & 0x410 != 0x10 {
        return Ok(false);
    }
    private_stage_acl_handle(&handle)
}
#[cfg(windows)]
fn private_stage_acl_handle(handle: &File) -> Result<bool, String> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};
    #[link(name = "Advapi32")]
    extern "system" {
        fn GetSecurityInfo(
            handle: *mut c_void,
            kind: u32,
            info: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
        fn GetAce(acl: *mut c_void, index: u32, ace: *mut *mut c_void) -> i32;
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            class: u32,
            buffer: *mut c_void,
            size: u32,
            needed: *mut u32,
        ) -> i32;
    }
    #[link(name = "Kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    struct Allocation(*mut c_void);
    impl Drop for Allocation {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn sid_text(sid: *mut c_void) -> Option<String> {
        let mut text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return None;
        }
        let _allocation = Allocation(text.cast());
        let mut len = 0;
        unsafe {
            while *text.add(len) != 0 {
                len += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(text, len)).ok()
        }
    }
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(-1isize as *mut c_void, 8, &mut token) } == 0 {
        return Ok(false);
    }
    let mut needed = 0;
    unsafe {
        GetTokenInformation(token, 1, std::ptr::null_mut(), 0, &mut needed);
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    let queried =
        unsafe { GetTokenInformation(token, 1, buffer.as_mut_ptr().cast(), needed, &mut needed) };
    unsafe {
        CloseHandle(token);
    }
    if queried == 0 || buffer.is_empty() {
        return Ok(false);
    }
    let Some(current) = sid_text(unsafe { *(buffer.as_ptr().cast::<*mut c_void>()) }) else {
        return Ok(false);
    };
    let mut descriptor = std::ptr::null_mut();
    let mut dacl = std::ptr::null_mut();
    let mut owner = std::ptr::null_mut();
    let error = unsafe {
        GetSecurityInfo(
            handle.as_raw_handle(),
            1,
            5,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Ok(false);
    }
    let _allocation = Allocation(descriptor);
    if dacl.is_null() {
        return Ok(false);
    }
    let Some(owner) = sid_text(owner) else {
        return Ok(false);
    };
    if owner != current && !matches!(owner.as_str(), "S-1-5-18" | "S-1-5-32-544") {
        return Ok(false);
    }
    let count = unsafe { std::ptr::read_unaligned((dacl as *const u8).add(4).cast::<u16>()) };
    if count > 256 {
        return Ok(false);
    }
    for i in 0..count {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(dacl, i as u32, &mut ace) } == 0 {
            return Ok(false);
        }
        let raw = ace as *const u8;
        let kind = unsafe { *raw };
        let size = unsafe { std::ptr::read_unaligned(raw.add(2).cast::<u16>()) };
        if size < 12 || !matches!(kind, 0 | 1) {
            return Ok(false);
        } // ordinary allow/deny only
        if kind == 1 {
            continue;
        } // denies cannot expose data
        let Some(sid) = sid_text(unsafe { raw.add(8) } as *mut c_void) else {
            return Ok(false);
        };
        if sid != current
            && !(sid == "S-1-3-0" && unsafe { *raw.add(1) } & 8 != 0)
            && !matches!(sid.as_str(), "S-1-5-18" | "S-1-5-32-544" | "S-1-3-4")
        {
            return Ok(false);
        }
    }
    Ok(true)
}

// A sibling inherits the same parent DACL as a new destination. An arbitrary
// same-volume staging location is insufficient for inherited security semantics.
fn new_directory_stage(
    path: &str,
    dest: &Path,
    opts: &EngineOptions,
    guards: &mut CheckedDirectories,
) -> Result<Option<(Option<tempfile::TempDir>, File)>, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if !opts.selected.is_empty() || dest.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let Some(parent) = dest.parent() else {
            return Ok(None);
        };
        if !parent.is_dir() {
            return Ok(None);
        }
        // Header signatures also cover the first standard split volume; unknown
        // formats retain the existing decoder/stage semantics.
        let mut magic = [0; 6];
        let mut input = File::open(path).map_err(|e| e.to_string())?;
        if input.read_exact(&mut magic).is_err()
            || !(magic == *b"7z\xbc\xaf'\x1c"
                || magic[..4] == *b"PK\x03\x04"
                || magic[..4] == *b"PK\x05\x06")
        {
            return Ok(None);
        }
        guards.check(parent)?;
        if !private_stage_acl(parent)? {
            return Ok(None);
        }
        let temp = match tempfile::Builder::new()
            .prefix(".open-archive-stage-")
            .tempdir_in(parent)
        {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let root = match fs::OpenOptions::new()
            .access_mode(0x30080)
            .share_mode(1)
            .custom_flags(0x02200000)
            .open(temp.path())
        {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };
        validate_transaction_root(&root, temp.path())?;
        if !private_stage_acl_handle(&root)? {
            return Ok(None);
        }
        Ok(Some((Some(temp), root)))
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = (path, dest, opts, guards);
        Ok(None)
    }
}
fn validate_transaction_root(root: &File, path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if directory_attributes(root)? & 0x410 != 0x10 {
            return Err("changed transaction root".into());
        }
        let current = fs::OpenOptions::new()
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(path)
            .map_err(|e| e.to_string())?;
        if directory_identity(root)? != directory_identity(&current)?
            || directory_attributes(&current)? & 0x410 != 0x10
        {
            return Err("transaction root identity changed".into());
        }
    }
    #[cfg(not(windows))]
    let _ = (root, path);
    Ok(())
}
fn normalize_staged_entry(
    path: &Path,
    entry: &Entry,
    meta: &fs::Metadata,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
) -> Result<(), String> {
    ctl.check()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let attributes = if entry.is_dir { 0x10 } else { 0x20 }
            | if entry.attributes.contains('R') { 1 } else { 0 };
        let modified =
            chrono::NaiveDateTime::parse_from_str(&entry.modified, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .and_then(|date| {
                    use chrono::TimeZone;
                    chrono::Local.from_local_datetime(&date).earliest()
                })
                .map(std::time::SystemTime::from);
        if meta.file_attributes() == attributes
            && modified.is_none_or(|time| meta.modified().ok() == Some(time))
        {
            return Ok(());
        }
    }
    if entry.is_dir {
        guards.allow_publication(&path)?;
    }
    guards.check(&path)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "Kernel32")]
        extern "system" {
            fn SetFileAttributesW(path: *const u16, attrs: u32) -> i32;
        }
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let attrs = if entry.is_dir { 0x10 } else { 0x20 }
            | if entry.attributes.contains('R') { 1 } else { 0 };
        if unsafe { SetFileAttributesW(wide.as_ptr(), attrs) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    restore_metadata(&path, entry).map_err(|e| format!("staged metadata: {e}"))?;
    Ok(())
}
fn rename_directory_noclobber(root: &File, target: &Path) -> Result<bool, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        #[link(name = "Kernel32")]
        extern "system" {
            fn SetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                class: i32,
                info: *const std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let name = target.as_os_str().encode_wide().collect::<Vec<_>>();
        if !target.is_absolute() || name.len() > 32767 || name.contains(&0) {
            return Ok(false);
        }
        let size = 22 + name.len() * 2;
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let raw = buffer.as_mut_ptr().cast::<u8>();
        unsafe {
            std::ptr::write_unaligned(raw.add(16).cast::<u32>(), (name.len() * 2) as u32);
            std::ptr::copy_nonoverlapping(name.as_ptr().cast::<u8>(), raw.add(20), name.len() * 2);
            if SetFileInformationByHandle(root.as_raw_handle(), 3, raw.cast(), size as u32) != 0 {
                return Ok(true);
            }
        }
        // A competitor's directory is never replaced. Old publication handles
        // existing-destination collision policies after an unsupported commit.
        Ok(false)
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = (root, target);
        Ok(false)
    }
}

fn output_temp(parent: &Path) -> Result<tempfile::NamedTempFile, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if let Ok(file) = tempfile::Builder::new().make_in(parent, |path| {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .access_mode(0xc0010000)
                .share_mode(7)
                .open(path)
        }) {
            return Ok(file);
        }
    }
    tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())
}
#[cfg(all(windows, target_pointer_width = "64"))]
fn publish_output_by_handle(
    temp: &mut tempfile::NamedTempFile,
    target: &Path,
    overwrite: bool,
) -> bool {
    if !target.is_absolute() {
        return false;
    }
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    #[link(name = "Kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            class: i32,
            info: *const std::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    // FILE_BASIC_INFO: preserve all times and match the ARCHIVE attribute
    // left by the existing SetFileAttributes/MoveFileEx publication on Windows.
    // The existing handle avoids a path reopen and clears TEMPORARY.
    let mut basic = [0u64; 5];
    basic[4] = 0x20;
    let name = target.as_os_str().encode_wide().collect::<Vec<_>>();
    if name.len() <= 32767 && !name.contains(&0) {
        let bytes = 22 + name.len() * 2; // include a NUL for the Win32 wrapper
                                         // Aligned FILE_RENAME_INFO: BOOLEAN, padding, HANDLE, DWORD, WCHAR[].
        let mut rename = vec![0u64; bytes.div_ceil(8)];
        let raw = rename.as_mut_ptr() as *mut u8;
        unsafe {
            *raw = u8::from(overwrite);
            std::ptr::write_unaligned(raw.add(16) as *mut u32, (name.len() * 2) as u32);
            std::ptr::copy_nonoverlapping(name.as_ptr() as *const u8, raw.add(20), name.len() * 2);
            if SetFileInformationByHandle(
                temp.as_file().as_raw_handle(),
                0,
                basic.as_ptr().cast(),
                40,
            ) != 0
                && SetFileInformationByHandle(
                    temp.as_file().as_raw_handle(),
                    3,
                    raw.cast(),
                    bytes as u32,
                ) != 0
            {
                temp.disable_cleanup(true);
                return true;
            }
        }
    }
    false
}
fn publish_output(
    mut temp: tempfile::NamedTempFile,
    target: &Path,
    overwrite: bool,
) -> Result<File, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    if publish_output_by_handle(&mut temp, target, overwrite) {
        return Ok(temp.into_file());
    }
    // Unsupported filesystems, namespaces or permissions retain the old API.
    if overwrite {
        temp.persist(target)
    } else {
        temp.persist_noclobber(target)
    }
    .map_err(|e| e.error.to_string())
}

// Bounded publication is restricted to full, no-overwrite extraction. The entire
// stage has passed verification before entry. Rename/skip/overwrite retain their
// original serial ordering, including name-conflict resolution.
#[cfg(test)]
fn publish_small_files_parallel(
    entries: &[&Entry],
    dest: &Path,
    stage: &Path,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
) -> Result<usize, String> {
    publish_staged_files_parallel(entries, dest, StagedContents::Disk(stage), ctl, guards)
}
fn publish_staged_files_parallel(
    entries: &[&Entry],
    dest: &Path,
    stage: StagedContents<'_>,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
) -> Result<usize, String> {
    use std::sync::atomic::AtomicUsize;
    for e in entries.iter().filter(|e| e.is_dir) {
        ctl.check()?;
        let target = dest.join(validate_name(&e.name)?);
        guards.check(&target)?;
        guards.create_dirs(&target)?;
    }
    let files = entries
        .iter()
        .copied()
        .filter(|e| !e.is_dir)
        .collect::<Vec<_>>();
    let mut parents = HashSet::new();
    for entry in &files {
        ctl.check()?;
        let target = dest.join(validate_name(&entry.name)?);
        let parent = target.parent().ok_or("missing output directory")?;
        if parents.insert(parent.to_path_buf()) {
            guards.check(parent)?;
            if !guards.pinned(parent) {
                guards.create_dirs(parent)?;
            }
            guards.allow_publication(parent)?;
        }
    }
    let prepared_guards: &CheckedDirectories = guards;
    let next = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let first_error = Mutex::new(None::<String>);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(4);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..workers {
            let files = &files;
            let next = &next;
            let completed = &completed;
            let stopped = &stopped;
            let first_error = &first_error;
            handles.push(scope.spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<(), String> {
                        let mut buffer = vec![0u8; 1024 * 1024];
                        while !stopped.load(Ordering::Acquire) {
                            ctl.check()?;
                            let index = next.fetch_add(1, Ordering::Relaxed);
                            let Some(entry) = files.get(index) else { break };
                            let relative = validate_name(&entry.name)?;
                            let target = dest.join(&relative);
                            let parent = target.parent().ok_or("missing output directory")?;
                            // Retain early collision detection plus atomic noclobber.
                            choose_target_checked(target.clone(), "error", &mut |p| {
                                prepared_guards.check_prepared(p)
                            })?;
                            prepared_guards.check_prepared(parent)?;
                            let mut temp = output_temp(parent)?;
                            let mut source = stage.reader(&relative)?;
                            loop {
                                ctl.check()?;
                                if stopped.load(Ordering::Acquire) {
                                    return Ok(());
                                }
                                let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
                                if n == 0 {
                                    break;
                                }
                                std::io::Write::write_all(&mut temp, &buffer[..n])
                                    .map_err(|e| e.to_string())?;
                            }
                            ctl.check()?;
                            if stopped.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            prepared_guards.check_prepared(&target)?;
                            let published = publish_output(temp, &target, false)?;
                            restore_open_file_metadata(&published, entry)?;
                            let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                            ctl.set(
                                "extracting",
                                Some((count * 100 / files.len().max(1)) as u8),
                                &entry.name,
                            );
                        }
                        Ok(())
                    },
                ));
                let error = match outcome {
                    Ok(Err(e)) => Some(e),
                    Err(_) => Some("output publication worker failed".into()),
                    _ => None,
                };
                if let Some(error) = error {
                    let mut first = first_error.lock().unwrap_or_else(|e| e.into_inner());
                    if first.is_none() {
                        *first = Some(error);
                    }
                    stopped.store(true, Ordering::Release);
                }
            }));
        }
        // Never return or release guards/stage while an output worker is alive.
        for handle in handles {
            let _ = handle.join();
        }
    });
    if let Some(error) = first_error.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(error);
    }
    ctl.check()?;
    Ok(completed.load(Ordering::Relaxed))
}

pub fn extract_with_control(
    path: &str,
    dest: &str,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<usize, String> {
    let mut o = opts.clone();
    o.control = Some(ctl.clone());
    extract(path, dest, &o)
}
fn test_raw(path: &str, opts: &EngineOptions) -> Result<(), String> {
    let ctl = control(opts);
    ctl.set("testing", None, "Checking archive integrity");
    let mut a = base("t", opts)?;
    a.extend(["--".into(), absolute(path)?.display().to_string()]);
    run(&a, None, opts, None)?;
    ctl.set("complete", Some(100), "Archive integrity verified");
    Ok(())
}
pub fn test_with_control(path: &str, opts: &EngineOptions, ctl: &JobControl) -> Result<(), String> {
    let mut o = opts.clone();
    o.control = Some(ctl.clone());
    test(path, &o)
}
// Source directories belong to the user, not this operation. These strong
// handles exist ONLY for one input_paths preflight and are released before any
// encoder runs. They do not provide a snapshot across validation/encoding.
#[cfg(windows)]
struct SourceValidationGuards {
    held: std::collections::HashMap<PathBuf, File>,
    limit: usize,
}
#[cfg(windows)]
impl SourceValidationGuards {
    fn new() -> Self {
        Self {
            held: std::collections::HashMap::new(),
            limit: 4096,
        }
    }
    fn pin_directory(&mut self, path: &Path) -> Result<bool, String> {
        use std::os::windows::fs::OpenOptionsExt;
        if self.held.contains_key(path) {
            return Ok(true);
        }
        if self.held.len() >= self.limit {
            return Ok(false);
        }
        let handle = match fs::OpenOptions::new()
            .access_mode(0x81)
            .share_mode(1)
            .custom_flags(0x02200000)
            .open(path)
        {
            Ok(handle) => handle,
            Err(_) => return Ok(false),
        };
        let attrs = directory_attributes(&handle)?;
        if attrs & 0x400 != 0 {
            return Err(format!(
                "linked input directory rejected: {}",
                path.display()
            ));
        }
        if attrs & 0x10 == 0 {
            return Ok(false);
        }
        self.held.insert(path.to_path_buf(), handle);
        Ok(true)
    }
    fn validate(&mut self, path: &Path) -> Result<(), String> {
        // Unusual namespaces and every unpinnable branch retain the complete
        // original ancestor walk; never treat an unsuccessful pin as a cache hit.
        if attribute_query_path(path).is_none() {
            return validate_tree(path);
        }
        for parent in path
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if !self.pin_directory(parent)? {
                return validate_tree(path);
            }
        }
        self.validate_child(path)
    }
    // Only called after the full ancestor chain is strongly pinned, including
    // recursive calls below. Never reuse directory enumeration's cached metadata.
    fn validate_child(&mut self, path: &Path) -> Result<(), String> {
        let mut leaves = Vec::new();
        self.collect_checked(path, &mut leaves)?;
        Self::validate_leaves(&leaves)
    }
    fn validate_leaves(leaves: &[PathBuf]) -> Result<(), String> {
        // Directory pins remain owned by self until every scoped worker joins.
        // Enumeration only schedules work; each leaf receives a new OS query.
        let workers = if leaves.len() >= 512 {
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(4)
        } else { 1 };
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for chunk in leaves.chunks(leaves.len().div_ceil(workers).max(1)) {
                handles.push(scope.spawn(move || {
                    for path in chunk {
                        match fresh_path_attributes(path) {
                            Some(attrs) if attrs & 0x400 != 0 => return Err(format!("linked input path rejected: {}",path.display())),
                            Some(attrs) if attrs & 0x10 == 0 => (),
                            // A file may have become a directory since enumeration.
                            // Rewalk the entire branch, never trust the old type.
                            _ => {
                                // Unlike output validation, source validation must
                                // fail if an enumerated input has disappeared.
                                fs::symlink_metadata(path).map_err(|e|e.to_string())?;
                                validate_tree(path)?;
                            },
                        }
                    }
                    Ok(())
                }));
            }
            let mut first_error = None;
            for handle in handles {
                let result = handle.join().unwrap_or_else(|_|Err("source validation worker failed".into()));
                if let Err(error) = result { if first_error.is_none() {first_error=Some(error);} }
            }
            first_error.map_or(Ok(()), Err)
        })
    }
    fn collect_checked(&mut self, path: &Path, leaves: &mut Vec<PathBuf>) -> Result<(), String> {
        let attrs = match fresh_path_attributes(path) {
            Some(attrs) => attrs,
            None => {
                // Unsupported namespace/query failure keeps the original walk.
                fs::symlink_metadata(path).map_err(|e|e.to_string())?;
                return validate_tree(path);
            }
        };
        if attrs & 0x400 != 0 {
            return Err(format!("linked input path rejected: {}", path.display()));
        }
        if attrs & 0x10 != 0 {
            // Recheck the opened directory itself after the path metadata read:
            // an intervening replacement must not turn into a trusted reparse.
            if !self.pin_directory(path)? {
                return validate_tree(path);
            }
            for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().map_err(|e|e.to_string())?.is_dir() {
                    self.collect_checked(&entry.path(), leaves)?;
                } else {
                    leaves.push(entry.path());
                    // Bound retained paths even for very large source trees.
                    if leaves.len() == 4096 {
                        Self::validate_leaves(leaves)?;
                        leaves.clear();
                    }
                }
            }
        }
        Ok(())
    }
}

fn input_paths(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    let _timer = PhaseTimer::new("input-preflight");
    if inputs.is_empty() {
        return Err("no input files".into());
    }
    #[cfg(windows)]
    let mut source_guards = SourceValidationGuards::new();
    let mut result = Vec::new();
    let mut names = HashSet::new();
    for src in inputs {
        let p = absolute(src)?;
        no_links(&p)?;
        if !p.exists() {
            return Err(format!("input missing: {src}"));
        }
        let name = p
            .file_name()
            .ok_or("cannot archive filesystem root")?
            .to_string_lossy()
            .to_lowercase();
        if !names.insert(name) {
            return Err("input root filenames collide".into());
        }
        #[cfg(windows)]
        source_guards.validate(&p)?;
        #[cfg(not(windows))]
        validate_tree(&p)?;
        result.push(p)
    }
    Ok(result)
}
fn validate_tree(path: &Path) -> Result<(), String> {
    no_links(path)?;
    if path.is_dir() {
        for e in fs::read_dir(path).map_err(|e| e.to_string())? {
            validate_tree(&e.map_err(|e| e.to_string())?.path())?
        }
    }
    Ok(())
}
fn reject_output_in_inputs(parent: &Path, inputs: &[PathBuf]) -> Result<(), String> {
    // Resolve before mkdir: a rejected output must not add folders to a source.
    let existing = parent
        .ancestors()
        .find(|path| path.exists())
        .ok_or("output parent has no existing ancestor")?;
    let parent = fs::canonicalize(existing).map_err(|e| e.to_string())?;
    for input in inputs {
        if input.is_dir() && parent.starts_with(fs::canonicalize(input).map_err(|e| e.to_string())?)
        {
            return Err("output archive must be outside its input directories".into());
        }
    }
    Ok(())
}

fn compression(args: &mut Vec<String>, format: &str, opts: &EngineOptions) -> Result<(), String> {
    if !["zip", "7z", "tar", "gzip", "bzip2", "xz", "wim", "sfx"].contains(&format) {
        return Err("unsupported output format".into());
    }
    if opts.password.is_some() && !matches!(format, "zip" | "7z" | "sfx") {
        return Err("this format does not support passwords".into());
    }
    let method = opts
        .method
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase());
    let selected_method = match (format, method.as_deref()) {
        (_, None) => None,
        ("zip", Some("deflate")) => Some("Deflate"),
        ("zip", Some("store" | "copy")) => Some("Copy"),
        ("zip", Some("bzip2")) => Some("BZip2"),
        ("zip", Some("lzma")) => Some("LZMA"),
        ("zip", Some("ppmd")) => Some("PPMd"),
        ("7z" | "sfx", Some("lzma2")) => Some("LZMA2"),
        ("7z" | "sfx", Some("lzma")) => Some("LZMA"),
        ("7z" | "sfx", Some("ppmd")) => Some("PPMd"),
        ("7z" | "sfx", Some("bzip2")) => Some("BZip2"),
        ("7z" | "sfx", Some("copy" | "store")) => Some("Copy"),
        _ => return Err(format!("compression method is unsupported for {format}")),
    };
    let encryption = opts
        .encryption
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase());
    if encryption.is_some() && opts.password.is_none() {
        return Err("encryption requires a password".into());
    }
    let selected_encryption = match (format, encryption.as_deref()) {
        (_, None) => "AES256",
        ("zip", Some("zipcrypto")) => "ZipCrypto",
        ("zip", Some("aes128")) => "AES128",
        ("zip", Some("aes192")) => "AES192",
        ("zip" | "7z" | "sfx", Some("aes256")) => "AES256",
        _ => return Err(format!("encryption method is unsupported for {format}")),
    };
    args.push(format!("-t{}", if format == "sfx" { "7z" } else { format }));
    args.push("-sse".into());
    if let Some(n) = opts.threads {
        args.push(format!("-mmt={n}"));
    }
    if let Some(l) = opts.level {
        args.push(format!("-mx={l}"));
    }
    if let Some(method) = selected_method {
        args.push(format!(
            "{}={method}",
            if format == "zip" { "-mm" } else { "-m0" }
        ));
    }
    if let Some(n) = opts.volume_bytes {
        args.push(format!("-v{n}b"));
    }
    if opts.password.is_some() {
        if format == "zip" {
            args.push(format!("-mem={selected_encryption}"));
        } else {
            args.push("-mhe=on".into());
        }
    }
    if format == "sfx" {
        let stub = executable()?.with_file_name("7z.sfx");
        if !stub.is_file() {
            return Err("7z.sfx runtime missing".into());
        }
        args.push(format!("-sfx{}", stub.display()));
    }
    Ok(())
}

pub fn create(
    out: &str,
    inputs: &[String],
    format: &str,
    opts: &EngineOptions,
) -> Result<Vec<String>, String> {
    let _timer = PhaseTimer::new("create-total");
    if format == "zipx" {
        return create_libarchive(out, inputs, format, opts);
    }
    if matches!(
        format,
        "tgz" | "tar.gz" | "tbz2" | "tar.bz2" | "txz" | "tar.xz"
    ) {
        if opts.password.is_some() || opts.volume_bytes.is_some() {
            return Err("compressed TAR does not support password or split creation".into());
        }
        control(opts).check()?;
        // The recursive codec step sees a temporary TAR as its input, so check
        // the final destination against the original sources before staging.
        let paths = input_paths(inputs)?;
        let output = absolute(out)?;
        no_links(&output)?;
        reject_output_in_inputs(output.parent().ok_or("output has no parent")?, &paths)?;
        let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let tar = temp.path().join("contents.tar").display().to_string();
        create(&tar, inputs, "tar", opts)?;
        let codec = if matches!(format, "tgz" | "tar.gz") {
            "gzip"
        } else if matches!(format, "tbz2" | "tar.bz2") {
            "bzip2"
        } else {
            "xz"
        };
        return create(out, &[tar], codec, opts);
    }
    let ctl = control(opts);
    ctl.check()?;
    let paths = input_paths(inputs)?;
    let out = absolute(out)?;
    no_links(&out)?;
    if out.exists() {
        return Err("output archive already exists; use update to modify it".into());
    }
    if matches!(format, "gzip" | "bzip2" | "xz") && (paths.len() != 1 || !paths[0].is_file()) {
        return Err("single-stream format requires exactly one regular file; create a TAR first for multiple files".into());
    }
    let parent = out.parent().ok_or("output has no parent")?;
    reject_output_in_inputs(parent, &paths)?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    reject_output_in_inputs(parent, &paths)?;
    let temp = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    let staged = temp
        .path()
        .join(out.file_name().ok_or("output has no filename")?);
    let mut a = base("a", opts)?;
    compression(&mut a, format, opts)?;
    a.push("--".into());
    a.push(staged.display().to_string());
    a.extend(paths.iter().map(|p| p.display().to_string()));
    ctl.set("compressing", None, "Creating archive");
    run(&a, None, opts, None)?;
    ctl.check()?;
    let mut produced = fs::read_dir(temp.path())
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    produced.sort();
    if produced.is_empty() {
        return Err("engine produced no archive".into());
    }
    for p in &produced {
        let target = parent.join(p.file_name().unwrap());
        no_links(&target)?;
        if target.exists() {
            return Err(format!("output exists: {}", target.display()));
        }
    }
    let mut result = Vec::new();
    for p in produced {
        let target = parent.join(p.file_name().unwrap());
        publish_new(&p, &target, &ctl)?;
        result.push(target.display().to_string());
    }
    ctl.set("complete", Some(100), "Archive created");
    Ok(result)
}
pub fn create_with_control(
    out: &str,
    inputs: &[String],
    format: &str,
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<Vec<String>, String> {
    let mut o = opts.clone();
    o.control = Some(ctl.clone());
    create(out, inputs, format, &o)
}
pub(crate) fn update_lock(path: &Path, ctl: &JobControl) -> Result<File, String> {
    use std::hash::{Hash, Hasher};
    let root = std::env::temp_dir().join("OpenBandiZip-locks");
    no_links(&root)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    fs::canonicalize(path)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .to_lowercase()
        .hash(&mut hash);
    let path = root.join(format!("{:016x}.lock", hash.finish()));
    no_links(&path)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    loop {
        ctl.check()?;
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(40)),
            Err(std::fs::TryLockError::Error(e)) => return Err(e.to_string()),
        }
    }
}

/// action: add, delete, rename (flat old/new pairs).
pub fn update(
    path: &str,
    action: &str,
    inputs: &[String],
    opts: &EngineOptions,
) -> Result<(), String> {
    if opts.volume_bytes.is_some() {
        return Err("updating split archives is unsupported".into());
    }
    if inputs.is_empty() {
        return Err("no update entries".into());
    }
    let command = match action {
        "add" => "a",
        "delete" => "d",
        "rename" => "rn",
        _ => return Err("unknown archive update action".into()),
    };
    let locked_path = absolute(path)?;
    no_links(&locked_path)?;
    let _archive_lock = update_lock(&locked_path, &control(opts))?;
    if action != "add" {
        let existing = list(path, opts)?;
        let sources: Vec<&String> = if action == "rename" {
            inputs.iter().step_by(2).collect()
        } else {
            inputs.iter().collect()
        };
        for source in sources {
            if !existing.iter().any(|e| {
                e.name.replace('\\', "/") == source.replace('\\', "/")
                    || e.name.replace('\\', "/").starts_with(
                        &(source.replace('\\', "/").trim_end_matches('/').to_owned() + "/"),
                    )
            }) {
                return Err(format!("archive entry not found: {source}"));
            }
        }
        if action == "rename" {
            for new_name in inputs.iter().skip(1).step_by(2) {
                if existing.iter().any(|e| {
                    e.name
                        .replace('\\', "/")
                        .eq_ignore_ascii_case(&new_name.replace('\\', "/"))
                }) {
                    return Err(format!("rename destination exists: {new_name}"));
                }
            }
        }
    }
    let path = absolute(path)?;
    no_links(&path)?;
    if !path.is_file() {
        return Err("archive not found".into());
    }
    let parent = path.parent().ok_or("missing archive parent")?;
    let temp = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    let staged = temp
        .path()
        .join(path.file_name().ok_or("missing archive filename")?);
    fs::copy(&path, &staged).map_err(|e| e.to_string())?;
    let mut a = base(command, opts)?;
    let names = if action == "add" {
        {
            let paths = input_paths(inputs)?;
            reject_output_in_inputs(parent, &paths)?;
            let original = fs::canonicalize(&path).map_err(|e| e.to_string())?;
            if paths
                .iter()
                .any(|p| fs::canonicalize(p).ok().as_ref() == Some(&original))
            {
                return Err("cannot add an archive to itself".into());
            }
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        }
    } else {
        for n in inputs {
            validate_name(n)?;
        }
        if action == "rename" && inputs.len() % 2 != 0 {
            return Err("rename requires old/new pairs".into());
        }
        inputs.to_vec()
    };
    a.push("--".into());
    a.push(staged.display().to_string());
    a.extend(names);
    let ctl = control(opts);
    ctl.set("updating", None, "Updating a temporary archive copy");
    run(&a, None, opts, None)?;
    test(staged.to_str().ok_or("invalid archive path")?, opts)?;
    ctl.check()?;
    no_links(&path)?;
    fs::rename(&staged, &path).map_err(|e| e.to_string())?;
    ctl.set("complete", Some(100), "Archive updated");
    Ok(())
}
pub fn update_with_control(
    path: &str,
    action: &str,
    inputs: &[String],
    opts: &EngineOptions,
    ctl: &JobControl,
) -> Result<(), String> {
    let mut o = opts.clone();
    o.control = Some(ctl.clone());
    update(path, action, inputs, &o)
}
fn preview_raw(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    validate_name(name)?;
    let list = list(path, opts)?;
    let e = list
        .iter()
        .find(|e| e.name.replace('\\', "/") == name.replace('\\', "/") && !e.is_dir)
        .ok_or("preview entry not found")?;
    if e.size > 32 * 1024 * 1024 {
        return Err("preview exceeds 32 MiB limit".into());
    }
    let tmp = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut a = base("x", opts)?;
    a.extend([
        "-so".into(),
        "--".into(),
        absolute(path)?.display().to_string(),
        e.name.clone(),
    ]);
    run(
        &a,
        None,
        opts,
        Some(tmp.try_clone().map_err(|e| e.to_string())?),
    )?;
    use std::io::{Seek, SeekFrom};
    let mut tmp = tmp;
    tmp.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    tmp.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    if out.len() > 32 * 1024 * 1024 || out.len() as u64 != e.size {
        return Err("preview size mismatch".into());
    }
    Ok(out)
}

fn compound_tar(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [
        ".tar.gz", ".tgz", ".tar.bz2", ".tbz", ".tbz2", ".tar.xz", ".txz",
    ]
    .iter()
    .any(|suffix| p.ends_with(suffix))
}
fn unpack_tar(path: &str, opts: &EngineOptions) -> Result<(tempfile::TempDir, String), String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let inner = temp.path().join("contents.tar");
    let mut a = base("x", opts)?;
    a.extend([
        "-so".into(),
        "--".into(),
        absolute(path)?.display().to_string(),
    ]);
    control(opts).set("decompressing", None, "Opening compressed TAR stream");
    run(
        &a,
        None,
        opts,
        Some(File::create(&inner).map_err(|e| e.to_string())?),
    )?;
    Ok((temp, inner.display().to_string()))
}
pub fn list(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    if compound_tar(path) {
        let (_temp, inner) = unpack_tar(path, opts)?;
        list_raw(&inner, opts)
    } else {
        list_raw(path, opts)
    }
}
pub fn extract(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    if compound_tar(path) {
        let (_temp, inner) = unpack_tar(path, opts)?;
        extract_raw(&inner, dest, opts)
    } else {
        extract_raw(path, dest, opts)
    }
}
pub fn test(path: &str, opts: &EngineOptions) -> Result<(), String> {
    test_raw(path, opts)?;
    if compound_tar(path) {
        let (_temp, inner) = unpack_tar(path, opts)?;
        test_raw(&inner, opts)?;
    }
    Ok(())
}
pub fn preview(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    if compound_tar(path) {
        let (_temp, inner) = unpack_tar(path, opts)?;
        preview_raw(&inner, name, opts)
    } else {
        preview_raw(path, name, opts)
    }
}
// Keep the already-open published handle for metadata, avoiding both a second
// path lookup and a path-substitution window between publication and timestamp.
fn restore_open_file_metadata(file: &File, entry: &Entry) -> Result<(), String> {
    if !entry.modified.is_empty() {
        if let Ok(date) =
            chrono::NaiveDateTime::parse_from_str(&entry.modified, "%Y-%m-%d %H:%M:%S%.f")
        {
            use chrono::TimeZone;
            if let Some(local) = chrono::Local.from_local_datetime(&date).earliest() {
                let timestamp: std::time::SystemTime = local.into();
                file.set_times(fs::FileTimes::new().set_modified(timestamp))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    if entry.attributes.contains('R') {
        let mut permissions = file.metadata().map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(true);
        file.set_permissions(permissions)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(crate) fn restore_metadata(path: &Path, entry: &Entry) -> Result<(), String> {
    if !entry.modified.is_empty() {
        if let Ok(date) =
            chrono::NaiveDateTime::parse_from_str(&entry.modified, "%Y-%m-%d %H:%M:%S%.f")
        {
            use chrono::TimeZone;
            if let Some(local) = chrono::Local.from_local_datetime(&date).earliest() {
                let timestamp: std::time::SystemTime = local.into();
                let mut options = fs::OpenOptions::new();
                options.write(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.access_mode(0x100).custom_flags(0x02000000);
                }
                let file = options.open(path).map_err(|e| e.to_string())?;
                file.set_times(fs::FileTimes::new().set_modified(timestamp))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    if entry.attributes.contains('R') {
        let mut permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn create_libarchive(
    out: &str,
    inputs: &[String],
    _format: &str,
    opts: &EngineOptions,
) -> Result<Vec<String>, String> {
    use std::io::Write;
    let ctl = control(opts);
    ctl.check()?;
    let _ = base("a", opts)?;
    let method = opts
        .method
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase());
    if method
        .as_deref()
        .is_some_and(|m| !matches!(m, "xz" | "store" | "copy"))
    {
        return Err("ZIPX method must be XZ or Store".into());
    }
    let stored = method
        .as_deref()
        .is_some_and(|m| matches!(m, "store" | "copy"))
        || (method.is_none() && opts.level == Some(0));
    let encryption = opts
        .encryption
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase());
    if encryption.is_some() && opts.password.is_none() {
        return Err("encryption requires a password".into());
    }
    let aes = match encryption.as_deref() {
        None | Some("aes256") => zip::AesMode::Aes256,
        Some("aes128") => zip::AesMode::Aes128,
        Some("aes192") => zip::AesMode::Aes192,
        Some("zipcrypto") => {
            return Err(
                "ZIPX ZipCrypto creation is not supported by the native writer; use ZIP or AES"
                    .into(),
            )
        }
        _ => return Err("unsupported ZIPX encryption method".into()),
    };

    if opts.level.is_some_and(|v| v > 9) || opts.volume_bytes == Some(0) {
        return Err("invalid compression options".into());
    }
    let paths = input_paths(inputs)?;
    let out = absolute(out)?;
    no_links(&out)?;
    if out.exists() {
        return Err("output archive already exists".into());
    }
    let parent = out.parent().ok_or("output has no parent")?;
    // Reject containment before creating any directories inside the input tree.
    let ancestor = parent
        .ancestors()
        .find(|p| p.exists())
        .ok_or("output parent has no existing ancestor")?;
    reject_output_in_inputs(ancestor, &paths)?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    reject_output_in_inputs(parent, &paths)?;
    let temp = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    let staged = temp.path().join("archive.zipx");
    let mut files = Vec::<(PathBuf, String, bool)>::new();
    let mut names = HashSet::new();
    let mut pending = paths
        .iter()
        .map(|p| {
            (
                p.clone(),
                p.file_name().unwrap().to_string_lossy().into_owned(),
            )
        })
        .collect::<Vec<_>>();
    while let Some((p, name)) = pending.pop() {
        ctl.check()?;
        no_links(&p)?;
        validate_name(&name)?;
        if !names.insert(name.to_lowercase()) {
            return Err("archive names collide".into());
        }
        let meta = fs::metadata(&p).map_err(|e| e.to_string())?;
        if !meta.is_file() && !meta.is_dir() {
            return Err("unsupported input file type".into());
        }
        if meta.is_dir() {
            for child in fs::read_dir(&p).map_err(|e| e.to_string())? {
                let child = child.map_err(|e| e.to_string())?;
                pending.push((
                    child.path(),
                    format!("{name}/{}", child.file_name().to_string_lossy()),
                ));
            }
        }
        files.push((p, name, meta.is_dir()));
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let mut writer = zip::ZipWriter::new(File::create(&staged).map_err(|e| e.to_string())?);
    for (i, (p, name, is_dir)) in files.iter().enumerate() {
        ctl.check()?;
        no_links(p)?;
        ctl.set(
            "compressing",
            Some(((i * 100) / files.len().max(1)) as u8),
            "Creating ZIPX with XZ compression",
        );
        let meta = fs::metadata(p).map_err(|e| e.to_string())?;
        let mut options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default()
            .compression_method(if *is_dir || meta.len() == 0 || stored {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Xz
            })
            .compression_level(if *is_dir || meta.len() == 0 || stored {
                None
            } else {
                opts.level.map(|v| v as i64)
            })
            .large_file(meta.len() >= u32::MAX as u64);
        if let Ok(modified) = meta.modified() {
            use chrono::{Datelike, Timelike};
            let date: chrono::DateTime<chrono::Local> = modified.into();
            if let Ok(time) = zip::DateTime::from_date_and_time(
                date.year() as u16,
                date.month() as u8,
                date.day() as u8,
                date.hour() as u8,
                date.minute() as u8,
                date.second() as u8,
            ) {
                options = options.last_modified_time(time);
            }
        }
        if *is_dir {
            writer
                .add_directory(format!("{name}/"), options)
                .map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(password) = opts.password.as_deref() {
            options = options.with_aes_encryption(aes, password);
        }
        writer
            .start_file(name, options)
            .map_err(|e| e.to_string())?;
        let mut input = File::open(p).map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut total = 0u64;
        loop {
            ctl.check()?;
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            total += n as u64;
        }
        let after = input.metadata().map_err(|e| e.to_string())?;
        if total != meta.len()
            || after.len() != meta.len()
            || after.modified().ok() != meta.modified().ok()
        {
            return Err("input changed during compression".into());
        }
    }
    writer
        .finish()
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    test(staged.to_str().ok_or("invalid staging path")?, opts)?;
    ctl.check()?;
    let mut outputs = Vec::new();
    if let Some(volume) = opts.volume_bytes {
        let length = fs::metadata(&staged).map_err(|e| e.to_string())?.len();
        let count = length.div_ceil(volume);
        if count > 10000 {
            return Err("too many output volumes (maximum 10000)".into());
        }
        let mut input = File::open(&staged).map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        for i in 1..=count {
            ctl.check()?;
            let target = PathBuf::from(format!("{}.{i:03}", out.display()));
            no_links(&target)?;
            if target.exists() {
                return Err("output volume already exists".into());
            }
            let part = temp.path().join(format!("part-{i}"));
            let mut output = File::create(&part).map_err(|e| e.to_string())?;
            let mut remaining = volume;
            while remaining > 0 {
                ctl.check()?;
                let cap = buffer.len().min(remaining.min(usize::MAX as u64) as usize);
                let n = input.read(&mut buffer[..cap]).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                remaining -= n as u64;
            }
            output.sync_all().map_err(|e| e.to_string())?;
            parts.push((part, target));
        }
        for (part, target) in parts {
            publish_new(&part, &target, &ctl)?;
            outputs.push(target.display().to_string());
        }
    } else {
        publish_new(&staged, &out, &ctl)?;
        outputs.push(out.display().to_string());
    }
    ctl.set("complete", Some(100), "Archive created");
    Ok(outputs)
}

fn publish_new(staged: &Path, target: &Path, ctl: &JobControl) -> Result<(), String> {
    ctl.check()?;
    no_links(target)?;
    // Same-volume hard-link publication is atomic and never replaces an existing name.
    // The private staging link is removed by TempDir, leaving only the published link.
    if fs::hard_link(staged, target).is_ok() {
        return Ok(());
    }
    if target.exists() {
        return Err(format!("output exists: {}", target.display()));
    }
    let parent = target.parent().ok_or("output parent missing")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let mut source = File::open(staged).map_err(|e| e.to_string())?;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        ctl.check()?;
        let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(temp.as_file_mut(), &buffer[..n]).map_err(|e| e.to_string())?;
    }
    ctl.check()?;
    no_links(target)?;
    temp.persist_noclobber(target)
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod staging_tests {
    use super::*;

    #[test]
    fn bounded_sevenz_memory_is_shared_and_verifies_unselected_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixture.7z");
        fs::write(
            &path,
            crate::sevenz::memory_test_fixture(true, false, false),
        )
        .unwrap();
        let opts = EngineOptions::default();
        let entries = list_raw(path.to_str().unwrap(), &opts).unwrap();
        let contents = decode_memory_sevenz(
            memory_archive_source(path.to_str().unwrap(), &opts),
            &entries,
            &JobControl::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(contents.len(), 160);
        let mut first = None;
        for chunk in contents.values() {
            assert_eq!(chunk.as_slice(), vec![42; 1024]);
            match chunk {
                MemoryChunk::Shared { bytes, .. } => {
                    if let Some(previous) = first {
                        assert!(Arc::ptr_eq(previous, bytes));
                    } else {
                        first = Some(bytes);
                    }
                }
                _ => panic!("solid block must be shared"),
            }
        }
        let out = temp.path().join("selected");
        let options = EngineOptions {
            selected: vec!["folder/file-0.txt".into()],
            ..Default::default()
        };
        assert_eq!(
            extract(path.to_str().unwrap(), out.to_str().unwrap(), &options).unwrap(),
            1
        );
        assert_eq!(
            fs::read(out.join("folder/file-0.txt")).unwrap(),
            vec![42; 1024]
        );
        // Preserve all checks even when the corrupted member is not selected.
        let mut bytes = crate::sevenz::memory_test_fixture(false, false, false);
        bytes[32 + 159 * 1024] ^= 1;
        // Recompute pack/folder and outer CRCs so only the unselected file CRC
        // catches corruption after full decoding, rather than an outer checksum.
        let header = 32 + u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize;
        let old_crc = crc32fast::hash(&vec![42; 160 * 1024]).to_le_bytes();
        let new_crc = crc32fast::hash(&bytes[32..header]).to_le_bytes();
        let positions = (header..bytes.len() - 3)
            .filter(|&i| bytes[i..i + 4] == old_crc)
            .collect::<Vec<_>>();
        assert_eq!(positions.len(), 2);
        for i in positions {
            bytes[i..i + 4].copy_from_slice(&new_crc);
        }
        let hcrc = crc32fast::hash(&bytes[header..]);
        bytes[28..32].copy_from_slice(&hcrc.to_le_bytes());
        let scrc = crc32fast::hash(&bytes[12..32]);
        bytes[8..12].copy_from_slice(&scrc.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let badout = temp.path().join("bad");
        assert!(
            extract(path.to_str().unwrap(), badout.to_str().unwrap(), &options)
                .unwrap_err()
                .contains("entry CRC")
        );
        assert!(!badout.exists());
        let mut oversized = entries.clone();
        oversized[0].size = MEMORY_ZIP_FILE_LIMIT + 1;
        assert!(decode_memory_sevenz(
            memory_archive_source(path.to_str().unwrap(), &opts),
            &oversized,
            &JobControl::new()
        )
        .unwrap()
        .is_none());
        let ctl = JobControl::new();
        ctl.cancel();
        assert!(decode_memory_sevenz(
            memory_archive_source(path.to_str().unwrap(), &opts),
            &entries,
            &ctl
        )
        .unwrap_err()
        .contains("cancelled"));
    }

    #[cfg(windows)]
    #[test]
    fn source_preflight_preserves_roots_and_releases_phase_pins() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(first.join("deep")).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join("deep/a"), b"a").unwrap();
        fs::write(second.join("b"), b"b").unwrap();
        let inputs = vec![first.display().to_string(), second.display().to_string()];
        let result = input_paths(&inputs).unwrap();
        assert_eq!(result, vec![first.clone(), second.clone()]);
        for path in &result {
            validate_tree(path).unwrap();
        }
        fs::rename(&first, temp.path().join("moved")).unwrap();
        fs::rename(temp.path().join("moved"), &first).unwrap();
        let mut guards = SourceValidationGuards::new();
        assert!(guards.pin_directory(&first).unwrap());
        assert!(fs::rename(&first, temp.path().join("blocked")).is_err());
        drop(guards);
        fs::rename(&first, temp.path().join("released")).unwrap();
        fs::rename(temp.path().join("released"), &first).unwrap();
        assert!(input_paths(&[inputs[0].clone(), inputs[0].to_uppercase()])
            .unwrap_err()
            .contains("collide"));
        fs::rename(&first, temp.path().join("after-error")).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn source_recursive_child_requeries_after_enumeration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"untouched").unwrap();
        let mut guards = SourceValidationGuards::new();
        guards.validate(&root).unwrap();
        let leaf = root.join("leaf");
        fs::create_dir(&leaf).unwrap();
        let enumerated = fs::read_dir(&root).unwrap().next().unwrap().unwrap();
        assert!(enumerated.metadata().unwrap().is_dir());
        fs::remove_dir(&leaf).unwrap();
        assert!(Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&leaf).arg(&outside).output().unwrap().status.success());
        assert!(guards.validate_child(&enumerated.path()).unwrap_err().contains("linked"));
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"untouched");
        fs::remove_dir(&leaf).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn source_leaf_batches_reject_replaced_links_and_validate_new_subtrees() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();fs::create_dir(&outside).unwrap();
        let mut guards = SourceValidationGuards::new();guards.validate(&root).unwrap();
        let mut leaves = Vec::new();
        for i in 0..600 {let p=root.join(format!("file-{i}"));fs::write(&p,b"ok").unwrap();leaves.push(p);}
        SourceValidationGuards::validate_leaves(&leaves).unwrap();
        let gone = root.join("gone");
        fs::write(&gone,b"gone").unwrap();
        let mut missing_batch=leaves.clone();missing_batch.push(gone.clone());
        fs::remove_file(&gone).unwrap();
        assert!(SourceValidationGuards::validate_leaves(&missing_batch).is_err());
        assert!(guards.validate_child(&gone).is_err());
        // Previously enumerated file becomes a directory containing a junction.
        fs::remove_file(&leaves[599]).unwrap();fs::create_dir(&leaves[599]).unwrap();
        let link = leaves[599].join("hidden-link");
        assert!(Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&outside).output().unwrap().status.success());
        assert!(SourceValidationGuards::validate_leaves(&leaves).is_err());
        fs::remove_dir(&link).unwrap();
        SourceValidationGuards::validate_leaves(&leaves).unwrap();
        assert!(fs::rename(&root,temp.path().join("blocked")).is_err());
        drop(guards);fs::rename(&root,temp.path().join("released")).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn process_wait_wakes_on_exit_and_preserves_status() {
        use std::os::windows::process::CommandExt;
        let mut child = Command::new("cmd").args(["/C", "exit", "7"]).creation_flags(0x08000000).spawn().unwrap();
        wait_for_process(&child, Duration::from_secs(10)).unwrap();
        assert_eq!(child.try_wait().unwrap().unwrap().code(), Some(7));
        let start = std::time::Instant::now();
        wait_for_process(&child, Duration::from_secs(2)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[cfg(windows)]
    #[test]
    fn source_preflight_rejects_swapped_links_even_with_budget_or_sharing_fallback() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"keep").unwrap();
        let leaf = root.join("leaf");
        fs::create_dir(&leaf).unwrap();
        let enumerated = fs::read_dir(&root).unwrap().next().unwrap().unwrap();
        assert!(enumerated.metadata().unwrap().is_dir());
        fs::remove_dir(&leaf).unwrap();
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&leaf)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut guards = SourceValidationGuards::new();
        assert!(guards.validate(&enumerated.path()).is_err());
        drop(guards);
        let mut guards = SourceValidationGuards::new();
        guards.limit = 0;
        assert!(guards.validate(&root).is_err());
        drop(guards);
        let conflict = fs::OpenOptions::new()
            .access_mode(0x10000)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(&root)
            .unwrap();
        let mut guards = SourceValidationGuards::new();
        assert!(!guards.pin_directory(&root).unwrap());
        assert!(guards.validate(&root).is_err());
        drop(guards);
        drop(conflict);
        assert!(input_paths(&[leaf.display().to_string()]).is_err());
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
        fs::remove_dir(&leaf).unwrap();
        let mut limited = SourceValidationGuards::new();
        limited.limit = 0;
        limited.validate(&root).unwrap();
        drop(limited);
        fs::rename(&root, temp.path().join("released")).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn merged_verification_requeries_leaf_after_enumeration_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        let outside = temp.path().join("outside");
        fs::create_dir(&stage).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"untouched").unwrap();
        let leaf = stage.join("leaf");
        fs::create_dir(&leaf).unwrap();
        let enumerated = fs::read_dir(&stage).unwrap().next().unwrap().unwrap();
        assert!(enumerated.metadata().unwrap().is_dir());
        fs::remove_dir(&leaf).unwrap();
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&leaf)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut guards = CheckedDirectories::owned_root(&stage);
        assert!(checked_staged_metadata(&enumerated.path(), &mut guards)
            .unwrap_err()
            .contains("linked decoded"));
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"untouched");
        drop(guards);
        fs::remove_dir(&leaf).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn merged_metadata_keeps_directory_order_and_cleans_readonly_error_stage() {
        let (_fixture, path, entries) = memory_zip_fixture();
        let _ = path;
        let mut file = entries[1].clone();
        file.name = "dir/a.txt".into();
        file.size = 3;
        file.attributes = "R".into();
        file.modified = "2021-02-03 04:05:06".into();
        let mut first = entries.last().unwrap().clone();
        first.name = "dir".into();
        first.attributes = "R".into();
        first.modified = String::new();
        let mut second = first.clone();
        second.attributes = String::new();
        second.modified = "2019-01-02 03:04:05".into();
        let listed = vec![first, second, file];
        let expected = expected_tree(&listed).unwrap();
        let stage = tempfile::tempdir().unwrap();
        fs::create_dir(stage.path().join("dir")).unwrap();
        fs::write(stage.path().join("dir/a.txt"), b"abc").unwrap();
        let mut guards = CheckedDirectories::owned_root(stage.path());
        verify_stage_with_metadata(
            stage.path(),
            &expected,
            &JobControl::new(),
            &mut guards,
            Some(&listed),
        )
        .unwrap();
        use chrono::TimeZone;
        let time: std::time::SystemTime = chrono::Local
            .with_ymd_and_hms(2019, 1, 2, 3, 4, 5)
            .earliest()
            .unwrap()
            .into();
        assert_eq!(
            fs::metadata(stage.path().join("dir"))
                .unwrap()
                .modified()
                .unwrap(),
            time
        );
        assert!(fs::metadata(stage.path().join("dir/a.txt"))
            .unwrap()
            .permissions()
            .readonly());
        assert!(fs::metadata(stage.path().join("dir"))
            .unwrap()
            .permissions()
            .readonly());
        drop(guards);
        stage.close().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let root = stage.path().to_path_buf();
        fs::create_dir(root.join("dir")).unwrap();
        fs::write(root.join("dir/a.txt"), b"abc").unwrap();
        let mut wrong = expected;
        wrong.insert("missing.txt".into(), (false, 1));
        let mut guards = CheckedDirectories::owned_root(&root);
        assert!(verify_stage_with_metadata(
            &root,
            &wrong,
            &JobControl::new(),
            &mut guards,
            Some(&listed)
        )
        .unwrap_err()
        .contains("missing"));
        assert!(fs::metadata(root.join("dir/a.txt"))
            .unwrap()
            .permissions()
            .readonly());
        drop(guards);
        stage.close().unwrap();
        assert!(!root.exists());
    }

    #[cfg(windows)]
    fn private_test_directory(path: &Path) {
        use std::{ffi::c_void, os::windows::ffi::OsStrExt};
        #[link(name = "Advapi32")]
        extern "system" {
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text: *const u16,
                rev: u32,
                sd: *mut *mut c_void,
                size: *mut u32,
            ) -> i32;
            fn SetFileSecurityW(path: *const u16, info: u32, sd: *mut c_void) -> i32;
        }
        #[link(name = "Kernel32")]
        extern "system" {
            fn LocalFree(ptr: *mut c_void) -> *mut c_void;
        }
        let text = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut sd = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    text.as_ptr(),
                    1,
                    &mut sd,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let name = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        assert_ne!(
            unsafe { SetFileSecurityW(name.as_ptr(), 0x80000004, sd) },
            0
        );
        unsafe {
            LocalFree(sd);
        }
    }
    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn directory_transaction_noclobber_and_guard_identity() {
        let temp = tempfile::tempdir().unwrap();
        private_test_directory(temp.path());
        let archive = temp.path().join("a.zip");
        fs::write(&archive, b"PK\x03\x04xx").unwrap();
        let target = temp.path().join("result");
        let mut guards = CheckedDirectories::default();
        let (stage, root) = new_directory_stage(
            archive.to_str().unwrap(),
            &target,
            &EngineOptions::default(),
            &mut guards,
        )
        .unwrap()
        .unwrap();
        let stage = stage.unwrap();
        fs::create_dir(stage.path().join("child")).unwrap();
        fs::write(stage.path().join("child/a"), b"safe").unwrap();
        let mut childguards = CheckedDirectories::owned_root(stage.path());
        childguards.held.insert(
            stage.path().to_path_buf(),
            (root.try_clone().unwrap(), false),
        );
        childguards.check(&stage.path().join("child")).unwrap();
        assert!(childguards.pinned(&stage.path().join("child")));
        assert!(fs::rename(stage.path(), temp.path().join("moved")).is_err());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("competitor"), b"keep").unwrap();
        drop(childguards);
        assert!(!rename_directory_noclobber(&root, &target).unwrap());
        assert_eq!(fs::read(target.join("competitor")).unwrap(), b"keep");
        let destination = temp.path().join("committed");
        assert!(rename_directory_noclobber(&root, &destination).unwrap());
        let old = stage.keep();
        fs::create_dir(&old).unwrap();
        fs::write(old.join("replacement"), b"keep").unwrap();
        drop(root);
        assert!(old.join("replacement").exists());
        assert_eq!(fs::read(destination.join("child/a")).unwrap(), b"safe");
    }
    #[cfg(windows)]
    #[test]
    fn directory_transaction_matches_original_acl_metadata_and_shared_parent_falls_back() {
        use std::{
            ffi::c_void,
            os::windows::{ffi::OsStrExt, fs::MetadataExt},
        };
        #[link(name = "Advapi32")]
        extern "system" {
            fn GetFileSecurityW(
                path: *const u16,
                info: u32,
                buffer: *mut u8,
                size: u32,
                needed: *mut u32,
            ) -> i32;
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text: *const u16,
                rev: u32,
                sd: *mut *mut c_void,
                size: *mut u32,
            ) -> i32;
            fn SetFileSecurityW(path: *const u16, info: u32, sd: *mut c_void) -> i32;
        }
        #[link(name = "Kernel32")]
        extern "system" {
            fn LocalFree(ptr: *mut c_void) -> *mut c_void;
        }
        fn wide(p: &Path) -> Vec<u16> {
            p.as_os_str().encode_wide().chain(Some(0)).collect()
        }
        fn dacl(p: &Path) -> Vec<u8> {
            let path = wide(p);
            let mut size = 0;
            unsafe {
                GetFileSecurityW(path.as_ptr(), 4, std::ptr::null_mut(), 0, &mut size);
            }
            let mut out = vec![0; size as usize];
            assert_ne!(
                unsafe { GetFileSecurityW(path.as_ptr(), 4, out.as_mut_ptr(), size, &mut size) },
                0
            );
            out
        }
        fn security(p: &Path, sddl: &str) {
            let text = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let mut sd = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        text.as_ptr(),
                        1,
                        &mut sd,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            assert_ne!(
                unsafe { SetFileSecurityW(wide(p).as_ptr(), 0x80000004, sd) },
                0
            );
            unsafe {
                LocalFree(sd);
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        fs::create_dir(&parent).unwrap();
        // Protected complex inheritance, including inherited-only and no-propagate.
        security(
            &parent,
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)(A;OICIIO;GR;;;CO)(A;CINP;GR;;;SY)",
        );
        assert!(private_stage_acl(&parent).unwrap());
        let archive = temp.path().join("fixture.zip");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        use std::io::Write;
        writer
            .start_file(
                "nested/deep/a.txt",
                zip::write::SimpleFileOptions::default().unix_permissions(0o444),
            )
            .unwrap();
        writer.write_all(b"content").unwrap();
        writer
            .add_directory("empty/", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        let mut archive_bytes = fs::read(&archive).unwrap();
        let central = archive_bytes
            .windows(4)
            .position(|b| b == b"PK\x01\x02")
            .unwrap();
        archive_bytes[central + 38] |= 1;
        fs::write(&archive, archive_bytes).unwrap();

        let old = parent.join("old");
        fs::create_dir(&old).unwrap();
        let new = parent.join("new");
        let candidate = new_directory_stage(
            archive.to_str().unwrap(),
            &new,
            &EngineOptions::default(),
            &mut CheckedDirectories::default(),
        )
        .unwrap()
        .expect("private parent must qualify");
        drop(candidate.1);
        candidate.0.unwrap().close().unwrap();
        extract(
            archive.to_str().unwrap(),
            old.to_str().unwrap(),
            &EngineOptions::default(),
        )
        .unwrap();
        extract(
            archive.to_str().unwrap(),
            new.to_str().unwrap(),
            &EngineOptions::default(),
        )
        .unwrap();
        for rel in ["", "nested", "nested/deep", "nested/deep/a.txt", "empty"] {
            assert_eq!(dacl(&old.join(rel)), dacl(&new.join(rel)), "DACL {rel}");
            if !rel.is_empty() {
                let a = fs::metadata(old.join(rel)).unwrap();
                let b = fs::metadata(new.join(rel)).unwrap();
                assert_eq!(a.file_attributes(), b.file_attributes(), "attributes {rel}");
                if rel == "nested/deep/a.txt" {
                    assert_eq!(a.last_write_time(), b.last_write_time());
                }
            }
        }
        assert!(fs::metadata(new.join("nested/deep/a.txt"))
            .unwrap()
            .permissions()
            .readonly());
        let mut bytes = fs::read(&archive).unwrap();
        let position = {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            let position = z.by_index(0).unwrap().data_start().unwrap() as usize;
            position
        };
        bytes[position] ^= 1;
        let broken = temp.path().join("broken.zip");
        fs::write(&broken, bytes).unwrap();
        let failed = parent.join("failed");
        assert!(extract(
            broken.to_str().unwrap(),
            failed.to_str().unwrap(),
            &EngineOptions::default()
        )
        .is_err());
        assert!(!failed.exists());
        assert_eq!(
            fs::read_dir(&parent).unwrap().count(),
            2,
            "failed stage must clean up"
        );
        let deletion = temp.path().join("delete-child");
        fs::create_dir(&deletion).unwrap();
        security(
            &deletion,
            "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)(A;;0x00000040;;;WD)",
        );
        assert!(!private_stage_acl(&deletion).unwrap());
        let shared = temp.path().join("shared");
        fs::create_dir(&shared).unwrap();
        security(&shared, "D:P(A;OICI;FA;;;WD)");
        assert!(!private_stage_acl(&shared).unwrap());
        assert!(new_directory_stage(
            archive.to_str().unwrap(),
            &shared.join("result"),
            &EngineOptions::default(),
            &mut CheckedDirectories::default()
        )
        .unwrap()
        .is_none());
    }

    fn memory_zip_fixture() -> (tempfile::TempDir, PathBuf, Vec<Entry>) {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixture.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut entries = Vec::new();
        for i in 0..160 {
            let name = format!("folder/file-{i:03}.txt");
            let data = if i == 0 { vec![] } else { vec![i as u8; 3073] };
            zip.start_file(&name, options).unwrap();
            zip.write_all(&data).unwrap();
            entries.push(Entry {
                name,
                size: data.len() as u64,
                packed_size: data.len() as u64,
                is_dir: false,
                crc: format!("{:08X}", crc32fast::hash(&data)),
                encrypted: false,
                modified: String::new(),
                method: "Store".into(),
                comment: String::new(),
                attributes: String::new(),
            });
        }
        zip.add_directory("empty/", options).unwrap();
        entries.push(Entry {
            name: "empty/".into(),
            size: 0,
            packed_size: 0,
            is_dir: true,
            crc: "00000000".into(),
            encrypted: false,
            modified: String::new(),
            method: "Store".into(),
            comment: String::new(),
            attributes: String::new(),
        });
        zip.finish().unwrap();
        (temp, path, entries)
    }
    #[test]
    fn bounded_zip_memory_verifies_all_bytes_and_preserves_empty_entries() {
        let (_temp, path, entries) = memory_zip_fixture();
        let contents = decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()),
            &entries,
            &JobControl::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(contents.len(), 160);
        assert!(contents["folder/file-000.txt"].is_empty());
        assert_eq!(contents["folder/file-159.txt"].as_slice(), vec![159; 3073]);
        assert!(contents.values().map(|v| v.capacity() as u64).sum::<u64>() <= MEMORY_ZIP_LIMIT);
        let out = tempfile::tempdir().unwrap();
        let refs = entries.iter().collect::<Vec<_>>();
        let mut guard = CheckedDirectories::default();
        assert_eq!(
            publish_staged_files_parallel(
                &refs,
                out.path(),
                StagedContents::Memory(&contents),
                &JobControl::new(),
                &mut guard
            )
            .unwrap(),
            160
        );
        assert!(out.path().join("empty").is_dir());
        assert_eq!(
            fs::read(out.path().join("folder/file-159.txt")).unwrap(),
            vec![159; 3073]
        );
    }
    #[test]
    fn bounded_zip_memory_crc_failure_is_error_not_fallback_even_when_unselected() {
        let (_temp, path, entries) = memory_zip_fixture();
        let mut bytes = fs::read(&path).unwrap();
        let mut cursor = 0;
        for i in 0..160 {
            assert_eq!(&bytes[cursor..cursor + 4], b"PK\x03\x04");
            let data = cursor
                + 30
                + zip_u16(&bytes, cursor + 26) as usize
                + zip_u16(&bytes, cursor + 28) as usize;
            if i == 159 {
                bytes[data] ^= 1;
                break;
            }
            cursor = data + zip_u32(&bytes, cursor + 18) as usize;
        }
        fs::write(&path, bytes).unwrap();
        let options = EngineOptions {
            selected: vec!["folder/file-001.txt".into()],
            ..Default::default()
        };
        let error = decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &options),
            &entries,
            &JobControl::new(),
        )
        .unwrap_err();
        assert!(
            error.contains("integrity") || error.contains("CRC"),
            "{error}"
        );
        let out = path.parent().unwrap().join("must-not-exist");
        assert!(extract(path.to_str().unwrap(), out.to_str().unwrap(), &options).is_err());
        assert!(
            !out.exists(),
            "no selected content may publish before all entries verify"
        );
        fs::create_dir_all(out.join("folder")).unwrap();
        fs::write(out.join("folder/file-001.txt"), b"preserve existing").unwrap();
        let options = EngineOptions {
            overwrite: "skip".into(),
            ..options
        };
        assert!(extract(path.to_str().unwrap(), out.to_str().unwrap(), &options).is_err());
        assert_eq!(
            fs::read(out.join("folder/file-001.txt")).unwrap(),
            b"preserve existing"
        );
        assert_eq!(fs::read_dir(out.join("folder")).unwrap().count(), 1);
    }
    #[test]
    fn bounded_zip_memory_budget_password_codepage_and_header_mismatch_fall_back() {
        let (_temp, path, mut entries) = memory_zip_fixture();
        for options in [
            EngineOptions {
                password: Some("password".into()),
                ..Default::default()
            },
            EngineOptions {
                codepage: Some(936),
                ..Default::default()
            },
        ] {
            assert!(memory_archive_source(path.to_str().unwrap(), &options).is_none());
        }
        entries[0].size = MEMORY_ZIP_FILE_LIMIT + 1;
        assert!(decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()),
            &entries,
            &JobControl::new()
        )
        .unwrap()
        .is_none());
        entries[0].size = 0;
        let mut bytes = fs::read(&path).unwrap();
        bytes[30] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()),
            &entries,
            &JobControl::new()
        )
        .unwrap()
        .is_none());
        let ctl = JobControl::new();
        ctl.cancel();
        assert!(decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()),
            &entries,
            &ctl
        )
        .unwrap_err()
        .contains("cancelled"));
    }
    #[cfg(windows)]
    #[test]
    fn bounded_zip_source_lock_denies_write_delete_until_decoding_finishes() {
        let (_temp, path, _entries) = memory_zip_fixture();
        let lock =
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()).unwrap();
        assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
        assert!(fs::rename(&path, path.with_extension("moved")).is_err());
        drop(lock);
        assert!(fs::OpenOptions::new().write(true).open(&path).is_ok());
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn handle_publication_is_atomic_keeps_inherited_acl_and_matches_original_attributes() {
        use std::io::Write;
        use std::os::windows::fs::MetadataExt;
        fn dacl(path: &Path) -> Vec<u8> {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "Advapi32")]
            extern "system" {
                fn GetFileSecurityW(
                    path: *const u16,
                    info: u32,
                    buffer: *mut u8,
                    length: u32,
                    needed: *mut u32,
                ) -> i32;
            }
            let path = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let mut size = 0;
            unsafe {
                GetFileSecurityW(path.as_ptr(), 4, std::ptr::null_mut(), 0, &mut size);
            }
            assert!(size > 0);
            let mut result = vec![0; size as usize];
            assert_ne!(
                unsafe { GetFileSecurityW(path.as_ptr(), 4, result.as_mut_ptr(), size, &mut size) },
                0
            );
            result
        }
        let temp = tempfile::tempdir().unwrap();
        let oldpath = temp.path().join("old.txt");
        let old = tempfile::NamedTempFile::new_in(temp.path())
            .unwrap()
            .persist(&oldpath)
            .unwrap();
        for name in ["short.txt", "中文😀文件.txt", "odd-name-length.bin"] {
            let path = temp.path().join(name);
            let mut output = output_temp(temp.path()).unwrap();
            let scratch = output.path().to_path_buf();
            output.write_all(b"verified").unwrap();
            assert!(
                publish_output_by_handle(&mut output, &path, false),
                "native handle API must be exercised"
            );
            let file = output.into_file();
            assert!(!scratch.exists());
            assert_eq!(fs::read(&path).unwrap(), b"verified");
            assert_eq!(
                file.metadata().unwrap().file_attributes(),
                old.metadata().unwrap().file_attributes()
            );
            assert_eq!(dacl(&path), dacl(&oldpath));
            let mut collision = output_temp(temp.path()).unwrap();
            let scratch = collision.path().to_path_buf();
            collision.write_all(b"must-not-replace").unwrap();
            assert!(!publish_output_by_handle(&mut collision, &path, false));
            drop(collision);
            assert!(!scratch.exists());
            assert_eq!(fs::read(&path).unwrap(), b"verified");
            drop(file);
        }
        let path = temp.path().join("fallback.txt");
        let mut without_delete = tempfile::NamedTempFile::new_in(temp.path()).unwrap();
        without_delete.write_all(b"fallback").unwrap();
        assert!(!publish_output_by_handle(&mut without_delete, &path, false));
        drop(publish_output(without_delete, &path, false).unwrap());
        assert_eq!(fs::read(path).unwrap(), b"fallback");
        let target = temp.path().join("readonly.txt");
        fs::write(&target, b"preserve").unwrap();
        let mut perm = fs::metadata(&target).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&target, perm).unwrap();
        assert!(publish_output(output_temp(temp.path()).unwrap(), &target, true).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"preserve");
        let mut perm = fs::metadata(&target).unwrap().permissions();
        perm.set_readonly(false);
        fs::set_permissions(&target, perm).unwrap();
        let newpath = temp.path().join("metadata.txt");
        let published = publish_output(output_temp(temp.path()).unwrap(), &newpath, false).unwrap();
        let entry = Entry {
            name: "metadata.txt".into(),
            size: 0,
            packed_size: 0,
            is_dir: false,
            crc: String::new(),
            encrypted: false,
            modified: "2020-01-02 03:04:05".into(),
            method: String::new(),
            comment: String::new(),
            attributes: "R".into(),
        };
        restore_metadata(&oldpath, &entry).unwrap();
        restore_open_file_metadata(&published, &entry).unwrap();
        assert_eq!(
            fs::metadata(&oldpath).unwrap().modified().unwrap(),
            fs::metadata(&newpath).unwrap().modified().unwrap()
        );
        assert_eq!(
            fs::metadata(&oldpath).unwrap().file_attributes(),
            fs::metadata(&newpath).unwrap().file_attributes()
        );
        for path in [&oldpath, &newpath] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).unwrap();
        }
    }
    #[test]
    fn bounded_zip_unknown_extra_subfields_are_not_fast_path_eligible() {
        let mut ntfs = vec![0x0a, 0, 32, 0];
        ntfs.extend([0u8; 32]);
        ntfs[8] = 1;
        ntfs[10] = 24;
        assert!(ordinary_zip_extra(&ntfs));
        ntfs[8] = 2;
        assert!(!ordinary_zip_extra(&ntfs));
        assert!(ordinary_zip_extra(&[0x55, 0x54, 5, 0, 1, 0, 0, 0, 0]));
        assert!(!ordinary_zip_extra(&[0x55, 0x54, 5, 0, 7, 0, 0, 0, 0]));
        assert!(!ordinary_zip_extra(&[0x75, 0x78, 5, 0, 2, 1, 0, 1, 0]));
    }
    #[cfg(windows)]
    fn old_no_links(path: &Path) -> Result<(), String> {
        use std::os::windows::fs::MetadataExt;
        for p in path.ancestors() {
            match fs::symlink_metadata(p) {
                Ok(m) if m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 => {
                    return Err(format!("linked output path rejected: {}", p.display()));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(())
    }
    #[cfg(windows)]
    #[test]
    fn checked_directories_pin_ancestors_and_allow_child_publication() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let parent = t.path().join("parent");
        let child = parent.join("child");
        fs::create_dir_all(&child).unwrap();
        let mut guard = CheckedDirectories::default();
        guard.check(&child.join("not-yet-created")).unwrap();
        assert!(guard.pinned(&parent));
        assert!(guard.pinned(&child));
        assert!(fs::rename(&parent, t.path().join("replaced")).is_err());
        assert!(fs::rename(&child, parent.join("replaced")).is_err());
        let mut temp = tempfile::NamedTempFile::new_in(&child).unwrap();
        std::io::Write::write_all(&mut temp, b"published").unwrap();
        guard.allow_publication(&child).unwrap();
        temp.persist_noclobber(child.join("published.txt")).unwrap();
        fs::write(child.join("ordinary.txt"), b"child creation still works").unwrap();
        drop(guard);
        let writer = fs::OpenOptions::new()
            .access_mode(0x40000000)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(&child)
            .unwrap();
        drop(writer);
        fs::rename(&parent, t.path().join("replaced")).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn checked_directories_failed_lock_never_caches_and_rechecks_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("replaceable");
        let other = t.path().join("other");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&other).unwrap();
        let writer = fs::OpenOptions::new()
            .access_mode(0x40000000)
            .share_mode(2)
            .custom_flags(0x02200000)
            .open(&dir)
            .unwrap();
        let mut guard = CheckedDirectories::default();
        guard.check(&dir).unwrap();
        assert!(!guard.pinned(&dir));
        drop(writer);
        guard.allow_publication(t.path()).unwrap();
        fs::rename(&dir, t.path().join("old")).unwrap();
        let result = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&dir)
            .arg(&other)
            .output()
            .unwrap();
        assert!(result.status.success());
        assert!(guard.check(&dir.join("missing").join("child")).is_err());
        fs::remove_dir(&dir).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn checked_directory_handle_rechecks_in_place_reparse_change() {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
        #[link(name = "Kernel32")]
        extern "system" {
            fn DeviceIoControl(
                handle: *mut std::ffi::c_void,
                code: u32,
                input: *const u8,
                input_size: u32,
                output: *mut u8,
                output_size: u32,
                returned: *mut u32,
                overlapped: *mut std::ffi::c_void,
            ) -> i32;
        }
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("checked");
        let target = t.path().join("target");
        fs::create_dir(&dir).unwrap();
        fs::create_dir(&target).unwrap();
        let mut guard = CheckedDirectories::owned_root(&dir);
        guard.check(&dir).unwrap();
        assert!(guard.pinned(&dir));
        let target = fs::canonicalize(target).unwrap();
        let printable = target
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned();
        let substitute = format!(r"\??\{printable}")
            .encode_utf16()
            .collect::<Vec<_>>();
        let printed = std::ffi::OsStr::new(&printable)
            .encode_wide()
            .collect::<Vec<_>>();
        let sublen = (substitute.len() * 2) as u16;
        let printlen = (printed.len() * 2) as u16;
        let mut data = Vec::new();
        data.extend(0xA0000003u32.to_le_bytes());
        data.extend((8 + sublen + 2 + printlen + 2).to_le_bytes());
        data.extend(0u16.to_le_bytes());
        for v in [0u16, sublen, sublen + 2, printlen] {
            data.extend(v.to_le_bytes());
        }
        for v in substitute
            .into_iter()
            .chain(Some(0))
            .chain(printed)
            .chain(Some(0))
        {
            data.extend(v.to_le_bytes());
        }
        assert!(
            fs::OpenOptions::new()
                .access_mode(0x40000000)
                .share_mode(7)
                .custom_flags(0x02200000)
                .open(&dir)
                .is_err(),
            "strong directory must reject reparse write handle"
        );
        guard.allow_publication(&dir).unwrap();
        let writer = fs::OpenOptions::new()
            .access_mode(0x40000000)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(&dir)
            .unwrap();
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                writer.as_raw_handle(),
                0x000900A4,
                data.as_ptr(),
                data.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            ok,
            0,
            "junction insertion: {}",
            std::io::Error::last_os_error()
        );
        drop(writer);
        assert!(
            guard.check(&dir.join("missing")).is_err(),
            "held metadata must be re-read"
        );
        drop(guard);
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn published_handle_restores_same_timestamp_and_readonly_as_path() {
        let t = tempfile::tempdir().unwrap();
        let mut entries = parse_listing(
            b"Path = file\nSize = 3\nModified = 2021-02-03 04:05:06.125\nAttributes = R\n\n",
        )
        .unwrap();
        let entry = entries.remove(0);
        let original = t.path().join("path-metadata");
        fs::write(&original, b"abc").unwrap();
        restore_metadata(&original, &entry).unwrap();
        let target = t.path().join("handle-metadata");
        let mut temp = tempfile::NamedTempFile::new_in(t.path()).unwrap();
        std::io::Write::write_all(&mut temp, b"abc").unwrap();
        let file = temp.persist_noclobber(&target).unwrap();
        restore_open_file_metadata(&file, &entry).unwrap();
        let a = fs::metadata(&original).unwrap();
        let b = file.metadata().unwrap();
        assert_eq!(a.modified().unwrap(), b.modified().unwrap());
        assert!(b.permissions().readonly());
        assert_eq!(fs::read(&target).unwrap(), b"abc");
        drop(file);
        for path in [&original, &target] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).unwrap();
        }
    }
    #[cfg(windows)]
    #[test]
    fn shared_ancestor_checks_do_not_block_unrelated_temporary_publication() {
        let common = tempfile::tempdir().unwrap();
        let mut guard = CheckedDirectories::default();
        guard
            .create_dirs(&common.path().join("owned-output/child"))
            .unwrap();
        assert!(guard.pinned(common.path()));
        let temporary = tempfile::NamedTempFile::new_in(common.path()).unwrap();
        temporary
            .persist_noclobber(common.path().join("unrelated-sibling.txt"))
            .unwrap();
        assert!(common.path().join("unrelated-sibling.txt").is_file());
    }
    fn parallel_fixture(count: usize, bytes: usize) -> (tempfile::TempDir, Vec<Entry>) {
        let stage = tempfile::tempdir().unwrap();
        let mut listing = String::new();
        for i in 0..count {
            let name = format!("file-{i:04}.bin");
            fs::write(stage.path().join(&name), vec![(i % 251) as u8; bytes]).unwrap();
            listing.push_str(&format!(
                "Path = {name}\nSize = {bytes}\nModified = 2020-01-02 03:04:05\n\n"
            ));
        }
        let entries = parse_listing(listing.as_bytes()).unwrap();
        (stage, entries)
    }
    #[test]
    fn parallel_publication_writes_verified_bytes_and_no_temporary_files() {
        let (stage, entries) = parallel_fixture(160, 8193);
        let dest = tempfile::tempdir().unwrap();
        let expected = expected_tree(&entries).unwrap();
        verify_stage(stage.path(), &expected, &JobControl::new()).unwrap();
        let refs = entries.iter().collect::<Vec<_>>();
        let mut guards = CheckedDirectories::default();
        assert_eq!(
            publish_small_files_parallel(
                &refs,
                dest.path(),
                stage.path(),
                &JobControl::new(),
                &mut guards
            )
            .unwrap(),
            160
        );
        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(
                fs::read(dest.path().join(&entry.name)).unwrap_or_else(|e| panic!(
                    "{}: {} ; present {:?}",
                    dest.path().join(&entry.name).display(),
                    e,
                    fs::read_dir(dest.path())
                        .unwrap()
                        .map(|e| e.unwrap().file_name())
                        .collect::<Vec<_>>()
                )),
                vec![(i % 251) as u8; 8193]
            );
        }
        assert_eq!(fs::read_dir(dest.path()).unwrap().count(), 160);
    }
    #[test]
    fn parallel_publication_stops_and_joins_after_collision_and_worker_error() {
        let (stage, entries) = parallel_fixture(160, 8193);
        let refs = entries.iter().collect::<Vec<_>>();
        let dest = tempfile::tempdir().unwrap();
        fs::write(dest.path().join(&entries[0].name), b"keep original").unwrap();
        let mut guards = CheckedDirectories::default();
        assert!(publish_small_files_parallel(
            &refs,
            dest.path(),
            stage.path(),
            &JobControl::new(),
            &mut guards
        )
        .is_err());
        assert_eq!(
            fs::read(dest.path().join(&entries[0].name)).unwrap(),
            b"keep original"
        );
        let files = fs::read_dir(dest.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        assert!(files.len() < 160);
        assert!(files
            .iter()
            .all(|f| f.to_string_lossy().starts_with("file-")));
        let failed = tempfile::tempdir().unwrap();
        fs::remove_file(stage.path().join(&entries[0].name)).unwrap();
        assert!(publish_small_files_parallel(
            &refs,
            failed.path(),
            stage.path(),
            &JobControl::new(),
            &mut guards
        )
        .is_err());
        let count = fs::read_dir(failed.path()).unwrap().count();
        assert!(count < 160);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(fs::read_dir(failed.path()).unwrap().count(), count);
        assert!(fs::read_dir(failed.path()).unwrap().all(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("file-")));
    }
    #[test]
    fn parallel_publication_running_cancel_waits_for_all_workers() {
        let (stage, entries) = parallel_fixture(512, 64 * 1024);
        let refs = entries.iter().collect::<Vec<_>>();
        let dest = tempfile::tempdir().unwrap();
        let ctl = JobControl::new();
        let done = AtomicBool::new(false);
        let mut guards = CheckedDirectories::default();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    if ctl.snapshot().phase == "extracting" {
                        ctl.cancel();
                        break;
                    }
                    std::thread::yield_now();
                }
            });
            let result =
                publish_small_files_parallel(&refs, dest.path(), stage.path(), &ctl, &mut guards);
            done.store(true, Ordering::Release);
            result
        });
        assert!(result.unwrap_err().contains("cancelled"));
        let count = fs::read_dir(dest.path()).unwrap().count();
        assert!(count > 0 && count < 512);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(fs::read_dir(dest.path()).unwrap().count(), count);
        assert!(fs::read_dir(dest.path()).unwrap().all(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("file-")));
    }
    #[cfg(windows)]
    #[test]
    fn parallel_publication_rejects_output_junction_without_external_writes() {
        let (stage, mut entries) = parallel_fixture(16, 10);
        let dest = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        for entry in &mut entries {
            entry.name = format!("linked/{}", entry.name);
        }
        let link = dest.path().join("linked");
        let result = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(external.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        let refs = entries.iter().collect::<Vec<_>>();
        let mut guards = CheckedDirectories::default();
        assert!(publish_small_files_parallel(
            &refs,
            dest.path(),
            stage.path(),
            &JobControl::new(),
            &mut guards
        )
        .is_err());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
        drop(guards);
        fs::remove_dir(link).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn parallel_publication_handles_case_alias_directory_names() {
        let (stage, mut entries) = parallel_fixture(128, 10);
        fs::create_dir(stage.path().join("Dir")).unwrap();
        for entry in &mut entries {
            fs::rename(
                stage.path().join(&entry.name),
                stage.path().join("Dir").join(&entry.name),
            )
            .unwrap();
            entry.name = format!("dir/{}", entry.name);
        }
        let mut directory = parse_listing(b"Path = Dir\nSize = 0\nFolder = +\n\n").unwrap();
        entries.insert(0, directory.remove(0));
        let expected = expected_tree(&entries).unwrap();
        verify_stage(stage.path(), &expected, &JobControl::new()).unwrap();
        let dest = tempfile::tempdir().unwrap();
        let refs = entries.iter().collect::<Vec<_>>();
        let mut guard = CheckedDirectories::default();
        assert_eq!(
            publish_small_files_parallel(
                &refs,
                dest.path(),
                stage.path(),
                &JobControl::new(),
                &mut guard
            )
            .unwrap(),
            128
        );
        assert_eq!(fs::read_dir(dest.path().join("Dir")).unwrap().count(), 128);
    }
    #[cfg(windows)]
    #[test]
    fn attribute_fastpath_is_limited_to_unambiguous_disk_paths() {
        assert!(attribute_query_path(Path::new(r"C:\plain\file.txt")).is_some());
        assert!(attribute_query_path(Path::new(r"\\?\C:\plain\file.txt")).is_some());
        for path in [
            r"relative\file",
            r"C:relative",
            r"\rooted",
            r"\\server\share\file",
            r"\\?\UNC\server\share\file",
            r"\\.\PhysicalDrive0",
            r"C:\a\..\file",
            r"C:\a\file.",
            r"C:\a\file:stream",
            r"C:\NUL",
            r"C:/plain/file",
        ] {
            assert!(attribute_query_path(Path::new(path)).is_none(), "{path}");
        }
        let long = format!("C:\\{}\\file", "long\\".repeat(60));
        assert!(attribute_query_path(Path::new(&long)).is_none());
        assert!(attribute_query_path(Path::new(&format!(r"\\?\{long}"))).is_some());
    }
    #[cfg(windows)]
    #[test]
    fn attribute_fastpath_matches_metadata_on_controlled_fixtures() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("ordinary");
        fs::create_dir(&dir).unwrap();
        let file = dir.join("file.txt");
        fs::write(&file, b"fixture").unwrap();
        let missing = dir.join("missing").join("leaf");
        assert_eq!(fast_reparse_attribute(&missing), None);
        for path in [&dir, &file, &missing] {
            assert_eq!(no_links(path), old_no_links(path));
        }
        let mut deep = dir.clone();
        for _ in 0..35 {
            deep.push("deep-folder");
        }
        fs::create_dir_all(&deep).unwrap();
        // Ordinary long paths fall back; std alone supplies their normalization.
        let ordinary = PathBuf::from(deep.to_string_lossy().trim_start_matches(r"\\?\"));
        assert!(attribute_query_path(&ordinary).is_none());
        assert_eq!(no_links(&ordinary), old_no_links(&ordinary));
        let verbatim = fs::canonicalize(&deep).unwrap();
        assert_eq!(fast_reparse_attribute(&verbatim), Some(false));
        assert_eq!(no_links(&verbatim), old_no_links(&verbatim));
    }
    #[cfg(windows)]
    #[test]
    fn attribute_fastpath_rejects_junction_at_every_ancestor_depth() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("target");
        let link = t.path().join("junction");
        fs::create_dir(&target).unwrap();
        let result = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fast_reparse_attribute(&link), Some(true));
        let mut path = link.clone();
        for _ in 0..8 {
            let new = no_links(&path);
            assert!(new
                .as_ref()
                .unwrap_err()
                .contains("linked output path rejected"));
            assert_eq!(new, old_no_links(&path));
            path.push("missing-child");
        }
        fs::remove_dir(&link).unwrap();
        assert!(target.is_dir());
    }
    #[test]
    fn normalized_aliases_and_file_parent_conflicts_are_rejected() {
        assert!(parse_listing(b"Path = ./a\nSize = 1\n\nPath = a\nSize = 1\n\n").is_err());
        for bytes in [
            b"Path = a\nSize = 1\n\nPath = a/b\nSize = 1\n\n".as_slice(),
            b"Path = a/b\nSize = 1\n\nPath = a\nSize = 1\n\n".as_slice(),
        ] {
            assert!(expected_tree(&parse_listing(bytes).unwrap()).is_err());
        }
    }
    #[test]
    fn staging_tree_must_match_manifest_before_publication() {
        let entries = parse_listing(b"Path = dir/file\nSize = 3\n\n").unwrap();
        let expected = expected_tree(&entries).unwrap();
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("dir")).unwrap();
        fs::write(t.path().join("dir/file"), b"abc").unwrap();
        verify_stage(t.path(), &expected, &JobControl::new()).unwrap();
        fs::write(t.path().join("surprise"), b"abc").unwrap();
        assert!(verify_stage(t.path(), &expected, &JobControl::new()).is_err());
        fs::remove_file(t.path().join("surprise")).unwrap();
        fs::write(t.path().join("dir/file"), b"shortened or enlarged").unwrap();
        assert!(verify_stage(t.path(), &expected, &JobControl::new()).is_err());
    }
}
