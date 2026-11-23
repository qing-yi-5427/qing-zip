"""Measure ordinary ZIP create/extract with the installed Windows CLI."""

import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import random
import runpy
import statistics
import tempfile
import zipfile

import archive_process_metrics as processes


interop = runpy.run_path(str(Path(__file__).with_name("check-archive-interop.py")))
file_digest = interop["file_digest"]
manifest = interop["manifest"]
digest = interop["digest"]
add_file = interop["add_file"]
populate = interop["populate"]

MIB = 1024 * 1024
PROFILES = {
    "small-files": "many-small-files",
    "mixed-files": "solid-mixed",
    "large-files-32mib": "few-large-random",
    "deep-tree": None,
}


def fixture(case, profile):
    source = case / "inputs" / "sample"
    source.mkdir(parents=True)
    existing = PROFILES[profile]
    if existing:
        populate(source, existing)
    else:
        rng = random.Random(1963)
        for group in range(12):
            for branch in range(8):
                base = f"项目 {group:02d}/第 {branch:02d} 组/资料/二级/三级"
                for index in range(8):
                    text = (f"测量文件 {group} {branch} {index}\n" * (48 + index)).encode()
                    block = rng.randbytes(16 * 1024) if index % 4 == 0 else b""
                    add_file(source, f"{base}/记录 {index}.dat", text + block)
                if branch % 2 == 0:
                    (source / base / "空目录").mkdir()
                if branch == 0:
                    add_file(source, f"{base}/空文件.txt", b"")
    return source


def zip_manifest(archive):
    entries, explicit = {}, set()
    with zipfile.ZipFile(archive) as package:
        for info in package.infolist():
            member = PurePosixPath(info.filename)
            name = member.as_posix()
            if member.is_absolute() or ".." in member.parts or name in explicit:
                raise RuntimeError(f"Unexpected ZIP name: {name}")
            explicit.add(name)
            if not info.is_dir():
                with package.open(info) as data:
                    entries[name] = digest(data)
            else:
                entries[name] = None
            for parent in member.parents:
                if parent != PurePosixPath("."):
                    entries.setdefault(parent.as_posix(), None)
    return entries


def operation(kernel, cli, args, work, env, *, interval, use_job):
    out, err, timing = processes.run(kernel, cli, args, work, env, interval_ms=interval, use_job=use_job)
    return json.loads(out), err, timing


def run_pair(kernel, cli, source, expected, work, *, interval, use_job, profile=False):
    archive = work / "output.zip"
    destination = work / "extracted"
    env = os.environ.copy()
    env.pop("OPEN_ARCHIVE_PROFILE", None)
    if profile:
        env["OPEN_ARCHIVE_PROFILE"] = "1"
    for name in ("LOCALAPPDATA", "APPDATA", "TEMP", "TMP"):
        path = work / name
        path.mkdir()
        env[name] = str(path)
    opts = {"interval": interval, "use_job": use_job}
    create = ["create", archive, source, "--format", "zip", "--method", "deflate", "--level", "5", "--threads", "4"]
    _, create_log, made = operation(kernel, cli, create, work, env, **opts)
    created = file_digest(archive)
    if zip_manifest(archive) != expected:
        raise RuntimeError("ZIP contains different files or directory structure")
    extract = ["extract", archive, destination, "--threads", "4"]
    result, extract_log, opened = operation(kernel, cli, extract, work, env, **opts)
    if result["files"] != sum(value is not None for value in expected.values()):
        raise RuntimeError("CLI extraction result has the wrong file count")
    if (manifest(destination) != expected or manifest(source.parent) != expected
            or file_digest(archive) != created):
        raise RuntimeError("Extraction output, input files, or archive contents changed")
    report = {"create": made, "extract": opened, "zip_bytes": archive.stat().st_size,
              "zip_sha256": created, "verified_source_archive_and_output": True}
    if profile:
        for action, log in (("create", create_log), ("extract", extract_log)):
            report[action]["profile_phases"] = sorted({line.decode().split()[1] for line in log.splitlines()
                                                       if line.startswith(b"OA_PHASE ")})
    return report


