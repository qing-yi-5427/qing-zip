
"""Observe an ordinary batch after forcibly exiting only its isolated Windows GUI."""
import argparse
import json
import hashlib
from pathlib import Path
import runpy
from statistics import median
import tempfile
import time
import zipfile

scripts = Path(__file__).resolve().parent
resume = runpy.run_path(str(scripts / "check-batch-resume-desktop.py"))
batch, helpers, expect = resume["batch"], resume["helpers"], resume["expect"]
snapshot = batch["snapshot"]


def fixture(root):
    inputs, paths, output, expected = batch["fixtures"](root, True)
    block = bytes(range(256)) * 256
    with zipfile.ZipFile(paths[1], "a") as archive:
        for index in range(1024):
            archive.writestr(zipfile.ZipInfo(f"more/{index:04}.bin"), block)
    fourth = inputs / "fourth.zip"
    with zipfile.ZipFile(fourth, "w") as archive:
        archive.writestr(zipfile.ZipInfo("file.txt"), b"ordinary fourth content")
    paths.append(fourth)
    expected["fourth"] = helpers["digest"](b"ordinary fourth content")
    return inputs, paths, output, expected


def live_task(page, index, count):
    job = page.wait_for_function("""label => {
        const panel = document.querySelector('#oa-task-title');
        const id = Number(document.querySelector('#oa-task-output')?.dataset.jobOutput);
        return panel?.textContent === label && id;
    }""", arg=f"批量 {index}/{count} · 解压", timeout=12000).json_value()
    status = page.evaluate("(id) => window.__TAURI__.core.invoke('operation_status', {id})", job)
    assert status["status"] == "running", status
    return {"id": job, "status": status["status"], "progress": status["progress"]}


def wait_exit(process):
    events = helpers["win32event"]
    assert events.WaitForSingleObject(process, 10000) == events.WAIT_OBJECT_0
    return helpers["win32process"].GetExitCodeProcess(process)


def fail_active_window(args, page, process, index, count):
    live_task(page, index, count)
    page.screenshot(path=args.out / "running-before-exit.png")
    observed = live_task(page, index, count)
    helpers["win32process"].TerminateProcess(process, 137)
    assert wait_exit(process) == 137
    return observed


def after_restart(args, root, output):
    before = snapshot(output)
    with batch["launch_local"](args.exe, root, helpers) as (page, wait):
        page.locator('[data-view="history"]').click()
        expect(page.get_by_text("还没有任务记录。")).to_be_visible()
        assert page.locator("#oa-history-batch").count() == 0
        assert snapshot(output) == before
        page.screenshot(path=args.out / "reopened.png")
        resume["close"](page, wait)
    assert snapshot(output) == before
    return before


def summarize(outputs, neighbors):
    return {
        "preexisting_files": neighbors,
        "outputs_by_directory": {
            directory: {
                "file_count": len(files),
                "file_manifest_sha256": hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
            }
            for directory in sorted({name.partition("/")[0] for name in outputs})
            if (files := {name: sha for name, sha in outputs.items() if name.startswith(directory + "/")})
        },
    }


def exercise(args, root):
    inputs, paths, output, expected = fixture(root)
    occupied = output / "4-fourth/file.txt"
    occupied.parent.mkdir()
    occupied.write_bytes(b"existing fourth destination")
    originals = snapshot(inputs)
    neighbors = snapshot(output)
    result = {"source": args.source, **helpers["verify_package"](args)}
    first = output / "1-first/file.txt"
    with helpers["private_process"](args.exe, root, None) as (page, process, _pid):
        assert Path(page.evaluate("window.__TAURI__.core.invoke('current_executable')")).resolve() == args.exe
        batch["fill_batch"](page, paths, output, expect)
        page.locator("#qa-batch-conflict").select_option("overwrite")
        page.locator('[data-action="batch-start"]').click()
        observation = live_task(page, 2, 4)
        if not first.exists():
            print(json.dumps({"observation": observation, "output_files": snapshot(output),
                              "task_surface": batch["text_without_paths"](page.locator("#qing-archive").inner_text(), root)},
                             ensure_ascii=False))
        assert batch["digest"](first) == expected["first"]
        identity = batch["published_identity"](first)
        result["native_running_at_exit"] = fail_active_window(args, page, process, 2, 4)


    before_restart = after_restart(args, root, output)
    assert before_restart["1-first/file.txt"] == expected["first"]
    assert all(before_restart[name] == digest for name, digest in neighbors.items())
    assert not any(name.startswith("3-third/") for name in before_restart)
    assert batch["published_identity"](first) == identity
    assert snapshot(inputs) == originals
    return {**result, "no_resume_after_forced_exit": True, "sources": originals,
            **summarize(before_restart, neighbors), "first_published_file_identity_preserved": True}


