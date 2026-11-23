"""Verify replacement confirmation for a stopped batch in the same GUI session."""

import argparse
import json
import os
from pathlib import Path
import platform
import runpy
import tempfile
import time


SCRIPTS = Path(__file__).resolve().parent
batch = runpy.run_path(str(SCRIPTS / "check-batch-extract-desktop.py"))
helpers = runpy.run_path(str(SCRIPTS / "check-nested-multi-layer.py"))
expect = helpers["expect"]


def exercise(args, root, report):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    blocker = root / "local/qing-zip/BatchResume"
    if args.case == "unavailable":
        blocker.parent.mkdir(parents=True)
        blocker.write_bytes(b"ordinary file prevents the private resume directory")
        blocker_sha = batch["digest"](blocker)

    conflict = output / "3-third/file.txt"
    conflict.parent.mkdir()
    conflict.write_bytes(b"preexisting destination sentinel\n")
    original_inputs = batch["snapshot"](inputs)
    original_output = batch["snapshot"](output)
    report.update(original_inputs=original_inputs, original_output=original_output)

    if args.hosted:
        session = batch["launch_hosted"](args.exe, root, args.out / "desktop-session.json", helpers)
    else:
        session = batch["launch_local"](args.exe, root, helpers)
    with session as (page, wait):
        try:
            actual = page.evaluate("window.__TAURI__.core.invoke('current_executable')")
            if Path(actual).resolve(strict=True) != args.exe:
                raise RuntimeError("Attached GUI is not the requested executable")
            report["attached_executable_verified"] = True
            batch["fill_batch"](page, paths, output, expect)
            page.locator("#qa-batch-conflict").select_option("overwrite")
            page.locator('[data-action="batch-start"]').click()
            batch["stop_active_second"](page, report, expect)
            view = batch["candidate_report"](page, expect, "成功 1 · 已取消 1 · 未开始 1")
            if args.case == "unavailable":
                expect(view).to_contain_text("未能保存待继续项目。本次会话仍可继续。")
                report["resume_unavailable_warning_visible"] = True
            before = batch["snapshot"](output)
            assert before["3-third/file.txt"] == original_output["3-third/file.txt"]
            report["before_continue"] = before
            batch["take_screenshot"](page, args.out, "stopped", root, view)

            view.get_by_role("button", name="继续未开始的项目").click()
            dialog = page.get_by_role("dialog", name="确认继续批量解压")
            if args.expect == "confirm":
                expect(dialog).to_be_visible(timeout=10000)
                batch["take_screenshot"](page, args.out, "confirm", root, dialog)
                dialog.get_by_role("button", name="返回").click()
                assert batch["snapshot"](output) == before
                report["return_preserved_output"] = True
                batch["take_screenshot"](page, args.out, "returned", root, view)
                view.get_by_role("button", name="继续未开始的项目").click()
                expect(dialog).to_be_visible()
                dialog.get_by_role("button", name="按替换策略继续").click()
            else:
                batch["candidate_report"](page, expect, "成功 2 · 已取消 1")
                assert dialog.count() == 0
                report["confirmed_no_dialog"] = True

            final = batch["candidate_report"](page, expect, "成功 2 · 已取消 1")
            batch["take_screenshot"](page, args.out, "completed", root, final)
            after = batch["snapshot"](output)
            assert after["3-third/file.txt"] == expected["third"]
            assert after["sentinel.txt"] == original_output["sentinel.txt"]
            assert after["2-second/outside-archive.txt"] == original_output["2-second/outside-archive.txt"]
            assert batch["snapshot"](inputs) == original_inputs
            if args.case == "unavailable":
                assert batch["digest"](blocker) == blocker_sha
                report["private_record_blocker_preserved"] = True
            report.update(after_continue=after, source_and_neighbor_preserved=True,
                          conflicting_target_replaced_after_continue=True)
            page.locator('[data-window="close"]').click()
            report["gui_exit_code"] = wait()
            if report["gui_exit_code"] != 0:
                raise RuntimeError("The test GUI did not exit normally")
        except Exception:
            try:
                batch["take_screenshot"](page, args.out, "failure", root)
            except Exception:
                pass
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("exe", "out", "fixtures-in"):
        parser.add_argument(f"--{field}", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--case", choices=("normal", "unavailable"), default="normal")
    parser.add_argument("--expect", choices=("bypass", "confirm"), required=True)
    parser.add_argument("--hosted", action="store_true")
    parser.add_argument("--package", type=Path)
    parser.add_argument("--hardening", type=Path)
    parser.add_argument("--staged-sha256", nargs=3)
    args = parser.parse_args()
    if args.hosted and (os.environ.get("GITHUB_ACTIONS"), os.environ.get("RUNNER_ENVIRONMENT"),
                        os.environ.get("RUNNER_OS"), os.environ.get("RUNNER_ARCH")) != (
                            "true", "github-hosted", "Windows", "X64"):
        parser.error("--hosted requires a disposable GitHub-hosted Windows x64 runner")
    if bool(args.staged_sha256) == bool(args.package and args.hardening):
        parser.error("provide a ZIP and hardening receipt, or all three staged binary hashes")
    args.exe = args.exe.resolve(strict=True)
    args.out.mkdir(parents=True, exist_ok=False)
    fixture = tempfile.TemporaryDirectory(prefix="qz-r9-same-", dir=args.fixtures_in)
    root = Path(fixture.name).resolve()
    report = {"status": "FAIL", "source": args.source, "case": args.case,
              "expected": args.expect, "platform": platform.platform(),
              "desktop": "GitHub-hosted Windows" if args.hosted else "private non-input Win32"}
    try:
        report.update(helpers["verify_package"](args))
        exercise(args, root, report)
        report["status"] = "PASS"
    except Exception as error:
        report["error"] = batch["text_without_paths"](repr(error), root)
        raise
    finally:
        deadline = time.monotonic() + 15
        while True:
            try:
                fixture.cleanup()
                break
            except OSError:
                if time.monotonic() >= deadline:
                    break
                time.sleep(.25)
        report["fixture_cleanup_verified"] = not root.exists()
        (args.out / "receipt.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        if not report["fixture_cleanup_verified"]:
            raise RuntimeError("Owned fixture cleanup failed")


if __name__ == "__main__":
    main()
