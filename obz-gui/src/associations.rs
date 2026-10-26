//! Per-user candidate registration. Windows owns the final default selection.
//! Never writes Explorer FileExts/UserChoice or any extension's default value.
use serde::Serialize;
use std::{collections::BTreeSet, path::Path, sync::Mutex};
use windows::{core::PCWSTR, Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND},
    System::Registry::*,
    UI::{Shell::{ShellExecuteW, SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST}, WindowsAndMessaging::SW_SHOWNORMAL},
}};

const PROGID: &str = "OpenArchive.Archive";
const APP_NAME: &str = "qing-zip";
const CAPABILITIES: &str = r"Software\OpenArchive\Capabilities";
const OWNER_KEY: &str = r"Software\OpenArchive\Associations";
const OWNER: &str = "OpenArchive.Associations.v1";
const EXTENSIONS: &[&str] = &[".zip", ".7z", ".rar", ".zipx", ".tar", ".gz", ".bz2", ".xz", ".tgz", ".tbz2", ".txz", ".iso", ".cab", ".wim", ".lzh", ".lha", ".ace", ".egg", ".alz", ".zst", ".asar", ".br", ".lz4", ".aes", ".lz", ".uu", ".uue", ".xxe", ".pma", ".zpaq", ".daa", ".isz"];
static REGISTRATION: Mutex<()> = Mutex::new(());

