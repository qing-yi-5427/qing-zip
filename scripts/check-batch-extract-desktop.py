"""Drive ordinary batch extraction on an isolated Windows desktop."""

import argparse
from contextlib import suppress
import hashlib
import json
from pathlib import Path
import platform
import re
import runpy
import tempfile
import time
import zipfile


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def snapshot(directory):
    return {p.relative_to(directory).as_posix(): digest(p)
            for p in sorted(directory.rglob("*")) if p.is_file()}


def fixtures(root, cancel):
    inputs, output = root / "in", root / "out"
    inputs.mkdir()
    output.mkdir()
    paths = [inputs / f"{name}.zip" for name in ('first', 'second', 'third')]
    expected = {}
    for path in paths:
        payload = f"ordinary {path.stem} archive\n".encode()
        expected[path.stem] = hashlib.sha256(payload).hexdigest()
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr(zipfile.ZipInfo("file.txt"), payload)
            if cancel and path.stem == "second":
                # Bound the real job to 64 MiB and many ordinary outputs so
                # the stop button can be used while extraction is still active.
                block = bytes(range(256)) * 256
                for index in range(1024):
                    archive.writestr(zipfile.ZipInfo(f"parts/{index:04}.bin"), block)
    if not cancel:
        conflict = output / "2-second" / "file.txt"
        conflict.parent.mkdir()
        conflict.write_bytes(b"existing file to keep\n")
    else:
        neighbor = output / "2-second" / "outside-archive.txt"
        neighbor.parent.mkdir()
        neighbor.write_bytes(b"existing neighbor of the cancelled extraction\n")
    (output / "sentinel.txt").write_bytes(b"unrelated destination file\n")
    return inputs, paths, output, expected


def fill_batch(page, paths, output, expect):
    page.locator('[data-action="app-menu"]').click()
    page.locator('[data-menu="tools"]').click()
    page.locator('#oa-menu [data-action="batch"]').click()
    expect(page.get_by_role("heading", name="批量处理")).to_be_visible()
    page.locator("#qa-batch-inputs").fill("\n".join(map(str, paths)))
    page.locator("#qa-batch-op").select_option("extract")
    page.locator("#qa-batch-dest").fill(str(output))
    page.locator("#qa-batch-conflict").select_option("error")
    expect(page.locator("#qa-batch-plan li")).to_have_count(len(paths))


def text_without_paths(text, root):
    return re.sub(re.escape(str(root)), "<fixture>", text, flags=re.IGNORECASE)


def take_screenshot(page, screenshot_dir, name, root, locator=None):
    target = locator or page
    masks = []
    if root is not None:
        masks.append(target.get_by_text(str(root), exact=False).filter(visible=True))
        if locator is None:
            masks.append(page.locator('#qa-batch-inputs:visible, #qa-batch-dest:visible'))
    target.screenshot(path=screenshot_dir / f"{name}.png", mask=masks, mask_color="#303642")


def job_rows(page, root, selector):
    rows = page.locator(selector).evaluate_all("""cards => cards.filter(card => card.getClientRects().length).map(card => ({
      name: card.querySelector('strong')?.textContent,
      state: card.querySelector('.qa-badge')?.dataset.state,
      text: card.innerText
    }))""")
    return [{**row, "text": text_without_paths(row["text"], root)} for row in rows]


def stop_active_second(page, result, expect):
    label = "批量 2/3 · 解压"
    modal = page.locator('#qa-overlay [data-job-card]:visible').filter(has_text=label)
    inline = page.locator('#oa-task-panel:visible').filter(has=page.locator('#oa-task-title').filter(has_text=label))
    expect(modal.or_(inline)).to_be_visible(timeout=15000)
    if inline.is_visible():
        toggle = inline.locator('#oa-task-toggle')
        if toggle.get_attribute('aria-expanded') == 'false':
            toggle.click()
        stop = inline.locator('#oa-task-cancel')
    else:
        stop = modal.locator('[data-action="canceljob"]')
    result["stop_control_text"] = stop.inner_text()
    # A trusted click through the real WebView, only on the active second job.
    stop.click(timeout=15000)


