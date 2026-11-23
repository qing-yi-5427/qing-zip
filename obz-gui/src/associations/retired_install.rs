//! Optional NSIS review for exactly one historical registered upgrade.
use super::{string_data, Registry};
use obz_core::retired_install::Review;
use std::{cell::Cell, ffi::OsString, path::Path};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::Registry::REG_SZ,
        UI::{
            Input::KeyboardAndMouse::{EnableWindow, SetFocus},
            WindowsAndMessaging::{
                DialogBoxIndirectParamW, EndDialog, GetDlgItem, GetWindowLongPtrW,
                SendDlgItemMessageW, SetDlgItemTextW, SetForegroundWindow, SetWindowLongPtrW,
                ShowWindow, BM_GETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, DS_CENTER,
                DS_MODALFRAME, DS_SETFONT, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE,
                ES_READONLY, GWLP_USERDATA, IDCANCEL, IDOK, SW_HIDE, SW_SHOW, WM_CLOSE, WM_COMMAND,
                WM_INITDIALOG, WS_BORDER, WS_CAPTION, WS_CHILD, WS_DISABLED, WS_POPUP, WS_SYSMENU,
                WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
            },
        },
    },
};

const INSTALLER: &str = r"Software\qing-zip\Installer";
const PRODUCT: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip";
const TITLE: &str = "qing-zip 旧版文件";
const INTRO: u16 = 100;
const RESULTS: u16 = 103;
const FIRST_FILE: u16 = 200;
const BUTTON: u16 = 0x80;
const EDIT: u16 = 0x81;
const STATIC: u16 = 0x82;

pub(crate) fn configure(args: &[OsString]) -> Option<Result<(), String>> {
    matches!(args, [command] if command == "--review-retired-install-files").then(review)
}

