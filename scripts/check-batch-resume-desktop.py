"""Exercise an ordinary ZIP batch across normal native window exits on Windows."""

import argparse
import json
from pathlib import Path
import runpy
import tempfile
import time
import zipfile
import win32file

directory = Path(__file__).resolve().parent
batch = runpy.run_path(str(directory / "check-batch-extract-desktop.py"))
helpers = runpy.run_path(str(directory / "check-nested-multi-layer.py"))
expect = helpers["expect"]
Error = helpers["Error"]
snapshot = batch["snapshot"]


def launch(args, root):
    if args.hosted:
        args.hosted_sessions += 1
        receipt = args.out / f"hosted-session-{args.hosted_sessions}.json"
        return batch["launch_hosted"](args.exe, root, receipt, helpers)
    return batch["launch_local"](args.exe, root, helpers)


def close(page, wait, active=False):
    try:
        page.locator('[data-window="close"]').click()
    except Error:
        if active:
            raise
    if active:
        prompt = page.locator("#oa-exit-confirm")
        expect(prompt).to_be_visible()
        try:
            prompt.locator('[data-action="cancel-exit"]').click()
        except Error:
            pass  # Closing the owning window can disconnect WebView2.
    assert wait() == 0


def prepare(root):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    block = bytes(range(256)) * 256
    with zipfile.ZipFile(paths[2], "a") as archive:
        # Keep the third job live long enough for a native close confirmation.
        for index in range(2048):
            archive.writestr(zipfile.ZipInfo(f"parts/{index:04}.bin"), block)
    for name in ("fourth", "fifth"):
        path = inputs / (name + ".zip")
        payload = f"ordinary {name} archive\n".encode()
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr(zipfile.ZipInfo("file.txt"), payload)
        expected[name] = helpers["digest"](payload)
        paths.append(path)
    occupied = output / "4-fourth/file.txt"
    occupied.parent.mkdir()
    occupied.write_bytes(b"original fourth file to preserve\n")
    return inputs, paths, output, expected


def live_control(page, index):
    title = f"批量 {index}/5 · 解压"
    modal = page.locator("#qa-overlay [data-job-card]:visible").filter(has_text=title)
    inline = page.locator("#oa-task-panel:visible").filter(
        has=page.locator("#oa-task-title").filter(has_text=title))
    expect(modal.or_(inline)).to_be_visible(timeout=15000)
    if inline.is_visible():
        toggle = inline.locator("#oa-task-toggle")
        if toggle.get_attribute("aria-expanded") == "false":
            toggle.click()
        return inline.locator("#oa-task-cancel")
    return modal.locator('[data-action="canceljob"]')


def show_saved(page, root, output, summary, policy="自动重命名"):
    page.locator('[data-view="history"]').click()
    report = page.locator("#oa-history-batch .oa-batch-outcome").filter(has_text=summary)
    expect(report).to_contain_text("待继续的批量解压")
    report.get_by_role("button", name="查看批量结果").click()
    report = page.locator("#oa-batch-result .oa-batch-outcome").filter(has_text=summary)
    expect(report).to_contain_text("同名文件 · " + policy)
    expect(report).to_contain_text(str(output))
    return report, batch["text_without_paths"](report.inner_text(), root)


def sealed(root, expected_statuses, paths, output, policy="rename"):
    record = root / "local/qing-zip/BatchResume/pending.json"
    content = record.read_text(encoding="utf-8")
    assert "password" not in content and '"message"' not in content
    plan = json.loads(content)["plan"]
    assert plan["overwrite"] == policy and Path(plan["destination"]) == output
    assert [entry["status"] for entry in plan["entries"]] == expected_statuses
    assert [Path(entry["path"]) for entry in plan["entries"]] == paths
    assert [Path(entry["dest"]) for entry in plan["entries"]] == [
        output / f"{index}-{path.stem}" for index, path in enumerate(paths, 1)]
    return record


def review_resume(page, report):
    report.get_by_role("button", name="继续未开始的项目").click()
    dialog = page.get_by_role("dialog", name="确认继续批量解压")
    expect(dialog).to_be_visible()
    return dialog


