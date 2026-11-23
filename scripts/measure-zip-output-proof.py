"""Pair complete ZIP updates and verify raw output-byte agreement on ordinary temporary files."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import re
import shutil
import statistics
import subprocess
import tempfile
import time
import zipfile


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def input_zip(path, mib, note_data):
    block = random.Random(312).randbytes(1024 * 1024)
    existing = hashlib.sha256()
    with zipfile.ZipFile(path, "x", zipfile.ZIP_STORED) as archive:
        info = zipfile.ZipInfo("existing.bin", (2024, 1, 1, 10, 0, 0))
        with archive.open(info, "w") as member:
            for _ in range(mib):
                member.write(block)
                existing.update(block)
        archive.writestr(zipfile.ZipInfo("before.txt", (2024, 1, 1, 10, 0, 0)), note_data)
    return {"existing.bin": existing.hexdigest(), "before.txt": hashlib.sha256(note_data).hexdigest()}


def measure(cli, action, output, source, note, expected, env, profile=False):
    shutil.copyfile(source, output)
    actions = {
        "add": [note],
        "rename": ["before.txt", "after.txt"],
        "delete": ["before.txt"],
    }
    env = env | ({"OPEN_ARCHIVE_PROFILE": "1"} if profile else {})
    start = time.perf_counter_ns()
    process = subprocess.run([cli, action, output, *actions[action]], cwd=output.parent, env=env,
                             stdin=subprocess.DEVNULL, capture_output=True, timeout=180,
                             creationflags=subprocess.CREATE_NO_WINDOW)
    elapsed_ms = (time.perf_counter_ns() - start) / 1e6
    if process.returncode:
        raise RuntimeError(f"{action} exited {process.returncode}: {process.stderr.decode(errors='replace')}")
    with zipfile.ZipFile(output) as archive:
        actual = {}
        for member in archive.namelist():
            with archive.open(member) as stream:
                actual[member] = hashlib.file_digest(stream, "sha256").hexdigest()
        if actual != expected:
            raise RuntimeError("Output member names or bytes differ")
    result = {"ms": round(elapsed_ms, 3), "zip_sha256": sha256(output)}
    if profile:
        match = re.search(rb"OA_ZIP_CANDIDATE bytes=(\d+) reread=(\d+)", process.stderr)
        if not match:
            raise RuntimeError(f"Missing owned ZIP proof diagnostic: {process.stderr!r}")
        candidate_bytes, reread = map(int, match.groups())
        result["candidate_bytes"] = candidate_bytes
        result["proof_reread_bytes"] = reread
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("previous", "candidate", "out"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--actions", nargs="+", choices=("add", "rename", "delete"),
                        default=("add", "rename", "delete"))
    parser.add_argument("--sizes", nargs="+", type=int, default=(128, 512))
    parser.add_argument("--diagnostics-only", action="store_true")
    args = parser.parse_args()
    if args.out.exists() or any(not 1 <= size <= 512 for size in args.sizes):
        parser.error("choose a new output file and fixture sizes between 1 and 512 MiB")
    engines = {key: getattr(args, key).resolve(strict=True) for key in ("previous", "candidate")}
    report = {"os": platform.platform(),
              "cli_sha256": {key: sha256(exe) for key, exe in engines.items()},
              "cache": "one warm-up pair, five alternating pairs, OS cache retained; independent verification outside timer",
              "cases": []}
    with tempfile.TemporaryDirectory(prefix="qing-zip-proof-perf-") as folder:
        root = Path(folder)
        env = os.environ.copy()
        env.pop("OPEN_ARCHIVE_PROFILE", None)
        for name in ("LOCALAPPDATA", "APPDATA", "TEMP", "TMP"):
            directory = root / name
            directory.mkdir()
            env[name] = str(directory)
        note = root / "note.txt"
        note_data = b"ordinary user file\n" * 128
        note.write_bytes(note_data)
        os.utime(note, (1700000000, 1700000000))
        note_hash = sha256(note)
        output = root / "working.zip"
        for mib in args.sizes:
            source = root / f"source-{mib}.zip"
            before = input_zip(source, mib, note_data)
            reference = sha256(source)
            for action in args.actions:
                expected = before.copy()
                if action == "add":
                    expected[note.name] = note_hash
                elif action == "rename":
                    expected["after.txt"] = expected.pop("before.txt")
                else:
                    expected.pop("before.txt")
                diagnostic = measure(engines["candidate"], action, output, source, note, expected, env, profile=True)
                case = {"mib": mib, "operation": action, "source_sha256": reference, "diagnostic": diagnostic}
                if not args.diagnostics_only:
                    for cli in engines.values():
                        measure(cli, action, output, source, note, expected, env)
                    rows = []
                    for n in range(5):
                        order = list(engines) if n % 2 == 0 else list(reversed(engines))
                        pair = {key: measure(engines[key], action, output, source, note, expected, env)
                                for key in order}
                        if pair["previous"]["zip_sha256"] != pair["candidate"]["zip_sha256"]:
                            raise RuntimeError("Complete output bytes differ between releases")
                        rows.append(pair)
                    deltas = [row["candidate"]["ms"] - row["previous"]["ms"] for row in rows]
                    case["pairs"] = rows
                    case["paired_delta_ms"] = {"min": min(deltas), "median": statistics.median(deltas), "max": max(deltas)}
                    case["medians_ms"] = {key: statistics.median(row[key]["ms"] for row in rows) for key in engines}
                if sha256(source) != reference or sha256(note) != note_hash:
                    raise RuntimeError("An update altered the source archive or input fixture")
                print(mib, action, diagnostic["proof_reread_bytes"], case.get("paired_delta_ms"), flush=True)
                report["cases"].append(case)
    with args.out.open("x", encoding="utf-8") as output:
        json.dump(report, output, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
