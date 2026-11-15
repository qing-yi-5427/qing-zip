//! Local dual-location tasks. Sources remain pinned until verified copies are
//! published. Moves use atomic no-replace renames; copy fallbacks retain sources.
use super::*;
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::os::windows::{
    fs::{MetadataExt, OpenOptionsExt},
    io::AsRawHandle,
};

#[derive(Serialize)]
pub struct LocalEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub linked: bool,
    pub size: u64,
}
#[derive(Serialize)]
pub struct Directory {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<LocalEntry>,
}
pub fn browse(path: &str) -> Result<Directory, String> {
    if path.is_empty() {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLogicalDrives() -> u32;
        }
        let mask = unsafe { GetLogicalDrives() };
        let entries = (0..26)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| {
                let path = format!("{}:\\", char::from(b'A' + i));
                LocalEntry {
                    name: path.clone(),
                    path,
                    is_dir: true,
                    linked: false,
                    size: 0,
                }
            })
            .collect();
        return Ok(Directory {
            path: String::new(),
            parent: None,
            entries,
        });
    }
    let root = absolute(path)?;
    let mut guards = CheckedDirectories::default();
    guards.check(&root)?;
    if !root.is_dir() {
        return Err("不是文件夹".into());
    }
    let mut entries = Vec::new();
    for child in fs::read_dir(&root).map_err(|e| e.to_string())? {
        let child = child.map_err(|e| e.to_string())?;
        let meta = fs::symlink_metadata(child.path()).map_err(|e| e.to_string())?;
        entries.push(LocalEntry {
            name: child.file_name().to_string_lossy().into_owned(),
            path: child.path().display().to_string(),
            is_dir: meta.is_dir(),
            linked: meta.file_attributes() & 0x400 != 0,
            size: meta.len(),
        });
        if entries.len() > 100000 {
            return Err("目录项目过多，请选择更具体的位置".into());
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Directory {
        path: root.display().to_string(),
        parent: root.parent().map(|p| p.display().to_string()),
        entries,
    })
}