def exercise(args, root):
    inputs, paths, output, expected = prepare(root)
    input_hashes, target_hashes = snapshot(inputs), snapshot(output)
    result = {"source": args.source, **helpers["verify_package"](args)}
    first = output / "1-first/file.txt"
    first_stage = {**target_hashes, "1-first/file.txt": expected["first"]}
    with launch(args, root) as (page, wait):
        batch["fill_batch"](page, paths, output, expect)
        page.locator("#qa-batch-conflict").select_option("rename")
        page.locator('[data-action="batch-start"]').click()
        stop = live_control(page, 2)
        expect(stop).to_have_text("停止批量处理")
        stop.click()
        summary = "成功 1 · 已取消 1 · 未开始 3"
        report = batch["candidate_report"](page, expect, summary)
        expect(report.get_by_role("button", name="继续未开始的项目")).to_be_visible()
        page.screenshot(path=args.out / "stopped.png")
        assert snapshot(output) == first_stage
        first_identity = batch["published_identity"](first)
        close(page, wait)

    if args.baseline:
        with launch(args, root) as (page, wait):
            page.locator('[data-view="history"]').click()
            expect(page.locator("#oa-history-batch")).to_have_count(0)
            expect(page.get_by_text("还没有任务记录。")).to_be_visible()
            page.screenshot(path=args.out / "reopened-no-batch.png")
            assert snapshot(output) == first_stage and snapshot(inputs) == input_hashes
            close(page, wait)
        return {**result, "resume_missing_after_restart": True, "output": snapshot(output)}

    record = sealed(root, ["complete", "cancelled", "pending", "pending", "pending"], paths, output)
    with launch(args, root) as (page, wait):
        report, result["reopened"] = show_saved(page, root, output, summary)
        assert snapshot(output) == first_stage
        assert Path(page.evaluate("window.__TAURI__.core.invoke('current_executable')")).resolve() == args.exe
        page.screenshot(path=args.out / "reopened.png")
        # Another process owns the Windows lock, while the GUI can still see the record.
        lock = win32file.CreateFile(str(record.with_name("record.lock")), 0x80000000, 0, None, 3, 0, None)
        try:
            report.get_by_role("button", name="继续未开始的项目").click()
            expect(page.get_by_text("暂时无法更新待继续项目。请稍后重试。", exact=True).first).to_be_visible()
            assert snapshot(output) == first_stage and record.exists()
            result["busy_record_claim_started_no_job"] = True
        finally:
            lock.Close()
        report.get_by_role("button", name="继续未开始的项目").click()
        expect(live_control(page, 3)).to_be_visible()
        close(page, wait, active=True)

    sealed(root, ["complete", "cancelled", "interrupted", "pending", "pending"], paths, output)
    with launch(args, root) as (page, wait):
        report, result["reopened_again"] = show_saved(page, root, output, "成功 1 · 已取消 1 · 已中断 1 · 未开始 2")
        assert snapshot(output) == first_stage
        details = report.locator("details[open]")
        for name in ("4. fourth.zip · 未开始", "5. fifth.zip · 未开始"):
            expect(details.get_by_text(name)).to_be_visible()
        expect(details).to_contain_text(str(output / "4-fourth"))
        expect(details).to_contain_text(str(output / "5-fifth"))
        page.screenshot(path=args.out / "reopened-again.png")
        report.get_by_role("button", name="继续未开始的项目").click()
        batch["candidate_report"](page, expect, "成功 3 · 已取消 1 · 已中断 1")
        page.locator('[data-view="history"]').click()
        rows = batch["job_rows"](page, root, "#oa-history-list [data-job-card]")
        assert [row["name"] for row in rows] == ["批量 5/5 · 解压", "批量 4/5 · 解压"], rows
        result["resumed_in_order"] = [row["name"] for row in reversed(rows)]
        page.screenshot(path=args.out / "completed.png")
        close(page, wait)

    with launch(args, root) as (page, wait):
        page.locator('[data-view="history"]').click()
        expect(page.get_by_text("还没有任务记录。")).to_be_visible()
        expect(page.locator("#oa-history-batch")).to_have_count(0)
        close(page, wait)
    output_files = snapshot(output)
    assert output_files == {
        **first_stage, "4-fourth/file (1).txt": expected["fourth"], "5-fifth/file.txt": expected["fifth"],
    }
    assert batch["published_identity"](first) == first_identity
    assert snapshot(inputs) == input_hashes and not record.exists()
    return {**result, "output": output_files, "sources_before_and_after": input_hashes,
            "first_identity_and_mtime_preserved": True, "consumed_record_absent_after_final_reopen": True}


def write_failure(args, root):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    before, existing = snapshot(inputs), snapshot(output)
    blocker = root / "local/qing-zip/BatchResume"
    blocker.parent.mkdir(parents=True)
    blocker.write_bytes(b"ordinary file prevents writing in the recovery directory")
    saved_hash = batch["digest"](blocker)
    result = {"source": args.source, **helpers["verify_package"](args)}
    with launch(args, root) as (page, wait):
        batch["fill_batch"](page, paths, output, expect)
        page.locator('[data-action="batch-start"]').click()
        batch["stop_active_second"](page, result, expect)
        report = batch["candidate_report"](page, expect, "成功 1 · 已取消 1 · 未开始 1")
        expect(report).to_contain_text("未能保存待继续项目。本次会话仍可继续。")
        page.screenshot(path=args.out / "storage-unavailable.png")
        report.get_by_role("button", name="继续未开始的项目").click()
        batch["candidate_report"](page, expect, "成功 2 · 已取消 1")
        close(page, wait)
    assert snapshot(inputs) == before and batch["digest"](blocker) == saved_hash
    assert snapshot(output) == {**existing, "1-first/file.txt": expected["first"], "3-third/file.txt": expected["third"]}
    return {**result, "write_failure_kept_current_session": True, "output": snapshot(output), "sources": before}


