//! Optional NSIS review for exactly one historical registered upgrade.
use super::{string_data, Registry};
use obz_core::retired_install::Review;
use std::{ffi::OsString, path::Path};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        System::Registry::REG_SZ,
        UI::WindowsAndMessaging::{
            MessageBoxW, IDCANCEL, IDYES, MB_DEFBUTTON2, MB_ICONINFORMATION, MB_OK,
            MB_SETFOREGROUND, MB_YESNO, MB_YESNOCANCEL, MESSAGEBOX_RESULT, MESSAGEBOX_STYLE,
        },
    },
};

const INSTALLER: &str = r"Software\qing-zip\Installer";
const PRODUCT: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip";

pub(crate) fn configure(args: &[OsString]) -> Option<Result<(), String>> {
    matches!(args, [command] if command == "--review-retired-install-files").then(review)
}

fn message(text: &str, buttons: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let wide: Vec<_> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide.as_ptr()),
            w!("qing-zip 旧版文件"),
            buttons | MB_ICONINFORMATION | MB_SETFOREGROUND,
        )
    }
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

    let recovery = review.recovery_path().display();
    let ask = format!(
        "发现 {} 个与 qing-zip 0.12.20 发行内容相同、且新版已不再安装的文件。\n\
         内容相同也可能是你后来放入的文件。你可以逐项保留，或将选中的文件移至恢复目录。\n\n\
         恢复位置（卸载后保留）：\n{recovery}\n\n是否逐项查看？",
        review.candidates().len()
    );
    if message(&ask, MB_YESNO | MB_DEFBUTTON2) != IDYES {
        return Ok(());
    }

    let mut selected = Vec::new();
    let total = review.candidates().len();
    for (index, candidate) in review.candidates().iter().enumerate() {
        let ask = format!(
            "旧版候选文件（{}/{total}）\n{}\n\n\
             要将这一项移至恢复目录吗？选择“否”保留原位置。\n\
             选择“取消”放弃本次全部移动。",
            index + 1,
            candidate.relative_path()
        );
        let answer = message(&ask, MB_YESNOCANCEL | MB_DEFBUTTON2);
        if answer == IDCANCEL || answer.0 == 0 {
            return Ok(());
        }
        if answer == IDYES {
            selected.push(index);
        }
    }
    if selected.is_empty() {
        return Ok(());
    }

    if !registered_v01220(root, &Registry::user())? {
        message("旧版安装登记发生变化，所有文件保留在原位置。", MB_OK);
        return Ok(());
    }
    let report = match review.relocate(&selected) {
        Ok(report) => report,
        Err(error) => {
            message(&format!("无法准备恢复目录，未移动文件。\n{error}"), MB_OK);
            return Err(error);
        }
    };
    let mut summary = String::new();
    for (relative, moved) in report {
        summary.push_str(if moved {
            "已移至恢复目录  "
        } else {
            "保留原位  "
        });
        summary.push_str(relative);
        summary.push('\n');
    }
    message(
        &format!(
            "{summary}\n恢复目录（卸载后保留）：\n{recovery}\n\n\
         文件保留原相对路径。需要恢复时，可将选中的文件移回原位置，避免覆盖现在的文件。"
        ),
        MB_OK,
    );
    Ok(())
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
