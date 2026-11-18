//! Local dual-location tasks. Sources remain pinned until verified copies are
//! published. Moves use atomic no-replace renames; copy fallbacks retain sources.
use super::*;
#[path = "file_workspace_create.rs"]
mod create;
pub use create::create_item;
#[path = "file_workspace_links.rs"]
mod links;
pub use links::{create_link, LinkKind, CreatedLink,inspect_link,edit_link,LinkReceipt,InspectedLink,EditedLink};
#[path = "file_workspace_stamp.rs"]
mod stamp;
#[path = "file_workspace_streams.rs"]
mod stream_inventory;
pub use stream_inventory::{list_streams, StreamInventory,export_streams,StreamExportItem,StreamExportReport};
pub use stamp::{directory_stamp,DirectoryStamp};
use sha2::{Digest, Sha256};
#[path = "file_workspace_preserved.rs"]
mod preserved;
pub(super) use preserved::{streams_preserving_access};
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
    pub revision: String,
}
#[derive(Serialize)]
pub struct Directory {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<LocalEntry>,
    pub revision: String,
}
// Ordinary Windows disk paths accept either separator. Normalize only that
// spelling so existing directory guards can pin the path; preserve case, dot
// components, trailing separators, and all UNC/verbatim namespace semantics.
fn local_path(text: &str) -> Result<PathBuf, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Prefix};
    let path = absolute(text)?;
    if matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))) {
        let wide = path.as_os_str().encode_wide().map(|c| if c == 47 { 92 } else { c }).collect::<Vec<_>>();
        return Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)));
    }
    Ok(path)
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
                    revision: String::new(),
                }
            })
            .collect();
        return Ok(Directory {
            path: String::new(),
            parent: None,
            entries,
            revision: String::new(),
        });
    }
    let root = local_path(path)?;
    let mut guards = CheckedDirectories::default();
    guards.check(&root)?;
    if !root.is_dir() {
        return Err("不是文件夹".into());
    }
    let revision=stamp::directory_revision(&guards.held.get(&root).ok_or("缺少目录锁")?.0)?;
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
            revision: stamp::entry_revision(&meta),
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
        revision,
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
// Archive Move stores the primary stream only. This is called exclusively under
// the transaction write lease, on its identity-checked committed reader.
// Any additional record retains the source; no source data is silently discarded.
pub(super) fn archive_primary_proof(source:&File,ctl:&JobControl)->Result<[u8;32],String>{
    use std::os::windows::fs::MetadataExt;
    let metadata=crate::private_stage::checked_metadata(source)?;
    if metadata.file_attributes()&0x4400!=0{return Err("加密或重解析来源保持不变".into());}
    let mut input=Backup{file:source,context:std::ptr::null_mut(),writing:false};
    let mut hash=Sha256::new();let mut seen=false;let mut buffer=vec![0;1024*1024];
    loop{
        ctl.check()?;let mut header=[0u8;20];let n=input.read(&mut header)?;
        if n==0{if !metadata.is_dir() && metadata.len()!=0 && !seen{return Err("主数据流缺失，保留来源".into());}return Ok(hash.finalize().into());}
        input.exact(&mut header[n..])?;
        let id=u32::from_le_bytes(header[..4].try_into().unwrap());let flags=u32::from_le_bytes(header[4..8].try_into().unwrap());
        let mut size=u64::from_le_bytes(header[8..16].try_into().unwrap());let name=u32::from_le_bytes(header[16..20].try_into().unwrap());
        if id!=1 || name!=0 || flags!=0 || seen || metadata.is_dir() || size!=metadata.len(){return Err("来源包含未归档的附加数据或特殊流，已复制并保留来源".into());}
        seen=true;
        while size!=0{ctl.check()?;let amount=usize::try_from(size.min(buffer.len() as u64)).unwrap();input.exact(&mut buffer[..amount])?;hash.update(&buffer[..amount]);size-=amount as u64;}
    }
}

