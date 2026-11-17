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
}
#[derive(Clone)]
struct Source {
    path: PathBuf,
    parts: Vec<PathBuf>,
    kind: Kind,
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
    pub summary: String,
    pub retained_archives: Vec<String>,
    pub undiscovered_sources: u64,
    pub publish_error: Option<String>,
    pub finished: bool,
    pub accounted_bytes: u64,
    pub verified_output_bytes: u64,
    pub accounting_estimated: bool,
    pub protected_archives: Vec<String>,
}
pub struct Session {
    stage: Option<ShellDragDirectory>,
    plan: Plan,
    sources: HashMap<String, Source>,
    options: EngineOptions,
    dest: PathBuf,
    smart: bool,
    outputs: Vec<String>,
    files: usize,
    published: BTreeSet<usize>,
    attached: BTreeSet<usize>,
    finished: bool,
    publish_error: Option<String>,
    serial: usize,
    passwords: Vec<String>,
    probes: HashMap<PathBuf, Option<Kind>>,
    verified_bytes: u64,
    estimated: bool,
    publication: file_workspace::DeepPublicationLedger,
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
        ".001",
    ]
    .iter()
    .any(|s| lower.ends_with(s))
}
fn sniff(path: &Path) -> Result<Option<Kind>, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut head = [0u8; 512];
    let n = file.read(&mut head).map_err(|e| e.to_string())?;
    let h = &head[..n];
    if h.starts_with(&[0x1f, 0x8b]) {
        return Ok(Some(Kind::Stream("gzip")));
    }
    if h.starts_with(b"BZh") {
        return Ok(Some(Kind::Stream("bzip2")));
    }
    if h.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        return Ok(Some(Kind::Stream("xz")));
    }
    if h.starts_with(b"PK\x03\x04")
        || h.starts_with(b"PK\x05\x06")
        || h.starts_with(b"PK\x07\x08")
        || h.starts_with(b"7z\xbc\xaf\x27\x1c")
        || h.starts_with(b"Rar!\x1a\x07")
        || (n >= 262 && &h[257..262] == b"ustar")
    {
        return Ok(Some(Kind::Archive));
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
fn application_container(entries: &[Entry]) -> bool {
    let names: HashSet<_> = entries
        .iter()
        .map(|e| e.name.replace('\\', "/").to_ascii_lowercase())
        .collect();
    (names.contains("[content_types].xml")
        && names
            .iter()
            .any(|n| n.starts_with("word/") || n.starts_with("xl/") || n.starts_with("ppt/")))
        || (names.contains("androidmanifest.xml")
            && names
                .iter()
                .any(|n| n == "classes.dex" || n.starts_with("res/")))
        || names.contains("meta-inf/manifest.mf")
        || (names.contains("mimetype")
            && (names.contains("meta-inf/container.xml") || names.contains("content.xml")))
}
fn secondary_volume(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if let Some((_, tail)) = lower.rsplit_once('.') {
        if tail.len() >= 3 && tail.bytes().all(|b| b.is_ascii_digit()) {
            return tail.parse::<u32>().is_ok_and(|n| n != 1);
        }
        if tail.len() == 3
            && matches!(tail.as_bytes()[0], b'z' | b'r')
            && tail.as_bytes()[1..].iter().all(u8::is_ascii_digit)
        {
            return true;
        }
    }
    if let Some(i) = lower.rfind(".part") {
        if let Some(number) = lower[i + 5..].strip_suffix(".rar") {
            return number.parse::<u32>().is_ok_and(|n| n != 1);
        }
    }
    false
}
fn family(path: &Path, peers: &[PathBuf]) -> Vec<PathBuf> {
    let name = ordinary_name(path).to_ascii_lowercase();
    let parent = path.parent();
    let numeric = name.strip_suffix(".001");
    let rar = name.rfind(".part").and_then(|i| {
        name[i + 5..]
            .strip_suffix(".rar")
            .filter(|n| n.parse::<u32>().ok() == Some(1))
            .map(|_| &name[..i])
    });
    peers
        .iter()
        .filter(|p| {
            if p.parent() != parent {
                return false;
            }
            let other = ordinary_name(p).to_ascii_lowercase();
            if other == name {
                return true;
            }
            if let Some(prefix) = numeric {
                return other
                    .strip_prefix(&format!("{prefix}."))
                    .is_some_and(|n| n.len() >= 3 && n.bytes().all(|b| b.is_ascii_digit()));
            }
            if let Some(prefix) = rar {
                return other
                    .strip_prefix(&format!("{prefix}.part"))
                    .and_then(|n| n.strip_suffix(".rar"))
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            }
            if let Some(prefix) = name.strip_suffix(".zip") {
                return other
                    .strip_prefix(&format!("{prefix}.z"))
                    .is_some_and(|n| n.len() == 2 && n.bytes().all(|b| b.is_ascii_digit()));
            }
            if let Some(prefix) = name.strip_suffix(".rar") {
                return other
                    .strip_prefix(&format!("{prefix}.r"))
                    .is_some_and(|n| n.len() == 2 && n.bytes().all(|b| b.is_ascii_digit()));
            }
            false
        })
        .cloned()
        .collect()
}
fn first_volume(path: &Path) -> Result<PathBuf, String> {
    let name = ordinary_name(path).to_ascii_lowercase();
    if !secondary_volume(&name) {
        return Ok(path.to_owned());
    }
    let parent = path.parent().ok_or("分卷缺少父目录")?;
    let prefix = if let Some((base, tail)) = name.rsplit_once('.') {
        if tail.len() >= 3 && tail.bytes().all(|b| b.is_ascii_digit()) {
            Some(format!("{base}.001"))
        } else if tail.starts_with('z') && tail.len() == 3 {
            Some(format!("{base}.zip"))
        } else if tail.starts_with('r') && tail.len() == 3 {
            Some(format!("{base}.rar"))
        } else {
            None
        }
    } else {
        None
    };
    for item in fs::read_dir(parent).map_err(|e| e.to_string())? {
        let p = item.map_err(|e| e.to_string())?.path();
        let n = ordinary_name(&p).to_ascii_lowercase();
        if prefix.as_ref() == Some(&n) {
            return Ok(p);
        }
        if let Some(i) = name.rfind(".part") {
            if let Some(number) = n
                .strip_prefix(&format!("{}.part", &name[..i]))
                .and_then(|s| s.strip_suffix(".rar"))
            {
                if number.parse::<u32>().ok() == Some(1) {
                    return Ok(p);
                }
            }
        }
    }
    Err(format!(
        "缺少第一卷，无法处理 {}；请将全部分卷放在同一文件夹",
        ordinary_name(path)
    ))
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
        let dest = absolute(destination)?;
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
                Err(error) => (input.clone(), Kind::Invalid(error)),
            };
            let key = source_key(&path);
            roots.push(Candidate {
                source: key.clone(),
                name: ordinary_name(&path),
            });
            sources.insert(
                key,
                Source {
                    parts: family(&path, &peers),
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
        let note="这是深度解压的恢复目录。原压缩包始终保留。node-* 包含已校验的中间结果；发生中断时可从这里恢复文件。此目录不保存密码。\r\n";
        fs::write(stage.path().join("恢复说明.txt"), note).map_err(|e| e.to_string())?;
        let passwords = options.password.take().into_iter().collect();
        Ok(Self {
            stage: Some(stage),
            plan,
            sources,
            options,
            dest,
            smart,
            outputs: vec![],
            files: 0,
            published: BTreeSet::new(),
            attached: BTreeSet::new(),
            finished: false,
            publish_error: None,
            serial: 0,
            passwords,
            probes: HashMap::new(),
            verified_bytes: 0,
            estimated: false,
            publication: Default::default(),
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
        *ctl.context.lock().unwrap_or_else(|e|e.into_inner())=Some(format!(
            "第 {} / {} 层 · {}\n已处理 {} 个文件 · {:.2} / {:.2} GiB · {} 个压缩包",
            self.plan.nodes[id].depth,self.plan.limits.depth,self.locator(id).join(" → "),
            self.plan.decoded_files,self.plan.decoded_bytes as f64/1073741824.0,
            self.plan.limits.bytes as f64/1073741824.0,self.plan.nodes.len()
        ));
        let source = self
            .sources
            .get(&self.plan.nodes[id].source)
            .ok_or("压缩包来源已失效")?
            .clone();
        if application_extension(&source.path) {
            self.plan.nodes[id].state = State::Protected;
            return Ok(());
        }
        if let Kind::Invalid(error) = &source.kind {
            return Err(error.clone());
        }
        let mut opts = self.options.clone();
        opts.control = Some(ctl.clone());
        opts.overwrite = "error".into();
        opts.zone_identifier = Some(crate::motw::archive_context(&source.path)?);
        if self.plan.nodes[id].parent.is_some() {
            opts.selected.clear();
        }
        ctl.set(
            "deep-inspect",
            None,
            &format!(
                "第 {} 层 · {}",
                self.plan.nodes[id].depth,
                self.locator(id).join(" → ")
            ),
        );
        let entries = match source.kind {
            Kind::Archive => {
                let path = string(&source.path)?;
                with_zip_source(&path, &opts, |p| list_raw(p, &opts))?
            }
            Kind::Stream(format) => vec![Entry {
                name: stream_name(&source.path)?,
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
            }],
            Kind::Invalid(ref error) => return Err(error.clone()),
        };
        // Validate ALL names and namespaces, including unselected entries.
        expected_tree(&entries)?;
        if application_container(&entries) {
            self.plan.nodes[id].state = State::Protected;
            return Ok(());
        }
        if entries.iter().any(|e| e.encrypted) && opts.password.is_none() {
            return Err("此压缩包需要密码".into());
        }
        let selected = Selection::new(&opts.selected);
        let files = entries
            .iter()
            .filter(|e| !e.is_dir && selected.contains(&e.name))
            .count() as u64;
        let declared = entries
            .iter()
            .filter(|e| !e.is_dir && selected.contains(&e.name))
            .try_fold(0u64, |sum, e| {
                e.size_known.then(|| sum.checked_add(e.size)).flatten()
            });
        self.plan.preflight(files, declared)?;
        let reserve = self.plan.limits.reserve_bytes;
        let required = declared
            .unwrap_or(65536)
            .checked_mul(2)
            .and_then(|n| n.checked_add(reserve))
            .ok_or("空间需求溢出")?;
        let stage = self.stage.as_ref().ok_or("恢复目录已关闭")?;
        if !has_stage_space(stage.path(), required) {
            return Err("恢复目录所在磁盘空间不足；请更改位置或释放空间后重试".into());
        }
        self.serial += 1;
        let tree = stage.path().join(format!("node-{id}-{}", self.serial));
        fs::create_dir(&tree).map_err(|e| e.to_string())?;
        let result = match source.kind {
            Kind::Archive => with_zip_source(&string(&source.path)?, &opts, |p| {
                extract_raw(p, &string(&tree)?, &opts)
            })
            .map(|_| ()),
            Kind::Stream(format) => {
                let target = tree.join(&entries[0].name);
                decode_stream(
                    &source.path,
                    &target,
                    format,
                    &opts,
                    self.plan.remaining_bytes(),
                    reserve,
                )
                .map(|_| ())
            }
            Kind::Invalid(ref error) => Err(error.clone()),
        };
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
        let mut paths = vec![];
        for entry in &entries {
            if !entry.is_dir && selected.contains(&entry.name) {
                paths.push(tree.join(validate_name(&entry.name)?));
            }
        }
        // Bounded discovery: once the archive queue is full, retain the remaining files.
        let mut candidates = vec![];
        let mut archive_parts = HashSet::new();
        for path in &paths {
            ctl.check()?;
            if secondary_volume(&ordinary_name(path)) {
                continue;
            }
            if self.plan.nodes.len() + candidates.len() > self.plan.limits.archives {
                self.plan.undiscovered_sources += 1;
                self.plan.nodes[id].unscanned = true;
                break;
            }
            let kind = if let Some(cached) = self.probes.get(path) {
                cached.clone()
            } else {
                let probed = sniff(path)?;
                self.probes.insert(path.clone(), probed.clone());
                probed
            };
            if let Some(kind) = kind {
                let key = source_key(path);
                let parts = family(path, &paths);
                archive_parts.extend(parts.iter().cloned());
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
        let meaningful_empty_directory = entries
            .iter()
            .filter(|e| e.is_dir && selected.contains(&e.name))
            .any(|e| {
                validate_name(&e.name).ok().is_some_and(|relative| {
                    !paths.iter().any(|p| p.starts_with(tree.join(&relative)))
                })
            });
        let pure = candidates.len() == 1
            && !paths.is_empty()
            && !meaningful_empty_directory
            && paths.iter().all(|p| archive_parts.contains(p));
        self.plan.decoded(id, string(&tree)?, candidates, pure);
        Ok(())
    }
    fn assemble(&mut self, id: usize) -> Result<(), String> {
        let node = self.plan.nodes[id].clone();
        let tree = PathBuf::from(node.tree.as_ref().ok_or("缺少解码目录")?);
        if node.archive_only
            && node.children.len() == 1
            && self.plan.nodes[node.children[0]].state == State::Done
            && self.plan.fully_expanded(node.children[0])
        {
            let child = self.plan.nodes[node.children[0]]
                .tree
                .clone()
                .ok_or("缺少内层结果")?;
            self.plan.assembled(id, child);
            return Ok(());
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
                for part in &source.parts {
                    remove_private_file(part)?;
                }
            }
        }
        self.plan.assembled(id, string(&tree)?);
        Ok(())
    }
    pub fn drive(&mut self, ctl: &JobControl) -> Result<Report, String> {
        let result = self.drive_inner(ctl);
        self.options.password = None;
        *ctl.context.lock().unwrap_or_else(|e|e.into_inner())=None;
        result
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
        if self.plan.complete() {
            self.publish(ctl)?;
        }
        Ok(self.report())
    }
    fn publish(&mut self, ctl: &JobControl) -> Result<(), String> {
        self.publish_error = None;
        for id in self.plan.roots.clone() {
            if self.published.contains(&id)
                || matches!(self.plan.nodes[id].state, State::Skipped | State::Protected)
            {
                continue;
            }
            ctl.check()?;
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
            let target = if self.smart {
                PathBuf::from(crate::smart_extract::destination(
                    &string(&source.path)?,
                    &string(&self.dest)?,
                    &entries,
                    &opts,
                )?)
            } else {
                self.dest
                    .join(crate::smart_extract::archive_name(&string(&source.path)?)?)
            };
            if !has_stage_space(
                self.dest
                    .ancestors()
                    .find(|p| p.is_dir())
                    .ok_or("目标路径无效")?,
                measure(&tree, ctl)?
                    .1
                    .saturating_add(self.plan.limits.reserve_bytes),
            ) {
                self.publish_error = Some("目标磁盘空间不足，已校验结果保留在恢复目录".into());
                break;
            }
            let mut guards = CheckedDirectories::default();
            guards.create_dirs(&target)?;
            drop(guards); // The existing copier acquires its own publication-compatible pins.
            let inputs = fs::read_dir(&tree)
                .map_err(|e| e.to_string())?
                .map(|e| e.map_err(|e| e.to_string()).and_then(|e| string(&e.path())))
                .collect::<Result<Vec<_>, _>>()?;
            if inputs.is_empty() {
                self.outputs.push(string(&target)?);
                self.published.insert(id);
                continue;
            }
            ctl.set(
                "deep-publish",
                None,
                "正在保存已校验的解压结果；取消后已保存的文件会保留",
            );
            match file_workspace::copy_for_deep(
                &inputs,
                &string(&target)?,
                &opts,
                &mut self.publication,
            ) {
                Ok(result) => {
                    let output = string(&target)?;
                    if !self.outputs.contains(&output) {
                        self.outputs.push(output);
                    }
                    if result.skipped > 0 {
                        self.publish_error = Some(format!(
                            "{} 个同名项目已跳过；完整结果保留在恢复目录",
                            result.skipped
                        ));
                    } else {
                        self.published.insert(id);
                    }
                }
                Err(error) => {
                    self.publish_error = Some(error);
                    break;
                }
            }
        }
        self.files = self.publication.len();
        self.finished = self.publish_error.is_none()
            && self.plan.roots.iter().all(|id| {
                self.published.contains(id)
                    || matches!(
                        self.plan.nodes[*id].state,
                        State::Skipped | State::Protected
                    )
            });
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
            .filter(|(id, node)| (node.parent.is_some() && !self.plan.fully_expanded(*id)) || (node.parent.is_none() && node.state==State::Skipped))
            .map(|(id, _)| self.locator(id).join(" → "))
            .collect::<Vec<_>>();
        let partial = !pending.is_empty()
            || !retained.is_empty()
            || self.plan.nodes.iter().any(|n| n.state == State::Skipped)
            || self.publish_error.is_some()
            || self.plan.undiscovered_sources > 0;
        let summary = if !pending.is_empty() {
            format!(
                "已处理 {} 个压缩包；{} 项等待重试或保留。已校验结果保存在恢复目录。",
                self.plan
                    .nodes
                    .iter()
                    .filter(|n| matches!(n.state, State::Done | State::Waiting))
                    .count(),
                pending.len()
            )
        } else if self.publish_error.is_some() {
            "部分结果已保存；其余已校验数据保留在恢复目录。请检查同名文件策略后重试保存。".into()
        } else if partial {
            format!(
                "深度解压已结束，已保存 {} 个文件；未展开的内层压缩包已保留。",
                self.files
            )
        } else {
            format!(
                "深度解压完成，已保存 {} 个文件；原压缩包已保留。",
                self.files
            )
        };
        Report {
            destination: self
                .outputs
                .first()
                .cloned()
                .unwrap_or_else(|| self.dest.display().to_string()),
            outputs: self.outputs.clone(),
            files: self.publication.len(),
            partial,
            pending,
            recovery_path: (!self.finished)
                .then(|| self.stage.as_ref().unwrap().path().display().to_string()),
            decoded_bytes: self.plan.decoded_bytes,
            decoded_files: self.plan.decoded_files,
            archives: self.plan.nodes.len(),
            summary,
            retained_archives: retained,
            undiscovered_sources: self.plan.undiscovered_sources,
            publish_error: self.publish_error.clone(),
            finished: self.finished,
            accounted_bytes: self.plan.decoded_bytes,
            verified_output_bytes: self.verified_bytes,
            accounting_estimated: self.estimated,
            protected_archives,
        }
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
    let mut child = command.spawn().map_err(|e| e.to_string())?;
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
