//! Per-user candidate registration. Windows owns the final default selection.
//! Never writes Explorer FileExts/UserChoice or any extension's default value.
pub(crate) mod shortcuts;
use serde::Serialize;
use std::{collections::BTreeSet, path::Path};
use windows::{core::PCWSTR, Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND},
    System::Registry::*,
    UI::{Shell::{ShellExecuteW, SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST}, WindowsAndMessaging::SW_SHOWNORMAL},
}};

mod registration_lock;
pub(crate) use registration_lock::registration_lock;

const PROGID: &str = "OpenArchive.Archive";
const APP_NAME: &str = "qing-zip";
const FOLDER_MENU: &str = r"Software\Classes\Directory\shell\qing-zip-compress";
pub(crate) mod file_menu;
pub(crate) use file_menu::file_compression_menu_enabled;

fn executable_path() -> Result<String, String> {
    std::env::current_exe().map_err(|e| e.to_string())?
        .into_os_string().into_string().map_err(|_| "程序路径不是有效Unicode".into())
}

#[tauri::command]
pub fn folder_menu_enabled() -> Result<bool,String> {
    Registry::user().owns_folder_menu(&handler_command(&executable_path()?, " --compress-folder")?)
}
#[tauri::command]
pub fn set_folder_menu(enabled: bool) -> Result<bool,String> {
    let _lock=registration_lock()?;
    let reg = Registry::user();
    let exe = executable_path()?;
    let command = handler_command(&exe, " --compress-folder")?;
    if enabled {
        apply_edits(&reg,vec![
            edit(FOLDER_MENU,"",Some(string_value("用 qing-zip 压缩…"))),
            edit(FOLDER_MENU,"Icon",Some(string_value(&format!("\"{exe}\",0")))),
            edit(FOLDER_MENU,"MultiSelectModel",Some(string_value("Single"))),
            edit(format!(r"{FOLDER_MENU}\command"),"",Some(string_value(&command))),
        ],None)?;
    } else if reg.owns_folder_menu(&command)? {
        let key=wide(FOLDER_MENU)?;
        let result=unsafe{RegDeleteTreeW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()))};
        if result!=ERROR_FILE_NOT_FOUND && result!=ERROR_PATH_NOT_FOUND {result.ok().map_err(|e|e.to_string())?;}
    }
    unsafe{SHChangeNotify(SHCNE_ASSOCCHANGED,SHCNF_IDLIST,None,None);}
    folder_menu_enabled()
}
const CAPABILITIES: &str = r"Software\OpenArchive\Capabilities";
const OWNER_KEY: &str = r"Software\OpenArchive\Associations";
const OWNER: &str = "OpenArchive.Associations.v1";
const EXTENSIONS: &[&str] = &[".zip", ".7z", ".rar", ".zipx", ".tar", ".gz", ".gzip", ".bz2", ".bzip2", ".xz", ".tgz", ".tbz", ".tbz2", ".txz", ".iso", ".cab", ".wim", ".lzh", ".lha", ".ace", ".egg", ".alz", ".zst", ".asar", ".br", ".lz4", ".lz5", ".liz", ".lizard", ".brotli", ".zstd", ".aes", ".lz", ".uu", ".uue", ".xxe", ".pma", ".zpaq", ".daa", ".isz", ".wasm", ".romfs", ".ufs", ".ufs2", ".zealfs"];