// Copy and verify main data, named $DATA streams, and validated EA. Other backup
// records fail before publication: never silently discard EFS/reparse metadata.
trait BackupInput {
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, String>;
    fn exact(&mut self, mut bytes: &mut [u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            let n = self.read(bytes)?;
            if n == 0 { return Err("backup proof ended early".into()); }
            bytes = &mut bytes[n..];
        }
        Ok(())
    }
}
impl BackupInput for Backup<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, String> { Backup::read(self, bytes) }
    fn exact(&mut self, bytes: &mut [u8]) -> Result<(), String> { Backup::exact(self, bytes) }
}
pub(super) fn streams(source: &File, target: Option<&File>, ctl: &JobControl) -> Result<[u8; 32], String> {
    streams_observed(source,target,ctl,None)
}
pub(super) fn streams_observed(source: &File, target: Option<&File>, ctl: &JobControl, alternate_read:Option<&mut dyn FnMut()>) -> Result<[u8; 32], String> {
    let mut input = Backup { file: source, context: std::ptr::null_mut(), writing: false };
    streams_with_input(source, target, ctl, &mut input, alternate_read)
}
// Complete parser shared by native copying and the create-only protected reader.
// Native retains the original reader, with no additional enumeration or plan.
fn streams_with_input(source: &File, target: Option<&File>, ctl: &JobControl,
                      input: &mut dyn BackupInput, mut alternate_read:Option<&mut dyn FnMut()>) -> Result<[u8; 32], String> {
    use super::file_workspace_compression as compression;
    use super::file_workspace_sparse as sparse;
    ctl.check()?;
    let main_compression = compression::get(source)?;
    let main_sparse = sparse::query(source, ctl)?;
    if let Some(target) = target { compression::set(target, main_compression)?; }
    let mut output = target.map(|file| Backup {
        file,
        context: std::ptr::null_mut(),
        writing: true,
    });
    let mut hash = Sha256::new();
    // Uncompressed empty directories retain the existing empty-stream digest.
    if main_compression != 0 { hash.update(b"main-compression"); hash.update(main_compression.to_le_bytes()); }
    hash_sparse(&mut hash, main_sparse.as_ref());
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut seen_ea = false;
    let mut pending:Option<PendingStorage>=None;
    let mut main_record_seen=false;
    for _ in 0..4096 {
        ctl.check()?;
        let mut header = [0u8; 20];
        let n = input.read(&mut header)?;
        if n == 0 {
            finish_storage(target,pending.take(),ctl)?;
            if !main_record_seen {
                let meta=source.metadata().map_err(|e|e.to_string())?;
                if !meta.is_dir() && meta.len()!=0{return Err("非空主数据流缺少备份记录".into());}
                // Server 2022 may omit BACKUP_DATA for an empty sparse main
                // stream. Restore only the independently proven empty layout;
                // neither the raw-record hash nor any content check is relaxed.
                if let (Some(target),Some(layout))=(target,main_sparse.as_ref()){
                    if layout.eof!=0 || !layout.ranges.is_empty(){return Err("缺失主流的稀疏布局并非空流".into());}
                    sparse::restore(target,layout,ctl)?;
                }
            }
            return Ok(hash.finalize().into());
        }
        input.exact(&mut header[n..])?;
        let id = u32::from_le_bytes(header[..4].try_into().unwrap());
        let mut size = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let namesize = u32::from_le_bytes(header[16..].try_into().unwrap()) as usize;
        if id!=9 {finish_storage(target,pending.take(),ctl)?;}
        if !matches!(id, 1 | 2 | 4 | 9) || namesize > 65536 || namesize % 2 != 0 {
            return Err("此文件含暂不支持的特殊数据，源文件保持不变".into());
        }
        let mut name = vec![0; namesize];
        input.exact(&mut name)?;
        if matches!(id,1|9) && namesize!=0 {return Err("主数据流或稀疏区间名称无效".into());}
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
        let mut alternate = None;
        if id == 4 {
            // BackupRead owns the payload reader; a caller can report this
            // separately from suppression of its original primary handle.
            if let Some(observer)=alternate_read.as_mut(){observer();}
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
            let pin = compression::stream(source, &units, false)?;
            let state = compression::get(&pin)?;
            let layout = sparse::query(&pin, ctl)?;
            alternate = Some((units, pin, state, layout));
        }
        if let Some((_, _, state, layout)) = &alternate {hash.update(b"alternate-compression");hash.update(state.to_le_bytes());hash_sparse(&mut hash, layout.as_ref());}
        let attributes=u32::from_le_bytes(header[4..8].try_into().unwrap());
        if !matches!(attributes,0|8) {return Err("暂不支持此数据流属性".into());}
        if id==1 {
            main_record_seen=true;
            if (attributes==8)!=main_sparse.is_some() {return Err("主数据流稀疏属性不一致".into());}
            pending=Some(PendingStorage{name:None,pin:None,compression:main_compression,layout:main_sparse.clone(),end:0});
        } else if let Some((name,pin,compression,layout))=alternate {
            validate_alternate_sparse_header(attributes,size,layout.as_ref())?;
            pending=Some(PendingStorage{name:Some(name),pin:Some(pin),compression,layout,end:0});
        }
        hash.update(header);
        hash.update(&name);
        if let Some(out) = output.as_mut() {
            out.write(&header)?;
            out.write(&name)?;
        }
        if id==9 {
            let active=pending.as_mut().ok_or("稀疏数据块没有所属数据流")?;
            let layout=active.layout.as_ref().ok_or("普通数据流包含稀疏数据块")?;
            if attributes!=8 || size<8 {return Err("稀疏数据块无效".into());}
            let mut offset=[0;8];input.exact(&mut offset)?;
            let start=u64::from_le_bytes(offset);let count=size-8;
            let end=sparse::block_end(layout.eof,active.end,start,size,attributes)?;
            active.end=end;size=count;
            hash.update(offset);
            if let Some(out)=output.as_mut(){out.write(&offset)?;}
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

fn validate_alternate_sparse_header(attributes:u32,size:u64,layout:Option<&super::file_workspace_sparse::Layout>)->Result<(),String>{
    // Windows Server 2022 emits a normal, zero-length BackupRead record for an
    // empty ADS even when FSCTL_QUERY_ALLOCATED_RANGES proves it is sparse.
    // Accept only that metadata-only case. Nonempty and malformed records
    // still require the native sparse flag to agree with the independent query.
    let empty_native=attributes==0 && size==0 && layout.is_some_and(|l|l.eof==0 && l.ranges.is_empty());
    if !matches!(attributes,0|8) || ((attributes==8)!=layout.is_some() && !empty_native){
        return Err("备用数据流稀疏属性不一致".into());
    }
    Ok(())
}

struct PendingStorage {
    name:Option<Vec<u16>>,
    // Hold the source stream identity through its complete backup record group.
    pin:Option<File>,
    compression:u16,
    layout:Option<super::file_workspace_sparse::Layout>,
    end:u64,
}
fn finish_storage(target:Option<&File>,pending:Option<PendingStorage>,ctl:&JobControl)->Result<(),String> {
    ctl.check()?;
    if let (Some(target),Some(pending))=(target,pending) {
        let alternate=match &pending.name {Some(name)=>Some(super::file_workspace_compression::stream(target,name,true)?),None=>None};
        let stream=alternate.as_ref().unwrap_or(target);
        super::file_workspace_compression::set(stream,pending.compression)?;
        if let Some(layout)=&pending.layout {super::file_workspace_sparse::restore(stream,layout,ctl)?;}
        drop(pending.pin);
    }
    Ok(())
}

fn hash_sparse(hash:&mut Sha256,layout:Option<&super::file_workspace_sparse::Layout>) {
    if let Some(layout)=layout {
        hash.update(b"sparse-layout");hash.update(layout.eof.to_le_bytes());
        hash.update((layout.ranges.len() as u64).to_le_bytes());
        for &(offset,length) in &layout.ranges {hash.update(offset.to_le_bytes());hash.update(length.to_le_bytes());}
    }
}

/// Only this module can construct a proof, after source and private target have
/// matching complete main/named data streams and EA. Archive publication never uses it.
pub(crate) struct LocalStreamProof {
    digest: [u8; 32],
    control: JobControl,
}
impl crate::private_stage::LocalOutputProof for LocalStreamProof {
    fn verify(&self, file: &File) -> Result<Vec<u8>, String> {
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
    // Compression is independently copied and verified per stream above.
    if info.file_attributes() & (0x400 | 0x4000) != 0 {
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
    if metadata.file_attributes() & 0x400 != 0 {
        return Err("文件夹含不支持的链接属性，源文件夹保持不变".into());
    }
    if metadata.file_attributes()&0x4000!=0{return super::file_workspace_efs::validate_directory(file,ctl);}
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
// Internal private input-copy route: iterative traversal, no local-manager
// item/depth cap. Ordinary transfer/move continues using collect unchanged.
fn collect_archive(root:&Path,nodes:&mut Vec<Source>,capture:&mut Vec<super::create_input::Snapshot>,ctl:&JobControl)->Result<(),String>{
    let mut pending=vec![(root.to_owned(),PathBuf::from(root.file_name().ok_or("missing source name")?))];
    while let Some((path,relative))=pending.pop(){
        ctl.check()?;no_links(&path)?;
        let handle=File::options().access_mode(0x80000000).share_mode(1).custom_flags(0x02200000).open(&path).map_err(|e|e.to_string())?;
        let meta=crate::private_stage::checked_metadata(&handle)?;
        if meta.file_attributes()&0x400!=0||(!meta.is_file()&&!meta.is_dir()){return Err("linked or special create input rejected".into());}
        let directory=meta.is_dir();
        let mut snapshot=super::create_input::Snapshot::capture(&path,&relative,&handle,&meta,ctl)?;
        if directory {
            check_directory_streams(&handle,ctl)?;
            for child in fs::read_dir(&path).map_err(|e|e.to_string())?{
                ctl.check()?;let child=child.map_err(|e|e.to_string())?;let name=child.file_name();
                snapshot.children.push(name.clone());pending.push((child.path(),relative.join(name)));
            }
        }
        capture.push(snapshot);nodes.push(Source{path,relative,handle:Some(handle),directory,skipped:false});
    }
    Ok(())
}

struct PublishedCopy {
    index: usize,
    path: PathBuf,
    source_id: (u32, u64),
    target_id: (u32, u64),
    digest: [u8; 32],
    created_directory: bool,
    unmerged_target_digest: Option<[u8;32]>,
}
#[derive(Debug, Serialize)]
pub struct Transfer {
    pub copied: usize,
    pub copied_directories: usize,
    pub moved: usize,
    pub skipped: usize,
    pub retained_sources: Vec<String>,
    pub retained_reasons: Vec<String>,
    /// Ordinary children merged; these source directory EA/ADS were not merged.
    pub metadata_not_merged: Vec<String>,
}
pub fn transfer(
    inputs: &[String],
    destination: &str,
    moving: bool,
    opts: &EngineOptions,
) -> Result<Transfer, String> {
    transfer_inner(inputs,destination,moving,opts,None,None,None)
}
pub(super) fn copy_for_archive(inputs:&[String],destination:&str,opts:&EngineOptions,capture:&mut Vec<super::create_input::Snapshot>)->Result<Transfer,String>{
    transfer_inner(inputs,destination,false,opts,Some(capture),None,None)
}
/// Deep sessions retain this in memory across publication retries. A commit is
/// confirmed only by the existing post-publication verification below.
pub(super) type DeepPublicationLedger=crate::deep_publication_state::Ledger;
pub(super) struct DeepInputProtection {
    paths: HashSet<String>,
    complete: bool,
    identities: HashSet<(u32, u64)>,
    // Deny replacement through aliases for the entire publication pass.
    _pins: Vec<File>,
}
impl DeepInputProtection {
    pub(super) fn new(inputs: &HashSet<PathBuf>, complete: bool) -> Result<Self, String> {
        let mut protection = Self {
            paths: inputs.iter().map(|path| path_key(path)).collect(),
            complete,
            identities: HashSet::new(),
            _pins: Vec::new(),
        };
        for path in inputs {
            let pin = match File::options().read(true).share_mode(1)
                .custom_flags(0x02200000).open(path)
            {
                Ok(pin) => pin,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("无法保护原压缩包 {}：{error}", path.display())),
            };
            protection.identities.insert(directory_identity(&pin)?);
            protection._pins.push(pin);
        }
        Ok(protection)
    }

    fn replace_target(&self, target: &Path, overwrite: bool) -> Result<bool, String> {
        let conflict = || format!("目标与原压缩包或分卷冲突，恢复结果已保留：{}", target.display());
        if self.paths.contains(&path_key(target)) {
            return Err(conflict());
        }
        if !overwrite {
            return Ok(false);
        }
        let handle = match File::options().access_mode(0x80).share_mode(7)
            .custom_flags(0x02200000).open(target)
        {
            Ok(handle) => handle,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(self.complete),
            Err(error) => return Err(error.to_string()),
        };
        if self.identities.contains(&directory_identity(&handle)?) {
            return Err(conflict());
        }
        if !self.complete {
            return Err("无法确认全部原始分卷，已保留目标和恢复结果".into());
        }
        Ok(true)
    }
}

pub(super) fn copy_for_deep(
    inputs: &[String],
    destination: &str,
    opts: &EngineOptions,
    ledger: &mut DeepPublicationLedger,
    protection: &DeepInputProtection,
) -> Result<Transfer, String> {
    transfer_inner(inputs, destination, false, opts, None, Some(ledger), Some(protection))
}
fn transfer_inner(inputs:&[String],destination:&str,moving:bool,opts:&EngineOptions,mut capture:Option<&mut Vec<super::create_input::Snapshot>>,mut ledger:Option<&mut DeepPublicationLedger>,protection:Option<&DeepInputProtection>)->Result<Transfer,String>{
    let ctl = control(opts);
    ctl.check()?;
    if inputs.is_empty() {
        return Err("请先选择文件或文件夹".into());
    }
    if !["error", "skip", "rename", "overwrite"].contains(&opts.overwrite.as_str()) {
        return Err("无效的冲突处理方式".into());
    }
    let dest = local_path(destination)?;
    let mut guards = CheckedDirectories::default();
    guards.check(&dest)?;
    if !dest.is_dir() {
        return Err("目标文件夹不存在".into());
    }
    let paths = inputs
        .iter()
        .map(|p| local_path(p))
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
        if let Some(capture)=capture.as_deref_mut(){
            collect_archive(path,&mut nodes,capture,&ctl)?;
        }else{
        collect(
            path,
            PathBuf::from(path.file_name().unwrap()),
            moving,
            &mut nodes,
            &ctl,
        )?;
        }
    }
    if capture.is_some() {
        guards.archive_snapshot_pin_limit=Some(nodes.iter().filter(|node|node.directory).count()
            .checked_add(dest.ancestors().count()).and_then(|count|count.checked_add(1))
            .ok_or("creation directory pin budget overflow")?);
    }
    let mut result = Transfer {
        copied: 0,
        copied_directories: 0,
        moved: 0,
        skipped: 0,
        retained_sources: Vec::new(),
        retained_reasons: Vec::new(),
        metadata_not_merged: Vec::new(),
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
                #[cfg(test)] let force_copy = FORCE_COPY_MOVE.with(|value| value.get());
                #[cfg(not(test))] let force_copy = false;
                if !force_copy && rename_handle_noclobber_in(
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
    let mut copies: Vec<PublishedCopy> = Vec::new();
    let mut copied_directories: Vec<(File, usize)> = Vec::new();
    let mut encrypted_directories:Vec<(File,usize,[u8;32])>=Vec::new();
    #[cfg(test)]
    BEFORE_COPY.with(|hook| {
        if let Some(run) = hook.borrow_mut().take() {
            run();
        }
    });
    let mut encrypted_roots=Vec::new();
    for node in &nodes{
        if node.directory && node.handle.as_ref().unwrap().metadata().map_err(|e|e.to_string())?.file_attributes()&0x4000!=0{
            encrypted_roots.push(node.path.clone());
        }
    }
    let operation = (|| -> Result<(), String> {
        for (node_index, node) in nodes.iter_mut().enumerate() {
            ctl.check()?;
            let mut target = dest.join(&node.relative);
            if path_key(&target) == path_key(&node.path) {
                return Err("源与目标是同一位置".into());
            }
            if node.directory {
                let parent = target.parent().ok_or("缺少目标父目录")?;
                guards.create_dirs(parent).map_err(|e|format!("准备父目录：{e}"))?;
                guards.allow_publication(parent).map_err(|e|format!("固定发布目录：{e}"))?;
                if node.handle.as_ref().unwrap().metadata().map_err(|e|e.to_string())?.file_attributes()&0x4000!=0{
                    if target.exists(){
                        guards.check(&target)?;
                        if !target.is_dir(){return Err("目标名称已被文件占用".into());}
                        // Existing directory policy/EA/ADS belong to its owner.
                        // Merge children, never Raw-import over public contents.
                        result.metadata_not_merged.push(node.path.display().to_string());
                    }else{
                        let expected=directory_identity(node.handle.as_ref().unwrap())?;
                        node.handle.take();
                        let parent_pin=File::options().access_mode(0x200a0).share_mode(3)
                            .custom_flags(0x02200000).open(parent).map_err(|e|e.to_string())?;
                        if directory_identity(&parent_pin)?!=directory_identity(&guards.held.get(parent).ok_or("目标父目录锁缺失")?.0)?{
                            return Err("EFS目标父目录已改变".into());
                        }
                        let prepared=super::file_workspace_efs::prepare_directory(&node.path,Some([expected.0,(expected.1>>32) as u32,expected.1 as u32]),parent,&parent_pin,&ctl)?;
                        let digest=prepared.digest();
                        node.handle=Some(prepared.source().try_clone().map_err(|e|e.to_string())?);
                        let output=prepared.publish(&target,&ctl)?;
                        result.copied_directories+=1;
                        guards.check(&target)?;
                        if directory_identity(&output)?!=directory_identity(&guards.held.get(&target).ok_or("EFS目标目录锁缺失")?.0)?{
                            return Err("已发布EFS目录被替换".into());
                        }
                        encrypted_directories.push((output,node_index,digest));
                    }
                    if moving{result.retained_reasons.push(format!("{}：EFS目录来源保留，未执行跨卷删除",node.path.display()));}
                    continue;
                }
                let source = node.handle.as_ref().unwrap();
                let existing_directory = target.exists();
                let mut unmerged_target_digest = None;
                if existing_directory {
                    guards.check(&target)?;
                    if !target.is_dir() {
                        return Err("目标名称已被文件占用".into());
                    }
                    let mut empty = Sha256::new();
                    let compression = super::file_workspace_compression::get(source)?;
                    if compression != 0 { empty.update(b"main-compression"); empty.update(compression.to_le_bytes()); }
                    let source_digest = streams(source, None, &ctl)?;
                    if source_digest != <[u8; 32]>::from(empty.finalize()) {
                        // Match directory-content merging without writing the existing
                        // directory's EA/ADS. NTFS exposes no create-only EA update;
                        // query-then-write could overwrite a concurrent new EA.
                        let target_reader = File::options().read(true).share_mode(3)
                            .custom_flags(0x02200000).open(&target).map_err(|e|e.to_string())?;
                        let metadata = crate::private_stage::checked_metadata(&target_reader)?;
                        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0
                            || directory_identity(&target_reader)? != directory_identity(
                                &guards.held.get(&target).ok_or("目标目录锁缺失")?.0)? {
                            return Err("目标文件夹已改变，未继续合并".into());
                        }
                        let target_digest = streams(&target_reader, None, &ctl)?;
                        if target_digest != source_digest {
                            unmerged_target_digest = Some(target_digest);
                            result.metadata_not_merged.push(node.path.display().to_string());
                        }
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
                if !encrypted_roots.iter().any(|root|node.path.starts_with(root)){copies.push(PublishedCopy {
                    index: node_index,
                    path: target.clone(),
                    source_id: directory_identity(source)?,
                    target_id: directory_identity(&guards.held.get(&target).ok_or("目标目录锁缺失")?.0)?,
                    digest: streams(source, None, &ctl)?,
                    created_directory: !existing_directory,
                    unmerged_target_digest,
                });}
                continue;
            }
            if let Some(ledger)=ledger.as_ref(){if ledger.completed(&node.path,&mut |target,proof|{
                let target=crate::engine_stream::pin(target).map_err(|e|format!("已保存文件无法确认，恢复结果已保留：{e}"))?;
                if directory_identity(&target)?!=proof.identity{return Err("已保存文件已改变，恢复结果已保留，未自动重复复制".into());}
                if proof.encrypted{super::file_workspace_efs::verify_published(&target,proof.digest,&ctl)?;}
                else if streams(&target,None,&ctl)?!=proof.digest{return Err("已保存内容已改变，恢复结果已保留，未自动重复复制".into());}
                Ok(())
            })?{result.copied+=1;continue;}}
            let Some(chosen) =
                choose_target_checked(target, &opts.overwrite, &mut |p| guards.check(p))?
            else {
                node.skipped = true;
                result.skipped += 1;
                continue;
            };
            target = chosen;
            let overwrite = opts.overwrite == "overwrite";
            let replace = match protection {
                Some(protection) => protection.replace_target(&target, overwrite)?,
                None => overwrite,
            };
            let parent = target.parent().ok_or("缺少目标父目录")?;
            guards.create_dirs(parent).map_err(|e|format!("准备父目录：{e}"))?;
            guards.allow_publication(parent).map_err(|e|format!("固定发布目录：{e}"))?;
            ctl.set("copying", None, "正在复制并验证文件");
            if node.handle.as_ref().unwrap().metadata().map_err(|e|e.to_string())?.file_attributes() & 0x4000 != 0
                || guards.held.get(parent).ok_or("missing destination guard")?.0.metadata().map_err(|e|e.to_string())?.file_attributes()&0x4000 != 0 {
                let expected_source = directory_identity(node.handle.as_ref().unwrap())?;
                // A moving source was collected with DELETE. Native EFS source
                // pins deny DELETE sharing; release that old access, immediately
                // pin again and require the identical source before publication.
                node.handle.take();
                guards.check_prepared(parent)?;
                let held=&guards.held.get(parent).ok_or("missing EFS destination guard")?.0;
                let parent_pin=File::options().access_mode(0x200a0).share_mode(3)
                    .custom_flags(0x02200000).open(parent).map_err(|e|e.to_string())?;
                let (parent_id,parent_attrs)=directory_identity_and_attributes(&parent_pin)?;
                if directory_identity(held)?!=parent_id || parent_attrs&0x410!=0x10 {
                    return Err("EFS destination parent identity changed".into());
                }
                let prepared=super::file_workspace_efs::prepare_bound(&node.path,parent,&parent_pin,&ctl,Some(expected_source))?;
                if directory_identity(prepared.source())? != expected_source {
                    return Err("EFS source identity changed before copy".into());
                }
                node.handle=Some(prepared.source().try_clone().map_err(|e|e.to_string())?);
                let expected_digest=prepared.digest();
                let output=prepared.publish(&target,replace,&ctl)?;
                if let Some(ledger)=ledger.as_mut(){ledger.committed(&node.path,&target);}
                result.copied+=1;
                let id=directory_identity(&output)?;
                if let Some(ledger)=ledger.as_mut(){ledger.proof(&node.path,crate::deep_publication_state::Proof{identity:id,digest:expected_digest,encrypted:true});}
                drop(output);
                let pin=crate::engine_stream::pin(&target)?;
                if directory_identity(&pin)?!=id{return Err("published EFS target identity changed".into());}
                super::file_workspace_efs::verify_published(&pin,expected_digest,&ctl)?;
                published.push(pin);
                if let Some(ledger)=ledger.as_mut(){ledger.confirmed(&node.path);}
                if moving {result.retained_reasons.push(format!("{}：EFS副本已验证，跨卷来源保留",node.path.display()));}
                // EFS key/stream proofs are intentionally not sent to the
                // ordinary BackupRead cross-volume source deletion finalizer.
                continue;
            }
            let input = node.handle.as_ref().unwrap();
            let (output, expected) = with_output_temp(parent, &guards, |temp| {
                let expected = streams(input, Some(temp.as_file()), &ctl).map_err(|e|format!("复制文件数据流：{e}"))?;
                temp.as_file().sync_all().map_err(|e| e.to_string())?;
                if streams(temp.as_file(), None, &ctl)? != expected {
                    return Err("目标文件验证失败，源文件保持不变".into());
                }
                copy_basic_metadata(input, temp.as_file()).map_err(|e|format!("复制文件基本属性：{e}"))?;
                ctl.check()?;
                let proof = LocalStreamProof {
                    digest: expected,
                    control: ctl.clone(),
                };
                temp.inner
                    .commit_local_streams(&target, replace, &proof)
                    .map(|file| (file, expected)).map_err(|e|format!("发布文件：{e}"))
            }).map_err(|e|format!("准备或写入输出文件：{e}"))?;
            if let Some(ledger)=ledger.as_mut(){ledger.committed(&node.path,&target);}
            let published_identity = directory_identity(&output)?;
            if let Some(ledger)=ledger.as_mut(){ledger.proof(&node.path,crate::deep_publication_state::Proof{identity:published_identity,digest:expected,encrypted:false});}
            drop(output);
            let published_pin = crate::engine_stream::pin(&target).map_err(|e|format!("固定已发布文件：{e}"))?;
            if directory_identity(&published_pin)? != published_identity || streams(&published_pin, None, &ctl).map_err(|e|format!("复验已发布文件：{e}"))? != expected {
                return Err("已发布目标内容改变，源文件保持不变".into());
            }
            if !encrypted_roots.iter().any(|root|node.path.starts_with(root)){copies.push(PublishedCopy { index: node_index, path: target,
                source_id: directory_identity(input)?, target_id: published_identity,
                digest: expected, created_directory: false, unmerged_target_digest: None });}
            published.push(published_pin);
            if let Some(ledger)=ledger.as_mut(){ledger.confirmed(&node.path);}
            result.copied += 1;
        }
        for (directory,index,digest) in encrypted_directories.iter().rev(){
            ctl.check()?;
            let source=nodes[*index].handle.as_ref().unwrap();
            super::file_workspace_efs::restore_directory_metadata(source,directory)?;
            super::file_workspace_efs::verify_published(source,*digest,&ctl)?;
            super::file_workspace_efs::verify_published(directory,*digest,&ctl)?;
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
        // Remove our old DELETE-access pins before taking transaction leases.
        // Identity and complete stream proofs are rechecked after acquisition.
        for node in &mut nodes { node.handle.take(); }
        drop(copied_directories);
        drop(encrypted_directories);
        let completed = finish_transactional_move(&nodes, &copies, &ctl, &mut result.retained_reasons)
            .map_err(|error|format!("{error}；此前已原子移动{}项，目标副本已保留",result.moved))?;
        for copy in &copies {
            if completed.contains(&copy.index) {
                result.moved += 1;
                if nodes[copy.index].directory {
                    if copy.created_directory { result.copied_directories -= 1; }
                } else { result.copied -= 1; }
            }
        }
        result.retained_sources.extend(paths.iter()
            .filter(|p| nodes.iter().enumerate().any(|(i,n)| n.path.starts_with(p) && !completed.contains(&i)))
            .map(|p| p.display().to_string()));
    }
    Ok(result)
}

fn finish_transactional_move(
    nodes: &[Source], copies: &[PublishedCopy], ctl: &JobControl, reasons: &mut Vec<String>,
) -> Result<HashSet<usize>, String> {
    use super::file_workspace_transaction::Transaction;
    if copies.is_empty() { return Ok(HashSet::new()); }
    let transaction = match Transaction::begin() {
        Ok(value) => value,
        Err(error) => { reasons.push(error); return Ok(HashSet::new()); }
    };
    let prepare = (|| -> Result<HashSet<usize>, String> {
        let mut verified = Vec::new();
        for copy in copies {
            ctl.check()?;
            let node = &nodes[copy.index];
            let verify = (|| {
                let source = transaction.lock(&node.path, copy.source_id).map_err(|e|format!("固定来源：{e}"))?;
                let target = transaction.lock_target(&copy.path, copy.target_id).map_err(|e|format!("固定目标：{e}"))?;
                if streams(source.reader(), None, ctl).map_err(|e|format!("复验来源：{e}"))? != copy.digest {
                    return Err("来源的内容、数据流或存储属性已改变，保留来源".to_string());
                }
                let target_digest = streams(target.reader(), None, ctl).map_err(|e|format!("复验目标：{e}"))?;
                if target_digest != copy.digest {
                    if copy.unmerged_target_digest == Some(target_digest) {
                        return Err("目标文件夹保留了原有附加数据，来源文件夹的附加数据未合并，因此保留来源文件夹".to_string());
                    }
                    return Err("目标的内容、数据流或存储属性已改变，保留来源".to_string());
                }
                let source_meta = source.reader().metadata().map_err(|e| e.to_string())?;
                let target_meta = target.reader().metadata().map_err(|e| e.to_string())?;
                if source_meta.file_attributes() & 0x3127 != target_meta.file_attributes() & 0x3127
                    || source_meta.creation_time() != target_meta.creation_time()
                    || source_meta.last_write_time() != target_meta.last_write_time() {
                    return Err("来源与目标的文件属性或时间不同，保留来源".to_string());
                }
                Ok((copy.index, source, target))
            })();
            match verify {
                Ok(value) => verified.push(value),
                Err(error) => reasons.push(format!("{}：{error}", node.path.display())),
            }
        }
        // Descendants must be marked and their handles closed before a directory
        // is removed. Unknown/uncopied descendants are never recursively deleted.
        verified.sort_by_key(|(index,_,_)| std::cmp::Reverse(nodes[*index].path.components().count()));
        let mut completed = HashSet::new();
        let mut targets = Vec::new();
        for (index, source, target) in verified {
            ctl.check()?;
            let node = &nodes[index];
            if node.directory && nodes.iter().enumerate().any(|(child,n)|
                child != index && n.path.starts_with(&node.path) && !completed.contains(&child)) {
                reasons.push(format!("{}：部分子项目未完成移动，保留源文件夹",node.path.display()));
                continue;
            }
            #[cfg(test)] BEFORE_TRANSACTION_DELETE.with(|hook|if let Some(run)=hook.borrow_mut().take(){run()});
            ctl.check()?;
            source.delete()?;
            targets.push(target);
            completed.insert(index);
        }
        ctl.check()?;
        // Dropping readers does not release the transaction's write exclusion.
        // All leased file handles must be closed before CommitTransaction.
        drop(targets);
        Ok(completed)
    })();
    match prepare {
        Ok(completed) => {
            #[cfg(test)] BEFORE_TRANSACTION_COMMIT.with(|hook|if let Some(run)=hook.borrow_mut().take(){run()});
            if let Err(error) = ctl.check() {
                transaction.rollback().map_err(|rollback|format!("{error}；{rollback}"))?;
                return Err(format!("{error}；本次复制项目的来源未删除"));
            }
            transaction.commit()?;
            Ok(completed)
        }
        Err(error) => {
            transaction.rollback().map_err(|rollback|format!("{error}；{rollback}"))?;
            Err(format!("{error}；已回滚本次删除，复制的来源仍保留"))
        }
    }
}

#[cfg(test)]
thread_local! {
 static FORCE_COPY_MOVE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};
 static BEFORE_TRANSACTION_DELETE:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
 static BEFORE_TRANSACTION_COMMIT:std::cell::RefCell<Option<Box<dyn FnOnce()>>>=const {std::cell::RefCell::new(None)};
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
    struct CopyMoveGuard;
    #[test]
    fn local_disk_separator_normalization_preserves_dot_case_and_special_namespaces() {
        assert_eq!(local_path("c:/Mixed/./Folder/../file/").unwrap().as_os_str(),std::ffi::OsStr::new("c:\\Mixed\\.\\Folder\\..\\file\\"));
        for special in [r"\\server\share/folder/../file",r"\\?\C:\folder/file",r"\\?\UNC\server\share/folder"] {
            assert_eq!(local_path(special).unwrap().as_os_str(),std::ffi::OsStr::new(special));
        }
    }
    #[test]
    fn copied_move_and_copy_accept_forward_slash_source_and_destination() {
        for moving in [false,true] {
            let (_a,_b,source,dest,_force)=copy_move_setup();
            let input=source.join("a.txt");let source_text=input.display().to_string().replace('\\',"/");let target_text=dest.display().to_string().replace('\\',"/");
            let result=transfer(&[source_text],&target_text,moving,&EngineOptions::default()).unwrap();
            assert_eq!(result.moved,usize::from(moving));assert!(result.retained_sources.is_empty(),"{:?}",result.retained_reasons);
            assert_eq!(input.exists(),!moving);assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"known bytes");
            assert!(browse(&target_text).unwrap().entries.iter().any(|e|e.name=="a.txt"));
        }
    }
    impl Drop for CopyMoveGuard { fn drop(&mut self) { FORCE_COPY_MOVE.with(|v|v.set(false)); } }
    fn copy_move_setup() -> (tempfile::TempDir,tempfile::TempDir,PathBuf,PathBuf,CopyMoveGuard) {
        let first=tempfile::tempdir().unwrap();
        let second=if let Some(root)=std::env::var_os("QING_ZIP_SECOND_VOLUME") {
            tempfile::Builder::new().prefix("qing-zip-move-test-").tempdir_in(root).unwrap()
        }else{tempfile::tempdir().unwrap()};
        let source=first.path().join("source");let dest=second.path().join("dest");
        fs::create_dir(&source).unwrap();fs::create_dir(&dest).unwrap();
        fs::create_dir(source.join("empty")).unwrap();fs::write(source.join("a.txt"),b"known bytes").unwrap();
        if std::env::var_os("QING_ZIP_SECOND_VOLUME").is_some() {
            let a=File::options().read(true).custom_flags(0x02200000).open(&source).unwrap();
            let b=File::options().read(true).custom_flags(0x02200000).open(&dest).unwrap();
            let av=directory_identity(&a).unwrap().0;let bv=directory_identity(&b).unwrap().0;
            assert_ne!(av,bv,"explicit second-volume acceptance requires genuinely different volumes");
            eprintln!("cross-volume acceptance: source serial {av:#x}; target serial {bv:#x}");
        }else{FORCE_COPY_MOVE.with(|v|v.set(true));}
        (first,second,source,dest,CopyMoveGuard)
    }
    #[test]
    fn copied_move_directory_keeps_ea_ads_compression_sparse_and_removes_all_sources() {
        let (_a,_b,source,dest,_force)=copy_move_setup();
        let file=source.join("a.txt");let sparse_path=source.join("稀疏.bin");
        put_ea(&source,0,b"directory EA");put_ea(&file,0,b"file EA");
        fs::write(format!("{}:extra",source.display()),b"directory stream").unwrap();
        fs::write(format!("{}:extra",file.display()),b"file stream").unwrap();
        {let f=File::options().read(true).write(true).open(&file).unwrap();super::super::file_workspace_compression::set(&f,2).unwrap();}
        {let f=File::options().read(true).write(true).create_new(true).open(&sparse_path).unwrap();make_sparse(&f,1024*1024,true);}
        let expected=streams(&File::open(&file).unwrap(),None,&JobControl::new()).unwrap();
        let sparse_expected=streams(&File::open(&sparse_path).unwrap(),None,&JobControl::new()).unwrap();
        let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions::default()).unwrap();
        assert!(result.retained_sources.is_empty(),"{:?}",result.retained_reasons);
        assert!(!source.exists());assert_eq!(result.moved,4);assert_eq!(result.copied,0);assert_eq!(result.copied_directories,0);
        assert_eq!(streams(&File::open(dest.join("source/a.txt")).unwrap(),None,&JobControl::new()).unwrap(),expected);
        assert_eq!(streams(&File::open(dest.join("source/稀疏.bin")).unwrap(),None,&JobControl::new()).unwrap(),sparse_expected);
        assert_eq!(get_ea(&dest.join("source")),{let name=b"QING.TEST";let value=b"directory EA";let mut raw=vec![0,0,0,0,0,name.len()as u8,value.len()as u8,0];raw.extend_from_slice(name);raw.push(0);raw.extend_from_slice(value);raw});
        assert_eq!(fs::read(format!("{}:extra",dest.join("source").display())).unwrap(),b"directory stream");
        assert!(dest.join("source/empty").is_dir());
    }
    #[test]
    fn copied_move_compressed_directory_with_compressed_ads_and_sparse_child() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let input=source.join("empty");
        put_ea(&input,0x80,b"directory metadata");fs::write(format!("{}:custom",input.display()),b"GUI ADS").unwrap();
        {
            let directory=File::options().read(true).write(true).custom_flags(0x02200000).open(&input).unwrap();
            super::super::file_workspace_compression::set(&directory,2).unwrap();
            let ads=super::super::file_workspace_compression::stream(&directory,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),true).unwrap();
            super::super::file_workspace_compression::set(&ads,2).unwrap();
        }
        {let file=File::options().read(true).write(true).create_new(true).open(input.join("sparse.bin")).unwrap();super::super::file_workspace_compression::set(&file,0).unwrap();make_sparse(&file,1024*1024+17,true);let mut writer=&file;std::io::Seek::seek(&mut writer,std::io::SeekFrom::End(-1)).unwrap();writer.write_all(b"!").unwrap();}
        let expected=super::super::file_workspace_sparse::query(&File::open(input.join("sparse.bin")).unwrap(),&JobControl::new()).unwrap();
        transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
        let result=transfer(&[input.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:"rename".into(),..Default::default()}).unwrap();
        eprintln!("combined-directory transfer: {result:?}");
        assert!(result.retained_sources.is_empty(),"{:?}",result.retained_reasons);assert!(!input.exists());assert_eq!(result.moved,2);
        for folder in ["empty","empty (1)"]{assert_eq!(super::super::file_workspace_sparse::query(&File::open(dest.join(folder).join("sparse.bin")).unwrap(),&JobControl::new()).unwrap(),expected,"{folder}");}
    }
    #[test]
    fn copied_move_all_conflict_modes_keep_skipped_sources_and_exact_rename_mapping() {
        for mode in ["error","skip","rename","overwrite"] {
            let (_a,_b,source,dest,_force)=copy_move_setup();let file=source.join("a.txt");
            let expected=put_ea(&file,0,b"copied EA");fs::write(dest.join("a.txt"),b"old target").unwrap();
            let result=transfer(&[file.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:mode.into(),..Default::default()});
            if mode=="error" {assert!(result.is_err());assert!(file.exists());assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"old target");continue}
            let result=result.unwrap();
            if mode=="skip" {assert_eq!(result.skipped,1);assert!(file.exists());assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"old target");continue}
            assert_eq!(result.moved,1,"{:?}",result.retained_reasons);assert!(!file.exists());
            let target=dest.join(if mode=="rename"{"a (1).txt"}else{"a.txt"});
            assert_eq!(fs::read(&target).unwrap(),b"known bytes");assert_eq!(get_ea(&target),expected);
            if mode=="rename"{assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"old target");}
        }
    }
    #[test]
    fn copied_move_cancel_before_commit_restores_readonly_source_and_ea() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let file=source.join("a.txt");let expected=put_ea(&file,0,b"source EA");
        let mut permissions=fs::metadata(&file).unwrap().permissions();permissions.set_readonly(true);fs::set_permissions(&file,permissions).unwrap();
        let ctl=JobControl::new();let cancel=ctl.clone();BEFORE_TRANSACTION_COMMIT.with(|h|*h.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        let result=transfer(&[file.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{control:Some(ctl),..Default::default()});
        assert!(result.is_err());assert_eq!(fs::read(&file).unwrap(),b"known bytes");assert_eq!(get_ea(&file),expected);assert!(fs::metadata(&file).unwrap().permissions().readonly());
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"known bytes");
        for path in [&file,&dest.join("a.txt")] {let mut p=fs::metadata(path).unwrap().permissions();p.set_readonly(false);fs::set_permissions(path,p).unwrap();}
    }
    #[test]
    fn copied_move_late_child_rolls_back_all_source_deletions() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let late=source.join("late");let added=late.clone();
        BEFORE_TRANSACTION_DELETE.with(|h|*h.borrow_mut()=Some(Box::new(move||fs::write(added,b"foreign child").unwrap())));
        assert!(transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions::default()).is_err());
        assert_eq!(fs::read(source.join("a.txt")).unwrap(),b"known bytes");assert!(source.join("empty").is_dir());assert_eq!(fs::read(late).unwrap(),b"foreign child");
        assert_eq!(fs::read(dest.join("source/a.txt")).unwrap(),b"known bytes");
    }
    #[test]
    fn copied_move_existing_ea_writer_retains_source_and_explains_failure() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let file=source.join("a.txt");
        let _writer=File::options().access_mode(0x18).share_mode(7).open(&file).unwrap();
        let result=transfer(&[file.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions::default()).unwrap();
        assert_eq!(result.moved,0);assert_eq!(result.copied,1);assert_eq!(result.retained_sources.len(),1);assert!(!result.retained_reasons.is_empty());
        assert_eq!(fs::read(file).unwrap(),b"known bytes");assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"known bytes");
    }
    #[test]
    fn copied_move_late_source_or_target_ads_never_deletes_unmatched_source() {
        for change_target in [false,true] {
            let (_a,_b,source,dest,_force)=copy_move_setup();let file=source.join("a.txt");let changed=if change_target{dest.join("a.txt")}else{file.clone()};
            let late=format!("{}:late",changed.display());let write=late.clone();
            BEFORE_MOVE.with(|h|*h.borrow_mut()=Some(Box::new(move||fs::write(write,b"new data").unwrap())));
            let result=transfer(&[file.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions::default()).unwrap();
            assert_eq!(result.moved,0);assert_eq!(result.retained_sources.len(),1);assert!(!result.retained_reasons.is_empty());
            assert!(result.retained_reasons.iter().any(|reason|reason.contains(if change_target{"目标的内容、数据流或存储属性已改变"}else{"来源的内容、数据流或存储属性已改变"})),"{:?}",result.retained_reasons);
            assert_eq!(fs::read(file).unwrap(),b"known bytes");assert_eq!(fs::read(late).unwrap(),b"new data");
        }
    }
    #[test]
    fn copied_move_target_ea_writer_blocks_delete_and_target_replacement_is_denied() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let file=source.join("a.txt");let output=dest.join("a.txt");
        let writer=std::rc::Rc::new(std::cell::RefCell::new(None));let saved=writer.clone();let target=output.clone();
        BEFORE_MOVE.with(|h|*h.borrow_mut()=Some(Box::new(move||{
            assert!(fs::rename(&target,target.with_extension("replaced")).is_err());
            *saved.borrow_mut()=Some(File::options().access_mode(0x18).share_mode(7).open(&target).unwrap());
        })));
        let result=transfer(&[file.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions::default()).unwrap();
        assert_eq!(result.moved,0);assert_eq!(result.retained_sources.len(),1);assert!(!result.retained_reasons.is_empty());
        assert_eq!(fs::read(file).unwrap(),b"known bytes");assert_eq!(fs::read(output).unwrap(),b"known bytes");
        writer.borrow_mut().take();
    }
    #[test]
    fn copied_move_directory_skip_keeps_original_skipped_child_and_parent() {
        let (_a,_b,source,dest,_force)=copy_move_setup();
        let competing=dest.join("source");
        BEFORE_COPY.with(|h|*h.borrow_mut()=Some(Box::new(move||{fs::create_dir(&competing).unwrap();fs::write(competing.join("a.txt"),b"old target").unwrap();})));
        fs::write(source.join("b.txt"),b"move this").unwrap();
        let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:"skip".into(),..Default::default()}).unwrap();
        assert_eq!(result.skipped,1);assert_eq!(result.retained_sources.len(),1);assert!(source.exists());
        assert_eq!(fs::read(source.join("a.txt")).unwrap(),b"known bytes");assert_eq!(fs::read(dest.join("source/a.txt")).unwrap(),b"old target");
        assert!(!source.join("b.txt").exists());assert_eq!(fs::read(dest.join("source/b.txt")).unwrap(),b"move this");
    }
    #[test]
    fn copied_move_unavailable_transaction_reports_retained_complete_source() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let file=source.join("a.txt");
        super::super::file_workspace_transaction::FAIL_BEGIN.with(|value|value.set(true));
        let result=transfer(&[file.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions::default()).unwrap();
        assert_eq!(result.moved,0);assert_eq!(result.copied,1);assert_eq!(result.retained_sources.len(),1);
        assert!(result.retained_reasons.iter().any(|v|v.contains("不支持文件事务")));
        assert_eq!(fs::read(file).unwrap(),b"known bytes");assert_eq!(fs::read(dest.join("a.txt")).unwrap(),b"known bytes");
    }
    fn make_sparse(file:&File,eof:u64,with_data:bool) {
        use std::io::{Seek,SeekFrom};
        use super::super::file_workspace_sparse as sparse;
        file.set_len(eof).unwrap();
        sparse::restore(file,&sparse::Layout{eof,ranges:Vec::new()},&JobControl::new()).unwrap();
        if with_data {
            let mut writer=file;
            writer.seek(SeekFrom::Start(131072)).unwrap();
            writer.write_all(b"sparse allocated bytes").unwrap();
        }
    }
    #[test]
    fn sparse_transfer_preserves_holes_eof_main_ads_ea_and_bytes() {
        use super::super::{file_workspace_sparse as sparse,file_workspace_compression as compression};
        for eof in [0,4*1024*1024+17] {
            for with_data in [false,true] {
                if eof==0 && with_data {continue;}
                let (_temp,source,dest)=setup();let input=source.join("a.txt");
                fs::write(format!("{}:custom",input.display()),b"").unwrap();
                let ea=put_ea(&input,0x80,b"sparse EA");
                let (main_layout,ads_layout)={
                    let file=File::options().read(true).write(true).share_mode(7).open(&input).unwrap();
                    make_sparse(&file,eof,with_data);
                    let ads=compression::stream(&file,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),true).unwrap();
                    make_sparse(&ads,eof,with_data);
                    (sparse::query(&file,&JobControl::new()).unwrap(),sparse::query(&ads,&JobControl::new()).unwrap())
                };
                if with_data {assert!(main_layout.as_ref().unwrap().ranges.iter().map(|v|v.1).sum::<u64>()<eof);}
                let expected=fs::read(&input).unwrap();let expected_ads=fs::read(format!("{}:custom",input.display())).unwrap();
                let inventory=list_streams(input.to_str().unwrap()).unwrap();
                assert_eq!(inventory.streams.iter().find(|v|v.name=="custom").unwrap().size,eof);
                transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
                let output=dest.join("a.txt");let file=File::open(&output).unwrap();
                let ads=compression::stream(&file,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),false).unwrap();
                assert_eq!(sparse::query(&file,&JobControl::new()).unwrap(),main_layout);
                assert_eq!(sparse::query(&ads,&JobControl::new()).unwrap(),ads_layout);
                assert_eq!(fs::read(&output).unwrap(),expected);
                assert_eq!(fs::read(format!("{}:custom",output.display())).unwrap(),expected_ads);
                assert_eq!(get_ea(&output),ea);
                assert_eq!(get_ea(&input),ea);
            }
        }
    }
    #[test]
    fn sparse_directory_ads_and_cancel_preserve_originals() {
        use super::super::{file_workspace_sparse as sparse,file_workspace_compression as compression};
        for cancel in [false,true] {
            let (_temp,source,dest)=setup();
            fs::write(format!("{}:custom",source.display()),b"").unwrap();
            let layout={
                let directory=File::options().read(true).write(true).share_mode(7).custom_flags(0x02000000).open(&source).unwrap();
                let ads=compression::stream(&directory,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),true).unwrap();
                make_sparse(&ads,4*1024*1024+17,true);
                sparse::query(&ads,&JobControl::new()).unwrap()
            };
            let ctl=JobControl::new();
            if cancel {let stop=ctl.clone();AFTER_CHUNK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||stop.cancel())));}
            let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions{control:Some(ctl),..Default::default()});
            if cancel {assert!(result.is_err());assert_eq!(fs::read_dir(&dest).unwrap().count(),0);}
            else {
                result.unwrap();
                let directory=File::options().read(true).share_mode(7).custom_flags(0x02000000).open(dest.join("source")).unwrap();
                let ads=compression::stream(&directory,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),false).unwrap();
                assert_eq!(sparse::query(&ads,&JobControl::new()).unwrap(),layout);
            }
            assert!(source.join("a.txt").exists());
        }
    }
    #[test]
    fn sparse_unsupported_target_never_publishes_and_keeps_source() {
        use super::super::file_workspace_sparse as sparse;
        let (_temp,source,dest)=setup();let input=source.join("a.txt");
        {let file=File::options().read(true).write(true).open(&input).unwrap();make_sparse(&file,4*1024*1024+17,true);}
        let before=fs::read(&input).unwrap();
        sparse::FAIL_RESTORE.with(|failure|failure.set(true));
        let error=transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap_err();
        assert!(error.contains("目标无法保留稀疏存储属性"));
        assert_eq!(fs::read_dir(dest).unwrap().count(),0);
        assert_eq!(fs::read(input).unwrap(),before);
        assert!(!sparse::FAIL_RESTORE.with(|failure|failure.replace(false)));
    }
    #[test]
    fn sparse_non_aligned_eof_last_byte_and_multiple_blocks_roundtrip() {
        use std::io::{Seek,SeekFrom};
        use super::super::file_workspace_sparse as sparse;
        let (_temp,source,dest)=setup();let input=source.join("a.txt");
        let eof=4*1024*1024+17;
        let expected_layout={
            let mut file=File::options().read(true).write(true).open(&input).unwrap();
            make_sparse(&file,eof,false);
            for (offset,byte) in [(0,1),(131072,2),(1024*1024,3),(eof-1,255)] {
                file.seek(SeekFrom::Start(offset)).unwrap();file.write_all(&[byte]).unwrap();
            }
            let layout=sparse::query(&file,&JobControl::new()).unwrap().unwrap();
            assert!(layout.ranges.len()>=4);
            let last=layout.ranges.last().unwrap();
            assert_eq!(last.0+last.1,eof,"native allocation query must clip to requested EOF");
            eprintln!("native sparse EOF={eof}, last allocation offset={} length={}",last.0,last.1);
            layout
        };
        let expected=fs::read(&input).unwrap();
        transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
        let output=dest.join("a.txt");
        assert_eq!(sparse::query(&File::open(&output).unwrap(),&JobControl::new()).unwrap(),Some(expected_layout));
        assert_eq!(fs::read(output).unwrap(),expected);
    }
    #[test]
    fn sparse_query_paginates_over_256_ranges_and_transfers_all_bytes() {
        use std::io::{Seek,SeekFrom};
        use super::super::file_workspace_sparse as sparse;
        let (_temp,source,dest)=setup();let input=source.join("a.txt");
        let eof=270*131072+17;
        let expected_layout={
            let mut file=File::options().read(true).write(true).open(&input).unwrap();
            make_sparse(&file,eof,false);
            for index in 0..270 {
                file.seek(SeekFrom::Start(index*131072)).unwrap();
                file.write_all(&[(index%251) as u8]).unwrap();
            }
            let layout=sparse::query(&file,&JobControl::new()).unwrap().unwrap();
            assert_eq!(layout.ranges.len(),270,"fixture must exercise native continuation");
            assert!(layout.ranges.windows(2).all(|v|v[0].0+v[0].1<v[1].0));
            eprintln!("native sparse continuation returned {} disjoint ranges using 256-range query buffers",layout.ranges.len());
            layout
        };
        let expected=fs::read(&input).unwrap();
        transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
        let output=dest.join("a.txt");
        assert_eq!(sparse::query(&File::open(&output).unwrap(),&JobControl::new()).unwrap(),Some(expected_layout));
        assert_eq!(fs::read(output).unwrap(),expected);
    }
    #[test]
    fn empty_sparse_main_with_optional_ea_and_streams_obeys_all_conflicts(){
        use super::super::{file_workspace_sparse as sparse,file_workspace_compression as compression};
        for with_ea in [false,true]{for kind in ["none","regular","sparse"]{for mode in ["new","error","skip","rename","overwrite"]{
            let (_temp,source,dest)=setup();let input=source.join("a.txt");
            {
                let main=File::options().read(true).write(true).share_mode(7).open(&input).unwrap();
                make_sparse(&main,0,false);
                if kind!="none"{
                    let named=format!("{}:custom",input.display());fs::write(&named,if kind=="regular"{b"named data".as_slice()}else{b"".as_slice()}).unwrap();
                    let stream=compression::stream(&main,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),true).unwrap();
                    if kind=="sparse"{make_sparse(&stream,0,false);}
                    stream.sync_all().unwrap();
                }
                main.sync_all().unwrap();
            }
            if with_ea{put_ea(&input,0x80,b"empty main EA");}
            let old=dest.join("a.txt");if mode!="new"{fs::write(&old,b"original target").unwrap();}
            let options=EngineOptions{overwrite:if mode=="new"{"error".into()}else{mode.into()},..Default::default()};
            let result=transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&options);
            if mode=="error"{assert!(result.is_err());}else{assert!(result.is_ok(),"{with_ea}/{kind}/{mode}: {result:?}");}
            if matches!(mode,"error"|"skip"|"rename"){assert_eq!(fs::read(&old).unwrap(),b"original target");}
            let original=File::open(&input).unwrap();let layout=sparse::query(&original,&JobControl::new()).unwrap();
            assert_eq!(layout,Some(sparse::Layout{eof:0,ranges:Vec::new()}));
            if !matches!(mode,"error"|"skip"){
                let out=dest.join(if mode=="rename"{"a (1).txt"}else{"a.txt"});let copied=File::open(&out).unwrap();
                assert_eq!(sparse::query(&copied,&JobControl::new()).unwrap(),layout);
                assert_eq!(fs::read(&out).unwrap(),Vec::<u8>::new());
                if with_ea{assert_eq!(get_ea(&out),get_ea(&input));}
                if kind!="none"{
                    let name=":custom:$DATA".encode_utf16().collect::<Vec<_>>();
                    let a=compression::stream(&original,&name,false).unwrap();let b=compression::stream(&copied,&name,false).unwrap();
                    assert_eq!(sparse::query(&a,&JobControl::new()).unwrap(),sparse::query(&b,&JobControl::new()).unwrap());
                    assert_eq!(fs::read(format!("{}:custom",out.display())).unwrap(),fs::read(format!("{}:custom",input.display())).unwrap());
                }
            }
        }}}
    }
    #[test]
    fn empty_native_sparse_ads_exception_never_accepts_fake_empty_or_bad_flags(){
        use super::super::file_workspace_sparse::Layout;
        let empty=Layout{eof:0,ranges:Vec::new()};
        assert!(validate_alternate_sparse_header(0,0,Some(&empty)).is_ok());
        assert!(validate_alternate_sparse_header(8,0,Some(&empty)).is_ok());
        assert!(validate_alternate_sparse_header(0,0,None).is_ok());
        for (flags,size,layout) in [
            (0,0,Layout{eof:1,ranges:Vec::new()}),
            (0,0,Layout{eof:1,ranges:vec![(0,1)]}),
            (0,1,empty.clone()),(4,0,empty.clone()),
            (0,0,Layout{eof:0,ranges:vec![(0,1)]}),
        ] {assert!(validate_alternate_sparse_header(flags,size,Some(&layout)).is_err());}
        assert!(validate_alternate_sparse_header(8,0,None).is_err());
    }
    #[test]
    fn sparse_and_regular_alternate_stream_groups_do_not_leak_state() {
        use std::io::{Seek,SeekFrom};
        use super::super::{file_workspace_sparse as sparse,file_workspace_compression as compression};
        let (_temp,source,dest)=setup();let input=source.join("a.txt");
        let names=["a-sparse","b-regular","c-sparse","d-regular"];
        let mut layouts=Vec::new();let mut bytes=Vec::new();
        {
            let main=File::options().read(true).write(true).share_mode(7).open(&input).unwrap();
            for (index,name) in names.iter().enumerate() {
                let path=format!("{}:{name}",input.display());fs::write(&path,b"ordinary ADS bytes").unwrap();
                let mut stream=compression::stream(&main,&format!(":{name}:$DATA").encode_utf16().collect::<Vec<_>>(),true).unwrap();
                if index%2==0 {
                    make_sparse(&stream,1024*1024+17,true);
                    stream.seek(SeekFrom::End(-1)).unwrap();stream.write_all(&[0x71]).unwrap();
                }
                stream.sync_all().unwrap();
            }
        }
        // Finish every stream before recording the fixture's allocation map.
        // NTFS can materialize a cached zero page when the creation handles
        // close; an in-construction query is not a stable copy expectation.
        {
            let main=File::options().read(true).write(true).share_mode(7).open(&input).unwrap();
            for name in &names{
                let stream=compression::stream(&main,&format!(":{name}:$DATA").encode_utf16().collect::<Vec<_>>(),true).unwrap();
                bytes.push(fs::read(format!("{}:{name}",input.display())).unwrap());
                stream.sync_all().unwrap();
                layouts.push(sparse::query(&stream,&JobControl::new()).unwrap());
            }
            main.sync_all().unwrap();
        }
        transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
        let output=dest.join("a.txt");let main=File::open(&output).unwrap();
        assert!(sparse::query(&main,&JobControl::new()).unwrap().is_none());
        let source_main=File::open(&input).unwrap();
        for (index,name) in names.iter().enumerate() {
            let stream=compression::stream(&main,&format!(":{name}:$DATA").encode_utf16().collect::<Vec<_>>(),false).unwrap();
            let source_stream=compression::stream(&source_main,&format!(":{name}:$DATA").encode_utf16().collect::<Vec<_>>(),false).unwrap();
            assert_eq!(sparse::query(&source_stream,&JobControl::new()).unwrap(),layouts[index],"source layout changed: {name}");
            assert_eq!(sparse::query(&stream,&JobControl::new()).unwrap(),layouts[index]);
            assert_eq!(fs::read(format!("{}:{name}",output.display())).unwrap(),bytes[index]);
        }
    }
    #[test]
    fn compressed_transfer_preserves_each_stream_ea_and_directory_state() {
        use super::super::file_workspace_compression as compression;
        for directory in [false,true] {
            for main in [0,2] {
                for ads in [0,2] {
                    let (_temp,source,dest)=setup();
                    let input=if directory {source.clone()}else{source.join("a.txt")};
                    let ea=put_ea(&input,0x80,b"compressed EA");
                    fs::write(format!("{}:custom",input.display()),b"compressed ADS").unwrap();
                    {
                        let file=File::options().read(true).write(true).share_mode(7)
                            .custom_flags(0x02000000).open(&input).unwrap();
                        compression::set(&file,main).unwrap();
                        let stream=compression::stream(&file,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),true).unwrap();
                        compression::set(&stream,ads).unwrap();
                    }
                    let inventory=list_streams(input.to_str().unwrap()).unwrap();
                    assert_eq!(inventory.streams.iter().map(|v|(&*v.name,v.size)).collect::<Vec<_>>(),vec![("custom",14)]);
                    transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
                    let output=dest.join(input.file_name().unwrap());
                    let file=File::options().read(true).share_mode(7).custom_flags(0x02000000).open(&output).unwrap();
                    assert_eq!(compression::get(&file).unwrap(),main);
                    let stream=compression::stream(&file,&":custom:$DATA".encode_utf16().collect::<Vec<_>>(),false).unwrap();
                    assert_eq!(compression::get(&stream).unwrap(),ads);
                    assert_eq!(get_ea(&output),ea);
                    assert_eq!(fs::read(format!("{}:custom",output.display())).unwrap(),b"compressed ADS");
                    assert_eq!(get_ea(&input),ea);
                    if directory {assert_eq!(fs::read(output.join("a.txt")).unwrap(),b"known local bytes");}
                    else {assert_eq!(fs::read(output).unwrap(),b"known local bytes");}
                }
            }
        }
    }
    #[test]
    fn compressed_copy_cancel_keeps_source_and_publishes_nothing() {
        use super::super::file_workspace_compression as compression;
        for directory in [false,true] {
            let (_temp,source,dest)=setup();
            let input=if directory {source.clone()}else{source.join("a.txt")};
            let ea=put_ea(&input,0,b"keep EA");
            fs::write(format!("{}:custom",input.display()),b"keep ADS").unwrap();
            {
                let file=File::options().read(true).write(true).share_mode(7)
                    .custom_flags(0x02000000).open(&input).unwrap();
                compression::set(&file,2).unwrap();
            }
            let ctl=JobControl::new();let cancel=ctl.clone();
            AFTER_CHUNK.with(|hook| *hook.borrow_mut()=Some(Box::new(move||cancel.cancel())));
            assert!(transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,
                &EngineOptions{control:Some(ctl),..Default::default()}).is_err());
            assert_eq!(fs::read_dir(dest).unwrap().count(),0);
            assert_eq!(get_ea(&input),ea);
            assert_eq!(fs::read(format!("{}:custom",input.display())).unwrap(),b"keep ADS");
        }
    }
    #[test]
    fn compressed_directory_merge_keeps_existing_destination_state() {
        use super::super::file_workspace_compression as compression;
        let (_temp,source,dest)=setup();
        {let file=File::options().read(true).write(true).share_mode(7).custom_flags(0x02000000).open(&source).unwrap();compression::set(&file,2).unwrap();}
        fs::create_dir(dest.join("source")).unwrap();
        transfer(&[source.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap();
        let file=File::options().read(true).custom_flags(0x02000000).open(dest.join("source")).unwrap();
        assert_eq!(compression::get(&file).unwrap(),0);
        assert_eq!(fs::read(dest.join("source/a.txt")).unwrap(),b"known local bytes");
    }
    #[test]
    fn compressed_unsupported_target_error_keeps_source_and_no_output() {
        use super::super::file_workspace_compression as compression;
        for directory in [false,true] {
            let (_temp,source,dest)=setup();
            let input=if directory{source.clone()}else{source.join("a.txt")};
            let ea=put_ea(&input,0,b"keep EA");
            {let file=File::options().read(true).write(true).share_mode(7).custom_flags(0x02000000).open(&input).unwrap();compression::set(&file,2).unwrap();}
            compression::FAIL_SET.with(|failure|failure.set(true));
            let error=transfer(&[input.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions::default()).unwrap_err();
            assert!(error.contains("目标无法保留压缩存储属性"));
            assert_eq!(fs::read_dir(dest).unwrap().count(),0);
            assert_eq!(get_ea(&input),ea);
            assert!(input.exists());
            assert!(!compression::FAIL_SET.with(|failure|failure.replace(false)));
        }
    }
    #[test] fn local_link_preserves_main_ads_ea_attributes_and_expected_link_count(){
        #[link(name="kernel32")] unsafe extern "system" {fn GetFileInformationByHandle(handle:Handle,data:*mut u32)->i32;}
        fn info(path:&Path)->[u32;13]{let file=File::open(path).unwrap();let mut data=[0;13];assert_ne!(unsafe{GetFileInformationByHandle(file.as_raw_handle(),data.as_mut_ptr())},0);data}
        let root=tempfile::tempdir().unwrap();let source=root.path().join("source.txt");fs::write(&source,b"main bytes").unwrap();
        fs::write(format!("{}:unicode_数据",source.display()),b"ADS bytes").unwrap();fs::write(format!("{}:empty",source.display()),b"").unwrap();
        let ea=put_ea(&source,0,b"preserved EA");let mut attributes=fs::metadata(&source).unwrap().permissions();attributes.set_readonly(true);fs::set_permissions(&source,attributes).unwrap();
        let before=info(&source);
        let result=create_link(source.to_str().unwrap(),root.path().to_str().unwrap(),"hard.txt",LinkKind::Hard,&EngineOptions::default()).unwrap();
        let after=info(&source);let mut expected=before;expected[10]+=1;assert_eq!(after,expected);assert_eq!(info(Path::new(&result.path)),after);
        assert_eq!(fs::read(&source).unwrap(),b"main bytes");assert_eq!(fs::read(&result.path).unwrap(),b"main bytes");
        for path in [&source,Path::new(&result.path)] {assert_eq!(fs::read(format!("{}:unicode_数据",path.display())).unwrap(),b"ADS bytes");assert_eq!(fs::read(format!("{}:empty",path.display())).unwrap(),b"");assert_eq!(get_ea(path),ea);}
        let mut attributes=fs::metadata(&source).unwrap().permissions();attributes.set_readonly(false);fs::set_permissions(&source,attributes).unwrap();
    }
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
        let merged=transfer(&[source.display().to_string()], dest.to_str().unwrap(), false,
            &EngineOptions::default()).unwrap();
        assert_eq!(merged.metadata_not_merged,vec![source.display().to_string()]);
        assert_eq!(fs::read(dest.join("source/a.txt")).unwrap(),b"known local bytes");
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
                assert!(!file.exists());
                assert!(result.retained_sources.is_empty());
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
        assert_eq!(result.moved, 2);
        assert!(!source.join("a.txt").exists());
        assert!(!source.join("empty").exists());
        assert_eq!(fs::read(dest.join("source/a.txt")).unwrap(), b"known local bytes");
        assert!(dest.join("source/empty").is_dir());
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
    fn existing_directory_metadata_is_preserved_across_child_conflict_choices() {
        for kind in ["source-only","target-only","conflicting","equal","different-names"] {
            for policy in ["error","skip","rename","overwrite"] {
                let (_temp,source,dest)=setup();let target=dest.join("source");fs::create_dir(&target).unwrap();
                fs::write(target.join("a.txt"),b"old destination child").unwrap();
                if kind!="target-only" {
                    put_ea(&source,0x80,b"source metadata");
                    fs::write(format!("{}:same",source.display()),b"source ADS").unwrap();
                }
                if kind!="source-only" {
                    put_ea(&target,0x80,if kind=="equal"{b"source metadata"}else{b"target metadata"});
                    fs::write(format!("{}:same",target.display()),if kind=="equal"{b"source ADS"}else{b"target ADS"}).unwrap();
                }
                if kind=="different-names" {
                    for (directory,name,value) in [(&source,b"QING.SOURCE".as_slice(),b"source only".as_slice()),(&target,b"QING.TARGET".as_slice(),b"target only".as_slice())] {
                        let mut raw=vec![0;8];raw[5]=name.len() as u8;raw[6..8].copy_from_slice(&(value.len() as u16).to_le_bytes());raw.extend_from_slice(name);raw.push(0);raw.extend_from_slice(value);
                        assert_eq!(put_raw_ea(directory,&raw),0);fs::write(format!("{}:{}",directory.display(),String::from_utf8_lossy(name)),value).unwrap();
                    }
                }
                let source_before=streams(&File::options().read(true).custom_flags(0x02200000).open(&source).unwrap(),None,&JobControl::new()).unwrap();
                let target_before=streams(&File::options().read(true).custom_flags(0x02200000).open(&target).unwrap(),None,&JobControl::new()).unwrap();
                let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),false,&EngineOptions{overwrite:policy.into(),..Default::default()});
                if policy=="error" {assert!(result.is_err());} else {
                    let result=result.unwrap();assert_eq!(result.metadata_not_merged.is_empty(),matches!(kind,"target-only"|"equal"),"{kind} {policy}");
                    let child=target.join(if policy=="rename"{"a (1).txt"}else{"a.txt"});
                    assert_eq!(fs::read(child).unwrap(),if policy=="skip"{b"old destination child".as_slice()}else{b"known local bytes".as_slice()});
                }
                assert_eq!(streams(&File::options().read(true).custom_flags(0x02200000).open(&source).unwrap(),None,&JobControl::new()).unwrap(),source_before);
                assert_eq!(streams(&File::options().read(true).custom_flags(0x02200000).open(&target).unwrap(),None,&JobControl::new()).unwrap(),target_before);
                assert_eq!(fs::read(source.join("a.txt")).unwrap(),b"known local bytes");
            }
        }
    }
    #[test]
    fn moving_directory_metadata_obeys_all_existing_conflict_choices() {
        for policy in ["error","skip","rename","overwrite"] {
            let (_a,_b,source,dest,_force)=copy_move_setup();let target=dest.join("source");fs::create_dir(&target).unwrap();fs::write(target.join("a.txt"),b"old").unwrap();
            let source_ea=put_ea(&source,0,b"source only");let target_ea=put_ea(&target,0,b"target only");
            fs::write(format!("{}:same",source.display()),b"source ADS").unwrap();fs::write(format!("{}:same",target.display()),b"target ADS").unwrap();
            let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:policy.into(),..Default::default()});
            if policy=="error" {assert!(result.is_err());} else {
                let result=result.unwrap();
                if policy=="rename" {
                    assert!(!source.exists());let renamed=dest.join("source (1)");assert_eq!(get_ea(&renamed),source_ea);
                    assert_eq!(fs::read(format!("{}:same",renamed.display())).unwrap(),b"source ADS");assert!(result.metadata_not_merged.is_empty());
                } else if policy=="overwrite" {
                    assert!(!source.join("a.txt").exists());assert_eq!(result.metadata_not_merged,vec![source.display().to_string()]);assert_eq!(result.retained_sources,vec![source.display().to_string()]);
                } else {assert!(source.join("a.txt").exists());assert_eq!(result.moved,0);}
            }
            assert_eq!(get_ea(&target),target_ea);assert_eq!(fs::read(format!("{}:same",target.display())).unwrap(),b"target ADS");
            if source.exists(){assert_eq!(get_ea(&source),source_ea);assert_eq!(fs::read(format!("{}:same",source.display())).unwrap(),b"source ADS");}
            assert_eq!(fs::read(target.join("a.txt")).unwrap(),if policy=="overwrite"{b"known bytes".as_slice()}else{b"old".as_slice()});
        }
    }
    #[test]
    fn nested_directory_merge_moves_children_but_keeps_distinct_source_metadata() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let target=dest.join("source");
        fs::create_dir(&target).unwrap();fs::create_dir(source.join("nested")).unwrap();fs::create_dir(target.join("nested")).unwrap();
        fs::write(source.join("nested/child"),b"nested content").unwrap();
        for (base,tag) in [(&source,b"source".as_slice()),(&target,b"target".as_slice())] {
            for folder in [base.clone(),base.join("nested")] {put_ea(&folder,0,tag);fs::write(format!("{}:custom",folder.display()),tag).unwrap();}
        }
        let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:"overwrite".into(),..Default::default()}).unwrap();
        assert_eq!(result.metadata_not_merged.len(),2);assert_eq!(result.retained_sources,vec![source.display().to_string()]);
        assert!(result.retained_reasons.iter().all(|reason|reason.contains("附加数据未合并")),"{:?}",result.retained_reasons);
        assert!(!source.join("a.txt").exists());assert!(!source.join("nested/child").exists());
        assert_eq!(fs::read(target.join("nested/child")).unwrap(),b"nested content");
        assert!(source.join("nested").is_dir());
        for (base,tag) in [(&source,b"source".as_slice()),(&target,b"target".as_slice())] {
            for folder in [base.clone(),base.join("nested")] {assert_eq!(fs::read(format!("{}:custom",folder.display())).unwrap(),tag);assert!(!get_ea(&folder).is_empty());}
        }
    }
    #[test]
    fn late_directory_metadata_change_is_not_disguised_as_expected_merge_retention() {
        let (_a,_b,source,dest,_force)=copy_move_setup();let target=dest.join("source");fs::create_dir(&target).unwrap();
        put_ea(&source,0,b"source metadata");put_ea(&target,0,b"original target metadata");
        let late=target.clone();BEFORE_MOVE.with(|hook|*hook.borrow_mut()=Some(Box::new(move||{put_ea(&late,0,b"late target metadata");})));
        let result=transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:"overwrite".into(),..Default::default()}).unwrap();
        assert_eq!(result.metadata_not_merged.len(),1);assert!(source.is_dir());
        assert!(result.retained_reasons.iter().any(|reason|reason.contains("目标的内容、数据流或存储属性已改变")),"{:?}",result.retained_reasons);
        assert!(!result.retained_reasons.iter().any(|reason|reason.contains("附加数据未合并")));
    }
    #[test]
    fn cancelling_directory_content_merge_preserves_both_directory_stream_sets() {
        let (_temp,source,dest)=setup();let target=dest.join("source");fs::create_dir(&target).unwrap();
        put_ea(&source,0,b"source EA");put_ea(&target,0,b"target EA");
        fs::write(format!("{}:source-only",source.display()),b"source").unwrap();fs::write(format!("{}:target-only",target.display()),b"target").unwrap();
        let source_ea=get_ea(&source);let target_ea=get_ea(&target);let ctl=JobControl::new();let cancel=ctl.clone();
        AFTER_CHUNK.with(|hook|*hook.borrow_mut()=Some(Box::new(move||cancel.cancel())));
        assert!(transfer(&[source.display().to_string()],dest.to_str().unwrap(),true,&EngineOptions{overwrite:"overwrite".into(),control:Some(ctl),..Default::default()}).unwrap_err().contains("cancelled"));
        assert_eq!(get_ea(&source),source_ea);assert_eq!(get_ea(&target),target_ea);
        assert_eq!(fs::read(format!("{}:source-only",source.display())).unwrap(),b"source");assert_eq!(fs::read(format!("{}:target-only",target.display())).unwrap(),b"target");
        assert_eq!(fs::read(source.join("a.txt")).unwrap(),b"known local bytes");assert!(!target.join("a.txt").exists());
    }
    #[test]
    fn directory_stream_conflict_never_overwrites_existing_directory_streams() {
        let (_t, source, dest) = setup();
        fs::write(format!("{}:custom", source.display()), b"source stream").unwrap();
        let existing = dest.join("source");
        fs::create_dir(&existing).unwrap();
        fs::write(format!("{}:custom", existing.display()), b"existing stream").unwrap();
        let result = transfer(
            &[source.display().to_string()],
            dest.to_str().unwrap(),
            false,
            &EngineOptions::default(),
        )
        .unwrap();
        assert_eq!(result.metadata_not_merged,vec![source.display().to_string()]);
        assert_eq!(
            fs::read(format!("{}:custom", existing.display())).unwrap(),
            b"existing stream"
        );
        assert_eq!(fs::read(existing.join("a.txt")).unwrap(),b"known local bytes");
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