def baseline_surface(page, cancel, expect):
    label = f"批量 {2 if cancel else 3}/3 · 解压"
    state, status = ("cancelled", "已取消") if cancel else ("complete", "已完成")
    modal = page.locator('#qa-overlay [data-job-card]:visible').filter(has_text=label)
    inline = page.locator('#oa-task-panel:visible').filter(has=page.locator('#oa-task-title').filter(has_text=label))
    phase = inline.locator('#oa-task-compact-phase:visible, #oa-task-phase:visible').filter(has_text=status).first
    expect(modal.locator(f'.qa-badge[data-state="{state}"]').or_(phase)).to_be_visible(timeout=20000)
    return page.locator('#qa-overlay:visible') if modal.is_visible() else inline


def candidate_report(page, expect, summary):
    report = page.locator('#oa-batch-result .oa-batch-outcome')
    # When a modal presents the compact result, open its canonical details view.
    overlay = page.locator('#qa-overlay:visible .oa-batch-outcome').filter(has_text=summary)
    expect(report.filter(has_text=summary).or_(overlay).first).to_be_visible(timeout=20000)
    if overlay.is_visible():
        overlay.locator('[data-action="batch-result"]').click()
    expect(report).to_contain_text(summary)
    return report


def published_identity(path):
    stat = path.stat()
    return stat.st_ino, stat.st_mtime_ns, stat.st_size


def check_candidate(page, root, output, evidence, result, cancel, redact, original_targets, expected, expect):
    label = "stop" if cancel else "mixed"
    report = candidate_report(page, expect, "成功 1 · 已取消 1 · 未开始 1" if cancel else "成功 2 · 失败 1")
    expect(report).to_contain_text("批量解压结果")
    result["direct_text"] = text_without_paths(report.inner_text(), root)
    details = report.locator('details')
    details.locator('summary').click()
    expect(details).to_contain_text("2. second.zip · 已取消" if cancel else "2. second.zip · 失败")
    resume = report.locator('[data-action="batch-resume"]')
    if cancel:
        expect(details).to_contain_text("3. third.zip · 未开始")
        assert result["stop_control_text"] == "停止批量处理"
    else:
        expect(details).to_contain_text("目标位置已有同名项目")
        expect(resume).to_have_count(0)
    result["detail_text"] = text_without_paths(details.inner_text(), root)
    take_screenshot(page, evidence, f"{label}-result", redact, report)
    if not cancel:
        return

    first = output / "1-first" / "file.txt"
    first_identity = published_identity(first)
    result["before_resume"] = snapshot(output)
    assert result["before_resume"] == {**original_targets, "1-first/file.txt": expected["first"]}
    expect(resume).to_have_text("继续未开始的项目")
    resume.click()
    report = candidate_report(page, expect, "成功 2 · 已取消 1")
    expect(report).not_to_contain_text("未开始")
    expect(report.locator('[data-action="batch-resume"]')).to_have_count(0)
    result["after_resume_text"] = text_without_paths(report.inner_text(), root)
    result["first_file_identity_and_mtime_unchanged"] = published_identity(first) == first_identity
    assert result["first_file_identity_and_mtime_unchanged"]
    assert snapshot(output) == {**result["before_resume"], "3-third/file.txt": expected["third"]}
    take_screenshot(page, evidence, "stop-resumed", redact, report)


