use super::*;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{CommitTransaction, CreateTransaction}};

const CLSID: &str = "{B1031E9C-F2AA-4658-8932-E2596415A942}";
const CLASS: &str = r"Software\Classes\CLSID\{B1031E9C-F2AA-4658-8932-E2596415A942}";
const VERB: &str = r"Software\Classes\*\shell\qing-zip-compress-files";
const OWNER_VALUE: &str = "QingZipExecutable";

fn server_command(exe: &str) -> Result<String, String> {
    Ok(format!("{} --shell-compress-server", quoted_executable(exe)?))
}

pub(super) fn enabled_for(reg: &Registry, exe: &str) -> Result<bool, String> {
    for (key, name, value) in [
        (CLASS.to_owned(), OWNER_VALUE, exe.to_owned()),
        (VERB.to_owned(), OWNER_VALUE, exe.to_owned()),
        (format!(r"{CLASS}\LocalServer32"), "", server_command(exe)?),
        (format!(r"{VERB}\command"), "DelegateExecute", CLSID.to_owned()),
    ] {
        if reg.read(&key, name)? != Some(string_value(&value)) { return Ok(false); }
    }
    Ok(true)
}

struct Transaction<'a> {
    reg: &'a Registry,
    transaction: OwnedHandle,
}

impl<'a> Transaction<'a> {
    fn start(reg: &'a Registry) -> Result<Self, String> {
        let handle = unsafe { CreateTransaction(std::ptr::null_mut(), std::ptr::null_mut(), 0, 0, 0, 0, PCWSTR::null()) }
            .map_err(|e| e.to_string())?;
        Ok(Self { reg, transaction: unsafe { OwnedHandle::from_raw_handle(handle.0) } })
    }

    fn handle(&self) -> HANDLE { HANDLE(self.transaction.as_raw_handle()) }

    fn open(&self, path: &str, create: bool) -> Result<Option<Key>, String> {
        let name = wide(&self.reg.path(path))?;
        let mut key = HKEY::default();
        let status = unsafe {
            if create {
                RegCreateKeyTransactedW(HKEY_CURRENT_USER, PCWSTR(name.as_ptr()), None, PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE, KEY_READ | KEY_WRITE, None, &mut key, None, self.handle(), None)
            } else {
                RegOpenKeyTransactedW(HKEY_CURRENT_USER, PCWSTR(name.as_ptr()), None,
                    KEY_READ | KEY_WRITE, &mut key, self.handle(), None)
            }
        };
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND { return Ok(None); }
        status.ok().map_err(|e| e.to_string())?;
        Ok(Some(Key(key)))
    }

    fn remove_if_empty(&self, path: &str) -> Result<(), String> {
        let Some(key) = self.open(path, false)? else { return Ok(()); };
        let (mut children, mut values) = (0, 0);
        unsafe { RegQueryInfoKeyW(key.0, None, None, None, Some(&mut children), None, None,
            Some(&mut values), None, None, None, None) }.ok().map_err(|e| e.to_string())?;
        drop(key);
        if children == 0 && values == 0 {
            let path = wide(&self.reg.path(path))?;
            unsafe { RegDeleteKeyTransactedW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, None,
                self.handle(), None) }.ok().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub(super) fn update(reg: &Registry, exe: &str, enable: bool) -> Result<(), String> {
    let tx = Transaction::start(reg)?;
    let server = format!(r"{CLASS}\LocalServer32");
    let command = format!(r"{VERB}\command");
    let owner_name = wide(OWNER_VALUE)?;
    let mut previous = None;
    for root in [CLASS, VERB] {
        if let Some(key) = tx.open(root, false)? {
            let owner = string_data(query_value(&key, &owner_name)?);
            if !enable && owner.as_deref() != Some(exe) { return Ok(()); }
            let owner = owner.ok_or("此右键入口已有其他注册")?;
            if previous.as_ref().is_some_and(|path| path != &owner) {
                return Err("右键入口的注册来自不同安装目录".into());
            }
            previous = Some(owner);
        } else if !enable {
            return Ok(());
        }
    }
    let old = previous.as_deref().unwrap_or(exe);
    if !enable {
        for (path, name, expected) in [(server.as_str(), "", server_command(exe)?),
                                      (command.as_str(), "DelegateExecute", CLSID.into())] {
            let Some(key) = tx.open(path, false)? else { return Ok(()); };
            if query_value(&key, &wide(name)?)? != Some(string_value(&expected)) { return Ok(()); }
        }
    }
    let values = [
        (CLASS, OWNER_VALUE, old.to_owned(), exe.to_owned()),
        (server.as_str(), "", server_command(old)?, server_command(exe)?),
        (VERB, OWNER_VALUE, old.to_owned(), exe.to_owned()),
        (VERB, "MUIVerb", "用 qing-zip 压缩…".into(), "用 qing-zip 压缩…".into()),
        (VERB, "Icon", format!("{},0", quoted_executable(old)?), format!("{},0", quoted_executable(exe)?)),
        (VERB, "MultiSelectModel", "Player".into(), "Player".into()),
        (command.as_str(), "DelegateExecute", CLSID.into(), CLSID.into()),
    ];
    for (path, name, before, after) in values {
        let Some(key) = tx.open(path, enable)? else { continue; };
        let name = wide(name)?;
        let value = query_value(&key, &name)?;
        if value.is_some() && value != Some(string_value(&before)) {
            if enable { return Err("已有其他程序或用户修改过此右键入口".into()); }
            continue;
        }
        if enable {
            let bytes = string_value(&after);
            unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, bytes.kind, Some(&bytes.data)) }
                .ok().map_err(|e| e.to_string())?;
        } else if value.is_some() {
            unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) }.ok().map_err(|e| e.to_string())?;
        }
    }
    if !enable {
        for path in [server.as_str(), CLASS, command.as_str(), VERB] { tx.remove_if_empty(path)?; }
    }
    unsafe { CommitTransaction(tx.handle()) }.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn file_compression_menu_enabled() -> Result<bool, String> {
    enabled_for(&Registry::user(), &executable_path()?)
}

#[tauri::command]
pub fn set_file_compression_menu(enabled: bool) -> Result<bool, String> {
    let exe = executable_path()?;
    let reg = Registry::user();
    update(&reg, &exe, enabled)?;
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None); }
    enabled_for(&reg, &exe)
}
