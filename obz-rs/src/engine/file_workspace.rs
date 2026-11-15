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
// Copy and verify main data, named $DATA streams, and validated EA. Other backup
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
    let mut seen_ea = false;
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
        if !matches!(id, 1 | 2 | 4) || namesize > 65536 || namesize % 2 != 0 {
            return Err("此文件含暂不支持的特殊数据，源文件保持不变".into());
        }
        let mut name = vec![0; namesize];
        input.exact(&mut name)?;
        if id == 2 {
            if seen_ea || size > super::file_workspace_ea::MAX_EA_BYTES as u64 {
                return Err("扩展属性重复或过大，源保持不变".into());
            }
            seen_ea = true;
            let mut payload = vec![0; size as usize];
            input.exact(&mut payload)?;
            super::file_workspace_ea::validate(
                u32::from_le_bytes(header[4..8].try_into().unwrap()),
                namesize,
                &payload,
                || ctl.check(),
            )?;
            hash.update(header);
            hash.update(&name);
            hash.update(&payload);
            if let Some(out) = output.as_mut() {
                out.write(&header)?;
                out.write(&payload)?;
                #[cfg(test)]
                AFTER_CHUNK.with(|hook| {
                    if let Some(run) = hook.borrow_mut().take() { run(); }
                });
            }
            ctl.check()?;
            continue;
        }
        if id == 4 {
            let units = name
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect::<Vec<_>>();
            let value = String::from_utf16(&units).map_err(|_| "无效的数据流名称")?;
            let Some(inner) = value
                .strip_prefix(':')
                .and_then(|v| v.get(..v.len().checked_sub(6)?))
            else {
                return Err("无效的数据流名称".into());
            };
            if !value.to_ascii_uppercase().ends_with(":$DATA")
                || inner.is_empty()
                || inner.chars().any(|c| matches!(c, ':' | '/' | '\\' | '\0'))
            {
                return Err("无效的数据流名称".into());
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
            if output.is_some() {
                AFTER_CHUNK.with(|hook| {
                    if let Some(run) = hook.borrow_mut().take() {
                        run();
                    }
                });
            }
        }
    }
    Err("文件数据流过多".into())
}

