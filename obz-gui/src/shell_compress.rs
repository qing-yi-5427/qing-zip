//! Explorer's DelegateExecute verb runs in a short-lived STA, before Tauri or
//! its single-instance plugin starts. COM supplies the entire selection.
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    os::windows::ffi::OsStrExt,
    path::{Component, Path},
    process::{Command, Stdio},
    rc::Rc,
};
use windows::{
    core::{implement, Error, IUnknown, Interface, Ref, Result, BOOL, GUID, PCWSTR},
    Win32::{
        Foundation::{
            CLASS_E_NOAGGREGATION, E_ACCESSDENIED, E_FAIL, E_INVALIDARG, E_POINTER, E_UNEXPECTED,
            POINT,
        },
        System::Com::{
            CoAddRefServerProcess, CoInitializeEx, CoRegisterClassObject, CoReleaseServerProcess,
            CoRevokeClassObject, CoTaskMemFree, CoUninitialize, IClassFactory, IClassFactory_Impl,
            CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, REGCLS_SINGLEUSE,
        },
        UI::{
            Shell::{
                IExecuteCommand, IExecuteCommand_Impl, IObjectWithSelection,
                IObjectWithSelection_Impl, IShellItemArray, SIGDN_FILESYSPATH,
            },
            WindowsAndMessaging::{
                DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage, SetTimer, MSG, WM_TIMER,
            },
        },
    },
};

const CLSID: GUID = GUID::from_u128(0xb1031e9c_f2aa_4658_8932_e2596415a942);
const SWITCH: &str = "--compress-files";
type Launch = fn(&[String]) -> Result<()>;

fn selected_paths(items: &IShellItemArray) -> Result<Vec<String>> {
    let count = unsafe { items.GetCount()? };
    if !(1..=128).contains(&count) {
        return Err(Error::new(E_INVALIDARG, "每次可选择 1 至 128 个普通文件"));
    }
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let item = unsafe { items.GetItemAt(i)? };
        let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        if name.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let copied = unsafe { name.to_string() };
        unsafe { CoTaskMemFree(Some(name.as_ptr().cast())) };
        let text = copied.map_err(|_| Error::new(E_INVALIDARG, "无效的 Unicode 文件名"))?;
        let path = Path::new(&text);
        if !path.is_absolute()
            || text.contains(['\r', '\n', '"'])
            || path.components().any(
                |part| matches!(part, Component::Normal(s) if s.to_string_lossy().contains(':')),
            )
        {
            return Err(Error::new(E_INVALIDARG, "请选择普通文件路径"));
        }
        // The receiving instance rechecks every file when it queues the group.
        paths.push(
            crate::instance::canonical_archive_path(text)
                .map_err(|e| Error::new(E_INVALIDARG, e))?,
        );
    }
    Ok(paths)
}

fn compression_command(exe: &Path, paths: &[String]) -> Result<Command> {
    // Rust quotes argv[0] and whitespace-bearing arguments for CreateProcessW.
    // A validated file path contains no double quotes or terminal backslash.
    let mut units = exe.as_os_str().encode_wide().count() + 3 + 1 + SWITCH.len();
    for path in paths {
        let quotes = if path.contains([' ', '\t']) { 2 } else { 0 };
        units += 1 + path.encode_utf16().count() + quotes;
    }
    if units > 32767 {
        return Err(Error::new(
            E_INVALIDARG,
            "所选文件的路径总长度超出 Windows 命令行上限，请减少选择",
        ));
    }
    let mut cmd = Command::new(exe);
    cmd.arg(SWITCH)
        .args(paths)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(cmd)
}

fn launch_gui(paths: &[String]) -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = compression_command(&exe, paths)?;
    if !crate::associations::file_compression_menu_enabled().map_err(|e| Error::new(E_FAIL, e))? {
        return Err(Error::new(
            E_ACCESSDENIED,
            "普通文件压缩入口已关闭或归属其他安装",
        ));
    }
    cmd.spawn()?;
    Ok(())
}