def resumed_then_exited(args, root):
    inputs, paths, output, expected = resume["prepare"](root)
    original_inputs, neighbors = snapshot(inputs), snapshot(output)
    result = {"source": args.source, **helpers["verify_package"](args)}
    with batch["launch_local"](args.exe, root, helpers) as (page, wait):
        batch["fill_batch"](page, paths, output, expect)
        page.locator("#qa-batch-conflict").select_option("rename")
        page.locator('[data-action="batch-start"]').click()
        resume["live_control"](page, 2).click()
        batch["candidate_report"](page, expect, "成功 1 · 已取消 1 · 未开始 3")
        resume["close"](page, wait)
    record = resume["sealed"](root, ["complete", "cancelled", "pending", "pending", "pending"], paths, output)
    result["record_present_after_normal_close"] = True
    first = output / "1-first/file.txt"
    identity = batch["published_identity"](first)
    with helpers["private_process"](args.exe, root, None) as (page, process, _pid):
        report, _ = resume["show_saved"](page, root, output, "成功 1 · 已取消 1 · 未开始 3")
        report.get_by_role("button", name="继续未开始的项目").click()
        live_task(page, 3, 5)
        # The approved normal-close checkpoint is consumed before dispatch.
        assert not record.exists()
        result["record_consumed_before_forced_exit"] = True
        result["native_running_at_exit"] = fail_active_window(args, page, process, 3, 5)
    files = after_restart(args, root, output)
    assert files["1-first/file.txt"] == expected["first"]
    assert all(files[name] == digest for name, digest in neighbors.items())
    assert not any(name.startswith("5-fifth/") or name.startswith("4-fourth/") and name not in neighbors
                   for name in files)
    assert snapshot(inputs) == original_inputs and batch["published_identity"](first) == identity
    return {**result, "no_resume_after_forced_exit": True, "sources": original_inputs,
            **summarize(files, neighbors), "first_published_file_identity_preserved": True}


def metrics(values):
    return {"median": round(median(values), 2), "min": min(values), "max": max(values)}


def timing(args, root):
    inputs, paths, output, expected = batch["fixtures"](root, False)
    original_inputs, neighbors = snapshot(inputs), snapshot(output)
    result = {"source": args.source, **helpers["verify_package"](args)}
    with batch["launch_local"](args.exe, root, helpers) as (page, wait):
        sample_plan = {
            "destination": str(output), "overwrite": "error",
            "entries": [{"path": str(path), "dest": str(output / f"{index}-{path.stem}"),
                         "status": "pending", "hasOutput": False} for index, path in enumerate(paths, 1)],
        }
        saves = page.evaluate("""async plan => {
            const invoke = window.__TAURI__.core.invoke;
            let previous = null;
            const samples = [];
            for (let index = 0; index < 8; index++) {
                const from = performance.now();
                const saved = await invoke('save_batch_resume', {plan, previous});
                samples.push(performance.now() - from);
                previous = saved.revision;
            }
            if (!(await invoke('take_batch_resume', {revision:previous}))) throw Error('test CAS was not consumed');
            return samples;
        }""", sample_plan)
        batch["fill_batch"](page, paths, output, expect)
        page.evaluate("""() => {
            const source = command => window.__TAURI_INTERNALS__.convertFileSrc(command, "ipc");
            const commands = new Map(["start_operation","set_batch_resume_running"].map(name => [source(name), name]));
            const actual = window.fetch.bind(window);
            window.measuredNativeCalls = {};
            window.fetch = async (url, init) => {
                const name = commands.get(url);
                const from = performance.now();
                const response = await actual(url, init);
                if(name)(window.measuredNativeCalls[name]??=[]).push(performance.now()-from);
                return response;
            };
        }""")
        page.locator('[data-action="batch-start"]').click()
        batch["candidate_report"](page, expect, "成功 2 · 失败 1")
        calls = page.evaluate("window.measuredNativeCalls")
        assert len(calls["start_operation"]) == 3 and len(calls["set_batch_resume_running"]) == 2
        resume["close"](page, wait)
    assert snapshot(inputs) == original_inputs
    assert snapshot(output) == {**neighbors,
                                "1-first/file.txt": expected["first"], "3-third/file.txt": expected["third"]}
    samples = [round(value,2) for value in saves[1:]]
    return {**result, "cas_warmup_ms":round(saves[0],2), "cas_save_roundtrip_ms":samples,
            "cas_save_summary_ms":metrics(samples),
            "native_start_request_ms":[round(value,2) for value in calls["start_operation"]],
            "native_guard_request_ms":[round(value,2) for value in calls["set_batch_resume_running"]],
            "boundary":"Existing save includes durable file sync and replace, not a proposed new checkpoint implementation."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("exe", "out", "fixtures-in"):
        parser.add_argument(f"--{field}", type=Path, required=True)
    parser.add_argument("--package", type=Path)
    parser.add_argument("--hardening", type=Path)
    parser.add_argument("--staged-sha256", nargs=3)
    parser.add_argument("--source", required=True)
    parser.add_argument("--scenario", choices=("fresh", "resumed", "timing"), default="fresh")
    args = parser.parse_args()
    args.exe = args.exe.resolve(strict=True)
    if bool(args.staged_sha256) == bool(args.package and args.hardening):
        parser.error("provide a ZIP and hardening receipt, or the three staged binary hashes")
    args.out.mkdir(parents=True, exist_ok=False)
    owner = tempfile.TemporaryDirectory(prefix="batch-crash-", dir=args.fixtures_in)
    try:
        test = {"fresh": exercise, "resumed": resumed_then_exited, "timing": timing}[args.scenario]
        result = {**test(args, Path(owner.name)), "scenario": args.scenario}
    finally:
        deadline = time.monotonic() + 12
        while True:
            try:
                owner.cleanup()
                break
            except OSError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.25)
    (args.out / "receipt.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
