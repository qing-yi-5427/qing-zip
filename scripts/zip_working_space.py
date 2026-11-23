"""Sample ordinary ZIP operations on the capacity runner's private NTFS volume."""
import hashlib
import os
import platform
import shutil
import statistics
import tempfile
import threading
import time
import zipfile
from pathlib import Path


def sample_operation(volume, operation):
    """Return the observed high-water mark, including the published result."""
    readings = []
    ready = threading.Event()
    stop = threading.Event()
    errors = []

    def sample():
        readings.append((time.perf_counter_ns(), shutil.disk_usage(volume).free))

    def observe():
        try:
            sample()
            ready.set()
            while not stop.wait(0.005):
                sample()
        except OSError as error:
            errors.append(error)
        finally:
            ready.set()

    watcher = threading.Thread(target=observe)
    watcher.start()
    try:
        ready.wait()
        if errors:
            raise errors[0]
        result = operation()
    finally:
        stop.set()
        watcher.join()
    if errors:
        raise errors[0]
    sample()
    if len(readings) < 3:
        raise RuntimeError("No intermediate sample during the operation")
    first, last = readings[0][1], readings[-1][1]
    minimum = min(free for _, free in readings)
    intervals = [(b[0] - a[0]) / 1e6 for a, b in zip(readings, readings[1:])]
    return result, {
        "samples": len(readings),
        "largest_sample_gap_ms": round(max(intervals), 3),
        "free_bytes_before": first,
        "free_bytes_minimum": minimum,
        "free_bytes_after": last,
        "peak_growth_bytes": first - minimum,
        "net_growth_bytes": first - last,
        "peak_above_final_bytes": last - minimum,
    }


def measure(cli, fixtures, volume, owned, env, invoke, sha):
    source, archive = fixtures / "payload.bin", fixtures / "source.zip"
    source_hash, archive_hash = sha(source), sha(archive)
    result = {
        "windows": platform.version(), "processor": os.environ.get("PROCESSOR_IDENTIFIER"),
        "logical_cpus": os.cpu_count(), "python": platform.python_version(),
        "volume_bytes": shutil.disk_usage(volume).total, "payload_bytes": source.stat().st_size,
        "source_sha256": source_hash, "archive_sha256": archive_hash, "cases": {},
    }
    for action in ("create", "extract"):
        rounds = []
        for _ in range(6):
            with tempfile.TemporaryDirectory(dir=owned, prefix="space-") as temporary:
                directory = Path(temporary)
                dest = directory / "output"
                dest.mkdir()
                scratch = directory / "scratch"
                scratch.mkdir()
                child_env = dict(env, TEMP=str(scratch), TMP=str(scratch),
                                 LOCALAPPDATA=str(scratch), APPDATA=str(scratch))
                sentinel = dest / "keep.txt"
                sentinel.write_bytes(b"keep\n")
                if action == "create":
                    target = dest / "created.zip"
                    args = (target, source, "--format", "zip", "--store", "--threads", "1")
                else:
                    target = dest / source.name
                    target.write_bytes(b"\x53" * source.stat().st_size)
                    args = (archive, dest, "--overwrite", "overwrite", "--threads", "1")
                prior_target_hash = sha(target) if target.exists() else None
                sentinel_hash = sha(sentinel)
                run, sample = sample_operation(volume, lambda: invoke(cli, fixtures, child_env, action, args))
                if run["exit"] != 0:
                    raise RuntimeError(f"{action} failed: {run['stderr']}")
                if action == "create":
                    with zipfile.ZipFile(target) as output:
                        if output.namelist() != [source.name] or output.getinfo(source.name).compress_type != zipfile.ZIP_STORED:
                            raise RuntimeError("Unexpected ZIP members or method")
                        with output.open(source.name) as member:
                            observed = hashlib.file_digest(member, "sha256").hexdigest()
                else:
                    observed = sha(target)
                if observed != source_hash or {p.name for p in dest.iterdir()} != {target.name, sentinel.name}:
                    raise RuntimeError("Published files differ from the fixture")
                if sentinel.read_bytes() != b"keep\n" or sha(source) != source_hash or sha(archive) != archive_hash:
                    raise RuntimeError("A reference file changed")
                files_left = [p.stat().st_size for p in scratch.rglob("*") if p.is_file()]
                sample["scratch_files_after"] = len(files_left)
                sample["scratch_logical_bytes_after"] = sum(files_left)
                sample["previous_target_sha256"] = prior_target_hash
                sample["sentinel_sha256"] = sentinel_hash
                sample["published_sha256"] = sha(target)
                sample["published_bytes"] = target.stat().st_size
                sample["published_member_sha256"] = observed
                rounds.append(sample)
        result["cases"][action] = {
            "warmup": rounds[0], "rounds": rounds[1:],
            "median_peak_growth_bytes": statistics.median(s["peak_growth_bytes"] for s in rounds[1:]),
            "median_peak_above_final_bytes": statistics.median(s["peak_above_final_bytes"] for s in rounds[1:]),
        }
    return result
