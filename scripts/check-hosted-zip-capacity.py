"""Exercise ordinary ZIP against the independently verified hosted VHD."""
import ctypes
from ctypes import wintypes
import errno
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import subprocess
import sys
import time
import zipfile

MIB = 1024 * 1024
CAPACITY = 96 * MIB
PAYLOAD = 16 * MIB
SPACE_ERROR = re.compile(
    r"not enough space|insufficient (?:disk )?space|no space left|disk (?:is )?full|"
    r"空间不足|没有足够的磁盘空间|磁盘已满|os error (?:112|39)\b|0x800700(?:70|27)\b",
    re.IGNORECASE,
)
HOSTED = {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted",
          "RUNNER_OS": "Windows", "RUNNER_ARCH": "X64", "GITHUB_REPOSITORY": "qing-yi-5427/qing-zip"}


def check(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    with path.open("rb") as data:
        return hashlib.file_digest(data, "sha256").hexdigest()


def snapshot(directory):
    return {p.relative_to(directory).as_posix(): sha(p) if p.is_file() else "directory"
            for p in sorted(directory.rglob("*"))}


def volume_identity(root, opened=None):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    label, filesystem = ctypes.create_unicode_buffer(261), ctypes.create_unicode_buffer(261)
    serial = wintypes.DWORD()
    if opened is None:
        query, primary, first_type = kernel.GetVolumeInformationW, str(root), wintypes.LPCWSTR
    else:
        import msvcrt
        query = kernel.GetVolumeInformationByHandleW
        primary, first_type = msvcrt.get_osfhandle(opened.fileno()), wintypes.HANDLE
    query.argtypes = [first_type, wintypes.LPWSTR, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
                      ctypes.c_void_p, ctypes.c_void_p, wintypes.LPWSTR, wintypes.DWORD]
    query.restype = wintypes.BOOL
    if not query(primary, label, len(label), ctypes.byref(serial), None, None, filesystem, len(filesystem)):
        raise ctypes.WinError(ctypes.get_last_error())
    return label.value, serial.value, filesystem.value


def verify_volume(root, marker, identity):
    check(volume_identity(root) == identity, "Test volume changed")
    usage = shutil.disk_usage(root)
    check(80 * MIB <= usage.total <= CAPACITY, "VHD capacity outside strict bounds")
    check((root / "qz-volume-owner").read_text(encoding="ascii") == marker, "VHD owner marker changed")
    return usage


def fill(filler, root, marker, identity, result):
    verify_volume(root, marker, identity)
    written, errors = 0, []
    with filler.open("x+b", buffering=0) as handle:
        check(volume_identity(root, handle) == identity, "Filler handle left verified volume")
        for size in (MIB, 64 * 1024, 4096):
            block = b"\x7d" * size
            while written + size <= CAPACITY:
                try:
                    count = handle.write(block)
                except OSError as error:
                    code = getattr(error, "winerror", None)
                    if error.errno != errno.ENOSPC and code not in (39, 112):
                        raise
                    errors.append({"write_bytes": size, "errno": error.errno, "winerror": code})
                    break
                check(count and 0 < count <= size, "Fill write made no progress")
                written += count
        os.fsync(handle.fileno())
    usage = verify_volume(root, marker, identity)
    result.update(volume_bytes=usage.total, free_at_failure=usage.free, filler_bytes=filler.stat().st_size,
                  os_full_errors=errors)
    check(errors and usage.free <= 4096, "Could not demonstrate native disk-full condition")


def invoke(cli, work, env, operation, args):
    completed = subprocess.run([cli, operation, *args], cwd=work, env=env, stdin=subprocess.DEVNULL,
                               capture_output=True, encoding="utf-8", errors="replace", timeout=60)
    message = completed.stderr.strip().replace(str(work.parent), "<qa-root>")
    for value in args:
        if isinstance(value, Path):
            message = message.replace(str(value), "<qa-path>")
    return {"exit": completed.returncode, "stderr": message[:1800],
            "space_diagnostic": bool(SPACE_ERROR.search(message))}


def exercise(case, cli, work, env, root, marker, identity, result):
    job = root / ("qa-" + marker) / case
    job.mkdir()
    source, archive = work / "payload.bin", work / "source.zip"
    (job / "unrelated.txt").write_bytes(b"Keep this pre-existing user file.\n")
    if case == "create":
        target = job / "result.zip"
        args = (target, source, "--format", "zip", "--store", "--threads", "1")
    else:
        target = job / source.name
        target.write_bytes(b"Keep this previous output on failure.\n")
        args = (archive, job, "--overwrite", "overwrite", "--threads", "1")

    def input_hashes():
        return {"source": sha(source), "archive": sha(archive)}

    inputs, before = input_hashes(), snapshot(job)
    result.update(state_before=before, input_sha256=inputs, free_before_fill=verify_volume(root, marker, identity).free)
    filler = job.parent / f"{case}-owned-filler.bin"
    try:
        fill(filler, root, marker, identity, result)
        result["status"] = "failed"
        result["full_attempt"] = invoke(cli, work, env, case, args)
        result["free_after_full_attempt"] = verify_volume(root, marker, identity).free
        result["state_after_full_attempt"], result["input_after_full_attempt"] = snapshot(job), input_hashes()
    finally:
        if filler.exists():
            verify_volume(root, marker, identity)
            filler.unlink()
            result["free_after_release"] = verify_volume(root, marker, identity).free

    check(result["free_after_release"] > PAYLOAD + MIB, "Filler removal did not restore sufficient space")
    result["retry"] = invoke(cli, work, env, case, args)
    result["state_after_retry"], result["input_after_retry"] = snapshot(job), input_hashes()
    expected = dict(before)
    if target.is_file() and case == "create":
        with zipfile.ZipFile(target) as created:
            check(created.namelist() == [source.name], "Retry ZIP directory changed")
            check(created.getinfo(source.name).compress_type == zipfile.ZIP_STORED, "Retry ZIP is not stored")
            result["retry_payload_sha256"] = hashlib.sha256(created.read(source.name)).hexdigest()
        expected[target.name] = sha(target)
    elif target.is_file():
        result["retry_payload_sha256"] = sha(target)
        expected[target.name] = inputs["source"]

    check(result["full_attempt"]["exit"] == 1 and result["full_attempt"]["space_diagnostic"],
          "Full-volume command did not report a handled disk-space error")
    check(result["state_after_full_attempt"] == before and result["input_after_full_attempt"] == inputs,
          "Full-volume failure changed input, destination, or directory entries")
    check(result["retry"]["exit"] == 0 and result.get("retry_payload_sha256") == inputs["source"]
          and result["state_after_retry"] == expected and result["input_after_retry"] == inputs,
          "Retry did not publish exact content while preserving unrelated files")
    result["status"] = "passed"

def reserve_for_midwrite(filler, root, marker, identity, headroom):
    usage = verify_volume(root, marker, identity)
    amount = (usage.free - headroom) // 4096 * 4096
    check(0 < amount < CAPACITY, "Insufficient isolated volume headroom to reserve")
    with filler.open("x+b", buffering=0) as handle:
        check(volume_identity(root, handle) == identity, "Reserve handle left verified volume")
        block = b"\x7d" * MIB
        while amount:
            chunk = block[:min(amount, len(block))]
            check(handle.write(chunk) == len(chunk), "Reserve write was short")
            amount -= len(chunk)
        os.fsync(handle.fileno())


def observe_extract(cli, work, env, args, stage_parents, root, prefix, payload_size, observations):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                                    ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.GetFileSizeEx.argtypes = [wintypes.HANDLE, ctypes.POINTER(ctypes.c_int64)]
    kernel.ReadFile.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD,
                                ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    size, count = ctypes.c_int64(), wintypes.DWORD()
    content = ctypes.create_string_buffer(len(prefix))
    largest = 0
    started = time.perf_counter()
    log = work / "midwrite-stderr.txt"
    with log.open("xb") as stderr:
        process = subprocess.Popen([cli, "extract", *args], cwd=work, env=env,
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=stderr)
        try:
            while process.poll() is None:
                elapsed = time.perf_counter() - started
                check(elapsed < 60, "Midwrite observation exceeded product timeout")
                observations["polls"] += 1
                for parent in stage_parents:
                    for directory in parent.glob(".qing-zip-stage-*"):
                        path = directory / "payload.bin"
                        # Open only the fixture's private file, read-only, sharing
                        # read, write and delete so cleanup remains unimpeded.
                        handle = kernel.CreateFileW(str(path), 0x80000000, 7, None, 3, 0x00200000, None)
                        if handle == wintypes.HANDLE(-1).value:
                            observations["open_errors"] += 1
                            continue
                        try:
                            if not kernel.GetFileSizeEx(handle, ctypes.byref(size)):
                                continue
                            if not largest < size.value < payload_size or size.value < len(prefix):
                                continue
                            if not kernel.ReadFile(handle, content, len(prefix), ctypes.byref(count), None):
                                continue
                            if count.value != len(prefix) or content.raw != prefix:
                                continue
                            largest = size.value
                            observations["samples"].append({
                                "elapsed_ms": round(elapsed * 1000, 2),
                                "file_bytes": largest,
                                "free_bytes": shutil.disk_usage(root).free,
                                "prefix_sha256": hashlib.sha256(content.raw).hexdigest(),
                                "private_temp_parent": parent.name,
                            })
                        finally:
                            kernel.CloseHandle(handle)
                time.sleep(0.001)
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()
    message = log.read_text(encoding="utf-8", errors="replace").strip()
    message = message.replace(str(work.parent), "<qa-root>").replace(str(root), "<qa-volume>/")
    return {"exit": process.returncode, "stderr": message[:1800],
            "space_diagnostic": bool(SPACE_ERROR.search(message))}


def exercise_midwrite(cli, work, env, root, marker, identity, result):
    # Above the 8 MiB memory-per-member threshold and small enough for a
    # complete extraction plus a second publication copy after releasing space.
    payload_size, headroom = 32 * MIB, 28 * MIB
    prefix_size = 64
    data = random.Random(112).randbytes(payload_size)
    archive = work / "midwrite.zip"
    with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_STORED) as fixture:
        fixture.writestr("payload.bin", data)
    job = root / ("qa-" + marker) / "midwrite"
    destination, temporary = job / "output", job / "temp"
    destination.mkdir(parents=True)
    temporary.mkdir()
    target, sentinel = destination / "payload.bin", destination / "unrelated.txt"
    target.write_bytes(b"Keep the existing destination after incomplete extraction.\n")
    sentinel.write_bytes(b"Keep this unrelated file.\n")
    args = (archive, destination, "--overwrite", "overwrite", "--threads", "1")
    env = dict(env, TEMP=str(temporary), TMP=str(temporary))
    before, archive_sha = snapshot(job), sha(archive)
    filler = job.parent / "midwrite-owned-reserve.bin"
    restored_required = 2 * payload_size + 4 * MIB
    free_before = verify_volume(root, marker, identity).free
    check(free_before >= restored_required, "Isolated volume cannot support an unconstrained retry")
    observations = {"polls": 0, "open_errors": 0, "samples": []}
    result.update(payload_bytes=payload_size, free_before_reserve=free_before, state_before=before,
                  archive_before=archive_sha, observations=observations,
                  expected_prefix_sha256=hashlib.sha256(data[:prefix_size]).hexdigest())
    try:
        reserve_for_midwrite(filler, root, marker, identity, headroom)
        free = verify_volume(root, marker, identity).free
        result["free_at_launch"] = free
        check(headroom - 64 * 1024 <= free < payload_size, "Expected genuine nonzero space shortfall")
        result["full_attempt"] = observe_extract(cli, work, env, args, (temporary, job, destination),
                                                 root, data[:prefix_size], payload_size, observations)
        result["free_after_failure"] = verify_volume(root, marker, identity).free
        result["state_after_failure"], result["archive_after_failure"] = snapshot(job), sha(archive)
    finally:
        if filler.exists():
            verify_volume(root, marker, identity)
            filler.unlink()
            result["free_after_release"] = verify_volume(root, marker, identity).free

    if result["state_after_failure"] != before or result["archive_after_failure"] != archive_sha:
        result["status"] = "failed"
        raise RuntimeError("Incomplete overwrite changed the original archive, existing target, or private stage tree")

    samples = observations["samples"]
    check(samples, "Did not observe actual payload bytes in an incomplete private staging file")
    check(any(payload_size - sample["file_bytes"] > sample["free_bytes"] for sample in samples),
          "Observed staging did not show a remaining payload larger than free space")
    failure = result["full_attempt"]
    # A failed native write can be larger than the remaining free clusters.
    # Polling also cannot capture the last instant before stage cleanup.
    os_error = re.search(
        r"os error (112|39)\b|0x800700(70|27)\b|"
        r"(?m:^System ERROR:[ \t]*\nThere is not enough space on the disk\.[ \t]*$)",
        failure["stderr"], re.I,
    )
    check(failure["space_diagnostic"] and os_error,
          "Could not confirm native disk exhaustion after observing staged payload bytes")
    result["native_disk_full_evidence"] = os_error.group()
    result["status"] = "failed"
    check(failure["exit"] == 1, "Expected a handled CLI failure after writing payload bytes")
    check(result["free_after_release"] >= restored_required, "Insufficient space after releasing owned reserve")
    result["retry"] = invoke(cli, work, env, "extract", args)
    expected_payload_sha = hashlib.sha256(data).hexdigest()
    expected_after = dict(before)
    expected_after["output/payload.bin"] = expected_payload_sha
    result["state_after_retry"], result["archive_after_retry"] = snapshot(job), sha(archive)
    result["expected_payload_sha256"] = expected_payload_sha
    check(result["retry"]["exit"] == 0 and result["state_after_retry"] == expected_after
          and result["archive_after_retry"] == archive_sha,
          "Retry did not publish the exact member and preserve all other files")
    result["status"] = "passed"