pub(crate) fn file_dialog_extensions() -> Vec<&'static str> {
    EXTENSIONS.iter().map(|ext| &ext[1..])
        .chain(["exe", "001", "z01", "img", "udf", "dmg", "arj", "deb", "rpm", "vhd", "vhdx", "msi"])
        .collect()
}

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
    query_value(&key, &name)
}
fn query_value(key: &Key, name: &[u16]) -> Result<Option<RawValue>, String> {
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
    fn advertised(&self) -> Result<bool, String> {
        Ok(string_data(self.read(r"Software\RegisteredApplications", APP_NAME)?)
            .as_deref() == Some(CAPABILITIES))
    }
    fn owns_folder_menu(&self, command: &str) -> Result<bool, String> {
        Ok(string_data(self.read(&format!(r"{FOLDER_MENU}\command"), "")?)
            .as_deref() == Some(command))
    }
    fn default_apps_uri(&self) -> Result<String, String> {
        let page = "ms-settings:defaultapps";
        if self.advertised()? {
            Ok(format!("{page}?registeredAppUser={APP_NAME}"))
        } else {
            Ok(page.into())
        }
    }
    fn statuses(&self)->Result<Vec<AssociationStatus>,String>{
        let handler=string_data(self.read(&format!(r"Software\Classes\{PROGID}\shell\open\command"),"")?).is_some();
        let advertised=self.advertised()?;
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
    handler_command(exe, "")
}
fn handler_command(exe:&str, arguments:&str)->Result<String,String>{
    Ok(format!("{}{arguments} \"%1\"", quoted_executable(exe)?))
}
fn quoted_executable(exe: &str) -> Result<String, String> {
    let path=Path::new(exe);
    if exe.contains(['\0','\r','\n','"']) || !path.is_absolute()
        || !path.extension().and_then(|v|v.to_str()).is_some_and(|v|v.eq_ignore_ascii_case("exe")) {
        return Err("程序路径不能用于文件关联".into());
    }
    Ok(format!("\"{exe}\""))
}
fn handler_values(progid: &str, exe: &str, command: &str) -> [Edit; 4] {
    [
        edit(progid,"",Some(string_value("qing-zip 压缩文件"))),
        edit(format!(r"{progid}\DefaultIcon"),"",Some(string_value(&format!("\"{exe}\",0")))),
        edit(format!(r"{progid}\shell\open\command"),"",Some(string_value(command))),
        edit(OWNER_KEY,"ManagedBy",Some(string_value(OWNER))),
    ]
}
fn capability_values(exe: &str) -> [Edit; 4] {
    [
        edit(CAPABILITIES,"ApplicationName",Some(string_value(APP_NAME))),
        edit(CAPABILITIES,"ApplicationDescription",Some(string_value("本地压缩与解压工具"))),
        edit(CAPABILITIES,"ApplicationIcon",Some(string_value(&format!("\"{exe}\",0")))),
        edit(r"Software\RegisteredApplications",APP_NAME,Some(string_value(CAPABILITIES))),
    ]
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
        edits.extend(handler_values(&progid, exe, &command));
    }else if owns {
        edits.extend([edit(&progid,"",None),edit(format!(r"{progid}\DefaultIcon"),"",None),edit(format!(r"{progid}\shell\open\command"),"",None)]);
    }
    if !selected.is_empty() {
        edits.extend(capability_values(exe));
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
fn owned_values(progid: &str, exe: &str, command: &str) -> Vec<Edit> {
    let mut values = Vec::new();
    for ext in EXTENSIONS {
        values.push(edit(
            format!(r"Software\Classes\{ext}\OpenWithProgids"),
            PROGID,
            Some(RawValue {
                kind: REG_NONE,
                data: vec![],
            }),
        ));
        values.push(edit(
            format!(r"{CAPABILITIES}\FileAssociations"),
            *ext,
            Some(string_value(PROGID)),
        ));
    }
    values.extend(capability_values(exe));
    values.extend(handler_values(progid, exe, command));
    values
}

enum RetireError {
    Unavailable(String),
    Other(String),
}
impl From<String> for RetireError {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}
impl From<windows::core::Error> for RetireError {
    fn from(error: windows::core::Error) -> Self {
        use windows::Win32::Foundation::{
            ERROR_CALL_NOT_IMPLEMENTED, ERROR_NOT_SUPPORTED, ERROR_RM_NOT_ACTIVE,
            ERROR_TRANSACTIONS_UNSUPPORTED_REMOTE,
        };
        if [
            ERROR_RM_NOT_ACTIVE,
            ERROR_NOT_SUPPORTED,
            ERROR_CALL_NOT_IMPLEMENTED,
            ERROR_TRANSACTIONS_UNSUPPORTED_REMOTE,
        ]
        .iter()
        .any(|code| code.to_hresult() == error.code())
        {
            Self::Unavailable(error.to_string())
        } else {
            Self::Other(error.to_string())
        }
    }
}

// Keep the key handle from comparison to deletion. A path substituted with a
// new key cannot cause us to delete or later restore a value in that new key.
fn remove_exact(reg: &Registry, entry: &Edit) -> Result<Option<(Key, Vec<u16>, RawValue)>, String> {
    let path = wide(&reg.path(&entry.key))?;
    let name = wide(&entry.name)?;
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut key,
        )
    };
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(None);
    }
    status.ok().map_err(|e| e.to_string())?;
    let key = Key(key);
    let Some(before) = query_value(&key, &name)? else {
        return Ok(None);
    };
    if Some(&before) != entry.value.as_ref() {
        return Ok(None);
    }
    let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    status.ok().map_err(|e| e.to_string())?;
    Ok(Some((key, name, before)))
}

