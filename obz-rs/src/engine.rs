//! Multi-format operations through bundled NanaZip with official 7-Zip fallback.
//! No shell is used. Extraction stages preflighted archives before publishing files.
#[cfg(nana_selected_worker)]
pub mod selected_worker;
#[cfg(windows)]
mod created_directories;
mod bounded_archive_stage;
mod zip_volume_view;
#[cfg(windows)]
pub(crate) mod edit_source;
#[cfg(windows)]
pub(crate) fn recover_interrupted_edit(path:&std::path::Path)->Result<bool,String>{ edit_source::recover_missing(path) }
mod zip_preview;
pub use zip_preview::ZipPreviewBatch;
#[cfg(windows)]
pub use crate::engine_stream::canonical_input_path;
pub(crate) use zip_volume_view::with_source as with_zip_source;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

/// Evidence for reusing plaintext previews between separately pinned reads.
#[cfg(windows)]
#[derive(PartialEq, Eq)]
pub enum PreviewRevision {
    Journal(u64, i64),
    Digest([u8; 32]),
}

#[cfg(windows)]
impl PreviewRevision {
    /// The caller holds a read handle that denies writers for this request.
    pub fn read(file: &File, index: u64) -> Option<Self> {
        if let Some((journal, usn)) = created_directories::read_revision(file, index) {
            return Some(Self::Journal(journal, usn));
        }
        // Bound the cost of cache validation on volumes without a usable journal.
        if file.metadata().ok()?.len() > 1024 * 1024 {
            return None;
        }
        use sha2::{Digest, Sha256};
        let mut input = file;
        let mut digest = Sha256::new();
        input.rewind().ok()?;
        let mut buffer = [0u8; 8192];
        loop {
            let count = input.read(&mut buffer).ok()?;
            if count == 0 { break; }
            digest.update(&buffer[..count]);
        }
        Some(Self::Digest(digest.finalize().into()))
    }
}

// Opt-in diagnostic timing; no paths, passwords or archive names are logged.
struct PhaseTimer(&'static str, Option<std::time::Instant>);
impl PhaseTimer {
    fn enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(||std::env::var_os("OPEN_ARCHIVE_PROFILE").is_some_and(|v|v=="1"))
    }
    fn new(label: &'static str) -> Self {
        Self(label, Self::enabled().then(std::time::Instant::now))
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
    pub dictionary_bytes: Option<u64>,
    pub solid: Option<bool>,
    pub header_encryption: Option<bool>,
    pub volume_bytes: Option<u64>,
    pub selected: Vec<String>,
    /// error (default), overwrite, skip, rename.
    pub overwrite: String,
    #[serde(skip)]
    pub control: Option<JobControl>,
    #[serde(skip)]
    pub zone_identifier: Option<crate::motw::ZoneIdentifier>,
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
            dictionary_bytes: None,
            solid: None,
            header_encryption: None,
            volume_bytes: None,
            selected: Vec::new(),
            overwrite: "error".into(),
            control: None,
            zone_identifier: None,
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
fn known_entry_size() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    /// False for formats whose decoded length is unavailable before decoding.
    #[serde(default = "known_entry_size")]
    pub size_known: bool,
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
    pub backend: String,
    pub version: String,
    pub available: bool,
    pub read_formats: Vec<String>,
    pub write_formats: Vec<String>,
    pub features: Vec<String>,
    pub limitations: Vec<String>,
}

// Keep ARM64 validation side by side with the distributed x64 runtime.
// Neither the policy verifier nor the isolated worker trusts an on-disk manifest.
#[cfg(target_arch = "aarch64")]
pub const BUNDLED_NANA_DIR: &str = "runtime/nanazip-arm64";
#[cfg(not(target_arch = "aarch64"))]
pub const BUNDLED_NANA_DIR: &str = "runtime/nanazip";
#[cfg(target_arch = "aarch64")]
pub(crate) const NANA_PROVENANCE: &str = include_str!("../native/nana/provenance-arm64.json");
#[cfg(not(target_arch = "aarch64"))]
pub(crate) const NANA_PROVENANCE: &str = include_str!("../../runtime/nanazip/provenance.json");

pub fn executable() -> Result<PathBuf, String> {
    let policy = crate::machine_policy::scope();
    if policy.restricted() { return policy.official_executable(); }
    unrestricted_executable()
}

fn unrestricted_executable() -> Result<PathBuf, String> {
    let mut paths = Vec::new();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let current = std::env::current_exe().ok();
    let adjacent = current.as_deref().and_then(Path::parent);
    for dir in adjacent.into_iter().chain(std::iter::once(workspace)) {
        paths.push(dir.join(BUNDLED_NANA_DIR).join("NanaZip.Universal.Console.exe"));
        paths.push(dir.join("runtime/7zip/7z.exe"));
    }
    if let Some(p) = std::env::var_os("ProgramFiles") {
        paths.push(PathBuf::from(p).join("7-Zip/7z.exe"));
    }
    if cfg!(windows) {
        paths.push(PathBuf::from("C:/Program Files/7-Zip/7z.exe"));
    }
    paths
        .into_iter()
        .find(|p| usable_engine(p))
        .ok_or_else(|| format!("archive engine not found; restore {BUNDLED_NANA_DIR} or install 7-Zip"))
}

#[cfg(not(all(windows, target_arch = "aarch64")))]
fn usable_engine(path: &Path) -> bool { path.is_file() }

#[cfg(all(windows, target_arch = "aarch64"))]
fn usable_engine(path: &Path) -> bool {
    // Windows on Arm can start an x64 console under emulation. The selected
    // native worker cannot load x64 DLLs, so only select an ARM64 console.
    let Ok(mut file) = File::open(path) else { return false };
    let mut dos = [0; 64];
    if file.read_exact(&mut dos).is_err() || &dos[..2] != b"MZ" { return false; }
    let pos = u32::from_le_bytes(dos[60..].try_into().unwrap());
    let mut header = [0; 6];
    file.seek(SeekFrom::Start(pos.into())).is_ok() && file.read_exact(&mut header).is_ok()
        && header == *b"PE\0\0\x64\xaa"
}

// Hash requests share the bounded subprocess output, cancellation and password
// redaction contract; only hash and codec discovery are accepted internally.
pub(crate) fn run_hash(args: &[String], opts: &EngineOptions) -> Result<Vec<u8>, String> {
    if !matches!(args.first().map(String::as_str), Some("h" | "i")) {
        return Err("hash or codec discovery command required".into());
    }
    run_program(unrestricted_executable()?, args, None, opts, None, false)
}

fn sfx_stub() -> Result<PathBuf, String> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("runtime/7zip/7z.sfx"));
        }
    }
    paths.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/7zip/7z.sfx"));
    paths
        .into_iter()
        .find(|p| usable_engine(p))
        .ok_or("matching 7z.sfx runtime missing".into())
}
pub fn capabilities() -> Capabilities {
    let executable = executable();
    let extended = executable.as_ref().is_ok_and(|p| {
        p.file_name()
            .is_some_and(|name| name == "NanaZip.Universal.Console.exe")
    });
    let extended_read_formats: &[&str] = if extended {
        &["wasm", "romfs", "ufs", "ufs2", "ZealFS v1 (direct image)", ".NET single-file bundle (uncompressed)"]
    } else { &[] };
    Capabilities { engine:executable.as_ref().map(|p|p.display().to_string()).unwrap_or_default(), available:executable.is_ok(),
      backend:if extended {"NanaZip"} else {"7-Zip"}.into(),
      version:if extended {"7.0.1882.0 (core 26.05)"} else {"bundled 26.05 or installed fallback"}.into(),
      read_formats:["7z","zip","zipx (supported methods)","rar","rar5","tar","gzip","bzip2","xz","zstd","iso","udf","cab","wim","msi","nsis","arj","lzh","deb","rpm","dmg","vhd","vhdx","sfx","split"].iter().chain(extended_read_formats.iter()).map(|s|s.to_string()).collect(),
      write_formats:["zip","7z","tar","gzip","bzip2","xz","wim","sfx","tgz","tbz2","txz","zipx"].iter().map(|s|s.to_string()).collect(),
      features:["AES-256 ZIP/7z","optional encrypted 7z headers","split volumes","ZIP64","compression/extraction thread limits","7z dictionary and solid settings","selected extraction","archive integrity test","add/delete/rename","bounded preview","cancel/progress","codepage override","overwrite policies","source Zone.Identifier propagation"].iter().map(|s|s.to_string()).collect(),
      limitations:["RAR/LZH creation and arbitrary ZIPX codecs are unsupported by this engine; separate adapters handle some legacy formats","Selected extraction verifies all entries. Eligible ZIPs discard unselected data after verification. On Windows x64 or ARM64 with the matching NanaZip runtime, eligible split 7z archives also stage only selected files for unencrypted non-solid Copy, or single-coder LZMA and LZMA2 (solid or non-solid, optionally with 7z AES). Other layouts may require full temporary staging","Full ZIP/7z extraction and eligible selected split 7z extraction may commit verified same-volume staging for new directories or move verified files into existing directories. Existing-directory moves require known sizes, serial publication, sufficient fallback space, ordinary NTFS storage/ACLs and exact metadata; unsupported cases retain copy publication","Archive symbolic links, hard links and alternate streams are rejected","NanaZip extended codecs require the bundled NanaZip runtime; official 7-Zip fallback and SFX support a smaller method set. Codec discovery does not imply every archive variant or encryption write method is supported","WASM, ROMFS, UFS and .NET bundle support requires bundled NanaZip. Verified .NET single-file bundle format v6: uncompressed entries can be extracted; compressed bundles are listing-only; standalone integrity test is unsupported for both","Passwords reach the main archive engine through an anonymous input pipe, not command arguments"].iter().map(|s|s.to_string()).collect() }
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
    if opts.level.is_some_and(|v| v > 49) {
        return Err("compression level cannot exceed 49".into());
    }
    if opts.threads == Some(0) || opts.volume_bytes == Some(0)
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
        crate::engine_password::validate(p)?;
        if crate::engine_password::needs_prompt_switch(command) {
            a.push("-p".into());
        }
    }
    if let Some(cp) = opts.codepage {
        a.push(format!("-mcp={cp}"));
    }
    // Creation already appends this in its format-specific argument builder.
    // Extraction/testing must pass the user's limit to the external decoder too.
    if matches!(command, "x" | "e" | "t") {
        if let Some(threads) = opts.threads { a.push(format!("-mmt={threads}")); }
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
    run_program(executable()?, args, cwd, opts, output, false)
}
// Only a pinned source's type probe and a successfully guarded direct image
// may opt into the incomplete handler. This is never a user-supplied option.
fn run_with_zealfs(args: &[String], cwd: Option<&Path>, opts: &EngineOptions, output: Option<File>) -> Result<Vec<u8>, String> {
    run_program(executable()?, args, cwd, opts, output, true)
}
fn has_zealfs_handler(program: &Path) -> bool {
    program.file_name().is_some_and(|name| name.eq_ignore_ascii_case("NanaZip.Universal.Console.exe"))
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
    allow_zealfs: bool,
) -> Result<Vec<u8>, String> {
    read_engine_output(run_program_log(program, args, cwd, opts, output, allow_zealfs)?)
}

const OUTPUT_LIMIT: u64 = 64 * 1024 * 1024;

fn read_engine_output(file: File) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > OUTPUT_LIMIT { return Err("engine listing exceeds supported limit".into()); }
    Ok(bytes)
}

fn run_program_log(
    program: PathBuf,
    args: &[String],
    cwd: Option<&Path>,
    opts: &EngineOptions,
    output: Option<File>,
    allow_zealfs: bool,
) -> Result<File, String> {
    run_program_log_with_directory(program, args, cwd, opts, output, allow_zealfs, false)
        .map(|(log, _)| log)
}
fn run_program_log_with_directory(
    program: PathBuf,
    args: &[String],
    cwd: Option<&Path>,
    opts: &EngineOptions,
    output: Option<File>,
    allow_zealfs: bool,
    collect_directory: bool,
) -> Result<(File, Option<Vec<Entry>>), String> {
    let _timer = PhaseTimer::new("engine-process");
    let archive_command = !matches!(args.first().map(String::as_str), Some("h" | "i"));
    let policy = archive_command.then(crate::machine_policy::scope);
    if let Some(policy) = &policy {
        if policy.restricted() && program != policy.official_executable()? {
            return Err("machine format policy requires the verified NanaZip executable".into());
        }
    }
    let ctl = control(opts);
    ctl.check()?;
    if let Some(password) = &opts.password { crate::engine_password::validate(password)?; }
    let mut logs = tempfile::tempfile().map_err(|e| format!("cannot create engine output log: {e}"))?;
    let mut errors = tempfile::NamedTempFile::new().map_err(|e| format!("cannot create engine progress log: {e}"))?;
    let pipe = collect_directory.then(directory_capture::pipe).and_then(Result::ok);
    let (stdout, capture_files) = if let Some((reader, writer)) = pipe {
        (writer, Some((reader, logs.try_clone().map_err(|e| e.to_string())?)))
    } else {
        let stdout = match output {
            Some(file) => file,
            None => logs.try_clone().map_err(|e| e.to_string())?,
        };
        (stdout, None)
    };
    let mut guarded_args = args.to_vec();
    if !allow_zealfs && has_zealfs_handler(&program) && policy.as_ref().is_none_or(|p| p.handler_may_be_enabled("ZealFS"))
        && matches!(args.first().map(String::as_str), Some("l" | "t" | "x" | "e")) {
        guarded_args.insert(1, "-stxZealFS".into());
    }
    let mut cmd = Command::new(program);
    cmd.args(&guarded_args)
        .stdin(if opts.password.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(errors.as_file().try_clone().map_err(|e| e.to_string())?));
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
    #[cfg(test)]
    crate::machine_policy::child_started(cmd.get_program(), args.first().map(String::as_str).unwrap_or(""));
    // Retain the normal exclusive, delete-on-close diagnostic spool. No
    // pathname or shareable temporary file is introduced for this optimization.
    drop(cmd);
    let mut capture = if let Some((pipe, spool)) = capture_files {
        match DirectoryCapture::start(pipe, spool, ctl.clone()) {
            Ok(capture) => Some(capture),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    } else { None };
    let password_input = match opts.password.as_deref() {
        Some(password) => match crate::engine_password::PasswordInput::spawn(&mut child, password) {
            Ok(input) => Some(input),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        },
        None => None,
    };
    let mut progress_reader = File::open(errors.path()).ok();
    let status = loop {
        if ctl.check().is_err() {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(input) = password_input { let _ = input.finish(); }
            return Err("operation cancelled".into());
        }
        if let Some(reader) = capture.as_mut().filter(|reader| reader.ended()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(reader.finish().err()
                .unwrap_or_else(|| "archive directory reader stopped unexpectedly".into()));
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
                    if let Some(input) = password_input { let _ = input.finish(); }
                    return Err(e.to_string());
                }
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(input) = password_input { let _ = input.finish(); }
                return Err(e.to_string());
            }
        }
    };
    let password_result = password_input.map(|input| input.finish()).unwrap_or(Ok(()));
    let parsed = match capture.as_mut() {
        Some(capture) => capture.finish()?,
        None => None,
    };
    use std::io::{Seek, SeekFrom};
    // Child writers share these file positions. Rewind only after the child
    // exits; the live progress reader above has an independent position.
    logs.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    if logs.metadata().map_err(|e| e.to_string())?.len() > OUTPUT_LIMIT {
        return Err("engine listing exceeds supported limit".into());
    }
    if !status.success() {
        let bytes = read_engine_output(logs)?;
        let errors = errors.as_file_mut();
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
    password_result?;
    if let Some(policy) = &policy { policy.unchanged()?; }
    if collect_directory { ctl.check()?; }
    Ok((logs, parsed))
}
/// Resolve a member name to the same safe relative path used during extraction.
pub fn validate_name(name: &str) -> Result<PathBuf, String> {
    if name.starts_with(['/', '\\']) {
        return Err("absolute archive path rejected".into());
    }
    if !name.split(['/', '\\']).any(|part| part == ".") {
        return crate::extract::relative_path(name);
    }
    let normalized = name
        .split(['/', '\\'])
        .filter(|part| *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    crate::extract::relative_path(&normalized)
}
fn parse_listing(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "engine returned invalid UTF-8")?;
    if text.contains("\r\r\n") {
        // A carriage return within a value immediately before CRLF has
        // different line-end semantics at a record boundary. Preserve those
        // historical semantics without copying ordinary LF or CRLF listings.
        let normalized = text.replace("\r\n", "\n");
        return parse_listing_lines(normalized.split("\n\n").flat_map(|block| {
            block.lines().map(Some).chain(std::iter::once(None))
        }));
    }
    parse_listing_lines(text.lines().map(|line| {
        if line.is_empty() { None } else { Some(line) }
    }).chain(std::iter::once(None)))
}

fn parse_listing_lines<'a>(lines: impl Iterator<Item = Option<&'a str>>) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    append_listing_lines(lines, &mut result, &mut seen)?;
    Ok(result)
}

fn append_listing_lines<'a>(
    lines: impl Iterator<Item = Option<&'a str>>,
    result: &mut Vec<Entry>,
    seen: &mut HashSet<String>,
) -> Result<(), String> {
    let mut props = listing_properties::Properties::default();
    for line in lines {
        if let Some(line) = line {
            if let Some((k, v)) = line.split_once(" = ") {
                if !props.insert(k, v) {
                    return Err("ambiguous archive entry metadata".into());
                }
            } else if !line.trim().is_empty() {
                return Err("unsupported archive filename or metadata".into());
            }
        } else {
            if let Some(name) = props.get(Field::Path) {
                if let Some(entry) = listing_entry(name, &props, seen)? {
                    result.push(entry);
                }
            }
            props.clear();
        }
    }
    Ok(())
}

mod directory_capture;
mod listing_properties;
use directory_capture::DirectoryCapture;
use listing_properties::Field;

// The technical listing is already in a private file. Parse complete records
// in batches so its whole text never coexists in memory with the decoded tree.
fn large_directory(log: &mut File, ctl: &JobControl) -> Result<Option<Vec<Entry>>, String> {
    const BATCH: usize = 128 * 1024;
    let mut reader = (&mut *log).take(OUTPUT_LIMIT + 1);
    let mut buffer = Vec::with_capacity(BATCH * 2);
    let mut block = vec![0; BATCH];
    let mut consumed = 0;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    loop {
        ctl.check()?;
        let count = reader.read(&mut block).map_err(|e| e.to_string())?;
        consumed += count as u64;
        if consumed > OUTPUT_LIMIT { return Err("engine listing exceeds supported limit".into()); }
        let start = buffer.len();
        buffer.extend_from_slice(&block[..count]);
        let end = if count == 0 {
            buffer.len()
        } else {
            let last_blank = (start..buffer.len()).rev().find(|&i| {
                buffer[i] == b'\n' && (buffer[..i].ends_with(b"\n") || buffer[..i].ends_with(b"\n\r"))
            });
            let Some(index) = last_blank else { continue; };
            index + 1
        };
        let parsed = std::str::from_utf8(&buffer[..end]).ok()
            .filter(|text| !text.contains("\r\r\n"))
            .is_some_and(|text| {
                let lines = text.lines().map(|line| (!line.is_empty()).then_some(line));
                append_listing_lines(lines.chain(std::iter::once(None)), &mut entries, &mut seen).is_ok()
            });
        if !parsed { break; }
        if count == 0 {
            if !entries.is_empty() { return Ok(Some(entries)); }
            break;
        }
        buffer.drain(..end);
    }
    // Preserve whole-list UTF-8 error precedence, unusual CR records and
    // anonymous stream headers by replaying the existing parser on fallback.
    log.rewind().map_err(|e| e.to_string())?;
    Ok(None)
}

fn listing_entry(
    name: &str,
    props: &listing_properties::Properties,
    seen: &mut HashSet<String>,
) -> Result<Option<Entry>, String> {
    if [Field::SymbolicLink, Field::HardLink, Field::AlternateStream]
        .into_iter()
        .any(|k| props.get(k).is_some_and(|s| !s.is_empty() && s != "-"))
    {
        return Err("archive links or alternate streams are unsupported".into());
    }
    let attrs = props.get(Field::Attributes).unwrap_or("");
    if attrs.starts_with('l') || attrs.contains(" lrwx") {
        return Err("archive symbolic link unsupported".into());
    }
    let is_dir = props.get(Field::Folder) == Some("+")
        || attrs.starts_with('D')
        || attrs.starts_with('d')
        || name.ends_with('/');
    if matches!(name, "." | "./") {
        if !is_dir {
            return Err("archive root must be a directory".into());
        }
        return Ok(None);
    }
    if !seen.insert(path_key(&validate_name(name)?)) {
        return Err("duplicate or case-colliding archive names".into());
    }
    let size = match props.get(Field::Size).filter(|value| !value.is_empty()) {
        Some(value) => Some(value.parse::<u64>().map_err(|_| "invalid archive entry size")?),
        None => None,
    };
    Ok(Some(Entry {
        size_known: is_dir || size.is_some(),
        name: name.into(),
        size: size.unwrap_or(0),
        packed_size: props
            .get(Field::PackedSize)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        is_dir,
        crc: props.get(Field::Crc).unwrap_or("").to_string(),
        encrypted: props.get(Field::Encrypted) == Some("+"),
        modified: props.get(Field::Modified).unwrap_or("").to_string(),
        method: props.get(Field::Method).unwrap_or("").to_string(),
        comment: props.get(Field::Comment).unwrap_or("").to_string(),
        attributes: attrs.to_string(),
    }))
}
fn unsupported_utc_listing_switch(error: &str) -> bool {
    // Older external engines can lack -slmu. Retry only the explicit command
    // line rejection of this one switch; never retry password/data/I/O errors.
    let Some(message) = error.strip_prefix("archive engine failed (7):") else {
        return false;
    };
    let Some((_, tail)) = message.split_once("Command Line Error:") else {
        return false;
    };
    let lines: Vec<_> = tail
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    matches!(
        lines.as_slice(),
        ["Unknown switch:" | "Unsupported switch:", "slmu" | "-slmu"]
    )
}