fn review() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let Some(root) = exe
        .parent()
        .filter(|_| exe.file_name().is_some_and(|name| name == "qing-zip.exe"))
    else {
        return Ok(());
    };
    if !registered_v01220(root, &Registry::user())? {
        return Ok(());
    }
    let review = Review::open(root)?;
    if review.candidates().is_empty() {
        return Ok(());
    }

    let template = dialog_template(&review);
    let state = DialogState {
        root,
        review: &review,
        finished: Cell::new(false),
    };
    // DialogBoxIndirectParamW is synchronous. The template and state live
    // until the window and all notifications have ended.
    let result = unsafe {
        DialogBoxIndirectParamW(
            None,
            template.as_ptr().cast(),
            None,
            Some(dialog_proc),
            LPARAM((&state as *const DialogState<'_>) as isize),
        )
    };
    if result == -1 {
        Err(format!(
            "旧版文件检查窗口无法打开：{}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

struct DialogState<'a> {
    root: &'a Path,
    review: &'a Review,
    finished: Cell<bool>,
}

unsafe extern "system" fn dialog_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if message == WM_INITDIALOG {
        unsafe {
            SetWindowLongPtrW(window, GWLP_USERDATA, lparam.0);
        }
        if let Ok(cancel) = unsafe { GetDlgItem(Some(window), IDCANCEL.0) } {
            unsafe {
                let _ = SetForegroundWindow(window);
                let _ = SetFocus(Some(cancel));
            }
            return 0;
        }
        return 1;
    }
    if message == WM_CLOSE {
        let _ = unsafe { EndDialog(window, 0) };
        return 1;
    }
    if message != WM_COMMAND {
        return 0;
    }
    let id = (wparam.0 & 0xffff) as u16;
    if id == IDCANCEL.0 as u16 {
        let _ = unsafe { EndDialog(window, 0) };
        return 1;
    }
    let state_ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const DialogState<'_>;
    let Some(state) = (unsafe { state_ptr.as_ref() }) else {
        return 0;
    };
    if state.finished.get() {
        return 0;
    }
    let count = state.review.candidates().len();
    if id != IDOK.0 as u16 && !(FIRST_FILE..FIRST_FILE + count as u16).contains(&id) {
        return 0;
    }
    let selected: Vec<_> = (0..count)
        .filter(|&index| unsafe {
            SendDlgItemMessageW(
                window,
                (FIRST_FILE + index as u16) as i32,
                BM_GETCHECK,
                WPARAM(0),
                LPARAM(0),
            )
            .0 == 1
        })
        .collect();
    if id == IDOK.0 as u16 && !selected.is_empty() {
        state.finished.set(true);
        show_result(window, state, &selected);
    } else if let Ok(button) = unsafe { GetDlgItem(Some(window), IDOK.0) } {
        unsafe {
            let _ = EnableWindow(button, !selected.is_empty());
        }
    }
    1
}

fn show_result(window: HWND, state: &DialogState<'_>, selected: &[usize]) {
    let applied = registered_v01220(state.root, &Registry::user()).and_then(|registered| {
        if registered {
            state.review.relocate(selected)
        } else {
            Err("旧版安装登记发生变化，所有文件保留在原位置。".into())
        }
    });
    let text = match applied {
        Ok(report) => report
            .into_iter()
            .map(|(name, result)| match result {
                Ok(()) => format!("已移动  {name}"),
                Err(reason) => format!("未移动  {name}\r\n{reason}"),
            })
            .collect::<Vec<_>>()
            .join("\r\n"),
        Err(reason) => format!("未移动文件。\r\n{reason}"),
    };
    set_text(window, INTRO, "处理结果。未选中的文件保留在原位置。\n已移动的文件保留原相对路径，卸载后可从下方目录恢复。");
    set_text(window, RESULTS, &text);
    for id in
        (FIRST_FILE..FIRST_FILE + state.review.candidates().len() as u16).chain([IDOK.0 as u16])
    {
        if let Ok(control) = unsafe { GetDlgItem(Some(window), id as i32) } {
            unsafe {
                let _ = ShowWindow(control, SW_HIDE);
            }
        }
    }
    set_text(window, IDCANCEL.0 as u16, "关闭");
    if let Ok(results) = unsafe { GetDlgItem(Some(window), RESULTS as i32) } {
        unsafe {
            let _ = ShowWindow(results, SW_SHOW);
            let _ = SetFocus(Some(results));
        }
    }
}

fn set_text(window: HWND, id: u16, text: &str) {
    let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let _ = SetDlgItemTextW(window, id as i32, PCWSTR(wide.as_ptr()));
    }
}

fn dialog_template(review: &Review) -> Vec<u32> {
    let candidates = review.candidates();
    let count = candidates.len() as u16;
    let footer = 65 + count * 13;
    let mut words = Vec::new();
    // Standard DLGTEMPLATE, followed by Unicode menu, class, caption and font.
    dword(
        &mut words,
        (WS_POPUP | WS_CAPTION | WS_SYSMENU).0 | (DS_MODALFRAME | DS_CENTER | DS_SETFONT) as u32,
    );
    dword(&mut words, 0);
    words.extend([count + 6, 0, 0, 356, footer + 26, 0, 0]);
    wide(&mut words, TITLE);
    words.push(9);
    wide(&mut words, "Segoe UI");
    let focusable = WS_TABSTOP.0;
    control(&mut words, INTRO, STATIC, WS_VISIBLE.0, [12, 10, 332, 27],
            "已找到与 qing-zip 0.12.20 发行内容相同的文件。这些文件也可能是你后来放入的。\n只勾选要移至恢复目录的文件。其余文件会保留在原位置。");
    control(
        &mut words,
        101,
        STATIC,
        WS_VISIBLE.0,
        [12, 36, 332, 10],
        "恢复目录（卸载后保留）",
    );
    control(
        &mut words,
        102,
        EDIT,
        (WS_VISIBLE | WS_BORDER).0 | focusable | (ES_READONLY | ES_AUTOHSCROLL) as u32,
        [12, 46, 332, 13],
        &review.recovery_path().display().to_string(),
    );
    for (index, file) in candidates.iter().enumerate() {
        let row = index as u16;
        control(
            &mut words,
            FIRST_FILE + row,
            BUTTON,
            WS_VISIBLE.0 | focusable | BS_AUTOCHECKBOX as u32,
            [12, 65 + row * 13, 332, 12],
            file.relative_path(),
        );
    }
    control(
        &mut words,
        RESULTS,
        EDIT,
        (WS_BORDER | WS_VSCROLL).0
            | focusable
            | (ES_READONLY | ES_MULTILINE | ES_AUTOVSCROLL) as u32,
        [12, 65, 332, count * 13],
        "",
    );
    control(
        &mut words,
        IDOK.0 as u16,
        BUTTON,
        (WS_VISIBLE | WS_DISABLED).0 | focusable,
        [204, footer + 5, 66, 14],
        "移动所选",
    );
    control(
        &mut words,
        IDCANCEL.0 as u16,
        BUTTON,
        WS_VISIBLE.0 | focusable | BS_DEFPUSHBUTTON as u32,
        [278, footer + 5, 66, 14],
        "全部保留",
    );
    while words.len() % 2 != 0 {
        words.push(0);
    }
    // Vec<u32> gives the DWORD alignment Windows requires for the header.
    words
        .chunks_exact(2)
        .map(|pair| u32::from(pair[0]) | (u32::from(pair[1]) << 16))
        .collect()
}

fn control(words: &mut Vec<u16>, id: u16, class: u16, style: u32, bounds: [u16; 4], text: &str) {
    if words.len() % 2 != 0 {
        words.push(0);
    }
    dword(words, style | WS_CHILD.0);
    dword(words, 0);
    words.extend(bounds);
    words.extend([id, 0xffff, class]);
    wide(words, text);
    words.push(0); // no creation data
}

fn dword(words: &mut Vec<u16>, value: u32) {
    words.extend([value as u16, (value >> 16) as u16]);
}

fn wide(words: &mut Vec<u16>, value: &str) {
    words.extend(value.encode_utf16().chain(Some(0)));
}

fn registered_v01220(root: &Path, registry: &Registry) -> Result<bool, String> {
    let read = |key, name| -> Result<Option<String>, String> {
        Ok(string_data(
            registry
                .read(key, name)?
                .filter(|value| value.kind == REG_SZ),
        ))
    };
    let Some(root_text) = root.to_str() else {
        return Ok(false);
    };
    let uninstaller = format!("\"{}\"", root.join("Uninstall.exe").display());
    Ok(
        read(PRODUCT, "DisplayVersion")?.as_deref() == Some("0.12.20")
            && read(INSTALLER, "InstallDir")?.as_deref() == Some(root_text)
            && read(PRODUCT, "InstallLocation")?.as_deref() == Some(root_text)
            && read(PRODUCT, "UninstallString")?.as_deref() == Some(uninstaller.as_str()),
    )
}
