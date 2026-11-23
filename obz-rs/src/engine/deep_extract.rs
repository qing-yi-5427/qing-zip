//! Deep extraction owns a private recovery tree and never removes original inputs.
//! Known-size archive limits are conservative preflight and verified-output checks;
//! raw streams additionally have an enforced stdout write bound.
use super::*;
use crate::deep_extract_plan::{Candidate, Limits, Plan, State};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub depth: usize,
    pub archives: usize,
    pub files: u64,
    pub bytes: u64,
    pub reserve_bytes: u64,
    pub recovery_directory: Option<String>,
    pub dedicated_output: bool,
}
impl Default for Settings {
    fn default() -> Self {
        let v = Limits::default();
        Self {
            depth: v.depth,
            archives: v.archives,
            files: v.files,
            bytes: v.bytes,
            reserve_bytes: v.reserve_bytes,
            recovery_directory: None,
            dedicated_output: false,
        }
    }
}
impl From<Settings> for Limits {
    fn from(s: Settings) -> Self {
        Self {
            depth: s.depth,
            archives: s.archives,
            files: s.files,
            bytes: s.bytes,
            reserve_bytes: s.reserve_bytes,
        }
    }
}
#[derive(Clone, Debug)]
enum Kind {
    Archive,
    Stream(&'static str),
    Invalid(String),
    MissingFirst(String),
}
#[derive(Clone)]
struct Source {
    path: PathBuf,
    parts: Vec<PathBuf>,
    kind: Kind,
}
struct VerifiedVolume {
    path: PathBuf,
    identity: (u32, u64),
    length: u64,
    modified: std::time::SystemTime,
}
impl VerifiedVolume {
    fn read(path: PathBuf, file: &File) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        Some(Self {
            path,
            identity: directory_identity(file).ok()?,
            length: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
    fn unchanged(&self) -> bool {
        crate::engine_stream::open_source_leaf(&self.path)
            .ok()
            .and_then(|file| Self::read(self.path.clone(), &file))
            .is_some_and(|current| {
                (current.identity, current.length, current.modified)
                    == (self.identity, self.length, self.modified)
            })
    }
}
#[derive(Serialize)]
pub struct Pending {
    pub node: usize,
    pub name: String,
    pub source: String,
    pub ancestry: Vec<String>,
    pub reason: String,
}
#[derive(Serialize)]
pub struct Report {
    pub destination: String,
    pub outputs: Vec<String>,
    pub files: usize,
    pub partial: bool,
    pub pending: Vec<Pending>,
    pub recovery_path: Option<String>,
    pub decoded_bytes: u64,
    pub decoded_files: u64,
    pub archives: usize,
    pub max_depth: usize,
    pub summary: String,
    pub retained_archives: Vec<String>,
    pub undiscovered_sources: u64,
    pub publish_error: Option<String>,
    pub finished: bool,
    pub accounted_bytes: u64,
    pub verified_output_bytes: u64,
    pub accounting_estimated: bool,
    pub protected_archives: Vec<String>,
    pub retained_matching_files: Vec<String>,
}
pub struct Session {
    stage: Option<ShellDragDirectory>,
    plan: Plan,
    sources: HashMap<String, Source>,
    options: EngineOptions,
    dest: Option<PathBuf>,
    smart: bool,
    dedicated_output: bool,
    assigned_targets: HashMap<usize, PathBuf>,
    outputs: Vec<String>,
    published: BTreeSet<usize>,
    published_files: BTreeSet<usize>,
    supplied_inputs: Vec<PathBuf>,
    root_inputs: HashSet<PathBuf>,
    attached: BTreeSet<usize>,
    finished: bool,
    publish_error: Option<String>,
    serial: usize,
    passwords: Vec<String>,
    used_passwords: Vec<(String, String)>,
    probes: HashMap<PathBuf, Option<Kind>>,
    verified_bytes: u64,
    estimated: bool,
    publication: file_workspace::DeepPublicationLedger,
    retained_matching_files: BTreeSet<String>,
    verified_split_parts: HashMap<usize, Vec<VerifiedVolume>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        // A partially decoded/published session is deliberately recoverable after app exit.
        if !self.finished {
            if let Some(stage) = self.stage.as_ref() {
                stage.retain_for_shell();
            }
        }
        self.options.password = None;
        self.passwords.clear();
    }
}
fn string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "路径不是有效 Unicode".into())
}
fn source_key(path: &Path) -> String {
    File::open(path)
        .ok()
        .and_then(|f| directory_identity(&f).ok())
        .map(|(volume, id)| format!("{volume}:{id}"))
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
fn ordinary_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn password_error(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    [
        "wrong password",
        "password required",
        "password is incorrect",
        "enter password",
        "需要密码",
        "密码错误",
    ]
    .iter()
    .any(|p| e.contains(p))
}
fn application_extension(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        [
            "docx",
            "docm",
            "dotx",
            "dotm",
            "xlsx",
            "xlsm",
            "xltx",
            "xltm",
            "pptx",
            "pptm",
            "potx",
            "potm",
            "ppsx",
            "ppsm",
            "odt",
            "ods",
            "odp",
            "epub",
            "apk",
            "aab",
            "apks",
            "xapk",
            "jar",
            "war",
            "ear",
            "nupkg",
            "vsix",
            "crx",
            "xpi",
            "ipa",
            "appx",
            "msix",
            "appxbundle",
            "msixbundle",
        ]
        .iter()
        .any(|v| e.eq_ignore_ascii_case(v))
    })
}
fn stream_name(path: &Path) -> Result<String, String> {
    let name = ordinary_name(path);
    let lower = name.to_ascii_lowercase();
    let mapped = if lower.ends_with(".tgz") {
        Some((4, ".tar"))
    } else if lower.ends_with(".tbz2") {
        Some((5, ".tar"))
    } else if lower.ends_with(".txz") {
        Some((4, ".tar"))
    } else {
        None
    };
    let name = if let Some((n, suffix)) = mapped {
        format!("{}{suffix}", &name[..name.len() - n])
    } else {
        path.file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("decoded")
            .to_owned()
    };
    validate_name(&name)?;
    Ok(name)
}
fn extension_hint(path: &Path) -> bool {
    let lower = ordinary_name(path).to_ascii_lowercase();
    [
        ".zip", ".7z", ".rar", ".tar", ".gz", ".gzip", ".bz2", ".xz", ".tgz", ".tbz2", ".txz",
    ]
    .iter()
    .any(|s| lower.ends_with(s))
        || matches!(
            crate::deep_volume_plan::classify(&ordinary_name(path), &[]),
            crate::deep_volume_plan::Member::First { .. }
        )
}
fn signature(header: &[u8]) -> Option<Kind> {
    if header.starts_with(&[0x1f, 0x8b]) {
        return Some(Kind::Stream("gzip"));
    }
    if header.starts_with(b"BZh") {
        return Some(Kind::Stream("bzip2"));
    }
    if header.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        return Some(Kind::Stream("xz"));
    }
    if header.starts_with(b"PK\x03\x04")
        || header.starts_with(b"PK\x05\x06")
        || header.starts_with(b"PK\x07\x08")
        || header.starts_with(b"7z\xbc\xaf\x27\x1c")
        || header.starts_with(b"Rar!\x1a\x07")
        || header.get(257..262) == Some(b"ustar")
    {
        return Some(Kind::Archive);
    }
    None
}
fn sniff(path: &Path) -> Result<Option<Kind>, String> {
    let mut file = crate::engine_stream::open_source_leaf(path).map_err(|e| e.to_string())?;
    let mut head = [0u8; 512];
    let n = file.read(&mut head).map_err(|e| e.to_string())?;
    let h = &head[..n];
    if let Some(kind) = signature(h) {
        return Ok(Some(kind));
    }
    // Candidate only: the existing listing/decoder remains the format proof.
    // A bounded tail probe catches prefixed ZIPs; SFX receives a bounded header scan.
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let tail_len = len.min(65557);
    file.seek(SeekFrom::End(-(tail_len as i64)))
        .map_err(|e| e.to_string())?;
    let mut tail = vec![0; tail_len as usize];
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    if tail.windows(4).any(|w| w == b"PK\x05\x06") {
        return Ok(Some(Kind::Archive));
    }
    if h.starts_with(b"MZ") {
        file.rewind().map_err(|e| e.to_string())?;
        let mut prefix = Vec::new();
        file.take(1024 * 1024)
            .read_to_end(&mut prefix)
            .map_err(|e| e.to_string())?;
        if prefix.windows(6).any(|w| w == b"7z\xbc\xaf\x27\x1c")
            || prefix.windows(7).any(|w| w == b"Rar!\x1a\x07")
        {
            return Ok(Some(Kind::Archive));
        }
    }
    Ok(extension_hint(path).then_some(Kind::Archive))
}
pub fn application_container(entries: &[Entry]) -> bool {
    let mut office_marker = false;
    let mut office_payload = false;
    let mut android_manifest = false;
    let mut android_payload = false;
    let mut mimetype = false;
    let mut publication_payload = false;
    for entry in entries {
        let mut components = entry.name.split(['/', '\\']);
        let first = components.next().unwrap_or_default();
        match components.next() {
            None => {
                office_marker |= first.eq_ignore_ascii_case("[content_types].xml");
                android_manifest |= first.eq_ignore_ascii_case("androidmanifest.xml");
                android_payload |= first.eq_ignore_ascii_case("classes.dex");
                mimetype |= first.eq_ignore_ascii_case("mimetype");
                publication_payload |= first.eq_ignore_ascii_case("content.xml");
            }
            Some(second) => {
                office_payload |= ["word", "xl", "ppt"].iter().any(|dir| first.eq_ignore_ascii_case(dir));
                android_payload |= first.eq_ignore_ascii_case("res");
                if first.eq_ignore_ascii_case("meta-inf") && components.next().is_none() {
                    if second.eq_ignore_ascii_case("manifest.mf") {
                        return true;
                    }
                    publication_payload |= second.eq_ignore_ascii_case("container.xml");
                }
            }
        }
    }
    (office_marker && office_payload)
        || (android_manifest && android_payload)
        || (mimetype && publication_payload)
}

