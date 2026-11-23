"""Verify recovery of the dirty middle archive after a real Windows GUI restart."""

import argparse
from contextlib import contextmanager, suppress
import json
import os
from pathlib import Path
import runpy
import socket
import subprocess
import tempfile
import time
import winreg


def require_hosted_windows():
    if (os.environ.get("GITHUB_ACTIONS"), os.environ.get("RUNNER_ENVIRONMENT"), os.environ.get("RUNNER_OS")) != (
        "true", "github-hosted", "Windows"
    ):
        raise RuntimeError("Disposable GitHub-hosted Windows required")


@contextmanager
def launch(exe, root, tag, archive, evidence):
    from playwright.sync_api import sync_playwright
    from arm_gui_lifecycle import guarded_run
    from nested_restart_gui import connect

    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    arguments = f"--remote-debugging-port={port} --remote-debugging-address=127.0.0.1"
    env = dict(os.environ, LOCALAPPDATA=str(root / "local"), APPDATA=str(root / "roaming"),
               TEMP=str(root / "transient"), TMP=str(root / "transient"),
               WEBVIEW2_USER_DATA_FOLDER=str(root / f"webview-{tag}"), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=arguments)
    policy = winreg.CreateKeyEx(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments",
                                0, winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY)
    session = {"passed": False}
    with guarded_run(winreg, policy, exe.name, arguments, session, evidence / f"desktop-{tag}.json") as lifetime:
        with sync_playwright() as driver:
            command = [str(exe)]
            if archive:
                command.append(str(archive))
            process = subprocess.Popen(command, cwd=root, env=env, stdin=subprocess.DEVNULL)
            lifetime["proc"] = process
            browser, page = connect(process, port, driver)
            page.set_default_timeout(15000)
            yield process, browser, page
            session["passed"] = True


def verify_export(recovered, entry, changed_name, qa):
    read = qa["checked_members"]
    digest = qa["digest"]
    assert digest(recovered) == entry["retained_private_middle_sha256"]
    middle = read(recovered)
    assert set(middle) == {"inner.zip", "middle-peer.txt"}
    assert digest(middle["middle-peer.txt"]) == entry["middle_peer_sha256"]
    assert digest(middle["inner.zip"]) == entry["edited_inner_sha256"]
    inner = read(middle["inner.zip"])
    assert set(inner) == {changed_name, "inner-peer.txt"}
    assert digest(inner[changed_name]) == entry["edited_leaf_sha256"]
    assert digest(inner["inner-peer.txt"]) == entry["inner_peer_sha256"]
    return {"archive_sha256": digest(recovered), "middle_members": sorted(middle), "inner_members": sorted(inner)}


