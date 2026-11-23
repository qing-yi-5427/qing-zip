"""Verify extraction and restoration of script-owned ZIP fixtures on a fresh hosted Windows runner."""
import argparse
import json
import os
from pathlib import Path
import runpy
import tempfile
import time
from uuid import uuid4
from zipfile import ZIP_DEFLATED, ZipFile


CONTENTS = {"one.txt": b"first published file\n", "two.txt": b"second published file\n"}
EXISTING = b"pre-existing destination data\n"
# FMTID_Displaced / PID_DISPLACED_FROM in the Windows SDK, shlguid.h.
ORIGINAL_FOLDER = "{9B174B33-40FF-11D2-A27E-00C04FC38A71} 2"


def until(probe, description):
    deadline = time.monotonic() + 16
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(0.12)
    raise AssertionError(f"Did not observe {description}")


def recycled_item(shell, archive):
    from pywintypes import com_error

    folder = shell.NameSpace(10)  # ssfBITBUCKET
    if folder is None:
        raise RuntimeError("The Windows Recycle Bin namespace is unavailable")
    matches = []
    for item in folder.Items():
        if item.Name not in (archive.name, archive.stem):
            continue
        try:
            original = item.ExtendedProperty(ORIGINAL_FOLDER)
        except com_error:
            continue
        if original and Path(original) == archive.parent:
            matches.append(item)
    if len(matches) > 1:
        raise AssertionError("Several Recycle Bin entries matched the unique fixture")
    return matches[0] if matches else None


def restore_fixture(shell, archive):
    item = until(lambda: recycled_item(shell, archive), "the fixture in the Windows Recycle Bin")
    verbs = [verb for verb in item.Verbs() if verb.Name.replace("&", "").strip().casefold() in ("restore", "还原")]
    if len(verbs) != 1 or archive.exists():
        raise AssertionError("Could not unambiguously restore the single absent fixture")
    verbs[0].DoIt()
    until(lambda: archive.is_file() and recycled_item(shell, archive) is None,
          "the recovered archive at its original location")


def choose_skip(browser):
    browser.click("#qa-conflict")
    browser.key("Home", "Home", 36)
    browser.key("ArrowDown", "ArrowDown", 40)
    browser.key("ArrowDown", "ArrowDown", 40)
    browser.key("Enter", "Enter", 13)
    browser.wait('document.querySelector("#qa-conflict").value === "skip"')


def exercise(name, executable, case, output, shell, debug, gui, report):
    archive = case / f"qing-recycle-{name}-{uuid4().hex}.zip"
    with ZipFile(archive, "x", ZIP_DEFLATED) as fixture:
        for file, contents in CONTENTS.items():
            fixture.writestr(file, contents)
    sha = gui["sha"]
    original = sha(archive)
    target = case / "output"
    target.mkdir()
    if name in ("skip", "conflict"):
        (target / "one.txt").write_bytes(EXISTING)
    browser = gui["Browser"]()
    snapshot = {"source_before_sha256": original}
    report["cases"][name] = snapshot
    try:
        browser.start(executable, case / "profile", archive, debug=debug)
        identity = browser.command("Runtime.evaluate", expression="window.__TAURI__.core.invoke('current_executable')",
                                   returnByValue=True, awaitPromise=True)
        if Path(identity["result"]["value"]).resolve() != executable:
            raise AssertionError("The WebView does not belong to the staged product")
        if name == "selected":
            browser.click('[data-select="one.txt"]')
        browser.click('[data-action="extract-menu"]')
        browser.click('[data-action="extract-recycle"]')
        browser.fill("#qa-destination", target)
        if browser.evaluate('document.querySelector("#qa-smart").checked'):
            browser.click("#qa-smart")
        browser.wait('document.querySelector("#oa-recycle-after").checked')
        if name == "selected":
            browser.click('input[name="extract-scope"][value="selected"]')
        if name == "skip":
            choose_skip(browser)
        blocked = name in ("selected", "skip")
        snapshot["recycle_option_blocked"] = browser.evaluate('''(() => {
            const input = document.querySelector("#oa-recycle-after");
            return input.disabled && !input.checked;
        })()''')
        if blocked != snapshot["recycle_option_blocked"]:
            raise AssertionError("Unexpected recycle eligibility for the chosen extraction options")
        browser.screenshot(output / f"{name}-options.png")
        browser.click('[data-action="startextract"]')
        terminal = "failed" if name == "conflict" else "complete"
        browser.wait('document.querySelector("#qa-overlay").hidden')
        job = browser.wait('Number(document.querySelector("#oa-task-output").dataset.jobOutput)')
        card = f'[data-job-card="{job}"]'
        status = f'{card} .qa-badge[data-state="{terminal}"]'
        browser.click('[data-view="history"]')
        browser.wait(f'document.querySelector({json.dumps(status)})?.getClientRects().length')
        result_selector = json.dumps(card + " .qa-jobresult")
        condition = '.includes("原压缩包已移入回收站")' if name == "full" else ''
        browser.wait(f'document.querySelector({result_selector})?.textContent{condition}')
        result = browser.evaluate(f'document.querySelector({result_selector}).textContent')
        snapshot["result"] = result.replace(str(case), "<fixture>")
        snapshot["terminal"] = terminal
        snapshot["source_present_after_extraction"] = archive.is_file()
        browser.screenshot(output / f"{name}-result.png")
        expected = {
            "selected": {"one.txt": CONTENTS["one.txt"]},
            "skip": {"one.txt": EXISTING, "two.txt": CONTENTS["two.txt"]},
            "conflict": {"one.txt": EXISTING},
            "full": CONTENTS,
        }[name]
        actual = {file.name: file.read_bytes() for file in target.iterdir()}
        if actual != expected:
            raise AssertionError(f"{name}: output files did not match the expected contents")
        if name == "full":
            if archive.exists() or "原压缩包已移入回收站" not in result:
                raise AssertionError("Complete extraction did not report a Recycle Bin move")
            restore_fixture(shell, archive)
            snapshot["restored_from_recycle_bin"] = True
        elif not archive.is_file() or recycled_item(shell, archive) is not None:
            raise AssertionError(f"{name}: an ineligible source was recycled")
        snapshot["source_final_sha256"] = sha(archive)
        if snapshot["source_final_sha256"] != original:
            raise AssertionError(f"{name}: the archive bytes changed")
        snapshot["output_sha256"] = {file: sha(target / file) for file in actual}
    finally:
        browser.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    if os.name != "nt" or os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("This test requires a disposable GitHub-hosted Windows desktop")

    from arm_gui_lifecycle import hosted_debug_session
    from win32com.client import Dispatch

    executable = args.exe.resolve(strict=True)
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    gui = runpy.run_path(str(Path(__file__).with_name("check-smart-extraction-gui.py")))
    report = {"passed": False, "gui_sha256": gui["sha"](executable), "cases": {}}
    try:
        shell = Dispatch("Shell.Application")
        if shell.NameSpace(10) is None:
            raise RuntimeError("The runner does not expose a Recycle Bin namespace")
        with tempfile.TemporaryDirectory(dir=os.environ["RUNNER_TEMP"], prefix="qing-recycle-", ignore_cleanup_errors=True) as scratch:
            with hosted_debug_session(executable, output / "desktop-session.json") as debug:
                for name in ("selected", "skip", "conflict", "full"):
                    case = Path(scratch) / name
                    case.mkdir()
                    exercise(name, executable, case, output, shell, debug, gui, report)
        report["passed"] = True
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (output / "receipt.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
