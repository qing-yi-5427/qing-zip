"""Sample the real parent and contemporaneous process tree on the same absent destination."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile

import psutil

old, new, output = (Path(s).resolve() for s in sys.argv[1:])
output.parent.mkdir(exist_ok=True)
nana = old.parent / "runtime/nanazip/NanaZip.Universal.Console.exe"


def digest(path):
    with path.open("rb") as src:
        return hashlib.file_digest(src, "sha256").hexdigest()


def measure(cmd):
    metrics = {f"{scope}_{measure}": 0 for scope in ("root", "tree") for measure in ("handles", "private_bytes")}
    samples = 0
    started = time.perf_counter()
    with tempfile.TemporaryFile() as log:
        child = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
        proc = psutil.Process(child.pid)
        while child.poll() is None:
            try:
                processes = [proc, *proc.children(recursive=True)]
            except psutil.NoSuchProcess:
                continue
            counts = [0, 0]
            for process in processes:
                try:
                    with process.oneshot():
                        pair = (process.num_handles(), process.memory_info().private)
                except psutil.NoSuchProcess:
                    continue
                for i, name in enumerate(("handles", "private_bytes")):
                    counts[i] += pair[i]
                    if process == proc:
                        metrics["root_"+name] = max(metrics["root_"+name], pair[i])
            for i, name in enumerate(("handles", "private_bytes")):
                metrics["tree_"+name] = max(metrics["tree_"+name], counts[i])
            samples += 1
            time.sleep(0.02)
        elapsed = time.perf_counter() - started
        if child.returncode:
            log.seek(0)
            raise RuntimeError(log.read()[-3000:])
    return dict(seconds=elapsed, samples=samples, **metrics)


programs = {"0.11.8": old, "candidate": new, "NanaZip": nana}
provenance = json.loads(Path("runtime/nanazip/provenance.json").read_bytes())
assert all(digest(nana.parent / path) == expected for path, expected in provenance["files"].items())
report = {"os": platform.platform(), "psutil": psutil.__version__, "count": 12000,
          "note": "20ms polling after each process tree walk. CPU sampling, polling and checkout included in wall time; very short descendants may be missed.",
          "tools": {k:digest(p) for k,p in programs.items()}, "runs":[]}
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    arc = root/"small.zip"
    expected = {}
    with zipfile.ZipFile(arc, "w", zipfile.ZIP_DEFLATED, compresslevel=5) as z:
        for i in range(report["count"]):
            name = f"folder-{i%32:02d}/file-{i:05d}.bin"
            data = (hashlib.sha256(str(i).encode()).digest()*128)
            z.writestr(name, data)
            expected[name] = hashlib.sha256(data).hexdigest()
    report["archive_sha"] = digest(arc)
    for index in range(3):
        keys = list(programs)
        for name in keys[index:]+keys[:index]:
            dst=root/"fresh"
            assert not dst.exists()
            arc.read_bytes()
            cmd = [programs[name], "extract", arc, dst] if name != "NanaZip" else [nana, "x","-y", "-o"+str(dst),"--",arc]
            row = measure(cmd)
            actual = {p.relative_to(dst).as_posix():digest(p) for p in dst.rglob("*") if p.is_file()}
            assert actual == expected
            report["runs"].append(dict(repetition=index, tool=name, verified=len(actual), **row))
            print(name, row, flush=True)
            output.write_text(json.dumps(report,indent=2))
            shutil.rmtree(dst)
    report["complete"] = True
    output.write_text(json.dumps(report,indent=2))
