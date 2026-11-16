//! Read-only named-stream metadata. No payload handle, write, or path-based ADS open.
use super::*;
use std::path::{Component, Prefix};
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_STREAMS: usize = 4096;
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct StreamInfo { pub name: String, pub size: u64, pub allocated_size: u64 }
#[derive(Debug, Serialize)]
pub struct StreamInventory { pub path: String, pub revision: String, pub streams: Vec<StreamInfo> }
#[link(name="kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandleEx(file: Handle, kind: i32, data: *mut u32, size: u32) -> i32;
    fn GetVolumeInformationByHandleW(file: Handle, name: *mut u16, name_size: u32, serial: *mut u32,
        component: *mut u32, flags: *mut u32, filesystem: *mut u16, filesystem_size: u32) -> i32;
}
fn checked_path(text: &str) -> Result<PathBuf, String> {
    if text.is_empty() || text.contains('\0') { return Err("请选择本地文件或文件夹".into()); }
    let path = PathBuf::from(text);
    if !path.is_absolute() || !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(),Prefix::Disk(_))) {
        return Err("数据流浏览仅支持普通本地绝对路径".into());
    }
    // Check raw spelling too: Path::components normalizes interior dot segments.
    let tail = text.get(3..).ok_or("无效的本地路径")?;
    for name in tail.split(['/', '\\']).filter(|s| !s.is_empty()) {
        let leaf = crate::extract::relative_path(name)?;
        if leaf.components().count() != 1 || name == "." || name == ".." { return Err("无效的本地路径组件".into()); }
    }
    local_path(text)
}
fn parse(data: &[u8]) -> Result<Vec<StreamInfo>, String> {
    if data.len() > MAX_BYTES { return Err("数据流元数据过大".into()); }
    let mut at = 0usize; let mut result = Vec::new(); let mut names = HashSet::new();
    for _ in 0..MAX_STREAMS {
        let h = data.get(at..at.checked_add(24).ok_or("数据流偏移溢出")?).ok_or("数据流记录不完整")?;
        let next = u32::from_le_bytes(h[..4].try_into().unwrap()) as usize;
        let len = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
        let size = i64::from_le_bytes(h[8..16].try_into().unwrap());
        let allocation = i64::from_le_bytes(h[16..24].try_into().unwrap());
        if len == 0 || len > 65536 || len % 2 != 0 || size < 0 || allocation < 0 { return Err("无效的数据流记录".into()); }
        let end = at.checked_add(24+len).ok_or("数据流偏移溢出")?;
        let raw = data.get(at+24..end).ok_or("数据流名称不完整")?;
        let units = raw.chunks_exact(2).map(|x|u16::from_le_bytes([x[0],x[1]])).collect::<Vec<_>>();
        let full = String::from_utf16(&units).map_err(|_|"无效的数据流 UTF-16 名称")?;
        if !names.insert(full.clone()) {return Err("重复的数据流名称".into());}
        if full != "::$DATA" {
            let name = full.strip_prefix(':').and_then(|s|s.strip_suffix(":$DATA")).ok_or("无效的数据流类型")?;
            if name.is_empty() || name.chars().any(|c| matches!(c, ':' | '/' | '\\' | '\0')) {
                return Err("无效或重复的数据流名称".into());
            }
            result.push(StreamInfo {name:name.into(),size:size as u64,allocated_size:allocation as u64});
        }
        if next == 0 { result.sort_by(|a,b|a.name.cmp(&b.name)); return Ok(result); }
        if next % 8 != 0 || next < 24+len || next > data.len()-at { return Err("无效的数据流记录偏移".into()); }
        at += next;
    }
    Err("数据流数量超过限制".into())
}
fn query(file: &File) -> Result<Vec<StreamInfo>, String> {
    let mut flags = 0;
    if unsafe {GetVolumeInformationByHandleW(file.as_raw_handle(),std::ptr::null_mut(),0,std::ptr::null_mut(),std::ptr::null_mut(),&mut flags,std::ptr::null_mut(),0)} == 0 {
        return Err(format!("无法查询数据流支持：{}",std::io::Error::last_os_error()));
    }
    if flags & 0x40000 == 0 {return Err("此文件系统不支持命名数据流".into());}
    let mut size = 65536;
    loop {
        let mut data=vec![0u8;size];
        if unsafe {GetFileInformationByHandleEx(file.as_raw_handle(),7,data.as_mut_ptr().cast(),size as u32)} != 0 {return parse(&data);}
        let e=std::io::Error::last_os_error();
        if e.raw_os_error()==Some(38) && file.metadata().map_err(|e|e.to_string())?.is_dir() {return Ok(Vec::new());}
        if matches!(e.raw_os_error(),Some(122|234)) && size < MAX_BYTES {size*=2;continue;}
        return Err(format!("无法读取数据流元数据：{e}"));
    }
}
pub fn list_streams(text: &str) -> Result<StreamInventory,String> {
    let path=checked_path(text)?;
    let mut guards=CheckedDirectories::default();
    let parent=path.parent().filter(|p|!p.as_os_str().is_empty()).unwrap_or(&path);
    guards.check(parent)?;
    // READ_ATTRIBUTES only: no base or named-stream content access. This is a
    // single OS metadata enumeration, not an atomic snapshot across streams.
    // OPEN_REPARSE_POINT never follows the leaf.
    let preliminary=fs::symlink_metadata(&path).map_err(|e|format!("无法读取所属项目属性：{e}"))?;
    if preliminary.file_attributes() & 0x400 != 0 {return Err("不浏览链接的数据流".into());}
    // Directory handles need BACKUP_SEMANTICS; ordinary files do not. Avoid
    // backup-intent ACL bypass for files in a process with backup privilege.
    let flags=0x00200000 | if preliminary.is_dir(){0x02000000}else{0};
    let file=fs::OpenOptions::new().access_mode(0x80).share_mode(1).custom_flags(flags)
        .open(&path).map_err(|e|format!("无法打开数据流所属项目：{e}"))?;
    let before=file.metadata().map_err(|e|e.to_string())?;
    if before.file_attributes() & 0x400 != 0 || (!before.is_file() && !before.is_dir()) {return Err("不浏览链接或特殊项目的数据流".into());}
    let identity=directory_identity(&file)?;
    let streams=query(&file)?;
    let after=file.metadata().map_err(|e|e.to_string())?;
    if identity!=directory_identity(&file)? || stamp::entry_revision(&before)!=stamp::entry_revision(&after) {return Err("项目在读取期间发生变化，请刷新".into());}
    Ok(StreamInventory {path:path.display().to_string(),revision:format!("{}:{}:{}",identity.0,identity.1,stamp::entry_revision(&after)),streams})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record(name:&str,size:i64)->Vec<u8>{let raw=name.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>();let mut b=vec![0;24];b[4..8].copy_from_slice(&(raw.len() as u32).to_le_bytes());b[8..16].copy_from_slice(&size.to_le_bytes());b.extend(raw);b}
    #[test]fn parser_rejects_malformed_and_unbounded_records(){
        for name in [":$DATA",":bad/name:$DATA",":bad\\name:$DATA",":bad:more:$DATA",":bad\0:$DATA",":foo:$INDEX_ALLOCATION"]{assert!(parse(&record(name,0)).is_err(),"{name}");}
        assert!(parse(&record(":foo:$DATA",-1)).is_err());assert!(parse(&[0;23]).is_err());
        let mut invalid=record(":foo:$DATA",0);invalid[24..26].copy_from_slice(&0xd800u16.to_le_bytes());assert!(parse(&invalid).is_err());
        let mut offset=record(":a:$DATA",0);offset[..4].copy_from_slice(&8u32.to_le_bytes());assert!(parse(&offset).is_err());
        assert!(parse(&vec![0;MAX_BYTES+1]).is_err());
        let mut all=Vec::new();for i in 0..MAX_STREAMS+1{let mut b=record(&format!(":s{i}:$DATA"),0);while b.len()%8!=0{b.push(0);}let n=b.len() as u32;b[..4].copy_from_slice(&n.to_le_bytes());all.extend(b);}assert!(parse(&all).is_err());
    }
    #[test]fn metadata_only_inventory_covers_file_directory_empty_unicode_and_releases_pins(){
        let temp=tempfile::tempdir().unwrap();
        for base in [temp.path().join("file"),temp.path().join("directory")]{
            if base.ends_with("directory"){fs::create_dir(&base).unwrap();}else{fs::write(&base,b"main payload").unwrap();}
            assert!(list_streams(base.to_str().unwrap()).unwrap().streams.is_empty());
            let named=PathBuf::from(format!("{}:Unicode-数据",base.display()));let empty=PathBuf::from(format!("{}:empty",base.display()));
            fs::write(&named,b"payload").unwrap();fs::write(&empty,b"").unwrap();
            let before=fs::metadata(&base).unwrap();let entries=list_streams(base.to_str().unwrap()).unwrap().streams;
            assert_eq!(entries.iter().map(|x|(&*x.name,x.size)).collect::<Vec<_>>(),vec![("Unicode-数据",7),("empty",0)]);
            let os=std::process::Command::new("pwsh").args(["-NoProfile","-NonInteractive","-Command","[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; @(Get-Item -LiteralPath $env:QZ_ADS_ENUM_FILE -Stream * -ErrorAction Stop | Where-Object Stream -ne ':$DATA' | Select-Object Stream,Length) | ConvertTo-Json -Compress"]).env("QZ_ADS_ENUM_FILE",&base).output().unwrap();assert!(os.status.success(),"{}",String::from_utf8_lossy(&os.stderr));
            let independent:serde_json::Value=serde_json::from_slice(os.stdout.strip_prefix(&[0xef,0xbb,0xbf]).unwrap_or(&os.stdout)).unwrap();
            let mut expected=independent.as_array().unwrap().iter().map(|v|(v["Stream"].as_str().unwrap().to_owned(),v["Length"].as_u64().unwrap())).collect::<Vec<_>>();expected.sort();assert_eq!(entries.iter().map(|e|(e.name.clone(),e.size)).collect::<Vec<_>>(),expected);
            let after=fs::metadata(&base).unwrap();assert_eq!((before.last_access_time(),before.last_write_time(),before.creation_time(),before.file_attributes()),(after.last_access_time(),after.last_write_time(),after.creation_time(),after.file_attributes()));
            assert_eq!(fs::read(&named).unwrap(),b"payload");assert_eq!(fs::read(&empty).unwrap(),b"");
            if base.is_file(){assert_eq!(fs::read(&base).unwrap(),b"main payload");}
            fs::rename(&base,base.with_extension("renamed")).unwrap();
        }
    }
    #[test]fn rejects_stream_paths_namespaces_and_reparse_without_disrupting_writers(){
        for path in ["", "relative",r"C:\a:stream",r"C:\a\..\b",r"C:\a\.\b",r"\\.\PhysicalDrive0",r"\\?\C:\a",r"\\host\share\file"]{assert!(checked_path(path).is_err(),"{path}");}
        let temp=tempfile::tempdir().unwrap();let file=temp.path().join("file");fs::write(&file,b"x").unwrap();
        let mut writer=fs::OpenOptions::new().write(true).share_mode(7).open(&file).unwrap();let _snapshot=list_streams(file.to_str().unwrap());assert!(writer.metadata().is_ok());std::io::Write::write_all(&mut writer,b"y").unwrap();drop(writer);assert_eq!(fs::read(&file).unwrap(),b"y");assert!(list_streams(file.to_str().unwrap()).is_ok());
        let ads=format!("{}:active",file.display());fs::write(&ads,b"one").unwrap();let mut stream_writer=fs::OpenOptions::new().write(true).share_mode(7).open(&ads).unwrap();let _snapshot=list_streams(file.to_str().unwrap());std::io::Write::write_all(&mut stream_writer,b"two").unwrap();drop(stream_writer);assert_eq!(fs::read(&ads).unwrap(),b"two");
        let link=temp.path().join("junction");let target=temp.path().join("target");fs::create_dir(&target).unwrap();
        let status=std::process::Command::new("powershell").args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:QZ_ADS_TEST_LINK -Target $env:QZ_ADS_TEST_TARGET | Out-Null"]).env("QZ_ADS_TEST_LINK",&link).env("QZ_ADS_TEST_TARGET",&target).output().unwrap();assert!(status.status.success());assert!(list_streams(link.to_str().unwrap()).is_err());fs::write(target.join("child"),b"outside").unwrap();assert!(list_streams(link.join("child").to_str().unwrap()).is_err());
    }
    #[test]fn attributes_permission_is_required_but_payload_read_permission_is_not(){
        use std::os::windows::ffi::OsStrExt;
        #[link(name="advapi32")]unsafe extern "system"{
            fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,result:*mut Handle,size:*mut u32)->i32;
            fn GetFileSecurityW(path:*const u16,info:u32,descriptor:*mut u8,size:u32,needed:*mut u32)->i32;
            fn SetFileSecurityW(path:*const u16,info:u32,descriptor:*mut c_void)->i32;
        }
        #[link(name="kernel32")]unsafe extern "system"{fn LocalFree(memory:Handle)->Handle;}
        let temp=tempfile::tempdir().unwrap();let base=temp.path().join("permissions");fs::write(&base,b"payload").unwrap();fs::write(format!("{}:ads",base.display()),b"stream").unwrap();
        let path=base.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();let mut needed=0;
        unsafe{GetFileSecurityW(path.as_ptr(),4,std::ptr::null_mut(),0,&mut needed);}
        let mut original=vec![0;needed as usize];assert_ne!(unsafe{GetFileSecurityW(path.as_ptr(),4,original.as_mut_ptr(),needed,&mut needed)},0);
        struct Restore{path:Vec<u16>,descriptor:Vec<u8>}
        impl Drop for Restore{fn drop(&mut self){assert_ne!(unsafe{SetFileSecurityW(self.path.as_ptr(),0x20000004,self.descriptor.as_ptr().cast_mut().cast())},0);}}
        let _restore=Restore{path:path.clone(),descriptor:original};
        let set=|acl_path:&[u16],denied:u32|{let text=format!("D:P(D;;0x{denied:x};;;WD)(A;;FA;;;OW)").encode_utf16().chain(Some(0)).collect::<Vec<_>>();let mut descriptor=std::ptr::null_mut();assert_ne!(unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(text.as_ptr(),1,&mut descriptor,std::ptr::null_mut())},0);let ok=unsafe{SetFileSecurityW(acl_path.as_ptr(),0x80000004,descriptor.cast())};unsafe{LocalFree(descriptor);}assert_ne!(ok,0);};
        set(&path,1);assert!(fs::File::open(&base).is_err());let inventory=list_streams(base.to_str().unwrap()).unwrap();assert_eq!(inventory.streams[0].name,"ads");
        set(&path,0x80);
        // Windows may grant READ_ATTRIBUTES through a listable parent even when
        // the leaf denies it. Deny parent enumeration too for a real denial.
        let parent=temp.path().as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();let mut n=0;unsafe{GetFileSecurityW(parent.as_ptr(),4,std::ptr::null_mut(),0,&mut n);}
        let mut saved=vec![0;n as usize];assert_ne!(unsafe{GetFileSecurityW(parent.as_ptr(),4,saved.as_mut_ptr(),n,&mut n)},0);
        let _parent_restore=Restore{path:parent.clone(),descriptor:saved};set(&parent,1);
        let error=list_streams(base.to_str().unwrap()).unwrap_err();assert!(!error.is_empty());
    }

}
