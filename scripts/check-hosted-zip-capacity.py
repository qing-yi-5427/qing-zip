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


def main():
    check(os.name == "nt" and all(os.environ.get(key) == value for key, value in HOSTED.items()),
          "Requires a disposable GitHub-hosted Windows x64 job")
    job, letter, label, marker, source_commit = sys.argv[1:]
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
        source = work / "payload.bin"
        source.write_bytes(random.Random(112).randbytes(PAYLOAD))
        with zipfile.ZipFile(work / "source.zip", "x", compression=zipfile.ZIP_STORED) as fixture:
            fixture.write(source, source.name)
        cli = job / "stage/obz.exe"
        report.update(cli_sha256=sha(cli), worker_sha256=sha(job / "stage/obz-worker.exe"))
        env = dict(os.environ)
        for variable, name in (("TEMP", "tmp"), ("TMP", "tmp"), ("LOCALAPPDATA", "local"), ("APPDATA", "roaming")):
            owned = work / name
            owned.mkdir(exist_ok=True)
            env[variable] = str(owned)
        for case in ("create", "extract"):
            result = {"status": "blocked"}
            report["cases"][case] = result
            try:
                exercise(case, cli, work, env, root, marker, identity, result)
            except (RuntimeError, OSError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
                result["error"] = str(error).replace(str(job), "<qa-root>").replace(str(root), "<qa-volume>/")
            save()
        statuses = {result["status"] for result in report["cases"].values()}
        report["status"] = "passed" if statuses == {"passed"} else ("failed" if "failed" in statuses else "blocked")
    except (RuntimeError, OSError) as error:
        report["environment_error"] = str(error).replace(str(job), "<qa-root>")
    finally:
        save()
    if report["status"] != "passed":
        raise SystemExit(f"ZIP disk capacity acceptance {report['status']}; see receipt")


if __name__ == "__main__":
    main()