fn list_raw(path: &str, opts: &EngineOptions) -> Result<Vec<Entry>, String> {
    control(opts).check()?;
    let mut source = crate::engine_stream::pin(&absolute(path)?)?;
    read_directory(path, opts, &mut source, true).map(|(entries, _)| entries)
}
enum PayloadSource {
    Engine,
    OfficialZeal,
    VerifiedZeal(MemoryFiles),
}
fn list_raw_with_source(path: &str, opts: &EngineOptions, source: &mut File) -> Result<(Vec<Entry>, PayloadSource), String> {
    read_directory(path, opts, source, false)
}

fn read_directory(
    path: &str, opts: &EngineOptions, source: &mut File, overlap_allowed: bool,
) -> Result<(Vec<Entry>, PayloadSource), String> {
    let _timer = PhaseTimer::new("list-total");
    let ctl = control(opts);
    ctl.set("listing", None, "Reading archive directory");
    let raw_format = crate::stream_formats::raw_format(path);
    if raw_format.is_some() { crate::stream_formats::validate_raw_options(opts)?; }
    let probe = if raw_format.is_none() && crate::machine_policy::scope().handler_may_be_enabled("ZealFS") && has_zealfs_handler(&executable()?) {
        Some(crate::zealfs_guard::SourceProbe::read(source)?)
    } else { None };
    let candidate = probe.as_ref().is_some_and(|p| p.is_candidate());
    let mut args = base("l", opts)?;
    if let Some(format) = raw_format { args.push(format!("-t{format}")); }
    args.extend([
        "-slt".into(),
        // UTC avoids NanaZip's millisecond-only local FILETIME conversion and
        // preserves the archive's full timestamp precision, including DST folds.
        "-slmu".into(),
        "--".into(),
        absolute(path)?.display().to_string(),
    ]);
    if !candidate { args.insert(1, "-ba".into()); }
    let overlap = overlap_allowed && !candidate && opts.password.is_none() && cfg!(windows)
        && Path::new(path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        && directory_capture::large_zip_hint(source);
    let (mut log, live_entries) = run_technical_listing_with_directory(&mut args, opts, candidate, overlap)?;
    if let Some(entries) = live_entries {
        ctl.set("complete", Some(100), "Archive directory ready");
        return Ok((entries, PayloadSource::Engine));
    }
    if !candidate && raw_format.is_none() && opts.password.is_none()
        && log.metadata().map_err(|e| e.to_string())?.len() >= 1024 * 1024 {
        if let Some(entries) = large_directory(&mut log, &ctl)? {
            ctl.set("complete", Some(100), "Archive directory ready");
            return Ok((entries, PayloadSource::Engine));
        }
    }
    let listing = read_engine_output(log)?;
    let listing = if opts.password.is_some() { crate::engine_password::listing_payload(&listing) } else { &listing };
    let bare_listing;
    let listing = if candidate {
        let kind = crate::zealfs_guard::first_type(listing)?;
        if kind.eq_ignore_ascii_case("ZealFS") {
            let (headers, body) = crate::zealfs_guard::header_payload(listing, true)?;
            let image = crate::zealfs_guard::check(&headers, source, probe.as_ref().unwrap())?
                .ok_or("missing direct ZealFS header")?;
            let policy = crate::machine_policy::scope();
            policy.unchanged()?;
            if policy.restricted() {
                let entries = parse_listing(body)?;
                if entries.len() != image.entries.len()
                    || expected_tree(&entries)? != expected_tree(&image.entries)?
                {
                    return Err("official ZealFS listing omits entries under the current machine policy".into());
                }
                ctl.set("complete", Some(100), "Archive directory ready");
                return Ok((entries, PayloadSource::OfficialZeal));
            }
            let files = image.files.into_iter().map(|(name, bytes)| (name, MemoryChunk::Owned(bytes))).collect();
            ctl.check()?;
            ctl.set("complete", Some(100), "Archive directory ready");
            return Ok((image.entries, PayloadSource::VerifiedZeal(files)));
        } else {
            if kind == "Split" {
                let (headers, _) = crate::zealfs_guard::header_payload(listing, true)?;
                crate::zealfs_guard::check(&headers, source, probe.as_ref().unwrap())?;
            }
            // A Z-prefixed ZIP/LZMA/etc is not ZealFS just because of magic.
            // Reuse the original bare protocol, excluding the broken handler.
            // Never parse non-ZealFS archive comments as technical metadata.
            args.insert(1, "-ba".into());
            bare_listing = run(&args, None, opts, None)?;
            if opts.password.is_some() { crate::engine_password::listing_payload(&bare_listing) } else { &bare_listing }
        }
    } else { listing };
    let mut out = parse_listing(listing)?;
    if let Some(format) = raw_format {
        if out.is_empty() {
            let name = Path::new(path).file_stem().and_then(|name| name.to_str())
                .ok_or("invalid stream filename")?.to_owned();
            validate_name(&name)?;
            out.push(Entry {
                name, size: 0, size_known: false,
                packed_size: fs::metadata(path).map_err(|error| error.to_string())?.len(),
                is_dir: false, crc: String::new(), encrypted: false, modified: String::new(),
                method: format.into(), comment: String::new(), attributes: String::new(),
            });
        }
        if out.len() != 1 || out[0].is_dir { return Err("invalid raw stream directory".into()); }
        ctl.set("complete", Some(100), "Stream metadata ready");
        return Ok((out, PayloadSource::Engine));
    }
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
            size_known: true,
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
    Ok((out, PayloadSource::Engine))
}

fn run_technical_listing(args: &mut Vec<String>, opts: &EngineOptions, allow: bool) -> Result<File, String> {
    run_technical_listing_with_directory(args, opts, allow, false).map(|(log, _)| log)
}
fn run_technical_listing_with_directory(
    args: &mut Vec<String>, opts: &EngineOptions, allow_zealfs: bool, collect_directory: bool,
) -> Result<(File, Option<Vec<Entry>>), String> {
    let execute = |args: &[String]| run_program_log_with_directory(
        executable()?, args, None, opts, None, allow_zealfs, collect_directory,
    );
    match execute(args) {
        Err(error) if unsupported_utc_listing_switch(&error) => {
            args.retain(|argument| argument != "-slmu");
            execute(args)
        }
        result => result,
    }
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
pub(crate) use selection::Selection;
mod selection;

// Attribute queries avoid opening and closing a handle for every ancestor. Keep
// ambiguous namespaces and DOS normalization cases on std's metadata path.
#[cfg(windows)]
pub(crate) fn attribute_query_path(path: &Path) -> Option<Vec<u16>> {
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
pub(crate) fn directory_attributes(file: &File) -> Result<u32, String> {
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
    #[cfg(windows)]
    selected_output_root: Option<PathBuf>,
    #[cfg(windows)]
    created_output_dirs: Vec<(PathBuf, created_directories::Identity)>,
    // Only validated private stages qualify. Ordinary owned TempDirs may
    // inherit shared permissions and must keep full path checks.
    #[cfg(windows)]
    private_stages: HashSet<PathBuf>,
}
#[cfg(windows)]
impl Drop for CheckedDirectories {
    fn drop(&mut self) {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        if self.created_output_dirs.is_empty() { return; }
        self.held.clear();
        #[link(name = "Kernel32")]
        extern "system" {
            fn SetFileInformationByHandle(handle: *mut std::ffi::c_void, class: i32,
                info: *const std::ffi::c_void, size: u32) -> i32;
        }
        while let Some((path, created)) = self.created_output_dirs.pop() {
            // Match original ownership against this no-delete-sharing handle.
            // Windows also refuses to dispose of a directory that is not empty.
            let Ok(current) = fs::OpenOptions::new().access_mode(0x10080)
                .share_mode(3).custom_flags(0x02200000).open(path) else { continue };
            if !created.matches(&current)
                || !directory_attributes(&current).is_ok_and(|attrs| attrs & 0x410 == 0x10) { continue; }
            let delete: u8 = 1;
            unsafe { SetFileInformationByHandle(current.as_raw_handle(), 4, (&delete as *const u8).cast(), 1); }
        }
    }
}
impl CheckedDirectories {
    #[cfg(windows)]
    fn private_metadata_parent(&self, parent: &Path) -> Option<&File> {
        if !self.private_stages.iter().any(|root| parent.starts_with(root)) {
            return None;
        }
        self.held.get(parent).and_then(|(file, weak)| (!*weak).then_some(file))
    }
    fn owned_root(root: &Path) -> Self {
        let mut guard = Self::default();
        #[cfg(windows)]
        guard.owned_roots.push(root.to_path_buf());
        #[cfg(not(windows))]
        let _ = root;
        guard
    }
    #[cfg(windows)]
    fn create_owned_output_dir(&self, path: &Path) -> Option<Result<File, String>> {
        let parent = path.parent()?;
        let leaf = path.file_name()?;
        if let Some((pinned, _)) = self.held.get(parent) {
            return Some(crate::private_stage::create_output_directory(pinned, leaf));
        }
        // Beyond the directory-pin cache, open each missing ancestor relative
        // to the nearest retained one. Keep the whole short chain through CREATE.
        let (anchor_path, anchor) = parent.ancestors()
            .find_map(|ancestor| self.held.get(ancestor).map(|(file, _)| (ancestor, file)))?;
        let mut transient = Vec::new();
        for name in parent.strip_prefix(anchor_path).ok()?.iter() {
            let previous = transient.last().unwrap_or(anchor);
            match crate::private_stage::open_output_directory(previous, name) {
                Ok(file) => transient.push(file),
                Err(error) => return Some(Err(error)),
            }
        }
        Some(crate::private_stage::create_output_directory(transient.last().unwrap_or(anchor), leaf))
    }
    fn create_dirs(&mut self, path: &Path) -> Result<(), String> {
        for component in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            self.check(component)?;
            if self.pinned(component) {
                continue;
            }
            #[cfg(windows)]
            if self.selected_output_root.as_ref().is_some_and(|root| component.starts_with(root)) {
                if let Some(result) = self.create_owned_output_dir(component) {
                    match result {
                        Ok(created) => {
                            let proof = if self.created_output_dirs.len() < 4096 {
                                Some(created_directories::Identity::Handle(created))
                            } else {
                                created_directories::Stamp::capture(&created)
                                    .map(created_directories::Identity::Journal)
                            };
                            if let Some(proof) = proof {
                                self.created_output_dirs.push((component.to_owned(), proof));
                            }
                            self.owned_dirs.insert(component.to_owned());
                            self.check(component)?;
                            continue;
                        }
                        Err(_) if component.is_dir() => { self.check(component)?; continue; }
                        Err(error) => return Err(error),
                    }
                }
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
    verify_stage_with_metadata(root, expected, ctl, guards, None, None)
}
#[cfg(test)]
fn checked_staged_metadata(
    path: &Path,
    guards: &mut CheckedDirectories,
) -> Result<fs::Metadata, String> {
    checked_staged_metadata_with_relative(path, guards, false)
}
fn checked_staged_metadata_with_relative(
    path: &Path,
    guards: &mut CheckedDirectories,
    permit_relative: bool,
) -> Result<fs::Metadata, String> {
    let parent = path.parent().ok_or("missing staged parent")?;
    #[cfg(windows)]
    let relative = if permit_relative {
        // Unsupported native opens retain the original complete path checks.
        // A successfully opened reparse leaf is rejected below, never retried.
        guards.private_metadata_parent(parent)
            .and_then(|handle| crate::private_stage::child_metadata(handle, path.file_name().unwrap_or_default()).ok())
    } else { None };
    #[cfg(not(windows))]
    let relative: Option<fs::Metadata> = { let _ = permit_relative; None };
    // Always query the current leaf; DirEntry metadata may be an older Windows
    // enumeration snapshot and must not be used as reparse/size evidence.
    let meta = match relative {
        Some(meta) => meta,
        None => {
            guards.check(parent)?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                let file = fs::OpenOptions::new().access_mode(0x80).share_mode(7)
                    .custom_flags(0x02200000).open(path).map_err(|e| e.to_string())?;
                crate::private_stage::checked_metadata(&file)?
            }
            #[cfg(not(windows))]
            { fs::symlink_metadata(path).map_err(|e| e.to_string())? }
        }
    };
    let mut linked = meta.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        linked |= meta.file_attributes() & 0x400 != 0;
    }
    if linked {
        return Err("linked decoded path rejected".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.is_file() && meta.nlink() != 1 {
            return Err("hard linked decoded file rejected".into());
        }
    }
    Ok(meta)
}
// The caller holds directory guards for a verified private stage. Write the
// stream through a fresh non-reparse handle, before the file becomes visible.
fn mark_staged_file(
    path: &Path,
    meta: &fs::Metadata,
    zone: Option<&crate::motw::ZoneIdentifier>,
) -> Result<(), String> {
    if zone.is_none_or(|z| !z.is_marked()) {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        // WRITE_ATTRIBUTES works on readonly files. Deny DELETE continuously so
        // neither clearing readonly nor opening the data handle can follow a
        // swapped leaf. Attributes are changed through this pinned handle only.
        let pin = fs::OpenOptions::new()
            .access_mode(0x181)
            .share_mode(3)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|e| e.to_string())?;
        let current = pin.metadata().map_err(|e| e.to_string())?;
        if !current.is_file()
            || current.len() != meta.len()
            || current.file_attributes() & 0x400 != 0
        {
            return Err("linked or changed staged file rejected".into());
        }
        let original = current.permissions();
        if original.readonly() {
            let mut writable = original.clone();
            writable.set_readonly(false);
            pin.set_permissions(writable).map_err(|e| e.to_string())?;
        }
        let result = (|| {
            let file = fs::OpenOptions::new()
                .write(true)
                .share_mode(3)
                .custom_flags(0x00200000)
                .open(path)
                .map_err(|e| e.to_string())?;
            if directory_identity(&pin)? != directory_identity(&file)? {
                return Err("staged file changed before zone marking".into());
            }
            crate::motw::apply_to_file(&file, zone)
        })();
        if original.readonly() {
            pin.set_permissions(original).map_err(|e| e.to_string())?;
        }
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (path, meta);
        Ok(())
    }
}

fn verify_stage_with_metadata(
    root: &Path,
    expected: &BTreeMap<String, (bool, u64)>,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
    entries: Option<&[Entry]>,
    zone: Option<&crate::motw::ZoneIdentifier>,
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
            let meta = checked_staged_metadata_with_relative(&path, guards, zone.is_none_or(|z| !z.is_marked()))?;
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
            } else {
                if zone.is_some() { mark_staged_file(&path, &meta, zone)?; }
                if let Some(entry) = files.get(&key) {
                    normalize_staged_entry(&path, entry, &meta, ctl, guards, zone.is_some_and(|zone| zone.is_marked()))?;
                }
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
            let meta = checked_staged_metadata_with_relative(&path, guards, zone.is_none_or(|z| !z.is_marked()))?;
            normalize_staged_entry(&path, entry, &meta, ctl, guards, false)?;
        }
    }
    Ok(())
}

const MEMORY_ZIP_LIMIT: u64 = 64 * 1024 * 1024;
const MEMORY_ZIP_FILE_LIMIT: u64 = 8 * 1024 * 1024;
const MEMORY_ZIP_ENTRY_LIMIT: usize = 16_384;

// Hold this source across the independent listing and decoding. Failed locking
// or an ineligible container keeps the original decoder path.
fn locked_archive_source(path: &str, opts: &EngineOptions, limit: u64) -> Option<File> {
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
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    Some(file)
}
fn memory_archive_source(path: &str, opts: &EngineOptions) -> Option<File> {
    locked_archive_source(path, opts, MEMORY_ZIP_LIMIT)
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

// The same conservative ZIP32 layout accepted by the bounded-memory path, but
// inspect headers through a locked file rather than allocating the packed file.
// ZIP64, split/encrypted entries, descriptors and unknown extras keep the engine.
fn ordinary_zip_file_layout(
    source: &mut File,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<bool, String> {
    fn at(source: &mut File, offset: u64, len: usize) -> Result<Vec<u8>, String> {
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0; len];
        source.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
    let size = source.metadata().map_err(|e| e.to_string())?.len();
    if !(22..u32::MAX as u64).contains(&size) || entries.len() > MEMORY_ZIP_ENTRY_LIMIT {
        return Ok(false);
    }
    let tail_len = size.min(65557) as usize;
    let tail = at(source, size - tail_len as u64, tail_len)?;
    let Some(end) = (0..=tail.len() - 22).rev().find(|&i| {
        tail[i..i + 4] == *b"PK\x05\x06" && i + 22 + zip_u16(&tail, i + 20) as usize == tail.len()
    }) else {
        return Ok(false);
    };
    let eocd = &tail[end..];
    let central = zip_u32(eocd, 16) as u64;
    let length = zip_u32(eocd, 12) as usize;
    if zip_u16(eocd, 4) != 0
        || zip_u16(eocd, 6) != 0
        || zip_u16(eocd, 8) as usize != entries.len()
        || zip_u16(eocd, 10) as usize != entries.len()
        || length > 8 * 1024 * 1024
        || central + length as u64 != size - tail_len as u64 + end as u64
    {
        return Ok(false);
    }
    let directory = at(source, central, length)?;
    let mut cursor = 0;
    let mut ranges = Vec::with_capacity(entries.len());
    for _ in entries {
        ctl.check()?;
        if cursor + 46 > length || directory[cursor..cursor + 4] != *b"PK\x01\x02" {
            return Ok(false);
        }
        let c = &directory[cursor..];
        let name_len = zip_u16(c, 28) as usize;
        let extra_len = zip_u16(c, 30) as usize;
        let next = cursor + 46 + name_len + extra_len + zip_u16(c, 32) as usize;
        if next > length
            || zip_u16(c, 8) & !0x0806 != 0
            || !matches!(zip_u16(c, 10), 0 | 8)
            || zip_u16(c, 34) != 0
            || zip_u32(c, 20) == u32::MAX
            || zip_u32(c, 24) == u32::MAX
        {
            return Ok(false);
        }
        let local = zip_u32(c, 42) as u64;
        if local + 30 > central {
            return Ok(false);
        }
        let l = at(source, local, 30)?;
        if l[..4] != *b"PK\x03\x04" {
            return Ok(false);
        }
        let local_name = zip_u16(&l, 26) as usize;
        let local_extra = zip_u16(&l, 28) as usize;
        let data = local + 30 + local_name as u64 + local_extra as u64;
        let data_end = data + zip_u32(c, 20) as u64;
        if data_end > central || name_len != local_name || c[6..28] != l[4..26] {
            return Ok(false);
        }
        let fields = at(source, local + 30, local_name + local_extra)?;
        if c[46..46 + name_len] != fields[..local_name]
            || !ordinary_zip_extra(&c[46 + name_len..46 + name_len + extra_len])
            || !ordinary_zip_extra(&fields[local_name..])
        {
            return Ok(false);
        }
        ranges.push((local, data_end));
        cursor = next;
    }
    if cursor != length {
        return Ok(false);
    }
    ranges.sort_unstable();
    let mut previous = 0;
    for (start, end) in ranges {
        if start != previous {
            return Ok(false);
        }
        previous = end;
    }
    Ok(previous == central)
}

// Preserve full-archive CRC verification, but never write unselected bytes to
// disk. Selection is intentionally not passed to the decoder's verification.
fn decode_selected_zip(
    source: Option<File>,
    entries: &[Entry],
    opts: &EngineOptions,
    root: &Path,
    guards: &mut CheckedDirectories,
    ctl: &JobControl,
) -> Result<bool, String> {
    let Some(mut source) = source else {
        return Ok(false);
    };
    if opts.selected.is_empty() || !ordinary_zip_file_layout(&mut source, entries, ctl)? {
        return Ok(false);
    }
    source.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut archive = match zip::ZipArchive::new(source) {
        Ok(a) => a,
        Err(_) => return Ok(false),
    };
    if archive.offset() != 0
        || archive.len() != entries.len()
        || archive.has_duplicate_names()
        || archive.validate_header_consistency().is_err()
    {
        return Ok(false);
    }
    let expected = entries
        .iter()
        .map(|e| Ok((path_key(&validate_name(&e.name)?), e)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
    let mut names = Vec::with_capacity(entries.len());
    let mut seen = HashSet::new();
    // Finish qualification before creating any files; unsupported archives fall
    // back without mixing output from two decoders.
    for i in 0..archive.len() {
        ctl.check()?;
        let m = archive.by_index_data(i).map_err(|e| e.to_string())?;
        let Ok(name) = std::str::from_utf8(m.name_raw()) else {
            return Ok(false);
        };
        let key = path_key(&validate_name(name)?);
        let Some(e) = expected.get(&key) else {
            return Ok(false);
        };
        let kind = m.unix_mode().map(|m| m & 0xf000).unwrap_or(0);
        if !seen.insert(key.clone())
            || m.encrypted()
            || m.is_symlink()
            || !matches!(kind, 0 | 0x4000 | 0x8000)
            || (kind == 0x4000 && !e.is_dir)
            || (kind == 0x8000 && e.is_dir)
            || m.is_dir() != e.is_dir
            || m.size() != e.size
            || (e.is_dir && e.size != 0)
            || m.external_attributes() & 0x400 != 0
            || (!e.is_dir && u32::from_str_radix(&e.crc, 16).ok() != Some(m.crc32()))
        {
            return Ok(false);
        }
        names.push(key);
    }
    let _timer = PhaseTimer::new("verify-selected-zip-stream");
    ctl.set(
        "decoding",
        Some(0),
        "Verifying archive and staging selected files",
    );
    let mut buffer = vec![0u8; 1024 * 1024];
    let selection = Selection::new(&opts.selected);
    for (i, key) in names.into_iter().enumerate() {
        ctl.check()?;
        let e = expected[&key];
        let target = root.join(validate_name(&e.name)?);
        let keep = selection.contains(&e.name);
        let mut output = if keep && !e.is_dir {
            Some(create_staged_output(&target, guards)?)
        } else {
            None
        };
        if keep && e.is_dir {
            guards.create_dirs(&target)?;
        }
        let mut member = archive
            .by_index(i)
            .map_err(|e| format!("ZIP decode: {e}"))?;
        let mut hash = crc32fast::Hasher::new();
        let mut count = 0u64;
        loop {
            ctl.check()?;
            let n = member
                .read(&mut buffer)
                .map_err(|e| format!("ZIP integrity: {e}"))?;
            if n == 0 {
                break;
            }
            count = count
                .checked_add(n as u64)
                .ok_or("ZIP decoded size overflow")?;
            if count > e.size {
                return Err("ZIP decoded length mismatch".into());
            }
            hash.update(&buffer[..n]);
            if let Some(output) = output.as_mut() {
                std::io::Write::write_all(output, &buffer[..n]).map_err(|e| e.to_string())?;
            }
        }
        if count != e.size || hash.finalize() != member.crc32() {
            return Err("ZIP size or CRC mismatch".into());
        }
    }
    ctl.check()?;
    Ok(true)
}

fn verified_zip_order<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<Option<Vec<usize>>, String> {
    if archive.offset() != 0
        || archive.len() != entries.len()
        || archive.has_duplicate_names()
        || archive.validate_header_consistency().is_err()
    {
        return Ok(None);
    }
    let expected = entries
        .iter()
        .enumerate()
        .map(|(i, e)| Ok((path_key(&validate_name(&e.name)?), i)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
    let mut names = Vec::with_capacity(entries.len());
    let mut seen = vec![false; entries.len()];
    for i in 0..archive.len() {
        ctl.check()?;
        let member = archive.by_index_data(i).map_err(|e| e.to_string())?;
        let Ok(name) = std::str::from_utf8(member.name_raw()) else {
            return Ok(None);
        };
        let key = path_key(&validate_name(name)?);
        let Some(&index) = expected.get(&key) else {
            return Ok(None);
        };
        let entry = &entries[index];
        let kind = member.unix_mode().map(|m| m & 0xf000).unwrap_or(0);
        if std::mem::replace(&mut seen[index], true)
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
        names.push(index);
    }
    if seen.contains(&false) {
        return Ok(None);
    }
    Ok(Some(names))
}

fn decode_memory_zip(
    source: Option<File>,
    entries: &[Entry],
    opts: &EngineOptions,
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
    let Some(names) = verified_zip_order(&mut archive, entries, ctl)? else {
        return Ok(None);
    };
    ctl.set(
        "decoding",
        Some(0),
        "Verifying complete archive in bounded memory",
    );
    let mut contents = MemoryFiles::with_capacity(if opts.selected.is_empty() { files } else { opts.selected.len().min(files) });
    let selection = Selection::new(&opts.selected);
    let mut discard = Vec::new();
    let mut allocated = 0u64;
    for (i, index) in names.into_iter().enumerate() {
        ctl.check()?;
        let entry = &entries[index];
        let mut member = archive
            .by_index(i)
            .map_err(|e| format!("ZIP decode: {e}"))?;
        let keep = !entry.is_dir && selection.contains(&entry.name);
        let mut data = if keep { Vec::new() } else { std::mem::take(&mut discard) };
        data.clear();
        data.try_reserve_exact(entry.size as usize).map_err(|e| e.to_string())?;
        // Count retained payloads plus the largest reusable discard buffer.
        let resident = allocated.checked_add(data.capacity() as u64)
            .and_then(|n| n.checked_add(discard.capacity() as u64))
            .ok_or("ZIP memory budget exceeded")?;
        if resident > MEMORY_ZIP_LIMIT {
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
        if keep {
            allocated += data.capacity() as u64;
            contents.insert(path_key(&validate_name(&entry.name)?), MemoryChunk::Owned(data));
        } else {
            discard = data;
        }
    }
    ctl.check()?;
    Ok(Some(contents))
}

struct SevenMemoryPlan {
    data: Vec<u8>,
    parsed: crate::sevenz::MemoryArchive,
    order: Vec<usize>,
}

fn prepare_memory_sevenz(
    source: Option<File>,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<Option<SevenMemoryPlan>, String> {
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
        .enumerate()
        .map(|(i, e)| Ok((path_key(&validate_name(&e.name)?), i)))
        .collect::<Result<std::collections::HashMap<_, _>, String>>()?;
    let mut seen = vec![false; entries.len()];
    let mut names = Vec::new();
    for member in &parsed.archive.entries {
        ctl.check()?;
        let key = path_key(&validate_name(&member.name)?);
        let Some(&index) = expected.get(&key) else {
            return Ok(None);
        };
        let entry = &entries[index];
        let kind = (member.attrs >> 16) & 0xf000;
        if std::mem::replace(&mut seen[index], true)
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
        names.push(index);
    }
    if seen.contains(&false) {
        return Ok(None);
    }
    Ok(Some(SevenMemoryPlan { data, parsed, order: names }))
}

fn decode_memory_sevenz(
    source: Option<File>,
    entries: &[Entry],
    ctl: &JobControl,
) -> Result<Option<MemoryFiles>, String> {
    let Some(SevenMemoryPlan { data, parsed, order }) = prepare_memory_sevenz(source, entries, ctl)? else {
        return Ok(None);
    };
    ctl.set(
        "decoding",
        Some(0),
        "Verifying complete solid archive in bounded memory",
    );
    let bytes = Arc::new(parsed.decode(&data, &|| ctl.check())?);
    if bytes.capacity() as u64 > MEMORY_ZIP_LIMIT {
        return Err("7z memory capacity budget exceeded".into());
    }
    let mut output = MemoryFiles::with_capacity(entries.len());
    for (index, member) in order.into_iter().zip(&parsed.archive.entries) {
        ctl.check()?;
        let start = usize::try_from(member.offset_in_stream).map_err(|_| "7z offset overflow")?;
        let end = start
            .checked_add(member.size as usize)
            .ok_or("7z range overflow")?;
        let chunk = bytes
            .get(start..end)
            .ok_or("7z entry exceeds decoded stream")?;
        let entry = &entries[index];
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
                path_key(&validate_name(&entry.name)?),
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
    let policy = crate::machine_policy::scope();
    let ctl = control(opts);
    ctl.check()?;
    // Keep both the source and its directory namespace fixed from the typed
    // listing/format guard through decoding and publication.
    let source_path = absolute(path)?;
    let mut source_guard = crate::engine_stream::pin(&source_path)?;
    #[cfg(windows)]
    let _volume_guards = crate::engine_stream::pin_numbered_7z_parts(&source_path)?;
    let mut memory_source = if policy.restricted() { None } else { memory_archive_source(path, opts) };
    let mut selected_source = if !policy.restricted() && !opts.selected.is_empty()
        && Path::new(path)
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
    {
        locked_archive_source(path, opts, u32::MAX as u64 - 1)
    } else {
        None
    };
    let (all_entries, payload_source) = list_raw_with_source(path, opts, &mut source_guard)?;
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
    #[cfg(windows)]
    if let Some(source) = selected_source.as_ref() {
        let same = locked_archive_source(path, opts, u32::MAX as u64 - 1).is_some_and(|current| {
            directory_identity(source)
                .ok()
                .zip(directory_identity(&current).ok())
                .is_some_and(|(before, after)| before == after)
        });
        if !same {
            selected_source = None;
        }
    }
    let mut expected = expected_tree(&all_entries)?;
    let selection = Selection::new(&opts.selected);
    let entries: Vec<_> = all_entries
        .iter()
        .filter(|e| selection.contains(&e.name))
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
    #[cfg(all(windows, target_pointer_width = "64"))]
    let selected_zip_candidate = matches!(payload_source, PayloadSource::Engine)
        && !opts.selected.is_empty() && selected_source.is_some()
        && memory_source.is_some() && entries.iter().filter(|e| !e.is_dir).nth(127).is_some()
        // The shared filesystem publisher uses at most four workers.
        && opts.threads.is_none_or(|limit| limit >= 4) && using_process_token();
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    let selected_zip_candidate = false;
    let defer_preflight = (opts.selected.is_empty() || selected_zip_candidate)
        && !dest.try_exists().map_err(|e| e.to_string())?
        && dest.parent().is_some_and(Path::is_dir);
    if !defer_preflight {
        preflight_output_targets(&entries, &dest, &opts.overwrite, &mut output_guards)?;
    }
    ctl.check()?;
    let official_zeal = matches!(payload_source, PayloadSource::OfficialZeal);
    let (mut bounded_plan, queued_transaction) = if matches!(payload_source, PayloadSource::Engine)
        && opts.selected.is_empty() && defer_preflight && expected.len() <= 65_536
    {
        bounded_archive_stage::plan_transaction(path, &dest, memory_source.as_mut(), &all_entries, &ctl, &mut output_guards)?
            .map_or((None, None), |(plan, transaction)| (Some(plan), Some(transaction)))
    } else {
        (None, None)
    };
    let memory = if bounded_plan.is_some() { None } else if let PayloadSource::VerifiedZeal(files) = payload_source {
        Some(files)
    } else if Path::new(path)
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("7z"))
    {
        decode_memory_sevenz(memory_source, &all_entries, &ctl)?
    } else {
        decode_memory_zip(memory_source, &all_entries, opts, &ctl)?
    };
    #[cfg(nana_selected_worker)]
    let split_plan = if !policy.restricted() && !opts.selected.is_empty() && !_volume_guards.is_empty() {
        selected_worker::plan(path, &source_guard, &_volume_guards, &all_entries, opts)?
    } else { None };
    #[cfg(nana_selected_worker)]
    let plan_selection = split_plan.is_some() && using_process_token();
    #[cfg(not(nana_selected_worker))]
    let plan_selection = false;
    // Only decode_memory_zip has read every member to EOF and retained exactly
    // the selected file bytes. A fallback decoder may still produce the whole tree.
    let memory_selection = selected_zip_candidate && defer_preflight && memory.is_some();
    let projected = (plan_selection || memory_selection).then(|| entries.iter().map(|e| (*e).clone()).collect::<Vec<_>>());
    let safe_projection = opts.selected.is_empty() || projected.is_some();
    // A selected stage may be committed only after the isolated worker verifies
    // the entire fixed archive family and confirms that it wrote this projection.
    let mut transaction = if queued_transaction.is_some() { queued_transaction } else if expected.len() <= 65_536 {
        new_directory_stage(path, &dest, safe_projection, &mut output_guards,
            projected.as_deref().unwrap_or(&all_entries), projected.is_some() || memory.is_some())?
    } else {
        None
    };
    if PhaseTimer::enabled() && !opts.selected.is_empty() {
        eprintln!("OA_SELECTED_PUBLISH bounded={} deferred={} memory={} transaction={}", selected_zip_candidate, defer_preflight, memory.is_some(), transaction.is_some());
    }
    // Small-file parallel publication keeps its established route. Large serial
    // output can move verified files from an atomic private stage on this volume.
    let serial_publication = !(opts.overwrite == "error"
        && opts.selected.is_empty()
        && entries.iter().filter(|e| !e.is_dir).count() >= 128
        && entries.iter().all(|e| e.is_dir || e.size <= 8 * 1024 * 1024));
    let mut file_publication_stage = false;
    stage = if let Some(transaction) = transaction.as_mut() {
        transaction.stage.take()
    } else if memory.is_none() {
        let preferred = if serial_publication {
            new_file_publication_stage(path, &dest, safe_projection, projected.as_deref().unwrap_or(&all_entries), &mut output_guards)?
        } else { None };
        file_publication_stage = preferred.is_some();
        Some(match preferred {
            Some(stage) => stage,
            None => new_fallback_stage(&std::env::temp_dir(), &mut output_guards)?,
        })
    } else {
        None
    };
    // Counts exist only for the explicit diagnostic run; formal timing keeps
    // profiling disabled. Never include paths, archive names or passwords.
    let mut publication_receipt = (file_publication_stage && PhaseTimer::enabled()).then(|| [0usize; 2]);
    if defer_preflight && transaction.is_none() {
        preflight_output_targets(&entries, &dest, &opts.overwrite, &mut output_guards)?;
    }
    let mut stage_guards = stage
        .as_ref()
        .map(|s| s.guards(transaction.as_ref().map(|t| &t.root)))
        .transpose()?;
    #[cfg(nana_selected_worker)]
    let split_streamed = if let Some(plan) = split_plan {
        let root = stage.as_ref().ok_or("selected worker stage unavailable")?.path();
        let guards = stage_guards.as_mut().ok_or("missing selected worker stage guards")?;
        guards.check(root)?;
        ctl.set("decoding", Some(0), "Verifying all archive entries");
        selected_worker::decode(plan, &source_guard, &_volume_guards, &all_entries, opts, root, guards, &ctl)?
    } else { false };
    #[cfg(not(nana_selected_worker))]
    let split_streamed = false;
    if plan_selection && !split_streamed && (transaction.is_some() || file_publication_stage) {
        // Unsupported native layouts return false only before granting outputs.
        // The general decoder can write the entire archive, so discard the
        // smaller reservation and use its original private TEMP stage.
        drop(stage_guards.take());
        drop(transaction.take());
        drop(stage.take());
        file_publication_stage = false;
        publication_receipt = None;
        let fallback = new_fallback_stage(&std::env::temp_dir(), &mut output_guards)?;
        stage_guards = Some(fallback.guards(None)?);
        stage = Some(fallback);
    }
    let verified_projection = split_streamed || memory_selection;
    let staged_entries = projected.as_deref().filter(|_| verified_projection).unwrap_or(&all_entries);
    if let Some(stage) = &stage {
        let stage_guards = stage_guards.as_mut().unwrap();
        stage_guards.check(stage.path())?;
        if !split_streamed {
            ctl.set("decoding", Some(0), "Decoding archive into temporary storage");
        }
        let streamed = split_streamed || (memory.is_none() && decode_selected_zip(
            selected_source.take(),
            &all_entries,
            opts,
            stage.path(),
            stage_guards,
            &ctl,
        )?);
        if let Some(plan) = bounded_plan.take() {
            plan.decode(&all_entries, stage.path(), stage_guards, &ctl, opts.threads)?;
        } else if let Some(files) = &memory {
            stage_verified_memory(files, staged_entries, stage.path(), stage_guards, &ctl, opts.threads)?;
        } else if !streamed {
            let mut args = base("x", opts)?;
            let execute = if official_zeal {
                args.push("-tZealFS".into());
                run_with_zealfs
            } else { run };
            if let Some(format) = crate::stream_formats::raw_format(path) {
                args.push(format!("-t{format}"));
            }
            if all_entries.len() == 1 && !all_entries[0].is_dir {
                // Raw compressors may synthesize different disk and listing names.
                // One stdout decode is unambiguous and retains the advertised name.
                let target = stage.path().join(validate_name(&all_entries[0].name)?);
                args.extend([
                    "-so".into(),
                    "--".into(),
                    absolute(path)?.display().to_string(),
                ]);
                let mut output = create_staged_output(&target, stage_guards)?;
                let native = if let Some(format) = crate::stream_formats::raw_format(path) {
                    crate::engine_stream::validate_frame_bounds(&absolute(path)?, format, opts)?;
                    crate::engine_stream::decode_native(&absolute(path)?, format, opts, None, &mut |bytes| {
                        std::io::Write::write_all(&mut output, bytes).map_err(|e|e.to_string())
                    })?
                } else { false };
                if !native { execute(&args, None, opts, Some(output))?; }
            } else {
                args.extend([
                    format!("-o{}", stage.path().display()),
                    "--".into(),
                    absolute(path)?.display().to_string(),
                ]);
                execute(&args, None, opts, None)?;
            }
        }
        let mut staged_expected = if verified_projection && projected.is_some() {
            expected_tree(staged_entries)?
        } else if streamed {
            expected_tree(&entries.iter().map(|e| (*e).clone()).collect::<Vec<_>>())?
        } else {
            expected.clone()
        };
        if crate::stream_formats::raw_format(path).is_some() {
            // Raw handlers may not know the decoded length before their single
            // checked decode. Resolve only those entries from the private stage;
            // ordinary archive size checks retain their declared expectations.
            for entry in all_entries.iter().filter(|entry| !entry.size_known) {
                let relative = validate_name(&entry.name)?;
                let target = stage.path().join(&relative);
                stage_guards.check(&target)?;
                let metadata = fs::symlink_metadata(&target).map_err(|error| error.to_string())?;
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("invalid decoded raw stream".into());
                }
                staged_expected.get_mut(&path_key(&relative)).ok_or("raw stream missing from manifest")?.1 = metadata.len();
            }
            // Every later commit/pin branch must use the same resolved sizes.
            expected = staged_expected.clone();
        }
        verify_stage_with_metadata(
            stage.path(),
            &staged_expected,
            &ctl,
            stage_guards,
            (transaction.is_some() || file_publication_stage).then_some(staged_entries),
            if transaction.is_some() || file_publication_stage { opts.zone_identifier.as_ref() } else { None },
        )?;
        if verified_projection && transaction.is_some() { expected = staged_expected; }
    }
    policy.validate_output()?;
    if let Some(transaction) = transaction.as_ref() {
        let root = &transaction.root;
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
        #[cfg(all(windows, target_pointer_width = "64"))]
        let compatible = if let Some(witness) = &transaction.witnessed {
            let pins = pin_committed_tree(temp.path(), &expected)?;
            let matches = using_process_token() && witness.compatible(&transaction.parent, root, &pins)?;
            if !matches && PhaseTimer::enabled() {
                eprintln!("OA_PUBLICATION witnessed_directory_fallback=1");
            }
            matches
        } else { true };
        #[cfg(not(all(windows, target_pointer_width = "64")))]
        let compatible = true;
        ctl.check()?;
        policy.validate_output()?;
        if compatible && rename_directory_noclobber_in(root, &dest, Some(&transaction.parent))? {
            let _published_path = stage.take().unwrap().keep();
            #[cfg(windows)]
            if transaction.restore_acl {
                // Still private here. Hold every object against replacement and
                // reparse conversion while recursive inheritance opens access.
                let publication_pins = pin_committed_tree(&dest, &expected)
                    .map_err(|e| format!("verified output was committed but remains private: {e}"))?;
                #[cfg(target_pointer_width = "64")]
                let restored = match &transaction.witnessed {
                    Some(witness) => witness.restore(&transaction.parent, root, &publication_pins),
                    None => crate::private_stage::inherit_tree(root, &transaction.parent, &publication_pins),
                };
                #[cfg(not(target_pointer_width = "64"))]
                let restored = crate::private_stage::inherit_tree(root, &transaction.parent, &publication_pins);
                restored.map_err(|e| format!("verified output was committed but permissions could not be restored: {e}"))?;
                drop(publication_pins);
            }
            ctl.set("complete", Some(100), "Extraction complete");
            #[cfg(all(windows, target_pointer_width = "64"))]
            if transaction.witnessed.is_some() && PhaseTimer::enabled() {
                eprintln!("OA_PUBLICATION witnessed_directory_commit=1");
            }
            if verified_projection && PhaseTimer::enabled() {
                eprintln!("OA_PUBLICATION selected_directory_commit=1");
            }
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
        #[cfg(windows)]
        guards.private_stages.insert(temp.path().to_path_buf());
        verify_stage_checked(temp.path(), &expected, &ctl, &mut guards)?;
        // An unsupported whole-directory rename may use old publication only
        // after atomically claiming the still-absent destination ourselves.
        // Never merge into a competitor created during fallback verification.
        ctl.check()?;
        output_guards.check(&dest)?;
        #[cfg(windows)]
        if verified_projection {
            // Claim a fresh directory without following or merging a competing
            // creation. Keep its identity for rollback if copying is cancelled.
            let leaf = dest.file_name().ok_or("missing fallback destination leaf")?;
            let created = crate::private_stage::create_output_directory(&transaction.parent, leaf)?;
            output_guards.created_output_dirs.push((
                dest.clone(),
                created_directories::Identity::Handle(created),
            ));
            output_guards.selected_output_root = Some(dest.clone());
        } else {
            fs::create_dir(&dest).map_err(|e| format!("claim fallback destination: {e}"))?;
        }
        #[cfg(not(windows))]
        fs::create_dir(&dest).map_err(|e| format!("claim fallback destination: {e}"))?;
        #[cfg(windows)]
        output_guards.owned_dirs.insert(dest.clone());
        output_guards.check(&dest)?;
        stage_guards = Some(guards);
    }
    drop(transaction.take());
    if memory.is_some() && stage.is_some() {
        // After an unsupported directory rename the fallback reads verified memory.
        // Release the staging handles and disk copy before creating more files.
        drop(stage_guards.take());
        drop(stage.take());
    }
    let staged = match &memory {
        Some(files) => StagedContents::Memory(files),
        None => StagedContents::Disk(stage.as_ref().unwrap().path()),
    };
    #[cfg(windows)]
    // The ledger records only directories whose FILE_CREATE succeeded. An
    // existing root may still acquire new empty children before cancellation.
    if !opts.selected.is_empty() {
        output_guards.selected_output_root = Some(dest.clone());
    }
    let file_count = entries.iter().filter(|e| !e.is_dir).count();
    let count = if opts.overwrite == "error"
        && (opts.selected.is_empty() || memory_selection)
        && file_count >= 128
        && entries
            .iter()
            .all(|e| e.is_dir || e.size <= 8 * 1024 * 1024)
    {
        publish_staged_files_parallel(
            &entries,
            &dest,
            staged,
            &ctl,
            &mut output_guards,
            opts.zone_identifier.as_ref(),
        )?
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
            if file_publication_stage && try_publish_staged_file(
                staged, &rel, e, &target, opts.overwrite == "overwrite", &ctl,
                stage_guards.as_mut().ok_or("missing private stage guards")?, &mut output_guards,
            )? {
                // Content, ADS and exact times preceded the atomic commit;
                // the helper also finalized permissions before returning success.
                // The caller performs no additional file metadata writes.
                if let Some(receipt) = publication_receipt.as_mut() { receipt[0] += 1; }
                count += 1;
                continue;
            }
            output_guards.allow_publication(parent)?;
            with_output_temp(parent, &output_guards, |tmp| {
                let mut source = staged.reader(&rel)?;
                let mut buf = vec![0; 1024 * 1024];
                loop {
                    ctl.check()?;
                    let n = source.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 { break; }
                    use std::io::Write;
                    tmp.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                }
                crate::motw::apply_to_file(tmp.as_file(), opts.zone_identifier.as_ref())?;
                restore_open_file_metadata(tmp.as_file(), e).map_err(|e| format!("file metadata: {e}"))?;
                ctl.check()?;
                output_guards.check_prepared(&target)?;
                drop(publish_output(tmp, &target, opts.overwrite == "overwrite")?);
                Ok(())
            })?;
            if let Some(receipt) = publication_receipt.as_mut() { receipt[1] += 1; }
            count += 1;
        }
        count
    };
    for e in entries.iter().rev().filter(|e| e.is_dir) {
        restore_metadata(&dest.join(validate_name(&e.name)?), e)
            .map_err(|e| format!("directory metadata: {e}"))?;
    }
    #[cfg(windows)]
    output_guards.created_output_dirs.clear();
    drop(stage_guards);
    if let Some([moved, copied]) = publication_receipt {
        eprintln!("OA_PUBLICATION same_volume_stage=1 moved={moved} copied={copied}");
    }
    ctl.set("complete", Some(100), "Extraction complete");
    Ok(count)
}

// Verify that the stage's owner and every allowed SID are trusted. A broad
// parent is supported only because the stage is created with a private DACL.
#[cfg(all(windows,test))]
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

enum StagingDir {
    Temporary(tempfile::TempDir),
    #[cfg(windows)]
    Private(crate::private_stage::PrivateStage),
}
impl StagingDir {
    fn guards(&self, transaction_root: Option<&File>) -> Result<CheckedDirectories, String> {
        let mut guards = CheckedDirectories::owned_root(self.path());
        #[cfg(windows)]
        if let Some(root) = transaction_root.or_else(|| self.private_root()) {
            // Reopening without DELETE sharing conflicts with this dedicated
            // root handle, so register its clone before any descendant checks.
            guards.held.insert(self.path().to_path_buf(), (root.try_clone().map_err(|e| e.to_string())?, false));
            guards.private_stages.insert(self.path().to_path_buf());
        }
        #[cfg(not(windows))]
        let _ = transaction_root;
        Ok(guards)
    }
    #[cfg(windows)]
    fn private_root(&self) -> Option<&File> {
        match self { Self::Private(stage) => Some(stage.root()), _ => None }
    }
    fn path(&self) -> &Path {
        match self {
            Self::Temporary(temp) => temp.path(),
            #[cfg(windows)]
            Self::Private(temp) => temp.path(),
        }
    }
    fn keep(self) -> PathBuf {
        match self {
            Self::Temporary(temp) => temp.keep(),
            #[cfg(windows)]
            Self::Private(temp) => temp.keep(),
        }
    }
}
/// Private output of one Explorer drag. Hold its directory identity until
/// preparation is abandoned or the shell accepts responsibility for the data.
#[cfg(windows)]
pub struct ShellDragDirectory {
    path: PathBuf,
    root: Option<File>,
    _parent_pin: File,
    _ancestors: CheckedDirectories,
    retained: AtomicBool,
}
#[cfg(windows)]
impl ShellDragDirectory {
    pub fn create() -> Result<Self, String> {
        use std::os::windows::fs::OpenOptionsExt;
        let parent = absolute(
            std::env::temp_dir()
                .to_str()
                .ok_or("invalid temporary directory")?,
        )?;
        let mut ancestors = CheckedDirectories::default();
        ancestors.check(&parent)?;
        let pin = File::options()
            .access_mode(0x20081)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(&parent)
            .map_err(|e| e.to_string())?;
        validate_transaction_root(&pin, &parent)?;
        let (path, root, parent_pin) = crate::private_stage::create_drag_root(&parent, &pin)?;
        Ok(Self {
            path,
            root: Some(root),
            _parent_pin: parent_pin,
            _ancestors: ancestors,
            retained: AtomicBool::new(false),
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn retain_for_shell(&self) {
        self.retained.store(true, Ordering::Release);
    }
    pub fn shell_read_finished(&self) {
        self.retained.store(false, Ordering::Release);
    }
    pub fn pin_for_read(&self) -> Result<File, String> {
        use std::os::windows::fs::OpenOptionsExt;
        let pin = File::options().access_mode(0x81).share_mode(1)
            .custom_flags(0x02200000).open(&self.path).map_err(|e| e.to_string())?;
        validate_transaction_root(&pin, &self.path)?;
        if directory_identity(&pin)? != directory_identity(self.root.as_ref().ok_or("missing drag root")?)? {
            return Err("drag export directory changed".into());
        }
        Ok(pin)
    }
}
#[cfg(windows)]
impl Drop for ShellDragDirectory {
    fn drop(&mut self) {
        use std::os::windows::fs::OpenOptionsExt;
        if self.retained.load(Ordering::Acquire) {
            return;
        }
        let Ok(strong) = self.pin_for_read() else { return };
        let Some(root) = self.root.take() else { return };
        let open = |access| {
            File::options()
                .access_mode(access)
                .share_mode(7)
                .custom_flags(0x02200000)
                .open(&self.path)
        };
        let Ok(anchor) = open(0x80) else { return };
        let Ok(id) = directory_identity(&root) else {
            return;
        };
        if directory_identity(&anchor).ok() != Some(id) {
            return;
        }
        // Walk only this operation's pinned, privately created directory.
        if let Ok(children) = fs::read_dir(&self.path) {
            for child in children.flatten() {
                let path = child.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(path);
                } else {
                    if let Ok(file) = File::options()
                        .access_mode(0x180)
                        .share_mode(7)
                        .custom_flags(0x00200000)
                        .open(&path)
                    {
                        if let Ok(meta) = crate::private_stage::checked_metadata(&file) {
                            if meta.permissions().readonly() {
                                let mut permissions = meta.permissions();
                                permissions.set_readonly(false);
                                let _ = file.set_permissions(permissions);
                            }
                        }
                    }
                    let _ = fs::remove_file(path);
                }
            }
        }
        drop(strong);
        drop(root);
        // The anchor keeps the ID alive across reopening with DELETE access.
        // The kernel only removes an empty directory; no replacement is walked.
        let Ok(delete) = open(0x10080) else { return };
        if directory_identity(&delete).ok() == Some(id) {
            crate::private_stage::delete_empty_directory(&delete);
        }
    }
}

// Windows TEMP can be shared or user-selected. Create the fallback with a
// private DACL before exposing a pathname.
fn new_fallback_stage(parent: &Path, guards: &mut CheckedDirectories) -> Result<StagingDir, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let parent = absolute(parent.to_str().ok_or("invalid temporary directory")?)?;
        guards.check(&parent)?;
        let parent_handle = fs::OpenOptions::new().access_mode(0x20081).share_mode(3)
            .custom_flags(0x02200000).open(&parent).map_err(|e| e.to_string())?;
        validate_transaction_root(&parent_handle, &parent)?;
        let stage = crate::private_stage::create(&parent, &parent_handle)?;
        validate_transaction_root(stage.root(), stage.path())?;
        if !private_stage_acl_handle(stage.root())? {
            return Err("temporary stage is not private".into());
        }
        Ok(StagingDir::Private(stage))
    }
    #[cfg(not(windows))]
    {
        guards.check(parent)?;
        tempfile::Builder::new().prefix(".qing-zip-stage-").tempdir_in(parent)
            .map(StagingDir::Temporary).map_err(|e| e.to_string())
    }
}
fn create_staged_output(target: &Path, guards: &mut CheckedDirectories) -> Result<File, String> {
    let parent = target.parent().ok_or("missing staged parent")?;
    if !guards.pinned(parent) {
        guards.create_dirs(parent)?;
    }
    // This still checks every weak ancestor as well as the new leaf.
    guards.check(target)?;
    fs::OpenOptions::new().write(true).create_new(true).open(target).map_err(|e| e.to_string())
}

struct MemoryOutput<'a> {
    entry: &'a Entry,
    target: PathBuf,
    bytes: &'a [u8],
}

fn open_prepared_stage_output(target: &Path, guards: &CheckedDirectories) -> Result<File, String> {
    let checked_create = || {
        guards.check_prepared(target)?;
        File::options().write(true).create_new(true).open(target)
            .map_err(|error| error.to_string())
    };
    #[cfg(windows)]
    let output = match target.parent().and_then(|p| guards.private_metadata_parent(p)) {
        Some(parent) => {
            let leaf = target.file_name().ok_or("missing stage filename")?;
            crate::private_stage::create_child_file(parent, leaf)?
        }
        None => checked_create()?,
    };
    #[cfg(not(windows))]
    let output = checked_create()?;
    Ok(output)
}

fn write_verified_bytes(
    entry: &Entry, target: &Path, bytes: &[u8],
    guards: &CheckedDirectories,
    ctl: &JobControl,
) -> Result<(), String> {
    use std::io::Write;
    ctl.check()?;
    if !entry.size_known || bytes.len() as u64 != entry.size {
        return Err("verified memory file length does not match archive".into());
    }
    let mut output = open_prepared_stage_output(target, guards)?;
    for chunk in bytes.chunks(64 * 1024) {
        ctl.check()?;
        output.write_all(chunk).map_err(|error| error.to_string())?;
    }
    restore_open_file_metadata(&output, entry)
}

fn stage_verified_memory(
    files: &MemoryFiles,
    entries: &[Entry],
    root: &Path,
    guards: &mut CheckedDirectories,
    ctl: &JobControl,
    thread_limit: Option<u32>,
) -> Result<(), String> {
    use std::sync::atomic::AtomicUsize;
    let _timer = PhaseTimer::new("stage-verified-memory");
    ctl.check()?;
    if files.len() != entries.iter().filter(|entry| !entry.is_dir).count() {
        return Err("verified memory file count does not match archive".into());
    }
    let mut jobs = Vec::with_capacity(files.len());
    let mut directories = HashSet::new();
    for entry in entries {
        ctl.check()?;
        let relative = validate_name(&entry.name)?;
        let target = root.join(&relative);
        let directory = if entry.is_dir {
            target.as_path()
        } else {
            target.parent().ok_or("missing staged parent")?
        };
        if !directories.contains(directory) {
            guards.create_dirs(directory)?;
            directories.insert(directory.to_path_buf());
        }
        if !entry.is_dir {
            let bytes = files.get(&path_key(&relative)).ok_or("verified memory entry missing")?.as_slice();
            jobs.push(MemoryOutput { entry, target, bytes });
        }
    }
    // All directories are prepared before sharing their guards. Only strong
    // direct parents inside this private stage qualify for handle-relative create.
    let guards: &CheckedDirectories = guards;
    let next = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let consume = || -> Result<(), String> {
        while !stopped.load(Ordering::Acquire) {
            let Some(job) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) else { break };
            if let Err(error) = write_verified_bytes(job.entry, &job.target, job.bytes, guards, ctl) {
                stopped.store(true, Ordering::Release);
                return Err(error);
            }
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if done % 64 == 0 {
                ctl.set("extracting", Some((done * 100 / jobs.len()) as u8), &job.entry.name);
            }
        }
        Ok(())
    };
    let workers = thread::available_parallelism().map_or(1, |n| n.get())
        .min(thread_limit.unwrap_or(4).clamp(1, 4) as usize);
    if workers == 1 || jobs.len() < 128 {
        consume()?;
    } else {
        thread::scope(|scope| {
            let mut handles = Vec::with_capacity(workers);
            let mut result = Ok(());
            for _ in 0..workers {
                match thread::Builder::new().spawn_scoped(scope, &consume) {
                    Ok(handle) => handles.push(handle),
                    Err(error) => {
                        stopped.store(true, Ordering::Release);
                        result = Err(error.to_string());
                        break;
                    }
                }
            }
            for handle in handles {
                let outcome = handle.join().unwrap_or_else(|_| Err("private stage writer panicked".into()));
                result = result.and(outcome);
            }
            result
        })?;
    }
    ctl.check()
}

// Reserve enough space for both the complete stage and the existing copy
// fallback. A nearly-full target must retain the other-volume TEMP route.
fn file_stage_reservation(entries: &[Entry]) -> Option<u64> {
    let mut total = 0u64;
    for entry in entries {
        if !entry.size_known { return None; }
        total = total.checked_add(entry.size)?;
    }
    total.checked_mul(2)?.checked_add(64 * 1024 * 1024)?
        .checked_add((entries.len() as u64).checked_mul(4096)?)
}
#[cfg(all(windows, target_pointer_width = "64"))]
fn has_stage_space(parent: &Path, required: u64) -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(path: *const u16, available: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let wide: Vec<_> = parent.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) != 0
            && available >= required
    }
}
#[cfg(all(windows, target_pointer_width = "64"))]
fn using_process_token() -> bool {
    use std::{ffi::c_void, os::windows::io::{FromRawHandle, OwnedHandle}};
    #[link(name = "Advapi32")]
    extern "system" {
        fn OpenThreadToken(thread: *mut c_void, access: u32, as_self: i32, token: *mut *mut c_void) -> i32;
    }
    let mut token = std::ptr::null_mut();
    if unsafe { OpenThreadToken(-2isize as *mut c_void, 8, 1, &mut token) } != 0 {
        drop(unsafe { OwnedHandle::from_raw_handle(token) });
        return false;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(1008) // ERROR_NO_TOKEN
}

fn new_file_publication_stage(
    archive: &str, dest: &Path, safe_projection: bool, entries: &[Entry],
    guards: &mut CheckedDirectories,
) -> Result<Option<StagingDir>, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if !safe_projection || !dest.is_dir() { return Ok(None); }
        let Some(required) = file_stage_reservation(entries) else { return Ok(None); };
        let Some(parent) = dest.parent().filter(|p| p.is_dir()) else { return Ok(None); };
        if attribute_query_path(parent).is_none() { return Ok(None); }
        let mut magic = [0; 6];
        let mut input = File::open(archive).map_err(|e| e.to_string())?;
        if input.read_exact(&mut magic).is_err()
            || !(magic == *b"7z\xbc\xaf'\x1c" || magic[..4] == *b"PK\x03\x04" || magic[..4] == *b"PK\x05\x06") {
            return Ok(None);
        }
        guards.check(parent)?;
        if !parent.ancestors().all(|p| guards.pinned(p)) { return Ok(None); }
        let handle = match fs::OpenOptions::new().access_mode(0x200a0).share_mode(3)
            .custom_flags(0x02200000).open(parent) {
            Ok(handle) => handle, Err(_) => return Ok(None),
        };
        validate_transaction_root(&handle, parent)?;
        if !crate::private_stage::supports_file_publication(&handle)? { return Ok(None); }
        if !has_stage_space(parent, required) { return Ok(None); }
        let stage = match crate::private_stage::create(parent, &handle) {
            Ok(stage) => stage, Err(_) => return Ok(None),
        };
        validate_transaction_root(stage.root(), stage.path())?;
        if !private_stage_acl_handle(stage.root())? { return Err("file publication stage is not private".into()); }
        Ok(Some(StagingDir::Private(stage)))
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    { let _ = (archive, dest, safe_projection, entries, guards); Ok(None) }
}

fn try_publish_staged_file(
    staged: StagedContents<'_>, relative: &Path, entry: &Entry, target: &Path,
    replace: bool, ctl: &JobControl, stage_guards: &mut CheckedDirectories,
    output_guards: &mut CheckedDirectories,
) -> Result<bool, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let StagedContents::Disk(root) = staged else { return Ok(false); };
        let Some(modified) = archive_modified_time(&entry.modified) else { return Ok(false); };
        let source = root.join(relative);
        stage_guards.check(&source)?;
        let source_parent = source.parent().ok_or("missing staged parent")?;
        let Some(source_pin) = stage_guards.private_metadata_parent(source_parent) else { return Ok(false); };
        let parent = target.parent().ok_or("missing publication parent")?;
        output_guards.allow_publication(parent)?;
        output_guards.check(target)?;
        let Some((existing, _)) = output_guards.held.get(parent) else { return Ok(false); };
        let parent_pin = match fs::OpenOptions::new().access_mode(0x200a0).share_mode(3)
            .custom_flags(0x02200000).open(parent) {
            Ok(pin) => pin, Err(_) => return Ok(false),
        };
        if directory_identity(existing)? != directory_identity(&parent_pin)?
            || directory_attributes(&parent_pin)? & 0x410 != 0x10 {
            return Err("publication parent identity changed".into());
        }
        match crate::private_stage::publish_verified_file(
            source_pin, source.file_name().ok_or("missing source leaf")?, &parent_pin,
            target.file_name().ok_or("missing destination leaf")?, entry.size, modified,
            entry.attributes.contains('R'), replace,
            || { ctl.check()?; output_guards.check(target) },
        )? {
            crate::private_stage::FilePublication::Unavailable => Ok(false),
            crate::private_stage::FilePublication::Committed(file) => { drop(file); Ok(true) },
        }
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    { let _ = (staged, relative, entry, target, replace, ctl, stage_guards, output_guards); Ok(false) }
}
struct DirectoryTransaction {
    // Release the no-delete-sharing root pin before a still-owned TempDir is
    // removed, including when an archive fails qualification.
    root: File,
    parent: File,
    restore_acl: bool,
    #[cfg(all(windows, target_pointer_width = "64"))]
    witnessed: Option<crate::private_stage::WitnessedInheritance>,
    stage: Option<StagingDir>,
}
#[cfg(windows)]
pub(crate) fn pin_committed_tree(
    root: &Path,
    expected: &BTreeMap<String, (bool, u64)>,
) -> Result<Vec<crate::private_stage::PermissionNode>, String> {
    use std::os::windows::{
        fs::OpenOptionsExt,
        io::AsRawHandle,
    };
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut u32) -> i32;
    }
    let mut pins = Vec::with_capacity(expected.len());
    let mut pending = vec![(root.to_path_buf(), None)];
    let mut found = HashSet::new();
    while let Some((directory, parent_index)) = pending.pop() {
        for item in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let path = item.map_err(|e| e.to_string())?.path();
            let key = path_key(path.strip_prefix(root).map_err(|e| e.to_string())?);
            let &(is_dir, size) = expected.get(&key).ok_or("unexpected committed entry")?;
            let pin = fs::OpenOptions::new()
                // READ_DATA/LIST_DIRECTORY is necessary for Windows to enforce
                // this pin's write/delete sharing restrictions. Attributes and
                // security-only access does not reliably activate share checks.
                .access_mode(0x60081)
                .share_mode(1)
                .custom_flags(0x02200000)
                .open(&path)
                .map_err(|e| e.to_string())?;
            // One fresh handle query covers type, length, reparse attributes and
            // link count. Do not query Metadata and then repeat the same I/O for
            // hard-link validation on every published file.
            let mut info = [0u32; 13];
            if unsafe { GetFileInformationByHandle(pin.as_raw_handle(), info.as_mut_ptr()) } == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let attributes = info[0];
            let actual_size = (u64::from(info[8]) << 32) | u64::from(info[9]);
            if !found.insert(key)
                || attributes & 0x400 != 0
                || (attributes & 0x10 != 0) != is_dir
                || (!is_dir && (attributes & 0x40 != 0 || actual_size != size))
            {
                return Err("committed entry identity or type changed".into());
            }
            if is_dir {
                pending.push((path, Some(pins.len())));
            } else {
                if info[10] != 1 {
                    return Err("hard linked committed entry rejected".into());
                }
            }
            pins.push(crate::private_stage::PermissionNode {
                file: pin,
                parent_index,
                is_dir,
            });
        }
    }
    if found.len() != expected.len() {
        return Err("committed entries missing".into());
    }
    Ok(pins)
}
// Create privately on the destination volume. Normal inherited permissions are
// restored only after verified content has been committed by its pinned handle.
fn new_directory_stage(
    path: &str,
    dest: &Path,
    safe_projection: bool,
    guards: &mut CheckedDirectories,
    entries: &[Entry],
    reserve: bool,
) -> Result<Option<DirectoryTransaction>, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        if !safe_projection || dest.try_exists().map_err(|e| e.to_string())? {
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
        if !parent.ancestors().all(|part| guards.pinned(part)) {
            return Ok(None);
        }
        let parent_handle = match fs::OpenOptions::new()
            .access_mode(0x200a0)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(parent)
        {
            Ok(handle) => handle,
            Err(_) => return Ok(None),
        };
        validate_transaction_root(&parent_handle, parent)?;
        if reserve {
            // Copy publication inherits storage attributes and uses the effective
            // token. Whole-tree ACL derivation uses the process token.
            let Some(required) = file_stage_reservation(entries) else { return Ok(None); };
            if !using_process_token() || !has_stage_space(parent, required)
                || !crate::private_stage::supports_file_publication(&parent_handle)? {
                return Ok(None);
            }
        }
        let private_parent = private_stage_acl_handle(&parent_handle)?;
        let witnessed = if (!private_parent || reserve) && !crate::private_stage::supports_inheritance(&parent_handle)? {
            if private_parent || !using_process_token() { return Ok(None); }
            let Some(required) = file_stage_reservation(entries) else { return Ok(None); };
            if !has_stage_space(parent, required) { return Ok(None); }
            // A missing capability (including witness creation/access) always
            // preserves the ordinary per-file publication route.
            let Some(witness) = crate::private_stage::WitnessedInheritance::qualify(parent, &parent_handle)?
                else { return Ok(None); };
            Some(witness)
        } else { None };
        if private_parent {
            let temp = match tempfile::Builder::new()
                .prefix(".open-archive-stage-")
                .tempdir_in(parent)
            {
                Ok(temp) => temp,
                Err(_) => return Ok(None),
            };
            let root = match fs::OpenOptions::new()
                .access_mode(0x30080)
                .share_mode(1)
                .custom_flags(0x02200000)
                .open(temp.path())
            {
                Ok(root) => root,
                Err(_) => return Ok(None),
            };
            validate_transaction_root(&root, temp.path())?;
            if !private_stage_acl_handle(&root)?
                || (reserve && !crate::private_stage::supports_file_publication(&root)?) {
                return Ok(None);
            }
            return Ok(Some(DirectoryTransaction {
                stage: Some(StagingDir::Temporary(temp)),
                root,
                parent: parent_handle,
                restore_acl: false,
                witnessed: None,
            }));
        }
        let temp = match crate::private_stage::create(parent, &parent_handle) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        let root = temp.root().try_clone().map_err(|e| e.to_string())?;
        validate_transaction_root(&root, temp.path())?;
        if !private_stage_acl_handle(&root)?
            || (reserve && !crate::private_stage::supports_file_publication(&root)?) {
            return Ok(None);
        }
        Ok(Some(DirectoryTransaction {
            stage: Some(StagingDir::Private(temp)),
            root,
            parent: parent_handle,
            restore_acl: true,
            witnessed,
        }))
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = (path, dest, safe_projection, guards, entries, reserve);
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
    metadata_changed: bool,
) -> Result<(), String> {
    ctl.check()?;
    #[cfg(not(windows))]
    let _ = metadata_changed;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let attributes = if entry.is_dir { 0x10 } else { 0x20 }
            | if entry.attributes.contains('R') { 1 } else { 0 };
        let modified = archive_modified_time(&entry.modified);
        // ADS writes can change mtime after this snapshot was read. Never
        // skip restoration based on pre-marking metadata.
        if !metadata_changed
            && meta.file_attributes() == attributes
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
#[cfg(test)]
fn rename_directory_noclobber(root: &File, target: &Path) -> Result<bool, String> {
    rename_directory_noclobber_in(root, target, None)
}
fn rename_directory_noclobber_in(
    root: &File,
    target: &Path,
    parent: Option<&File>,
) -> Result<bool, String> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        #[link(name = "Ntdll")]
        extern "system" {
            fn NtSetInformationFile(
                handle: *mut std::ffi::c_void,
                status: *mut usize,
                info: *const std::ffi::c_void,
                size: u32,
                class: u32,
            ) -> i32;
        }
        #[link(name = "Kernel32")]
        extern "system" {
            fn SetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                class: i32,
                info: *const std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let name = if parent.is_some() {
            target
                .file_name()
                .ok_or("missing destination name")?
                .encode_wide()
                .collect::<Vec<_>>()
        } else {
            target.as_os_str().encode_wide().collect::<Vec<_>>()
        };
        if !target.is_absolute() || name.len() > 32767 || name.contains(&0) {
            return Ok(false);
        }
        let size = 22 + name.len() * 2;
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let raw = buffer.as_mut_ptr().cast::<u8>();
        unsafe {
            if let Some(parent) = parent {
                std::ptr::write_unaligned(
                    raw.add(8).cast::<*mut std::ffi::c_void>(),
                    parent.as_raw_handle(),
                );
            }
            std::ptr::write_unaligned(raw.add(16).cast::<u32>(), (name.len() * 2) as u32);
            std::ptr::copy_nonoverlapping(name.as_ptr().cast::<u8>(), raw.add(20), name.len() * 2);
            let success = if parent.is_some() {
                let mut status = [0usize; 2];
                NtSetInformationFile(
                    root.as_raw_handle(),
                    status.as_mut_ptr(),
                    raw.cast(),
                    size as u32,
                    10,
                ) >= 0
            } else {
                SetFileInformationByHandle(root.as_raw_handle(), 3, raw.cast(), size as u32) != 0
            };
            if success {
                return Ok(true);
            }
        }
        // A competitor's directory is never replaced. Old publication handles
        // existing-destination collision policies after an unsupported commit.
        Ok(false)
    }
    #[cfg(not(all(windows, target_pointer_width = "64")))]
    {
        let _ = (root, target, parent);
        Ok(false)
    }
}

