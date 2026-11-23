"""Pair two ordinary core ZIP updates in one native process, with output checks."""
from argparse import ArgumentParser
from hashlib import sha256
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
from tempfile import TemporaryDirectory
from time import monotonic, sleep
from zipfile import ZipFile

import psutil


def digest(data):
    return sha256(data).hexdigest()


def engine_for(executable):
    return (executable.parent / "runtime/nanazip/NanaZip.Universal.Console.exe").resolve(strict=True)


def runtime_files(executable):
    folder = engine_for(executable).parent
    names = ("NanaZip.Universal.Console.exe", "NanaZip.Core.dll", "NanaZip.Codecs.dll",
             "NanaZip.ZS.Codecs.dll", "K7Base.dll", "K7User.dll")
    return {name: digest((folder / name).read_bytes()) for name in names}


def check_build_idle():
    compiling = [p.info["name"] for p in psutil.process_iter(["name"])
                 if (p.info["name"] or "").lower() in {"cargo.exe", "rustc.exe"}]
    if compiling:
        raise RuntimeError("Concurrent compilation would contaminate timing")


def observe_overlap(process, engine, sources):
    expected = {str(path).casefold(): path.name for path in sources}
    parent = psutil.Process(process.pid)
    deadline = monotonic() + 15
    while process.poll() is None and monotonic() < deadline:
        live = {}
        try:
            children = parent.children()
        except psutil.NoSuchProcess:
            break
        for child in children:
            try:
                if Path(child.exe()).resolve() != engine or child.ppid() != process.pid:
                    continue
                args = child.cmdline()
                matches = [expected[arg.casefold()] for arg in args if arg.casefold() in expected]
                if args[1:2] != ["a"] or len(matches) != 1 or not child.is_running():
                    continue
                counters = child.io_counters()
                live[matches[0]] = {"pid": child.pid, "write_bytes": counters.write_bytes,
                                   "read_bytes": counters.read_bytes}
            except (psutil.Error, OSError):
                continue
        if len(live) == 2 and len({info["pid"] for info in live.values()}) == 2:
            return {"parent_pid": process.pid, "children": live, "file_arguments_match": True}
        sleep(.004)
    return None


def measure(executable, root, sources, inputs_sha, witness=False):
    root.mkdir()
    env = {**os.environ, "TEMP": str(root), "TMP": str(root),
           "LOCALAPPDATA": str(root / "local"), "APPDATA": str(root / "roaming")}
    pairs = []
    for index, source in enumerate(sources):
        archive = root / f"ordinary archive {index}.zip"
        kept = f"original fixture {index}\n".encode()
        with ZipFile(archive, "x") as opened:
            opened.writestr("keep.txt", kept)
        pairs.append((archive, source, kept))
    command = [executable, *(arg for pair in pairs for arg in pair[:2])]
    with subprocess.Popen(command, cwd=root, env=env, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as proc:
        overlap = (
            observe_overlap(proc, engine_for(executable), sources)
            if witness else None
        )
        stdout, stderr = proc.communicate(timeout=30)
        if proc.returncode:
            raise RuntimeError(stderr.decode(errors="replace"))
    output = json.loads(stdout)
    members = []
    for archive, source, kept in pairs:
        expected = {"keep.txt": digest(kept), source.name: inputs_sha[source]}
        with ZipFile(archive) as final:
            actual = {name: digest(final.read(name)) for name in final.namelist()}
            assert len(final.namelist()) == 2 and actual == expected
        assert digest(source.read_bytes()) == inputs_sha[source]
        members.append(actual)
    output["output_member_sha256"] = members
    if witness:
        output["same_parent_overlap"] = overlap
    return output


def summarize(samples):
    r2 = [row["r2"]["two_jobs_ms"] for row in samples]
    r3 = [row["r3"]["two_jobs_ms"] for row in samples]
    delta = [new - old for old, new in zip(r2, r3)]
    return {
        label: {"median_ms": statistics.median(values), "min_ms": min(values), "max_ms": max(values)}
        for label, values in (("r2", r2), ("r3", r3), ("paired_delta_r3_minus_r2", delta))
    }


p = ArgumentParser(description=__doc__)
p.add_argument("--r2", type=Path, required=True)
p.add_argument("--r3", type=Path, required=True)
p.add_argument("--out", type=Path, required=True)
a = p.parse_args()
bins = {name: getattr(a, name).resolve(strict=True) for name in ("r2", "r3")}
report = {"platform": platform.platform(), "cpu": platform.processor(),
          "python": platform.python_version(), "psutil": psutil.__version__,
          "benchmark_source_sha256": digest(Path(__file__).with_name("concurrent_zip.rs").read_bytes()),
          "binary_sha256": {name: digest(path.read_bytes()) for name, path in bins.items()},
          "engine_runtime_sha256": {name: runtime_files(path) for name, path in bins.items()},
          "cases": {}}
assert report["engine_runtime_sha256"]["r2"] == report["engine_runtime_sha256"]["r3"]
assert len(set(report["binary_sha256"].values())) == 2
check_build_idle()
with TemporaryDirectory(prefix="qing-concurrent-ordinary-zip-") as temp:
    root = Path(temp)
    for size, rounds in ((1024, 10), (8 * 1024 * 1024, 8)):
        case_root = root / str(size)
        case_root.mkdir()
        source_sha = {}
        for index in range(2):
            source = case_root / f"ordinary input {index}.bin"
            source.write_bytes(random.Random(size + index).randbytes(size))
            source_sha[source] = digest(source.read_bytes())
        sources = list(source_sha)
        warm = {name: measure(binary, case_root / ("warm-" + name), sources, source_sha, True)
                for name, binary in bins.items()}
        if size >= 8 * 1024 * 1024:
            assert all(warm[name]["same_parent_overlap"] is not None for name in bins)
        samples = []
        for i in range(rounds):
            order = list(bins) if i % 2 == 0 else list(reversed(bins))
            batch = {"pair": i + 1, "order": order}
            for name in order:
                batch[name] = measure(bins[name], case_root / f"{i + 1}-{name}", sources, source_sha)
            samples.append(batch)
        report["cases"][str(size)] = {
            "input_bytes_per_archive": size, "warmup_overlap": warm, "pairs": samples,
            "timing": summarize(samples),
        }
check_build_idle()
report["no_cargo_or_rustc_at_measurement_boundaries"] = True
with a.out.open("x", encoding="utf-8", newline="\n") as receipt:
    json.dump(report, receipt, indent=2)
    receipt.write("\n")
print(json.dumps({name: case["timing"] for name, case in report["cases"].items()}))