/// Read only a bounded signature for formats supported by deep extraction.
pub fn home_deep_candidate(path: &Path) -> Result<bool, String> {
    Ok(!application_extension(path) && sniff(path)?.is_some())
}

pub fn home_application_extension(path: &Path) -> bool {
    application_extension(path)
}

/// Keep multi-volume inputs together in the existing deep workflow.
pub fn home_volume_member(path: &Path) -> bool {
    use crate::deep_volume_plan::{classify, Member};
    let name = ordinary_name(path);
    match classify(&name, &[]) {
        Member::First { .. } => true,
        Member::Later { .. } => first_volume(path).is_ok_and(|first| {
            sniff(&first).is_ok_and(|kind| kind.is_some())
        }),
        Member::Ordinary => match path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("zip") => path.with_extension("z01").is_file() || path.with_extension("z001").is_file(),
            Some("rar") => path.with_extension("r00").is_file(),
            _ => false,
        },
    }
}

/// Use the browsing listing first. For a small, ordinary ZIP wrapper, look at
/// at most four unencrypted payload prefixes without creating any output.
pub fn home_nested_candidate(path: &Path, entries: &[Entry]) -> bool {
    let mut files = entries.iter().filter(|e| !e.is_dir);
    if files.clone().any(|e| {
        let name = Path::new(&e.name);
        !application_extension(name) && extension_hint(name)
    }) {
        return true;
    }
    if entries.len() > 128
        || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        || files.clone().take(5).count() > 4
        || crate::machine_policy::scope().restricted()
    {
        return false;
    }
    let Ok(mut source) = crate::engine_stream::pin(path) else {
        return false;
    };
    let Ok(Some(index)) = zip_volume_view::single_disk_record(&mut source) else {
        return false;
    };
    if index.zip64 || usize::from(index.entries) != entries.len()
        || index.directory_size > 256 * 1024
        || u64::from(index.directory) + u64::from(index.directory_size) != index.end_record
    {
        return false;
    }
    let Ok(mut zip) = zip::ZipArchive::new(&mut *source) else {
        return false;
    };
    if zip.len() != entries.len() {
        return false;
    }
    files.any(|entry| {
        if entry.encrypted || entry.size < 6
            || !matches!(entry.method.as_str(), "Store" | "Deflate")
            || application_extension(Path::new(&entry.name))
        {
            return false;
        }
        let Some(index) = zip.index_for_name(&entry.name.replace('\\', "/")) else {
            return false;
        };
        let Ok(member) = zip.by_index_raw(index) else {
            return false;
        };
        zip_prefix_has_archive_signature(member)
    })
}

fn zip_prefix_has_archive_signature<R: Read + Seek>(mut member: zip::read::ZipFile<'_, R>) -> bool {
    use miniz_oxide::inflate::{
        core::{decompress, inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF, DecompressorOxide},
        TINFLStatus,
    };

    if member.encrypted() || member.size() < 6 {
        return false;
    }
    let mut prefix = [0u8; 512];
    let length = member.size().min(prefix.len() as u64) as usize;
    let prefix = &mut prefix[..length];
    match member.compression() {
        zip::CompressionMethod::Stored => {
            if member.read_exact(prefix).is_err() {
                return false;
            }
        }
        zip::CompressionMethod::Deflated => {
            // Read at most 64 KiB of Deflate input even if it emits no prefix.
            // The output buffer stops decompression at the desired prefix.
            let mut packed = vec![0; member.compressed_size().min(64 * 1024) as usize];
            if member.read_exact(&mut packed).is_err() {
                return false;
            }
            let (status, _, written) = decompress(
                &mut DecompressorOxide::new(),
                &packed,
                prefix,
                0,
                TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
            );
            if written != length || !matches!(status, TINFLStatus::Done | TINFLStatus::HasMoreOutput) {
                return false;
            }
        }
        _ => return false,
    }
    signature(prefix).is_some()
}

fn family(path: &Path, peers: &[PathBuf]) -> Vec<PathBuf> {
    let siblings = peers
        .iter()
        .filter(|p| p.parent() == path.parent())
        .map(|p| ordinary_name(p))
        .collect::<Vec<_>>();
    let names = crate::deep_volume_plan::content_family(&ordinary_name(path), &siblings);
    peers
        .iter()
        .filter(|p| p.parent() == path.parent() && names.contains(&ordinary_name(p)))
        .cloned()
        .collect()
}

