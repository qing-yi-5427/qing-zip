"""Exercise an ordinary three-layer ZIP and an outer-archive conflict in a real Tauri window."""

import argparse
import ctypes
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid
import zipfile
from contextlib import contextmanager
from ctypes import wintypes

import win32api
import win32con
import win32event
import win32process

local_tools = Path(__file__).resolve().with_name("python-tools")
if local_tools.is_dir():
    sys.path.insert(0, str(local_tools))

from playwright.sync_api import Error, expect, sync_playwright


def digest(data):
    if isinstance(data, Path):
        data = data.read_bytes()
    return hashlib.sha256(data).hexdigest()


def archive(members):
    contents = io.BytesIO()
    with zipfile.ZipFile(contents, "w") as package:
        for name, data in members.items():
            package.writestr(zipfile.ZipInfo(name), data)
    return contents.getvalue()


def checked_members(data):
    if isinstance(data, bytes):
        data = io.BytesIO(data)
    with zipfile.ZipFile(data) as package:
        assert package.testzip() is None
        paths = package.namelist()
        assert len(paths) == len(set(paths))
        return {name: package.read(name) for name in paths}


def verify_package(args):
    names = ("qing-zip.exe", "obz.exe", "obz-worker.exe")
    actual = {name: digest(args.exe.with_name(name)) for name in names}
    if args.staged_sha256:
        assert actual == dict(zip(names, (value.lower() for value in args.staged_sha256), strict=True))
        return {"binaries_sha256": actual, "source_artifact": "locally staged portable directory"}
    expected = json.loads(args.hardening.read_text(encoding="utf-8-sig"))
    assert expected["target"] == "x86_64-pc-windows-msvc"
    binaries = {item["name"]: item["sha256"] for item in expected["binaries"]}
    assert actual == {"qing-zip.exe": binaries["obz-gui.exe"], "obz.exe": binaries["obz.exe"],
                      "obz-worker.exe": binaries["obz-worker.exe"]}
    return {"binaries_sha256": actual, "portable_zip_sha256": digest(args.package)}


def allow_private_launch():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenMutexW.argtypes = (wintypes.DWORD, wintypes.BOOL, wintypes.LPCWSTR)
    kernel.OpenMutexW.restype = wintypes.HANDLE
    for name in ("dev.openarchive.app-sim", "dev.openarchive.app_0.12-sim", "dev.openarchive.app_0.12.23-sim"):
        handle = kernel.OpenMutexW(0x100000, False, name)
        if handle:
            win32api.CloseHandle(handle)
            raise RuntimeError("Another qing-zip instance owns the product mutex")
        if ctypes.get_last_error() != 2:
            raise ctypes.WinError(ctypes.get_last_error())


