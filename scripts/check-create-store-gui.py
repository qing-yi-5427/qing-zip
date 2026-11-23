"""Check the Windows creation form against the archive it writes. Requires Python Playwright (and py7zr for 7z)."""
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


def archive_facts(archive, source, archive_format, scratch):
    """Read the method and unpacked content without using qing-zip or its engines."""
    if archive_format == "zip":
        with zipfile.ZipFile(archive) as opened:
            entry = opened.getinfo(source.name)
            if opened.testzip() is not None:
                raise RuntimeError("The created ZIP failed its CRC check")
            data = opened.read(entry)
            stored = entry.compress_type == zipfile.ZIP_STORED
            return {"actual_method": entry.compress_type, "stored": stored,
                    "unpacked_bytes": entry.file_size, "packed_bytes": entry.compress_size,
                    "unpacked_sha256": hashlib.sha256(data).hexdigest()}
    import py7zr  # Independent 7z reader, only needed for --format 7z.
    with py7zr.SevenZipFile(archive) as opened:
        methods = list(opened.archiveinfo().method_names)
        target = scratch / "unpacked"
        opened.extractall(path=target)
    data = (target / source.name).read_bytes()
    return {"actual_method": methods, "stored": methods == ["COPY"],
            "unpacked_bytes": len(data), "packed_bytes": archive.stat().st_size,
            "unpacked_sha256": hashlib.sha256(data).hexdigest()}


def run(executable, output, scratch, before, scenario, archive_format):
    source = scratch / "example.txt"
    source.write_bytes(b"ordinary archive content\n" * 9000)
    archive = scratch / f"result.{archive_format}"
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
            method = page.locator("#oa-method")
            expect(level).to_be_visible()
            page.locator("#qa-format").select_option(archive_format)
            page.locator("#qa-save").fill(str(archive))
            if scenario == "store":
                page.locator("[data-action=compression-advanced]").click()
                method.select_option("Deflate" if archive_format == "zip" else "LZMA2")
                level.select_option("0")
                form_level = "0"
                expected_stored = not before
            else:
                # Only the visible level changes; the advanced panel stays collapsed.
                level.select_option("0")
                level.select_option("5")
                expect(method).to_be_hidden()
                form_level = "5"
                expected_stored = before
            expect(level).to_have_value(form_level)
            form_method = method.input_value()
            page.screenshot(path=output / "form.png")
            page.locator("[data-action=startcompress]").click()
            expect(level).not_to_be_visible(timeout=45000)
            # Wait for the desktop's creation and verification steps to finish.
            expect(page.get_by_text("检查通过", exact=True).first).to_be_visible(timeout=60000)
            page.screenshot(path=output / "complete.png")
            facts = archive_facts(archive, source, archive_format, scratch)
            receipt = {"mode": "baseline" if before else "candidate", "scenario": scenario, "format": archive_format,
                       "gui_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
                       "form_level": int(form_level), "form_method": form_method or "自动", **facts,
                       "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
            (output / "receipt.json").write_text(json.dumps(receipt, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            print(json.dumps(receipt, ensure_ascii=False))
            if facts["unpacked_sha256"] != receipt["source_sha256"]:
                raise RuntimeError("The created archive did not reproduce the input")
            if facts["stored"] != expected_stored:
                raise RuntimeError(f"Unexpected actual method {facts['actual_method']} for form level {form_level}")
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
    parser.add_argument("--baseline", action="store_true", help="Expect the old discrepancy of the chosen scenario")
    parser.add_argument("--scenario", choices=["store", "roundtrip"], default="store",
                        help="store: explicit method, then level 0; roundtrip: level 0, then 5 with advanced settings collapsed")
    parser.add_argument("--format", choices=["zip", "7z"], default="zip")
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixture = tempfile.TemporaryDirectory(prefix="qing-zip-create-", dir=args.fixtures_in)
    try:
        run(args.exe.resolve(strict=True), output, Path(fixture.name).resolve(), args.baseline, args.scenario, args.format)
    finally:
        # WebView2 can briefly hold profile/cache handles after the app has exited.
        deadline = time.monotonic() + 20
        while True:
            try:
                fixture.cleanup()
                break
            except OSError:  # PermissionError, or Crashpad still writing (WinError 145).
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.25)