def overwrite_review(args, root):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    original_inputs, original_outputs = snapshot(inputs), snapshot(output)
    result = {"source": args.source, **helpers["verify_package"](args)}
    with launch(args, root) as (page, wait):
        batch["fill_batch"](page, paths, output, expect)
        page.locator("#qa-batch-conflict").select_option("overwrite")
        page.locator('[data-action="batch-start"]').click()
        batch["stop_active_second"](page, result, expect)
        batch["candidate_report"](page, expect, "成功 1 · 已取消 1 · 未开始 1")
        close(page, wait)
    first = output / "1-first/file.txt"
    identity = batch["published_identity"](first)
    record = sealed(root, ["complete", "cancelled", "pending"], paths, output, "overwrite")
    before_record = batch["digest"](record)
    payload = b"replacement archive contents while qing-zip was closed\n"
    replacement = inputs / "new-third.zip"
    with zipfile.ZipFile(replacement, "w") as archive:
        archive.writestr(zipfile.ZipInfo("file.txt"), payload)
    replacement.replace(paths[2])
    current_inputs = snapshot(inputs)
    assert current_inputs.keys() == original_inputs.keys()
    assert all(current_inputs[name] == expected for name, expected in original_inputs.items() if name != "third.zip")
    assert current_inputs["third.zip"] != original_inputs["third.zip"]
    conflict = output / "3-third/file.txt"
    assert not conflict.exists()
    conflict.parent.mkdir()
    conflict.write_bytes(b"new same-named output to preserve unless explicitly confirmed")
    occupied = batch["digest"](conflict)
    with launch(args, root) as (page, wait):
        report, _ = show_saved(page, root, output, "成功 1 · 已取消 1 · 未开始 1", "替换已有文件")
        dialog = review_resume(page, report)
        expect(dialog).to_contain_text("继续时会读取以下路径上的当前压缩包")
        expect(dialog).to_contain_text("替换已有文件")
        expect(dialog).to_contain_text(str(paths[2]))
        expect(dialog).to_contain_text(str(conflict.parent))
        page.screenshot(path=args.out / "confirm-overwrite.png")
        dialog.get_by_role("button", name="返回").click()
        assert batch["digest"](record) == before_record and batch["digest"](conflict) == occupied
        assert snapshot(inputs) == current_inputs
        dialog = review_resume(page, report)
        dialog.get_by_role("button", name="按替换策略继续").click()
        batch["candidate_report"](page, expect, "成功 2 · 已取消 1")
        close(page, wait)
    assert not record.exists()
    output_hashes = snapshot(output)
    assert batch["published_identity"](first) == identity and snapshot(inputs) == current_inputs
    assert output_hashes == {**original_outputs, "1-first/file.txt": expected["first"],
                             "3-third/file.txt": helpers["digest"](payload)}
    return {**result, "same_named_target_after_restart": occupied, "input_before_restart": original_inputs,
            "current_inputs_after_replacement_and_after_resume": current_inputs, "output": output_hashes,
            "declining_preserved_target_and_record": True, "first_identity_and_mtime_preserved": True}


def record_recovery(args, root):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    original_inputs, original_outputs = snapshot(inputs), snapshot(output)
    store = root / "local/qing-zip/BatchResume"
    store.mkdir(parents=True)
    pending = store / "pending.json"
    damaged = b'{"version":1,"revision":"cut short",'
    pending.write_bytes(damaged)
    result = {"source": args.source, **helpers["verify_package"](args)}
    with launch(args, root) as (page, wait):
        batch["fill_batch"](page, paths, output, expect)
        page.locator('[data-action="batch-start"]').click()
        batch["stop_active_second"](page, result, expect)
        report = batch["candidate_report"](page, expect, "成功 1 · 已取消 1 · 未开始 1")
        expect(report).to_contain_text("本次会话仍可继续")
        assert pending.read_bytes() == damaged
        page.get_by_role("button", name="备份损坏记录").click()
        notice = page.get_by_role("dialog", name="操作提示")
        expect(notice).to_contain_text("可以保存新的待继续项目")
        page.screenshot(path=args.out / "backup-confirmed.png")
        archived = list(store.glob("damaged-*.json"))
        assert len(archived) == 1 and archived[0].read_bytes() == damaged and not pending.exists()
        notice.locator('.qa-dialog-footer button').click()
        report.get_by_role("button", name="继续未开始的项目").click()
        batch["candidate_report"](page, expect, "成功 2 · 已取消 1")
        close(page, wait)
    future = b'{"version":2,"future":"keep these bytes"}'
    pending.write_bytes(future)
    with launch(args, root) as (page, wait):
        page.locator('[data-view="history"]').click()
        expect(page.get_by_text("此版本无法识别待继续记录。", exact=False)).to_be_visible()
        expect(page.get_by_role("button", name="备份损坏记录")).to_have_count(0)
        close(page, wait)
    assert pending.read_bytes() == future and archived[0].read_bytes() == damaged
    assert snapshot(inputs) == original_inputs
    output_hashes = snapshot(output)
    assert output_hashes == {**original_outputs, "1-first/file.txt": expected["first"],
                             "3-third/file.txt": expected["third"]}
    return {**result, "damaged_backup_sha256": batch["digest"](archived[0]),
            "future_record_sha256": batch["digest"](pending), "ordinary_batch_completed": True,
            "sources": original_inputs, "output": output_hashes}