def run_case(page, process, root, evidence, result, cancel, candidate, show_paths, expect, error, win32event, win32process):
    inputs, paths, output, expected = fixtures(root, cancel)
    before = snapshot(inputs)
    original_targets = snapshot(output)
    label = "stop" if cancel else "mixed"
    redact = None if show_paths else root
    fill_batch(page, paths, output, expect)
    if not cancel:
        take_screenshot(page, evidence, "batch-plan", redact)
    page.locator('[data-action="batch-start"]').click()
    try:
        if cancel:
            stop_active_second(page, result, expect)
        if candidate:
            check_candidate(page, root, output, evidence, result, cancel, redact, original_targets, expected, expect)
        else:
            surface = baseline_surface(page, cancel, expect)
            result["direct_text"] = text_without_paths(surface.inner_text(), root)
            take_screenshot(page, evidence, f"{label}-result", redact, surface)
    except (AssertionError, error):
        result["diagnostic_page"] = text_without_paths(page.locator("body").inner_text(), root)
        result["diagnostic_cards"] = job_rows(page, root, '[data-job-card]')
        result["diagnostic_output"] = snapshot(output)
        take_screenshot(page, evidence, f"{label}-diagnostic", redact)
        raise
    close = page.locator('#qa-overlay:visible [data-action="close"]')
    if close.is_visible():
        close.click()

    page.locator('[data-view="history"]').click()
    expect(page.get_by_role("heading", name="任务", exact=True)).to_be_visible()
    result["history_cards"] = job_rows(page, root, '#oa-history-list [data-job-card]')
    take_screenshot(page, evidence, f"{label}-history", redact)

    result.update(sources_before=before, sources_after=snapshot(inputs),
                  original_targets=original_targets, output_files=snapshot(output),
                  output_directories=sorted(p.name for p in output.iterdir() if p.is_dir()))
    assert result["sources_after"] == before
    assert result["output_files"]["1-first/file.txt"] == expected["first"]
    assert all(result["output_files"].get(name) == hash for name, hash in original_targets.items())
    if cancel and not candidate:
        assert {row["state"] for row in result["history_cards"]} >= {"complete", "cancelled"}
        assert "3-third" not in result["output_directories"]
        assert not any("third.zip" in row["text"] for row in result["history_cards"])
        fill_batch(page, [paths[2]], output, expect)
        result["remaining_archive_new_plan"] = text_without_paths(page.locator('#qa-batch-plan').inner_text(), root)
        assert result["remaining_archive_new_plan"].endswith('\\1-third')
        take_screenshot(page, evidence, "stop-manual-requeue", redact)
    else:
        assert result["output_files"] == {**original_targets, "1-first/file.txt": expected["first"],
                                          "3-third/file.txt": expected["third"]}
        history = result["history_cards"]
        assert len(history) == 3
        for row, name, state in zip(history, ("third", "second", "first"),
                                    ("complete", "cancelled" if cancel else "failed", "complete"), strict=True):
            assert row["state"] == state and f"{name}.zip" in row["text"], row
    with suppress(error):
        page.locator('[data-window="close"]').click()
    assert win32event.WaitForSingleObject(process, 10000) == win32event.WAIT_OBJECT_0
    assert win32process.GetExitCodeProcess(process) == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--case", choices=("mixed", "stop", "both"), default="both")
    parser.add_argument("--mode", choices=("baseline", "candidate"), required=True)
    parser.add_argument("--public-scratch-root", type=Path,
                        help="existing non-personal parent for temporary fixture paths shown in screenshots")
    args = parser.parse_args()
    exe = args.exe.resolve(strict=True)
    evidence = args.out.resolve()
    scratch = args.public_scratch_root.resolve(strict=True) if args.public_scratch_root is not None else evidence.parent
    evidence.mkdir(parents=True, exist_ok=False)
    helpers = runpy.run_path(str(Path(__file__).with_name("check-nested-multi-layer.py")))
    receipt = {"status": "running", "mode": args.mode, "platform": platform.platform(), "desktop": "private non-input Win32",
               "binaries_sha256": {path.name: digest(path) for path in
                                   (exe, exe.with_name("obz.exe"), exe.with_name("obz-worker.exe"))},
               "cases": {}}
    try:
        for name in ("mixed", "stop") if args.case == "both" else (args.case,):
            result = receipt["cases"][name] = {}
            temporary = tempfile.TemporaryDirectory(prefix="qing-zip-batch-", dir=scratch)
            root = Path(temporary.name).resolve()
            try:
                with helpers["private_process"](exe, root, None) as (page, process, _):
                    try:
                        run_case(page, process, root, evidence, result, name == "stop", args.mode == "candidate",
                                 args.public_scratch_root is not None, helpers["expect"],
                                 helpers["Error"], helpers["win32event"], helpers["win32process"])
                    finally:
                        if helpers["win32event"].WaitForSingleObject(process, 0) != helpers["win32event"].WAIT_OBJECT_0:
                            with suppress(helpers["Error"]):
                                page.locator('[data-window="close"]').click(timeout=2500)
            finally:
                deadline = time.monotonic() + 8
                while root.exists():
                    try:
                        temporary.cleanup()
                    except OSError as error:
                        if error.winerror not in (5, 32, 145) or time.monotonic() >= deadline:
                            raise
                        time.sleep(0.25)
            assert not root.exists()
            result["owned_temp_cleaned"] = True
        receipt["status"] = "passed"
    except Exception:
        receipt["status"] = "failed"
        raise
    finally:
        (evidence / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