/// Only this module can construct a proof, after source and private target have
/// matching complete main/named data streams and EA. Archive publication never uses it.
pub(crate) struct LocalStreamProof {
    digest: [u8; 32],
    control: JobControl,
}
impl LocalStreamProof {
    pub(crate) fn verify(&self, file: &File) -> Result<Vec<u8>, String> {
        let actual = streams(file, None, &self.control)?;
        if actual != self.digest {
            return Err("本地文件的数据流在发布前发生变化".into());
        }
        Ok(actual.to_vec())
    }
}
fn copy_basic_metadata(source: &File, target: &File) -> Result<(), String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileInformationByHandle(
            file: Handle,
            class: i32,
            data: *const c_void,
            size: u32,
        ) -> i32;
    }
    let info = source.metadata().map_err(|e| e.to_string())?;
    // Reparse, EFS, sparse and compressed storage need their own native pipeline.
    if info.file_attributes() & (0x400 | 0x4000 | 0x200 | 0x800) != 0 {
        return Err("此存储属性需要专用复制，源文件保持不变".into());
    }
    let attrs = info.file_attributes() & 0x3127;
    let basic = [
        info.creation_time(),
        info.last_access_time(),
        info.last_write_time(),
        0,
        u64::from(if attrs == 0 { 0x80 } else { attrs }),
    ];
    if unsafe { SetFileInformationByHandle(target.as_raw_handle(), 0, basic.as_ptr().cast(), 40) }
        == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn check_directory_streams(file: &File, ctl: &JobControl) -> Result<(), String> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.file_attributes() & (0x400 | 0x4000 | 0x200 | 0x800) != 0 {
        return Err("文件夹含不支持的链接、加密、压缩或稀疏属性，源文件夹保持不变".into());
    }
    streams(file, None, ctl).map(|_| ())
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
        check_directory_streams(&handle, ctl)?;
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
    pub copied_directories: usize,
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
        copied_directories: 0,
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
                    check_directory_streams(root.handle.as_ref().unwrap(), &ctl)?;
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
    let mut copied_directories: Vec<(File, usize)> = Vec::new();
    #[cfg(test)]
    BEFORE_COPY.with(|hook| {
        if let Some(run) = hook.borrow_mut().take() {
            run();
        }
    });
    let operation = (|| -> Result<(), String> {
        for (node_index, node) in nodes.iter_mut().enumerate() {
            ctl.check()?;
            let mut target = dest.join(&node.relative);
            if path_key(&target) == path_key(&node.path) {
                return Err("源与目标是同一位置".into());
            }
            if node.directory {
                let parent = target.parent().ok_or("缺少目标父目录")?;
                guards.create_dirs(parent)?;
                guards.allow_publication(parent)?;
                let source = node.handle.as_ref().unwrap();
                if target.exists() {
                    guards.check(&target)?;
                    if !target.is_dir() {
                        return Err("目标名称已被文件占用".into());
                    }
                    if streams(source, None, &ctl)? != <[u8; 32]>::from(Sha256::digest([])) {
                        return Err(
                            "目标文件夹已存在，未覆盖其备用数据流或扩展属性；请选择新目标文件夹".into()
                        );
                    }
                } else {
                    let held = &guards.held.get(parent).ok_or("缺少目标父目录锁")?.0;
                    let parent_pin = File::options()
                        .access_mode(0x200a0)
                        .share_mode(3)
                        .custom_flags(0x02200000)
                        .open(parent)
                        .map_err(|e| e.to_string())?;
                    if directory_identity(held)? != directory_identity(&parent_pin)? {
                        return Err("目标父目录已改变".into());
                    }
                    let mut temporary =
                        crate::private_stage::create_local_directory_output(parent, &parent_pin)?;
                    let attempt = (|| -> Result<File, String> {
                        let expected = streams(source, Some(temporary.as_file()), &ctl)?;
                        if streams(temporary.as_file(), None, &ctl)? != expected {
                            return Err("目标文件夹数据流验证失败，源保持不变".into());
                        }
                        copy_basic_metadata(source, temporary.as_file())?;
                        #[cfg(test)]
                        BEFORE_DIRECTORY_COMMIT.with(|hook| {
                            if let Some(run) = hook.borrow_mut().take() {
                                run();
                            }
                        });
                        ctl.check()?;
                        let proof = LocalStreamProof {
                            digest: expected,
                            control: ctl.clone(),
                        };
                        temporary.commit_local_streams(&target, false, &proof)
                    })();
                    let cleanup = temporary.abort();
                    let output = match (attempt, cleanup) {
                        (Ok(file), Ok(())) => file,
                        (Err(error), Ok(())) => return Err(error),
                        (Ok(_), Err(error)) => return Err(error),
                        (Err(error), Err(cleanup)) => return Err(format!("{error}; {cleanup}")),
                    };
                    result.copied_directories += 1;
                    #[cfg(test)]
                    AFTER_DIRECTORY_COMMIT.with(|hook| {
                        if let Some(run) = hook.borrow_mut().take() {
                            run();
                        }
                    });
                    guards.check(&target)?;
                    if directory_identity(&output)?
                        != directory_identity(
                            &guards.held.get(&target).ok_or("目标文件夹锁缺失")?.0,
                        )?
                    {
                        return Err("已发布文件夹被替换，未继续写入目标".into());
                    }
                    copied_directories.push((output, node_index));
                }
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
                copy_basic_metadata(input, temp.as_file())?;
                ctl.check()?;
                let proof = LocalStreamProof {
                    digest: expected,
                    control: ctl.clone(),
                };
                temp.inner
                    .commit_local_streams(&target, opts.overwrite == "overwrite", &proof)
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
        for (directory, index) in copied_directories.iter().rev() {
            ctl.check()?;
            copy_basic_metadata(nodes[*index].handle.as_ref().unwrap(), directory)?;
        }
        ctl.check()?;
        Ok(())
    })();
    if let Err(error) = operation {
        return Err(format!(
            "{error}；已复制{}个文件、{}个目录，已原子移动{}项；复制项目的源仍保留",
            result.copied, result.copied_directories, result.moved
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
                "{error}；已复制{}个文件、{}个目录，已原子移动{}项；复制项目的源仍保留",
                result.copied, result.copied_directories, result.moved
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
 static BEFORE_DIRECTORY_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
 static AFTER_DIRECTORY_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
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
    fn put_ea(path: &Path, flags: u8, value: &[u8]) -> Vec<u8> {
        let name = b"QING.TEST";
        let mut data = vec![0; 8];
        data[4] = flags;
        data[5] = name.len() as u8;
        data[6..8].copy_from_slice(&(value.len() as u16).to_le_bytes());
        data.extend_from_slice(name);
        data.push(0);
        data.extend_from_slice(value);
        assert_eq!(put_raw_ea(path, &data), 0);
        data
    }
    fn put_raw_ea(path: &Path, data: &[u8]) -> i32 {
        #[repr(C)]
        struct IoStatus { status: usize, information: usize }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtSetEaFile(file: Handle, status: *mut IoStatus, data: *const u8, size: u32) -> i32;
        }
        let file = File::options().access_mode(0x0008 | 0x0010).share_mode(7)
            .custom_flags(0x02200000).open(path).unwrap();
        let mut status = IoStatus { status: 0, information: 0 };
        unsafe { NtSetEaFile(file.as_raw_handle(), &mut status,
            data.as_ptr(), data.len() as u32) }
    }
    fn get_ea(path: &Path) -> Vec<u8> {
        let file = File::options().read(true).share_mode(7).custom_flags(0x02200000).open(path).unwrap();
        let mut reader = Backup { file: &file, context: std::ptr::null_mut(), writing: false };
        loop {
            let mut header = [0; 20];
            let n = reader.read(&mut header).unwrap();
            if n == 0 { return Vec::new(); }
            reader.exact(&mut header[n..]).unwrap();
            let id = u32::from_le_bytes(header[..4].try_into().unwrap());
            let size = u64::from_le_bytes(header[8..16].try_into().unwrap()) as usize;
            let name = u32::from_le_bytes(header[16..].try_into().unwrap()) as usize;
            assert!(size + name < 1024 * 1024);
            let mut payload = vec![0; size + name];
            reader.exact(&mut payload).unwrap();
            if id == 2 { return payload; }
        }
    }

    #[test]
    fn local_ea_file_directory_and_ads_roundtrip() {
        for flags in [0, 0x80] {
            let (_temp, source, dest) = setup();
            let directory_ea = put_ea(&source, flags, b"directory");
            let file_ea = put_ea(&source.join("a.txt"), flags, b"file");
            for path in [&source, &source.join("a.txt")] {
                fs::write(format!("{}:custom", path.display()), b"ads preserved").unwrap();
            }
            transfer(&[source.display().to_string()], dest.to_str().unwrap(), false,
                &EngineOptions::default()).unwrap();
            assert_eq!(get_ea(&dest.join("source")), directory_ea);
            assert_eq!(get_ea(&dest.join("source/a.txt")), file_ea);
            assert_eq!(get_ea(&source), directory_ea);
            for path in [dest.join("source"), dest.join("source/a.txt")] {
                assert_eq!(fs::read(format!("{}:custom", path.display())).unwrap(), b"ads preserved");
            }
        }
    }

    #[test]
    fn local_ea_native_multi_entry_order_large_values_and_high_bytes() {
        fn chain(items: &[(&[u8], u8, &[u8])]) -> Vec<u8> {
            let mut all = Vec::new();
            for (index, (name, flags, value)) in items.iter().enumerate() {
                let mut item = vec![0; 8];
                item[4] = *flags;
                item[5] = name.len() as u8;
                item[6..8].copy_from_slice(&(value.len() as u16).to_le_bytes());
                item.extend_from_slice(name);
                item.push(0);
                item.extend_from_slice(value);
                if index + 1 != items.len() {
                    item.resize((item.len() + 3) & !3, 0);
                    let len = item.len() as u32;
                    item[..4].copy_from_slice(&len.to_le_bytes());
                }
                all.extend(item);
            }
            all
        }
        let large = (0..30001).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        for reversed in [false, true] {
            for directory in [false, true] {
                let (_temp, source, dest) = setup();
                let input = if directory { source.clone() } else { source.join("a.txt") };
                let mut items = vec![(b"Z-LAST".as_slice(), 0, b"end".as_slice()),
                    (b"A-FIRST".as_slice(), 0x80, large.as_slice()),
                    (b"MIDDLE".as_slice(), 0, b"\0\xff\x01".as_slice())];
                if reversed { items.reverse(); }
                assert_eq!(put_raw_ea(&input, &chain(&items)), 0);
                // Compare the complete native snapshot, not our set-buffer: the
                // filesystem may canonicalize names/padding when accepting EAs.
                let expected = get_ea(&input);
                super::super::file_workspace_ea::validate(0, 0, &expected, || Ok(())).unwrap();
                fs::write(format!("{}:custom", input.display()), b"coexisting ADS").unwrap();
                transfer(&[input.display().to_string()], dest.to_str().unwrap(), false,
                    &EngineOptions::default()).unwrap();
                let output = dest.join(input.file_name().unwrap());
                assert_eq!(get_ea(&output), expected);
                assert_eq!(get_ea(&input), expected);
                assert_eq!(fs::read(format!("{}:custom", output.display())).unwrap(), b"coexisting ADS");
            }
        }
        let (_temp, source, dest) = setup();
        let input = source.join("a.txt");
        let high = chain(&[(b"USER.\xe9", 0, b"high-name")]);
        let status = put_raw_ea(&input, &high);
        if status == 0 {
            let expected = get_ea(&input);
            transfer(&[input.display().to_string()], dest.to_str().unwrap(), false,
                &EngineOptions::default()).unwrap();
            assert_eq!(get_ea(&dest.join("a.txt")), expected);
            eprintln!("native high-byte EA name accepted and copied byte-for-byte");
        } else {
            // Only documented invalid-name rejection is an allowed platform result.
            assert_eq!(status as u32, 0x80000013);
            eprintln!("native high-byte EA name rejected: {status:#x}");
        }
    }

    #[test]
    fn local_ea_file_conflict_policies() {
        for mode in ["overwrite", "skip", "rename", "error"] {
            let (_temp, source, dest) = setup();
            let input = source.join("a.txt");
            let original = put_ea(&input, 0x80, b"source");
            let target = dest.join("a.txt");
            fs::write(&target, b"old").unwrap();
            let old = put_ea(&target, 0, b"old");
            let result = transfer(&[input.display().to_string()], dest.to_str().unwrap(), false,
                &EngineOptions { overwrite: mode.into(), ..Default::default() });
            if mode == "error" { assert!(result.is_err()); } else { result.unwrap(); }
            assert_eq!(get_ea(&input), original);
            assert_eq!(get_ea(&target), if mode == "overwrite" { original.clone() } else { old });
            if mode == "rename" {
                let renamed = fs::read_dir(&dest).unwrap().map(|v| v.unwrap().path())
                    .find(|path| path != &target).unwrap();
                assert_eq!(get_ea(&renamed), original);
            }
        }
    }

    #[test]
    fn local_ea_cancel_before_publication_and_existing_directory_keep_data() {
        for directory in [false, true] {
            let (_temp, source, dest) = setup();
            let input = if directory { source.clone() } else { source.join("a.txt") };
            let original = put_ea(&input, 0x80, b"source");
            let ctl = JobControl::new();
            let cancel = ctl.clone();
            AFTER_CHUNK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || cancel.cancel())));
            assert!(transfer(&[input.display().to_string()], dest.to_str().unwrap(), false,
                &EngineOptions { control: Some(ctl), ..Default::default() }).is_err());
            assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
            assert_eq!(get_ea(&input), original);
        }
        let (_temp, source, dest) = setup();
        let original = put_ea(&source, 0, b"source directory");
        fs::create_dir(dest.join("source")).unwrap();
        fs::write(dest.join("source/keep"), b"keep").unwrap();
        assert!(transfer(&[source.display().to_string()], dest.to_str().unwrap(), false,
            &EngineOptions::default()).is_err());
        assert_eq!(get_ea(&source), original);
        assert!(get_ea(&dest.join("source")).is_empty());
        assert_eq!(fs::read(dest.join("source/keep")).unwrap(), b"keep");
    }
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
    fn custom_ads_and_basic_attributes_are_copied_without_removing_source() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        for name in ["custom", "中文数据", "empty"] {
            fs::write(
                format!("{}:{name}", file.display()),
                if name == "empty" {
                    b"".as_slice()
                } else {
                    b"must survive".as_slice()
                },
            )
            .unwrap();
        }
        let input = File::options().read(true).write(true).open(&file).unwrap();
        let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1600000000);
        input
            .set_times(fs::FileTimes::new().set_accessed(old).set_modified(old))
            .unwrap();
        drop(input);
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms).unwrap();
        let original = fs::metadata(&file).unwrap();
        let result = transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(result.copied, 1);
        assert!(file.exists());
        let target = dest.join("a.txt");
        for name in ["custom", "中文数据", "empty"] {
            assert_eq!(
                fs::read(format!("{}:{name}", file.display())).unwrap(),
                fs::read(format!("{}:{name}", target.display())).unwrap()
            );
        }
        let copied = fs::metadata(&target).unwrap();
        assert_eq!(copied.creation_time(), original.creation_time());
        assert_eq!(copied.last_write_time(), original.last_write_time());
        assert_eq!(copied.file_attributes(), original.file_attributes());
        for path in [&file, &target] {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_readonly(false);
            fs::set_permissions(path, permissions).unwrap();
        }
    }
    #[test]
    fn custom_ads_requires_local_proof_and_tampering_is_not_published() {
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        fs::write(format!("{}:custom", file.display()), b"known").unwrap();
        let input = crate::engine_stream::pin(&file).unwrap();
        let ctl = JobControl::new();
        let mut guards = CheckedDirectories::default();
        guards.check(&dest).unwrap();
        guards.allow_publication(&dest).unwrap();
        let target = dest.join("output.txt");
        let error = with_output_temp(&dest, &guards, |temp| {
            streams(&input, Some(temp.as_file()), &ctl)?;
            publish_output(temp, &target, false)
        })
        .unwrap_err();
        assert!(
            error.contains("unexpected private output stream"),
            "{error}"
        );
        assert!(!target.exists());
        let error = with_output_temp(&dest, &guards, |temp| {
            let expected = streams(&input, Some(temp.as_file()), &ctl)?;
            let proof = LocalStreamProof {
                digest: expected,
                control: ctl.clone(),
            };
            use std::io::{Seek, SeekFrom};
            temp.as_file_mut().seek(SeekFrom::Start(0)).unwrap();
            temp.as_file_mut().write_all(b"modified").unwrap();
            temp.inner.commit_local_streams(&target, false, &proof)
        })
        .unwrap_err();
        assert!(error.contains("数据流在发布前发生变化"), "{error}");
        assert!(!target.exists());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
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
    fn directory_ads_and_times_survive_copy_and_atomic_move() {
        for empty in [false, true] {
            for moving in [false, true] {
                for stream in ["custom", "Zone.Identifier"] {
                    let temp = tempfile::tempdir().unwrap();
                    let source = temp.path().join("source");
                    let target = temp.path().join("target");
                    fs::create_dir(&source).unwrap();
                    fs::create_dir(&target).unwrap();
                    if !empty {
                        fs::write(source.join("child.txt"), b"child survives").unwrap();
                    }
                    fs::write(
                        format!("{}:{stream}", source.display()),
                        b"directory stream survives",
                    )
                    .unwrap();
                    let before = fs::metadata(&source).unwrap();
                    let result = transfer(
                        &[source.display().to_string()],
                        target.to_str().unwrap(),
                        moving,
                        &EngineOptions::default(),
                    )
                    .unwrap();
                    assert!(result.retained_sources.is_empty());
                    assert_eq!(source.exists(), !moving);
                    let copied = target.join("source");
                    assert_eq!(
                        fs::read(format!("{}:{stream}", copied.display())).unwrap(),
                        b"directory stream survives"
                    );
                    assert_eq!(
                        fs::metadata(&copied).unwrap().last_write_time(),
                        before.last_write_time()
                    );
                    if !empty {
                        assert_eq!(
                            fs::read(copied.join("child.txt")).unwrap(),
                            b"child survives"
                        );
                    }
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
    #[test]
    fn custom_ads_conflicts_and_cancel_preserve_existing_sources() {
        for mode in ["error", "skip", "rename", "overwrite"] {
            let (_t, source, dest) = setup();
            let file = source.join("a.txt");
            let target = dest.join("a.txt");
            fs::write(format!("{}:custom", file.display()), b"new stream").unwrap();
            fs::write(&target, b"old file").unwrap();
            fs::write(format!("{}:custom", target.display()), b"old stream").unwrap();
            let opts = EngineOptions {
                overwrite: mode.into(),
                ..Default::default()
            };
            let result = transfer(
                &[file.display().to_string()],
                dest.to_str().unwrap(),
                false,
                &opts,
            );
            if mode == "error" {
                assert!(result.is_err());
            } else {
                assert!(result.is_ok());
            }
            assert!(file.exists());
            if mode == "overwrite" {
                assert_eq!(
                    fs::read(format!("{}:custom", target.display())).unwrap(),
                    b"new stream"
                );
            } else {
                assert_eq!(
                    fs::read(format!("{}:custom", target.display())).unwrap(),
                    b"old stream"
                );
            }
            if mode == "rename" {
                assert_eq!(
                    fs::read(format!("{}:custom", dest.join("a (1).txt").display())).unwrap(),
                    b"new stream"
                );
            }
        }
        let (_t, source, dest) = setup();
        let file = source.join("a.txt");
        fs::write(&file, b"").unwrap();
        let data = vec![9; 2 * 1024 * 1024];
        fs::write(format!("{}:custom", file.display()), &data).unwrap();
        let ctl = JobControl::new();
        let cancel = ctl.clone();
        AFTER_CHUNK.with(|h| *h.borrow_mut() = Some(Box::new(move || cancel.cancel())));
        let opts = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        assert!(transfer(
            &[file.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &opts
        )
        .unwrap_err()
        .contains("cancelled"));
        assert_eq!(
            fs::read(format!("{}:custom", file.display())).unwrap(),
            data
        );
        assert_eq!(fs::read_dir(dest).unwrap().count(), 0);
    }
    #[test]
    fn directory_stream_conflict_never_overwrites_existing_directory_streams() {
        let (_t, source, dest) = setup();
        fs::write(format!("{}:custom", source.display()), b"source stream").unwrap();
        let existing = dest.join("source");
        fs::create_dir(&existing).unwrap();
        fs::write(format!("{}:custom", existing.display()), b"existing stream").unwrap();
        let error = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap_err();
        assert!(error.contains("未覆盖其备用数据流"), "{error}");
        assert_eq!(
            fs::read(format!("{}:custom", existing.display())).unwrap(),
            b"existing stream"
        );
        assert!(!existing.join("a.txt").exists());
        assert_eq!(
            fs::read(format!("{}:custom", source.display())).unwrap(),
            b"source stream"
        );
    }
    #[test]
    fn cancelling_written_directory_ads_leaves_no_final_or_temporary_directory() {
        let (_t, source, dest) = setup();
        let data = vec![3; 2 * 1024 * 1024];
        fs::write(format!("{}:large", source.display()), &data).unwrap();
        let ctl = JobControl::new();
        let cancel = ctl.clone();
        AFTER_CHUNK.with(|h| *h.borrow_mut() = Some(Box::new(move || cancel.cancel())));
        let options = EngineOptions {
            control: Some(ctl),
            ..Default::default()
        };
        let error = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &options,
        )
        .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(error.contains("0个目录"), "{error}");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
        assert_eq!(
            fs::read(format!("{}:large", source.display())).unwrap(),
            data
        );
    }
    #[test]
    fn directory_publication_never_overwrites_a_late_competitor() {
        let (_t, source, dest) = setup();
        fs::write(format!("{}:custom", source.display()), b"source stream").unwrap();
        let competitor = dest.join("source");
        let late = competitor.clone();
        BEFORE_DIRECTORY_COMMIT.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                fs::create_dir(&late).unwrap();
                fs::write(late.join("keep.txt"), b"competitor").unwrap();
                fs::write(format!("{}:custom", late.display()), b"competitor stream").unwrap();
            }))
        });
        assert!(transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default()
        )
        .is_err());
        assert_eq!(
            fs::read(competitor.join("keep.txt")).unwrap(),
            b"competitor"
        );
        assert_eq!(
            fs::read(format!("{}:custom", competitor.display())).unwrap(),
            b"competitor stream"
        );
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
        assert!(source.join("a.txt").exists());
    }
    #[test]
    fn directory_replaced_after_commit_is_not_used_for_child_copies() {
        let (_t, source, dest) = setup();
        let target = dest.join("source");
        let displaced = dest.join("displaced");
        let late = target.clone();
        let moved = displaced.clone();
        AFTER_DIRECTORY_COMMIT.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                fs::rename(&late, &moved).unwrap();
                fs::create_dir(&late).unwrap();
                fs::write(late.join("keep.txt"), b"competitor").unwrap();
            }))
        });
        let error = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap_err();
        assert!(error.contains("已发布文件夹被替换"), "{error}");
        assert!(error.contains("1个目录"), "{error}");
        assert!(!target.join("a.txt").exists());
        assert!(!displaced.join("a.txt").exists());
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"competitor");
        assert!(source.join("a.txt").exists());
    }
    #[test]
    fn private_directory_cleanup_retains_unexpected_competitor_child() {
        let (_t, source, dest) = setup();
        let scan = dest.clone();
        BEFORE_DIRECTORY_COMMIT.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                let temp = fs::read_dir(scan).unwrap().next().unwrap().unwrap().path();
                fs::write(temp.join("keep.txt"), b"competitor").unwrap();
            }))
        });
        let error = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap_err();
        assert!(error.contains("unexpected children"), "{error}");
        assert!(!dest.join("source").exists());
        let retained = fs::read_dir(&dest).unwrap().next().unwrap().unwrap().path();
        assert_eq!(fs::read(retained.join("keep.txt")).unwrap(), b"competitor");
        assert!(source.join("a.txt").exists());
    }
    #[test]
    fn private_directory_publication_matches_native_directory_inheritance() {
        #[link(name = "advapi32")]
        extern "system" {
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text: *const u16,
                revision: u32,
                result: *mut *mut c_void,
                size: *mut u32,
            ) -> i32;
            fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd: *mut c_void,
                revision: u32,
                information: u32,
                text: *mut *mut u16,
                size: *mut u32,
            ) -> i32;
            fn SetKernelObjectSecurity(file: Handle, information: u32, sd: *mut c_void) -> i32;
            fn GetSecurityInfo(
                file: Handle,
                kind: u32,
                information: u32,
                owner: *mut Handle,
                group: *mut Handle,
                dacl: *mut Handle,
                sacl: *mut Handle,
                sd: *mut Handle,
            ) -> u32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn LocalFree(pointer: *mut c_void) -> *mut c_void;
        }
        fn descriptor(path: &Path) -> String {
            let file = File::options()
                .access_mode(0x20080)
                .share_mode(7)
                .custom_flags(0x02200000)
                .open(path)
                .unwrap();
            let mut sd = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    GetSecurityInfo(
                        file.as_raw_handle(),
                        1,
                        4,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &mut sd,
                    )
                },
                0
            );
            let mut text = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    ConvertSecurityDescriptorToStringSecurityDescriptorW(
                        sd,
                        1,
                        4,
                        &mut text,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            let mut length = 0;
            unsafe {
                while *text.add(length) != 0 {
                    length += 1;
                }
            }
            let result =
                String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) }).unwrap();
            unsafe {
                LocalFree(text.cast());
                LocalFree(sd);
            }
            result
        }
        for policy in [
            "AUTO",
            "D:PAI(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;CI;GR;;;BU)(A;OI;GW;;;BU)",
            "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICINP;GR;;;BU)",
            "D:P(A;;FA;;;OW)(A;OICIIO;FA;;;CO)(A;OICI;FA;;;SY)",
            "D:P(D;OICI;GW;;;BG)(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;GR;;;BU)",
        ] {
            let (_t, source, dest) = setup();
            let parent = File::options()
                .access_mode(0x60080)
                .share_mode(7)
                .custom_flags(0x02200000)
                .open(&dest)
                .unwrap();
            if policy == "AUTO" {
                crate::private_stage::ordinary_file_publication_test_parent(&parent);
            } else {
                let text: Vec<_> = policy.encode_utf16().chain(Some(0)).collect();
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
                    unsafe { SetKernelObjectSecurity(parent.as_raw_handle(), 0x80000004, sd) },
                    0
                );
                unsafe {
                    LocalFree(sd);
                }
            }
            drop(parent);
            fs::create_dir(dest.join("native")).unwrap();
            transfer(
                &[source.display().to_string()],
                dest.to_str().unwrap(),
                false,
                &EngineOptions::default(),
            )
            .unwrap();
            assert_eq!(
                descriptor(&dest.join("source")),
                descriptor(&dest.join("native")),
                "{policy}"
            );
            fs::create_dir(dest.join("native/empty")).unwrap();
            assert_eq!(
                descriptor(&dest.join("source/empty")),
                descriptor(&dest.join("native/empty")),
                "nested {policy}"
            );
        }
    }
    #[test]
    fn directory_parent_revocation_is_preserved_and_no_output_is_published() {
        #[link(name = "advapi32")]
        extern "system" {
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text: *const u16,
                revision: u32,
                result: *mut *mut c_void,
                size: *mut u32,
            ) -> i32;
            fn SetKernelObjectSecurity(file: Handle, information: u32, sd: *mut c_void) -> i32;
            fn GetFileSecurityW(
                path: *const u16,
                information: u32,
                sd: *mut u8,
                length: u32,
                needed: *mut u32,
            ) -> i32;
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn LocalFree(pointer: *mut c_void) -> *mut c_void;
        }
        fn set(path: &Path, policy: &str) {
            let parent = File::options()
                .access_mode(0x40080)
                .share_mode(7)
                .custom_flags(0x02200000)
                .open(path)
                .unwrap();
            let text: Vec<_> = policy.encode_utf16().chain(Some(0)).collect();
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
                unsafe { SetKernelObjectSecurity(parent.as_raw_handle(), 0x80000004, sd) },
                0
            );
            unsafe {
                LocalFree(sd);
            }
        }
        fn security(path: &Path) -> Vec<u8> {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut size = 0;
            unsafe {
                GetFileSecurityW(wide.as_ptr(), 4, std::ptr::null_mut(), 0, &mut size);
            }
            assert!(size > 0);
            let mut data = vec![0u8; size as usize];
            assert_ne!(
                unsafe {
                    GetFileSecurityW(wide.as_ptr(), 4, data.as_mut_ptr().cast(), size, &mut size)
                },
                0
            );
            data
        }
        let (_t, source, dest) = setup();
        set(&dest, "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;GR;;;BU)");
        let changed = dest.clone();
        let recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = recorded.clone();
        BEFORE_DIRECTORY_COMMIT.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                set(&changed, "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)");
                *record.lock().unwrap() = security(&changed);
            }))
        });
        let error = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap_err();
        assert!(error.contains("destination permissions changed"), "{error}");
        assert_eq!(security(&dest), *recorded.lock().unwrap());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
        assert!(source.join("a.txt").exists());
    }
    #[test]
    fn directory_parent_cannot_be_replaced_during_private_copy() {
        let (_t, source, dest) = setup();
        let parent = dest.clone();
        BEFORE_DIRECTORY_COMMIT.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                assert_eq!(
                    fs::rename(&parent, parent.with_extension("moved"))
                        .unwrap_err()
                        .raw_os_error(),
                    Some(32)
                );
            }))
        });
        let result = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(result.copied_directories, 2);
        assert_eq!(
            fs::read(dest.join("source/a.txt")).unwrap(),
            b"known local bytes"
        );
    }
}