// These COM objects are apartment-bound. In particular, the Shell item array
// may be an STA proxy and must not be transferred to arbitrary worker threads.
#[implement(IExecuteCommand, IObjectWithSelection, Agile = false)]
struct Verb {
    selection: RefCell<Option<IShellItemArray>>,
    used: Cell<bool>,
    launch: Launch,
}
impl IObjectWithSelection_Impl for Verb_Impl {
    fn SetSelection(&self, items: Ref<IShellItemArray>) -> Result<()> {
        self.selection.replace(items.cloned());
        self.used.set(false);
        Ok(())
    }
    fn GetSelection(&self, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = std::ptr::null_mut() };
        if iid.is_null() {
            return Err(E_POINTER.into());
        }
        let items = self
            .selection
            .borrow()
            .clone()
            .ok_or_else(|| Error::from_hresult(E_FAIL))?;
        unsafe { items.query(iid, out).ok() }
    }
}
impl IExecuteCommand_Impl for Verb_Impl {
    fn SetKeyState(&self, _: u32) -> Result<()> {
        Ok(())
    }
    fn SetParameters(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn SetPosition(&self, _: &POINT) -> Result<()> {
        Ok(())
    }
    fn SetShowWindow(&self, _: i32) -> Result<()> {
        Ok(())
    }
    fn SetNoShowUI(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
    fn SetDirectory(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn Execute(&self) -> Result<()> {
        let selection = self
            .selection
            .borrow()
            .clone()
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        if self.used.replace(true) {
            return Err(E_UNEXPECTED.into());
        }
        (self.launch)(&selected_paths(&selection)?)
    }
}

fn release_server() {
    // COM suspends further activations atomically when its process reference
    // count reaches zero. Quit only after the final object or server lock.
    if unsafe { CoReleaseServerProcess() } == 0 {
        unsafe { PostQuitMessage(0) };
    }
}
impl Drop for Verb {
    fn drop(&mut self) {
        release_server();
    }
}
#[implement(IClassFactory, Agile = false)]
struct Factory {
    activated: Rc<Cell<bool>>,
    locks: Cell<u32>,
    launch: Launch,
}
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = std::ptr::null_mut() };
        if iid.is_null() {
            return Err(E_POINTER.into());
        }
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        self.activated.set(true);
        unsafe { CoAddRefServerProcess() };
        let object: IExecuteCommand = Verb {
            selection: RefCell::new(None),
            used: Cell::new(false),
            launch: self.launch,
        }
        .into();
        unsafe { object.query(iid, out).ok() }
    }
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            self.locks.set(
                self.locks
                    .get()
                    .checked_add(1)
                    .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?,
            );
            self.activated.set(true);
            unsafe { CoAddRefServerProcess() };
        } else {
            self.locks.set(
                self.locks
                    .get()
                    .checked_sub(1)
                    .ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?,
            );
            release_server();
        }
        Ok(())
    }
}

struct Apartment;
impl Apartment {
    fn open() -> Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct Registration(u32);
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { CoRevokeClassObject(self.0) };
    }
}
struct Timer(usize);
impl Drop for Timer {
    fn drop(&mut self) {
        let _ = unsafe { KillTimer(None, self.0) };
    }
}

fn serve(
    clsid: &GUID,
    launch: Launch,
    idle_ms: u32,
    ready: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _apartment = Apartment::open()?;
    let activated = Rc::new(Cell::new(false));
    let factory: IClassFactory = Factory {
        activated: activated.clone(),
        locks: Cell::new(0),
        launch,
    }
    .into();
    let _registration = Registration(unsafe {
        CoRegisterClassObject(clsid, &factory, CLSCTX_LOCAL_SERVER, REGCLS_SINGLEUSE)?
    });
    let id = unsafe { SetTimer(None, 0, idle_ms, None) };
    if id == 0 {
        return Err(Error::from_thread());
    }
    let _timer = Timer(id);
    ready()?;
    loop {
        let mut message = MSG::default();
        match unsafe { GetMessageW(&mut message, None, 0, 0) }.0 {
            -1 => return Err(Error::from_thread()),
            0 => return Ok(()),
            _ => (),
        }
        if message.hwnd.is_invalid()
            && message.message == WM_TIMER
            && message.wParam.0 == id
            && !activated.get()
        {
            return Ok(());
        }
        unsafe { DispatchMessageW(&message) };
    }
}

#[derive(Debug, PartialEq)]
enum Startup { Server, Unregister }

fn startup_mode(mut args: impl Iterator<Item = std::ffi::OsString>) -> Option<Result<Startup>> {
    let mode = args.next()?;
    if mode == "--unregister-compress-menu" {
        return Some(if args.next().is_none() { Ok(Startup::Unregister) } else { Err(E_INVALIDARG.into()) });
    }
    if mode != "--shell-compress-server" { return None; }
    if args.next().is_some_and(|s| !s.eq_ignore_ascii_case("-Embedding") && !s.eq_ignore_ascii_case("/Embedding"))
        || args.next().is_some() {
        return Some(Err(E_INVALIDARG.into()));
    }
    Some(Ok(Startup::Server))
}

pub fn startup() -> Option<Result<()>> {
    Some(startup_mode(std::env::args_os().skip(1))?.and_then(|mode| match mode {
        Startup::Server => serve(&CLSID, launch_gui, 30000, || Ok(())),
        Startup::Unregister => crate::associations::file_menu::set_file_compression_menu(false)
            .map(|_| ()).map_err(|e| Error::new(E_FAIL, e)),
    }))
}

#[cfg(test)]
mod tests;
