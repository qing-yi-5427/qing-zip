"""Exercise saving a real nested archive tab on the disposable capacity volume."""
from contextlib import suppress
import hashlib
import io
import os
from pathlib import Path
import random
import re
import socket
import subprocess
import time
import urllib.error
import urllib.request
import winreg
import zipfile

from playwright.sync_api import expect, sync_playwright

from arm_gui_lifecycle import guarded_run


def read_member(archive, member):
    with zipfile.ZipFile(archive) as opened:
        return opened.read(member)


def inspect_inner(content):
    with zipfile.ZipFile(io.BytesIO(content)) as inner:
        return {name: hashlib.sha256(inner.read(name)).hexdigest() for name in inner.namelist()}


def save_copy_on_another_volume(page, app, destination, evidence, result):
    from pywinauto import Desktop

    page.locator('.oa-nested-bar [data-action="save-as"]').click()
    picker = Desktop(backend="uia").window(process=app.pid, class_name="#32770")
    picker.wait("visible", timeout=20)
    result["save_as_dialog_title"] = picker.window_text()
    try:
        # The standard Windows file dialog exposes its file-name box under
        # FileNameControlHost. Its dialog button ID stays stable across locales.
        filename = picker.child_window(auto_id="FileNameControlHost").child_window(control_type="Edit")
        filename.wait("visible", timeout=10)
        filename.set_edit_text(str(destination))
        picker.capture_as_image().save(evidence / "nested-save-as-dialog.png")
        picker.child_window(auto_id="1", control_type="Button").click_input()
        expect(page.locator("#qa-toast")).to_contain_text("已另存为", timeout=30000)
    except Exception:
        if picker.exists(timeout=1):
            result["native_save_controls"] = [
                [control.element_info.control_type, control.element_info.automation_id, control.window_text()]
                for control in picker.descendants()
            ][:80]
        raise