def exit_while_saving(args, root):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    original_inputs, original_outputs = snapshot(inputs), snapshot(output)
    result = {"source": args.source, **helpers["verify_package"](args)}
    with launch(args, root) as (page, wait):
        batch["fill_batch"](page, paths, output, expect)
        # Delay only delivery of the actual seal operation. Native work,
        # close admission, the decision and the on-disk record remain real.
        page.evaluate("""() => {
            const fetchActual = window.fetch.bind(window);
            const sealIpc = window.__TAURI_INTERNALS__.convertFileSrc("save_batch_resume", "ipc");
            const paused = new Promise(resolve => { window.releaseSealForQa = resolve; });
            window.fetch = async (url, init) => {
                if (url === sealIpc) {
                    document.body.dataset.qaSealWaiting = "true";
                    await paused;
                }
                return fetchActual(url, init);
            };
        }""")
        page.locator('[data-action="batch-start"]').click()
        batch["stop_active_second"](page, result, expect)
        expect(page.locator('body[data-qa-seal-waiting="true"]')).to_have_count(1)
        page.locator('[data-window="close"]').click()
        prompt = page.locator("#oa-exit-confirm")
        expect(prompt).to_be_visible()
        page.screenshot(path=args.out / "normal-close-awaits-record.png")
        prompt.locator('[data-action="cancel-exit"]').click()
        expect(prompt.locator('[data-action="cancel-exit"]')).to_be_disabled()
        page.evaluate("window.releaseSealForQa()")
        assert wait() == 0
    record = sealed(root, ["complete", "cancelled", "pending"], paths, output, "error")
    with launch(args, root) as (page, wait):
        report, _ = show_saved(page, root, output, "成功 1 · 已取消 1 · 未开始 1", "停止并报告冲突")
        report.get_by_role("button", name="继续未开始的项目").click()
        batch["candidate_report"](page, expect, "成功 2 · 已取消 1")
        close(page, wait)
    assert not record.exists() and snapshot(inputs) == original_inputs
    output_hashes = snapshot(output)
    assert output_hashes == {**original_outputs, "1-first/file.txt": expected["first"],
                             "3-third/file.txt": expected["third"]}
    return {**result, "normal_close_waited_for_seal": True, "sources": original_inputs, "output": output_hashes}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("exe", "out", "fixtures-in"):
        parser.add_argument(f"--{field}", required=True, type=Path)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--hardening", type=Path)
    parser.add_argument("--staged-sha256", nargs=3, metavar=("GUI_SHA", "CLI_SHA", "WORKER_SHA"))
    parser.add_argument("--source", required=True)
    parser.add_argument("--hosted", action="store_true")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--baseline", action="store_true")
    modes.add_argument("--write-failure", action="store_true")
    modes.add_argument("--overwrite-review", action="store_true")
    modes.add_argument("--record-recovery", action="store_true")
    modes.add_argument("--exit-while-saving", action="store_true")
    args = parser.parse_args()
    args.hosted_sessions = 0
    args.exe = args.exe.resolve(strict=True)
    if bool(args.staged_sha256) == bool(args.package and args.hardening):
        parser.error("provide a ZIP and hardening receipt, or all three staged binary hashes")
    args.out.mkdir(parents=True, exist_ok=False)
    fixture = tempfile.TemporaryDirectory(prefix="batch-resume-", dir=args.fixtures_in)
    try:
        test = next((run for selected, run in (
            (args.write_failure, write_failure),
            (args.overwrite_review, overwrite_review),
            (args.record_recovery, record_recovery),
            (args.exit_while_saving, exit_while_saving),
        ) if selected), exercise)
        result = test(args, Path(fixture.name))
    finally:
        deadline = time.monotonic() + 15
        while True:
            try:
                fixture.cleanup()
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.25)
    (args.out / "receipt.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
