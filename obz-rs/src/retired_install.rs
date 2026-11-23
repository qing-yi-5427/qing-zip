//! Explicit, recoverable relocation of the known x64 v0.12.20 leftovers.
use crate::{engine, private_stage};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::File,
    io::Read,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf, Prefix},
    time::{SystemTime, UNIX_EPOCH},
};

const RELEASE: &[(&str, u64, &str)] = &[
    (
        "licenses/7zip-26.05/License.txt",
        5885,
        "32369594a3a9f7c643d124035120eaa6a7707e75e57c4386ef509f801447bc49",
    ),
    (
        "licenses/7zip-26.05/README.txt",
        433,
        "8aec3ac469c6aae7adc8424e234a256c6d885c9c173abdc0312eb7ec20d46ab7",
    ),
    (
        "licenses/python-3.14.9/LICENSE-ACEFILE.md",
        2117,
        "b78ee8da84166064863c3f8f0ffce4768393c56ba6e5c68412d2198fb5ebcdb3",
    ),
    (
        "licenses/python-3.14.9/LICENSE.txt",
        34824,
        "939d8e9fa591cbaff0d51e777edb4c1002045d2600ce43a1ae28530cecfc5b88",
    ),
    (
        "licenses/rust/form_urlencoded-1.2.2/LICENSE-APACHE",
        10847,
        "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    ),
    (
        "licenses/rust/form_urlencoded-1.2.2/LICENSE-MIT",
        1072,
        "20c7855c364d57ea4c97889a5e8d98470a9952dade37bd9248b9a54431670e5e",
    ),
    (
        "licenses/rust/url-2.5.8/LICENSE-APACHE",
        10847,
        "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    ),
    (
        "licenses/rust/url-2.5.8/LICENSE-MIT",
        1072,
        "b38f11f6096706e6de553dabe2a7ed142d59b6fa8c97e290c67496154745cdd5",
    ),
    (
        "runtime/zpaq/zpaq.exe",
        1094656,
        "a0f127a70943b0262060498c1723c795a8e2980f1acf0c42ee8c1dae72ae54b5",
    ),
    (
        "runtime/zpaq/zpaq64.exe",
        1125376,
        "7a94b4f1d6323a758c7b0b6344036f166bff0fd44f1c3c86f05b3688023496cb",
    ),
];

#[derive(PartialEq, Eq)]
struct Stamp {
    id: (u32, u64),
    created: u64,
    written: u64,
}

pub struct Candidate {
    entry: &'static (&'static str, u64, &'static str),
    stamp: Stamp,
}

impl Candidate {
    pub fn relative_path(&self) -> &'static str {
        self.entry.0
    }
}

pub struct Review {
    // Preserve ancestry and installation directory identity through selection.
    install: Vec<File>,
    recovery: PathBuf,
    candidates: Vec<Candidate>,
}

impl Review {
    pub fn open(root: &Path) -> Result<Self, String> {
        let install = pin_install_root(root)?;
        if install.len() < 3 {
            return Err("安装目录需要独立的上级文件夹".into());
        }
        let when = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?;
        let leaf = format!(
            "qing-zip-v0.12.20-Recovery-{}-{}",
            std::process::id(),
            when.as_millis()
        );
        let recovery = root.with_file_name(leaf);
        let source = install.last().unwrap();
        let candidates = RELEASE
            .iter()
            .filter_map(|entry| {
                let (_, _, stamp) = inspect(source, entry, false).ok()?;
                Some(Candidate { entry, stamp })
            })
            .collect();
        Ok(Self {
            install,
            recovery,
            candidates,
        })
    }

    pub fn recovery_path(&self) -> &Path {
        &self.recovery
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// The caller supplies only the items individually confirmed by the user.
    /// A new sibling recovery directory follows ordinary user ACL inheritance.
    pub fn relocate(&self, selected: &[usize]) -> Result<Vec<(&'static str, bool)>, String> {
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        let source = self.install.last().unwrap();
        let sibling_parent = &self.install[self.install.len() - 2];
        let leaf = self.recovery.file_name().ok_or("恢复目录没有名称")?;
        let recovery = create_directory(sibling_parent, leaf)?;
        let mut dirs = HashMap::from([(PathBuf::new(), recovery)]);
        let mut report = Vec::new();
        for &i in selected {
            let item = self.candidates.get(i).ok_or("未知的迁移候选")?;
            let name = item.relative_path();
            let moved = (|| -> Result<bool, String> {
                let (_source_parents, file, stamp) = inspect(source, item.entry, true)?;
                if stamp != item.stamp {
                    return Ok(false);
                }
                let relative = Path::new(name);
                let mut prefix = PathBuf::new();
                for component in relative.parent().ok_or("候选缺少上级目录")?.components() {
                    let parent = prefix.clone();
                    prefix.push(component.as_os_str());
                    if !dirs.contains_key(&prefix) {
                        let created = create_directory(&dirs[&parent], component.as_os_str())?;
                        dirs.insert(prefix.clone(), created);
                    }
                }
                let target = self.recovery.join(relative);
                engine::rename_handle_noclobber_in(&file, &target, Some(&dirs[&prefix]))
            })()
            .unwrap_or(false);
            report.push((name, moved));
        }
        Ok(report)
    }
}

fn checked_directory(file: File) -> Result<File, String> {
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
        return Err("安装或恢复路径不是普通目录".into());
    }
    Ok(file)
}

fn pin_install_root(path: &Path) -> Result<Vec<File>, String> {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Err("安装路径没有盘符".into());
    };
    if !matches!(prefix.kind(), Prefix::Disk(_)) || parts.next() != Some(Component::RootDir) {
        return Err("只支持普通本地安装路径".into());
    }
    let mut drive = PathBuf::from(prefix.as_os_str());
    drive.push(r"\");
    #[link(name = "Kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    let wide: Vec<_> = drive.as_os_str().encode_wide().chain(Some(0)).collect();
    // DRIVE_FIXED. Mounted network paths and removable disks are left alone.
    if unsafe { GetDriveTypeW(wide.as_ptr()) } != 3 {
        return Err("旧版文件检查只支持本地固定磁盘".into());
    }

    let root = File::options()
        .access_mode(0x100081)
        .share_mode(1)
        .custom_flags(0x02200000)
        .open(drive)
        .map_err(|e| e.to_string())?;
    let mut chain = vec![checked_directory(root)?];
    for component in parts {
        let Component::Normal(leaf) = component else {
            return Err("安装目录包含不受支持的路径".into());
        };
        let directory = private_stage::open_checksum_child(chain.last().unwrap(), leaf, true)?;
        chain.push(checked_directory(directory)?);
    }
    Ok(chain)
}

fn create_directory(parent: &File, leaf: &OsStr) -> Result<File, String> {
    let created = private_stage::create_output_directory(parent, leaf)?;
    let pinned = checked_directory(private_stage::open_checksum_child(parent, leaf, true)?)?;
    if engine::directory_identity(&created)? != engine::directory_identity(&pinned)? {
        return Err("恢复目录发生变化".into());
    }
    Ok(pinned)
}

fn inspect(
    root: &File,
    entry: &(&str, u64, &str),
    rename: bool,
) -> Result<(Vec<File>, File, Stamp), String> {
    let (relative, size, hash) = *entry;
    let mut parts = relative.split('/').peekable();
    let mut chain = Vec::new();
    let mut parent = root;
    let mut file = loop {
        let leaf = OsStr::new(parts.next().ok_or("空的旧版文件路径")?);
        if parts.peek().is_none() {
            break if rename {
                private_stage::open_retired_rename_source(parent, leaf)?
            } else {
                private_stage::open_checksum_child(parent, leaf, false)?
            };
        }
        chain.push(checked_directory(private_stage::open_checksum_child(
            parent, leaf, true,
        )?)?);
        parent = chain.last().unwrap();
    };
    let meta = private_stage::checked_metadata(&file)?;
    if !meta.is_file() || meta.file_attributes() & !(0x20 | 0x80) != 0 || meta.len() != size {
        return Err("当前文件不是可确认的普通旧版文件".into());
    }
    let stamp = Stamp {
        id: engine::directory_identity(&file)?,
        created: meta.creation_time(),
        written: meta.last_write_time(),
    };
    let mut bytes = Vec::with_capacity(size as usize);
    (&mut file)
        .take(size + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != size || crate::checksum::hex(&Sha256::digest(&bytes)) != hash {
        return Err("当前文件内容已经变化".into());
    }
    Ok((chain, file, stamp))
}