def exercise(args, root, receipt, qa):
    from playwright.sync_api import Error, expect
    from nested_restart_gui import retained_dialog, save_retained_copy, stop_own

    for name in ("local", "roaming", "transient"):
        (root / name).mkdir()
    originals = {}
    pids = set()
    for tag in ("project-a", "project-b"):
        case = root / tag
        case.mkdir()
        initial, sibling = qa["prepare_case"](case, leaf_payload=(tag + " ordinary content\n").encode())
        modified_name = f"edited-{tag}.txt"
        entry = receipt["cases"][tag] = {"status": "started"}
        with launch(args.exe, root, tag, initial, args.out) as (process, browser, page):
            assert process.pid not in pids
            pids.add(process.pid)
            _, middle = qa["exercise_conflict"](page, root, initial, sibling, entry, True, modified_name)
            page.screenshot(path=args.out / f"{tag}-conflict.png")
            assert process.poll() is None, "startup GUI exited before controlled stop"
            stop_own(process, browser)
            assert qa["digest"](middle) == entry["retained_private_middle_sha256"]
        originals[tag] = (initial, sibling, modified_name)
        entry["status"] = "preserved_after_owned_gui_termination"

    first, _, _ = originals["project-a"]
    with launch(args.exe, root, "cold", first, args.out) as (process, browser, page):
        assert process.pid not in pids
        expect(page.locator('[data-entry="middle.zip"]')).to_be_visible(timeout=60000)
        assert not qa["sessions"](page, root)
        retained_dialog(page, args.out, "cold-same-name")
        select = page.locator("#oa-retained-edit")
        expect(select).to_be_visible()
        options = select.locator("option")
        expect(options).to_have_count(2)
        labels = [label.replace(str(root), "<test-root>") for label in options.all_text_contents()]
        assert all("middle.zip" in label for label in labels), labels
        matching = {tag: [index for index, label in enumerate(labels) if tag in label.casefold()] for tag in originals}
        assert all(len(indices) == 1 for indices in matching.values()) and matching["project-a"] != matching["project-b"], labels
        receipt["visible_same_name_sources"] = labels
        receipt["fresh_process_without_live_nested_sessions"] = True

        chosen = "project-b"
        # Cancel an actual native save dialog before committing any export.
        from pywinauto import Desktop
        select.select_option(index=matching[chosen][0])
        page.locator('[data-action="external-retained-save"]').click()
        cancel_dialog = Desktop(backend="uia").window(process=process.pid, class_name="#32770")
        cancel_dialog.wait("visible", timeout=20)
        cancel_dialog.capture_as_image().save(args.out / "save-as-before-cancel.png")
        cancel_dialog.child_window(auto_id="2", control_type="Button").click_input()
        expect(page.locator('#oa-retained-edit option')).to_have_count(2)
        receipt['cancelled_save_retained_both_records'] = True
        select.select_option(index=matching[chosen][0])
        page.screenshot(path=args.out / "cold-selected-project-b.png")
        external = root / "export"
        external.mkdir()
        recovered = external / "recovered-middle.zip"
        save_retained_copy(page, process, recovered, args.out)
        _, _, modified_name = originals[chosen]
        receipt["export"] = verify_export(recovered, receipt["cases"][chosen], modified_name, qa)
        receipt["export"]["selected_source"] = chosen
        receipt["save_failure_retry"] = "not observed"
        assert {entry.name for entry in external.iterdir()} == {"recovered-middle.zip"}
        assert qa["digest"](recovered) != receipt["cases"]["project-a"]["retained_private_middle_sha256"]
        with suppress(Error):
            page.locator('[data-window="close"]').click()
        assert process.wait(timeout=20) == 0

    with launch(args.exe, root, "cold-again", first, args.out) as (process, browser, page):
        expect(page.locator('[data-entry="middle.zip"]')).to_be_visible(timeout=60000)
        retained_dialog(page, args.out, "cold-again")
        labels = [label.replace(str(root), "<test-root>") for label in page.locator('#oa-retained-edit option').all_text_contents()]
        assert len(labels) == 1 and "project-a" in labels[0].casefold() and "project-b" not in labels[0].casefold(), labels
        receipt['after_export_remaining_labels'] = labels
        receipt['successful_save_consumed_only_project_b'] = True
        stop_own(process, browser)

    for tag, (initial, sibling, _) in originals.items():
        entry = receipt["cases"][tag]
        assert qa["digest"](initial) == entry["external_outer_sha256"]
        assert qa["digest"](sibling) == entry["filesystem_peer_sha256"]
        assert "leaf.txt" in qa["checked_members"](qa["checked_members"](qa["checked_members"](initial)["middle.zip"])["inner.zip"])
        entry["outer_and_neighbour_preserved_after_cold_export"] = True


def main():
    require_hosted_windows()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", type=Path, default=Path("nested-restart-evidence/three-layer-recovery"))
    args = parser.parse_args()
    args.exe = args.exe.resolve(strict=True)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    qa = runpy.run_path(str(Path(__file__).with_name("check-nested-multi-layer.py")))
    runner = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    report = {"status": "failed", "gui_sha256": qa["digest"](args.exe), "cases": {},
              "desktop": "GitHub-hosted Windows UI and native Save As"}
    try:
        private = tempfile.TemporaryDirectory(prefix="qz-nested-multi-recovery-", dir=runner, ignore_cleanup_errors=True)
        root = Path(private.name).resolve(strict=True)
        try:
            exercise(args, root, report, qa)
        finally:
            deadline = time.monotonic() + 20
            while root.exists() and time.monotonic() < deadline:
                assert root.parent == runner and root.resolve() == root
                private.cleanup()
                if root.exists():
                    time.sleep(.2)
            assert not root.exists(), "Owned hosted fixture still in use"
        report["owned_fixture_cleaned"] = True
        report["status"] = "passed"
    finally:
        (args.out / "receipt.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
