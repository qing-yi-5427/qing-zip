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
import zipfile

TITLE = "qing-zip 旧版文件"
MOVE = {"runtime/zpaq/zpaq.exe", "licenses/7zip-26.05/README.txt"}
MODIFIED = "licenses/python-3.14.9/LICENSE.txt"
SAME_BYTES = "licenses/rust/url-2.5.8/LICENSE-MIT"
SAME_NAME = "runtime/zpaq/zpaq64.exe"


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
        self.user.PostMessageW.argtypes = [win.HWND, win.UINT, win.WPARAM, win.LPARAM]
        self.user.SendMessageTimeoutW.argtypes = [
            win.HWND, win.UINT, win.WPARAM, win.LPARAM, win.UINT, win.UINT, ctypes.POINTER(ctypes.c_size_t),
        ]
        self.user.SendMessageTimeoutW.restype = win.LPARAM
        self.kernel.OpenProcess.argtypes = [win.DWORD, win.BOOL, win.DWORD]
        self.kernel.OpenProcess.restype = win.HANDLE
        self.kernel.CloseHandle.argtypes = [win.HANDLE]
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


def interactive_setup(setup, root, inst, mode, old_paths, trace):
    ui = Dialogs()
    process = subprocess.Popen([str(setup), f"/D={inst}"], cwd=root, stdin=subprocess.DEVNULL)
    last = None
    try:
        deadline = time.monotonic() + 150
        while process.poll() is None and time.monotonic() < deadline:
            helper = [w for w in ui.windows()
                      if ui.caption(w) == TITLE and ui.is_exe(ui.pid(w), inst / "qing-zip.exe")]
            if helper:
                window = helper[0]
                text = "\n".join(ui.text(child) for child in ui.windows(window))
                signature = (window, text)
                if signature != last:
                    if "是否逐项查看" in text:
                        trace.append({"dialog": "offer"})
                        button = 7 if mode == "decline" else 6
                    elif "要将这一项移至恢复目录" in text:
                        name = next((name for name in old_paths if name in text), None)
                        if name is None:
                            raise AssertionError("unrecognized candidate prompt")
                        trace.append({"dialog": "candidate", "relative_path": name})
                        button = 6 if name in MOVE else 7
                    elif "文件保留原相对路径" in text:
                        trace.append({"dialog": "summary", "moved": sorted(name for name in MOVE if f"已移至恢复目录  {name}" in text)})
                        button = 1
                    else:
                        trace.append({"dialog": "other"})
                        button = 1
                    if ui.click(window, button):
                        last = signature
                        time.sleep(.2)
                        continue
            else:
                windows = [w for w in ui.windows() if ui.pid(w) == process.pid and "qing-zip" in ui.caption(w).casefold()]
                for window in windows:
                    next_button = ui.user.GetDlgItem(window, 1)
                    if not next_button:
                        continue
                    # MUI's Next, Agree, Install and Finish all use IDOK.
                    # The installer owns this window and no optional boxes are changed.
                    label = ui.text(next_button)
                    if label and ui.click(window, 1):
                        stage = {"dialog": "setup", "button": label}
                        if not trace or trace[-1] != stage:
                            trace.append(stage)
                        time.sleep(.3)
                        break
            time.sleep(.1)
        if process.poll() is None:
            raise RuntimeError("timed out driving actual NSIS setup windows")
        if process.returncode:
            raise RuntimeError(f"NSIS setup exited {process.returncode}")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


def tree(root, sha):
    return {p.relative_to(root).as_posix(): sha(p) for p in root.rglob("*") if p.is_file()}


def exercise(args, old, runner_temp, mode, shipped, manifest, report):
    root = Path(tempfile.mkdtemp(prefix=f"qz-retired-{mode}-", dir=runner_temp)).resolve()
    inst = root / "application"
    uninstalled = False
    report["status"] = "started"
    try:
        old.check(root.parent == runner_temp and not any(c.isspace() for c in str(inst)), "invalid private /D path")
        old.check(not old.key_exists(old.ARP) and not old.key_exists(old.INSTALL), "refusing pre-existing registration")
        old.run([args.old_setup, "/S", f"/D={inst}"], root)
        old.installed_version("0.12.20", inst)
        baseline = {name: old.sha(inst / name) for name in manifest}
        old.check(baseline == manifest, "initial retired-file hashes differ from official release")
        report["old_installed"] = baseline
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
        interactive_setup(args.candidate_setup, root, inst, mode, manifest, traces)
        old.installed_version("0.12.23", inst)
        old.check(all(old.sha(inst / name) == digest for name, digest in shipped.items()), "candidate file hash mismatch")
        old.check(any(d["dialog"] == "offer" for d in traces), "no explicit old file review was shown")
        recoveries = list(root.glob("qing-zip-v0.12.20-Recovery-*"))
        selected = MOVE if mode == "selected" else set()
        old.check(len(recoveries) == bool(selected), "unexpected recovery directory count")
        if selected:
            offered = {d["relative_path"] for d in traces if d["dialog"] == "candidate"}
            old.check(offered == set(manifest) - {MODIFIED, SAME_NAME} and SAME_BYTES in offered,
                      "candidate list included modified or excluded byte-identical unclaimed files")
            old.check(any(d.get("moved") == sorted(MOVE) for d in traces), "missing actual relocation summary")
            recovery = recoveries[0]
            report["recovered"] = tree(recovery, old.sha)
            old.check(report["recovered"] == {name: expected[name] for name in selected}, "wrong recovery content")
            old.check(all((recovery / name).stat().st_ino == before_ids[name] for name in selected),
                      "move did not preserve the original file objects")
            for name in selected:
                del expected[name]
                old.check(not (inst / name).exists(), "selected object remains in install root")
        else:
            old.check(not any(d["dialog"] == "candidate" for d in traces), "decline unexpectedly entered per-file review")
        old.check(all(old.sha(inst / name) == digest for name, digest in expected.items()), "retained file changed after upgrade")
        report["retained_after_upgrade"] = dict(expected)

        old.run([inst / "obz.exe", "test", archive], root)
        output = root / "verified-archive-output"
        old.run([inst / "obz.exe", "extract", archive, output], root)
        old.check(old.sha(output / source.name) == old.sha(source) and old.sha(archive) == archive_sha, "original archive changed")
        if selected:
            repeat = report.setdefault("repeat_dialogs", [])
            interactive_setup(args.candidate_setup, root, inst, "repeat", manifest, repeat)
            old.installed_version("0.12.23", inst)
            old.check(all(d["dialog"] == "setup" for d in repeat), "repeat unexpectedly offered old file migration")
            old.check(list(root.glob("qing-zip-v0.12.20-Recovery-*")) == recoveries, "repeat created a new recovery path")
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
        old.check(len(manifest) == 10 and MOVE | {MODIFIED, SAME_BYTES, SAME_NAME} <= set(manifest),
                  "unexpected official retired inventory")
        for mode in ("decline", "selected"):
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
