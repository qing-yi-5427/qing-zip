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
    transaction: Option<OwnedHandle>,
}

impl<'a> Transaction<'a> {
    fn start(reg: &'a Registry) -> Result<Self, String> {
        use windows::Win32::Foundation::{ERROR_CALL_NOT_IMPLEMENTED, ERROR_NOT_SUPPORTED,
            ERROR_RM_NOT_ACTIVE, ERROR_TRANSACTIONMANAGER_NOT_ONLINE, ERROR_TRANSACTIONS_UNSUPPORTED_REMOTE};
        let unavailable = |error: &windows::core::Error| {
            [ERROR_CALL_NOT_IMPLEMENTED, ERROR_NOT_SUPPORTED, ERROR_RM_NOT_ACTIVE,
             ERROR_TRANSACTIONMANAGER_NOT_ONLINE, ERROR_TRANSACTIONS_UNSUPPORTED_REMOTE]
                .iter().any(|code| code.to_hresult() == error.code())
        };
        let handle = match unsafe { CreateTransaction(std::ptr::null_mut(), std::ptr::null_mut(), 0, 0, 0, 0, PCWSTR::null()) } {
            Ok(handle) => unsafe { OwnedHandle::from_raw_handle(handle.0) },
            Err(e) if unavailable(&e) => return Ok(Self { reg, transaction: None }),
            Err(e) => return Err(e.to_string()),
        };
        let mut view = Self { reg, transaction: Some(handle) };
        // Creating the transaction can succeed while the HKCU resource manager is offline.
        // Probe both private paths before any edit. Never retry a failed commit nontransactionally.
        for root in [CLASS, VERB] {
            let path = wide(&reg.path(root))?;
            if let Err(e) = view.open_native(&path, false) {
                if unavailable(&e) { view.transaction = None; break; }
                return Err(e.to_string());
            }
        }
        Ok(view)
    }

    fn handle(&self) -> Option<HANDLE> { self.transaction.as_ref().map(|h| HANDLE(h.as_raw_handle())) }

    fn open(&self, path: &str, create: bool) -> Result<Option<Key>, String> {
        self.open_native(&wide(&self.reg.path(path))?, create).map_err(|e| e.to_string())
    }

    fn open_native(&self, path: &[u16], create: bool) -> windows::core::Result<Option<Key>> {
        let name = PCWSTR(path.as_ptr());
        let mut key = HKEY::default();
        let status = unsafe {
            match (self.handle(), create) {
                (Some(tx), true) => RegCreateKeyTransactedW(HKEY_CURRENT_USER, name, None, PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE, KEY_READ | KEY_WRITE, None, &mut key, None, tx, None),
                (Some(tx), false) => RegOpenKeyTransactedW(HKEY_CURRENT_USER, name, None,
                    KEY_READ | KEY_WRITE, &mut key, tx, None),
                (None, false) => RegOpenKeyExW(HKEY_CURRENT_USER, name, None, KEY_READ, &mut key),
                (None, true) => unreachable!("ordinary writes go through apply_edits"),
            }
        };
        if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND { return Ok(None); }
        status.ok()?;
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
            unsafe { match self.handle() {
                Some(tx) => RegDeleteKeyTransactedW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), 0, None, tx, None),
                None => RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())),
            } }.ok().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

pub(super) fn update(reg: &Registry, exe: &str, enable: bool) -> Result<(), String> {
    let _lock = registration_lock()?;
    update_in(Transaction::start(reg)?, exe, enable, None)
}

