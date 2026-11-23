"""Observe the real desktop settings and their effect on a running qing-zip menu."""
import base64
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
import platform
import time


def check_host():
    if (os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true"
            or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"):
        raise RuntimeError("Changing the desktop settings requires a disposable GitHub-hosted Windows runner")


class HighContrast(ctypes.Structure):
    _fields_ = [("size", wintypes.UINT), ("flags", wintypes.DWORD), ("scheme", wintypes.LPWSTR)]


def verify_menu(call, js, output, executable):
    check_host()
    spi = ctypes.WinDLL("user32", use_last_error=True).SystemParametersInfoW
    spi.argtypes = [wintypes.UINT, wintypes.UINT, wintypes.LPVOID, wintypes.UINT]
    spi.restype = wintypes.BOOL

    def invoke(action, size, value, flags=0):
        if not spi(action, size, value, flags):
            raise ctypes.WinError(ctypes.get_last_error())

    def read_os():
        contrast = HighContrast(ctypes.sizeof(HighContrast), 0, None)
        animations = wintypes.BOOL()
        invoke(0x0042, contrast.size, ctypes.byref(contrast))  # SPI_GETHIGHCONTRAST
        invoke(0x1042, 0, ctypes.byref(animations))  # SPI_GETCLIENTAREAANIMATION
        return contrast, bool(animations.value)

    def set_os(high, animations):
        state, current_animations = read_os()
        if bool(state.flags & 1) != high:
            state.flags = (state.flags & ~1) | int(high)
            invoke(0x0043, state.size, ctypes.byref(state), 3)  # persist and broadcast WM_SETTINGCHANGE
        if current_animations != animations:
            invoke(0x1043, 0, ctypes.c_void_p(int(animations)), 3)

    def snapshot():
        return js("""(() => {
            const menu = document.querySelector('#oa-menu');
            const item = menu.querySelector('button');
            const style = getComputedStyle(menu);
            return {forced:matchMedia('(forced-colors: active)').matches,
                    reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,
                    material:document.querySelector('#qing-archive').dataset.material,
                    visible:!menu.hidden && menu.getBoundingClientRect().width > 0,
                    background:style.backgroundColor, image:style.backgroundImage,
                    text:getComputedStyle(item).color,
                    transition:getComputedStyle(item).transitionDuration,
                    items:menu.querySelectorAll('button').length};
        })()""")

    original, original_animation = read_os()
    original_high = bool(original.flags & 1)
    report = {"platform": platform.platform(), "arch": platform.machine(),
              "gui_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
              "dpr": js("devicePixelRatio"), "original": {"high": original_high, "animations": original_animation},
              "states": {}, "restored": False, "status": "FAIL"}
    try:
        for name, high, animated in (("normal", False, True), ("contrast", True, True),
                                     ("reduced", False, False), ("normal-again", False, True)):
            set_os(high, animated)
            flags, enabled = read_os()
            state = report["states"][name] = {"os_high": bool(flags.flags & 1), "os_animations": enabled}
            if state["os_high"] != high or enabled != animated:
                raise RuntimeError(f"Windows did not apply the requested settings: {state}")
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                js("if(document.querySelector('#oa-menu').hidden)document.querySelector('[data-action=app-menu]').click()")
                view = state["webview"] = snapshot()
                if (view["visible"] and view["items"] == 7 and view["forced"] == high
                        and view["reduced"] == (not animated) and (not high or view["material"] == "solid")):
                    break
                time.sleep(0.20)
            else:
                raise RuntimeError(f"The running menu did not reflect the real Windows settings: {state}")
            if high and view["image"] != "none":
                raise RuntimeError("The forced-colors menu retained its decorative surface")
            if not animated and any(float(part.strip().removesuffix("s")) > 0.00002 for part in view["transition"].split(",")):
                raise RuntimeError(f"Reduced-motion transition remained active: {view['transition']}")
            if name == "normal-again" and view["material"] != report["states"]["normal"]["webview"]["material"]:
                raise RuntimeError("The window did not restore its previous material")
            png = base64.b64decode(call("Page.captureScreenshot", {"format": "png"})["data"])
            (output / (name + ".png")).write_bytes(png)
            state["screenshot_sha256"] = hashlib.sha256(png).hexdigest()
        report["status"] = "PASS"
    finally:
        try:
            set_os(original_high, original_animation)
            restored, animations = read_os()
            report["restored"] = bool(restored.flags & 1) == original_high and animations == original_animation
        finally:
            (output / "native-settings.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        if not report["restored"]:
            raise RuntimeError("Windows accessibility settings did not return to their original state")