def stat(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def summary(rows, payload, count):
    totals = {}
    for action in ("create", "extract"):
        trials = [row[action] for row in rows]
        keys = ["elapsed_ms", "peak_sampled_working_set_bytes", "peak_sampled_private_working_set_bytes",
                "peak_sampled_private_commit_bytes", "system_cpu_percent"]
        if "job_peak_private_commit_bytes" in trials[0]:
            keys += ["job_peak_private_commit_bytes", "job_total_processes"]
        totals[action] = {field: stat([t[field] for t in trials]) for field in keys}
        totals[action]["payload_mib_per_second"] = stat([payload / MIB * 1000 / t["elapsed_ms"] for t in trials])
        totals[action]["files_per_second"] = stat([count * 1000 / t["elapsed_ms"] for t in trials])
        gaps = [b["elapsed_ms"] - a["elapsed_ms"] for trial in trials
                for a, b in zip(trial["samples"], trial["samples"][1:])]
        totals[action]["observed_sample_gaps_ms"] = stat(gaps)
    return totals


def job_control(kernel, cli, root, interval):
    case = root / "job-check"
    source = fixture(case, "mixed-files")
    expected = manifest(source.parent)
    runs = {}
    for label, assigned in (("normal", False), ("job", True)):
        work = case / label
        work.mkdir()
        try:
            runs[label] = run_pair(kernel, cli, source, expected, work, interval=interval, use_job=assigned, profile=True)
        except (OSError, RuntimeError) as error:
            if not assigned:
                raise
            return {"job_enabled": False, "reason": type(error).__name__, "winerror": getattr(error, "winerror", None)}
    match = runs["normal"]["zip_sha256"] == runs["job"]["zip_sha256"]
    controls = {}
    for action in ("create", "extract"):
        controls[action] = {label: {field: report[action][field]
                                    for field in ("sampled_image_names", "profile_phases")}
                            for label, report in runs.items()}
        # Short subprocesses can race a snapshot, but the normal ZIP case
        # must show the same observed route on both launches.
        match &= controls[action]["normal"] == controls[action]["job"]
    return {"job_enabled": match, "archive_sha256_equal": runs["normal"]["zip_sha256"] == runs["job"]["zip_sha256"],
            "controls": controls, "job_accounted_processes": {
                action: runs["job"][action]["job_total_processes"] for action in ("create", "extract")
            }}


def verify_install(cli, inventory):
    installed = {}
    for relative, expected in inventory.items():
        observed = file_digest(cli.parent / relative)
        if observed != expected:
            raise RuntimeError(f"Installed file does not match the release: {relative}")
        installed[relative] = observed
    return installed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True, help="Verified portable release inventory")
    parser.add_argument("--build-receipt", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cpu", required=True)
    parser.add_argument("--ram-gib", type=int, required=True)
    parser.add_argument("--drive", required=True, help="Verified model and filesystem for the temporary directory")
    parser.add_argument("--source", required=True, help="Exact source commit bound to the verified build")
    parser.add_argument("--rounds", type=int, choices=range(5, 11), default=5)
    parser.add_argument("--interval-ms", type=int, choices=range(5, 11), default=8)
    parser.add_argument("--control-only", action="store_true")
    args = parser.parse_args()
    if os.name != "nt" or args.out.exists():
        parser.error("a current Windows host and a new receipt path are required")
    cli = args.cli.resolve(strict=True)
    if cli.name.lower() != "obz.exe":
        parser.error("use the installed obz.exe")
    inventory = json.loads(args.manifest.read_text(encoding="utf-8-sig"))
    build = json.loads(args.build_receipt.read_text(encoding="utf-8-sig"))
    if (build["main_commit"] != args.source or build["inner_portable_sha256"] != inventory["portable_sha256"]
            or build["verified_payload_files"] != len(inventory["files"])):
        raise RuntimeError("Build and payload receipts disagree")
    installed = verify_install(cli, inventory["files"])
    kernel = processes.win32()
    report = {
        "software": {"os": platform.platform(), "python": platform.python_version(), "source": args.source,
                     "windows_regression_run": build["windows_regression_run"],
                     "release_portable_sha256": inventory["portable_sha256"],
                     "installed_release_file_count": len(installed), "cli_sha256": installed["obz.exe"],
                     "worker_sha256": installed["obz-worker.exe"], "gui_sha256": installed["qing-zip.exe"],
                     "runtime_manifest_sha256": hashlib.sha256(json.dumps(
                        {k: v for k, v in installed.items() if k.startswith("runtime/")}, sort_keys=True
                     ).encode()).hexdigest()},
        "hardware": {"cpu": args.cpu, "logical_processors": os.cpu_count(),
                     "installed_ram_gib": args.ram_gib, "temporary_drive": args.drive},
        "scope": "ordinary local files; CLI launch through exit; independent ZIP, source, and extracted checks outside timing",
        "cache": "one warm-up per action; OS warm, archive read for digest check before extraction; fresh process and output per trial",
        "sampling_target_ms": args.interval_ms,
        "cases": [],
    }
    with tempfile.TemporaryDirectory(prefix="qing-zip-installed-perf-") as folder:
        root = Path(folder)
        control = job_control(kernel, cli, root, args.interval_ms)
        report["job_control"] = control
        print("Memory accounting:", json.dumps(control), flush=True)
        report["timing_started"] = datetime.now().astimezone().isoformat(timespec="seconds")
        profiles = () if args.control_only else PROFILES
        for profile in profiles:
            case = root / profile
            source = fixture(case, profile)
            expected = manifest(source.parent)
            payloads = [source.parent / relative for relative, checksum in expected.items() if checksum is not None]
            payload = sum(path.stat().st_size for path in payloads)
            samples = []
            for round_index in range(args.rounds + 1):
                with tempfile.TemporaryDirectory(prefix=f"trial-{round_index}-", dir=case) as trial:
                    samples.append(run_pair(kernel, cli, source, expected, Path(trial),
                                            interval=args.interval_ms, use_job=control["job_enabled"]))
            rows = samples[1:]
            report["cases"].append({
                "profile": profile, "files": len(payloads), "directories": len(expected) - len(payloads),
                "source_bytes": payload, "source_manifest_sha256": hashlib.sha256(
                    json.dumps(expected, ensure_ascii=True, sort_keys=True).encode()).hexdigest(),
                "warmup": samples[0], "trials": rows, "summary": summary(rows, payload, len(payloads)),
            })
            print(profile, [(action, report["cases"][-1]["summary"][action]["elapsed_ms"]["median"])
                            for action in ("create", "extract")], flush=True)
        report["timing_finished"] = datetime.now().astimezone().isoformat(timespec="seconds")
    if verify_install(cli, inventory["files"]) != installed:
        raise RuntimeError("Installed program changed during measurement")
    report["installed_release_unchanged"] = True
    with args.out.open("x", encoding="utf-8") as output:
        json.dump(report, output, ensure_ascii=True, separators=(",", ":"))
        output.write("\n")


if __name__ == "__main__":
    main()
