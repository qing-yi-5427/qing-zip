"""Measure a real ZIP add and check every resulting member on ordinary temporary files."""
import argparse
import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import tempfile
import time
import zipfile
from pathlib import Path

def sha_file(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()

def member_name(i):
    return f"folder-{i % 150:03d}/file-{i:05d}.txt"

def member_data(i):
    return f"archive fixture {i:05d}\n".encode() * 4

def measure(cli, base, output, addition, count, original_names):
    shutil.copyfile(base, output)
    started = time.perf_counter_ns()
    result = subprocess.run([cli, "add", output, addition], cwd=addition.parent,
                            capture_output=True, timeout=240, creationflags=subprocess.CREATE_NO_WINDOW)
    elapsed_ms = (time.perf_counter_ns() - started) / 1e6
    if result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace")[-1400:])
    with zipfile.ZipFile(output) as z:
        names = z.namelist()
        if len(names) != count + 1 or set(names) != original_names | {addition.name}:
            raise ValueError("Unexpected member set")
        for i in range(count):
            if z.read(member_name(i)) != member_data(i):
                raise ValueError(f"Changed member {i}")
        if z.read(addition.name) != addition.read_bytes():
            raise ValueError("New member does not match source")
        method = z.getinfo(addition.name).compress_type
    return {"ms": round(elapsed_ms, 3), "bytes": output.stat().st_size,
            "new_method": method, "members_verified": count + 1, "zip_sha256": sha_file(output)}

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--baseline", type=Path, required=True)
    ap.add_argument("--candidate", type=Path)
    ap.add_argument("--out", type=Path, required=True)
    args = ap.parse_args()
    binaries = {"baseline": args.baseline.resolve(strict=True)}
    if args.candidate:
        binaries["candidate"] = args.candidate.resolve(strict=True)
    report = {"system": platform.platform(), "python": platform.python_version(), "cpu": platform.processor(),
              "cache": "OS cache retained; untimed fixture copy immediately before each CLI call",
              "timing": "child startup through successful CLI exit; output verification outside timer",
              "binary_sha256": {k: sha_file(v) for k, v in binaries.items()}, "cases": []}
    with args.out.open("x", encoding="utf-8") as receipt, tempfile.TemporaryDirectory(prefix="qz-large-add-") as d:
        root = Path(d)
        addition = root / "added-note.txt"
        addition.write_bytes(b"meeting note\n" * 256)
        os.utime(addition, (1700000000, 1700000000))
        for count in (10000, 50000):
            base = root / f"original-{count}.zip"
            output = root / "working.zip"
            original_names = {member_name(i) for i in range(count)}
            with zipfile.ZipFile(base, "w") as z:
                for i in range(count):
                    info = zipfile.ZipInfo(member_name(i), (2024, 1, 1, 10, 0, 0))
                    info.compress_type = zipfile.ZIP_DEFLATED
                    z.writestr(info, member_data(i))
            source_hash = sha_file(base)
            case = {"original_entries": count, "original_bytes": base.stat().st_size, "original_sha256": source_hash,
                    "warmup": {}, "rounds": []}
            for name, cli in binaries.items():
                case["warmup"][name] = measure(cli, base, output, addition, count, original_names)
            for round_index in range(5):
                order = list(binaries)
                if round_index % 2:
                    order.reverse()
                row = {name: measure(binaries[name], base, output, addition, count, original_names) for name in order}
                case["rounds"].append(row)
            if sha_file(base) != source_hash:
                raise RuntimeError("Original fixture changed")
            case["median_ms"] = {name: statistics.median(r[name]["ms"] for r in case["rounds"]) for name in binaries}
            if args.candidate:
                deltas = [r["candidate"]["ms"] - r["baseline"]["ms"] for r in case["rounds"]]
                case["paired_delta_ms"] = {k: round(f(deltas), 3) for k, f in
                                            (("median", statistics.median), ("min", min), ("max", max))}
            report["cases"].append(case)
            print(count, case["median_ms"], flush=True)
        json.dump(report, receipt, indent=2)
        receipt.write("\n")

if __name__ == "__main__":
    main()
