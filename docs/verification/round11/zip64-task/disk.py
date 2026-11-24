"""Sample logical temporary and final file lengths during an ordinary installed ZIP64 extraction."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def snapshot(root, final, inputs):
    sizes = {"final": 0, "temporary": 0}
    files = []
    inaccessible = 0
    for folder, _, names in os.walk(root):
        for name in names:
            p = Path(folder) / name
            if p in inputs:
                continue
            try:
                length = p.stat().st_size
            except FileNotFoundError:
                continue
            except PermissionError:
                inaccessible += 1
                continue
            category = "final" if p == final else "temporary"
            sizes[category] += length
            if length > 1024**2:
                files.append([str(p.relative_to(root)).replace("\\", "/"), length])
    return sizes, files, inaccessible


def trial(cli, root, archive, original, valid):
    with tempfile.TemporaryDirectory(prefix="trial-", dir=root) as folder:
        area = Path(folder)
        output = area / "out"
        output.mkdir()
        env = dict(os.environ)
        for key in ("LOCALAPPDATA", "APPDATA", "TMP", "TEMP"):
            custom = area / key
            custom.mkdir()
            env[key] = str(custom)
        final = output / valid["installed_list"]["name"]
        before = sha(archive)
        free_before = shutil.disk_usage(root).free
        min_free = free_before
        started = time.perf_counter()
        times = []
        peak = {k: {"bytes": 0, "files_over_mib": [], "elapsed_s": 0} for k in ("final", "temporary", "combined")}
        missed = 0
        with subprocess.Popen([cli, "extract", archive, output], cwd=area, env=env, stdin=subprocess.DEVNULL,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=subprocess.CREATE_NO_WINDOW) as process:
            while True:
                elapsed = time.perf_counter() - started
                if elapsed > 300:
                    process.kill()
                    raise TimeoutError("Installed extraction timed out")
                is_done = process.poll() is not None
                sizes, files, unavailable = snapshot(root, final, {archive, original})
                min_free = min(min_free, shutil.disk_usage(root).free)
                times.append(elapsed)
                missed += unavailable
                sizes["combined"] = sizes["temporary"] + sizes["final"]
                for k in sizes:
                    if sizes[k] > peak[k]["bytes"]:
                        peak[k] = {"bytes": sizes[k], "files_over_mib": files, "elapsed_s": round(elapsed, 4)}
                if is_done:
                    break
                time.sleep(0.025)
            stdout, stderr = process.communicate()
            if process.returncode or json.loads(stdout)["files"] != 1:
                raise RuntimeError(f"Extraction failed ({process.returncode}): {stderr[-500:]!r}")
        paths = list(output.rglob("*"))
        if paths != [final] or final.stat().st_size != valid["requested_member_bytes"]:
            raise RuntimeError("Unexpected extraction result")
        content = sha(final)
        if content != valid["extracted_member"]["sha256"] or before != sha(archive):
            raise RuntimeError("Archive or output checksum mismatch")
        return {"seconds_to_last_sample": round(times[-1], 4), "samples": len(times),
                "volume_free_before": free_before, "lowest_sampled_volume_free": min_free,
                "volume_free_drop": free_before - min_free,
                "max_sample_interval_s": round(max(y-x for x,y in zip(times, times[1:])), 4),
                "permission_errors": missed, "peak": peak, "output_sha256": content, "archive_unchanged": True}


def main():
    parser = argparse.ArgumentParser()
    for field in ("cli", "member-script", "member-receipt", "scratch", "out"):
        parser.add_argument("--"+field, type=Path, required=True)
    args = parser.parse_args()
    if args.out.exists():
        parser.error("Use a new result filename")
    parent = args.scratch.resolve(strict=True)
    if shutil.disk_usage(parent).free < 24 * 1024**3:
        raise RuntimeError("24 GiB of free temporary space required")
    cli = args.cli.resolve(strict=True)
    prior = json.loads(args.member_receipt.read_text(encoding="utf8"))
    valid = prior["cases"][-1]
    spec = importlib.util.spec_from_file_location("fixture", args.member_script)
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    actual_binaries = {name: sha(cli.with_name(name)) for name in ("obz.exe", "obz-worker.exe")}
    if cli.name.lower() != "obz.exe" or any(value != prior["installed_binaries_sha256"][name] for name, value in actual_binaries.items()):
        raise RuntimeError("CLI or worker differs from the reference run")
    results = {"status": "FAIL", "cli_sha256": actual_binaries["obz.exe"], "worker_sha256": actual_binaries["obz-worker.exe"],
               "source": prior["source"], "platform": platform.platform(),
               "requested_sample_delay_ms": 25, "trials": []}
    try:
        with tempfile.TemporaryDirectory(prefix="zip64-disk-", dir=parent) as folder:
            root = Path(folder)
            source = root / fixture.MEMBER
            archive = root / "ordinary.zip"
            meta, _ = fixture.fixture(source, valid["requested_member_bytes"])
            fixture.create_zip(source, archive)
            if meta != valid["synthetic_input"] or sha(archive) != valid["archive_before"]["sha256"]:
                raise RuntimeError("Generated archive differs from verified ZIP")
            for index in range(6):
                result = trial(cli, root, archive, source, valid)
                if index:
                    results["trials"].append(result)
                else:
                    results["warmup"] = result
            if sha(source) != meta["sha256"]:
                raise RuntimeError("Original source changed")
            results["source_unchanged"] = True
        results["fixture_removed"] = not root.exists()
        if not results["fixture_removed"]:
            raise RuntimeError("Temporary directory remained")
        for category in ("final", "temporary", "combined"):
            values = [r["peak"][category]["bytes"] for r in results["trials"]]
            results[category+"_sampled_bytes"] = {"median": statistics.median(values), "min": min(values), "max": max(values)}
        drops = [r["volume_free_drop"] for r in results["trials"]]
        results["volume_free_drop_bytes"] = {"median": statistics.median(drops), "min": min(drops), "max": max(drops)}
        results["status"] = "PASS"
    except Exception as exc:
        results["error"] = str(exc)
        raise
    finally:
        with args.out.open("x", encoding="utf8") as f:
            json.dump(results, f, indent=2, ensure_ascii=False)


if __name__ == "__main__":
    main()