// On a registry without active transaction support, cooperating qing-zip
// processes still share the per-user mutex. Recheck both guard values and the
// exact data at each write. Retire the handler and marker last, so a failed
// attempt can restore its changes without reviving a superseded handler.
fn retire_without_transaction(
    reg: &Registry,
    exe: &str,
    mut before_delete: impl FnMut(usize) -> Result<(), String>,
) -> Result<bool, String> {
    let progid = format!(r"Software\Classes\{PROGID}");
    let handler = format!(r"{progid}\shell\open\command");
    let command = open_command(exe)?;
    let owner_value = Some(string_value(OWNER));
    let open_value = Some(string_value(&command));
    let still_owned = |removed_handler: bool| -> Result<bool, String> {
        let live = reg.read(&handler, "")?;
        Ok(reg.read(OWNER_KEY, "ManagedBy")? == owner_value
            && if removed_handler {
                live.is_none()
            } else {
                live == open_value
            })
    };
    if !still_owned(false)? {
        return Ok(false);
    }
    let mut removed = Vec::new();
    let mut removed_handler = false;
    let result = (|| {
        for entry in owned_values(&progid, exe, &command) {
            if reg.read(&entry.key, &entry.name)? != entry.value {
                continue;
            }
            before_delete(removed.len())?;
            if !still_owned(removed_handler)? {
                return Err("文件关联已被接管".to_owned());
            }
            let Some(backup) = remove_exact(reg, &entry)? else {
                continue;
            };
            if entry.key == handler {
                removed_handler = true;
            }
            removed.push((entry.key == handler, backup));
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut restored = true;
        for (is_handler, (key, name, before)) in removed.iter().rev() {
            if !still_owned(removed_handler).unwrap_or(false) {
                restored = false;
                break;
            }
            if query_value(key, name).ok() != Some(None) {
                restored = false;
                continue;
            }
            if unsafe {
                RegSetValueExW(
                    key.0,
                    PCWSTR(name.as_ptr()),
                    None,
                    before.kind,
                    Some(&before.data),
                )
            }
            .is_err()
            {
                restored = false;
                break;
            }
            if *is_handler {
                removed_handler = false;
            }
        }
        return Err(if restored {
            format!("{error}；本次移除已还原")
        } else {
            format!("{error}；保留已变化的注册，部分移除未还原")
        });
    }
    Ok(true)
}
fn uninstall_associations(reg: &Registry, exe: &str) -> Result<bool, String> {
    let _lock = registration_lock()?;
    match retire_transactionally(reg, exe) {
        Ok(changed) => Ok(changed),
        Err(RetireError::Other(error)) => Err(error),
        Err(RetireError::Unavailable(reason)) => retire_without_transaction(reg, exe, |_| Ok(()))
            .map_err(|error| format!("{error} (Windows registry transactions: {reason})")),
    }
}
// Pin the ownership reads and all matching removals to one registry transaction.
// A conflicting registration must never leave mixed candidates or a dead handler.
fn retire_transactionally(reg: &Registry, exe: &str) -> Result<bool, RetireError> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{CommitTransaction, CreateTransaction},
    };

    let command = open_command(exe)?;
    let progid = format!(r"Software\Classes\{PROGID}");
    let tx = unsafe {
        CreateTransaction(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            PCWSTR::null(),
        )
    }?;
    let tx = unsafe { OwnedHandle::from_raw_handle(tx.0) };
    let open = |path: &str, access: REG_SAM_FLAGS| -> Result<Option<Key>, RetireError> {
        let path = wide(&reg.path(path))?;
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyTransactedW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                access,
                &mut key,
                HANDLE(tx.as_raw_handle()),
                None,
            )
        };
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        status.ok()?;
        Ok(Some(Key(key)))
    };
    let handler = format!(r"{progid}\shell\open\command");
    for (path, name, expected) in [
        (OWNER_KEY, "ManagedBy", OWNER),
        (handler.as_str(), "", command.as_str()),
    ] {
        let Some(key) = open(path, KEY_QUERY_VALUE)? else {
            return Ok(false);
        };
        if query_value(&key, &wide(name)?)? != Some(string_value(expected)) {
            return Ok(false);
        }
    }

    // An extension default or Explorer UserChoice is never among the owned values.
    let values = owned_values(&progid, exe, &command);
    for value in values {
        let Some(key) = open(&value.key, KEY_QUERY_VALUE)? else {
            continue;
        };
        let name = wide(&value.name)?;
        if query_value(&key, &name)? == value.value {
            let key = open(&value.key, KEY_SET_VALUE)?.ok_or_else(|| "注册项已改变".to_owned())?;
            unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) }.ok()?;
        }
    }
    unsafe { CommitTransaction(HANDLE(tx.as_raw_handle())) }?;
    Ok(true)
}

