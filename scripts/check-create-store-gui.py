"""Check the Windows creation form against the ZIP it writes. Requires Python Playwright."""
import argparse
import hashlib
import json
import os
import subprocess
import tempfile
import time
import zipfile
from pathlib import Path

from playwright.sync_api import Error, expect, sync_playwright


def debugging_port(profile, process):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("The test window exited. Close other qing-zip instances before running.")
        marker = next(profile.rglob("DevToolsActivePort"), None)
        if marker:
            lines = marker.read_text().splitlines()
            if lines:
                return int(lines[0])
        time.sleep(0.1)
    raise TimeoutError("The test WebView did not open its local debugging port")


def run(executable, output, scratch, before):
    source = scratch / "example.txt"
    source.write_bytes(b"ordinary archive content\n" * 9000)
    archive = scratch / "result.zip"
    profile = scratch / "profile"
    env = {**os.environ, "LOCALAPPDATA": str(scratch / "local"),
           "TEMP": str(scratch), "TMP": str(scratch),
           "WEBVIEW2_USER_DATA_FOLDER": str(profile),
           "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--remote-debugging-port=0 --remote-debugging-address=127.0.0.1"}
    process = subprocess.Popen([executable, "--compress-files", source], env=env, cwd=scratch)
    try:
        port = debugging_port(profile, process)
        with sync_playwright() as automation:
            browser = automation.chromium.connect_over_cdp(f"http://127.0.0.1:{port}")
            page = browser.contexts[0].pages[0]
            level = page.locator("#qa-level")
            expect(level).to_be_visible()
            page.locator("#qa-format").select_option("zip")
            page.locator("#qa-save").fill(str(archive))
            page.locator("[data-action=compression-advanced]").click()
            method = page.locator("#oa-method")
            method.select_option("Deflate")
            level.select_option("0")
            expected_method = "Deflate" if before else "Store"
            expect(method).to_have_value(expected_method)
            expect(level).to_have_value("0")
            page.screenshot(path=output / "form.png")
            page.locator("[data-action=startcompress]").click()
            expect(level).not_to_be_visible(timeout=45000)
            # Wait for the desktop's creation and verification steps to finish.
            expect(page.get_by_text("检查通过", exact=True).first).to_be_visible(timeout=60000)
            with zipfile.ZipFile(archive) as opened:
                entry = opened.getinfo(source.name)
                if opened.read(entry) != source.read_bytes() or opened.testzip() is not None:
                    raise RuntimeError("The created ZIP did not reproduce the input")
                expected_zip_method = zipfile.ZIP_DEFLATED if before else zipfile.ZIP_STORED
                if entry.compress_type != expected_zip_method:
                    raise RuntimeError(f"Unexpected actual ZIP method {entry.compress_type}")
            page.screenshot(path=output / "complete.png")
            receipt = {"mode": "baseline" if before else "candidate", "gui_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
                       "form_method": expected_method, "level": 0, "actual_zip_method": entry.compress_type,
                       "unpacked_bytes": entry.file_size, "packed_bytes": entry.compress_size,
                       "content_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
            print(json.dumps(receipt))
            try:
                page.evaluate('window.__TAURI__.core.invoke("window_action", {action:"close"})')
            except Error:
                pass  # Closing the native window can disconnect the debugger.
    finally:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.wait(timeout=10)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--fixtures-in", type=Path, help="Parent directory for an isolated temporary fixture")
    parser.add_argument("--baseline", action="store_true", help="Verify the old Deflate/level 0 discrepancy")
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixture = tempfile.TemporaryDirectory(prefix="qing-zip-create-", dir=args.fixtures_in)
    try:
        run(args.exe.resolve(strict=True), output, Path(fixture.name).resolve(), args.baseline)
    finally:
        # WebView2 can briefly hold profile/cache handles after the app has exited.
        deadline = time.monotonic() + 20
        while True:
            try:
                fixture.cleanup()
                break
            except PermissionError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.25)