type Handle = *mut c_void;
#[link(name = "kernel32")]
extern "system" {
    fn BackupRead(
        file: Handle,
        buffer: *mut u8,
        size: u32,
        read: *mut u32,
        abort: i32,
        security: i32,
        context: *mut Handle,
    ) -> i32;
    fn BackupWrite(
        file: Handle,
        buffer: *const u8,
        size: u32,
        written: *mut u32,
        abort: i32,
        security: i32,
        context: *mut Handle,
    ) -> i32;
}
struct Backup<'a> {
    file: &'a File,
    context: Handle,
    writing: bool,
}
impl Drop for Backup<'_> {
    fn drop(&mut self) {
        let mut n = 0;
        unsafe {
            if self.writing {
                BackupWrite(
                    self.file.as_raw_handle(),
                    std::ptr::null(),
                    0,
                    &mut n,
                    1,
                    0,
                    &mut self.context,
                );
            } else {
                BackupRead(
                    self.file.as_raw_handle(),
                    std::ptr::null_mut(),
                    0,
                    &mut n,
                    1,
                    0,
                    &mut self.context,
                );
            }
        }
    }
}
impl Backup<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, String> {
        let mut n = 0;
        if unsafe {
            BackupRead(
                self.file.as_raw_handle(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                &mut n,
                0,
                0,
                &mut self.context,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(n as usize)
    }
    fn exact(&mut self, mut bytes: &mut [u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            let n = self.read(bytes)?;
            if n == 0 {
                return Err("文件数据提前结束".into());
            }
            bytes = &mut bytes[n..];
        }
        Ok(())
    }
    fn write(&mut self, mut bytes: &[u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            let mut n = 0;
            if unsafe {
                BackupWrite(
                    self.file.as_raw_handle(),
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut n,
                    0,
                    0,
                    &mut self.context,
                )
            } == 0
                || n == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
            bytes = &bytes[n as usize..];
        }
        Ok(())
    }
}
// Copy and verify main data and the supported Windows zone stream. Other backup
// records fail before publication: never silently discard EFS/reparse metadata.
fn streams(source: &File, target: Option<&File>, ctl: &JobControl) -> Result<[u8; 32], String> {
    let mut input = Backup {
        file: source,
        context: std::ptr::null_mut(),
        writing: false,
    };
    let mut output = target.map(|file| Backup {
        file,
        context: std::ptr::null_mut(),
        writing: true,
    });
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    for _ in 0..4096 {
        ctl.check()?;
        let mut header = [0u8; 20];
        let n = input.read(&mut header)?;
        if n == 0 {
            return Ok(hash.finalize().into());
        }
        input.exact(&mut header[n..])?;
        let id = u32::from_le_bytes(header[..4].try_into().unwrap());
        let mut size = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let namesize = u32::from_le_bytes(header[16..].try_into().unwrap()) as usize;
        if !matches!(id, 1 | 4) || namesize > 65536 || namesize % 2 != 0 {
            return Err("此文件含暂不支持的特殊数据，源文件保持不变".into());
        }
        let mut name = vec![0; namesize];
        input.exact(&mut name)?;
        if id == 4 {
            let units = name
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            let value = String::from_utf16(&units).map_err(|_| "无效的数据流名称")?;
            if !value.eq_ignore_ascii_case(":Zone.Identifier:$DATA") {
                return Err("暂不支持复制自定义备用数据流，源文件保持不变".into());
            }
        }
        hash.update(header);
        hash.update(&name);
        if let Some(out) = output.as_mut() {
            out.write(&header)?;
            out.write(&name)?;
        }
        while size != 0 {
            ctl.check()?;
            let amount = size.min(buffer.len() as u64) as usize;
            input.exact(&mut buffer[..amount])?;
            hash.update(&buffer[..amount]);
            if let Some(out) = output.as_mut() {
                out.write(&buffer[..amount])?;
            }
            size -= amount as u64;
            #[cfg(test)]
            AFTER_CHUNK.with(|hook| {
                if let Some(run) = hook.borrow_mut().take() {
                    run();
                }
            });
        }
    }
    Err("文件数据流过多".into())
}

fn check_directory_streams(file: &File) -> Result<(), String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.file_attributes() & (0x400 | 0x4000 | 0x200) != 0 {
        return Err("文件夹含不支持的链接、加密或稀疏属性，源文件夹保持不变".into());
    }
    let mut backup = Backup {
        file,
        context: std::ptr::null_mut(),
        writing: false,
    };
    let mut header = [0u8; 20];
    if backup.read(&mut header)? != 0 {
        return Err(
            "暂不支持带备用数据流或特殊元数据的文件夹（包括Zone.Identifier），源文件夹保持不变"
                .into(),
        );
    }
    Ok(())
}

struct Source {
    path: PathBuf,
    relative: PathBuf,
    handle: Option<File>,
    directory: bool,
    skipped: bool,
}
fn collect(
    path: &Path,
    relative: PathBuf,
    moving: bool,
    nodes: &mut Vec<Source>,
    ctl: &JobControl,
) -> Result<(), String> {
    ctl.check()?;
    if relative.components().count() > 256 {
        return Err("文件夹层级过深".into());
    }
    if nodes.len() >= 100000 {
        return Err("一次操作最多处理100000个项目".into());
    }
    let handle = File::options()
        .access_mode(0x80000000 | if moving { 0x10000 } else { 0 })
        .share_mode(1)
        .custom_flags(0x02200000)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let meta = crate::private_stage::checked_metadata(&handle)?;
    if meta.file_attributes() & 0x400 != 0 || (!meta.is_file() && !meta.is_dir()) {
        return Err("不能复制或移动链接及特殊文件".into());
    }
    let directory = meta.is_dir();
    if directory {
        check_directory_streams(&handle)?;
    }
    nodes.push(Source {
        path: path.to_owned(),
        relative: relative.clone(),
        handle: Some(handle),
        directory,
        skipped: false,
    });
    if directory {
        for child in fs::read_dir(path).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            collect(
                &child.path(),
                relative.join(child.file_name()),
                moving,
                nodes,
                ctl,
            )?;
        }
    }
    Ok(())
}
#[derive(Debug, Serialize)]
pub struct Transfer {
    pub copied: usize,
    pub moved: usize,
    pub skipped: usize,
    pub retained_sources: Vec<String>,
}
pub fn transfer(
    inputs: &[String],
    destination: &str,
    moving: bool,
    opts: &EngineOptions,
) -> Result<Transfer, String> {
    let ctl = control(opts);
    ctl.check()?;
    if inputs.is_empty() {
        return Err("请先选择文件或文件夹".into());
    }
    if !["error", "skip", "rename", "overwrite"].contains(&opts.overwrite.as_str()) {
        return Err("无效的冲突处理方式".into());
    }
    let dest = absolute(destination)?;
    let mut guards = CheckedDirectories::default();
    guards.check(&dest)?;
    if !dest.is_dir() {
        return Err("目标文件夹不存在".into());
    }
    let paths = inputs
        .iter()
        .map(|p| absolute(p))
        .collect::<Result<Vec<_>, _>>()?;
    let mut keys = HashSet::new();
    for (i, path) in paths.iter().enumerate() {
        if path.file_name().is_none() || !keys.insert(path_key(path)) {
            return Err("不能选择根目录或重复的源".into());
        }
        if paths
            .iter()
            .enumerate()
            .any(|(j, other)| i != j && path_key(path).starts_with(&(path_key(other) + "/")))
        {
            return Err("所选文件夹与其子项目不能重复操作".into());
        }
        if path_key(&dest) == path_key(path) || path_key(&dest).starts_with(&(path_key(path) + "/"))
        {
            return Err("目标不能位于源文件夹内".into());
        }
    }
    let mut sources = CheckedDirectories::default();
    let mut nodes = Vec::new();
    for path in &paths {
        sources.check(path.parent().ok_or("缺少源父目录")?)?;
        collect(
            path,
            PathBuf::from(path.file_name().unwrap()),
            moving,
            &mut nodes,
            &ctl,
        )?;
    }
    let mut result = Transfer {
        copied: 0,
        moved: 0,
        skipped: 0,
        retained_sources: Vec::new(),
    };
    if moving {
        let parent = File::options()
            .access_mode(0x80000000)
            .share_mode(3)
            .custom_flags(0x02200000)
            .open(&dest)
            .map_err(|e| e.to_string())?;
        let atomic_result = (|| -> Result<(), String> {
            for path in &paths {
                ctl.check()?;
                let Some(index) = nodes.iter().position(|n| n.path == *path) else {
                    continue;
                };
                let original_target = dest.join(path.file_name().unwrap());
                // Merges and replacements need a separate transaction. Retain source.
                if original_target.exists()
                    && (opts.overwrite == "overwrite"
                        || (nodes[index].directory && opts.overwrite == "error"))
                {
                    continue;
                }
                let Some(target) =
                    choose_target_checked(original_target, &opts.overwrite, &mut |p| {
                        guards.check(p)
                    })?
                else {
                    result.skipped += nodes
                        .iter()
                        .filter(|n| n.path.starts_with(path) && !n.directory)
                        .count();
                    nodes.retain(|n| !n.path.starts_with(path));
                    continue;
                };
                nodes[index].relative =
                    PathBuf::from(target.file_name().ok_or("missing target name")?);
                // Descendant handles deny delete sharing and prevent a directory
                // rename. The root remains pinned: rename transfers the same whole
                // object, including concurrent additions, without traversing them.
                let directory = nodes[index].directory;
                if directory {
                    for node in &mut nodes {
                        if node.path != *path && node.path.starts_with(path) {
                            node.handle.take();
                        }
                    }
                }
                #[cfg(test)]
                BEFORE_ATOMIC.with(|hook| {
                    if let Some(run) = hook.borrow_mut().take() {
                        run();
                    }
                });
                ctl.check()?;
                // Move the original object with all streams; never replace a competitor.
                if rename_handle_noclobber_in(
                    nodes[index].handle.as_ref().unwrap(),
                    &target,
                    Some(&parent),
                )? {
                    result.moved += nodes.iter().filter(|n| n.path.starts_with(path)).count();
                    nodes.retain(|n| !n.path.starts_with(path));
                    #[cfg(test)]
                    AFTER_ATOMIC.with(|hook| {
                        if let Some(run) = hook.borrow_mut().take() {
                            run();
                        }
                    });
                } else if directory {
                    // Cross-volume or unsupported rename: re-read descendants while
                    // retaining the original root handle, then copy without deletion.
                    nodes.retain(|n| n.path == *path || !n.path.starts_with(path));
                    let root = nodes.iter().find(|n| n.path == *path).unwrap();
                    check_directory_streams(root.handle.as_ref().unwrap())?;
                    let relative = root.relative.clone();
                    for child in fs::read_dir(path).map_err(|e| e.to_string())? {
                        let child = child.map_err(|e| e.to_string())?;
                        collect(
                            &child.path(),
                            relative.join(child.file_name()),
                            true,
                            &mut nodes,
                            &ctl,
                        )?;
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = atomic_result {
            return Err(format!(
                "{error}；已原子移动{}项，未完成项目仍保留",
                result.moved
            ));
        }
    }
    let mut published = Vec::new();
    #[cfg(test)]
    BEFORE_COPY.with(|hook| {
        if let Some(run) = hook.borrow_mut().take() {
            run();
        }
    });
    let operation = (|| -> Result<(), String> {
        for node in &mut nodes {
            ctl.check()?;
            let mut target = dest.join(&node.relative);
            if path_key(&target) == path_key(&node.path) {
                return Err("源与目标是同一位置".into());
            }
            if node.directory {
                guards.create_dirs(&target)?;
                continue;
            }
            let Some(chosen) =
                choose_target_checked(target, &opts.overwrite, &mut |p| guards.check(p))?
            else {
                node.skipped = true;
                result.skipped += 1;
                continue;
            };
            target = chosen;
            let parent = target.parent().ok_or("缺少目标父目录")?;
            guards.create_dirs(parent)?;
            guards.allow_publication(parent)?;
            ctl.set("copying", None, "正在复制并验证文件");
            let input = node.handle.as_ref().unwrap();
            let (output, expected) = with_output_temp(parent, &guards, |temp| {
                let expected = streams(input, Some(temp.as_file()), &ctl)?;
                temp.as_file().sync_all().map_err(|e| e.to_string())?;
                if streams(temp.as_file(), None, &ctl)? != expected {
                    return Err("目标文件验证失败，源文件保持不变".into());
                }
                let info = input.metadata().map_err(|e| e.to_string())?;
                let mut times = fs::FileTimes::new();
                if let Ok(t) = info.modified() {
                    times = times.set_modified(t);
                }
                if let Ok(t) = info.accessed() {
                    times = times.set_accessed(t);
                }
                temp.as_file().set_times(times).map_err(|e| e.to_string())?;
                ctl.check()?;
                publish_output(temp, &target, opts.overwrite == "overwrite")
                    .map(|file| (file, expected))
            })?;
            drop(output);
            let published_pin = crate::engine_stream::pin(&target)?;
            if streams(&published_pin, None, &ctl)? != expected {
                return Err("已发布目标内容改变，源文件保持不变".into());
            }
            published.push(published_pin);
            result.copied += 1;
        }
        ctl.check()?;
        Ok(())
    })();
    if let Err(error) = operation {
        return Err(format!(
            "{error}；已复制{}个文件，已原子移动{}项；复制项目的源仍保留",
            result.copied, result.moved
        ));
    }
    if moving {
        #[cfg(test)]
        BEFORE_MOVE.with(|hook| {
            if let Some(run) = hook.borrow_mut().take() {
                run();
            }
        });
        ctl.check().map_err(|error| {
            format!(
                "{error}；已复制{}个文件，已原子移动{}项；复制项目的源仍保留",
                result.copied, result.moved
            )
        })?;
        // New ADS can bypass a main-stream pin. Never dispose copied sources.
        result.retained_sources.extend(
            paths
                .iter()
                .filter(|p| nodes.iter().any(|n| n.path == **p))
                .map(|p| p.display().to_string()),
        );
    }
    Ok(result)
}

#[cfg(test)]
thread_local! {
 static AFTER_CHUNK:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
 static BEFORE_COPY:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
 static AFTER_ATOMIC:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
 static BEFORE_ATOMIC:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
 static BEFORE_MOVE:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("source");
        let dest = t.path().join("dest");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&dest).unwrap();
        fs::create_dir(source.join("empty")).unwrap();
        fs::write(source.join("a.txt"), b"known local bytes").unwrap();
        (t, source, dest)
    }
    #[test]
    fn local_copy_and_move_preserve_nested_files_and_alternate_streams() {
        for moving in [false, true] {
            let (_t, source, dest) = setup();
            let alternate = format!("{}:Zone.Identifier", source.join("a.txt").display());
            fs::write(&alternate, b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
            let result = transfer(
                &[source.display().to_string()],
                dest.to_str().unwrap(),
                moving,
                &EngineOptions::default(),
            )
            .unwrap();
            assert_eq!(result.copied, usize::from(!moving));
            assert!(result.retained_sources.is_empty());
            assert_eq!(
                fs::read(dest.join("source/a.txt")).unwrap(),
                b"known local bytes"
            );
            assert!(dest.join("source/empty").is_dir());
            assert_eq!(
                fs::read(format!(
                    "{}:Zone.Identifier",
                    dest.join("source/a.txt").display()
                ))
                .unwrap(),
                b"[ZoneTransfer]\r\nZoneId=3\r\n"
            );
            assert_eq!(source.exists(), !moving);
        }
    }
    #[test]
    fn local_cancel_keeps_source_and_rejects_links() {
        let (_t, source, dest) = setup();
        let ctl = JobControl::new();
        ctl.cancel();
        let opts = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        assert!(transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            true,
            &opts
        )
        .is_err());
        assert!(source.join("a.txt").exists());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
        fs::hard_link(source.join("a.txt"), source.join("hard.txt")).unwrap();
        assert!(transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            true,
            &EngineOptions::default()
        )
        .is_err());
        assert!(source.join("a.txt").exists());
    }
    #[test]
    fn local_conflicts_preserve_existing_bytes_and_skipped_sources() {
        for mode in ["error", "skip", "rename", "overwrite"] {
            let (_t, source, dest) = setup();
            let file = source.join("a.txt");
            fs::write(dest.join("a.txt"), b"old").unwrap();
            let opts = EngineOptions {
                overwrite: mode.into(),
                ..Default::default()
            };
            let result = transfer(
                &[file.display().to_string()],
                dest.to_str().unwrap(),
                true,
                &opts,
            );
            if mode == "error" {
                assert!(result.is_err());
                assert!(file.exists());
                assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"old");
                continue;
            }
            let result = result.unwrap();
            if mode == "skip" {
                assert_eq!(result.skipped, 1);
                assert!(file.exists());
                assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"old");
            } else {
                assert_eq!(file.exists(), mode == "overwrite");
                assert_eq!(
                    result.retained_sources.len(),
                    usize::from(mode == "overwrite")
                );
                let target = dest.join(if mode == "rename" {
                    "a (1).txt"
                } else {
                    "a.txt"
                });
                assert_eq!(fs::read(target).unwrap(), b"known local bytes");
                if mode == "rename" {
                    assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"old");
                }
            }
        }
    }
    #[test]
    fn atomic_move_cancel_before_rename_keeps_source() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        let ctl = JobControl::new();
        let cancel = ctl.clone();
        BEFORE_ATOMIC.with(|h| *h.borrow_mut() = Some(Box::new(move || cancel.cancel())));
        let options = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        assert!(transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            true,
            &options
        )
        .unwrap_err()
        .contains("cancelled"));
        assert!(file.exists());
        assert!(!dest.join("a.txt").exists());
    }
    #[test]
    fn atomic_move_carries_ads_added_after_validation_with_share_delete() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        let late = file.clone();
        BEFORE_ATOMIC.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                File::options()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .share_mode(7)
                    .open(format!("{}:late", late.display()))
                    .unwrap()
                    .write_all(b"late survives")
                    .unwrap();
            }))
        });
        let result = transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            true,
            &EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(result.moved, 1);
        assert_eq!(result.copied, 0);
        assert!(!file.exists());
        assert_eq!(
            fs::read(format!("{}:late", dest.join("a.txt").display())).unwrap(),
            b"late survives"
        );
    }
    #[test]
    fn atomic_move_never_replaces_a_racing_target() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        let late = dest.join("a.txt");
        BEFORE_ATOMIC.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || fs::write(late, b"competitor").unwrap()))
        });
        assert!(transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            true,
            &EngineOptions::default()
        )
        .is_err());
        assert!(file.exists());
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"competitor");
    }
    #[test]
    fn custom_ads_is_rejected_without_removing_source_or_publishing_file() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        fs::write(format!("{}:custom", file.display()), b"must survive").unwrap();
        assert!(transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default()
        )
        .is_err());
        assert_eq!(
            fs::read(format!("{}:custom", file.display())).unwrap(),
            b"must survive"
        );
        assert!(!dest.join("a.txt").exists());
    }
    #[test]
    fn cancellation_after_written_chunk_discards_private_output_and_keeps_source() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        let data = vec![7u8; 2 * 1024 * 1024];
        fs::write(&file, &data).unwrap();
        let ctl = JobControl::new();
        let cancel = ctl.clone();
        AFTER_CHUNK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || cancel.cancel())));
        let options = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        assert!(transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &options
        )
        .unwrap_err()
        .contains("cancelled"));
        assert_eq!(fs::read(&file).unwrap(), data);
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn directories_with_any_ads_never_lose_empty_or_nonempty_sources() {
        for empty in [false, true] {
            for stream in ["custom", "Zone.Identifier"] {
                let temp = tempfile::tempdir().unwrap();
                let source = temp.path().join("source");
                let target = temp.path().join("target");
                fs::create_dir(&source).unwrap();
                fs::create_dir(&target).unwrap();
                if !empty {
                    fs::write(source.join("child.txt"), b"child survives").unwrap();
                }
                let ads = format!("{}:{stream}", source.display());
                fs::write(&ads, b"directory stream survives").unwrap();
                for moving in [false, true] {
                    let failure = transfer(
                        &[source.display().to_string()],
                        target.to_str().unwrap(),
                        moving,
                        &EngineOptions::default(),
                    )
                    .unwrap_err();
                    assert!(failure.contains("备用数据流"), "{failure}");
                    assert!(source.is_dir());
                    assert_eq!(fs::read(&ads).unwrap(), b"directory stream survives");
                    if !empty {
                        assert_eq!(
                            fs::read(source.join("child.txt")).unwrap(),
                            b"child survives"
                        );
                    }
                    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
                }
            }
        }
    }
    #[test]
    fn directory_ads_added_after_copy_keeps_original_directory_and_data() {
        let (_t, source, dest) = setup();
        fs::create_dir(dest.join("source")).unwrap();
        let path = source.clone();
        BEFORE_MOVE.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(format!("{}:late", path.display()), b"late directory stream").unwrap();
            }))
        });
        let result = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            true,
            &EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(result.retained_sources.len(), 1);
        assert_eq!(result.moved, 0);
        assert!(source.join("a.txt").exists());
        assert_eq!(
            fs::read(format!("{}:late", source.display())).unwrap(),
            b"late directory stream"
        );
    }
    #[test]
    fn atomic_directory_move_carries_late_directory_and_child_streams() {
        for empty in [false, true] {
            let (_t, source, dest) = setup();
            if empty {
                fs::remove_file(source.join("a.txt")).unwrap();
            }
            let late = source.clone();
            BEFORE_ATOMIC.with(|h| {
                *h.borrow_mut() = Some(Box::new(move || {
                    File::options()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .share_mode(7)
                        .open(format!("{}:late", late.display()))
                        .unwrap()
                        .write_all(b"directory data")
                        .unwrap();
                    if !empty {
                        fs::write(
                            format!("{}:late", late.join("a.txt").display()),
                            b"child data",
                        )
                        .unwrap();
                    }
                }))
            });
            let result = transfer(
                &[source.display().to_string()],
                dest.to_str().unwrap(),
                true,
                &EngineOptions::default(),
            )
            .unwrap();
            assert!(result.retained_sources.is_empty());
            assert!(!source.exists());
            assert_eq!(
                fs::read(format!("{}:late", dest.join("source").display())).unwrap(),
                b"directory data"
            );
            if !empty {
                assert_eq!(
                    fs::read(format!("{}:late", dest.join("source/a.txt").display())).unwrap(),
                    b"child data"
                );
            }
        }
    }
    #[test]
    fn partial_atomic_move_reports_committed_items_on_conflict_or_cancel() {
        for cancel_after_first in [false, true] {
            let (_t, source, dest) = setup();
            let a = source.join("a.txt");
            let b = source.join("b.txt");
            fs::write(&b, b"second").unwrap();
            let ctl = JobControl::new();
            if cancel_after_first {
                let cancel = ctl.clone();
                AFTER_ATOMIC.with(|h| *h.borrow_mut() = Some(Box::new(move || cancel.cancel())));
            } else {
                fs::write(dest.join("b.txt"), b"existing").unwrap();
            }
            let opts = EngineOptions {
                control: Some(ctl),
                ..Default::default()
            };
            let error = transfer(
                &[a.display().to_string(), b.display().to_string()],
                dest.to_str().unwrap(),
                true,
                &opts,
            )
            .unwrap_err();
            assert!(error.contains("已原子移动1项"), "{error}");
            assert!(!a.exists());
            assert!(dest.join("a.txt").exists());
            assert_eq!(fs::read(&b).unwrap(), b"second");
        }
    }
}
