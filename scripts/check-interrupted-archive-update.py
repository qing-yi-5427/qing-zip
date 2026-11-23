"""Interrupt a normal packaged CLI update while another independent archive completes."""
import argparse
import ctypes
from ctypes import wintypes
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import subprocess
import tempfile
import time
import zipfile

import psutil


class ProcessIo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in
                ('reads', 'writes', 'other', 'read_bytes', 'write_bytes', 'other_bytes')]


@contextmanager
def observed_writer(pid):
    api = ctypes.WinDLL('kernel32', use_last_error=True)
    api.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    api.OpenProcess.restype = wintypes.HANDLE
    api.CloseHandle.argtypes = [wintypes.HANDLE]
    api.GetProcessIoCounters.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessIo)]
    api.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    api.WaitForSingleObject.restype = wintypes.DWORD
    # Query the exact observed writer after exit, without granting termination.
    process = api.OpenProcess(0x100000 | 0x1000, False, pid)
    if not process:
        raise ctypes.WinError(ctypes.get_last_error())

    def written():
        stats = ProcessIo()
        if not api.GetProcessIoCounters(process, ctypes.byref(stats)):
            raise ctypes.WinError(ctypes.get_last_error())
        return stats.write_bytes

    def wait():
        if api.WaitForSingleObject(process, 45_000) != 0:
            raise RuntimeError('The observed update writer did not exit')

    try:
        yield written, wait
    finally:
        api.CloseHandle(process)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def contents(archive):
    with zipfile.ZipFile(archive) as opened:
        return {name: digest(opened.read(name)) for name in opened.namelist()}


def retained_files(job, scratch):
    # Include the app's private recovery folders, which are siblings of the archive.
    # Classify by area to avoid persisting random temp paths in a published receipt.
    counts = {}
    for folder in job.iterdir():
        if folder.is_dir():
            kind = "temp" if folder == scratch else "archive_sibling"
            files = [item for item in folder.rglob("*") if item.is_file()]
            data = counts.setdefault(kind, {"roots": 0, "files": 0, "bytes": 0})
            data["roots"] += 1
            data["files"] += len(files)
            data["bytes"] += sum(item.stat().st_size for item in files)
    return counts