// Recognize a raw 7z byte stream cut into consecutively numbered volumes.
// Never claim an extra file just because it shares a name with the first part.
fn open_exact_numbered_7z(first: &Path, ctl: &JobControl) -> Option<Vec<(PathBuf, File)>> {
    let prefix = first.file_name()?.to_str()?.strip_suffix(".001")?;
    if !Path::new(prefix).extension()?.eq_ignore_ascii_case("7z") {
        return None;
    }
    let mut first_file = crate::engine_stream::open_source_leaf(first).ok()?;
    let mut start = [0u8; 32];
    first_file.read_exact(&mut start).ok()?;
    if !start.starts_with(b"7z\xbc\xaf\x27\x1c")
        || start[6] != 0
        || crc32fast::hash(&start[12..]) != u32::from_le_bytes(start[8..12].try_into().ok()?)
    {
        return None;
    }
    let next_offset = u64::from_le_bytes(start[12..20].try_into().ok()?);
    let next_size = u64::from_le_bytes(start[20..28].try_into().ok()?);
    let end = 32u64.checked_add(next_offset)?.checked_add(next_size)?;
    let mut total = first_file.metadata().ok()?.len();
    if next_size == 0 || total >= end {
        return None;
    }
    let mut parts = vec![(first.to_owned(), first_file)];
    for number in 2..=1024 {
        ctl.check().ok()?;
        let path = first.with_file_name(format!("{prefix}.{number:03}"));
        let file = crate::engine_stream::open_source_leaf(&path).ok()?;
        let length = file.metadata().ok()?.len();
        if length == 0 {
            return None;
        }
        total = total.checked_add(length)?;
        if total > end {
            return None;
        }
        parts.push((path, file));
        if total == end {
            let next = first.with_file_name(format!("{prefix}.{:03}", number + 1));
            return match fs::symlink_metadata(next) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(parts),
                _ => None,
            };
        }
    }
    None
}

fn confirmed_numbered_7z(
    parts: &[(PathBuf, File)],
    opts: &EngineOptions,
) -> Option<Vec<VerifiedVolume>> {
    let proof = parts
        .iter()
        .map(|(path, file)| VerifiedVolume::read(path.clone(), file))
        .collect::<Option<Vec<_>>>()?;
    let properties = archive_properties::read(proof.first()?.path.to_str()?, opts).ok()?;
    let [outer, inner] = properties.layers.as_slice() else {
        return None;
    };
    let field = |layer: &archive_properties::Layer, key: &str| layer.fields.get(key).cloned();
    let total = proof
        .iter()
        .try_fold(0u64, |sum, part| sum.checked_add(part.length))?;
    (!properties.partial
        && field(outer, "Type").as_deref() == Some("Split")
        && field(inner, "Type").as_deref() == Some("7z")
        && field(outer, "Volumes").and_then(|n| n.parse::<usize>().ok()) == Some(proof.len())
        && field(outer, "Total Physical Size").and_then(|n| n.parse::<u64>().ok()) == Some(total)
        && field(inner, "Physical Size").and_then(|n| n.parse::<u64>().ok()) == Some(total))
    .then_some(proof)
}
// Existing bounded engine metadata can prove an independently readable file.
// Unknown metadata never authorizes consuming or removing a guessed companion.
fn standalone_archive(path: &Path, opts: &EngineOptions) -> bool {
    let Ok(properties) = archive_properties::read(&path.to_string_lossy(), opts) else {
        return false;
    };
    if properties.partial || properties.layers.len() != 1 {
        return false;
    }
    let fields = &properties.layers[0].fields;
    let kind = fields
        .get("Type")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if !["zip", "7z", "tar", "gzip", "bzip2", "xz"].contains(&kind.as_str())
        || fields.get("Multivolume").is_some_and(|s| s == "+")
        || fields
            .get("Volumes")
            .is_some_and(|s| s.parse::<u64>().ok() != Some(1))
    {
        return false;
    }
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    fields
        .get("Physical Size")
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|n| n > 0 && n <= size)
}
fn add_root_family(
    path: &Path,
    inputs: &mut HashSet<PathBuf>,
    directories: &mut HashMap<PathBuf, Option<Vec<String>>>,
) -> bool {
    use crate::deep_volume_plan::{classify, root_family, Member};
    inputs.insert(path.to_owned());
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    inputs.insert(canonical.clone());
    let Some(parent) = canonical.parent() else {
        return false;
    };
    let name = ordinary_name(&canonical);
    if !matches!(
        canonical
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("zip" | "rar" | "exe")
    ) && !crate::deep_volume_plan::first_probe(&name)
        && matches!(classify(&name, &[]), Member::Ordinary)
    {
        return true;
    }
    let siblings = directories.entry(parent.to_owned()).or_insert_with(|| {
        let entries = match fs::read_dir(parent) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(Vec::new()),
            Err(_) => return None,
        };
        entries
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<Vec<_>>>()
            .ok()
    });
    let Some(siblings) = siblings else {
        return false;
    };
    let first = match classify(&name, siblings) {
        Member::Later {
            first: Some(first), ..
        } => first,
        Member::Later { expected, .. } => expected,
        _ => name,
    };
    inputs.insert(parent.join(&first));
    inputs.extend(
        root_family(&first, siblings)
            .into_iter()
            .map(|name| parent.join(name)),
    );
    true
}
fn first_volume(path: &Path) -> Result<PathBuf, String> {
    use crate::deep_volume_plan::{classify, Member};
    let name = ordinary_name(path);
    if !matches!(classify(&name, &[]), Member::Later { .. }) {
        return Ok(path.to_owned());
    }
    let parent = path.parent().ok_or("分卷缺少父目录")?;
    let siblings = fs::read_dir(parent)
        .map_err(|e| e.to_string())?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    match classify(&name, &siblings) {
        Member::Later {
            first: Some(first), ..
        } => Ok(parent.join(first)),
        Member::Later { expected, .. } => Err(format!(
            "缺少第一卷 {expected}；现有分卷已保留，请定位第一卷后重试或保留此包继续"
        )),
        _ => Ok(path.to_owned()),
    }
}
fn volume_member(path: &Path, peers: &[PathBuf]) -> crate::deep_volume_plan::Member {
    let initial = crate::deep_volume_plan::classify(&ordinary_name(path), &[]);
    if !matches!(initial, crate::deep_volume_plan::Member::Later { .. }) {
        return initial;
    }
    let siblings = peers
        .iter()
        .filter(|p| p.parent() == path.parent())
        .map(|p| ordinary_name(p))
        .collect::<Vec<_>>();
    crate::deep_volume_plan::classify(&ordinary_name(path), &siblings)
}
fn missing_volume_key(path: &Path, family: &str) -> String {
    format!(
        "missing-first:{}:{family}",
        path.parent().unwrap_or(Path::new("")).display()
    )
}

