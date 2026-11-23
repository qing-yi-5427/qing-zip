"""Check actual Windows Display > Scale, with no WebView device emulation."""

import base64
import ctypes
from ctypes import wintypes
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import subprocess
import time

from native_menu_accessibility import check_host


class MonitorInfo(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("monitor", wintypes.RECT),
                ("work", wintypes.RECT), ("flags", wintypes.DWORD)]


class DisplayMode(ctypes.Structure):
    # DEVMODEW with the display members of its two unions.
    _fields_ = [
        ("device", wintypes.WCHAR * 32), ("spec_version", wintypes.WORD),
        ("driver_version", wintypes.WORD), ("size", wintypes.WORD), ("driver_extra", wintypes.WORD),
        ("fields", wintypes.DWORD), ("position", wintypes.POINT), ("orientation", wintypes.DWORD),
        ("fixed_output", wintypes.DWORD), ("printing", wintypes.SHORT * 5),
        ("form_name", wintypes.WCHAR * 32), ("log_pixels", wintypes.WORD),
        ("bits", wintypes.DWORD), ("width", wintypes.DWORD), ("height", wintypes.DWORD),
        ("flags", wintypes.DWORD), ("frequency", wintypes.DWORD), ("trailing", wintypes.DWORD * 8),
    ]

    def dimensions(self):
        return [self.width, self.height]


def display_modes():
    check_host()
    user = ctypes.WinDLL("user32", use_last_error=True)
    enumerate_mode = user.EnumDisplaySettingsW
    enumerate_mode.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(DisplayMode)]
    enumerate_mode.restype = wintypes.BOOL
    change = user.ChangeDisplaySettingsW
    change.argtypes = [ctypes.POINTER(DisplayMode), wintypes.DWORD]
    change.restype = wintypes.LONG

    def read(index):
        mode = DisplayMode()
        mode.size = ctypes.sizeof(mode)
        return mode if enumerate_mode(None, index, ctypes.byref(mode)) else None

    current = read(0xFFFFFFFF)  # ENUM_CURRENT_SETTINGS
    if not current:
        raise RuntimeError("The primary display graphics mode could not be read")
    offered = {}
    for index in itertools.count():
        mode = read(index)
        if not mode:
            break
        if (mode.bits, mode.frequency, mode.flags) == (current.bits, current.frequency, current.flags):
            offered[tuple(mode.dimensions())] = mode

    def temporary(mode):
        # Bit depth, width, height, display flags and frequency from the driver.
        mode.fields = 0x007C0000
        mode.driver_extra = 0
        result = change(ctypes.byref(mode), 2)  # CDS_TEST
        if result == 0:
            result = change(ctypes.byref(mode), 4)  # CDS_FULLSCREEN (temporary)
        if result != 0:
            raise RuntimeError(f"Windows rejected temporary display mode {mode.dimensions()}: {result}")

    return current, offered, temporary


def window_metrics(pid):
    user = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

    def bind(name, result, *args):
        fn = getattr(user, name)
        fn.argtypes, fn.restype = args, result
        return fn

    enum = bind("EnumWindows", wintypes.BOOL, callback_type, wintypes.LPARAM)
    visible = bind("IsWindowVisible", wintypes.BOOL, wintypes.HWND)
    owner = bind("GetWindowThreadProcessId", wintypes.DWORD, wintypes.HWND, ctypes.POINTER(wintypes.DWORD))
    title = bind("GetWindowTextW", ctypes.c_int, wintypes.HWND, wintypes.LPWSTR, ctypes.c_int)
    dpi = bind("GetDpiForWindow", wintypes.UINT, wintypes.HWND)
    context = bind("GetWindowDpiAwarenessContext", wintypes.HANDLE, wintypes.HWND)
    awareness = bind("GetAwarenessFromDpiAwarenessContext", ctypes.c_int, wintypes.HANDLE)
    monitor = bind("MonitorFromWindow", wintypes.HANDLE, wintypes.HWND, wintypes.DWORD)
    monitor_info = bind("GetMonitorInfoW", wintypes.BOOL, wintypes.HANDLE, ctypes.POINTER(MonitorInfo))
    thread_context = bind("SetThreadDpiAwarenessContext", wintypes.HANDLE, wintypes.HANDLE)
    metrics = bind("GetSystemMetrics", ctypes.c_int, ctypes.c_int)

    def measure():
        windows = []
        titles = []

        @callback_type
        def collect(hwnd, _):
            candidate = wintypes.DWORD()
            owner(hwnd, ctypes.byref(candidate))
            if candidate.value == pid and visible(hwnd) and dpi(hwnd):
                name = ctypes.create_unicode_buffer(256)
                title(hwnd, name, len(name))
                titles.append(name.value)
                if name.value == "qing-zip":
                    windows.append(hwnd)
            return True

        if not enum(collect, 0) or len(windows) != 1:
            raise RuntimeError(f"Expected one titled qing-zip main window; visible package titles: {titles}")
        hwnd = windows[0]
        info = MonitorInfo()
        info.size = ctypes.sizeof(info)
        previous = thread_context(ctypes.c_void_p(-4))  # Per-monitor v2, test thread only.
        if not previous:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            if not monitor_info(monitor(hwnd, 2), ctypes.byref(info)):
                raise ctypes.WinError(ctypes.get_last_error())
            return {"window_dpi": dpi(hwnd), "awareness": awareness(context(hwnd)),
                    "window_title": "qing-zip", "visible_package_windows": len(titles),
                    "monitor_pixels": [info.monitor.right - info.monitor.left, info.monitor.bottom - info.monitor.top],
                    "remote_session": bool(metrics(0x1000)), "primary_monitor": bool(info.flags & 1)}
        finally:
            thread_context(previous)

    return measure