struct OutputTemp {
    #[cfg(windows)]
    inner: crate::private_stage::PrivateOutputFile,
    #[cfg(not(windows))]
    inner: Option<tempfile::NamedTempFile>,
}
impl OutputTemp {
    fn as_file(&self) -> &File {
        #[cfg(windows)] { self.inner.as_file() }
        #[cfg(not(windows))] { self.inner.as_ref().expect("owned output").as_file() }
    }
    fn as_file_mut(&mut self) -> &mut File {
        #[cfg(windows)] { self.inner.as_file_mut() }
        #[cfg(not(windows))] { self.inner.as_mut().expect("owned output").as_file_mut() }
    }
    #[cfg(test)]
    fn path(&self) -> &Path {
        #[cfg(windows)] { self.inner.path() }
        #[cfg(not(windows))] { self.inner.as_ref().expect("owned output").path() }
    }
    fn abort(&mut self) -> Result<(), String> {
        #[cfg(windows)] { self.inner.abort() }
        #[cfg(not(windows))] {
            self.inner.take().map_or(Ok(()), |file| file.close().map_err(|e| e.to_string()))
        }
    }
}
impl std::io::Write for OutputTemp {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.as_file_mut().write(bytes) }
    fn flush(&mut self) -> std::io::Result<()> { self.as_file_mut().flush() }
}
fn output_temp(parent: &Path, guards: &CheckedDirectories) -> Result<OutputTemp, String> {
    #[cfg(windows)] {
        use std::os::windows::fs::OpenOptionsExt;
        guards.check_prepared(parent)?;
        let (held, _) = guards.held.get(parent).ok_or("missing output parent guard")?;
        let pin = fs::OpenOptions::new().access_mode(0x200a0).share_mode(3)
            .custom_flags(0x02200000).open(parent).map_err(|e| e.to_string())?;
        if directory_identity(held)? != directory_identity(&pin)?
            || directory_attributes(&pin)? & 0x410 != 0x10 {
            return Err("output parent identity changed".into());
        }
        Ok(OutputTemp { inner: crate::private_stage::create_output(parent, &pin)? })
    }
    #[cfg(not(windows))] {
        let _ = guards;
        Ok(OutputTemp { inner: Some(tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?) })
    }
}
// Explicit cleanup reports a failed private-temp deletion on every ordinary
// error/cancel path. Drop remains only a last-resort panic safeguard.
fn with_output_temp<T>(parent: &Path, guards: &CheckedDirectories,
    write_and_publish: impl FnOnce(&mut OutputTemp) -> Result<T, String>) -> Result<T, String> {
    let mut temp = output_temp(parent, guards)?;
    let result = write_and_publish(&mut temp);
    match (result, temp.abort()) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(reason), Ok(())) => Err(reason),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(reason), Err(cleanup)) => Err(format!("{reason}; {cleanup}")),
    }
}
fn publish_output(temp: &mut OutputTemp, target: &Path, overwrite: bool) -> Result<File, String> {
    #[cfg(windows)] { temp.inner.commit(target, overwrite) }
    #[cfg(not(windows))] {
        let file = temp.inner.take().ok_or("output already published")?;
        let result = if overwrite { file.persist(target) } else { file.persist_noclobber(target) };
        match result {
            Ok(file) => Ok(file),
            Err(failed) => { temp.inner = Some(failed.file); Err(failed.error.to_string()) },
        }
    }
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
    publish_staged_files_parallel(entries, dest, StagedContents::Disk(stage), ctl, guards, None)
}
fn publish_staged_files_parallel(
    entries: &[&Entry],
    dest: &Path,
    stage: StagedContents<'_>,
    ctl: &JobControl,
    guards: &mut CheckedDirectories,
    zone: Option<&crate::motw::ZoneIdentifier>,
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
                            let committed = with_output_temp(parent, prepared_guards, |temp| {
                                let mut source = stage.reader(&relative)?;
                                loop {
                                    ctl.check()?;
                                    if stopped.load(Ordering::Acquire) { return Ok(false); }
                                    let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
                                    if n == 0 { break; }
                                    std::io::Write::write_all(temp, &buffer[..n]).map_err(|e| e.to_string())?;
                                }
                                crate::motw::apply_to_file(temp.as_file(), zone)?;
                                restore_open_file_metadata(temp.as_file(), entry)?;
                                ctl.check()?;
                                if stopped.load(Ordering::Acquire) { return Ok(false); }
                                prepared_guards.check_prepared(&target)?;
                                drop(publish_output(temp, &target, false)?);
                                Ok(true)
                            })?;
                            if !committed { return Ok(()); }
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
    let policy = crate::machine_policy::scope();
    let ctl = control(opts);
    ctl.check()?;
    ctl.set("testing", None, "Checking archive integrity");
    let mut a = base("t", opts)?;
    let mut source_guard = crate::engine_stream::pin(&absolute(path)?)?;
    let probe = if crate::stream_formats::raw_format(path).is_none() && policy.handler_may_be_enabled("ZealFS") && has_zealfs_handler(&executable()?) {
        Some(crate::zealfs_guard::SourceProbe::read(&mut *source_guard)?)
    } else { None };
    if let Some(format) = crate::stream_formats::raw_format(path) {
        crate::stream_formats::validate_raw_options(opts)?;
        let source = absolute(path)?;
        if policy.restricted() { list_raw_with_source(path, opts, &mut source_guard)?; }
        crate::engine_stream::validate_frame_bounds(&source, format, opts)?;
        if crate::engine_stream::decode_native(&source, format, opts, None, &mut |_| Ok(()))? {
            ctl.set("complete", Some(100), "Archive integrity verified");
            return Ok(());
        }
        a.push(format!("-t{format}"));
    }
    a.extend(["--".into(), absolute(path)?.display().to_string()]);
    if let Some(probe) = probe.filter(|p| p.is_candidate()) {
        let result = run_with_zealfs(&a, None, opts, None)?;
        let kind = crate::zealfs_guard::first_type(&result)?;
        if kind.eq_ignore_ascii_case("ZealFS") || kind == "Split" {
            let (headers, _) = crate::zealfs_guard::header_payload(&result, false)?;
            let image = crate::zealfs_guard::check(&headers, &mut source_guard, &probe)?;
            if policy.restricted() && image.is_some_and(|image| image.entries.iter().any(|e| e.name.contains('/'))) {
                // The official test exit alone cannot prove all nested items were visited.
                list_raw_with_source(path, opts, &mut source_guard)?;
            }
            policy.unchanged()?;
        }
        if !kind.eq_ignore_ascii_case("ZealFS") { run(&a, None, opts, None)?; }
    } else { run(&a, None, opts, None)?; }
    drop(source_guard);
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

fn checked_input_roots(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    if inputs.is_empty() {
        return Err("no input files".into());
    }
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
        result.push(p)
    }
    Ok(result)
}