fn measure(path: &Path, ctl: &JobControl) -> Result<(u64, u64), String> {
    ctl.check()?;
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("解码结果包含链接".into());
    }
    if meta.is_file() {
        return Ok((1, meta.len()));
    }
    let mut result = (0u64, 0u64);
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let (files, bytes) = measure(&entry.map_err(|e| e.to_string())?.path(), ctl)?;
        result.0 = result.0.checked_add(files).ok_or("文件数量溢出")?;
        result.1 = result.1.checked_add(bytes).ok_or("解码量溢出")?;
    }
    Ok(result)
}
impl Session {
    pub fn start(
        inputs: Vec<String>,
        destination: &str,
        smart: bool,
        mut options: EngineOptions,
        settings: Settings,
    ) -> Result<Self, String> {
        let dest = (!destination.trim().is_empty())
            .then(|| absolute(destination))
            .transpose()?;
        let dedicated_output = settings.dedicated_output || dest.is_none();
        let ctl = control(&options);
        ctl.check()?;
        let base = if let Some(path) = settings
            .recovery_directory
            .as_deref()
            .filter(|p| !p.trim().is_empty())
        {
            absolute(path)?
        } else {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .ok_or("无法定位恢复文件夹")?
                .join("qing-zip")
                .join("DeepRecovery")
        };
        no_links(&base)?;
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let stage = ShellDragDirectory::create_named_in(&base, "deep-")?;
        let peers = inputs
            .iter()
            .map(|p| absolute(p))
            .collect::<Result<Vec<_>, _>>()?;
        let mut root_inputs: HashSet<_> = peers.iter().cloned().collect();
        let mut directories = HashMap::new();
        let mut roots = vec![];
        let mut sources = HashMap::new();
        for input in &peers {
            ctl.check()?;
            let (path, kind) = match first_volume(input) {
                Ok(path) => {
                    let kind = match sniff(&path) {
                        Ok(Some(kind)) => kind,
                        Ok(None) => {
                            Kind::Invalid(format!("未识别为支持的压缩包：{}", path.display()))
                        }
                        Err(error) => Kind::Invalid(error),
                    };
                    (path, kind)
                }
                Err(error) => (
                    input.clone(),
                    match volume_member(input, &peers) {
                        crate::deep_volume_plan::Member::Later { expected, .. } => {
                            Kind::MissingFirst(expected)
                        }
                        _ => Kind::Invalid(error),
                    },
                ),
            };
            add_root_family(&path, &mut root_inputs, &mut directories);
            let key = if matches!(kind, Kind::MissingFirst(_)) {
                match volume_member(&path, &peers) {
                    crate::deep_volume_plan::Member::Later { family, .. } => {
                        missing_volume_key(&path, &family)
                    }
                    _ => source_key(&path),
                }
            } else {
                source_key(&path)
            };
            roots.push(Candidate {
                source: key.clone(),
                name: ordinary_name(&path),
            });
            sources.insert(
                key,
                Source {
                    parts: vec![path.clone()],
                    path,
                    kind,
                },
            );
        }
        let plan = Plan::new(roots, settings.into())?;
        // Selection applies exclusively to the selected root; child archives are complete.
        if plan.roots.len() > 1 && !options.selected.is_empty() {
            return Err("批量深度解压不支持跨压缩包选定范围".into());
        }
        options.verification_capture = None;
        options.verification_reuse = None;
        let note="这是深度解压的恢复目录。node-* 包含已校验的中间结果；发生中断时可从这里恢复文件。此目录不保存密码。\r\n";
        fs::write(stage.path().join("恢复说明.txt"), note).map_err(|e| e.to_string())?;
        if let Some(password) = options.password.as_deref() {
            crate::engine_password::validate(password)?;
        }
        let passwords = options.password.take().into_iter().collect();
        Ok(Self {
            stage: Some(stage),
            plan,
            sources,
            options,
            dest,
            smart,
            dedicated_output,
            assigned_targets: HashMap::new(),
            outputs: vec![],
            published: BTreeSet::new(),
            published_files: BTreeSet::new(),
            supplied_inputs: peers,
            root_inputs,
            attached: BTreeSet::new(),
            finished: false,
            publish_error: None,
            serial: 0,
            passwords,
            used_passwords: Vec::new(),
            probes: HashMap::new(),
            verified_bytes: 0,
            estimated: false,
            publication: Default::default(),
            retained_matching_files: BTreeSet::new(),
            verified_split_parts: HashMap::new(),
        })
    }
    pub fn resume(
        &mut self,
        node: usize,
        skip: bool,
        password: Option<String>,
        settings: Option<Settings>,
    ) -> Result<(), String> {
        if let Some(password) = password.as_ref() {
            crate::engine_password::validate(password)?;
        }
        let limits = settings
            .map(Limits::from)
            .unwrap_or_else(|| self.plan.limits.clone());
        limits.validate()?;
        let item = self.plan.nodes.get(node).ok_or("深度解压项目不存在")?;
        if !matches!(item.state, State::Pending(_)) {
            return Err("该项目无需重试或保留".into());
        }
        if !skip && (item.depth > limits.depth || node >= limits.archives) {
            return Err("请先提高限制，或保留此压缩包继续".into());
        }
        self.plan.limits = limits;
        if skip {
            self.plan.skip(node)?;
        } else {
            self.plan.retry(node)?;
        }
        // Only retain the supplied password during this drive, and clear it afterwards.
        if let Some(password) = password {
            self.passwords.retain(|p| p != &password);
            self.passwords.insert(0, password);
            self.passwords.truncate(16);
            // A newly supplied credential also retries other password-paused branches once.
            for (id, item) in self.plan.nodes.iter_mut().enumerate() {
                if id != node
                    && item.tree.is_none()
                    && item.depth <= self.plan.limits.depth
                    && id < self.plan.limits.archives
                    && matches!(&item.state,State::Pending(error) if password_error(error))
                {
                    item.state = State::Ready;
                }
            }
        }
        Ok(())
    }
    fn decode(&mut self, id: usize, ctl: &JobControl) -> Result<(), String> {
        let location = self.locator(id);
        let depth = self.plan.nodes[id].depth;
        let context = format!(
            "第 {} 层（上限 {} 层） · {}\n已处理 {} 个文件 · 累计解码 {:.2} GiB（上限 {:.2} GiB） · 已识别 {} 个压缩包",
            depth,
            self.plan.limits.depth,
            location.join(" → "),
            self.plan.decoded_files,
            self.plan.decoded_bytes as f64 / 1073741824.0,
            self.plan.limits.bytes as f64 / 1073741824.0,
            self.plan.nodes.len()
        );
        let archive = location.last().cloned().unwrap_or_default();
        *ctl.context.lock().unwrap_or_else(|e| e.into_inner()) = Some((context, DeepProgress { archive, depth }));
        let mut source = self
            .sources
            .get(&self.plan.nodes[id].source)
            .ok_or("压缩包来源已失效")?
            .clone();
        if matches!(source.kind, Kind::MissingFirst(_)) {
            let first = first_volume(&source.path)?;
            let kind = sniff(&first)?.ok_or("第一卷尚未识别为支持的压缩包；现有分卷已保留")?;
            source = Source {
                parts: vec![first.clone()],
                path: first,
                kind,
            };
            self.sources
                .insert(self.plan.nodes[id].source.clone(), source.clone());
        }
        if application_extension(&source.path) {
            self.plan.nodes[id].state = State::Protected;
            return Ok(());
        }
        if let Kind::Invalid(error) = &source.kind {
            return Err(error.clone());
        }
        let mut opts = self.options.clone();
        opts.extraction_password_use = (opts.extraction_password_use.is_some() && opts.password.is_some())
            .then(|| Arc::new(ExtractionPasswordUse::default()));
        opts.control = Some(ctl.clone());
        opts.overwrite = "error".into();
        opts.zone_identifier = Some(crate::motw::archive_context(&source.path)?);
        if self.plan.nodes[id].parent.is_some() {
            opts.selected.clear();
        }
        let split_pins = self.plan.nodes[id]
            .parent
            .and_then(|_| open_exact_numbered_7z(&source.path, ctl));
        ctl.set(
            "deep-inspect",
            None,
            &format!(
                "第 {} 层 · {}",
                self.plan.nodes[id].depth,
                self.locator(id).join(" → ")
            ),
        );
        let selected = Selection::new(&opts.selected);
        let reserve = self.plan.limits.reserve_bytes;
        let stage = self.stage.as_ref().ok_or("恢复目录已关闭")?;
        self.serial += 1;
        let tree = stage.path().join(format!("node-{id}-{}", self.serial));
        let mut entries = Vec::new();
        let mut declared = None;
        let mut files = 0;
        let mut prepared = false;
        let mut protected = false;
        let archive = matches!(source.kind, Kind::Archive);
        let mut prepare = |inventory: &[Entry]| -> Result<PathBuf, String> {
            if application_container(inventory) {
                protected = true;
                return Err("检测到应用程序容器".into());
            }
            if opts.password.is_none()
                && inventory.iter().any(|e| e.encrypted && selected.contains(&e.name))
            {
                return Err("此压缩包需要密码".into());
            }
            let members = || inventory.iter().filter(|e| !e.is_dir && selected.contains(&e.name));
            files = members().count() as u64;
            declared = members().try_fold(0u64, |sum, e| {
                e.size_known.then(|| sum.checked_add(e.size)).flatten()
            });
            self.plan.preflight(files, declared)?;
            // Archive routes perform this check once the engine chooses
            // a direct writer or a particular private staging volume.
            if !archive {
                let required = declared.unwrap_or(65536).checked_add(reserve).ok_or("空间需求溢出")?;
                if !has_stage_space(stage.path(), required) {
                    return Err("恢复目录所在磁盘空间不足；请更改位置或释放空间后重试".into());
                }
            }
            fs::create_dir(&tree).map_err(|e| e.to_string())?;
            prepared = true;
            entries.extend_from_slice(inventory);
            Ok(tree.clone())
        };
        let result = match source.kind {
            Kind::Archive => with_zip_source(&string(&source.path)?, &opts, |p| {
                extract_raw_to(p, &opts, true, Some(reserve), prepare)
            })
            .map(|_| ()),
            Kind::Stream(format) => {
                let name = stream_name(&source.path)?;
                let inventory = [Entry {
                    name: name.clone(),
                    size: 0,
                    size_known: false,
                    is_dir: false,
                    packed_size: 0,
                    encrypted: false,
                    created: String::new(),
                    accessed: String::new(),
                    modified: String::new(),
                    crc: String::new(),
                    method: format.into(),
                    comment: String::new(),
                    attributes: String::new(),
                }];
                prepare(&inventory)?;
                decode_stream(
                    &source.path,
                    &tree.join(name),
                    format,
                    &opts,
                    self.plan.remaining_bytes(),
                    reserve,
                ).map(|_| ())
            }
            Kind::Invalid(ref error) => Err(error.clone()),
            Kind::MissingFirst(ref expected) => {
                Err(format!("缺少第一卷 {expected}；现有分卷已保留"))
            }
        };
        if protected {
            self.plan.nodes[id].state = State::Protected;
            return Ok(());
        }
        if !prepared {
            return result;
        }
        // Accounting survives decoder/checksum/cancellation failure and subsequent retries.
        let observed = measure(&tree, &JobControl::new())?;
        // Known-size failed native stages can be cleaned internally. Charge their full
        // selected declared work conservatively; this is not a hard decoder disk-write cap.
        let charged = declared.map_or(observed.1, |n| n.max(observed.1));
        self.estimated |= charged != observed.1;
        if result.is_ok() {
            self.verified_bytes = self.verified_bytes.saturating_add(observed.1);
        }
        let charge = self.plan.charge(files.max(observed.0), charged);
        result?;
        charge?;
        ctl.check()?;
        if let (Some(parent), Some(parts)) = (self.plan.nodes[id].parent, split_pins.as_deref()) {
            if let Some(proof) = confirmed_numbered_7z(parts, &opts) {
                let parent_tree = Path::new(
                    self.plan.nodes[parent]
                        .tree
                        .as_deref()
                        .ok_or("缺少分卷暂存目录")?,
                );
                let locator = self.locator(parent).join(" → ");
                for part in proof.iter().skip(1) {
                    if let Ok(relative) = part.path.strip_prefix(parent_tree) {
                        self.retained_matching_files
                            .remove(&format!("{locator} → {}", relative.display()));
                    }
                }
                self.verified_split_parts.insert(id, proof);
            }
        }
        ctl.check()?;
        let mut paths = vec![];
        for entry in &entries {
            if !entry.is_dir && selected.contains(&entry.name) {
                paths.push(tree.join(validate_name(&entry.name)?));
            }
        }
        paths.sort_by_key(|path| !crate::deep_volume_plan::first_probe(&ordinary_name(path)));
        // Bounded discovery: once the archive queue is full, retain the remaining files.
        let mut candidates = vec![];
        let mut archive_parts = HashSet::new();
        let mut possible_companions = HashSet::new();
        for path in &paths {
            ctl.check()?;
            if self.plan.nodes.len() + candidates.len() > self.plan.limits.archives {
                self.plan.undiscovered_sources += 1;
                self.plan.nodes[id].unscanned = true;
                break;
            }
            let mut companion = possible_companions.contains(path);
            if let crate::deep_volume_plan::Member::Later {
                family: group,
                expected,
                first,
            } = volume_member(path, &paths)
            {
                if first.is_none() {
                    let key = missing_volume_key(path, &group);
                    if !self.sources.contains_key(&key) {
                        self.sources.insert(
                            key.clone(),
                            Source {
                                path: path.clone(),
                                parts: vec![],
                                kind: Kind::MissingFirst(expected),
                            },
                        );
                        candidates.push(Candidate {
                            source: key,
                            name: path
                                .strip_prefix(&tree)
                                .map_err(|e| e.to_string())?
                                .to_string_lossy()
                                .into_owned(),
                        });
                    }
                    continue;
                }
                companion = true;
            }
            let kind = if let Some(cached) = self.probes.get(path) {
                cached.clone()
            } else {
                let probed = sniff(path)?;
                self.probes.insert(path.clone(), probed.clone());
                probed
            };
            if companion && (kind.is_none() || !standalone_archive(path, &opts)) {
                self.retained_matching_files.insert(format!(
                    "{} → {}",
                    self.locator(id).join(" → "),
                    path.strip_prefix(&tree)
                        .map_err(|e| e.to_string())?
                        .display()
                ));
                continue;
            }
            if let Some(kind) = kind {
                let key = source_key(path);
                possible_companions.extend(family(path, &paths).into_iter().filter(|p| p != path));
                // Names propose discovery groups only. The decoder verified this exact
                // source; no inferred sibling ever receives deletion/collapse authority.
                let parts = vec![path.clone()];
                archive_parts.insert(path.clone());
                self.sources.insert(
                    key.clone(),
                    Source {
                        path: path.clone(),
                        parts,
                        kind,
                    },
                );
                candidates.push(Candidate {
                    source: key,
                    name: path
                        .strip_prefix(&tree)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .into_owned(),
                });
            }
        }
        let mut pure = candidates.len() == 1
            && crate::deep_volume_plan::contains_only_confirmed(&paths, |p| {
                archive_parts.contains(p)
            });
        if pure {
            let mut directories = entries
                .iter()
                .filter(|entry| entry.is_dir && selected.contains(&entry.name))
                .peekable();
            if directories.peek().is_some() {
                let populated: HashSet<_> = paths
                    .iter()
                    .flat_map(|path| {
                        path.ancestors()
                            .skip(1)
                            .take_while(|parent| parent.starts_with(&tree))
                    })
                    .map(path_key)
                    .collect();
                for entry in directories {
                    let directory = tree.join(validate_name(&entry.name)?);
                    if !populated.contains(&path_key(&directory)) {
                        pure = false;
                        break;
                    }
                }
            }
        }
        self.plan.decoded(id, string(&tree)?, candidates, pure);
        if let Some((evidence, password)) = opts.extraction_password_use.zip(opts.password) {
            if evidence.verified(observed.0 as usize)
                && !self.used_passwords.iter().any(|(_, used)| used == &password)
            {
                self.used_passwords.push((self.plan.nodes[id].name.clone(), password));
            }
        }
        Ok(())
    }
    fn assemble(&mut self, id: usize) -> Result<(), String> {
        let node = self.plan.nodes[id].clone();
        let tree = PathBuf::from(node.tree.as_ref().ok_or("缺少解码目录")?);
        if let [child] = node.children.as_slice() {
            if self.plan.nodes[*child].state == State::Done && self.plan.fully_expanded(*child) {
                let mut archive_only = node.archive_only;
                if let Some(parts) = self.verified_split_parts.get(child) {
                    if parts.iter().any(|part| !part.unchanged()) {
                        return Err("已校验的暂存分卷发生变化，恢复结果已保留".into());
                    }
                    let mut entries = vec![];
                    collect_entries(&tree, &tree, &mut entries, &JobControl::new())?;
                    archive_only = entries.iter().all(|entry| {
                        validate_name(&entry.name).is_ok_and(|relative| {
                            let path = tree.join(relative);
                            parts.iter().any(|part| {
                                if entry.is_dir {
                                    part.path.starts_with(&path)
                                } else {
                                    part.path == path
                                }
                            })
                        })
                    });
                }
                if archive_only {
                    let child_tree = self.plan.nodes[*child].tree.clone().ok_or("缺少内层结果")?;
                    self.plan.assembled(id, child_tree);
                    return Ok(());
                }
            }
        }
        for child in &node.children {
            if self.plan.nodes[*child].state != State::Done {
                continue;
            }
            let source = self
                .sources
                .get(&self.plan.nodes[*child].source)
                .ok_or("缺少内层来源")?;
            let mut child_tree = PathBuf::from(
                self.plan.nodes[*child]
                    .tree
                    .as_ref()
                    .ok_or("缺少内层结果")?,
            );
            let parent = source.path.parent().ok_or("缺少内层父目录")?;
            let mut entries = vec![];
            collect_entries(&child_tree, &child_tree, &mut entries, &JobControl::new())?;
            let destination = crate::smart_extract::destination(
                &string(&source.path)?,
                &string(parent)?,
                &entries,
                &EngineOptions::default(),
            )?;
            let stem = if Path::new(&destination) == parent {
                let folder = crate::smart_extract::archive_name(&string(&source.path)?)?;
                child_tree = child_tree.join(&folder);
                string(&parent.join(folder))?
            } else {
                destination
            };
            let mut target = PathBuf::from(&stem);
            let mut suffix = 2;
            while target.exists() {
                target = parent.join(format!(
                    "{} ({suffix})",
                    Path::new(&stem)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ));
                suffix += 1;
            }
            if !self.attached.contains(child) {
                fs::rename(&child_tree, &target).map_err(|e| e.to_string())?;
                self.plan.nodes[*child].tree = Some(string(&target)?);
                self.attached.insert(*child);
            }
            // Remove only decoded, session-owned intermediates whose entire subtree expanded.
            if self.plan.fully_expanded(*child) {
                if let Some(parts) = self.verified_split_parts.get(child) {
                    if parts.iter().any(|part| !part.unchanged()) {
                        return Err("已校验的暂存分卷发生变化，恢复结果已保留".into());
                    }
                    for part in parts {
                        remove_private_file(&part.path)?;
                    }
                } else {
                    for part in &source.parts {
                        remove_private_file(part)?;
                    }
                }
            }
        }
        self.plan.assembled(id, string(&tree)?);
        Ok(())
    }
    pub fn drive(&mut self, ctl: &JobControl) -> Result<Report, String> {
        let result = self.drive_inner(ctl);
        self.options.password = None;
        *ctl.context.lock().unwrap_or_else(|e| e.into_inner()) = None;
        result
    }

