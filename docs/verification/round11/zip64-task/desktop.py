"""Open a reproducible, valid 4 GiB+ ZIP in a private installed qing-zip window."""
import argparse
import hashlib
import importlib.util
import json
import shutil
import tempfile
import time
from pathlib import Path

import win32event
import win32process
from playwright.sync_api import expect


def load_module(name, script):
    spec = importlib.util.spec_from_file_location(name, script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def extract_large_member(page, work, out, expected):
    destination = work / "output"
    destination.mkdir()
    page.locator("[data-action=extract-menu]").click()
    page.locator("[data-action=extract-standard]").click()
    expect(page.locator("#qa-destination")).to_be_visible()
    page.locator("#qa-destination").fill(str(destination))
    page.locator("#qa-smart").uncheck()
    started = time.monotonic()
    page.locator("[data-action=startextract]").click()
    panel = page.locator("#oa-task-panel")
    expect(panel).to_be_visible(timeout=90000)
    header = panel.locator("[aria-controls=oa-task-body]")
    if header.get_attribute("aria-expanded") == "false":
        header.click()
    samples = []
    captured = False
    while time.monotonic() - started < 120:
        view = panel.evaluate("""p => {
          const text = id => p.querySelector(id)?.innerText || '';
          const bar = p.querySelector('#oa-task-progress');
          return { phase: text('#oa-task-phase'), summary: text('#oa-task-compact-phase'),
                   metric: text('#oa-task-metric'), percent: bar?.getAttribute('aria-valuenow') };
        }""")
        if not samples or any(samples[-1].get(k) != v for k,v in view.items()):
            samples.append({**view, "elapsed_s": round(time.monotonic() - started, 3)})
        if "已完成" in view["summary"]:
            break
        if not captured and view["percent"] is not None and 0 < float(view["percent"]) < 100:
            panel.screenshot(path=str(out / "installed-running.png"))
            captured = True
        page.wait_for_timeout(100)
    else:
        raise RuntimeError("No completed task observed in the installed desktop")
    button = panel.locator("#oa-task-output")
    expect(button).to_be_visible()
    panel.screenshot(path=str(out / "installed-complete.png"))
    entries = list(destination.rglob("*"))
    if len(entries) != 1 or entries[0].name != "ordinary-member.bin":
        raise RuntimeError("Unexpected files in the extraction destination")
    target = entries[0]
    with target.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if target.stat().st_size != expected["requested_member_bytes"] or digest != expected["extracted_member"]["sha256"]:
        raise RuntimeError("Installed GUI output mismatch")
    return {"samples": samples, "running_screenshot": captured, "completion_action": button.inner_text(),
            "output_bytes": target.stat().st_size, "output_sha256": digest}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repo", "exe", "scratch", "out", "member_receipt"):
        parser.add_argument("--" + name.replace("_", "-"), type=Path, required=True)
    parser.add_argument("--extract", action="store_true")
    args = parser.parse_args()
    root = args.scratch.resolve(strict=True)
    if shutil.disk_usage(root).free < 12 * 1024**3:
        raise RuntimeError("Provide at least 12 GiB temporary space")
    args.out.mkdir(exist_ok=False)
    fixture_module = load_module("member", args.repo / "docs/verification/round11/zip64-size/member.py")
    desktop = load_module("desktop", args.repo / "scripts/check-nested-multi-layer.py")
    expected = json.loads(args.member_receipt.read_text(encoding="utf8"))["cases"][-1]
    size = expected["requested_member_bytes"]
    exe = args.exe.resolve(strict=True)
    receipt = {"status": "FAIL", "member_bytes": size, "gui_sha256": fixture_module.checksum(exe)}
    try:
        with tempfile.TemporaryDirectory(prefix="zip64-window-", dir=root, ignore_cleanup_errors=True) as tmp:
            work = Path(tmp)
            source = work / fixture_module.MEMBER
            archive = work / "ordinary-large.zip"
            generated, _ = fixture_module.fixture(source, size)
            fixture_module.create_zip(source, archive)
            before = fixture_module.checksum(archive)
            if generated != expected["synthetic_input"] or before != expected["archive_before"]["sha256"]:
                raise RuntimeError("New synthetic archive differs from the fully verified CLI archive")
            receipt["archive_sha256"] = before
            with desktop.private_process(exe, work, archive) as (page, handle, pid):
                row = page.locator('[data-entry="ordinary-member.bin"]')
                expect(row).to_be_visible(timeout=90000)
                actual = page.evaluate("window.__TAURI__.core.invoke('current_executable')")
                if Path(actual).resolve() != exe:
                    raise RuntimeError("Unexpected GUI executable")
                receipt["row"] = row.inner_text()
                row.click()
                page.keyboard.press("Alt+Enter")
                info = page.get_by_role("dialog")
                expect(info).to_be_visible()
                properties = {pair.locator("dt").inner_text(): pair.locator("dd").inner_text()
                              for pair in info.locator("dl.qa-properties > div").all()}
                if properties.get("大小") != f"{size:,} 字节":
                    raise RuntimeError(f"Incorrect exact file size: {properties.get('大小')}")
                receipt["fields"] = {key: properties[key] for key in ("大小", "压缩后大小", "包内路径")}
                receipt["dpr"] = page.evaluate("window.devicePixelRatio")
                info.screenshot(path=str(args.out / "installed-properties.png"))
                info.locator('[data-action=close]').last.click()
                if args.extract:
                    receipt["task"] = extract_large_member(page, work, args.out, expected)
                page.locator('[data-window=close]').click()
                if win32event.WaitForSingleObject(handle, 20000) != win32event.WAIT_OBJECT_0:
                    raise RuntimeError("Installed window did not exit")
                receipt["exit_code"] = win32process.GetExitCodeProcess(handle)
            receipt["archive_preserved"] = fixture_module.checksum(archive) == before
            if receipt["exit_code"] or not receipt["archive_preserved"]:
                raise RuntimeError("Unexpected exit code or changed archive")
        receipt["status"] = "PASS"
    except Exception as err:
        receipt["error"] = str(err)
        raise
    finally:
        # WebView2 can briefly hold its private profile open after the application exits.
        for _ in range(24):
            if "work" not in locals() or not work.exists():
                break
            if work.resolve().parent != root or not work.name.startswith("zip64-window-") or work.is_symlink() or work.is_junction():
                raise RuntimeError("Unexpected temporary directory")
            time.sleep(0.5)
            shutil.rmtree(work, ignore_errors=True)
        receipt["fixture_removed"] = "work" in locals() and not work.exists()
        if not receipt["fixture_removed"]:
            receipt["status"] = "FAIL"
        (args.out / "receipt.json").write_text(json.dumps(receipt, indent=2, ensure_ascii=False), encoding="utf8")
    if not receipt["fixture_removed"]:
        raise RuntimeError("Private fixture cleanup incomplete")


if __name__ == "__main__":
    main()
