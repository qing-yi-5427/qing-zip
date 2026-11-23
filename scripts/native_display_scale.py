"""Check actual Windows Display > Scale, with no WebView device emulation."""

import base64
import ctypes
from ctypes import wintypes
import hashlib
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
    dpi = bind("GetDpiForWindow", wintypes.UINT, wintypes.HWND)
    context = bind("GetWindowDpiAwarenessContext", wintypes.HANDLE, wintypes.HWND)
    awareness = bind("GetAwarenessFromDpiAwarenessContext", ctypes.c_int, wintypes.HANDLE)
    monitor = bind("MonitorFromWindow", wintypes.HANDLE, wintypes.HWND, wintypes.DWORD)
    monitor_info = bind("GetMonitorInfoW", wintypes.BOOL, wintypes.HANDLE, ctypes.POINTER(MonitorInfo))
    thread_context = bind("SetThreadDpiAwarenessContext", wintypes.HANDLE, wintypes.HANDLE)
    metrics = bind("GetSystemMetrics", ctypes.c_int, ctypes.c_int)

    def measure():
        windows = []

        @callback_type
        def collect(hwnd, _):
            candidate = wintypes.DWORD()
            owner(hwnd, ctypes.byref(candidate))
            if candidate.value == pid and visible(hwnd) and dpi(hwnd):
                windows.append(hwnd)
            return True

        if not enum(collect, 0) or len(windows) != 1:
            raise RuntimeError(f"Expected one visible package window, found {len(windows)}")
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
              "observations": {}, "restored": False}

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
            report["status"] = "BLOCKED_ALREADY_200"
            raise RuntimeError("The runner is already at 200%, so it does not provide a before/after transition")
        if not any(it["percent"] == 200 and it["enabled"] for it in probe.get("options", ())):
            report["status"] = "BLOCKED_200_UNAVAILABLE"
            raise RuntimeError("The observed Windows Scale list does not provide an enabled 200% option")
        attempted = True
        selected = settings("apply-200", 200)
        if not selected.get("selection_invoked"):
            raise RuntimeError(selected.get("error", "Windows did not accept the 200% selection"))
        snapshot("at-200", 192, original["primary_monitor"])
        report["status"] = "PASS"
    finally:
        try:
            if original is not None:
                report["change_attempted"] = attempted
                if attempted:
                    if measure()["window_dpi"] != original["window_dpi"]:
                        restored = settings("restore", original_percent)
                        if not restored.get("selection_invoked"):
                            raise RuntimeError(restored.get("error", "Windows did not accept the original Scale"))
                    confirmation = settings("restored")
                    if confirmation.get("current") != original_percent:
                        raise RuntimeError("Original Scale was not observable after restoration")
                snapshot("restored", original["window_dpi"], original["primary_monitor"], capture=attempted)
                report["restored"] = True
        except BaseException as error:
            report["status"] = "FAIL_RESTORE"
            report["restore_error"] = str(error)
            raise
        finally:
            (output / "system-scale.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