fn input_paths(inputs: &[String]) -> Result<Vec<PathBuf>, String> {
    let _timer = PhaseTimer::new("input-preflight");
    let paths = checked_input_roots(inputs)?;
    #[cfg(windows)]
    let mut source_guards = SourceValidationGuards::new();
    for path in &paths {
        #[cfg(windows)]
        source_guards.validate(path)?;
        #[cfg(not(windows))]
        validate_tree(path)?;
    }
    Ok(paths)
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
    if !["zip", "7z", "tar", "gzip", "bzip2", "xz", "wim", "sfx", "brotli", "lz4", "lz5", "lizard", "zstd"].contains(&format) {
        return Err("unsupported output format".into());
    }
    if opts.password.is_some() && !matches!(format, "zip" | "7z" | "sfx") {
        return Err("this format does not support passwords".into());
    }
    let method = opts
        .method
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase());
    let raw = crate::stream_formats::canonical_raw_format(format);
    if raw.is_some() { crate::stream_formats::validate_raw_options(opts)?; }
    let selected_method = if let Some(raw) = raw {
        if method.as_deref().is_some_and(|method| crate::stream_formats::canonical_raw_format(method) != Some(raw)) {
            return Err("compression method does not match the raw stream format".into());
        }
        Some(match raw {"brotli"=>"BROTLI","lz4"=>"LZ4","lz5"=>"LZ5","lizard"=>"LIZARD","zstd"=>"ZSTD",_=>unreachable!()})
    } else { match (format, method.as_deref()) {
        (_, None) => None,
        ("zip", Some("deflate")) => Some("Deflate"),
        ("zip", Some("deflate64")) => Some("Deflate64"),
        ("zip" | "7z", Some("zstd" | "zstandard")) => Some("ZSTD"),
        ("zip", Some("store" | "copy")) => Some("Copy"),
        ("zip", Some("bzip2")) => Some("BZip2"),
        ("zip", Some("lzma")) => Some("LZMA"),
        ("zip", Some("ppmd")) => Some("PPMd"),
        ("7z" | "sfx", Some("lzma2")) => Some("LZMA2"),
        ("7z" | "sfx", Some("lzma")) => Some("LZMA"),
        ("7z" | "sfx", Some("ppmd")) => Some("PPMd"),
        ("7z" | "sfx", Some("bzip2")) => Some("BZip2"),
        ("7z" | "sfx", Some("copy" | "store")) => Some("Copy"),
        ("7z", Some("brotli")) => Some("BROTLI"),
        ("7z", Some("lz4")) => Some("LZ4"),
        ("7z", Some("lz5")) => Some("LZ5"),
        ("7z", Some("lizard")) => Some("LIZARD"),
        ("7z", Some("kanzi")) => Some("KANZI"),
        _ => return Err(format!("compression method is unsupported for {format}")),
    }};
    let (minimum_level, maximum_level) = match selected_method {
        // NanaZip's native ZSTD codec maps application levels 0..9 to
        // {-3,-3,-1,1,3,6,10,16,19,22}; these are not direct ZSTD levels.
        Some("ZSTD") => (0, 9),
        Some("BROTLI") => (0, 11),
        Some("LZ4") => (1, 12),
        Some("LZ5") => (1, 15),
        Some("LIZARD") => (10, 49),
        _ => (0, 9),
    };
    if opts.level.is_some_and(|level| level < minimum_level || level > maximum_level) {
        return Err(format!("compression level must be {minimum_level}..={maximum_level} for this method"));
    }
    let extended_method = matches!(
        selected_method,
        Some("ZSTD" | "BROTLI" | "LZ4" | "LZ5" | "LIZARD" | "KANZI")
    );
    if extended_method
        && !executable()?
            .file_name()
            .is_some_and(|name| name == "NanaZip.Universal.Console.exe")
    {
        return Err("this compression method requires the bundled NanaZip engine".into());
    }
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
    if (opts.solid.is_some() || opts.header_encryption.is_some()) && !matches!(format, "7z" | "sfx")
    {
        return Err("solid and header encryption settings require 7z or SFX".into());
    }
    if opts.header_encryption == Some(true) && opts.password.is_none() {
        return Err("header encryption requires a password".into());
    }
    if let Some(bytes) = opts.dictionary_bytes {
        if !matches!(format, "7z" | "sfx")
            || !matches!(selected_method, None | Some("LZMA" | "LZMA2"))
        {
            return Err("dictionary size is supported for 7z/SFX LZMA or LZMA2".into());
        }
        if !(4096..=1024 * 1024 * 1024).contains(&bytes) {
            return Err("dictionary size must be between 4 KiB and 1 GiB".into());
        }
        args.push(format!("-md={bytes}b"));
    }
    if let Some(solid) = opts.solid {
        args.push(format!("-ms={}", if solid { "on" } else { "off" }));
    }
    args.push(format!("-t{}", if format == "sfx" { "7z" } else { format }));
    args.push("-sse".into());
    if let Some(n) = opts.threads {
        args.push(format!("-mmt={n}"));
    }
    if let Some(l) = opts.level {
        args.push(format!("-mx={l}"));
    }
    if let Some(method) = selected_method.filter(|_| raw.is_none()) {
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
            args.push(format!(
                "-mhe={}",
                if opts.header_encryption.unwrap_or(true) {
                    "on"
                } else {
                    "off"
                }
            ));
        }
    }
    if format == "sfx" {
        let stub = sfx_stub()?;
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
    let policy = crate::machine_policy::scope();
    if policy.restricted() && matches!(format, "zipx" | "sfx" | "iso" | "lzh") {
        return Err(format!("{format} creation uses a separate writer and is unavailable under NanaZip machine format policy"));
    }
    let _timer = PhaseTimer::new("create-total");
    let raw_format = crate::stream_formats::canonical_raw_format(format);
    let format = raw_format.unwrap_or(format);
    if raw_format.is_some() {
        crate::stream_formats::validate_raw_options(opts)?;
        if crate::stream_formats::raw_format(out) != raw_format {
            return Err("raw stream filename extension must match its format".into());
        }
    }
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
        // Check final-output containment against the original roots. The TAR
        // creation below validates every descendant immediately before use.
        let paths = checked_input_roots(inputs)?;
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
    #[cfg(nana_selected_worker)]
    if selected_worker::create_zip::eligible(format, inputs, opts)
        && selected_worker::create_zip::try_create(out, inputs, opts, &ctl)? {
        if PhaseTimer::enabled() { eprintln!("OA_CREATE_WORKER status=complete"); }
        ctl.set("complete",Some(100),"Archive created");
        return Ok(vec![absolute(out)?.display().to_string()]);
    }
    let paths = input_paths(inputs)?;
    let out = absolute(out)?;
    no_links(&out)?;
    if out.exists() {
        return Err("output archive already exists; use update to modify it".into());
    }
    if (raw_format.is_some() || matches!(format, "gzip" | "bzip2" | "xz")) && (paths.len() != 1 || !paths[0].is_file()) {
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

// Index names once so a large selection does not repeatedly scan and normalize
// the archive. An implicit directory can be a source; rename targets retain
// their original exact-entry, ASCII-insensitive collision rule.
fn validate_update_names(mut names: Vec<String>, inputs: &[String], rename: bool) -> Result<(), String> {
    for name in &mut names {
        if name.contains('\\') {
            *name = name.replace('\\', "/");
        }
    }
    names.sort_unstable();
    for source in inputs.iter().step_by(if rename { 2 } else { 1 }) {
        let name = source.replace('\\', "/");
        if names.binary_search(&name).is_ok() { continue; }
        let prefix = format!("{}/", name.trim_end_matches('/'));
        let next = names.partition_point(|item| item < &prefix);
        if !names.get(next).is_some_and(|item| item.starts_with(&prefix)) {
            return Err(format!("archive entry not found: {source}"));
        }
    }
    if rename {
        for name in &mut names { name.make_ascii_lowercase(); }
        names.sort_unstable();
        for target in inputs.iter().skip(1).step_by(2) {
            let name = target.replace('\\', "/").to_ascii_lowercase();
            if names.binary_search(&name).is_ok() {
                return Err(format!("rename destination exists: {target}"));
            }
        }
    }
    Ok(())
}

/// action: add, delete, rename (flat old/new pairs).
pub fn update(
    path: &str,
    action: &str,
    inputs: &[String],
    opts: &EngineOptions,
) -> Result<(), String> {
    let _policy = crate::machine_policy::scope();
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
    #[cfg(windows)]
    let source = edit_source::EditSource::open(&locked_path)?;
    zip_volume_view::require_editable(&locked_path)?;
    if action != "add" {
        let existing = list(path, opts)?;
        validate_update_names(existing.into_iter().map(|entry| entry.name).collect(), inputs, action == "rename")?;
    }
    let path = locked_path;
    no_links(&path)?;
    if !path.is_file() {
        return Err("archive not found".into());
    }
    let parent = path.parent().ok_or("missing archive parent")?;
    #[cfg(windows)]
    let output = source.output()?;
    #[cfg(windows)]
    let staged = output.path().to_owned();
    #[cfg(not(windows))]
    let temp = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    let staged = temp.path().join(path.file_name().ok_or("missing archive filename")?);
    #[cfg(windows)]
    let direct = source.direct_zip(&path)?;
    #[cfg(not(windows))]
    let direct = false;
    let mut a = base(command, opts)?;
    if direct {
        a.extend(["-u-".to_owned(), format!("-u!{}", staged.display())]);
    } else {
        fs::copy(&path, &staged).map_err(|e| e.to_string())?;
    }
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
    a.push(if direct { &path } else { &staged }.display().to_string());
    a.extend(names);
    let ctl = control(opts);
    ctl.set("updating", None, "Updating a temporary archive copy");
    run(&a, None, opts, None)?;
    test(staged.to_str().ok_or("invalid archive path")?, opts)?;
    ctl.check()?;
    no_links(&path)?;
    #[cfg(windows)]
    source.commit(output)?;
    #[cfg(not(windows))]
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
/// Preview an independent stream with a caller-selected bound (at most 32 MiB).
/// The decoder is stopped at limit+1 bytes; unknown size never implies unlimited IO.
pub fn preview_raw_stream(path: &str, name: &str, opts: &EngineOptions, limit: u64) -> Result<Vec<u8>, String> {
    let _policy = crate::machine_policy::scope();
    if limit == 0 || limit > 32 * 1024 * 1024 { return Err("invalid raw stream preview limit".into()); }
    let format=crate::stream_formats::raw_format(path).ok_or("raw stream format required")?;
    crate::stream_formats::validate_raw_options(opts)?;
    let absolute=absolute(path)?;
    let mut source=crate::engine_stream::pin(&absolute)?;
    validate_name(name)?;
    let (entries, _)=list_raw_with_source(path,opts,&mut source)?;
    let entry=entries.iter().find(|entry| !entry.is_dir && path_key(Path::new(&entry.name))==path_key(Path::new(name)))
        .ok_or("preview entry not found")?;
    if entry.size_known && entry.size>limit { return Err("preview exceeds size limit".into()); }
    let bytes=crate::engine_stream::preview(&absolute,format,opts,limit)?;
    if entry.size_known && bytes.len() as u64!=entry.size {return Err("preview size mismatch".into());}
    Ok(bytes)
}
fn preview_raw(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    control(opts).check()?;
    if crate::stream_formats::raw_format(path).is_some() {
        return preview_raw_stream(path,name,opts,32*1024*1024);
    }
    validate_name(name)?;
    let mut source = crate::engine_stream::pin(&absolute(path)?)?;
    let (list, payload_source) = list_raw_with_source(path, opts, &mut source)?;
    let e = list
        .iter()
        .find(|e| e.name.replace('\\', "/") == name.replace('\\', "/") && !e.is_dir)
        .ok_or("preview entry not found")?;
    if e.size > 32 * 1024 * 1024 {
        return Err("preview exceeds 32 MiB limit".into());
    }
    if let PayloadSource::VerifiedZeal(mut files) = payload_source {
        return match files.remove(&path_key(Path::new(&e.name))) {
            Some(MemoryChunk::Owned(bytes)) => Ok(bytes),
            _ => Err("missing ZealFS preview entry".into()),
        };
    }
    let tmp = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut a = base("x", opts)?;
    let execute = if matches!(payload_source, PayloadSource::OfficialZeal) {
        a.push("-tZealFS".into());
        run_with_zealfs
    } else { run };
    a.extend([
        "-so".into(),
        "--".into(),
        absolute(path)?.display().to_string(),
        e.name.clone(),
    ]);
    execute(
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
    control(opts).check()?;
    let _source = crate::engine_stream::pin(&absolute(path)?)?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let inner = temp.path().join("contents.tar");
    let mut a = base("x", opts)?;
    // This route promises a declared compressed TAR container. Do not let a
    // renamed filesystem image bypass its typed listing by autodetection here.
    let lower = path.to_ascii_lowercase();
    let format = if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") { "gzip" }
        else if [".tar.bz2", ".tbz", ".tbz2"].iter().any(|s| lower.ends_with(s)) { "bzip2" }
        else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") { "xz" }
        else { return Err("unsupported compressed TAR container".into()); };
    a.push(format!("-t{format}"));
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
    let _policy = crate::machine_policy::scope();
    let read = || with_zip_source(path, opts, |path| {
        if compound_tar(path) {
            let (_temp, inner) = unpack_tar(path, opts)?;
            list_raw(&inner, opts)
        } else {
            list_raw(path, opts)
        }
    });
    let result=read();
    #[cfg(windows)]
    if result.is_err() && edit_source::recover_missing(Path::new(path))? { return read(); }
    result
}
pub fn extract(path: &str, dest: &str, opts: &EngineOptions) -> Result<usize, String> {
    let _policy = crate::machine_policy::scope();
    let mut marked;
    let opts = if opts.zone_identifier.is_none() {
        marked = opts.clone();
        marked.zone_identifier = Some(crate::motw::archive_context(Path::new(path))?);
        &marked
    } else {
        opts
    };
    with_zip_source(path, opts, |path| {
        if compound_tar(path) {
            let (_temp, inner) = unpack_tar(path, opts)?;
            extract_raw(&inner, dest, opts)
        } else {
            extract_raw(path, dest, opts)
        }
    })
}

pub fn test(path: &str, opts: &EngineOptions) -> Result<(), String> {
    let _policy = crate::machine_policy::scope();
    with_zip_source(path, opts, |path| {
        test_raw(path, opts)?;
        if compound_tar(path) {
            let (_temp, inner) = unpack_tar(path, opts)?;
            test_raw(&inner, opts)?;
        }
        Ok(())
    })
}
pub fn preview(path: &str, name: &str, opts: &EngineOptions) -> Result<Vec<u8>, String> {
    let _policy = crate::machine_policy::scope();
    with_zip_source(path, opts, |path| {
        if compound_tar(path) {
            let (_temp, inner) = unpack_tar(path, opts)?;
            preview_raw(&inner, name, opts)
        } else {
            preview_raw(path, name, opts)
        }
    })
}
// Keep the already-open output handle for metadata, avoiding both a second
// path lookup and a path-substitution window between publication and timestamp.
fn archive_modified_time(value: &str) -> Option<std::time::SystemTime> {
    use chrono::TimeZone;
    let utc = value.strip_suffix('Z');
    let date =
        chrono::NaiveDateTime::parse_from_str(utc.unwrap_or(value), "%Y-%m-%d %H:%M:%S%.f").ok()?;
    let date = if utc.is_some() {
        date.and_utc()
    } else {
        // Keep adapters/older external engines' existing local-time semantics.
        chrono::Local
            .from_local_datetime(&date)
            .earliest()?
            .with_timezone(&chrono::Utc)
    };
    let seconds = date.timestamp();
    let nanos = Duration::from_nanos(date.timestamp_subsec_nanos() as u64);
    let whole = if seconds < 0 {
        std::time::UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))?
    } else {
        std::time::UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))?
    };
    whole.checked_add(nanos)
}