fn update_in(tx: Transaction<'_>, exe: &str, enable: bool, fail_after: Option<usize>) -> Result<(), String> {
    let server = format!(r"{CLASS}\LocalServer32");
    let command = format!(r"{VERB}\command");
    let owner_name = wide(OWNER_VALUE)?;
    let mut previous = None;
    for root in [CLASS, VERB] {
        if let Some(key) = tx.open(root, false)? {
            let raw = query_value(&key, &owner_name)?;
            let owner = string_data(raw.clone()).filter(|s| raw == Some(string_value(s)));
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
    let paths = [server.as_str(), CLASS, command.as_str(), VERB];
    let mut absent = Vec::new();
    if enable && tx.handle().is_none() {
        for path in paths {
            if tx.open(path, false)?.is_none() { absent.push(path); }
        }
    }
    let mut edits = Vec::new();
    for (path, name, before, after) in values {
        let value = match tx.open(path, false)? {
            Some(key) => query_value(&key, &wide(name)?)?,
            None => None,
        };
        if value.is_some() && value != Some(string_value(&before)) {
            if enable { return Err("已有其他程序或用户修改过此右键入口".into()); }
            continue;
        }
        let desired = enable.then(|| string_value(&after));
        if value != desired {
            let mut change = edit(path, name, desired);
            change.expected = Some(value);
            edits.push(change);
        }
    }
    if !enable { edits.reverse(); }
    if tx.handle().is_some() {
        for change in edits {
            let key = tx.open(&change.key, enable)?.ok_or("右键菜单注册项不存在")?;
            let name = wide(&change.name)?;
            unsafe { match change.value {
                Some(bytes) => RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, bytes.kind, Some(&bytes.data)),
                None => RegDeleteValueW(key.0, PCWSTR(name.as_ptr())),
            } }.ok().map_err(|e| e.to_string())?;
        }
    } else if let Err(error) = apply_edits(tx.reg, edits, fail_after) {
        for path in absent {
            tx.remove_if_empty(path).map_err(|e| format!("{error}；无法清理空注册项：{e}"))?;
        }
        return Err(error);
    }
    if !enable {
        for path in paths { tx.remove_if_empty(path)?; }
    }
    if let Some(handle) = tx.handle() {
        unsafe { CommitTransaction(handle) }.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn file_compression_menu_enabled() -> Result<bool, String> {
    let _lock = registration_lock()?;
    enabled_for(&Registry::user(), &executable_path()?)
}

#[tauri::command]
pub fn set_file_compression_menu(enabled: bool) -> Result<bool, String> {
    let _lock = registration_lock()?;
    let exe = executable_path()?;
    let reg = Registry::user();
    update(&reg, &exe, enabled)?;
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None); }
    enabled_for(&reg, &exe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::associations::tests::TestRegistry;

    #[test]
    fn file_compression_fallback_rolls_back_each_interrupted_write() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let installed = r"C:\Apps\qing-zip.exe";
        let portable = r"D:\Apps\qing-zip.exe";
        let view = || Transaction { reg, transaction: None };
        for index in 0..7 {
            assert!(update_in(view(), installed, true, Some(index)).is_err());
            for path in [CLASS, VERB] {
                assert!(view().open(path, false).unwrap().is_none(), "orphan at {index}: {path}");
            }
        }
        update_in(view(), installed, true, None).unwrap();
        for index in 0..4 {
            assert!(update_in(view(), portable, true, Some(index)).is_err());
            assert!(enabled_for(reg, installed).unwrap());
        }
        for index in 0..7 {
            assert!(update_in(view(), installed, false, Some(index)).is_err());
            assert!(enabled_for(reg, installed).unwrap());
            assert_eq!(reg.read(VERB, "MultiSelectModel").unwrap(), Some(string_value("Player")));
        }
        reg.write(VERB, "Unrelated", Some(&string_value("retained"))).unwrap();
        update_in(view(), installed, false, None).unwrap();
        assert_eq!(reg.read(VERB, "Unrelated").unwrap(), Some(string_value("retained")));
    }

    #[test]
    fn file_compression_fallback_keeps_values_changed_after_preflight() {
        let sandbox = TestRegistry::new();
        let reg = &sandbox.0;
        let mut first = edit(CLASS, OWNER_VALUE, Some(string_value("ours")));
        first.expected = Some(None);
        let mut second = edit(VERB, OWNER_VALUE, Some(string_value("ours")));
        second.expected = Some(None);
        reg.write(VERB, OWNER_VALUE, Some(&string_value("foreign"))).unwrap();
        assert!(apply_edits(reg, vec![first, second], None).is_err());
        assert!(reg.read(CLASS, OWNER_VALUE).unwrap().is_none());
        assert_eq!(reg.read(VERB, OWNER_VALUE).unwrap(), Some(string_value("foreign")));
    }
}