def exercise_nested_save(exe, work, env, root, marker, identity, result,
                         fill, verify_volume, sha, snapshot, expect_feedback):
    if os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted" or os.environ.get("RUNNER_OS") != "Windows":
        raise RuntimeError("A disposable hosted Windows desktop is required")
    evidence = work.parent / "evidence"
    folder = root / ("qa-" + marker) / "nested"
    folder.mkdir()
    outer = folder / "outer.zip"
    (folder / "unrelated.txt").write_bytes(b"Keep this neighbour.")
    inner = io.BytesIO()
    with zipfile.ZipFile(inner, "w", zipfile.ZIP_STORED) as archive:
        archive.writestr("before.bin", random.Random(4).randbytes(2 * 1024 * 1024))
        archive.writestr("unchanged.txt", b"Keep this member unchanged.")
    old_inner = inner.getvalue()
    with zipfile.ZipFile(outer, "x", zipfile.ZIP_STORED) as archive:
        archive.writestr("inner.zip", old_inner)
        archive.writestr("sibling.zip", old_inner)
        archive.writestr("outer-only.txt", b"Keep the outer member.")
    before = snapshot(folder)
    original = inspect_inner(old_inner)
    expected = dict(original)
    expected["after.bin"] = expected.pop("before.bin")
    result.update(outer_before=before, inner_before=original, gui_sha256=sha(exe))
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    profile = work / "nested-webview-profile"
    arguments = f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
    env = dict(env, WEBVIEW2_USER_DATA_FOLDER=str(profile), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=arguments)
    policy = winreg.CreateKeyEx(winreg.HKEY_LOCAL_MACHINE,
                                r"SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments",
                                0, winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY)
    session = {"passed": False}
    with guarded_run(winreg, policy, exe.name, arguments, session, evidence / "nested-desktop.json") as lifetime:
        app = subprocess.Popen([exe, outer], env=env, cwd=work, stdin=subprocess.DEVNULL)
        lifetime["proc"] = app
        deadline = time.monotonic() + 75
        endpoint = f"http://127.0.0.1:{port}"
        while time.monotonic() < deadline:
            if app.poll() is not None:
                raise RuntimeError(f"Packaged desktop exited during startup with {app.returncode}")
            with suppress(OSError, urllib.error.URLError):
                with urllib.request.urlopen(endpoint + "/json/version", timeout=1):
                    break
            time.sleep(.2)
        else:
            raise RuntimeError("Packaged WebView2 debugging endpoint unavailable")
        with sync_playwright() as driver:
            browser = driver.chromium.connect_over_cdp(endpoint)
            page = browser.contexts[0].pages[0]
            try:
                page.locator('[data-entry="inner.zip"]').dblclick(timeout=60000)
                row = page.locator('[data-entry="before.bin"]')
                row.click(timeout=60000)
                page.keyboard.press("F2")
                page.locator("#qa-rename").fill("after.bin")
                page.locator('[data-action="rename-submit"]').click()
                expect(page.locator('[data-entry="after.bin"]')).to_be_visible(timeout=60000)
                save = page.locator('.oa-nested-bar [data-action="save-nested"]')
                expect(save).to_be_visible(timeout=10000)
                work_copies = list(Path(env["TEMP"]).rglob("inner.zip"))
                if len(work_copies) != 1:
                    raise RuntimeError(f"Expected one nested working copy, found {len(work_copies)}")
                working = work_copies[0]
                edited = sha(working)
                if inspect_inner(working.read_bytes()) != expected or snapshot(folder) != before:
                    raise RuntimeError("Editing the inner tab changed the wrong archive")
                result["edited_copy_sha256"] = edited
                close = page.locator('#qa-overlay:not([hidden]) .qa-dialog-footer [data-action="close"]')
                filler = folder.parent / "nested-reserve.bin"
                try:
                    fill(filler, root, marker, identity, result)
                    result["status"] = "failed"
                    save.click()
                    message = page.get_by_text(re.compile(r"空间不足|磁盘已满|not enough space|disk full|os error 112", re.I))
                    expect(message.first).to_be_visible(timeout=30000)
                    result["failure_text"] = message.first.inner_text()[:500]
                    dialog = page.get_by_role("dialog")
                    result["dialog_path"] = dialog.locator(".qa-jobcard p.qa-sub").first.inner_text()
                    result["dialog_alerts"] = [text for text in dialog.get_by_role("alert").all_inner_texts() if text]
                    if expect_feedback:
                        advice = "磁盘空间不足。内层更改已保留，释放空间后可重试，也可另存为。"
                        expect(dialog.get_by_text(advice, exact=True)).to_have_count(1)
                        if result["dialog_alerts"] != [advice]:
                            raise RuntimeError("Expected one accessible and actionable space error")
                        display = result["dialog_path"].removeprefix("\\\\?\\")
                        if display != f"{outer} › inner.zip":
                            raise RuntimeError("Nested task did not show the original outer path and member")
                    page.screenshot(path=evidence / "nested-insufficient-space.png")
                    result["outer_after_failure"] = snapshot(folder)
                    result["working_after_failure"] = sha(working)
                    if result["outer_after_failure"] != before or result["working_after_failure"] != edited:
                        raise RuntimeError("Failed nested save changed the outer archive, neighbour, or edited working copy")
                    if expect_feedback:
                        close.click()
                        destination = work / "saved-inner.zip"
                        if destination.exists() or destination.drive.casefold() == root.drive.casefold():
                            raise RuntimeError("Expected a fresh copy on the fixture volume")
                        save_copy_on_another_volume(page, app, destination, evidence, result)
                        result["saved_as_sha256"] = sha(destination)
                        result["saved_as_members"] = inspect_inner(destination.read_bytes())
                        result["free_after_save_as"] = verify_volume(root, marker, identity).free
                        if (result["saved_as_sha256"] != edited or result["saved_as_members"] != expected
                                or result["free_after_save_as"] != 0 or snapshot(folder) != before
                                or sha(working) != edited):
                            raise RuntimeError("Save As on another volume did not preserve all data")
                        page.screenshot(path=evidence / "nested-save-as-complete.png")
                finally:
                    if filler.exists():
                        verify_volume(root, marker, identity)
                        filler.unlink()
                        result["free_after_release"] = verify_volume(root, marker, identity).free
                if close.is_visible():
                    close.click()
                save.click()
                expect(page.locator("#qa-toast")).to_contain_text("已保存至外层压缩包", timeout=60000)
                page.screenshot(path=evidence / "nested-save-retried.png")
                committed = read_member(outer, "inner.zip")
                result["committed_inner_sha256"] = hashlib.sha256(committed).hexdigest()
                result["inner_after_retry"] = inspect_inner(committed)
                result["outer_after_retry"] = snapshot(folder)
                with zipfile.ZipFile(outer) as updated:
                    preserved = (sorted(updated.namelist()) == ["inner.zip", "outer-only.txt", "sibling.zip"]
                                 and updated.read("sibling.zip") == old_inner
                                 and updated.read("outer-only.txt") == b"Keep the outer member.")
                if (result["committed_inner_sha256"] != edited or result["inner_after_retry"] != expected
                        or not preserved or result["outer_after_retry"] != {**before, outer.name: sha(outer)}):
                    raise RuntimeError("Published nested edit or sibling data differs from expected")
                session["passed"] = True
            except Exception as error:
                with suppress(Exception):
                    page.screenshot(path=evidence / "nested-last-window.png")
                    result["last_visible_text"] = page.locator("body").inner_text(timeout=3000)[-1500:]
                raise RuntimeError(str(error)) from error
    result["status"] = "passed"