fn restore_open_file_metadata(file: &File, entry: &Entry) -> Result<(), String> {
    if let Some(timestamp) = archive_modified_time(&entry.modified) {
        file.set_times(fs::FileTimes::new().set_modified(timestamp))
            .map_err(|e| e.to_string())?;
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
    if let Some(timestamp) = archive_modified_time(&entry.modified) {
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
    fn indexed_update_sources_and_destinations_keep_their_original_matching_rules() {
        let check = |inputs: &[&str], rename| {
            let archive = ["a-obstacle", "dir!unrelated", "dir\\item", "plain.txt", "CASE", "sub/explicit/"];
            validate_update_names(archive.map(str::to_owned).to_vec(),
                &inputs.iter().map(|s| s.to_string()).collect::<Vec<_>>(), rename)
        };
        assert!(check(&["plain.txt", "dir", "sub/explicit/", "dir///", "dir/item"], false).is_ok());
        assert!(check(&["plain.txt/", "missing"], false).unwrap_err().ends_with("plain.txt/"));
        assert!(check(&["DIR"], false).is_err());
        assert!(check(&["dir", "fresh", "plain.txt", "case"], true).unwrap_err().ends_with("case"));
        assert!(check(&["dir", "fresh", "missing", "CASE"], true).unwrap_err().ends_with("missing"));
        assert!(check(&["plain.txt", "dir"], true).is_ok());
    }
    #[test]
    fn large_listing_preserves_record_boundaries_and_global_validation() {
        use std::io::Write;
        let first = "Path = a\nComment = ";
        for boundary in [131071, 131072, 131073] {
            let mut text = first.to_owned();
            text.push_str(&"x".repeat(boundary - first.len()));
            text.push_str("\r\n\r\nPath = 中文\nSize = 1");
            let mut log = tempfile::tempfile().unwrap();
            log.write_all(text.as_bytes()).unwrap();
            log.rewind().unwrap();
            let actual = large_directory(&mut log, &JobControl::new()).unwrap().unwrap();
            let expected = parse_listing(text.as_bytes()).unwrap();
            assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
            log.seek(SeekFrom::End(0)).unwrap();
            log.write_all(b"\n\nPath = A\nSize = 1").unwrap();
            log.rewind().unwrap();
            assert!(large_directory(&mut log, &JobControl::new()).unwrap().is_none());
            assert_eq!(log.stream_position().unwrap(), 0);
            assert!(parse_listing(&read_engine_output(log).unwrap()).unwrap_err().contains("duplicate"));
        }
    }
    #[test]
    fn technical_listing_preserves_mixed_line_endings_and_metadata() {
        let data = b"Path = a\r\nSize = 2\nPacked Size = 1\r\nCRC = ff\n\t\nEncrypted = +\r\nModified = date\nMethod = M\nComment = kept \r\nAttributes = A\r\n\nPath = b/\nFolder = +";
        let entries = parse_listing(data).unwrap();
        assert_eq!(entries.len(), 2);
        let a = &entries[0];
        assert_eq!((&*a.name, a.size, a.size_known, a.packed_size, a.is_dir), ("a", 2, true, 1, false));
        assert_eq!((&*a.crc, a.encrypted, &*a.modified, &*a.method, &*a.comment, &*a.attributes),
                   ("ff", true, "date", "M", "kept ", "A"));
        assert!(entries[1].is_dir && entries[1].size_known && entries[1].crc.is_empty());
        let controls = parse_listing(b"Path = a\r\nComment = inner\r\r\nCRC = c\r\n\r\nPath = b\nComment = end\r\r\n\r\n").unwrap();
        assert_eq!((&*controls[0].comment, &*controls[1].comment), ("inner", "end\r"));
    }

    #[test]
    fn technical_listing_rejects_ambiguous_and_unsafe_records() {
        for data in [
            b"Unknown = 1\r\nUnknown = 2".as_slice(),
            b"Path = a\r\n \nPath = b",
            b"Path = a\r\ninvalid continuation",
            b"Path = a\nHard Link = target",
            b"Path = a\nAlternate Stream = +",
            b"Path = a\nSymbolic Link = target",
            b"Path = ../outside",
            b"Path = a\n\r\nPath = A",
            b"Path = a\nSize = -1",
            b"Path = a\xff",
        ] {
            assert!(parse_listing(data).is_err(), "{data:?}");
        }
    }
    #[cfg(windows)]
    #[test]
    fn abandoned_selected_publication_removes_only_its_own_empty_directories() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("new");
        let abandoned = dest.join("empty");
        let occupied = dest.join("occupied");
        let replacement = dest.join("replacement");
        let mut guards = CheckedDirectories::default();
        guards.selected_output_root = Some(dest.clone());
        for dir in [&abandoned, &occupied, &replacement] {
            guards.create_dirs(dir).unwrap();
        }
        fs::write(occupied.join("external"), b"keep").unwrap();
        guards.held.clear();
        fs::rename(&replacement, dest.join("moved")).unwrap();
        fs::create_dir(&replacement).unwrap();
        drop(guards);
        assert!(!abandoned.exists());
        assert!(replacement.is_dir() && dest.join("moved").is_dir());
        assert_eq!(fs::read(occupied.join("external")).unwrap(), b"keep");

        let empty = temp.path().join("new-empty");
        let mut guards = CheckedDirectories::default();
        guards.selected_output_root = Some(empty.clone());
        guards.create_dirs(&empty.join("child")).unwrap();
        drop(guards);
        assert!(!empty.exists());

        let existing = temp.path().join("existing");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("keep"), b"user-owned").unwrap();
        let mut guards = CheckedDirectories::default();
        guards.selected_output_root = Some(existing.clone());
        guards.create_dirs(&existing.join("new/nested")).unwrap();
        drop(guards);
        assert!(!existing.join("new").exists());
        assert_eq!(fs::read(existing.join("keep")).unwrap(), b"user-owned");
    }

    #[test]
    fn fallback_seven_zip_never_receives_unknown_zealfs_exclusion() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let seven = root.join("../runtime/7zip/7z.exe");
        let archive = root.join("tests/fixtures/original-bandizip/created.zipx");
        assert!(!has_zealfs_handler(&seven));
        assert!(has_zealfs_handler(Path::new("NanaZip.Universal.Console.exe")));
        let args = vec!["l".into(), "-slt".into(), "-ba".into(), "-sccUTF-8".into(),
            "--".into(), archive.display().to_string()];
        let listing = run_program(seven, &args, None, &EngineOptions::default(), None, false).unwrap();
        assert!(parse_listing(&listing).unwrap().iter().any(|entry| entry.name.ends_with("hello.txt")));
    }

    #[test]
    fn file_publication_reservation_keeps_full_copy_fallback_room() {
        let mut entries = parse_listing(b"Path = file\nSize = 123\n\n").unwrap();
        assert_eq!(file_stage_reservation(&entries), Some(246 + 64 * 1024 * 1024 + 4096));
        entries[0].size_known = false;
        assert_eq!(file_stage_reservation(&entries), None);
        entries[0].size_known = true; entries[0].size = u64::MAX;
        assert_eq!(file_stage_reservation(&entries), None);
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn file_publication_engine_route_moves_verified_identity_and_honors_cancel() {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        for cancel in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let archive = temp.path().join("archive.7z");
            fs::write(&archive, b"7z\xbc\xaf'\x1c").unwrap();
            let zone_bytes = b"[ZoneTransfer]\r\nZoneId=3\r\n";
            fs::write(format!("{}:Zone.Identifier", archive.display()), zone_bytes).unwrap();
            let dest = temp.path().join("existing"); fs::create_dir(&dest).unwrap();
            let policy_handle = fs::OpenOptions::new().access_mode(0x60081).share_mode(3)
                .custom_flags(0x02200000).open(&dest).unwrap();
            crate::private_stage::ordinary_file_publication_test_parent(&policy_handle);
            drop(policy_handle);
            fs::write(dest.join("keep"), b"old unrelated file").unwrap();
            let entries = parse_listing(b"Path = nested/file.txt\nSize = 3\nModified = 2023-11-14 22:13:20.1234567Z\nAttributes = R\n\n").unwrap();
            let mut output_guards = CheckedDirectories::default(); output_guards.check(&dest).unwrap();
            let stage = new_file_publication_stage(archive.to_str().unwrap(), &dest, true, &entries, &mut output_guards)
                .unwrap().expect("ordinary NTFS existing destination qualifies");
            assert_eq!(stage.path().parent(), dest.parent());
            let mut stage_guards = stage.guards(None).unwrap();
            let source = stage.path().join("nested/file.txt");
            stage_guards.create_dirs(source.parent().unwrap()).unwrap();
            fs::write(&source, b"abc").unwrap();
            let ctl = JobControl::new();
            let zone = crate::motw::archive_context(&archive).unwrap();
            verify_stage_with_metadata(stage.path(), &expected_tree(&entries).unwrap(), &ctl,
                &mut stage_guards, Some(&entries), Some(&zone)).unwrap();
            let source_pin = File::open(&source).unwrap();
            let before = directory_identity(&source_pin).unwrap();
            let source_meta = crate::private_stage::checked_metadata(&source_pin).unwrap();
            assert_eq!(source_meta.len(), entries[0].size);
            assert_eq!(source_meta.file_attributes(), 0x21);
            assert_eq!(source_meta.modified().unwrap(), archive_modified_time(&entries[0].modified).unwrap());
            drop(source_pin);
            assert!(stage_guards.private_metadata_parent(source.parent().unwrap()).is_some(), "verified source parent must retain its strong private pin");
            let target = dest.join("nested/file.txt");
            output_guards.create_dirs(target.parent().unwrap()).unwrap();
            let target_parent = fs::OpenOptions::new().access_mode(0x200a0).share_mode(3)
                .custom_flags(0x02200000).open(target.parent().unwrap()).unwrap();
            let target_attrs = directory_attributes(&target_parent).unwrap();
            let target_auto_acl = crate::private_stage::supports_inheritance(&target_parent).unwrap();
            assert!(crate::private_stage::supports_file_publication(&target_parent).unwrap(), "target storage attributes: {target_attrs:#x}");
            assert!(target_auto_acl, "fixture must have an ordinary automatic-inheritance target DACL; attrs={target_attrs:#x}");
            assert_eq!(directory_identity(&target_parent).unwrap().0, before.0, "fixture must be on one volume");
            drop(target_parent);
            if cancel { ctl.cancel(); }
            let result = try_publish_staged_file(StagedContents::Disk(stage.path()), Path::new("nested/file.txt"),
                &entries[0], &target, false, &ctl, &mut stage_guards, &mut output_guards);
            if cancel {
                assert!(matches!(result, Err(message) if message == "operation cancelled"));
                assert!(source.exists()); assert!(!target.exists());
            } else {
                assert!(result.unwrap(), "qualified move returned Unavailable: target_attrs={target_attrs:#x}, target_auto_acl={target_auto_acl}, source_attrs={:#x}, source_mtime={:?}, source_size={}, source_exists={}, target_exists={}", source_meta.file_attributes(), source_meta.modified(), source_meta.len(), source.exists(), target.exists());
                assert!(!source.exists());
                assert_eq!(directory_identity(&File::open(&target).unwrap()).unwrap(), before);
                assert_eq!(fs::read(&target).unwrap(), b"abc");
                assert_eq!(fs::read(format!("{}:Zone.Identifier", target.display())).unwrap(), zone_bytes);
                assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), archive_modified_time(&entries[0].modified).unwrap());
                assert!(fs::metadata(&target).unwrap().permissions().readonly());
            }
            assert_eq!(fs::read(&archive).unwrap(), b"7z\xbc\xaf'\x1c");
            assert_eq!(fs::read(dest.join("keep")).unwrap(), b"old unrelated file");
            let remaining = if cancel { &source } else { &target };
            let attributes = fs::OpenOptions::new().access_mode(0x100).share_mode(7).open(remaining).unwrap();
            let mut permissions = attributes.metadata().unwrap().permissions(); permissions.set_readonly(false);
            attributes.set_permissions(permissions).unwrap(); drop(attributes);
            drop(stage_guards); drop(output_guards); drop(stage);
        }
    }

    #[cfg(windows)]
    #[test]
    fn file_publication_existing_extraction_keeps_conflicts_source_and_motw() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("bundle"); fs::create_dir(&input).unwrap();
        fs::write(input.join("a.txt"), b"new alpha").unwrap();
        fs::write(input.join("b.txt"), b"new beta").unwrap();
        let modified = std::time::UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_700);
        for path in [input.join("a.txt"), input.join("b.txt")] {
            fs::OpenOptions::new().access_mode(0x100).open(path).unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified)).unwrap();
        }
        for format in ["zip", "7z"] {
            let archive = temp.path().join(format!("bundle.{format}"));
            let mut opts = EngineOptions { password: Some("file-publication-fixture".into()), level: Some(0), ..Default::default() };
            create(archive.to_str().unwrap(), &[input.display().to_string()], format, &opts).unwrap();
            let archive_bytes = fs::read(&archive).unwrap();
            let zone = b"[ZoneTransfer]\r\nZoneId=3\r\n";
            fs::write(format!("{}:Zone.Identifier", archive.display()), zone).unwrap();
            for policy in ["error", "skip", "rename", "overwrite"] {
                let dest = temp.path().join(format!("{format}-{policy}")); fs::create_dir_all(dest.join("bundle")).unwrap();
                fs::write(dest.join("bundle/a.txt"), b"old alpha sentinel").unwrap();
                fs::write(dest.join("keep"), b"unrelated sentinel").unwrap();
                opts.overwrite = policy.into();
                let result = extract(archive.to_str().unwrap(), dest.to_str().unwrap(), &opts);
                if policy == "error" {
                    assert!(result.is_err()); assert!(!dest.join("bundle/b.txt").exists());
                } else {
                    assert_eq!(result.unwrap(), if policy == "skip" { 1 } else { 2 });
                    assert_eq!(fs::read(dest.join("bundle/b.txt")).unwrap(), b"new beta");
                    assert_eq!(fs::metadata(dest.join("bundle/b.txt")).unwrap().modified().unwrap(), modified);
                    assert_eq!(fs::read(format!("{}:Zone.Identifier", dest.join("bundle/b.txt").display())).unwrap(), zone);
                }
                let a = dest.join(if policy == "rename" { "bundle/a (1).txt" } else { "bundle/a.txt" });
                if policy == "overwrite" || policy == "rename" {
                    assert_eq!(fs::read(a).unwrap(), b"new alpha");
                }
                if policy != "overwrite" { assert_eq!(fs::read(dest.join("bundle/a.txt")).unwrap(), b"old alpha sentinel"); }
                assert_eq!(fs::read(dest.join("keep")).unwrap(), b"unrelated sentinel");
                assert_eq!(fs::read(&archive).unwrap(), archive_bytes);
            }
        }
    }

    #[test]
    fn utc_metadata_parser_preserves_100ns_and_legacy_local_dates() {
        use chrono::TimeZone;
        assert_eq!(
            archive_modified_time("1970-01-01 00:00:00.1234567Z"),
            Some(std::time::UNIX_EPOCH + Duration::from_nanos(123_456_700))
        );
        assert_eq!(
            archive_modified_time("1969-12-31 23:59:59.9999999Z"),
            Some(std::time::UNIX_EPOCH - Duration::from_nanos(100))
        );
        // Distinct instants in a DST fold stay distinct; UTC has no ambiguity.
        let before = archive_modified_time("2024-11-03 05:30:00.1234567Z").unwrap();
        let after = archive_modified_time("2024-11-03 06:30:00.1234567Z").unwrap();
        assert_eq!(
            after.duration_since(before).unwrap(),
            Duration::from_secs(3600)
        );
        for value in [
            "2024-03-10 02:30:00.1234567",
            "2024-11-03 01:30:00.7654321",
            "2024-01-01 10:20:30",
            "2024-07-01 10:20:30.001",
        ] {
            let date =
                chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f").unwrap();
            let old = chrono::Local
                .from_local_datetime(&date)
                .earliest()
                .map(std::time::SystemTime::from);
            assert_eq!(
                archive_modified_time(value),
                old,
                "legacy local semantics: {value}"
            );
        }
        for value in [
            "",
            "invalid",
            "2024-02-30 00:00:00Z",
            "2024-01-01 00:00:00ZZ",
        ] {
            assert!(archive_modified_time(value).is_none(), "{value}");
        }
    }

    #[test]
    fn utc_listing_fallback_is_only_for_explicit_unknown_switch() {
        for detail in ["Unknown switch:\nslmu", "Unsupported switch:\r\n-slmu"] {
            assert!(unsupported_utc_listing_switch(&format!(
                "archive engine failed (7): banner\nCommand Line Error:\n{detail}\n"
            )));
        }
        for error in [
            "archive engine failed (2): Command Line Error:\nUnknown switch:\nslmu",
            "archive engine failed (7): Wrong password",
            "operation cancelled",
            "archive engine failed (7): Command Line Error:\nUnknown switch:\nmcp",
            "archive engine failed (7): Command Line Error:\nUnknown switch:\nslmu\nData error",
            "archive engine failed (7): slmu is unsupported",
        ] {
            assert!(!unsupported_utc_listing_switch(error), "{error}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn utc_metadata_normalize_keeps_exact_timestamp_but_repairs_attributes() {
        use std::os::windows::fs::MetadataExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file");
        fs::write(&path, b"data").unwrap();
        let mut entry = parse_listing(
            b"Path = file\nSize = 4\nModified = 1970-01-01 00:00:00.1234567Z\nAttributes = R\n\n",
        )
        .unwrap()
        .remove(0);
        let expected = std::time::UNIX_EPOCH + Duration::from_nanos(123_456_700);
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(expected))
            .unwrap();
        let mut guards = CheckedDirectories::owned_root(temp.path());
        normalize_staged_entry(
            &path,
            &entry,
            &fs::metadata(&path).unwrap(),
            &JobControl::default(),
            &mut guards,
            false,
        )
        .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().file_attributes(), 0x21);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), expected);
        entry.attributes = "A".into();
        normalize_staged_entry(
            &path,
            &entry,
            &fs::metadata(&path).unwrap(),
            &JobControl::default(),
            &mut guards,
            false,
        )
        .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().file_attributes(), 0x20);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), expected);
    }

    #[cfg(windows)]
    #[test]
    fn utc_metadata_roundtrips_native_staged_partial_existing_and_motw() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("bundle");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a.txt"), b"alpha").unwrap();
        fs::write(source.join("b.txt"), b"beta").unwrap();
        let timestamp = std::time::UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_700);
        for path in [&source, &source.join("a.txt"), &source.join("b.txt")] {
            fs::OpenOptions::new()
                .access_mode(0x100)
                .custom_flags(0x02000000)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(timestamp))
                .unwrap();
        }
        let readonly = source.join("a.txt");
        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&readonly, permissions).unwrap();
        let zone = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        for format in ["zip", "7z"] {
            for encrypted in [false, true] {
                let archive = temp.path().join(format!("{format}-{encrypted}.{format}"));
                let options = EngineOptions {
                    level: Some(0),
                    solid: (format == "7z").then_some(false),
                    password: encrypted.then(|| "metadata-test-password".into()),
                    ..Default::default()
                };
                create(
                    archive.to_str().unwrap(),
                    &[source.display().to_string()],
                    format,
                    &options,
                )
                .unwrap();
                let listing = list_raw(archive.to_str().unwrap(), &options).unwrap();
                assert_eq!(listing.len(), 3);
                for entry in &listing {
                    assert!(entry.modified.ends_with('Z'), "UTC listing expected");
                    assert_eq!(archive_modified_time(&entry.modified), Some(timestamp));
                }
                for mode in ["new", "existing", "selected", "marked"] {
                    let target = temp.path().join(format!("out-{format}-{encrypted}-{mode}"));
                    if mode == "existing" {
                        fs::create_dir(&target).unwrap();
                        fs::write(target.join("keep"), b"sentinel").unwrap();
                    }
                    if mode == "marked" {
                        fs::write(format!("{}:Zone.Identifier", archive.display()), zone).unwrap();
                    }
                    let mut extract_options = options.clone();
                    if mode == "selected" {
                        extract_options.selected = vec!["bundle/a.txt".into()];
                    }
                    extract(
                        archive.to_str().unwrap(),
                        target.to_str().unwrap(),
                        &extract_options,
                    )
                    .unwrap();
                    let out = target.join("bundle/a.txt");
                    assert_eq!(fs::read(&out).unwrap(), b"alpha");
                    assert_eq!(
                        fs::metadata(&out).unwrap().modified().unwrap(),
                        timestamp,
                        "{format} encrypted={encrypted} mode={mode}"
                    );
                    assert!(fs::metadata(&out).unwrap().permissions().readonly());
                    if mode != "selected" {
                        assert_eq!(fs::read(target.join("bundle/b.txt")).unwrap(), b"beta");
                        assert_eq!(
                            fs::metadata(target.join("bundle/b.txt"))
                                .unwrap()
                                .modified()
                                .unwrap(),
                            timestamp
                        );
                        assert_eq!(
                            fs::metadata(target.join("bundle"))
                                .unwrap()
                                .modified()
                                .unwrap(),
                            timestamp
                        );
                    } else {
                        assert!(!target.join("bundle/b.txt").exists());
                    }
                    if mode == "existing" {
                        assert_eq!(fs::read(target.join("keep")).unwrap(), b"sentinel");
                    }
                    if mode == "marked" {
                        assert_eq!(
                            fs::read(format!("{}:Zone.Identifier", out.display())).unwrap(),
                            zone
                        );
                    }
                    let mut permissions = fs::metadata(&out).unwrap().permissions();
                    permissions.set_readonly(false);
                    fs::set_permissions(&out, permissions).unwrap();
                }
            }
        }
        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&readonly, permissions).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn private_fallback_stage_rejects_preoccupied_hardlinks_and_cleans_on_cancel() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("shared-temp");
        fs::create_dir(&shared).unwrap();
        let icacls = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/icacls.exe");
        let result = std::process::Command::new(icacls).arg(&shared)
            .args(["/inheritance:r", "/grant:r", "*S-1-1-0:(OI)(CI)F"])
            .output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert!(!private_stage_acl(&shared).unwrap());
        let sentinel = temp.path().join("sentinel.txt");
        fs::write(&sentinel, b"untouched external data").unwrap();
        let mut output_guards = CheckedDirectories::default();
        let stage = new_fallback_stage(&shared, &mut output_guards).unwrap();
        let stage_path = stage.path().to_owned();
        assert!(private_stage_acl_handle(stage.private_root().unwrap()).unwrap());
        let mut guards = stage.guards(None).unwrap();
        assert!(guards.private_metadata_parent(stage.path()).is_some());
        assert!(fs::rename(stage.path(), shared.join("stolen")).is_err());

        // The new root's continuous pin already prevents hard-link insertion.
        assert!(fs::hard_link(&sentinel, stage.path().join("occupied.txt")).is_err());
        // Also exercise the output helper with a preoccupied legacy-style
        // directory: create_new must fail without touching the external file.
        let occupied = tempfile::tempdir().unwrap();
        let target = occupied.path().join("occupied.txt");
        fs::hard_link(&sentinel, &target).unwrap();
        let mut occupied_guards = CheckedDirectories::owned_root(occupied.path());
        assert!(create_staged_output(&target, &mut occupied_guards).is_err());
        let expected = BTreeMap::from([("occupied.txt".into(), (false, 23))]);
        let error = verify_stage_checked(occupied.path(), &expected, &JobControl::default(), &mut occupied_guards).unwrap_err();
        assert!(error.contains("hard linked"), "{error}");
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched external data");
        drop(occupied_guards);
        drop(occupied);

        let file = stage.path().join("nested/a.txt");
        create_staged_output(&file, &mut guards).unwrap().write_all(b"data").unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();
        let ctl = JobControl::default();
        ctl.cancel();
        assert!(verify_stage_checked(stage.path(), &BTreeMap::new(), &ctl, &mut guards).is_err());
        drop(guards);
        drop(stage);
        assert!(!stage_path.exists(), "cancelled fallback stage must be removed");
        assert_eq!(fs::read(&sentinel).unwrap(), b"untouched external data");
        drop(output_guards);
    }

    #[cfg(windows)]
    #[test]
    fn hardlinked_stage_is_rejected_before_zone_or_attribute_changes() {
        let temp = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("sentinel.txt");
        fs::write(&sentinel, b"sentinel").unwrap();
        fs::hard_link(&sentinel, stage.path().join("a.txt")).unwrap();
        let source = temp.path().join("source.zip");
        fs::write(&source, b"source").unwrap();
        fs::write(format!("{}:Zone.Identifier", source.display()), b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
        let zone = crate::motw::archive_context(&source).unwrap();
        let mut entry = Entry { name: "a.txt".into(), size: 8, size_known: true, packed_size: 0,
            is_dir: false, crc: String::new(), encrypted: false, modified: String::new(),
            method: String::new(), comment: String::new(), attributes: "R".into() };
        entry.modified = "2019-01-02 03:04:05".into();
        let before = fs::metadata(&sentinel).unwrap();
        let expected = expected_tree(std::slice::from_ref(&entry)).unwrap();
        let error = verify_stage_with_metadata(stage.path(), &expected, &JobControl::default(),
            &mut CheckedDirectories::default(), Some(&[entry]), Some(&zone)).unwrap_err();
        assert!(error.contains("hard linked"), "{error}");
        let after = fs::metadata(&sentinel).unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"sentinel");
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(before.permissions().readonly(), after.permissions().readonly());
        assert!(!PathBuf::from(format!("{}:Zone.Identifier", sentinel.display())).exists());
    }

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
            None,
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
            Some(&listed),
            None,
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
    #[test]
    fn private_relative_metadata_checks_current_leaf_and_keeps_shared_stages_ineligible() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let parent = fs::OpenOptions::new().access_mode(0x200a0).share_mode(3)
            .custom_flags(0x02200000).open(t.path()).unwrap();
        let stage = crate::private_stage::create(t.path(), &parent).unwrap();
        let leaf = stage.path().join("leaf");
        fs::write(&leaf, b"abc").unwrap();
        let mut guards = CheckedDirectories::owned_root(stage.path());
        guards.held.insert(stage.path().to_owned(), (stage.root().try_clone().unwrap(), false));
        assert!(guards.private_metadata_parent(stage.path()).is_none(), "owned_root alone must never enable relative metadata");
        assert!(private_stage_acl_handle(stage.root()).unwrap());
        guards.private_stages.insert(stage.path().to_owned());
        assert!(guards.private_metadata_parent(stage.path()).is_some());
        assert!(fs::rename(stage.path(), t.path().join("swapped")).is_err());
        assert!(fs::OpenOptions::new().access_mode(0x40000000).share_mode(7)
            .custom_flags(0x02200000).open(stage.path()).is_err());
        let stale = fs::read_dir(stage.path()).unwrap().next().unwrap().unwrap();
        assert_eq!(stale.metadata().unwrap().len(), 3);
        fs::write(&leaf, b"changed size").unwrap();
        let current = checked_staged_metadata_with_relative(&leaf, &mut guards, true).unwrap();
        assert_eq!(current.len(), 12);
        let writer = fs::OpenOptions::new().write(true).share_mode(7).open(&leaf).unwrap();
        assert!(crate::private_stage::child_metadata(stage.root(), std::ffi::OsStr::new("leaf")).is_err());
        assert_eq!(checked_staged_metadata_with_relative(&leaf, &mut guards, true).unwrap().len(), 12,
            "native sharing failure must retain the original complete-check path");
        drop(writer);
        let expected = BTreeMap::from([("leaf".to_owned(), (false, 3))]);
        assert!(verify_stage_checked(stage.path(), &expected, &JobControl::new(), &mut guards).is_err());
        let mut readonly = current.permissions(); readonly.set_readonly(true);
        fs::set_permissions(&leaf, readonly).unwrap();
        assert!(checked_staged_metadata_with_relative(&leaf, &mut guards, true).unwrap().permissions().readonly());
        fs::remove_file(&leaf).unwrap();
        fs::create_dir(&leaf).unwrap();
        assert!(checked_staged_metadata_with_relative(&leaf, &mut guards, true).unwrap().is_dir());
        assert!(verify_stage_checked(stage.path(), &expected, &JobControl::new(), &mut guards).is_err());
        fs::remove_dir(&leaf).unwrap();
        let outside = t.path().join("outside"); fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"untouched").unwrap();
        let link = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&leaf).arg(&outside).output().unwrap();
        assert!(link.status.success());
        assert!(checked_staged_metadata_with_relative(&leaf, &mut guards, true).unwrap_err().contains("linked decoded"));
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"untouched");
        fs::remove_dir(&leaf).unwrap();
        drop(guards);
    }

    #[cfg(windows)]
    #[test]
    fn relative_metadata_rejects_multi_component_and_stream_names() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let parent = fs::OpenOptions::new().access_mode(0x200a0).share_mode(3)
            .custom_flags(0x02200000).open(t.path()).unwrap();
        for name in ["", ".", "..", "child/leaf", "child\\leaf", "file:Zone.Identifier", "nul\0suffix"] {
            assert!(crate::private_stage::child_metadata(&parent, std::ffi::OsStr::new(name)).is_err(), "{name:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn committed_permission_pins_really_deny_file_writers_delete_and_replacement() {
        use std::os::windows::fs::OpenOptionsExt;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("published"); fs::create_dir(&root).unwrap();
        let file = root.join("file"); fs::write(&file, b"abc").unwrap();
        let expected = BTreeMap::from([("file".to_owned(), (false, 3))]);
        let pins = pin_committed_tree(&root, &expected).unwrap();
        assert!(fs::OpenOptions::new().write(true).share_mode(7).open(&file).is_err());
        assert!(fs::rename(&file, t.path().join("replaced")).is_err());
        assert!(fs::remove_file(&file).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"abc");
        drop(pins);
        fs::write(&file, b"def").unwrap();
        let alias = t.path().join("external-link"); fs::hard_link(&file, &alias).unwrap();
        assert!(pin_committed_tree(&root, &expected).err().unwrap().contains("hard linked"));
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
        let transaction = new_directory_stage(
            archive.to_str().unwrap(),
            &target,
            true,
            &mut guards,
            &[], false,
        )
        .unwrap()
        .unwrap();
        let stage = transaction.stage.unwrap();
        let root = transaction.root;
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
    fn directory_transaction_matches_original_acl_metadata_and_shared_parent_is_private() {
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
            #[link(name="Advapi32")]
            extern "system" {
                fn SetNamedSecurityInfoW(path:*mut u16, kind:u32, info:u32, owner:*mut c_void, group:*mut c_void, dacl:*mut c_void, sacl:*mut c_void)->u32;
                fn GetSecurityDescriptorDacl(descriptor:*mut c_void,present:*mut i32,dacl:*mut *mut c_void,defaulted:*mut i32)->i32;
            }
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
            if sddl.starts_with("D:PAI") {
                let(mut present,mut defaulted,mut acl)=(0,0,std::ptr::null_mut());
                assert_ne!(unsafe {GetSecurityDescriptorDacl(sd,&mut present,&mut acl,&mut defaulted)},0);
                assert_eq!(unsafe {SetNamedSecurityInfoW(wide(p).as_mut_ptr(),1,0x80000004,std::ptr::null_mut(),std::ptr::null_mut(),acl,std::ptr::null_mut())},0);
            }
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
            true,
            &mut CheckedDirectories::default(),
            &[], false,
        )
        .unwrap()
        .expect("private parent must qualify");
        let staged_path = candidate.stage.as_ref().unwrap().path().to_path_buf();
        drop(candidate.root);
        drop(candidate.stage);
        assert!(!staged_path.exists());
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
        let oracle_root = parent.join("oracle");
        fs::create_dir_all(oracle_root.join("nested/deep")).unwrap();
        let oracle_file = oracle_root.join("nested/deep/ordinary.txt");
        fs::write(&oracle_file, b"content").unwrap();
        let created_acl = dacl(&oracle_file);
        fs::rename(&oracle_file, oracle_root.join("nested/deep/a.txt")).unwrap();
        assert_eq!(dacl(&oracle_root.join("nested/deep/a.txt")), created_acl, "ordinary same-parent rename oracle");
        assert_eq!(dacl(&old.join("nested/deep/a.txt")), created_acl, "copy output against ordinary file oracle");
        assert_eq!(dacl(&new.join("nested/deep/a.txt")), created_acl, "directory commit against ordinary file oracle");
        fs::remove_dir_all(&oracle_root).unwrap();
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
        assert!(new_directory_stage(archive.to_str().unwrap(), &shared.join("legacy"),
            true, &mut CheckedDirectories::default(), &[], false).unwrap().is_none());
        security(&shared, "D:PAI(A;OICI;FA;;;WD)");
        let candidate = new_directory_stage(
            archive.to_str().unwrap(),
            &shared.join("result"),
            true,
            &mut CheckedDirectories::default(),
            &[], false,
        )
        .unwrap().expect("shared parent must use a private stage");
        assert!(private_stage_acl_handle(&candidate.root).unwrap());
        let stage = candidate.stage.unwrap();
        assert_eq!(stage.path().parent(), Some(shared.as_path()));
        let final_path = shared.join("committed");
        fs::write(stage.path().join("data"), b"verified").unwrap();
        assert!(rename_directory_noclobber_in(&candidate.root, &final_path, Some(&candidate.parent)).unwrap());
        stage.keep();
        let expected=BTreeMap::from([("data".into(),(false,8))]);
        let pins=pin_committed_tree(&final_path,&expected).unwrap();
        crate::private_stage::inherit_tree(&candidate.root, &candidate.parent,&pins).unwrap();
        drop(pins);
        drop(candidate.root);
        let normal = shared.join("normal");
        fs::create_dir(&normal).unwrap();
        fs::write(normal.join("data"), b"verified").unwrap();
        assert_eq!(dacl(&final_path), dacl(&normal));
        assert_eq!(dacl(&final_path.join("data")), dacl(&normal.join("data")));
        let shared_existing=shared.join("existing");
        let shared_new=shared.join("new");
        fs::create_dir(&shared_existing).unwrap();
        for destination in [&shared_existing,&shared_new] {
            extract(archive.to_str().unwrap(),destination.to_str().unwrap(),&EngineOptions::default()).unwrap();
        }
        for rel in ["","nested","nested/deep","nested/deep/a.txt","empty"] {
            assert_eq!(dacl(&shared_existing.join(rel)),dacl(&shared_new.join(rel)),"shared DACL {rel}");
        }
        assert!(fs::metadata(shared_new.join("nested/deep/a.txt")).unwrap().permissions().readonly());
        let failed=shared.join("failed");
        assert!(extract(broken.to_str().unwrap(),failed.to_str().unwrap(),&EngineOptions::default()).is_err());
        assert!(!failed.exists());
        assert_eq!(fs::read_dir(&shared).unwrap().count(),4,"failed shared stage must clean up");
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    fn memory_test_parent(path: &Path) -> File {
        use std::os::windows::fs::OpenOptionsExt;
        fs::create_dir(path).unwrap();
        let pin = fs::OpenOptions::new().access_mode(0x600a1).share_mode(7)
            .custom_flags(0x02200000).open(path).unwrap();
        crate::private_stage::ordinary_file_publication_test_parent(&pin);
        pin
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn memory_directory_transaction_preserves_verified_bytes_metadata_and_zone() {
        let (fixture, archive, mut entries) = memory_zip_fixture();
        let parent = fixture.path().join("ordinary");
        let _parent = memory_test_parent(&parent);
        let destination = parent.join("result");
        let opts = EngineOptions::default();
        let control = JobControl::new();
        let zone_bytes = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        fs::write(format!("{}:Zone.Identifier", archive.display()), zone_bytes).unwrap();
        let zone = crate::motw::archive_context(&archive).unwrap();
        let files = decode_memory_zip(memory_archive_source(archive.to_str().unwrap(), &opts),
            &entries, &opts, &control).unwrap().unwrap();
        let precise = "2024-12-30 12:34:56.1234567Z";
        entries[0].attributes = "R".into();
        entries[0].modified = precise.into();
        entries.last_mut().unwrap().modified = precise.into();
        let mut output_guards = CheckedDirectories::default();
        let transaction = new_directory_stage(archive.to_str().unwrap(), &destination, opts.selected.is_empty(),
            &mut output_guards, &entries, true).unwrap().expect("ordinary NTFS destination");
        let stage = transaction.stage.unwrap();
        let mut guards = stage.guards(Some(&transaction.root)).unwrap();
        stage_verified_memory(&files, &entries, stage.path(), &mut guards, &control, Some(4)).unwrap();
        let expected = expected_tree(&entries).unwrap();
        verify_stage_with_metadata(stage.path(), &expected, &control, &mut guards, Some(&entries), Some(&zone)).unwrap();
        drop(guards);
        assert!(rename_directory_noclobber_in(&transaction.root, &destination, Some(&transaction.parent)).unwrap());
        stage.keep();
        let pins = pin_committed_tree(&destination, &expected).unwrap();
        crate::private_stage::inherit_tree(&transaction.root, &transaction.parent, &pins).unwrap();
        drop(pins);
        for entry in entries.iter().filter(|entry| !entry.is_dir) {
            let relative = validate_name(&entry.name).unwrap();
            let output = destination.join(&relative);
            assert_eq!(fs::read(&output).unwrap(), files[&path_key(&relative)].as_slice());
            assert_eq!(fs::read(format!("{}:Zone.Identifier", output.display())).unwrap(), zone_bytes);
        }
        let empty_file = destination.join(&entries[0].name);
        let metadata = fs::metadata(&empty_file).unwrap();
        assert!(metadata.permissions().readonly());
        assert_eq!(metadata.modified().unwrap(), archive_modified_time(precise).unwrap());
        assert_eq!(fs::metadata(destination.join("empty")).unwrap().modified().unwrap(), archive_modified_time(precise).unwrap());
        assert_eq!(fs::read_dir(destination.join("empty")).unwrap().count(), 0);
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn memory_directory_transaction_keeps_ineligible_outputs_on_per_file_path() {
        use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
        #[link(name = "Kernel32")]
        extern "system" { fn SetFileAttributesW(path: *const u16, attrs: u32) -> i32; }
        let (fixture, archive, entries) = memory_zip_fixture();
        let parent = fixture.path().join("ordinary");
        let _parent = memory_test_parent(&parent);
        let target = parent.join("target");
        let candidate = |options: &EngineOptions, items: &[Entry]| {
            new_directory_stage(archive.to_str().unwrap(), &target, options.selected.is_empty(),
                &mut CheckedDirectories::default(), items, true).unwrap()
        };
        let opts = EngineOptions::default();
        assert!(candidate(&opts, &entries).is_some());
        #[link(name = "Advapi32")]
        extern "system" {
            fn ImpersonateSelf(level: i32) -> i32;
            fn RevertToSelf() -> i32;
        }
        struct Revert;
        impl Drop for Revert {
            fn drop(&mut self) { assert_ne!(unsafe { RevertToSelf() }, 0); }
        }
        {
            assert_ne!(unsafe { ImpersonateSelf(2) }, 0);
            let _revert = Revert;
            assert!(candidate(&opts, &entries).is_none(), "keep the effective-token publisher");
        }
        let selected = EngineOptions { selected: vec![entries[0].name.clone()], ..Default::default() };
        assert!(candidate(&selected, &entries).is_none());
        let mut too_large = entries[0].clone();
        too_large.size = u64::MAX / 4;
        assert!(candidate(&opts, &[too_large]).is_none());
        let normal = fs::metadata(&parent).unwrap().file_attributes();
        let wide: Vec<_> = parent.as_os_str().encode_wide().chain(Some(0)).collect();
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), normal | 0x2000) }, 0);
        assert!(candidate(&opts, &entries).is_none(), "preserve inherited non-indexed attribute");
        assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), normal) }, 0);
        private_test_directory(&parent); // Legacy, non-automatic DACL.
        assert!(candidate(&opts, &entries).is_none());
        fs::create_dir(&target).unwrap();
        assert!(candidate(&opts, &entries).is_none());
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn memory_directory_transaction_cancellation_and_invalid_bytes_keep_destination_absent() {
        let (fixture, archive, mut entries) = memory_zip_fixture();
        let parent = fixture.path().join("ordinary");
        let _parent = memory_test_parent(&parent);
        let destination = parent.join("result");
        let opts = EngineOptions::default();
        let control = JobControl::new();
        let mut files = decode_memory_zip(memory_archive_source(archive.to_str().unwrap(), &opts),
            &entries, &opts, &control).unwrap().unwrap();
        let previous = path_key(&validate_name(&entries[0].name).unwrap());
        let empty = files.remove(&previous).unwrap();
        entries[0].name = "readonly-at-root.txt".into();
        entries[0].attributes = "R".into();
        files.insert(path_key(Path::new(&entries[0].name)), empty);
        let abandoned;
        {
            let transaction = new_directory_stage(archive.to_str().unwrap(), &destination, opts.selected.is_empty(),
                &mut CheckedDirectories::default(), &entries, true).unwrap().unwrap();
            let stage = transaction.stage.as_ref().unwrap();
            abandoned = stage.path().to_path_buf();
            let mut guards = stage.guards(Some(&transaction.root)).unwrap();
            let cancelled = JobControl::new();
            cancelled.cancel();
            assert!(stage_verified_memory(&files, &entries, stage.path(), &mut guards, &cancelled, Some(1))
                .unwrap_err().contains("cancelled"));
            assert_eq!(fs::read_dir(stage.path()).unwrap().count(), 0);
            entries[2].size += 1;
            assert!(stage_verified_memory(&files, &entries, stage.path(), &mut guards, &control, Some(1))
                .unwrap_err().contains("length"));
            assert!(stage.path().join(&entries[0].name).is_file());
        }
        assert!(!abandoned.exists());
        assert!(!destination.exists());
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn failed_parallel_memory_writes_leave_private_stage_and_hard_link_safe() {
        let (fixture, archive, entries) = memory_zip_fixture();
        let parent = fixture.path().join("ordinary");
        let _parent = memory_test_parent(&parent);
        let destination = parent.join("result");
        let sentinel = fixture.path().join("sentinel.txt");
        fs::write(&sentinel, b"unchanged").unwrap();
        let opts = EngineOptions::default();
        let ctl = JobControl::new();
        let files = decode_memory_zip(memory_archive_source(archive.to_str().unwrap(), &opts),
            &entries, &opts, &ctl).unwrap().unwrap();
        let abandoned;
        {
            let transaction = new_directory_stage(archive.to_str().unwrap(), &destination, opts.selected.is_empty(),
                &mut CheckedDirectories::default(), &entries, true).unwrap().unwrap();
            let stage = transaction.stage.as_ref().unwrap();
            abandoned = stage.path().to_path_buf();
            fs::create_dir(stage.path().join("folder")).unwrap();
            fs::hard_link(&sentinel, stage.path().join("folder/file-080.txt")).unwrap();
            let mut guards = stage.guards(Some(&transaction.root)).unwrap();
            assert!(stage_verified_memory(&files, &entries, stage.path(), &mut guards, &ctl, Some(4)).is_err());
        }
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
        assert!(!abandoned.exists(), "all worker handles must close before cleanup");
        assert!(!destination.exists());
    }

    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn memory_directory_transaction_real_extract_matches_existing_target() {
        use std::os::windows::fs::MetadataExt;
        let (fixture, archive, entries) = memory_zip_fixture();
        let parent = fixture.path().join("ordinary");
        let _parent = memory_test_parent(&parent);
        let new = parent.join("new");
        let existing = parent.join("existing");
        fs::create_dir(&existing).unwrap();
        for destination in [&new, &existing] {
            assert_eq!(extract(archive.to_str().unwrap(), destination.to_str().unwrap(),
                &EngineOptions::default()).unwrap(), 160);
        }
        for entry in &entries {
            let path = validate_name(&entry.name).unwrap();
            let a = new.join(&path);
            let b = existing.join(&path);
            let ma = fs::metadata(&a).unwrap();
            let mb = fs::metadata(&b).unwrap();
            assert_eq!(ma.file_attributes(), mb.file_attributes());
            assert_eq!(ma.modified().unwrap(), mb.modified().unwrap());
            if !entry.is_dir { assert_eq!(fs::read(&a).unwrap(), fs::read(&b).unwrap()); }
        }
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
                size_known: true,
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
            size_known: true,
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
    #[cfg(all(windows, target_pointer_width = "64"))]
    #[test]
    fn selected_zip_directory_commit_verifies_unselected_files_and_preserves_zone() {
        let (fixture, archive, entries) = memory_zip_fixture();
        let parent = fixture.path().join("ordinary");
        let _parent = memory_test_parent(&parent);
        let destination = parent.join("out");
        let zone = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        fs::write(format!("{}:Zone.Identifier", archive.display()), zone).unwrap();
        let selected = entries.iter().take(128).map(|entry| entry.name.clone())
            .chain(std::iter::once("empty".into())).collect();
        let opts = EngineOptions { selected, ..Default::default() };
        let unpack = |dest: &Path| extract(archive.to_str().unwrap(), dest.to_str().unwrap(), &opts);
        assert_eq!(unpack(&destination).unwrap(), 128);
        assert!(destination.join("empty").is_dir());
        assert_eq!(fs::read_dir(destination.join("folder")).unwrap().count(), 128);
        for index in 0..128 {
            let path = destination.join(&entries[index].name);
            let bytes = fs::read(&path).unwrap();
            assert_eq!(bytes.len() as u64, entries[index].size);
            assert!(bytes.iter().all(|byte| *byte == index as u8));
            assert_eq!(fs::read(format!("{}:Zone.Identifier", path.display())).unwrap(), zone);
        }
        assert!(unpack(&destination).is_err());
        let cancelled_dest = parent.join("cancelled");
        let ctl = JobControl::new();
        let cancellable = EngineOptions { control: Some(ctl.clone()), ..opts.clone() };
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| extract(archive.to_str().unwrap(), cancelled_dest.to_str().unwrap(), &cancellable));
            while !worker.is_finished() {
                if ctl.snapshot().phase == "extracting" {
                    ctl.cancel();
                    break;
                }
                std::thread::yield_now();
            }
            assert!(worker.join().unwrap().unwrap_err().contains("cancelled"));
        });
        assert!(!cancelled_dest.exists());
        let mut archive_bytes = fs::read(&archive).unwrap();
        let corrupt_at = archive_bytes.windows(32).position(|slice| slice == [159; 32]).unwrap();
        archive_bytes[corrupt_at] ^= 1;
        fs::write(&archive, archive_bytes).unwrap();
        let corrupt_dest = parent.join("corrupt");
        assert!(unpack(&corrupt_dest).is_err());
        assert!(!corrupt_dest.exists());
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
    }
    #[test]
    fn selected_zip_streams_large_unselected_data_without_staging_it() {
        use std::io::Write;
        let temp=tempfile::tempdir().unwrap();
        let archive=temp.path().join("large.zip");
        let mut writer=zip::ZipWriter::new(File::create(&archive).unwrap());
        let options=zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("keep/a.txt",options).unwrap();
        writer.write_all(b"selected content").unwrap();
        writer.start_file("unselected.bin",options).unwrap();
        let bytes=vec![17;1024*1024];
        for _ in 0..72 {writer.write_all(&bytes).unwrap();}
        writer.finish().unwrap();
        let opts=EngineOptions {selected:vec!["keep".into()],..Default::default()};
        let entries=list_raw(archive.to_str().unwrap(),&opts).unwrap();
        let stage=tempfile::tempdir().unwrap();
        let mut guards=CheckedDirectories::owned_root(stage.path());
        assert!(decode_selected_zip(locked_archive_source(archive.to_str().unwrap(),&opts,u32::MAX as u64-1),
            &entries,&opts,stage.path(),&mut guards,&JobControl::new()).unwrap());
        assert_eq!(fs::read(stage.path().join("keep/a.txt")).unwrap(),b"selected content");
        assert!(!stage.path().join("unselected.bin").exists());
        assert_eq!(fs::read_dir(stage.path()).unwrap().count(),1);
        let output=temp.path().join("output");
        extract(archive.to_str().unwrap(),output.to_str().unwrap(),&opts).unwrap();
        assert_eq!(fs::read(output.join("keep/a.txt")).unwrap(),b"selected content");
        assert!(!output.join("unselected.bin").exists());
    }
    #[test]
    fn decoder_thread_limit_is_forwarded_and_invalid_zero_rejected() {
        let opts=EngineOptions{threads:Some(3),..Default::default()};
        for command in ["x","e","t"] {
            assert!(base(command,&opts).unwrap().contains(&"-mmt=3".to_owned()));
        }
        assert!(!base("x",&EngineOptions::default()).unwrap().iter().any(|a|a.starts_with("-mmt")));
        assert!(base("x",&EngineOptions{threads:Some(0),..Default::default()}).is_err());
    }
    #[test]
    fn selected_zip_stream_preserves_unselected_crc_failure_and_no_publication() {
        let (temp,path,entries)=memory_zip_fixture();
        let position={let mut z=zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
            let offset=z.by_index(159).unwrap().data_start().unwrap();offset};
        let mut data=fs::read(&path).unwrap();data[position as usize]^=1;fs::write(&path,data).unwrap();
        let opts=EngineOptions{selected:vec![entries[1].name.clone()],..Default::default()};
        let stage=tempfile::tempdir().unwrap();
        let result=decode_selected_zip(locked_archive_source(path.to_str().unwrap(),&opts,u32::MAX as u64-1),
            &entries,&opts,stage.path(),&mut CheckedDirectories::owned_root(stage.path()),&JobControl::new());
        assert!(result.unwrap_err().contains("ZIP"));
        let output=temp.path().join("output");
        assert!(extract(path.to_str().unwrap(),output.to_str().unwrap(),&opts).is_err());
        assert!(!output.exists());
    }
    #[test]
    fn selected_zip_stream_preserves_empty_directories_and_cancel() {
        let (_temp,path,entries)=memory_zip_fixture();
        let opts=EngineOptions{selected:vec!["empty/".into()],..Default::default()};
        let stage=tempfile::tempdir().unwrap();
        assert!(decode_selected_zip(locked_archive_source(path.to_str().unwrap(),&opts,u32::MAX as u64-1),
            &entries,&opts,stage.path(),&mut CheckedDirectories::owned_root(stage.path()),&JobControl::new()).unwrap());
        assert!(stage.path().join("empty").is_dir());
        assert_eq!(fs::read_dir(stage.path()).unwrap().count(),1);
        let ctl=JobControl::new();ctl.cancelled.store(true,Ordering::Release);
        assert!(decode_selected_zip(locked_archive_source(path.to_str().unwrap(),&opts,u32::MAX as u64-1),
            &entries,&opts,stage.path(),&mut CheckedDirectories::owned_root(stage.path()),&ctl).is_err());
    }
    #[test]
    fn selected_zip_unknown_layout_falls_back_without_writing() {
        let (_temp,path,entries)=memory_zip_fixture();
        let mut data=fs::read(&path).unwrap();
        // Unsupported descriptor flag must never be guessed by this path.
        let offset=data.windows(4).position(|b|b==b"PK\x01\x02").unwrap();
        data[offset+8]|=8;
        fs::write(&path,data).unwrap();
        let opts=EngineOptions{selected:vec![entries[1].name.clone()],..Default::default()};
        let stage=tempfile::tempdir().unwrap();
        assert!(!decode_selected_zip(locked_archive_source(path.to_str().unwrap(),&opts,u32::MAX as u64-1),
            &entries,&opts,stage.path(),&mut CheckedDirectories::owned_root(stage.path()),&JobControl::new()).unwrap());
        assert_eq!(fs::read_dir(stage.path()).unwrap().count(),0);
    }
    #[test]
    fn bounded_zip_memory_verifies_all_bytes_and_preserves_empty_entries() {
        let (_temp, path, entries) = memory_zip_fixture();
        let contents = decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()),
            &entries,
            &EngineOptions::default(),
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
                &mut guard,
                None
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
    fn selected_memory_zip_retains_only_published_payload() {
        let (_temp, path, entries) = memory_zip_fixture();
        let opts = EngineOptions { selected: vec!["folder/file-031.txt".into()], ..Default::default() };
        let contents = decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &opts),
            &entries, &opts, &JobControl::new(),
        ).unwrap().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents["folder/file-031.txt"].as_slice(), vec![31; 3073]);
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
            &options,
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
            &EngineOptions::default(),
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
            &EngineOptions::default(),
            &JobControl::new()
        )
        .unwrap()
        .is_none());
        let ctl = JobControl::new();
        ctl.cancel();
        assert!(decode_memory_zip(
            memory_archive_source(path.to_str().unwrap(), &EngineOptions::default()),
            &entries,
            &EngineOptions::default(),
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
        let mut guards = CheckedDirectories::default();
        guards.check(temp.path()).unwrap();
        guards.allow_publication(temp.path()).unwrap();
        let oldpath = temp.path().join("old.txt");
        let old = tempfile::NamedTempFile::new_in(temp.path())
            .unwrap()
            .persist(&oldpath)
            .unwrap();
        for name in ["short.txt", "中文😀文件.txt", "odd-name-length.bin"] {
            let path = temp.path().join(name);
            let mut output = output_temp(temp.path(), &guards).unwrap();
            let scratch = output.path().to_path_buf();
            output.write_all(b"verified").unwrap();
            let file = publish_output(&mut output, &path, false).unwrap();
            assert!(!scratch.exists());
            assert_eq!(fs::read(&path).unwrap(), b"verified");
            assert_eq!(
                file.metadata().unwrap().file_attributes(),
                old.metadata().unwrap().file_attributes()
            );
            assert_eq!(dacl(&path), dacl(&oldpath));
            let mut collision = output_temp(temp.path(), &guards).unwrap();
            let scratch = collision.path().to_path_buf();
            collision.write_all(b"must-not-replace").unwrap();
            assert!(publish_output(&mut collision, &path, false).is_err());
            collision.abort().unwrap();
            drop(collision);
            assert!(!scratch.exists());
            assert_eq!(fs::read(&path).unwrap(), b"verified");
            drop(file);
        }
        let target = temp.path().join("readonly.txt");
        fs::write(&target, b"preserve").unwrap();
        let mut perm = fs::metadata(&target).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&target, perm).unwrap();
        assert!(publish_output(&mut output_temp(temp.path(), &guards).unwrap(), &target, true).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"preserve");
        let mut perm = fs::metadata(&target).unwrap().permissions();
        perm.set_readonly(false);
        fs::set_permissions(&target, perm).unwrap();
        let newpath = temp.path().join("metadata.txt");
        let published = publish_output(&mut output_temp(temp.path(), &guards).unwrap(), &newpath, false).unwrap();
        let entry = Entry {
            size_known: true,
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
        // Even an explicitly authorized private root loses relative-read
        // eligibility when its direct directory pin permits writes.
        guard.private_stages.insert(dir.clone());
        assert!(guard.private_metadata_parent(&dir).is_none());
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
        assert!(checked_staged_metadata_with_relative(&dir.join("missing"), &mut guard, true).is_err());
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
