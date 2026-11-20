use serde::Serialize;
use std::cell::Cell;
use std::sync::OnceLock;
use tauri::{utils::config::WindowEffectsConfig, utils::WindowEffect, Theme, WebviewWindow};
use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(version: *mut OSVERSIONINFOW) -> i32;
}

pub(crate) fn windows_build() -> u32 {
    static BUILD: OnceLock<u32> = OnceLock::new();
    *BUILD.get_or_init(|| {
        let mut version = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        if unsafe { RtlGetVersion(&mut version) } == 0 {
            version.dwBuildNumber
        } else {
            0
        }
    })
}

#[derive(Serialize)]
pub(crate) struct Appearance {
    dark: bool,
    material: &'static str,
}

thread_local! {
    static MAIN_EFFECT: Cell<Option<(&'static str, bool)>> = const { Cell::new(None) };
}

#[tauri::command]
pub(crate) fn set_appearance(
    window: WebviewWindow,
    theme: String,
    material: String,
) -> Result<Appearance, String> {
    let theme = match theme.as_str() {
        "system" => None,
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => return Err("unknown theme".into()),
    };
    window.set_theme(theme).map_err(|err| err.to_string())?;
    // Read the native window after releasing any explicit override. A WebView's
    // media query can reflect its previous forced profile theme at this point.
    let dark = window.theme().map_err(|err| err.to_string())? == Theme::Dark;
    let (material, effect) = match material.as_str() {
        "acrylic" if windows_build() >= 17763 => ("acrylic", Some(WindowEffect::Acrylic)),
        "mica" if windows_build() >= 22000 => (
            "mica",
            Some(if dark {
                WindowEffect::MicaDark
            } else {
                WindowEffect::MicaLight
            }),
        ),
        "solid" | "mica" | "acrylic" => ("solid", None),
        _ => return Err("unknown material".into()),
    };
    if window.label() == "main" {
        MAIN_EFFECT.with(|last| {
            let next = (material, material == "mica" && dark);
            if last.get() == Some(next) {
                return Ok(());
            }
            // Some Windows 11 releases implement Mica and Acrylic with separate
            // attributes. Clear the old material before installing a different one.
            if effect.is_none() || last.get().is_some_and(|old| old.0 != material) {
                window.set_effects(None).map_err(|err| err.to_string())?;
            }
            if let Some(effect) = effect {
                window
                    .set_effects(WindowEffectsConfig {
                        effects: vec![effect],
                        ..Default::default()
                    })
                    .map_err(|err| err.to_string())?;
            }
            last.set(Some(next));
            Ok::<_, String>(())
        })?;
    }
    Ok(Appearance { dark, material })
}
