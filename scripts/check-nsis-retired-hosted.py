"""Drive the x64 NSIS retired-file dialog only on a disposable hosted runner."""
import argparse
import ctypes
from ctypes import wintypes as win
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import uuid
import zipfile

TITLE = "qing-zip 旧版文件"
MOVE = {"runtime/zpaq/zpaq.exe", "licenses/7zip-26.05/README.txt"}
MODIFIED = "licenses/python-3.14.9/LICENSE.txt"
SAME_BYTES = "licenses/rust/url-2.5.8/LICENSE-MIT"
SAME_NAME = "runtime/zpaq/zpaq64.exe"
CHANGED_AFTER_PREVIEW = "licenses/rust/form_urlencoded-1.2.2/LICENSE-MIT"
LOCKED_AFTER_PREVIEW = "licenses/7zip-26.05/License.txt"
FIRST_FILE, RECOVERY_EDIT, RESULTS = 200, 102, 103


def require_hosted():
    if os.name != "nt" or any(os.environ.get(key) != value for key, value in {
        "GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted",
        "RUNNER_OS": "Windows", "RUNNER_ARCH": "X64", "GITHUB_REPOSITORY": "qing-yi-5427/qing-zip",
    }.items()):
        raise RuntimeError("requires a disposable GitHub-hosted Windows x64 runner")


