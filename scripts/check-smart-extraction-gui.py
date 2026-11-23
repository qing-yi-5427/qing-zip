"""Drive real Windows archive entry points against baseline and candidate portable builds."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from urllib.request import urlopen
import zipfile

from websockets.sync.client import connect


def expect(condition, details):
    if not condition:
        raise AssertionError(details)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def sha(path):
    return digest(path.read_bytes())


class Browser:
    def __init__(self):
        self.process = None
        self.socket = None
        self.sequence = 0

    def start(self, executable, scratch, archive=None, debug=None):
        profile = scratch / "webview"
        local = scratch / "local-app-data"
        local.mkdir(parents=True)
        env = dict(
            os.environ,
            LOCALAPPDATA=str(local),
            WEBVIEW2_USER_DATA_FOLDER=str(profile),
            WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=(
                "--remote-debugging-port=0 --remote-debugging-address=127.0.0.1"
            ),
            TEMP=str(scratch),
            TMP=str(scratch),
        )
        self.env = env
        if debug:
            env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = debug["arguments"]
        args = [str(executable)]
        if archive:
            args.append(str(archive))
        self.process = subprocess.Popen(args, env=env, cwd=scratch)
        if debug:
            debug["proc"] = self.process
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            expect(self.process.poll() is None, f"GUI exited during launch: {self.process.returncode}")
            marker = next(profile.rglob("DevToolsActivePort"), None)
            if marker or debug:
                try:
                    port = debug["port"] if debug else int(marker.read_text().splitlines()[0])
                    with urlopen(f"http://127.0.0.1:{port}/json", timeout=1) as response:
                        tab = next(item for item in json.load(response) if item["type"] == "page"
                                   and (not debug or "tauri.localhost" in item.get("url", "")))
                    self.socket = connect(tab["webSocketDebuggerUrl"], open_timeout=3, max_size=16 * 1024 * 1024)
                    break
                except (OSError, StopIteration, ValueError):
                    pass
            time.sleep(0.12)
        expect(self.socket is not None, "No live WebView2 page")
        ready = '#oa-toolbar [data-action=extract-menu]' if archive else '#oa-home-smart-drop'
        self.wait(f"document.querySelector({json.dumps(ready)})?.getClientRects().length")

    def command(self, method, **params):
        self.sequence += 1
        self.socket.send(json.dumps(dict(id=self.sequence, method=method, params=params)))
        while True:
            message = json.loads(self.socket.recv(timeout=12))
            if message.get("id") == self.sequence:
                expect("error" not in message, message)
                return message["result"]

    def evaluate(self, expression):
        answer = self.command("Runtime.evaluate", expression=expression, returnByValue=True)
        expect("exceptionDetails" not in answer, answer)
        return answer["result"].get("value")

    def wait(self, expression, seconds=18):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if value := self.evaluate(expression):
                return value
            time.sleep(0.10)
        raise AssertionError((expression, self.evaluate("document.body.innerText.slice(-4000)")))

    def click(self, selector):
        query = json.dumps(selector)
        self.wait(f"document.querySelector({query})?.getClientRects().length")
        self.evaluate(f'document.querySelector({query}).scrollIntoView({{block:"center"}})')
        x, y = self.evaluate(f"""(() => {{
            const bounds = document.querySelector({query}).getBoundingClientRect();
            return [bounds.x + bounds.width / 2, bounds.y + bounds.height / 2];
        }})()""")
        self.command("Input.dispatchMouseEvent", type="mousePressed", x=x, y=y, button="left", buttons=1, clickCount=1)
        self.command("Input.dispatchMouseEvent", type="mouseReleased", x=x, y=y, button="left", buttons=0, clickCount=1)

    def key(self, key, code, vk, modifiers=0):
        args = dict(key=key, code=code, windowsVirtualKeyCode=vk, modifiers=modifiers)
        self.command("Input.dispatchKeyEvent", type="rawKeyDown", **args)
        self.command("Input.dispatchKeyEvent", type="keyUp", **args)

    def fill(self, selector, value):
        self.click(selector)
        self.key("a", "KeyA", 65, 2)
        self.command("Input.insertText", text=str(value))
        self.wait(f"document.querySelector({json.dumps(selector)})?.value === {json.dumps(str(value))}")

    def screenshot(self, path):
        path.write_bytes(base64.b64decode(self.command("Page.captureScreenshot", format="png")["data"]))

    def close(self):
        if self.socket:
            try:
                self.evaluate('window.__TAURI__.core.invoke("window_action", {action:"close"}); true')
            except Exception:
                pass
            self.socket.close()
        if self.process:
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                self.process.wait(timeout=10)


def fixture_zip(path, member, data):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(member, data)


def complete_job(browser):
    browser.wait("document.querySelector('#qa-overlay').hidden && !document.querySelector('#oa-task-output').hidden")
    browser.click("[data-view=history]")


def task_summary(browser, directory):
    expected = json.dumps(f"已提取 1 个文件至 {directory}")
    return browser.wait(f"Array.from(document.querySelectorAll('.qa-jobresult')).some(e => e.textContent === {expected})")


def confirm_deep_output(browser, directory):
    path = json.dumps(str(directory))
    browser.wait(f"Array.from(document.querySelectorAll('[data-deep-action=output]')).some(e => e.dataset.path === {path})")


def check_standard(browser, destination, output, mode):
    browser.click("#oa-toolbar [data-action=extract-menu]")
    browser.click("#oa-menu [data-action=extract-standard]")
    browser.fill("#qa-destination", destination)
    expect(browser.evaluate('document.querySelector("#qa-smart").checked && !document.querySelector("#oa-recycle-after").checked'),
           "Use the isolated profile with smart extraction on and recycling off.")
    preview = browser.evaluate('document.querySelector("#oa-expected-output").value')
    expect(Path(preview) == destination / "BUNDLE", preview)
    browser.screenshot(output / f"{mode}-preview.png")
    browser.click("[data-action=startextract]")
    complete_job(browser)
    reported = destination if mode == "before" else destination / "BUNDLE"
    task_summary(browser, reported)
    materialized = destination / "BUNDLE/photos/a.txt"
    expect(materialized.read_bytes() == b"original photo", "Mismatched standard extraction output.")
    browser.screenshot(output / f"{mode}-task.png")
    return dict(preview="out/BUNDLE", reported=reported.relative_to(destination.parent).as_posix(),
                file="out/BUNDLE/photos/a.txt", output_sha256=sha(materialized))


def check_deep_and_batch(browser, root, archive):
    deep = root / "deep"
    prior = deep / "BUNDLE/local.txt"
    prior.parent.mkdir(parents=True)
    prior.write_bytes(b"local folder")
    browser.click("[data-view=files]")
    browser.click("#oa-toolbar [data-action=extract-menu]")
    browser.click("#oa-menu [data-action=extract-deep]")
    browser.fill("#qa-destination", deep)
    expect(Path(browser.evaluate('document.querySelector("#oa-expected-output").value')) == deep / "BUNDLE",
           "Unexpected deep extraction estimate.")
    expect("展开后的内容确定实际目录" in browser.evaluate('document.querySelector("#oa-extract-layout-help").textContent'),
           "Missing explanation that the final name can change.")
    browser.click("[data-action=startextract]")
    complete_job(browser)
    resolved = deep / "BUNDLE (2)"
    confirm_deep_output(browser, resolved)
    expect(prior.read_bytes() == b"local folder" and (resolved / "photos/a.txt").read_bytes() == b"original photo",
           "Deep extraction altered the occupied directory or produced different bytes.")

    browser.click("[data-view=tools]")
    browser.click("[data-action=deep-batch]")
    browser.click("details.oa-deep-paste summary")
    browser.fill("#oa-deep-inputs", archive)
    batch = root / "batch"
    browser.fill("#oa-deep-destination", batch)
    expect(Path(browser.evaluate('document.querySelector("#oa-deep-path-0").textContent')) == batch / "BUNDLE",
           "Unexpected batch preview.")
    browser.click("[data-deep-action=start-batch]")
    complete_job(browser)
    confirm_deep_output(browser, batch / "BUNDLE")
    expect((batch / "BUNDLE/photos/a.txt").read_bytes() == b"original photo", "Mismatched batch output.")
    return {
        "deep_with_conflict": {"preview": "deep/BUNDLE", "reported": "deep/BUNDLE (2)", "file": "deep/BUNDLE (2)/photos/a.txt"},
        "toolbox_batch": {"preview": "batch/BUNDLE", "reported": "batch/BUNDLE", "file": "batch/BUNDLE/photos/a.txt"},
    }


def check_shell(browser, executable, root):
    source = root / "shell.zip"
    fixture_zip(source, "SHELL/item.txt", b"shell archive")
    original = sha(source)
    subprocess.run([str(executable), "--smart-extract", str(source)], check=True, cwd=root, env=browser.env, timeout=15)
    task_summary(browser, root / "SHELL")
    output = root / "SHELL/item.txt"
    expect(output.read_bytes() == b"shell archive" and sha(source) == original, "Mismatched shell input or output.")
    return {"reported": "SHELL", "file": "SHELL/item.txt", "source_sha256": original, "output_sha256": sha(output)}


def check_home_picker(executable, root, output):
    from pywinauto import Desktop

    source = root / "home.dat"
    fixture_zip(source, "home/item.txt", b"home archive")
    original = sha(source)
    browser = Browser()
    try:
        browser.start(executable, root / "profile-home")
        browser.click("#oa-home-smart-drop")
        picker = Desktop(backend="uia").window(process=browser.process.pid, title="选择要识别的文件", class_name="#32770")
        picker.wait("visible enabled ready", timeout=15)
        name = picker.child_window(auto_id="1148", control_type="Edit")
        name.set_edit_text(str(source))
        picker.child_window(auto_id="1", control_type="Button").invoke()
        browser.wait("document.querySelector('#oa-deep-path-0')?.getClientRects().length")
        preview = browser.evaluate('document.querySelector("#oa-deep-path-0").textContent')
        expect(Path(preview) == root / "home", preview)
        browser.screenshot(output / "home-picker-preview.png")
        browser.click("[data-deep-action=start-batch]")
        complete_job(browser)
        confirm_deep_output(browser, root / "home")
        extracted = root / "home/item.txt"
        expect(extracted.read_bytes() == b"home archive" and sha(source) == original, "Mismatched home input or output.")
        return {"preview": "home", "reported": "home", "file": "home/item.txt",
                "source_sha256": original, "output_sha256": sha(extracted)}
    finally:
        browser.close()
        time.sleep(0.75)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True, help="Path to the pre-fix portable qing-zip.exe.")
    parser.add_argument("--candidate", type=Path, required=True, help="Path to the fixed portable qing-zip.exe.")
    parser.add_argument("--out", type=Path, required=True, help="New directory for screenshots and the JSON receipt.")
    parser.add_argument("--scratch-parent", type=Path, help="Parent of the temporary fixture. Use a public folder when sharing screenshots.")
    args = parser.parse_args()
    others = subprocess.run(
        ["pwsh", "-NoProfile", "-Command", "Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue | ForEach-Object Id"],
        capture_output=True,
    ).stdout.strip()
    expect(not others, "Close other qing-zip windows before starting.")
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    baseline = args.baseline.resolve(strict=True)
    candidate = args.candidate.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="qing-zip-smart-", dir=args.scratch_parent, ignore_cleanup_errors=True) as scratch:
        root = Path(scratch)
        archive = root / "bundle.zip"
        fixture_zip(archive, "BUNDLE/photos/a.txt", b"original photo")
        original = sha(archive)
        receipt = {"standard_fixture_sha256": original, "binary_sha256": {}}
        for mode, executable in [("before", baseline), ("after", candidate)]:
            receipt["binary_sha256"][mode] = {
                leaf: sha(executable.with_name(leaf)) for leaf in [executable.name, "obz.exe", "obz-worker.exe"]
            }
            profile = root / mode / "profile"
            destination = root / mode / "out"
            destination.mkdir(parents=True)
            browser = Browser()
            try:
                browser.start(executable, profile, archive)
                receipt[mode] = check_standard(browser, destination, output, mode)
                if mode == "after":
                    receipt.update(check_deep_and_batch(browser, root, archive))
                    receipt["shell_startup_switch"] = check_shell(browser, executable, root)
            finally:
                browser.close()
                time.sleep(0.75)
        receipt["home_file_picker"] = check_home_picker(candidate, root, output)
        expect(sha(archive) == original, "Standard source archive changed.")
        receipt["inputs_preserved"] = True
        (output / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