def verify_scale(call, js, output, executable, pid):
    check_host()
    driver = Path(__file__).with_name("display-scale-settings.ps1")
    shell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    measure = window_metrics(pid)
    report = {"status": "FAIL", "platform": platform.platform(), "arch": platform.machine(),
              "gui_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
              "settings_mutation": "Windows Settings Scale combobox via UI Automation",
              "observations": {}, "restored": False, "transition_tested": False}

    def settings(label, percent=0):
        record = output / f"settings-{label}.json"
        image = output / f"settings-{label}.png"
        result = subprocess.run([str(shell), "-NoProfile", "-File", str(driver), "-Receipt", str(record),
                                 "-Screenshot", str(image), "-SelectPercent", str(percent)],
                                capture_output=True, text=True, errors="replace", timeout=55)
        if not record.is_file():
            raise RuntimeError(f"Settings automation emitted no receipt (exit {result.returncode}): {result.stderr[-800:]}")
        observed = json.loads(record.read_text(encoding="utf-8-sig"))
        if image.is_file():
            observed["screenshot_sha256"] = hashlib.sha256(image.read_bytes()).hexdigest()
        report["observations"][f"settings-{label}"] = observed
        return observed

    def snapshot(label, expected_dpi, expected_monitor, capture=True):
        deadline = time.monotonic() + 20
        slot = report["observations"][label] = {}
        while time.monotonic() < deadline:
            native = slot["native"] = measure()
            webview = slot["webview"] = js("({dpr:devicePixelRatio, width:innerWidth, height:innerHeight})")
            if (native["window_dpi"] == expected_dpi and native["awareness"] == 2
                    and native["primary_monitor"] == expected_monitor
                    and abs(webview["dpr"] - expected_dpi / 96) < 0.01):
                break
            time.sleep(0.15)
        else:
            raise RuntimeError(f"{label}: package DPI and WebView DPR did not match {expected_dpi}")
        if not capture:
            return
        call("Page.bringToFront")
        if js("document.querySelector('#oa-menu').hidden"):
            x, y = js("""(() => {
                const rect = document.querySelector('[data-action=app-menu]').getBoundingClientRect();
                return [rect.x + rect.width / 2, rect.y + rect.height / 2];
            })()""")
            for event in ("mousePressed", "mouseReleased"):
                call("Input.dispatchMouseEvent", {"type": event, "x": x, "y": y, "button": "left", "clickCount": 1})
        js("new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))", True)
        box = webview["menu"] = js("""(() => {
            const menu = document.querySelector('#oa-menu');
            const r = menu.getBoundingClientRect();
            return {hidden:menu.hidden, left:r.left, top:r.top, right:r.right, bottom:r.bottom,
                    width:r.width, height:r.height, items:menu.querySelectorAll('button').length};
        })()""")
        if (box["hidden"] or box["items"] != 7 or box["width"] <= 0 or box["height"] <= 0
                or box["left"] < 0 or box["top"] < 0
                or box["right"] > webview["width"] or box["bottom"] > webview["height"]):
            raise RuntimeError(f"{label}: visible Fluent menu must be entirely inside the viewport")
        png = base64.b64decode(call("Page.captureScreenshot", {"format": "png"})["data"])
        (output / f"menu-{label}.png").write_bytes(png)
        slot["screenshot_sha256"] = hashlib.sha256(png).hexdigest()

    original = None
    attempted = False
    resolution_touched = False
    try:
        original = report["initial_native"] = measure()
        original_percent = round(original["window_dpi"] * 100 / 96)
        snapshot("before", original["window_dpi"], original["primary_monitor"])
        probe = settings("before")
        if probe.get("error"):
            report["status"] = "BLOCKED_SETTINGS_UI"
            raise RuntimeError(probe["error"])
        if probe.get("current") != original_percent or not original["primary_monitor"]:
            report["status"] = "BLOCKED_DISPLAY_BINDING"
            raise RuntimeError("The settings selection must match the package window on the primary display")
        if original_percent == 200:
            # Existing 200% is a valid geometry observation; no change or
            # transition can be exercised in this particular run.
            snapshot("at-200", 192, original["primary_monitor"])
            report["status"] = "PASS_EXISTING_200"
            report["transition_note"] = "Not tested: display was already at 200%"
            return

        def has_200(state):
            return any(it["percent"] == 200 and it["enabled"] for it in state.get("options", ()))

        if not has_200(probe):
            original_mode, offered, set_resolution = display_modes()
            if original_mode.dimensions() != original["monitor_pixels"]:
                raise RuntimeError("The graphics mode did not match the original app monitor")
            report["offered_modes_at_original_refresh"] = sorted(offered)
            choices = report["temporary_resolution_attempts"] = []
            for size in ((1600, 1200), (1920, 1080)):
                if size not in offered:
                    continue
                choice = {"pixels": size}
                choices.append(choice)
                resolution_touched = True
                try:
                    set_resolution(offered[size])
                except RuntimeError as error:
                    choice["error"] = str(error)
                    continue
                for _ in range(50):
                    if measure()["monitor_pixels"] == list(size):
                        break
                    time.sleep(0.2)
                else:
                    raise RuntimeError("The temporary mode did not reach the app's monitor")
                probe = settings(f"at-{size[0]}x{size[1]}")
                if probe.get("error"):
                    raise RuntimeError(probe["error"])
                choice["options"] = probe["options"]
                if has_200(probe):
                    break
        if not has_200(probe):
            report["status"] = "BLOCKED_200_UNAVAILABLE"
            raise RuntimeError("The tested Windows display modes do not offer an enabled 200% Scale")
        attempted = True
        selected = settings("apply-200", 200)
        if not selected.get("selection_invoked"):
            raise RuntimeError(selected.get("error", "Windows did not accept the 200% selection"))
        snapshot("at-200", 192, original["primary_monitor"])
        report["transition_tested"] = True
        report["status"] = "PASS"
    finally:
        try:
            if original is not None:
                report["change_attempted"] = attempted
                report["temporary_resolution_attempted"] = resolution_touched
                if attempted or resolution_touched:
                    try:
                        if measure()["window_dpi"] != original["window_dpi"]:
                            restored = settings("restore", original_percent)
                            if not restored.get("selection_invoked"):
                                raise RuntimeError(restored.get("error", "Windows did not accept the original Scale"))
                            snapshot("scale-restored", original["window_dpi"], original["primary_monitor"], capture=False)
                    finally:
                        if resolution_touched:
                            set_resolution(original_mode)
                            for _ in range(50):
                                if measure()["monitor_pixels"] == original["monitor_pixels"]:
                                    break
                                time.sleep(0.2)
                            else:
                                raise RuntimeError("Original monitor resolution did not return")
                    confirmation = settings("restored")
                    if confirmation.get("current") != original_percent:
                        raise RuntimeError("Original Scale was not observable after restoration")
                snapshot("restored", original["window_dpi"], original["primary_monitor"],
                         capture=attempted or resolution_touched)
                report["restored"] = True
        except BaseException as error:
            report["status"] = "FAIL_RESTORE"
            report["restore_error"] = str(error)
            raise
        finally:
            (output / "system-scale.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