#[derive(Debug, Serialize)]
pub struct AssociationStatus {
    extension: String,
    registered: bool,
    is_default: bool,
    current_progid: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct RawValue { kind: REG_VALUE_TYPE, data: Vec<u8> }
fn wide(value: &str) -> Result<Vec<u16>,String> {
    if value.contains('\0') {return Err("注册信息包含无效字符".into());}
    Ok(value.encode_utf16().chain(Some(0)).collect())
}
fn string_value(value: &str) -> RawValue {
    RawValue{kind:REG_SZ,data:value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()}
}
fn string_data(value: Option<RawValue>) -> Option<String> {
    let value=value?;
    if ![REG_SZ,REG_EXPAND_SZ].contains(&value.kind) || value.data.len()%2!=0 {return None;}
    let words=value.data.chunks_exact(2).map(|b|u16::from_le_bytes([b[0],b[1]])).take_while(|x|*x!=0).collect::<Vec<_>>();
    String::from_utf16(&words).ok().filter(|s|!s.is_empty())
}
struct Key(HKEY);
impl Drop for Key {fn drop(&mut self){unsafe{let _=RegCloseKey(self.0);}}}
fn read_value(hive:HKEY,path:&str,name:&str)->Result<Option<RawValue>,String>{
    let path=wide(path)?;let name=wide(name)?;let mut key=HKEY::default();
    let status=unsafe{RegOpenKeyExW(hive,PCWSTR(path.as_ptr()),None,KEY_QUERY_VALUE,&mut key)};
    if status==ERROR_FILE_NOT_FOUND || status==ERROR_PATH_NOT_FOUND {return Ok(None);}
    status.ok().map_err(|e|e.to_string())?;let key=Key(key);
    let mut kind=REG_VALUE_TYPE::default();let mut len=0u32;
    let status=unsafe{RegQueryValueExW(key.0,PCWSTR(name.as_ptr()),None,Some(&mut kind),None,Some(&mut len))};
    if status==ERROR_FILE_NOT_FOUND {return Ok(None);}
    status.ok().map_err(|e|e.to_string())?;
    if len>65536{return Err("文件关联注册值过大".into());}
    let mut data=vec![0u8;len as usize];
    unsafe{RegQueryValueExW(key.0,PCWSTR(name.as_ptr()),None,Some(&mut kind),Some(data.as_mut_ptr()),Some(&mut len))}.ok().map_err(|e|e.to_string())?;
    data.truncate(len as usize);Ok(Some(RawValue{kind,data}))
}
fn write_value(path:&str,name:&str,value:Option<&RawValue>)->Result<(),String>{
    let path=wide(path)?;let name=wide(name)?;let mut key=HKEY::default();
    if let Some(value)=value {
        unsafe{RegCreateKeyExW(HKEY_CURRENT_USER,PCWSTR(path.as_ptr()),None,PCWSTR::null(),REG_OPTION_NON_VOLATILE,KEY_SET_VALUE,None,&mut key,None)}.ok().map_err(|e|e.to_string())?;
        let key=Key(key);
        unsafe{RegSetValueExW(key.0,PCWSTR(name.as_ptr()),None,value.kind,Some(&value.data))}.ok().map_err(|e|e.to_string())
    }else{
        let status=unsafe{RegOpenKeyExW(HKEY_CURRENT_USER,PCWSTR(path.as_ptr()),None,KEY_SET_VALUE,&mut key)};
        if status==ERROR_FILE_NOT_FOUND || status==ERROR_PATH_NOT_FOUND{return Ok(());}
        status.ok().map_err(|e|e.to_string())?;let key=Key(key);
        let status=unsafe{RegDeleteValueW(key.0,PCWSTR(name.as_ptr()))};
        if status==ERROR_FILE_NOT_FOUND {Ok(())}else{status.ok().map_err(|e|e.to_string())}
    }
}
/// Tests inject an isolated prefix under HKCU; no test touches real extensions.
struct Registry {prefix:String}
impl Registry {
    fn user()->Self{Self{prefix:String::new()}}
    fn path(&self,key:&str)->String{if self.prefix.is_empty(){key.into()}else{format!("{}\\{key}",self.prefix)}}
    fn read(&self,key:&str,name:&str)->Result<Option<RawValue>,String>{read_value(HKEY_CURRENT_USER,&self.path(key),name)}
    fn write(&self,key:&str,name:&str,value:Option<&RawValue>)->Result<(),String>{write_value(&self.path(key),name,value)}
    fn default_progid(&self,ext:&str)->Result<Option<String>,String>{
        let choice=format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{ext}\UserChoice");
        if let Some(value)=string_data(self.read(&choice,"ProgId")?){return Ok(Some(value));}
        if let Some(value)=string_data(self.read(&format!(r"Software\Classes\{ext}"),"")?){return Ok(Some(value));}
        if self.prefix.is_empty(){Ok(string_data(read_value(HKEY_CLASSES_ROOT,ext,"")?))}
        else{Ok(string_data(self.read(&format!(r"MachineClasses\{ext}"),"")?))}
    }
    fn statuses(&self)->Result<Vec<AssociationStatus>,String>{
        let handler=string_data(self.read(&format!(r"Software\Classes\{PROGID}\shell\open\command"),"")?).is_some();
        let advertised=string_data(self.read(r"Software\RegisteredApplications",APP_NAME)?).as_deref()==Some(CAPABILITIES);
        EXTENSIONS.iter().map(|ext|{
            let current=self.default_progid(ext)?;
            let registered=handler && advertised && self.read(&format!(r"Software\Classes\{ext}\OpenWithProgids"),PROGID)?.is_some()
                && string_data(self.read(&format!(r"{CAPABILITIES}\FileAssociations"),ext)?).as_deref()==Some(PROGID);
            Ok(AssociationStatus{extension:(*ext).into(),registered,is_default:current.as_deref().is_some_and(|v|v.eq_ignore_ascii_case(PROGID)),current_progid:current})
        }).collect()
    }
}
#[derive(Clone)]
struct Edit {key:String,name:String,value:Option<RawValue>}
fn edit(key:impl Into<String>,name:impl Into<String>,value:Option<RawValue>)->Edit{Edit{key:key.into(),name:name.into(),value}}
fn selections(extensions:Vec<String>)->Result<BTreeSet<String>,String>{
    if extensions.len()>EXTENSIONS.len(){return Err("文件类型数量超出支持范围".into());}
    let mut selected=BTreeSet::new();
    for ext in extensions {
        let ext=ext.to_ascii_lowercase();
        if !EXTENSIONS.contains(&ext.as_str()){return Err(format!("不支持注册此文件类型：{ext}"));}
        selected.insert(ext);
    }
    Ok(selected)
}
fn open_command(exe:&str)->Result<String,String>{
    let path=Path::new(exe);
    if exe.contains(['\0','\r','\n','"']) || !path.is_absolute()
        || !path.extension().and_then(|v|v.to_str()).is_some_and(|v|v.eq_ignore_ascii_case("exe")) {
        return Err("程序路径不能用于文件关联".into());
    }
    Ok(format!("\"{exe}\" \"%1\""))
}
fn registration_edits(reg:&Registry,selected:&BTreeSet<String>,exe:&str)->Result<Vec<Edit>,String>{
    let command=open_command(exe)?;
    let progid=format!(r"Software\Classes\{PROGID}");
    let owns=string_data(reg.read(OWNER_KEY,"ManagedBy")?).as_deref()==Some(OWNER);
    if !owns {
        if selected.is_empty(){return Ok(Vec::new());}
        for (key,name) in [
            (progid.clone(),""), (format!(r"{progid}\DefaultIcon"),""),
            (format!(r"{progid}\shell\open\command"),""),
            (CAPABILITIES.into(),"ApplicationName"),(CAPABILITIES.into(),"ApplicationDescription"),
            (CAPABILITIES.into(),"ApplicationIcon"),(OWNER_KEY.into(),"ManagedBy"),
            (r"Software\RegisteredApplications".into(),APP_NAME),
        ] {
            if reg.read(&key,name)?.is_some(){return Err("发现同名但未标记归属的注册，已保留原设置，未覆盖".into());}
        }
        for ext in EXTENSIONS {
            if reg.read(&format!(r"Software\Classes\{ext}\OpenWithProgids"),PROGID)?.is_some()
                || reg.read(&format!(r"{CAPABILITIES}\FileAssociations"),ext)?.is_some() {
                return Err("发现同名但未标记归属的文件类型注册，已保留原设置".into());
            }
        }
    }
    let mut edits=Vec::new();
    for ext in EXTENSIONS {
        let want=selected.contains(*ext);
        let openwith=format!(r"Software\Classes\{ext}\OpenWithProgids");
        let caps=format!(r"{CAPABILITIES}\FileAssociations");
        if want || owns {
            edits.push(edit(openwith,PROGID,want.then(||RawValue{kind:REG_NONE,data:vec![]})));
            // A foreign value stored under our capability key is never removed.
            if want || string_data(reg.read(&caps,ext)?).as_deref()==Some(PROGID) {
                edits.push(edit(caps,*ext,want.then(||string_value(PROGID))));
            }
        }
    }
    let keep_handler=!selected.is_empty() || reg.statuses()?.iter().any(|s|s.is_default);
    if keep_handler {
        edits.extend([
            edit(&progid,"",Some(string_value("qing-zip 压缩文件"))),
            edit(format!(r"{progid}\DefaultIcon"),"",Some(string_value(&format!("\"{exe}\",0")))),
            edit(format!(r"{progid}\shell\open\command"),"",Some(string_value(&command))),
            edit(OWNER_KEY,"ManagedBy",Some(string_value(OWNER))),
        ]);
    }else if owns {
        edits.extend([edit(&progid,"",None),edit(format!(r"{progid}\DefaultIcon"),"",None),edit(format!(r"{progid}\shell\open\command"),"",None)]);
    }
    if !selected.is_empty() {
        edits.extend([
            edit(CAPABILITIES,"ApplicationName",Some(string_value(APP_NAME))),
            edit(CAPABILITIES,"ApplicationDescription",Some(string_value("本地压缩与解压工具"))),
            edit(CAPABILITIES,"ApplicationIcon",Some(string_value(&format!("\"{exe}\",0")))),
            edit(r"Software\RegisteredApplications",APP_NAME,Some(string_value(CAPABILITIES))),
        ]);
    }else if owns {
        edits.extend([edit(CAPABILITIES,"ApplicationName",None),edit(CAPABILITIES,"ApplicationDescription",None),edit(CAPABILITIES,"ApplicationIcon",None)]);
        if string_data(reg.read(r"Software\RegisteredApplications",APP_NAME)?).as_deref()==Some(CAPABILITIES) {
            edits.push(edit(r"Software\RegisteredApplications",APP_NAME,None));
        }
    }
    Ok(edits)
}
fn apply_edits(reg:&Registry,edits:Vec<Edit>,fail_after:Option<usize>)->Result<(),String>{
    let mut applied:Vec<(Edit,Option<RawValue>)>=Vec::new();
    let result=(||{
        for (index,entry) in edits.into_iter().enumerate() {
            if fail_after==Some(index){return Err("injected registration failure".to_string());}
            let before=reg.read(&entry.key,&entry.name)?;
            reg.write(&entry.key,&entry.name,entry.value.as_ref())?;
            applied.push((entry,before));
        }
        Ok(())
    })();
    if let Err(error)=result {
        let mut rollback_failed=false;
        for (entry,before) in applied.iter().rev() {
            // Do not overwrite changes made by another process during rollback.
            if reg.read(&entry.key,&entry.name).ok()==Some(entry.value.clone()) {
                rollback_failed|=reg.write(&entry.key,&entry.name,before.as_ref()).is_err();
            }else{rollback_failed=true;}
        }
        return Err(if rollback_failed{format!("{error}；部分注册无法还原，请刷新检查状态")}else{format!("{error}；已还原本次注册值")});
    }
    Ok(())
}
#[tauri::command]
pub fn association_status()->Result<Vec<AssociationStatus>,String>{Registry::user().statuses()}
#[tauri::command]
pub fn set_file_associations(extensions:Vec<String>)->Result<Vec<AssociationStatus>,String>{
    let _lock=REGISTRATION.lock().map_err(|_|"文件关联操作不可用")?;
    let selected=selections(extensions)?;
    let exe=std::env::current_exe().map_err(|e|e.to_string())?;
    let exe=exe.to_str().ok_or("程序路径不是有效Unicode")?;
    let reg=Registry::user();
    let edits=registration_edits(&reg,&selected,exe)?;
    apply_edits(&reg,edits,None)?;
    unsafe{SHChangeNotify(SHCNE_ASSOCCHANGED,SHCNF_IDLIST,None,None);}
    reg.statuses()
}
#[tauri::command]
pub fn open_default_apps()->Result<(),String>{
    let uri=wide("ms-settings:defaultapps?registeredAppUser=Open%20Archive")?;
    let verb=wide("open")?;
    let result=unsafe{ShellExecuteW(None,PCWSTR(verb.as_ptr()),PCWSTR(uri.as_ptr()),PCWSTR::null(),PCWSTR::null(),SW_SHOWNORMAL)};
    if result.0 as isize<=32 {Err("无法打开 Windows 默认应用设置".into())}else{Ok(())}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64,Ordering};
    struct TestRegistry(Registry);
    impl TestRegistry {
        fn new()->Self{static NEXT:AtomicU64=AtomicU64::new(0);Self(Registry{prefix:format!(r"Software\OpenArchiveTests\Associations-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))})}
    }
    impl Drop for TestRegistry{fn drop(&mut self){let key=wide(&self.0.prefix).unwrap();unsafe{let _=RegDeleteTreeW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()));}}}
    #[test]fn invalid_extensions_and_unquoted_paths_are_rejected(){
        assert!(selections(vec![".zip\\shell".into()]).is_err());
        assert!(selections(vec![".exe".into()]).is_err());
        assert!(open_command("relative.exe").is_err());
        assert!(open_command("C:\\bad\"name.exe").is_err());
        assert_eq!(open_command("C:\\Program Files\\qing-zip\\qing-zip.exe").unwrap(),"\"C:\\Program Files\\qing-zip\\qing-zip.exe\" \"%1\"");
    }
    #[test]fn registration_preserves_other_defaults_and_candidates(){
        let t=TestRegistry::new();let reg=&t.0;
        reg.write(r"Software\Classes\.zip","",Some(&string_value("Other.Archive"))).unwrap();
        reg.write(r"Software\Classes\.zip\OpenWithProgids","Other.Archive",Some(&RawValue{kind:REG_NONE,data:vec![]})).unwrap();
        let choice=r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.zip\UserChoice";
        reg.write(choice,"ProgId",Some(&string_value("User.Selected"))).unwrap();
        reg.write(choice,"Hash",Some(&string_value("unchanged hash"))).unwrap();
        let chosen=selections(vec![".zip".into(),".7z".into()]).unwrap();
        apply_edits(reg,registration_edits(reg,&chosen,r"C:\qing-zip.exe").unwrap(),None).unwrap();
        let statuses=reg.statuses().unwrap();let zip=statuses.iter().find(|s|s.extension==".zip").unwrap();
        assert!(zip.registered);assert!(!zip.is_default);assert_eq!(zip.current_progid.as_deref(),Some("User.Selected"));
        apply_edits(reg,registration_edits(reg,&BTreeSet::new(),r"C:\qing-zip.exe").unwrap(),None).unwrap();
        assert!(reg.read(r"Software\Classes\.zip\OpenWithProgids",PROGID).unwrap().is_none());
        assert!(reg.read(r"Software\Classes\.zip\OpenWithProgids","Other.Archive").unwrap().is_some());
        assert_eq!(string_data(reg.read(choice,"Hash").unwrap()).as_deref(),Some("unchanged hash"));
        assert_eq!(string_data(reg.read(r"Software\Classes\.zip","").unwrap()).as_deref(),Some("Other.Archive"));
    }
    #[test]fn partial_failure_rolls_back_only_our_values(){
        let t=TestRegistry::new();let reg=&t.0;
        reg.write(r"Software\RegisteredApplications","Other",Some(&string_value("keep"))).unwrap();
        let edits=registration_edits(reg,&selections(vec![".zip".into(),".7z".into()]).unwrap(),r"C:\qing-zip.exe").unwrap();
        assert!(apply_edits(reg,edits,Some(3)).is_err());
        assert!(reg.read(r"Software\Classes\.zip\OpenWithProgids",PROGID).unwrap().is_none());
        assert!(reg.read(r"Software\Classes\.7z\OpenWithProgids",PROGID).unwrap().is_none());
        assert_eq!(string_data(reg.read(r"Software\RegisteredApplications","Other").unwrap()).as_deref(),Some("keep"));
    }
    #[test]fn deregistration_keeps_handler_when_windows_still_uses_it(){
        let t=TestRegistry::new();let reg=&t.0;
        let chosen=selections(vec![".zip".into()]).unwrap();
        apply_edits(reg,registration_edits(reg,&chosen,r"C:\qing-zip.exe").unwrap(),None).unwrap();
        reg.write(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.zip\UserChoice","ProgId",Some(&string_value(PROGID))).unwrap();
        apply_edits(reg,registration_edits(reg,&BTreeSet::new(),r"C:\qing-zip.exe").unwrap(),None).unwrap();
        assert!(reg.read(&format!(r"Software\Classes\{PROGID}\shell\open\command"),"").unwrap().is_some());
        assert!(reg.statuses().unwrap().iter().find(|s|s.extension==".zip").unwrap().is_default);
    }
    #[test]fn unowned_registration_is_never_overwritten(){
        let t=TestRegistry::new();let reg=&t.0;
        reg.write(&format!(r"Software\Classes\{PROGID}"),"",Some(&string_value("Existing owner"))).unwrap();
        assert!(registration_edits(reg,&selections(vec![".zip".into()]).unwrap(),r"C:\qing-zip.exe").is_err());
        assert!(registration_edits(reg,&BTreeSet::new(),r"C:\qing-zip.exe").unwrap().is_empty());
        assert_eq!(string_data(reg.read(&format!(r"Software\Classes\{PROGID}"),"").unwrap()).as_deref(),Some("Existing owner"));
    }
}