def legacy_harness():
    spec = importlib.util.spec_from_file_location("nsis_upgrade_guard", Path(__file__).with_name("check-nsis-upgrade-hosted.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Dialogs:
    def __init__(self):
        self.user = ctypes.WinDLL("user32", use_last_error=True)
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.callback = ctypes.WINFUNCTYPE(win.BOOL, win.HWND, win.LPARAM)
        self.user.EnumWindows.argtypes = [self.callback, win.LPARAM]
        self.user.EnumChildWindows.argtypes = [win.HWND, self.callback, win.LPARAM]
        self.user.GetWindowThreadProcessId.argtypes = [win.HWND, ctypes.POINTER(win.DWORD)]
        self.user.GetWindowTextW.argtypes = [win.HWND, win.LPWSTR, ctypes.c_int]
        self.user.IsWindowVisible.argtypes = [win.HWND]
        self.user.IsWindowEnabled.argtypes = [win.HWND]
        self.user.GetDlgItem.argtypes = [win.HWND, ctypes.c_int]
        self.user.GetDlgItem.restype = win.HWND
        self.user.GetWindowLongW.argtypes = [win.HWND, ctypes.c_int]
        self.user.GetWindowLongW.restype = ctypes.c_long
        self.user.GetWindowRect.argtypes = [win.HWND, ctypes.POINTER(win.RECT)]
        self.user.SetForegroundWindow.argtypes = [win.HWND]
        self.user.PostMessageW.argtypes = [win.HWND, win.UINT, win.WPARAM, win.LPARAM]
        self.user.SendMessageTimeoutW.argtypes = [
            win.HWND, win.UINT, win.WPARAM, win.LPARAM, win.UINT, win.UINT, ctypes.POINTER(ctypes.c_size_t),
        ]
        self.user.SendMessageTimeoutW.restype = win.LPARAM
        self.kernel.OpenProcess.argtypes = [win.DWORD, win.BOOL, win.DWORD]
        self.kernel.OpenProcess.restype = win.HANDLE
        self.kernel.CloseHandle.argtypes = [win.HANDLE]
        self.kernel.CreateFileW.argtypes = [win.LPCWSTR, win.DWORD, win.DWORD, ctypes.c_void_p, win.DWORD, win.DWORD, win.HANDLE]
        self.kernel.CreateFileW.restype = win.HANDLE
        self.kernel.QueryFullProcessImageNameW.argtypes = [win.HANDLE, win.DWORD, win.LPWSTR, ctypes.POINTER(win.DWORD)]

    def windows(self, parent=None):
        results = []

        @self.callback
        def visit(window, _):
            if self.user.IsWindowVisible(window):
                results.append(window)
            return True

        if parent is None:
            self.user.EnumWindows(visit, 0)
        else:
            self.user.EnumChildWindows(parent, visit, 0)
        return results

    def caption(self, window):
        text = ctypes.create_unicode_buffer(1024)
        self.user.GetWindowTextW(window, text, len(text))
        return text.value

    def text(self, window):
        text = ctypes.create_unicode_buffer(4096)
        count = ctypes.c_size_t()
        self.user.SendMessageTimeoutW(window, 0x000D, len(text), ctypes.addressof(text), 2, 150, ctypes.byref(count))
        return text.value

    def pid(self, window):
        process = win.DWORD()
        self.user.GetWindowThreadProcessId(window, ctypes.byref(process))
        return process.value

    def is_exe(self, pid, expected):
        handle = self.kernel.OpenProcess(0x1000, False, pid)
        if not handle:
            return False
        try:
            path = ctypes.create_unicode_buffer(32768)
            length = win.DWORD(len(path))
            if not self.kernel.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(length)):
                return False
            return os.path.normcase(path.value) == os.path.normcase(str(expected))
        finally:
            self.kernel.CloseHandle(handle)

    def click(self, window, button):
        target = self.user.GetDlgItem(window, button)
        if not target or not self.user.IsWindowEnabled(target):
            return False
        return bool(self.user.PostMessageW(window, 0x0111, button, target))



    def message(self, window, message):
        result = ctypes.c_size_t()
        if not self.user.SendMessageTimeoutW(window, message, 0, 0, 2, 1000, ctypes.byref(result)):
            raise RuntimeError(f"control message 0x{message:x} was not delivered")
        return result.value

    def select(self, window, checkbox):
        self.user.SetForegroundWindow(window)
        self.message(checkbox, 0x00F5)  # BM_CLICK
        if self.message(checkbox, 0x00F0) != 1:
            raise AssertionError("native checkbox did not become checked")

    def lock_reader(self, path):
        handle = self.kernel.CreateFileW(str(path), 0x80000000, 1, None, 3, 0, None)
        if handle == win.HANDLE(-1).value:
            raise ctypes.WinError(ctypes.get_last_error())
        return handle

    def screenshot(self, window, filename):
        from PIL import ImageGrab
        self.user.SetForegroundWindow(window)
        time.sleep(.25)
        rectangle = win.RECT()
        if not self.user.GetWindowRect(window, ctypes.byref(rectangle)):
            raise ctypes.WinError(ctypes.get_last_error())
        region = (rectangle.left, rectangle.top, rectangle.right, rectangle.bottom)
        image = ImageGrab.grab(bbox=region)
        if image.width < 100 or image.height < 100 or not any(lo != hi for lo, hi in image.getextrema()):
            raise AssertionError("installer screenshot contains no visible dialog")
        image.save(filename)


def interactive_setup(setup, root, inst, mode, old_paths, trace, evidence, edit_after_preview=None):
    ui = Dialogs()
    process = subprocess.Popen([str(setup), f"/D={inst}"], cwd=root, stdin=subprocess.DEVNULL)
    handled = False
    result_closed = False
    locked = None
    try:
        deadline = time.monotonic() + 150
        while process.poll() is None and time.monotonic() < deadline:
            helper = [w for w in ui.windows()
                      if ui.caption(w) == TITLE and ui.is_exe(ui.pid(w), inst / "qing-zip.exe")]
            if helper:
                window = helper[0]
                result_box = ui.user.GetDlgItem(window, RESULTS)
                if not result_box:
                    raise AssertionError("expected one native list and in-place result")
                if ui.user.IsWindowVisible(result_box):
                    if not result_closed:
                        text = ui.text(result_box)
                        trace.append({"dialog": "summary",
                                      "moved": sorted(name for name in MOVE if f"已移动  {name}" in text),
                                      "retained": sorted(name for name in old_paths if f"未移动  {name}" in text),
                                      "text": text})
                        ui.screenshot(window, evidence / f"{mode}-result.png")
                        if not ui.click(window, 2):
                            raise AssertionError("result close button is unavailable")
                        result_closed = True
                        if locked:
                            ui.kernel.CloseHandle(locked)
                            locked = None
                elif not handled:
                    items = {}
                    for index in range(len(old_paths)):
                        control = ui.user.GetDlgItem(window, FIRST_FILE + index)
                        if control:
                            name = ui.text(control)
                            if name not in old_paths or name in items:
                                raise AssertionError("unexpected or duplicate native checkbox")
                            if ui.user.GetWindowLongW(control, -16) & 0xF != 3 or ui.message(control, 0x00F0) != 0:
                                raise AssertionError("offers must be unchecked native automatic checkboxes")
                            items[name] = control
                    apply = ui.user.GetDlgItem(window, 1)
                    recovery = Path(ui.text(ui.user.GetDlgItem(window, RECOVERY_EDIT)))
                    if not items or not apply or ui.user.IsWindowEnabled(apply):
                        raise AssertionError("empty or preselected native file list")
                    if recovery.parent != inst.parent or not recovery.name.startswith("qing-zip-v0.12.20-Recovery-"):
                        raise AssertionError("recovery location was not displayed in the actual window")
                    trace.append({"dialog": "offer", "candidates": sorted(items), "all_unchecked": True})
                    ui.screenshot(window, evidence / f"{mode}-offer.png")
                    if mode == "repeat":
                        raise AssertionError("repeat unexpectedly offered the old migration")
                    choices = MOVE | {CHANGED_AFTER_PREVIEW, LOCKED_AFTER_PREVIEW} if mode == "selected" else set(sorted(items)[:1])
                    if not choices <= items.keys():
                        raise AssertionError("requested test candidate missing from the window")
                    for name in sorted(choices):
                        ui.select(window, items[name])
                    if not ui.user.IsWindowEnabled(apply):
                        raise AssertionError("checked candidates did not enable the submit button")
                    ui.screenshot(window, evidence / f"{mode}-choices.png")
                    if mode == "selected":
                        edit_after_preview()
                        locked = ui.lock_reader(inst / LOCKED_AFTER_PREVIEW)
                    trace.append({"dialog": "commit" if mode == "selected" else "cancel",
                                  "checked": sorted(choices)})
                    if not ui.click(window, 1 if mode == "selected" else 2):
                        raise AssertionError("configured list action was unavailable")
                    handled = True
            else:
                windows = [w for w in ui.windows() if ui.pid(w) == process.pid and "qing-zip" in ui.caption(w).casefold()]
                for window in windows:
                    next_button = ui.user.GetDlgItem(window, 1)
                    if not next_button:
                        continue
                    # MUI's Next, Agree, Install and Finish share IDOK.
                    label = ui.text(next_button)
                    if label and ui.user.IsWindowEnabled(next_button):
                        stage = {"dialog": "setup", "button": label}
                        new_stage = not trace or trace[-1] != stage
                        if new_stage and "install" in label.casefold() and mode == "selected":
                            ui.screenshot(window, evidence / "installer-before-upgrade.png")
                        if ui.click(window, 1):
                            if new_stage:
                                trace.append(stage)
                            time.sleep(.3)
                            break
            time.sleep(.1)
        if process.poll() is None:
            raise RuntimeError("timed out driving actual NSIS setup windows")
        if process.returncode:
            raise RuntimeError(f"NSIS setup exited {process.returncode}")
    finally:
        if locked:
            ui.kernel.CloseHandle(locked)
        if process.poll() is None:
            process.kill()
            process.wait()


def tree(root, sha):
    return {p.relative_to(root).as_posix(): sha(p) for p in root.rglob("*") if p.is_file()}


def exercise(args, old, runner_temp, mode, shipped, manifest, report):
    root = Path(tempfile.mkdtemp(prefix=f"qz-retired-{mode}-", dir=runner_temp)).resolve()
    current = json.loads((Path(__file__).resolve().parents[1] / "obz-gui/tauri.conf.json").read_text(encoding="utf-8"))["version"]
    direct = mode == "decline"
    drive_root = Path(runner_temp.anchor)
    inst = drive_root / ("qz-retired-cancel-" + uuid.uuid4().hex) if direct else root / "application"
    recovery_parent = inst.parent
    before_recoveries = set(recovery_parent.glob("qing-zip-v0.12.20-Recovery-*"))
    uninstalled = False
    report["status"] = "started"
    try:
        old.check(root.parent == runner_temp and not inst.exists() and not any(c.isspace() for c in str(inst)),
                  "invalid fresh /D path")
        report["drive_root_child"] = direct
        old.check(not old.key_exists(old.ARP) and not old.key_exists(old.INSTALL), "refusing pre-existing registration")
        old.run([args.old_setup, "/S", f"/D={inst}"], root)
        old.installed_version("0.12.20", inst)
        baseline = {name: old.sha(inst / name) for name in manifest}
        old.check(baseline == manifest, "initial retired-file hashes differ from official release")
        report["old_installed"] = baseline
        old.run([args.candidate_stage / "qing-zip.exe", "--review-retired-install-files"], root, timeout=15)
        old.check(set(recovery_parent.glob("qing-zip-v0.12.20-Recovery-*")) == before_recoveries,
                  "mismatched install directory created a recovery")
        report["mismatched_registration_preserved"] = True
        expected = dict(baseline)

        user = inst / "user-data"
        user.mkdir()
        user_copy = user / "zpaq.exe"
        shutil.copyfile(inst / "runtime/zpaq/zpaq.exe", user_copy)
        expected["user-data/zpaq.exe"] = old.sha(user_copy)
        source = root / "input.txt"
        source.write_bytes(b"User archive contents before upgrade.\n" * 256)
        archive = user / "original.zip"
        old.run([inst / "obz.exe", "create", archive, source, "--format", "zip"], root)
        archive_sha = old.sha(archive)
        expected["user-data/original.zip"] = archive_sha
        report["original_zip_sha256"] = archive_sha

        if mode == "selected":
            (inst / MODIFIED).write_bytes((inst / MODIFIED).read_bytes() + b"\nUser annotation\n")
            replacement = inst / SAME_BYTES
            staged = root / "same-bytes-copy"
            shutil.copyfile(replacement, staged)
            original_id = replacement.stat().st_ino
            os.replace(staged, replacement)
            old.check(replacement.stat().st_ino != original_id and old.sha(replacement) == manifest[SAME_BYTES],
                      "identical replacement did not have a distinct file identity")
            (inst / SAME_NAME).write_bytes(b"Independent user file at a historical name; preserve.")
            expected.update({name: old.sha(inst / name) for name in (MODIFIED, SAME_BYTES, SAME_NAME)})
        traces = report.setdefault("dialogs", [])
        before_ids = {name: (inst / name).stat().st_ino for name in MOVE}
        def edit_after_preview():
            path = inst / CHANGED_AFTER_PREVIEW
            path.write_bytes(path.read_bytes() + b"\nUser annotation added while reviewing.\n")
            expected[CHANGED_AFTER_PREVIEW] = old.sha(path)

        if mode == "quiet":
            old.run([args.candidate_setup, "/S", f"/D={inst}"], root)
        else:
            interactive_setup(args.candidate_setup, root, inst, mode, manifest, traces, args.out, edit_after_preview)
        old.installed_version(current, inst)
        old.check(all(old.sha(inst / name) == digest for name, digest in shipped.items()), "candidate file hash mismatch")
        if mode != "quiet":
            old.check(any(d["dialog"] == "offer" for d in traces), "no explicit old file review was shown")
        recoveries = sorted(set(recovery_parent.glob("qing-zip-v0.12.20-Recovery-*")) - before_recoveries)
        selected = MOVE if mode == "selected" else set()
        old.check(len(recoveries) == bool(selected), "unexpected recovery directory count")
        if selected:
            offered = set(next(d["candidates"] for d in traces if d["dialog"] == "offer"))
            old.check(offered == set(manifest) - {MODIFIED, SAME_NAME} and SAME_BYTES in offered,
                      "candidate list included modified or excluded byte-identical unclaimed files")
            result = next((d for d in traces if d["dialog"] == "summary"), None)
            old.check(result is not None and result["moved"] == sorted(MOVE)
                      and result["retained"] == sorted([CHANGED_AFTER_PREVIEW, LOCKED_AFTER_PREVIEW]),
                      "missing mixed success and safe retention in the in-place result")
            user_outcome, separator, technical = result["text"].partition("\r\n\r\n技术信息\r\n")
            old.check(f"未移动  {LOCKED_AFTER_PREVIEW}\r\n文件正被其他程序使用，已保留在原位置。" in user_outcome
                      and separator and f"{LOCKED_AFTER_PREVIEW}\r\npin selected retired file:" in technical
                      and "(os error 32)" in technical and "(os error 32)" not in user_outcome,
                      "occupied-file result did not explain retention and separate the native diagnostic")
            recovery = recoveries[0]
            report["recovered"] = tree(recovery, old.sha)
            old.check(report["recovered"] == {name: expected[name] for name in selected}, "wrong recovery content")
            old.check(all((recovery / name).stat().st_ino == before_ids[name] for name in selected),
                      "move did not preserve the original file objects")
            for name in selected:
                del expected[name]
                old.check(not (inst / name).exists(), "selected object remains in install root")
        elif mode == "decline":
            old.check(any(d["dialog"] == "cancel" for d in traces), "cancel did not exercise a checked item")
            old.check(not any(d["dialog"] == "summary" for d in traces), "cancel unexpectedly committed the list")
        old.check(all(old.sha(inst / name) == digest for name, digest in expected.items()), "retained file changed after upgrade")
        report["retained_after_upgrade"] = dict(expected)

        old.run([inst / "obz.exe", "test", archive], root)
        output = root / "verified-archive-output"
        old.run([inst / "obz.exe", "extract", archive, output], root)
        old.check(old.sha(output / source.name) == old.sha(source) and old.sha(archive) == archive_sha, "original archive changed")
        if selected:
            repeat = report.setdefault("repeat_dialogs", [])
            interactive_setup(args.candidate_setup, root, inst, "repeat", manifest, repeat, args.out)
            old.installed_version(current, inst)
            old.check(all(d["dialog"] == "setup" for d in repeat), "repeat unexpectedly offered old file migration")
            old.check(set(recovery_parent.glob("qing-zip-v0.12.20-Recovery-*")) == before_recoveries | set(recoveries), "repeat created a new recovery path")
            old.check(tree(recovery, old.sha) == report["recovered"], "repeat changed recovery bytes")
        old.uninstall(inst, root)
        uninstalled = True
        report["retained_after_uninstall"] = tree(inst, old.sha)
        old.check(report["retained_after_uninstall"] == expected, "candidate uninstall changed retained files")
        if selected:
            old.check(tree(recovery, old.sha) == report["recovered"], "uninstall changed recovery copies")
            for name in selected:
                original = inst / name
                original.parent.mkdir(parents=True, exist_ok=True)
                os.rename(recovery / name, original)
                old.check(old.sha(original) == manifest[name], "restored bytes differ from original")
        report["status"] = "passed"
    finally:
        if not uninstalled and (inst / "Uninstall.exe").is_file() and old.value(old.INSTALL, "InstallDir") == str(inst):
            try:
                old.uninstall(inst, root)
                uninstalled = True
            except (OSError, RuntimeError, AssertionError):
                report["cleanup_uninstall"] = "not confirmed"
        if uninstalled and root.parent == runner_temp:
            if direct and inst.exists() and inst.resolve() == inst and inst.parent == drive_root and inst.name.startswith("qz-retired-cancel-"):
                shutil.rmtree(inst)
                report["direct_install_cleaned"] = not inst.exists()
            shutil.rmtree(root)
            report["owned_root_cleaned"] = not root.exists()


def main():
    require_hosted()
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("old-setup", "old-portable", "candidate-setup", "candidate-stage", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    old = legacy_harness()
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    old.check(old.sha(args.old_setup) == old.OLD_SETUP and old.sha(args.old_portable) == old.OLD_PORTABLE,
              "prior official downloads do not match")
    old.check(not old.key_exists(old.ARP) and not old.key_exists(old.INSTALL), "unsafe existing registration")
    programs = old.value(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders", "Programs")
    old.check(programs and not (Path(programs) / "qing-zip").exists(), "existing Start menu installation")
    args.out.mkdir(parents=True, exist_ok=False)
    report = {"status": "failed", "candidate_setup_sha256": old.sha(args.candidate_setup),
              "candidate_gui_sha256": old.sha(args.candidate_stage / "qing-zip.exe"), "cases": {}}
    try:
        shipped = tree(args.candidate_stage, old.sha)
        with zipfile.ZipFile(args.old_portable) as prior:
            manifest = {name: old.hashlib.sha256(prior.read(name)).hexdigest()
                        for name in prior.namelist() if not name.endswith("/") and name not in shipped}
        old.check(len(manifest) == 10 and MOVE | {MODIFIED, SAME_BYTES, SAME_NAME, CHANGED_AFTER_PREVIEW, LOCKED_AFTER_PREVIEW} <= set(manifest),
                  "unexpected official retired inventory")
        for mode in ("quiet", "decline", "selected"):
            case = report["cases"][mode] = {}
            exercise(args, old, runner_temp, mode, shipped, manifest, case)
        report["status"] = "passed"
    except Exception as error:
        report["error"] = f"{type(error).__name__}: {error}"[:500]
        raise
    finally:
        (args.out / "receipt.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
