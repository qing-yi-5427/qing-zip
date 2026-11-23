"""Verify an Explorer drag into the real home window on a disposable Windows desktop."""
import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import runpy
import tempfile
import time


def until(find, description):
    deadline = time.monotonic() + 25
    while time.monotonic() < deadline:
        found = find()
        if found:
            return found
        time.sleep(0.15)
    raise RuntimeError(f"Timed out waiting for {description}")


def exercise(executable, output, scratch, report, debug):
    from pywinauto import Desktop, mouse
    from win32com.client import Dispatch
    import win32gui

    gui = runpy.run_path(str(Path(__file__).with_name("check-smart-extraction-gui.py")))
    browser = gui["Browser"]()
    folder = scratch / "Input"
    folder.mkdir()
    source = folder / "parcel.dat"
    payload = b"Native Windows Explorer drop into qing-zip\n"
    gui["fixture_zip"](source, "parcel/item.txt", payload)
    original = gui["sha"](source)
    shell = Dispatch("Shell.Application")
    shell.Open(str(folder))

    def find_explorer():
        for window in shell.Windows():
            try:
                if os.path.normcase(window.Document.Folder.Self.Path) == os.path.normcase(str(folder)):
                    return window
            except Exception:
                continue

    explorer = until(find_explorer, "the fixture's Explorer window")
    pressed = False
    try:
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.WindowFromPoint.argtypes = [wintypes.POINT]
        user32.WindowFromPoint.restype = wintypes.HWND
        user32.GetAncestor.argtypes = [wintypes.HWND, wintypes.UINT]
        user32.GetAncestor.restype = wintypes.HWND
        def owner(point):
            return user32.GetAncestor(user32.WindowFromPoint(wintypes.POINT(*point)), 2)

        browser.start(executable, scratch / "profile", debug=debug)
        identity = browser.command("Runtime.evaluate", expression="window.__TAURI__.core.invoke('current_executable')",
                                   returnByValue=True, awaitPromise=True)
        if Path(identity["result"]["value"]).resolve() != executable:
            raise RuntimeError("The attached WebView2 page belongs to a different application")
        desktop = Desktop(backend="uia")
        app = until(lambda: next((win for win in desktop.windows(process=browser.process.pid) if win.is_visible()), None),
                    "a visible qing-zip window")
        width, height = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
        explorer_hwnd = int(explorer.HWND)
        win32gui.ShowWindow(explorer_hwnd, 9)
        win32gui.ShowWindow(app.handle, 9)
        win32gui.SetWindowPos(explorer_hwnd, -2, -185, 0, 820, height - 45, 0x40)
        win32gui.SetWindowPos(app.handle, -1, width // 3, 0, max(760, width * 2 // 3), height - 45, 0x40)
        explorer.Document.CurrentViewMode = 4  # FVM_DETAILS
        xp = desktop.window(handle=explorer_hwnd)

        def fixture_item():
            return next((item for item in xp.descendants(control_type="ListItem")
                         if item.element_info.name in (source.name, source.stem) and item.is_visible()), None)

        item = until(fixture_item, "the archive fixture in Explorer")
        target = desktop.window(handle=app.handle).child_window(auto_id="oa-home-smart-drop", control_type="Button")
        target.wait("visible", timeout=15)
        item_rect = item.rectangle()
        target_rect = target.rectangle()
        start = (max(8, item_rect.left + 24), (item_rect.top + item_rect.bottom) // 2)
        finish = (min(width - 8, (target_rect.left + target_rect.right) // 2),
                  (target_rect.top + target_rect.bottom) // 2)
        report.update(desktop=[width, height], source_point=start, target_point=finish,
                      source_is_explorer=owner(start) == explorer_hwnd, target_is_app=owner(finish) == app.handle)
        if not report["source_is_explorer"] or not report["target_is_app"]:
            raise RuntimeError(f"Fixture or target is obscured: {report}")
        pressed = True
        mouse.press(coords=start)
        for i in range(1, 25):
            mouse.move(coords=(start[0] + (finish[0] - start[0]) * i // 24,
                               start[1] + (finish[1] - start[1]) * i // 24))
            time.sleep(.075)
        browser.wait("document.querySelector('#oa-home-smart-drop')?.classList.contains('oa-drop-active')", seconds=7)
        browser.screenshot(output / "hover.png")
        mouse.release(coords=finish)
        pressed = False
        browser.wait("document.querySelector('#oa-deep-path-0')?.getClientRects().length")
        preview = Path(browser.evaluate("document.querySelector('#oa-deep-path-0').textContent"))
        if preview != folder / "parcel":
            raise AssertionError(f"Preview differs from the fixture destination: {preview}")
        browser.screenshot(output / "preview.png")
        browser.click("[data-deep-action=start-batch]")
        gui["complete_job"](browser)
        gui["confirm_deep_output"](browser, preview)
        result = preview / "item.txt"
        if result.read_bytes() != payload or gui["sha"](source) != original:
            raise AssertionError("Output content or source archive changed unexpectedly")
        browser.screenshot(output / "result.png")
        report.update(status="PASS", source_sha256=original, output_sha256=gui["sha"](result),
                      output=result.relative_to(folder).as_posix(), gui_sha256=gui["sha"](executable))
    finally:
        if pressed:
            mouse.release()
        try:
            browser.close()
        finally:
            explorer.Quit()


def main():
    if os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("The physical drag requires a disposable GitHub-hosted Windows desktop")
    ctypes.windll.user32.SetProcessDPIAware()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    report = {"status": "FAIL"}
    try:
        from arm_gui_lifecycle import hosted_debug_session
        exe = args.exe.resolve(strict=True)
        with tempfile.TemporaryDirectory(dir=os.environ["RUNNER_TEMP"], prefix="qing-home-", ignore_cleanup_errors=True) as scratch:
            with hosted_debug_session(exe, output / "desktop-session.json") as debug:
                exercise(exe, output, Path(scratch), report, debug)
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(report, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