pub(crate) fn unregister_for_uninstall() -> Result<(), String> {
    if uninstall_associations(&Registry::user(), &executable_path()?)? {
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None); }
    }
    Ok(())
}

#[tauri::command]
pub fn association_status()->Result<Vec<AssociationStatus>,String>{Registry::user().statuses()}
#[tauri::command]
pub fn set_file_associations(extensions:Vec<String>)->Result<Vec<AssociationStatus>,String>{
    let _lock=registration_lock()?;
    let selected=selections(extensions)?;
    let exe=executable_path()?;
    let reg=Registry::user();
    let edits=registration_edits(&reg,&selected,&exe)?;
    apply_edits(&reg,edits,None)?;
    unsafe{SHChangeNotify(SHCNE_ASSOCCHANGED,SHCNF_IDLIST,None,None);}
    reg.statuses()
}
#[tauri::command]
pub fn open_default_apps()->Result<(),String>{
    let uri=wide(&Registry::user().default_apps_uri()?)?;
    let verb=wide("open")?;
    let result=unsafe{ShellExecuteW(None,PCWSTR(verb.as_ptr()),PCWSTR(uri.as_ptr()),PCWSTR::null(),PCWSTR::null(),SW_SHOWNORMAL)};
    if result.0 as isize<=32 {Err("无法打开 Windows 默认应用设置".into())}else{Ok(())}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64,Ordering};
    pub(super) struct TestRegistry(pub(super) Registry);
    impl TestRegistry {
        pub(super) fn new()->Self{static NEXT:AtomicU64=AtomicU64::new(0);Self(Registry{prefix:format!(r"Software\OpenArchiveTests\Associations-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))})}
    }
    impl Drop for TestRegistry{fn drop(&mut self){let key=wide(&self.0.prefix).unwrap();unsafe{let _=RegDeleteTreeW(HKEY_CURRENT_USER,PCWSTR(key.as_ptr()));}}}
    #[test]fn invalid_extensions_and_unquoted_paths_are_rejected(){
        assert!(selections(vec![".zip\\shell".into()]).is_err());
        assert!(selections(vec![".exe".into()]).is_err());
        assert!(open_command("relative.exe").is_err());
        assert!(open_command("C:\\bad\"name.exe").is_err());
        assert_eq!(open_command("C:\\Program Files\\qing-zip\\qing-zip.exe").unwrap(),"\"C:\\Program Files\\qing-zip\\qing-zip.exe\" \"%1\"");
    }
    #[test]
    fn file_compression_verb_moves_only_on_explicit_enable() {
        let registry = TestRegistry::new();
        let reg = &registry.0;
        let installed = r"C:\Program Files\qing-zip.exe";
        let portable = r"D:\便携\qing-zip.exe";
        assert!(!file_menu::enabled_for(reg, installed).unwrap());
        file_menu::update(reg, installed, true).unwrap();
        assert!(file_menu::enabled_for(reg, installed).unwrap());
        file_menu::update(reg, portable, true).unwrap();
        file_menu::update(reg, installed, false).unwrap();
        assert!(!file_menu::enabled_for(reg, installed).unwrap());
        assert!(file_menu::enabled_for(reg, portable).unwrap());
        file_menu::update(reg, portable, false).unwrap();
        assert!(!file_menu::enabled_for(reg, portable).unwrap());
        assert!(reg.read(r"Software\Classes\*\shell\qing-zip-compress-files\command", "DelegateExecute").unwrap().is_none());
    }
    #[test]
    fn file_compression_registration_preserves_foreign_or_reassigned_verbs() {
        let registry = TestRegistry::new();
        let reg = &registry.0;
        let exe = r"C:\qing-zip.exe";
        let verb = r"Software\Classes\*\shell\qing-zip-compress-files";
        reg.write(verb, "MUIVerb", Some(&string_value("Other"))).unwrap();
        assert!(file_menu::update(reg, exe, true).is_err());
        assert_eq!(string_data(reg.read(verb,"MUIVerb").unwrap()).as_deref(),Some("Other"));
        let other = TestRegistry::new();
        file_menu::update(&other.0, exe, true).unwrap();
        let key = format!(r"{verb}\command");
        other.0.write(&key, "DelegateExecute", Some(&string_value("Foreign"))).unwrap();
        file_menu::update(&other.0, exe, false).unwrap();
        assert_eq!(string_data(other.0.read(&key,"DelegateExecute").unwrap()).as_deref(),Some("Foreign"));
    }
    #[test]
    fn folder_menu_status_tracks_the_executable_registered_by_explorer() {
        let t = TestRegistry::new();
        let reg = &t.0;
        let command = handler_command(r"C:\My Programs\qing-zip.exe", " --compress-folder").unwrap();
        assert_eq!(command, r#""C:\My Programs\qing-zip.exe" --compress-folder "%1""#);
        assert!(!reg.owns_folder_menu(&command).unwrap());
        let key = format!(r"{FOLDER_MENU}\command");
        reg.write(&key, "", Some(&string_value(&command))).unwrap();
        assert!(reg.owns_folder_menu(&command).unwrap());
        let moved = handler_command(r"D:\Portable\qing-zip.exe", " --compress-folder").unwrap();
        assert!(!reg.owns_folder_menu(&moved).unwrap());
    }
    #[test]fn registration_preserves_other_defaults_and_candidates(){
        let t=TestRegistry::new();let reg=&t.0;
        assert_eq!(reg.default_apps_uri().unwrap(), "ms-settings:defaultapps");
        reg.write(r"Software\Classes\.zip","",Some(&string_value("Other.Archive"))).unwrap();
        reg.write(r"Software\Classes\.zip\OpenWithProgids","Other.Archive",Some(&RawValue{kind:REG_NONE,data:vec![]})).unwrap();
        let choice=r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.zip\UserChoice";
        reg.write(choice,"ProgId",Some(&string_value("User.Selected"))).unwrap();
        reg.write(choice,"Hash",Some(&string_value("unchanged hash"))).unwrap();
        let chosen=selections(vec![".zip".into(),".7z".into()]).unwrap();
        apply_edits(reg,registration_edits(reg,&chosen,r"C:\qing-zip.exe").unwrap(),None).unwrap();
        assert_eq!(reg.default_apps_uri().unwrap(), "ms-settings:defaultapps?registeredAppUser=qing-zip");
        let statuses=reg.statuses().unwrap();let zip=statuses.iter().find(|s|s.extension==".zip").unwrap();
        assert!(zip.registered);assert!(!zip.is_default);assert_eq!(zip.current_progid.as_deref(),Some("User.Selected"));
        apply_edits(reg,registration_edits(reg,&BTreeSet::new(),r"C:\qing-zip.exe").unwrap(),None).unwrap();
        assert_eq!(reg.default_apps_uri().unwrap(), "ms-settings:defaultapps");
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
    #[test]
    fn uninstall_clears_own_candidates_but_preserves_foreign_values_and_user_choice() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let exe = r"C:\Program Files\qing-zip\qing-zip.exe";
        let progid = format!(r"Software\Classes\{PROGID}");
        let choice = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.zip\UserChoice";
        let chosen = selections(vec![".zip".into(), ".rar".into()]).unwrap();
        apply_edits(reg, registration_edits(reg, &chosen, exe).unwrap(), None).unwrap();
        reg.write(choice, "ProgId", Some(&string_value(PROGID))).unwrap();
        reg.write(choice, "Hash", Some(&string_value("untouched"))).unwrap();
        let openwith = r"Software\Classes\.zip\OpenWithProgids";
        reg.write(openwith, "Another.Archive", Some(&string_value("foreign"))).unwrap();
        reg.write(r"Software\Classes\.zip", "", Some(&string_value("Another.Archive"))).unwrap();
        let caps = format!(r"{CAPABILITIES}\FileAssociations");
        reg.write(&caps, ".rar", Some(&string_value("Other.RAR"))).unwrap();
        reg.write(CAPABILITIES, "ApplicationDescription", Some(&string_value("custom"))).unwrap();

        assert!(uninstall_associations(reg, exe).unwrap());
        assert!(!uninstall_associations(reg, exe).unwrap());
        for (path, name) in [
            (OWNER_KEY, "ManagedBy"), (&progid, ""), (openwith, PROGID), (caps.as_str(), ".zip"),
            (r"Software\Classes\.rar\OpenWithProgids", PROGID), (CAPABILITIES, "ApplicationIcon"),
            (&format!(r"{progid}\DefaultIcon"), ""), (&format!(r"{progid}\shell\open\command"), ""),
            (r"Software\RegisteredApplications", APP_NAME),
        ] { assert_eq!(reg.read(path, name).unwrap(), None, "{path}:{name}"); }
        for (path, name, expected) in [
            (choice, "ProgId", PROGID), (choice, "Hash", "untouched"),
            (openwith, "Another.Archive", "foreign"), (r"Software\Classes\.zip", "", "Another.Archive"),
            (caps.as_str(), ".rar", "Other.RAR"), (CAPABILITIES, "ApplicationDescription", "custom"),
        ] { assert_eq!(reg.read(path, name).unwrap(), Some(string_value(expected))); }
    }
    #[test]
    fn untransacted_retirement_restores_its_changes_on_failure_then_retries() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let exe = r"C:\Installed\qing-zip.exe";
        let selected = selections(vec![".zip".into(), ".rar".into()]).unwrap();
        let registered = registration_edits(reg, &selected, exe).unwrap();
        apply_edits(reg, registered.clone(), None).unwrap();
        assert!(retire_without_transaction(reg, exe, |removed| {
            if removed == 3 {
                Err("injected failure".into())
            } else {
                Ok(())
            }
        })
        .is_err());
        for e in registered {
            assert_eq!(reg.read(&e.key, &e.name).unwrap(), e.value);
        }
        assert!(retire_without_transaction(reg, exe, |_| Ok(())).unwrap());
        assert!(!retire_without_transaction(reg, exe, |_| Ok(())).unwrap());
    }
    #[test]
    fn failed_retirement_does_not_restore_candidates_into_a_replaced_key() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let exe = r"C:\Installed\qing-zip.exe";
        let selected = selections(vec![".zip".to_owned()]).unwrap();
        apply_edits(reg, registration_edits(reg, &selected, exe).unwrap(), None).unwrap();
        let old = r"Software\Classes\.zip\OpenWithProgids";
        let moved = r"Software\Classes\.zip\PreservedOriginal";
        let source = wide(&reg.path(old)).unwrap();
        let target = wide("PreservedOriginal").unwrap();
        let mut renamed = false;
        let result = retire_without_transaction(reg, exe, |count| {
            if count != 1 {
                return Ok(());
            }
            // A different registry editor archives the old key and replaces
            // the path before an unrelated error interrupts our operation.
            unsafe {
                RegRenameKey(
                    HKEY_CURRENT_USER,
                    PCWSTR(source.as_ptr()),
                    PCWSTR(target.as_ptr()),
                )
            }
            .ok()
            .map_err(|e| e.to_string())?;
            reg.write(old, "Another.Application", Some(&string_value("external")))?;
            renamed = true;
            Err("injected failure".into())
        });
        assert!(renamed && result.is_err(), "{result:?}");
        assert_eq!(
            reg.read(old, "Another.Application").unwrap(),
            Some(string_value("external"))
        );
        assert_eq!(
            reg.read(old, PROGID).unwrap(),
            None,
            "must not introduce our candidate into a replacement key"
        );
        assert_eq!(
            reg.read(moved, PROGID).unwrap(),
            Some(RawValue {
                kind: REG_NONE,
                data: vec![]
            })
        );
    }
    #[test]
    fn untransacted_retirement_stops_when_another_installation_takes_over() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let installed = r"C:\Installed\qing-zip.exe";
        let portable = r"D:\Portable\qing-zip.exe";
        let zip = selections(vec![".zip".into()]).unwrap();
        let rar = selections(vec![".rar".into()]).unwrap();
        apply_edits(reg, registration_edits(reg, &zip, installed).unwrap(), None).unwrap();
        let takeover = registration_edits(reg, &rar, portable).unwrap();
        assert!(retire_without_transaction(reg, installed, |removed| {
            if removed == 1 {
                apply_edits(reg, takeover.clone(), None)?;
            }
            Ok(())
        })
        .is_err());
        for e in takeover {
            assert_eq!(reg.read(&e.key, &e.name).unwrap(), e.value);
        }
    }
    #[test]
    fn uninstall_after_settings_deselection_retires_the_default_handler() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let exe = r"C:\Installed\qing-zip.exe";
        let command = format!(r"Software\Classes\{PROGID}\shell\open\command");
        let choice = r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.zip\UserChoice";
        let selected = selections(vec![".zip".into()]).unwrap();
        apply_edits(reg, registration_edits(reg, &selected, exe).unwrap(), None).unwrap();
        reg.write(choice, "ProgId", Some(&string_value(PROGID))).unwrap();
        apply_edits(reg, registration_edits(reg, &BTreeSet::new(), exe).unwrap(), None).unwrap();
        assert_eq!(reg.read(&command, "").unwrap(), Some(string_value(&open_command(exe).unwrap())));
        assert!(uninstall_associations(reg, exe).unwrap());
        assert_eq!(reg.read(&command, "").unwrap(), None);
        assert_eq!(reg.read(choice, "ProgId").unwrap(), Some(string_value(PROGID)));
    }
    #[test]
    fn uninstall_preserves_a_portable_takeover_and_unmarked_registrations() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let installed = r"C:\Installed\qing-zip.exe";
        let portable = r"D:\Portable\qing-zip.exe";
        let chosen = selections(vec![".zip".into(), ".7z".into()]).unwrap();
        apply_edits(reg, registration_edits(reg, &chosen, installed).unwrap(), None).unwrap();
        let edits = registration_edits(reg, &chosen, portable).unwrap();
        apply_edits(reg, edits.clone(), None).unwrap();
        assert!(!uninstall_associations(reg, installed).unwrap());
        for v in edits { assert_eq!(reg.read(&v.key, &v.name).unwrap(), v.value); }
        reg.write(OWNER_KEY, "ManagedBy", Some(&string_value("other"))).unwrap();
        assert!(!uninstall_associations(reg, portable).unwrap());
        assert_eq!(reg.read(r"Software\RegisteredApplications", APP_NAME).unwrap(), Some(string_value(CAPABILITIES)));
    }
    #[test]fn unowned_registration_is_never_overwritten(){
        let t=TestRegistry::new();let reg=&t.0;
        reg.write(&format!(r"Software\Classes\{PROGID}"),"",Some(&string_value("Existing owner"))).unwrap();
        assert!(registration_edits(reg,&selections(vec![".zip".into()]).unwrap(),r"C:\qing-zip.exe").is_err());
        assert!(registration_edits(reg,&BTreeSet::new(),r"C:\qing-zip.exe").unwrap().is_empty());
        assert_eq!(string_data(reg.read(&format!(r"Software\Classes\{PROGID}"),"").unwrap()).as_deref(),Some("Existing owner"));
    }
}