    pub fn used_passwords(&self) -> impl Iterator<Item = (&str, &str)> {
        self.used_passwords.iter().map(|(label, password)| (label.as_str(), password.as_str()))
    }
    fn drive_inner(&mut self, ctl: &JobControl) -> Result<Report, String> {
        loop {
            ctl.check()?;
            if let Some(id) = self.plan.assembly_ready() {
                if let Err(error) = self.assemble(id) {
                    self.plan.pending(id, error);
                }
                continue;
            }
            let Some(id) = self.plan.next() else {
                break;
            };
            let passwords = if self.passwords.is_empty() {
                vec![None]
            } else {
                self.passwords.iter().cloned().map(Some).collect()
            };
            let mut failure = None;
            for password in passwords {
                self.options.password = password;
                match self.decode(id, ctl) {
                    Ok(()) => {
                        failure = None;
                        break;
                    }
                    Err(error) => {
                        if ctl.cancelled.load(Ordering::SeqCst) {
                            return Err(error);
                        }
                        let retry = password_error(&error);
                        failure = Some(error);
                        if !retry {
                            break;
                        }
                    }
                }
            }
            self.options.password = None;
            if let Some(error) = failure {
                self.plan.pending(id, error);
            }
        }
        self.publish(ctl)?;
        Ok(self.report())
    }
    fn publish(&mut self, ctl: &JobControl) -> Result<(), String> {
        self.publish_error = None;
        let ready: Vec<_> = self
            .plan
            .roots
            .iter()
            .copied()
            .filter(|id| !self.published.contains(id) && self.plan.nodes[*id].state == State::Done)
            .collect();
        if !ready.is_empty() {
            // Keep previously discovered members and include any newly supplied volumes.
            let mut directories = HashMap::new();
            let mut complete = true;
            for path in &self.supplied_inputs {
                complete &= add_root_family(path, &mut self.root_inputs, &mut directories);
            }
            let protection = file_workspace::DeepInputProtection::new(&self.root_inputs, complete);
            match protection {
                Ok(protection) => {
                    let mut errors = Vec::new();
                    for id in ready {
                        ctl.check()?;
                        if let Err(error) = self.publish_root(id, ctl, &protection) {
                            ctl.check()?;
                            errors.push(format!("{}：{error}", self.plan.nodes[id].name));
                        }
                    }
                    if !errors.is_empty() {
                        self.publish_error = Some(errors.join("\n"));
                    }
                }
                Err(error) => self.publish_error = Some(error),
            }
        }
        self.finished = self.publish_error.is_none()
            && self.plan.complete()
            && self.plan.roots.iter().all(|id| {
                self.published.contains(id)
                    || matches!(
                        self.plan.nodes[*id].state,
                        State::Skipped | State::Protected
                    )
            });
        Ok(())
    }
    fn publish_root(
        &mut self,
        id: usize,
        ctl: &JobControl,
        protection: &file_workspace::DeepInputProtection,
    ) -> Result<(), String> {
        let tree = PathBuf::from(self.plan.nodes[id].tree.as_ref().ok_or("缺少最终结果")?);
        let mut entries = vec![];
        collect_entries(&tree, &tree, &mut entries, ctl)?;
        let mut opts = self.options.clone();
        opts.selected.clear();
        opts.control = Some(ctl.clone());
        let source = self
            .sources
            .get(&self.plan.nodes[id].source)
            .ok_or("缺少原包来源")?;
        let base = self
            .dest
            .clone()
            .unwrap_or_else(|| source.path.parent().unwrap_or(&source.path).to_path_buf());
        let folder = crate::smart_extract::archive_name(&string(&source.path)?)?;
        let target = if self.smart {
            PathBuf::from(crate::smart_extract::destination(
                &string(&source.path)?,
                &string(&base)?,
                &entries,
                &opts,
            )?)
        } else {
            base.join(crate::extract::relative_path(&folder)?)
        };
        let mut roots = entries.iter().filter(|entry| !entry.name.contains('/'));
        let wrapper = if target == base && self.dedicated_output {
            let wrapper = roots.next().ok_or("已校验的顶层目录发生变化")?;
            if !wrapper.is_dir || roots.next().is_some() {
                return Err("已校验的顶层目录发生变化".into());
            }
            Some(wrapper.name.as_str())
        } else {
            None
        };
        let target = wrapper.map_or(target, |wrapper| base.join(wrapper));
        let inputs = entries.iter().filter(|entry| {
            if let Some(wrapper) = wrapper {
                entry.name.strip_prefix(wrapper).and_then(|name| name.strip_prefix('/'))
                    .is_some_and(|name| !name.contains('/'))
            } else {
                !entry.name.contains('/')
            }
        }).map(|entry| validate_name(&entry.name).and_then(|path| string(&tree.join(path))))
            .collect::<Result<Vec<_>, _>>()?;
        let bytes = entries.iter().filter(|entry| !entry.is_dir).try_fold(0u64, |size, entry| {
            size.checked_add(entry.size).ok_or("空间需求溢出")
        })?;
        if !has_stage_space(
            base.ancestors()
                .find(|p| p.is_dir())
                .ok_or("目标路径无效")?,
            bytes.saturating_add(self.plan.limits.reserve_bytes),
        ) {
            return Err("目标磁盘空间不足，已校验结果保留在恢复目录".into());
        }
        let mut guards = CheckedDirectories::default();
        let target = if self.dedicated_output {
            guards.create_dirs(&base)?;
            if let Some(assigned) = self.assigned_targets.get(&id) {
                guards.create_dirs(assigned)?;
                assigned.clone()
            } else {
                let name = ordinary_name(&target);
                let mut chosen = None;
                for suffix in 1..10000 {
                    let candidate = if suffix == 1 {
                        target.clone()
                    } else {
                        base.join(format!("{name} ({suffix})"))
                    };
                    match fs::create_dir(&candidate) {
                        Ok(()) => {
                            chosen = Some(candidate);
                            break;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(error) => return Err(error.to_string()),
                    }
                }
                let chosen = chosen.ok_or("未找到可用的专属输出目录名称")?;
                self.assigned_targets.insert(id, chosen.clone());
                chosen
            }
        } else {
            guards.create_dirs(&target)?;
            target
        };
        drop(guards); // The existing copier acquires its own publication-compatible pins.
        ctl.set(
            "deep-publish",
            None,
            "正在保存已校验的解压结果；取消后已保存的文件会保留",
        );
        let output = string(&target)?;
        let before = self.publication.len();
        let result = if inputs.is_empty() {
            Ok(0)
        } else {
            file_workspace::copy_for_deep(
                &inputs,
                &output,
                &opts,
                &mut self.publication,
                protection,
            )
            .map(|result| result.skipped)
        };
        if (matches!(result, Ok(0)) || self.publication.len() > before)
            && !self.outputs.contains(&output)
        {
            self.outputs.push(output);
        }
        let skipped = result?;
        if skipped > 0 {
            return Err(format!(
                "{skipped} 个同名项目已跳过；完整结果保留在恢复目录"
            ));
        }
        self.published.insert(id);
        if entries.iter().any(|entry| !entry.is_dir) {
            self.published_files.insert(id);
        }
        Ok(())
    }
    pub fn report(&self) -> Report {
        let pending = self
            .plan
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| {
                if let State::Pending(reason) = &node.state {
                    let ancestry = self.locator(id);
                    Some(Pending {
                        node: id,
                        name: node.name.clone(),
                        source: ancestry.join(" → "),
                        ancestry,
                        reason: reason.clone(),
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let protected_archives = self
            .plan
            .nodes
            .iter()
            .filter(|n| n.state == State::Protected)
            .map(|n| n.name.clone())
            .collect::<Vec<_>>();
        let retained = self
            .plan
            .nodes
            .iter()
            .enumerate()
            .filter(|(id, node)| {
                (node.parent.is_some() && !self.plan.fully_expanded(*id))
                    || (node.parent.is_none() && node.state == State::Skipped)
            })
            .map(|(id, _)| self.locator(id).join(" → "))
            .collect::<Vec<_>>();
        let partial = !pending.is_empty()
            || !retained.is_empty()
            || self.plan.nodes.iter().any(|n| n.state == State::Skipped)
            || self.publish_error.is_some()
            || self.plan.undiscovered_sources > 0;
        let mut summary = if !pending.is_empty() {
            format!(
                "已处理 {} 个压缩包，已保存 {} 个文件；{} 项等待重试或保留。未保存的已校验结果保留在恢复目录。",
                self.plan
                    .nodes
                    .iter()
                    .filter(|n| matches!(n.state, State::Done | State::Waiting))
                    .count(),
                self.publication.len(),
                pending.len()
            )
        } else if self.publish_error.is_some() {
            "部分结果已保存；其余已校验数据保留在恢复目录。请检查同名文件策略后重试保存。".into()
        } else if partial {
            format!(
                "深度解压已结束，已保存 {} 个文件；未展开的内层压缩包已保留。",
                self.publication.len()
            )
        } else {
            format!(
                "深度解压完成，已保存 {} 个文件；原压缩包已保留。",
                self.publication.len()
            )
        };
        if !self.retained_matching_files.is_empty() {
            summary.push_str(&format!(
                " 另保留 {} 个同名匹配文件，未凭文件名推断分卷身份。",
                self.retained_matching_files.len()
            ));
        }
        Report {
            destination: self
                .outputs
                .first()
                .cloned()
                .or_else(|| self.dest.as_ref().map(|dest| dest.display().to_string()))
                .unwrap_or_default(),
            outputs: self.outputs.clone(),
            files: self.publication.len(),
            partial,
            pending,
            recovery_path: (!self.finished)
                .then(|| self.stage.as_ref().unwrap().path().display().to_string()),
            decoded_bytes: self.plan.decoded_bytes,
            decoded_files: self.plan.decoded_files,
            archives: self.plan.nodes.len(),
            max_depth: self
                .plan
                .nodes
                .iter()
                .map(|node| node.depth)
                .max()
                .unwrap_or(0),
            summary,
            retained_archives: retained,
            undiscovered_sources: self.plan.undiscovered_sources,
            publish_error: self.publish_error.clone(),
            finished: self.finished,
            accounted_bytes: self.plan.decoded_bytes,
            verified_output_bytes: self.verified_bytes,
            accounting_estimated: self.estimated,
            protected_archives,
            retained_matching_files: self.retained_matching_files.iter().cloned().collect(),
        }
    }
    /// Only hand fully expanded and published roots to the desktop's identity
    /// guards. A resolved volume alias or protected container is never a source.
    pub fn for_each_recyclable_root(
        &self,
        mut confirm: impl FnMut(&Path, &[String]) -> Result<(), String>,
    ) -> Result<(), String> {
        if !self.finished
            || !self.options.selected.is_empty()
            || self.plan.roots.len() != self.supplied_inputs.len()
            || self.plan.undiscovered_sources != 0
            || !self.retained_matching_files.is_empty()
        {
            return Err("任务未完整展开，或包含无法确认的来源，原压缩包已保留".into());
        }
        for id in &self.plan.roots {
            let source = &self.sources[&self.plan.nodes[*id].source];
            if !self.published_files.contains(id)
                || !self.plan.fully_expanded(*id)
                || source.parts.len() != 1
                || !self.supplied_inputs.contains(&source.path)
            {
                return Err("无法确认原压缩包为已完整解压的单卷，原压缩包已保留".into());
            }
            confirm(&source.path, &self.passwords)?;
        }
        Ok(())
    }
    fn locator(&self, mut id: usize) -> Vec<String> {
        let mut parts = vec![];
        loop {
            let node = &self.plan.nodes[id];
            if let Some(parent) = node.parent {
                parts.push(node.name.clone());
                id = parent;
            } else {
                parts.push(
                    self.sources
                        .get(&node.source)
                        .map(|s| s.path.display().to_string())
                        .unwrap_or_else(|| node.name.clone()),
                );
                break;
            }
        }
        parts.reverse();
        parts
    }
    pub fn retry_publication(&mut self, overwrite: String) -> Result<(), String> {
        if !["error", "overwrite", "skip", "rename"].contains(&overwrite.as_str()) {
            return Err("同名文件处理方式无效".into());
        }
        self.options.overwrite = overwrite;
        Ok(())
    }
}
fn remove_private_file(path: &Path) -> Result<(), String> {
    match fs::metadata(path) {
        Ok(meta) => {
            if meta.permissions().readonly() {
                let mut permissions = meta.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(path, permissions).map_err(|e| e.to_string())?;
            }
            fs::remove_file(path).map_err(|e| e.to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
fn collect_entries(
    base: &Path,
    path: &Path,
    out: &mut Vec<Entry>,
    ctl: &JobControl,
) -> Result<(), String> {
    ctl.check()?;
    for child in fs::read_dir(path).map_err(|e| e.to_string())? {
        let child = child.map_err(|e| e.to_string())?;
        let meta = child.metadata().map_err(|e| e.to_string())?;
        let p = child.path();
        out.push(Entry {
            name: p
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/"),
            size: meta.len(),
            size_known: true,
            is_dir: meta.is_dir(),
            packed_size: 0,
            encrypted: false,
            created: String::new(),
            accessed: String::new(),
            modified: String::new(),
            crc: String::new(),
            method: String::new(),
            comment: String::new(),
            attributes: String::new(),
        });
        if meta.is_dir() {
            collect_entries(base, &p, out, ctl)?;
        }
    }
    Ok(())
}

fn decode_stream(
    path: &Path,
    target: &Path,
    format: &str,
    opts: &EngineOptions,
    limit: u64,
    reserve: u64,
) -> Result<(), String> {
    use crate::engine_password::PasswordInput;
    use std::sync::mpsc;
    let ctl = control(opts);
    ctl.check()?;
    let source_guard = crate::engine_stream::pin(path)?;
    let mut command = Command::new(executable()?);
    command
        .args(["x", "-so", "-y", "-sccUTF-8", "-spd", "-bso0", "-bsp0"])
        .arg(format!("-t{format}"));
    if let Some(threads) = opts.threads {
        command.arg(format!("-mmt={threads}"));
    }
    let errors = tempfile::tempfile().map_err(|e| e.to_string())?;
    command
        .arg("--")
        .arg(path)
        .stdin(if opts.password.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(errors.try_clone().map_err(|e| e.to_string())?);
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
    let mut output = File::options()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| e.to_string())?;
    let mut child = super::spawn_archive_process(&mut command).map_err(|e| e.to_string())?;
    let password = if let Some(password) = opts.password.as_deref() {
        match PasswordInput::spawn(&mut child, password) {
            Ok(v) => Some(v),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        }
    } else {
        None
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("解码输出不可用".into());
    };
    let disk = target.parent().ok_or("缺少解码目标")?.to_owned();
    let worker_ctl = ctl.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let attempt =
            crate::deep_stream_copy::copy_bounded(&mut stdout, &mut output, limit, &mut |next| {
                worker_ctl.check()?;
                if !has_stage_space(&disk, reserve.saturating_add(next)) {
                    return Err("磁盘剩余空间不足，已停止解码".into());
                }
                Ok(())
            });
        let _ = tx.send(attempt.error.is_some());
        attempt
    });
    let status = loop {
        if let Err(e) = ctl.check() {
            break Err(e);
        }
        if rx.try_recv() == Ok(true) {
            break Ok(None);
        }
        match child.try_wait() {
            Ok(Some(s)) => break Ok(Some(s)),
            Ok(None) => {
                if let Err(e) = wait_for_process(&child, Duration::from_millis(20)) {
                    break Err(e.to_string());
                }
            }
            Err(e) => break Err(e.to_string()),
        }
    };
    if !matches!(&status, Ok(Some(_))) {
        let _ = child.kill();
        let _ = child.wait();
    }
    let attempt = worker.join().map_err(|_| "解码输出线程异常")?;
    if let Some(password) = password {
        password.finish()?;
    }
    drop(source_guard);
    if let Some(error) = attempt.error {
        return Err(error);
    }
    let status = status?.ok_or("解码已中断")?;
    if !status.success() {
        let mut errors = errors;
        errors.rewind().map_err(|e| e.to_string())?;
        let mut message = String::new();
        let _ = errors.take(4096).read_to_string(&mut message);
        return Err(format!("解码或校验失败：{}", message.trim()));
    }
    ctl.check()
}