def trial(cli, root, round_number, require_owned):
    job = root / f"update-{round_number}"
    job.mkdir()
    scratch = job / "temp"
    scratch.mkdir()
    env = {**os.environ, "TEMP": str(scratch), "TMP": str(scratch),
           "LOCALAPPDATA": str(scratch / "local"), "APPDATA": str(scratch / "roaming")}
    source = job / "addition.bin"
    source.write_bytes(random.Random(round_number).randbytes(128 * 1024 * 1024))
    original = b"existing archive member"
    archive = job / "editing.zip"
    with zipfile.ZipFile(archive, "w") as opened:
        opened.writestr("keep.txt", original)
    original_hash = digest(archive.read_bytes())
    added_hash = digest(source.read_bytes())
    other_archive = job / "independent.zip"
    other_input = job / "other.txt"
    other_input.write_bytes(random.Random(1000 + round_number).randbytes(64 * 1024 * 1024))
    command = [cli, "add", archive, source]
    # Children inherit the temporary paths above. Only the explicit editing
    # process is terminated. Descendants are observed to measure actual cleanup.
    start_time = time.monotonic()
    with (job / "output.log").open("wb") as log:
        with subprocess.Popen(command, cwd=job, env=env, stdin=subprocess.DEVNULL,
                              stdout=log, stderr=subprocess.STDOUT) as edit:
            with subprocess.Popen([cli, "create", other_archive, other_input, "--format", "zip"],
                                  cwd=job, env=env, stdin=subprocess.DEVNULL,
                                  stdout=log, stderr=subprocess.STDOUT) as other:
                editor = psutil.Process(edit.pid)
                seen = {}
                active = None
                writer_pid = None
                independent_active = None
                observations = {}
                max_other_write = 0
                other_observations = {}
                deadline = time.monotonic() + 30
                while edit.poll() is None and time.monotonic() < deadline:
                    active = None
                    independent_active = None
                    try:
                        children = editor.children(recursive=True)
                    except psutil.NoSuchProcess:
                        break
                    for child in children:
                        try:
                            image = Path(child.exe())
                            if not image.is_relative_to(cli.parent):
                                continue
                            seen[child.pid] = child
                            io = child.io_counters()
                            argv = child.cmdline()
                            key = f"{image.name}:{argv[1] if len(argv)>1 else ''}"
                            observations[key] = max(observations.get(key, 0), io.read_bytes)
                            if (len(argv) > 1 and argv[1] == 'a' and str(archive).casefold() in [arg.casefold() for arg in argv]
                                    and str(source).casefold() in [arg.casefold() for arg in argv]
                                    and io.read_bytes >= 2 * 1024 * 1024 and io.write_bytes >= 1024 * 1024):
                                active = {"image": image.name, "command": "a", "read_bytes": io.read_bytes,
                                          "write_bytes": io.write_bytes, "arguments_match_fixture": True,
                                          "observed_seconds": round(time.monotonic() - start_time, 3)}
                                writer_pid = child.pid
                        except (psutil.NoSuchProcess, psutil.AccessDenied):
                            continue
                    try:
                        worker = psutil.Process(other.pid)
                        for process in [worker, *worker.children(recursive=True)]:
                            io = process.io_counters()
                            args = process.cmdline()
                            role = Path(process.exe()).name + ':' + (args[1] if len(args) > 1 else '')
                            max_other_write = max(max_other_write, io.write_bytes)
                            other_observations[role] = max(other_observations.get(role, 0), io.write_bytes)
                            if (len(args)>1 and args[1]=='a' and str(other_input).casefold() in [arg.casefold() for arg in args]
                                    and io.write_bytes >= 1024 * 1024 and other.poll() is None):
                                independent_active = {"image": Path(process.exe()).name, "command":"a", "input_matches_fixture":True, "write_bytes": io.write_bytes,
                                                      "observed_seconds": round(time.monotonic() - start_time, 3)}
                    except (psutil.NoSuchProcess, psutil.AccessDenied):
                        pass
                    if active and independent_active and other.poll() is None:
                        break
                    time.sleep(0.01)
                if not active or not independent_active or edit.poll() is not None or other.poll() is not None:
                    log.flush()
                    detail = (job / 'output.log').read_text(encoding='utf-8', errors='replace')[-2000:]
                    raise RuntimeError(f"Could not observe both jobs concurrently: edit={edit.poll()}, other={other.poll()}, update={active}, independent={independent_active}, seen={observations}, independent_seen={other_observations}, independent_max_write={max_other_write}, log={detail}")
                with observed_writer(writer_pid) as (writer_bytes, wait_writer):
                    before = writer_bytes()
                    interrupted_at = time.monotonic()
                    independent_running_at_kill = other.poll() is None
                    edit.kill()
                    exit_code = edit.wait(timeout=30)
                    owner_exited_at = time.monotonic()
                    at_owner_exit = writer_bytes()
                    wait_writer()
                    released_after_owner_exit = time.monotonic() - owner_exited_at
                    final = writer_bytes()
                bytes_after_owner_exit = final - at_owner_exit
                if require_owned and (released_after_owner_exit >= 1 or bytes_after_owner_exit != 0):
                    raise RuntimeError(f'Update writer outlived its owner for {released_after_owner_exit:.3f}s '
                                       f'and wrote {bytes_after_owner_exit} additional bytes')
                start = owner_exited_at
                _, alive = psutil.wait_procs(list(seen.values()), timeout=45)
                cleanup_seconds = time.monotonic() - start
                if alive:
                    raise RuntimeError("An observed helper remained alive after the interruption")
                if not exit_code or not independent_running_at_kill or other.wait(timeout=120) != 0:
                    raise RuntimeError("Unexpected completion state of interrupted or independent job")
                independent_observed_done = time.monotonic()
    independent = contents(other_archive)
    if independent != {"other.txt": digest(other_input.read_bytes())}:
        raise RuntimeError("The independent job produced incorrect bytes")
    reopen_start = time.monotonic()
    reopened = subprocess.run([cli, "list", archive], cwd=job, env=env, capture_output=True,
                              stdin=subprocess.DEVNULL, timeout=60, check=True)
    reopen_seconds = time.monotonic() - reopen_start
    if ([entry["name"] for entry in json.loads(reopened.stdout)["entries"]] != ["keep.txt"]
            or digest(archive.read_bytes()) != original_hash or digest(source.read_bytes()) != added_hash):
        raise RuntimeError("The interrupted archive or the input changed")
    after_reopen = retained_files(job, scratch)
    retry_start = time.monotonic()
    subprocess.run(command, cwd=job, env=env, capture_output=True, stdin=subprocess.DEVNULL,
                   timeout=120, check=True)
    retry_seconds = time.monotonic() - retry_start
    if contents(archive) != {"keep.txt": digest(original), source.name: added_hash} or digest(source.read_bytes()) != added_hash:
        raise RuntimeError("Retried archive did not contain the expected original and added bytes")
    return {"round": round_number, "observed": active, "independent_observed": independent_active,
            "interrupted_seconds": round(interrupted_at-start_time, 3),
            "independent_running_at_kill": independent_running_at_kill, "exit_code": exit_code,
            "independent_completion_observed_by_seconds": round(independent_observed_done-start_time, 3),
            "reopen_seconds": round(reopen_seconds, 3), "retry_seconds": round(retry_seconds, 3),
            "observed_helpers_released_seconds": round(cleanup_seconds, 3),
            "writer_released_after_owner_exit_seconds": round(released_after_owner_exit, 4),
            "writer_bytes_before_interrupt": before,
            "writer_bytes_at_owner_exit": at_owner_exit,
            "writer_bytes_written_after_owner_exit": bytes_after_owner_exit,
            "archive_unchanged_after_reopen": True, "source_unchanged": True,
            "independent_job_verified": True, "retry_verified": True,
            "retained_after_reopen": after_reopen, "retained_after_retry": retained_files(job, scratch)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--out", type=Path, help="write sanitized JSONL to a new file")
    parser.add_argument("--require-owned", action="store_true", help="require the update writer to stop with its owner")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="qing-zip-interrupted-update-") as work:
        receipt = args.out.open('x', encoding='utf-8') if args.out else None
        try:
            for number in range(1, 4):
                line = json.dumps({"cli_sha256": digest(cli.read_bytes()), "script_sha256": digest(Path(__file__).read_bytes()),
                                   "platform": platform.platform(), "python": platform.python_version(),
                                   "psutil": psutil.__version__, **trial(cli, Path(work), number, args.require_owned)})
                print(line, flush=True)
                if receipt:
                    receipt.write(line+'\n'); receipt.flush()
        finally:
            if receipt: receipt.close()
