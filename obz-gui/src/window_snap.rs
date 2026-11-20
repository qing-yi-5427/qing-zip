use serde::Deserialize;
use std::sync::OnceLock;
use tauri::{Emitter, WebviewWindow};
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Gdi::ValidateRect,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Input::KeyboardAndMouse::{
                GetCapture, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TME_NONCLIENT,
                TRACKMOUSEEVENT,
            },
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
            WindowsAndMessaging::*,
        },
    },
};

const CLASS_NAME: PCWSTR = w!("QingZip.MaximizeHitTarget");
const STATE_EVENT: &str = "snap-button-state";
const PARENT_SUBCLASS_ID: usize = 0x515A;

struct Target {
    window: WebviewWindow,
    hover: bool,
    pressed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ButtonBounds {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    viewport_width: f64,
}

// The renderer runs in a child HWND. A sibling above it exposes an actual
// non-client maximize target over the web button, without painting over it.
#[tauri::command]
pub(crate) fn set_snap_button_bounds(
    window: WebviewWindow,
    bounds: ButtonBounds,
) -> Result<(), String> {
    if window.label() != "main" || crate::appearance::windows_build() < 22000 {
        return Ok(());
    }
    let parent = window.hwnd().map_err(|err| err.to_string())?;
    unsafe { place_target(window, parent, bounds) }
}

unsafe fn place_target(
    window: WebviewWindow,
    parent: HWND,
    bounds: ButtonBounds,
) -> Result<(), String> {
    let mut client = RECT::default();
    GetClientRect(parent, &mut client).map_err(|err| err.to_string())?;
    if bounds.viewport_width <= 0.0 || client.right <= 0 {
        return Ok(());
    }
    let scale = f64::from(client.right) / bounds.viewport_width;
    let physical = |value: f64| (value * scale).round() as i32;
    let left = physical(bounds.left).clamp(0, client.right);
    let right = physical(bounds.right).clamp(left, client.right);
    let border = if IsZoomed(parent).as_bool() {
        0
    } else {
        GetSystemMetricsForDpi(SM_CYFRAME, GetDpiForWindow(parent)) + 1
    };
    let top = physical(bounds.top).max(border).clamp(0, client.bottom);
    let bottom = physical(bounds.bottom).clamp(top, client.bottom);
    let existing = FindWindowExW(Some(parent), None, CLASS_NAME, PCWSTR::null()).ok();
    if left == right || top == bottom {
        if let Some(target) = existing {
            let _ = ShowWindow(target, SW_HIDE);
        }
        return Ok(());
    }
    let target = match existing {
        Some(target) => target,
        None => {
            let instance = HINSTANCE(GetModuleHandleW(None).map_err(|err| err.to_string())?.0);
            static REGISTERED: OnceLock<bool> = OnceLock::new();
            if !*REGISTERED.get_or_init(|| {
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(target_proc),
                    lpszClassName: CLASS_NAME,
                    hInstance: instance,
                    ..Default::default()
                }) != 0
            }) {
                return Err("could not register the maximize hit target".into());
            }
            let target = CreateWindowExW(
                WS_EX_NOPARENTNOTIFY,
                CLASS_NAME,
                PCWSTR::null(),
                WS_CHILD,
                left,
                top,
                right - left,
                bottom - top,
                Some(parent),
                None,
                Some(instance),
                None,
            )
            .map_err(|err| err.to_string())?;
            if !SetWindowSubclass(parent, Some(parent_hit_test), PARENT_SUBCLASS_ID, 0).as_bool() {
                let _ = DestroyWindow(target);
                return Err("could not attach the maximize hit target".into());
            }
            let state = Box::new(Target {
                window,
                hover: false,
                pressed: false,
            });
            SetWindowLongPtrW(target, GWLP_USERDATA, Box::into_raw(state) as isize);
            target
        }
    };
    SetWindowPos(
        target,
        Some(HWND_TOP),
        left,
        top,
        right - left,
        bottom - top,
        SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOREDRAW,
    )
    .map_err(|err| err.to_string())
}