@contextmanager
def private_process(exe, fixture, archive_path):
    allow_private_launch()
    local, roaming, profile, transient = (fixture / name for name in ("local", "roaming", "profile", "transient"))
    for part in (local, roaming, profile, transient):
        part.mkdir(exist_ok=True)
    for old_marker in profile.rglob("DevToolsActivePort"):
        old_marker.unlink()
    env = dict(os.environ, LOCALAPPDATA=str(local), APPDATA=str(roaming), TEMP=str(transient), TMP=str(transient),
               WEBVIEW2_USER_DATA_FOLDER=str(profile),
               WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=0 --remote-debugging-address=127.0.0.1")
    user = ctypes.WinDLL("user32", use_last_error=True)
    user.CreateDesktopW.argtypes = (wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.LPVOID, wintypes.DWORD,
                                   wintypes.DWORD, wintypes.LPVOID)
    user.CreateDesktopW.restype = wintypes.HANDLE
    user.CloseDesktop.argtypes = (wintypes.HANDLE,)
    user.CloseDesktop.restype = wintypes.BOOL
    desktop_name = "QzNestedQa" + uuid.uuid4().hex
    desktop = user.CreateDesktopW(desktop_name, None, None, 0, 0x01ff, None)
    if not desktop:
        raise ctypes.WinError(ctypes.get_last_error())
    process = thread = None
    try:
        startup = win32process.STARTUPINFO()
        startup.lpDesktop = desktop_name
        arguments = [str(exe)]
        if archive_path:
            arguments.append(str(archive_path))
        process, thread, pid, _ = win32process.CreateProcess(str(exe), subprocess.list2cmdline(arguments),
            None, None, False, win32con.CREATE_UNICODE_ENVIRONMENT, env, str(fixture), startup)
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline:
            if win32event.WaitForSingleObject(process, 0) == win32event.WAIT_OBJECT_0:
                raise RuntimeError("Owned qing-zip process exited before WebView2 connected")
            marker = next(profile.rglob("DevToolsActivePort"), None)
            if marker and (lines := marker.read_text().splitlines()):
                port = int(lines[0])
                break
            time.sleep(.1)
        else:
            raise TimeoutError("No private WebView2 debugging endpoint")
        with sync_playwright() as automation:
            browser = automation.chromium.connect_over_cdp(f"http://127.0.0.1:{port}")
            page = browser.contexts[0].pages[0]
            page.set_default_timeout(12000)
            yield page, process, pid
    finally:
        if process is not None:
            if win32event.WaitForSingleObject(process, 2000) != win32event.WAIT_OBJECT_0:
                win32process.TerminateProcess(process, 1)
                win32event.WaitForSingleObject(process, 10000)
            win32api.CloseHandle(process)
        if thread is not None:
            win32api.CloseHandle(thread)
        if not user.CloseDesktop(desktop):
            raise ctypes.WinError(ctypes.get_last_error())


def sessions(page, fixture):
    active = page.evaluate("window.__TAURI__.core.invoke('nested_sessions')")
    assert all(Path(row["path"]).resolve().is_relative_to(fixture / "transient") for row in active)
    result = {row["member"]: row for row in active}
    assert len(result) == len(active)
    return result


def wait_for_conflict(page, feedback):
    task = page.locator('[data-job-card]').filter(has_text="保存至外层压缩包").last
    if feedback:
        expect(task).to_contain_text("外层压缩包或其中的文件已被修改", timeout=35000)
        expect(task).to_contain_text("请从有未保存更改的标签另存为")
    else:
        expect(task).to_contain_text("写回失败", timeout=35000)
        expect(task).to_contain_text("parent archive or its member changed since opening")
    failure = task.inner_text()
    return failure


def arm_click_timing(page, feedback):
    detail = ("外层压缩包或其中的文件已被修改" if feedback else
              "parent archive or its member changed since opening")
    page.evaluate("""detail => {
        const timing = { start: null, failure_card_ms: null, correct_middle_tab_ms: null };
        const visible = el => el.getClientRects().length && getComputedStyle(el).visibility === 'visible';
        const observer = new MutationObserver(() => {
            if (timing.start === null) return;
            const now = performance.now() - timing.start;
            if (timing.failure_card_ms === null) {
                for (const card of document.querySelectorAll('[data-job-card]')) {
                    if (card.querySelector('[data-state="failed"]') && visible(card) &&
                        card.textContent.includes('保存至外层压缩包') && card.textContent.includes(detail)) {
                        timing.failure_card_ms = now;
                        break;
                    }
                }
            }
            if (timing.correct_middle_tab_ms === null) {
                const labels = [...document.querySelectorAll('[data-archive-tab]')]
                    .filter(visible).map(tab => tab.textContent.trim());
                if (labels.some(label => label.startsWith('● middle.zip')) &&
                    !labels.some(label => label.startsWith('● inner.zip'))) timing.correct_middle_tab_ms = now;
            }
        });
        observer.observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true });
        document.addEventListener('click', event => {
            if (timing.start === null && event.target.closest('[data-action="save-nested"]')) {
                timing.start = performance.now();
            }
        }, { capture: true });
        window.qzNestedTiming = () => {
            observer.disconnect();
            return { failure_card_ms: timing.failure_card_ms, correct_middle_tab_ms: timing.correct_middle_tab_ms };
        };
    }""", detail)


def close_baseline(page, process):
    page.locator('[data-window="close"]').click()
    expect(page.get_by_text("保存内层压缩包的更改？", exact=True)).to_be_visible()
    overlay = page.locator('#qa-overlay:not([hidden])')
    expect(overlay).to_contain_text("middle.zip")
    expect(overlay).not_to_contain_text("inner.zip")
    try:
        page.locator('[data-action="nested-close-discard"]').click()
    except Error:
        pass  # Closing the native window can disconnect WebView2 first.
    assert win32event.WaitForSingleObject(process, 10000) == win32event.WAIT_OBJECT_0
    assert win32process.GetExitCodeProcess(process) == 0


def prepare_case(fixture, leaf_payload=b"ordinary leaf payload\n"):
    leaf_original = {"leaf.txt": leaf_payload, "inner-peer.txt": b"untouched inner peer\n"}
    inner_original = archive(leaf_original)
    middle_peer = b"untouched middle peer\n"
    outer_peer = b"untouched outer peer\n"
    middle_original = archive({"inner.zip": inner_original, "middle-peer.txt": middle_peer})
    outer_members = {"middle.zip": middle_original, "outer-peer.txt": outer_peer}
    initial = fixture / "outer.zip"
    initial.write_bytes(archive(outer_members))
    sibling = fixture / "filesystem-peer.txt"
    sibling.write_bytes(b"untouched filesystem peer\n")
    return initial, sibling


def exercise_conflict(page, fixture, initial, sibling, report, expect_feedback, new_name="changed-leaf.txt"):
    outer_members = checked_members(initial)
    middle_original = outer_members["middle.zip"]
    inner_original = checked_members(middle_original)["inner.zip"]
    leaf_original = checked_members(inner_original)
    middle_peer = checked_members(middle_original)["middle-peer.txt"]
    outer_peer = outer_members["outer-peer.txt"]
    report["initial_outer_sha256"] = digest(initial)
    report["filesystem_peer_sha256"] = digest(sibling)
    page.locator('[data-entry="middle.zip"]').dblclick(timeout=45000)
    page.locator('[data-entry="inner.zip"]').dblclick(timeout=45000)
    page.locator('[data-entry="leaf.txt"]').click(button="right", timeout=45000)
    page.locator('#oa-menu [data-action="rename"]').click()
    page.locator('#qa-rename').fill(new_name)
    page.locator('[data-action="rename-submit"]').click()
    expect(page.locator(f'[data-entry="{new_name}"]')).to_be_visible(timeout=45000)
    before = sessions(page, fixture)
    assert set(before) == {"inner.zip", "middle.zip"}
    assert before["inner.zip"]["dirty"] and not before["middle.zip"]["dirty"], before
    inner_copy, middle_copy = (Path(before[name]["path"]) for name in ("inner.zip", "middle.zip"))
    inner_sha = digest(inner_copy)
    edited = {new_name: leaf_original["leaf.txt"], "inner-peer.txt": leaf_original["inner-peer.txt"]}
    assert checked_members(inner_copy) == edited and digest(middle_copy) == digest(middle_original)
    assert digest(initial) == report["initial_outer_sha256"]

    external_middle = archive({"inner.zip": inner_original, "middle-peer.txt": middle_peer,
                               "external-middle.txt": b"separate writer\n"})
    external_outer = {**outer_members, "middle.zip": external_middle,
                      "external-outer.txt": b"separate writer\n"}
    external_bytes = archive(external_outer)
    expected_outer = digest(external_bytes)
    identity = initial.stat().st_ino
    # A separate ordinary writer can retain the parent identity while replacing the selected member's bytes.
    with initial.open("r+b") as output:
        output.write(external_bytes)
        output.truncate()
        output.flush()
        os.fsync(output.fileno())
    assert identity == initial.stat().st_ino
    report["same_outer_file_id_after_external_write"] = True
    assert expected_outer != report["initial_outer_sha256"] and digest(initial) == expected_outer

    arm_click_timing(page, expect_feedback)
    page.locator('[data-action="save-nested"]').click()
    failure = wait_for_conflict(page, expect_feedback)
    after = sessions(page, fixture)
    newest = checked_members(middle_copy)
    assert newest.keys() == {"inner.zip", "middle-peer.txt"}
    assert newest["middle-peer.txt"] == middle_peer and digest(newest["inner.zip"]) == inner_sha
    assert checked_members(newest["inner.zip"]) == edited
    assert not after["inner.zip"]["dirty"] and after["middle.zip"]["dirty"], after
    assert digest(initial) == expected_outer and checked_members(initial) == external_outer
    assert digest(sibling) == report["filesystem_peer_sha256"]
    assert checked_members(checked_members(initial)["middle.zip"])["inner.zip"] == inner_original
    if expect_feedback:
        expect(page.locator('[data-archive-tab]').filter(has_text="middle.zip")).to_contain_text("● middle.zip")
        expect(page.locator('[data-archive-tab]').filter(has_text="inner.zip")).not_to_contain_text("●")
    timing = page.evaluate("window.qzNestedTiming()")
    assert isinstance(timing["failure_card_ms"], (float, int)) and timing["failure_card_ms"] >= 0
    if expect_feedback:
        assert isinstance(timing["correct_middle_tab_ms"], (float, int)) and timing["correct_middle_tab_ms"] >= 0
    report.update({"external_outer_sha256": expected_outer, "outer_untouched_by_conflicting_save": True,
                   "after_save_dirty": {name: row["dirty"] for name, row in after.items()},
                   "immediate_dirty_tab_labels": [text.strip() for text in
                       page.locator('[data-archive-tab]').all_text_contents() if "●" in text],
                   "timing": timing, "retained_private_middle_sha256": digest(middle_copy), "edited_inner_sha256": inner_sha,
                   "outer_peer_sha256": digest(outer_peer), "middle_peer_sha256": digest(middle_peer),
                   "inner_peer_sha256": digest(leaf_original["inner-peer.txt"]),
                   "edited_leaf_sha256": digest(edited[new_name]),
                   "save_failure": failure.replace(str(fixture), "<test-root>")})
    return inner_copy, middle_copy


def exercise(args, fixture, report):
    initial, sibling = prepare_case(fixture)
    with private_process(args.exe, fixture, initial) as (page, process, pid):
        report["first_process_id"] = pid
        inner_copy, middle_copy = exercise_conflict(page, fixture, initial, sibling, report, args.expect_feedback)
        page.screenshot(path=args.out / "conflicting-save.png")
        close_baseline(page, process)
    assert digest(initial) == report["external_outer_sha256"] and digest(sibling) == report["filesystem_peer_sha256"]
    report["normal_close_discarded_private_copies"] = not inner_copy.exists() and not middle_copy.exists()
    assert report["normal_close_discarded_private_copies"]
    with private_process(args.exe, fixture, initial) as (page, process, pid):
        expect(page.locator('[data-entry="middle.zip"]')).to_be_visible(timeout=45000)
        assert pid != report["first_process_id"] and not sessions(page, fixture)
        report["cold_process_id"] = pid
        page.locator('[data-action="app-menu"]').click()
        page.locator('#oa-menu [data-menu="edit"]').click()
        page.locator('#oa-menu [data-action="external-copies"]').click()
        overlay = page.locator('#qa-overlay:not([hidden])')
        expect(overlay).to_contain_text("没有保留的编辑副本")
        assert not page.locator('#oa-retained-edit option').count()
        page.screenshot(path=args.out / "cold-no-retained.png")
        overlay.locator('[data-action="close"]').last.click()
        try:
            page.locator('[data-window="close"]').click()
        except Error:
            pass
        assert win32event.WaitForSingleObject(process, 10000) == win32event.WAIT_OBJECT_0
        assert win32process.GetExitCodeProcess(process) == 0
    report["after_cold_start"] = {"native_sessions": 0, "retained_edit_options": 0,
                                  "outer_sha256": digest(initial), "filesystem_peer_sha256": digest(sibling)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for label in ("exe", "out"):
        parser.add_argument("--" + label, required=True, type=Path)
    for label in ("hardening", "package"):
        parser.add_argument("--" + label, type=Path)
    parser.add_argument("--staged-sha256", nargs=3, metavar=("GUI_SHA", "CLI_SHA", "WORKER_SHA"))
    parser.add_argument("--source", required=True)
    parser.add_argument("--expect-feedback", action="store_true",
                        help="Require localized conflict guidance and exactly the backend's dirty middle tab")
    args = parser.parse_args()
    if args.staged_sha256 and (args.hardening or args.package):
        parser.error("A staged directory uses --staged-sha256 without a ZIP or hardening receipt")
    if not args.staged_sha256 and not (args.hardening and args.package):
        parser.error("Provide --hardening and --package, or explicitly pin three staged SHA-256s")
    for label in ("exe", "hardening", "package"):
        path = getattr(args, label)
        if path:
            setattr(args, label, path.resolve(strict=True))
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    report = {"status": "failed", "source": args.source,
              "windows_build": sys.getwindowsversion().build, "expect_feedback": args.expect_feedback,
              "desktop": "private Win32 non-input desktop, real Tauri/WebView2 product"}
    report.update(verify_package(args))
    try:
        private = tempfile.TemporaryDirectory(prefix="qz-three-layer-", dir=args.out.parent,
                                              ignore_cleanup_errors=True)
        fixture = Path(private.name).resolve()
        try:
            exercise(args, fixture, report)
        finally:
            deadline = time.monotonic() + 15
            while fixture.exists() and time.monotonic() < deadline:
                assert fixture.parent == args.out.parent and fixture.resolve() == fixture
                private.cleanup()
                if fixture.exists():
                    time.sleep(.2)
            assert not fixture.exists(), "Owned WebView2 profile still in use"
        report["owned_fixture_cleaned"] = True
        report["status"] = "passed"
    finally:
        (args.out / "receipt.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
