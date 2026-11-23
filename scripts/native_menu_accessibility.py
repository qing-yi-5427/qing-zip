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


def wait_os(read_os, high, animations=None, timeout=10):
    """Wait for the native setting to settle; retain the last observed value on failure."""
    deadline = time.monotonic() + timeout
    while True:
        state, enabled = read_os()
        actual = {"os_high": bool(state.flags & 1), "os_animations": enabled}
        if actual["os_high"] == high and (animations is None or enabled == animations):
            return actual
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Windows did not apply the requested settings: {actual}")
        time.sleep(0.1)


def apply_os(read_os, invoke, high, animations):
    state, _ = read_os()
    if bool(state.flags & 1) != high:
        state.flags = (state.flags & ~1) | int(high)
        invoke(0x0043, state.size, ctypes.byref(state), 3)
        wait_os(read_os, high)
    # Applying a contrast scheme may itself change animations. Read it again.
    _, current_animations = read_os()
    if current_animations != animations:
        invoke(0x1043, 0, ctypes.c_void_p(int(animations)), 3)
    return wait_os(read_os, high, animations)


def verify_menu(call, js, output, executable, test_form=False):
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
        return apply_os(read_os, invoke, high, animations)

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
            state = report["states"][name] = set_os(high, animated)
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
            if test_form:
                js("document.querySelector('[data-action=app-menu]').click()")
                if name == 'normal-again':
                    js("document.querySelector('[data-theme=dark]').click()")
                js("Array.from(document.querySelectorAll('[data-action=compress]')).find(e=>e.getClientRects().length).click()")
                js("""(() => {
                    const set=(id,value)=>{let e=document.getElementById(id);e.value=value;e.dispatchEvent(new Event('input',{bubbles:true}))};
                    set('qa-inputs','C:/qz-synthetic/sample.txt');set('qa-save','C:/qz-synthetic/result.zip');
                    document.querySelector('[data-action=compression-advanced]').click();
                    set('qa-threads','999');document.querySelector('[data-action=startcompress]').click();
                })()""")
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    form = state["form"] = js("""(() => {
                        const input=document.querySelector('#qa-threads'), body=input.closest('.oa-dialog-body');
                        const i=input.getBoundingClientRect(),b=body.getBoundingClientRect(),error=document.querySelector('#qa-formerror');
                        return {forced:matchMedia('(forced-colors: active)').matches,
                                reduced:matchMedia('(prefers-reduced-motion: reduce)').matches,
                                active:document.activeElement.id, invalid:input.getAttribute('aria-invalid'),
                                error:error.textContent, input:[i.top,i.bottom],body:[b.top,b.bottom],
                                viewport:[innerWidth,innerHeight],dpr:devicePixelRatio,
                                button:(()=>{let e=document.querySelector('[data-action=startcompress]'),s=getComputedStyle(e),r=e.getBoundingClientRect();return {text:e.innerText,color:s.color,background:s.backgroundColor,forcedAdjust:s.forcedColorAdjust,rect:[r.x,r.y,r.width,r.height],html:e.innerHTML}})()};
                    })()""")
                    if form['invalid']=='true' and form['error'] and form['active']=='qa-threads':
                        break
                    time.sleep(.1)
                else:
                    raise RuntimeError(f"Compression validation was not observable: {form}")
                if form['forced'] != high or form['reduced'] != (not animated):
                    raise RuntimeError(f"Compression form did not reflect desktop accessibility: {form}")
                if form['input'][0] < form['body'][0] or form['input'][1] > form['body'][1]:
                    raise RuntimeError(f"Focused invalid field is clipped: {form}")
                time.sleep(1)  # allow the native contrast transition and compositor to settle
                form['button_settled'] = js("""(() => {let e=document.querySelector('[data-action=startcompress]'),s=getComputedStyle(e);return {color:s.color,background:s.backgroundColor,forcedAdjust:s.forcedColorAdjust}})()""")
                if high and form['button_settled']['forcedAdjust'] != 'none':
                    raise RuntimeError(f"Primary button still uses automatic forced colors: {form}")
                if not high and form['button_settled']['forcedAdjust'] != 'auto':
                    raise RuntimeError(f"Normal appearance unexpectedly opts out of forced colors: {form}")
                if high:
                    form['disabled'] = js("""(() => {let e=document.querySelector('[data-action=startcompress]');e.disabled=true;let s=getComputedStyle(e);let result={color:s.color,background:s.backgroundColor,disabled:e.disabled};e.disabled=false;return result})()""")
                    if not form['disabled']['disabled'] or form['disabled']['color'] == form['disabled']['background']:
                        raise RuntimeError(f"Disabled primary button is unreadable: {form}")
                    for _ in range(30):
                        for event in ('rawKeyDown','keyUp'):
                            call('Input.dispatchKeyEvent', {'type':event,'key':'Tab','code':'Tab','windowsVirtualKeyCode':9})
                        if js("document.activeElement.matches('[data-action=startcompress]')"):
                            break
                    else:
                        raise RuntimeError('Primary button not reachable by Tab')
                    form['keyboard_focus'] = js("""(() => {let e=document.activeElement,s=getComputedStyle(e);return {focusVisible:e.matches(':focus-visible'),outlineStyle:s.outlineStyle,outlineWidth:s.outlineWidth}})()""")
                    if not form['keyboard_focus']['focusVisible'] or form['keyboard_focus']['outlineStyle']=='none':
                        raise RuntimeError(f"Keyboard focus not visible: {form}")
                    png = base64.b64decode(call('Page.captureScreenshot', {'format': 'png'})['data'])
                    (output / 'form-contrast-keyboard.png').write_bytes(png)
                    js("document.querySelector('#qa-threads').focus()")
                png = base64.b64decode(call('Page.captureScreenshot', {'format': 'png'})['data'])
                (output / ('form-' + name + '.png')).write_bytes(png)
                form['screenshot_sha256'] = hashlib.sha256(png).hexdigest()
                js("document.querySelector('.qa-dialog-header [data-action=close]').click()")
        report["status"] = "PASS"
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        try:
            report["restoration"] = set_os(original_high, original_animation)
            report["restored"] = True
        except Exception as error:
            report["restoration_error"] = str(error)
            raise
        finally:
            (output / "native-settings.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        if not report["restored"]:
            raise RuntimeError("Windows accessibility settings did not return to their original state")