def main():
    check(os.name == "nt" and all(os.environ.get(key) == value for key, value in HOSTED.items()),
          "Requires a disposable GitHub-hosted Windows x64 job")
    job, letter, label, marker, source_commit, mode = sys.argv[1:]
    check(mode in {'workspace', 'historical', 'midwrite'}, 'Invalid test mode')
    job = Path(job).resolve(strict=True)
    check(job.parent == Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
          and job.name.startswith("qz-capacity-"), "Expected exact job-owned runner temp root")
    check(re.fullmatch(r"[D-Z]", letter) and re.fullmatch(r"QZ-[0-9a-f]{16}", label)
          and re.fullmatch(r"[0-9a-f]{32}", marker) and re.fullmatch(r"[0-9a-f]{40}", source_commit),
          "Invalid hosted fixture metadata")
    root = Path(f"{letter}:\\")
    check(root.drive.casefold() not in {job.drive.casefold(), os.environ["SystemDrive"].casefold()},
          "Refusing backing-file or system drive")
    receipt = job / "evidence/zip-space-receipt.json"
    report = {"status": "blocked", "product_source": source_commit, "fixed_vhd_bytes": CAPACITY, "cases": {}}

    def save():
        receipt.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    try:
        identity = volume_identity(root)
        check(identity[0] == label and identity[2] == "NTFS", "Unexpected VHD label or filesystem")
        verify_volume(root, marker, identity)
        (root / ("qa-" + marker)).mkdir()
        work = job / "fixtures"
        work.mkdir()
        cli = job / "stage/obz.exe"
        report.update(cli_sha256=sha(cli), worker_sha256=sha(job / "stage/obz-worker.exe"))
        env = dict(os.environ)
        for variable, name in (("TEMP", "tmp"), ("TMP", "tmp"), ("LOCALAPPDATA", "local"), ("APPDATA", "roaming")):
            owned = work / name
            owned.mkdir(exist_ok=True)
            env[variable] = str(owned)
        if mode != 'midwrite':
            source = work / "payload.bin"
            source.write_bytes(random.Random(112).randbytes(PAYLOAD))
            with zipfile.ZipFile(work / "source.zip", "x", compression=zipfile.ZIP_STORED) as fixture:
                fixture.write(source, source.name)
        if mode == "workspace":
            from zip_working_space import measure

            try:
                verify_volume(root, marker, identity)
                measurements = measure(cli, work, root, root / ("qa-" + marker), env, invoke, sha)
                verify_volume(root, marker, identity)
                report["ordinary_working_space"] = {"status": "passed", **measurements}
            except (RuntimeError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
                report["ordinary_working_space"] = {
                    "status": "failed",
                    "error": str(error).replace(str(job), "<qa-root>").replace(str(root), "<qa-volume>/"),
                }
            save()
        for case in ("midwrite",) if mode == 'midwrite' else ("create", "extract"):
            result = {"status": "blocked"}
            report["cases"][case] = result
            try:
                if case == "midwrite":
                    exercise_midwrite(cli, work, env, root, marker, identity, result)
                else:
                    exercise(case, cli, work, env, root, marker, identity, result)
            except (RuntimeError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
                result["error"] = str(error).replace(str(job), "<qa-root>").replace(str(root), "<qa-volume>/")
            save()
        statuses = {result["status"] for result in report["cases"].values()}
        if mode == "workspace":
            statuses.add(report["ordinary_working_space"]["status"])
        report["status"] = "passed" if statuses == {"passed"} else ("failed" if "failed" in statuses else "blocked")
    except (RuntimeError, OSError) as error:
        report["environment_error"] = str(error).replace(str(job), "<qa-root>")
    finally:
        save()
    if report["status"] != "passed":
        raise SystemExit(f"ZIP disk capacity acceptance {report['status']}; see receipt")


if __name__ == "__main__":
    main()
