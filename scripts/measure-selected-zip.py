"""Attribute or pair timings for single-file extraction from an ordinary wide ZIP."""

import argparse
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import tempfile
import time


def sample(cli, work, archive, target, target_hash, archive_hash, run, profile):
    destination = work / run
    command = [cli, "extract", archive, destination, "--include", target, "--threads", "4"]
    environment = dict(os.environ)
    if profile:
        environment["OPEN_ARCHIVE_PROFILE"] = "1"
    else:
        environment.pop("OPEN_ARCHIVE_PROFILE", None)
    start = time.perf_counter()
    result = subprocess.run(command, env=environment, cwd=work, stdin=subprocess.DEVNULL,
                            capture_output=True, timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
    elapsed = (time.perf_counter() - start) * 1000
    stderr = result.stderr.decode("utf-8", errors="replace")
    if result.returncode or json.loads(result.stdout)["files"] != 1:
        raise RuntimeError(f"Extraction failed: {result.returncode} {stderr}")
    actual = {p.relative_to(destination).as_posix() for p in destination.rglob("*")}
    if actual != {"wide", target} or sha256((destination / target).read_bytes()) != target_hash:
        raise RuntimeError("Wrong selected output tree or bytes")
    if sha256(archive.read_bytes()) != archive_hash:
        raise RuntimeError("ZIP input bytes changed")
    row = {"wall_ms": round(elapsed, 3)}
    if profile:
        mode = 2 if archive.suffix == ".zip" else 1
        if f"OA_DIRECT_EXTRACT files=1 mode={mode}" not in stderr:
            raise RuntimeError(f"Expected direct extraction: {stderr}")
        row["phase_ms"] = {
            phase: round(float(seconds) * 1000, 3)
            for phase, seconds in re.findall(r"^OA_PHASE ([\w-]+) ([0-9.]+)$", stderr, re.MULTILINE)
        }
        if "extract-engine-total" not in row["phase_ms"]:
            raise RuntimeError("Expected instrumented CLI with extraction phase timings")
    return row


def solid_fixture(cli, work, archive):
    source = work / "wide"
    source.mkdir()
    for index in range(99):
        (source / f"member-{index:06}.txt").write_bytes(b"ordinary solid archive control\n")
    target_data = b"ordinary selected solid archive member\n"
    (source / "target-needle.txt").write_bytes(target_data)
    command = [cli, "create", archive, source, "--format", "7z", "--method", "lzma2",
               "--solid", "--level", "5", "--threads", "4"]
    build = subprocess.run(command, cwd=work, stdin=subprocess.DEVNULL, capture_output=True,
                           timeout=30, creationflags=subprocess.CREATE_NO_WINDOW, check=True)
    if not build.stdout:
        raise RuntimeError("Expected CLI response creating the solid archive control")
    return "wide/target-needle.txt", sha256(target_data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True, help="Staged baseline obz.exe")
    parser.add_argument("--candidate", type=Path, help="Staged candidate obz.exe for paired timing")
    parser.add_argument("--profile", action="store_true", help="Require extraction phase timings")
    parser.add_argument("--controls", action="store_true", help="Also time a small ZIP and a solid 7z")
    parser.add_argument("--rounds", type=int, choices=range(3, 13), default=7, metavar="3-12")
    parser.add_argument("--receipt", type=Path, required=True, help="New JSON file")
    args = parser.parse_args()
    binaries = {"baseline": args.baseline.resolve(strict=True)}
    if args.candidate:
        binaries["candidate"] = args.candidate.resolve(strict=True)
    report = {"profile": args.profile, "binaries": {
        key: {"cli_sha256": sha256(path.read_bytes()),
              "worker_sha256": sha256(path.with_name("obz-worker.exe").read_bytes())}
        for key, path in binaries.items()
    }, "cases": []}
    with tempfile.TemporaryDirectory(prefix="qing-selected-zip-timing-") as temporary:
        work = Path(temporary).resolve()
        cases = [(10000, "zip"), (50000, "zip")]
        if args.controls:
            cases = [(100, "zip"), *cases, (100, "7z")]
        for count, format_name in cases:
            archive = work / f"wide-{count}.{format_name}"
            if format_name == "zip":
                _, target, target_hash = fixture(archive, count)
            else:
                target, target_hash = solid_fixture(binaries["baseline"], work, archive)
            archive_hash = sha256(archive.read_bytes())
            rounds = []
            for index in range(args.rounds + 1):
                order = list(binaries)
                if index % 2 == 0:
                    order.reverse()
                rows = {label: sample(binaries[label], work, archive, target, target_hash, archive_hash,
                                     f"out-{format_name}-{count}-{index}-{label}", args.profile)
                        for label in order}
                rounds.append({"order": order, **rows})
            report["cases"].append({"entries": count, "format": format_name, "archive_sha256": archive_hash,
                                    "selected_sha256": target_hash, "warmup": rounds[0], "samples": rounds[1:]})
    with args.receipt.open("x", encoding="utf-8") as output:
        print(json.dumps(report, indent=2), file=output)


browse = runpy.run_path(str(Path(__file__).with_name("benchmark-archive-browse.py")))
fixture, sha256 = browse["fixture"], browse["sha256"]

if __name__ == "__main__":
    main()