unsafe extern "system" fn parent_hit_test(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCHITTEST {
        // Shell queries addressed directly to the top-level window must
        // describe the same target as queries addressed to the child.
        if let Ok(target) = FindWindowExW(Some(hwnd), None, CLASS_NAME, PCWSTR::null()) {
            let mut rect = RECT::default();
            if IsWindowVisible(target).as_bool()
                && GetWindowRect(target, &mut rect).is_ok()
                && contains(rect, lparam)
            {
                return LRESULT(HTMAXBUTTON as isize);
            }
        }
    } else if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(parent_hit_test), id);
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

fn contains(rect: RECT, coords: LPARAM) -> bool {
    let x = i32::from(coords.0 as i16);
    let y = i32::from((coords.0 >> 16) as i16);
    (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
}

unsafe extern "system" fn target_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Target;
    match message {
        WM_NCHITTEST => return LRESULT(HTMAXBUTTON as isize),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let _ = ValidateRect(Some(hwnd), None);
            return LRESULT(0);
        }
        WM_NCDESTROY if !state.is_null() => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(state));
        }
        WM_NCMOUSEMOVE | WM_NCMOUSELEAVE | WM_MOUSEMOVE | WM_NCLBUTTONDOWN | WM_LBUTTONUP
        | WM_NCLBUTTONUP | WM_CANCELMODE | WM_CAPTURECHANGED
            if !state.is_null() =>
        {
            let mut rect = RECT::default();
            let inside = match message {
                WM_MOUSEMOVE | WM_LBUTTONUP => GetClientRect(hwnd, &mut rect).is_ok(),
                WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN | WM_NCLBUTTONUP => {
                    GetWindowRect(hwnd, &mut rect).is_ok()
                }
                _ => false,
            } && contains(rect, lparam);
            let previous = ((*state).hover, (*state).pressed);
            match message {
                WM_NCMOUSEMOVE | WM_MOUSEMOVE => {
                    if !(*state).hover && inside {
                        let _ = TrackMouseEvent(&mut TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_NONCLIENT | TME_LEAVE,
                            hwndTrack: hwnd,
                            ..Default::default()
                        });
                    }
                    (*state).hover = inside;
                }
                WM_NCLBUTTONDOWN if inside => {
                    SetCapture(hwnd);
                    (*state).pressed = true;
                    (*state).hover = true;
                }
                WM_LBUTTONUP | WM_NCLBUTTONUP => {
                    let click = std::mem::take(&mut (*state).pressed) && inside;
                    if GetCapture() == hwnd {
                        let _ = ReleaseCapture();
                    }
                    if click {
                        if let Ok(parent) = GetParent(hwnd) {
                            let message = if IsZoomed(parent).as_bool() {
                                SC_RESTORE
                            } else {
                                SC_MAXIMIZE
                            };
                            let _ = PostMessageW(
                                Some(parent),
                                WM_SYSCOMMAND,
                                WPARAM(message as usize),
                                LPARAM(0),
                            );
                        }
                    }
                    (*state).hover = inside;
                }
                WM_NCMOUSELEAVE if GetCapture() == hwnd => (*state).hover = false,
                _ => {
                    (*state).hover = false;
                    (*state).pressed = false;
                    if message == WM_CANCELMODE && GetCapture() == hwnd {
                        let _ = ReleaseCapture();
                    }
                }
            }
            if previous != ((*state).hover, (*state).pressed) {
                let status = if (*state).hover && (*state).pressed {
                    "pressed"
                } else if (*state).hover {
                    "hover"
                } else {
                    ""
                };
                let _ = (*state).window.emit_to("main", STATE_EVENT, status);
            }
            if message != WM_NCMOUSEMOVE && message != WM_NCMOUSELEAVE {
                return LRESULT(0);
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}
