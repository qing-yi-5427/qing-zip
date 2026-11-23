"""Time a three-ZIP mixed-outcome batch in the real WebView2 window."""

import argparse
from contextlib import contextmanager, suppress
import json
import os
from pathlib import Path
import platform
import runpy
import statistics
import tempfile
import time

SCRIPTS = Path(__file__).resolve().parents[4] / "scripts"
batch = runpy.run_path(str(SCRIPTS / "check-batch-extract-desktop.py"))
desktop = runpy.run_path(str(SCRIPTS / "check-nested-multi-layer.py"))


@contextmanager
def fixture():
    owned = tempfile.TemporaryDirectory(prefix="qing-batch-timing-")
    try:
        yield Path(owned.name)
    finally:
        deadline = time.monotonic() + 8
        while True:
            try:
                owned.cleanup()
                break
            except OSError as error:
                if error.winerror not in (5, 32, 145) or time.monotonic() >= deadline:
                    raise
                time.sleep(0.25)


def measure(exe, candidate):
    with fixture() as root:
        inputs, paths, output, expected = batch["fixtures"](root, False)
        before = batch["snapshot"](inputs)
        existing = batch["snapshot"](output)
        with desktop["private_process"](exe, root, None) as (page, process, _):
            batch["fill_batch"](page, paths, output, desktop["expect"])
            # Observe presentation only. Both the click and extraction use product entry points.
            page.evaluate("""candidate => {
                const start = document.querySelector('[data-action="batch-start"]');
                start.addEventListener('click', function begin(event) {
                    if (!event.isTrusted) throw Error('Expected trusted pointer event');
                    const started = performance.now();
                    const observer = new MutationObserver(() => {
                        const title = document.querySelector('#oa-task-title');
                        const phase = document.querySelector('#oa-task-compact-phase');
                        const summary = document.querySelector('#oa-batch-result');
                        const finished = candidate
                            ? summary?.getClientRects().length && summary.textContent.includes('成功 2 · 失败 1')
                            : title?.getClientRects().length && title.textContent.includes('批量 3/3')
                              && phase?.getClientRects().length && phase.textContent.includes('已完成');
                        if (finished) {
                            window.batchMeasuredMs = performance.now() - started;
                            observer.disconnect();
                        }
                    });
                    observer.observe(document.querySelector('#qing-archive'),
                                     {subtree:true,childList:true,attributes:true,characterData:true});
                }, {capture:true,once:true});
            }""", candidate)
            page.locator('[data-action="batch-start"]').click()
            try:
                if candidate:
                    batch["candidate_report"](page, desktop["expect"], "成功 2 · 失败 1")
                else:
                    batch["baseline_surface"](page, False, desktop["expect"])
                elapsed = page.evaluate("window.batchMeasuredMs")
                assert isinstance(elapsed, (float, int)) and elapsed > 0, elapsed
            finally:
                with suppress(desktop["Error"]):
                    page.locator('[data-window="close"]').click()
            assert batch["snapshot"](inputs) == before
            assert batch["snapshot"](output) == {**existing, "1-first/file.txt": expected["first"],
                                                 "3-third/file.txt": expected["third"]}
            assert desktop["win32event"].WaitForSingleObject(process, 10000) == 0
            assert desktop["win32process"].GetExitCodeProcess(process) == 0
        return round(elapsed, 2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    paths = {key: getattr(args, key).resolve(strict=True) for key in ("baseline", "candidate")}
    data = {"os": platform.platform(), "cpu": os.environ.get("PROCESSOR_IDENTIFIER"),
            "cache": "No OS cache flush; one warm-up pair. A fresh process and fixture per run.",
            "boundary": "Trusted start click to visible terminal result; startup, output checks and screenshots excluded.",
            "binaries_sha256": {key: {name: batch["digest"](exe.with_name(name)) for name in
                                ("qing-zip.exe", "obz.exe", "obz-worker.exe")} for key, exe in paths.items()},
            "samples_ms": []}
    for i in range(6):
        order = list(paths) if i % 2 == 0 else list(reversed(paths))
        row = {key: measure(paths[key], key == "candidate") for key in order}
        if i == 0:
            data["warmup_ms"] = row
        else:
            data["samples_ms"].append(row)
    samples = data["samples_ms"]
    deltas = [row["candidate"] - row["baseline"] for row in samples]
    data["summary_ms"] = {key: {"median": statistics.median(values), "min": min(values), "max": max(values)}
                          for key, values in (("baseline", [r["baseline"] for r in samples]),
                                              ("candidate", [r["candidate"] for r in samples]),
                                              ("paired_difference", [round(d, 2) for d in deltas]))}
    with args.out.open("x", encoding="utf-8") as out:
        json.dump(data, out, ensure_ascii=False, indent=2)
        out.write("\n")


if __name__ == "__main__":
    main()
