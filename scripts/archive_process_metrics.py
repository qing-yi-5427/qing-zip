"""Sample a locally launched Windows CLI and the verified images in its process tree."""

import _winapi
import ctypes as c
from ctypes import wintypes as w
import msvcrt
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time


class ProcessEntry(c.Structure):
    _fields_ = [
        ("size", w.DWORD), ("usage", w.DWORD), ("pid", w.DWORD),
        ("heap", c.c_size_t), ("module", w.DWORD), ("threads", w.DWORD),
        ("parent_pid", w.DWORD), ("priority", w.LONG), ("flags", w.DWORD),
        ("image", w.WCHAR * 260),
    ]


class MemoryCounters(c.Structure):
    _fields_ = [
        ("size", w.DWORD), ("page_faults", w.DWORD),
        *((name, c.c_size_t) for name in (
            "peak_working_set", "working_set", "peak_paged_pool", "paged_pool",
            "peak_nonpaged_pool", "nonpaged_pool", "pagefile", "peak_pagefile",
            "private_commit", "private_working_set",
        )),
        ("shared_commit", c.c_uint64),
    ]


class JobLimits(c.Structure):
    _fields_ = [
        ("process_time", c.c_int64), ("job_time", c.c_int64), ("flags", w.DWORD),
        ("min_working_set", c.c_size_t), ("max_working_set", c.c_size_t),
        ("active_process_limit", w.DWORD), ("affinity", c.c_size_t),
        ("priority", w.DWORD), ("scheduling", w.DWORD),
    ]


class JobMemory(c.Structure):
    _fields_ = [
        ("limits", JobLimits), ("io_counters", c.c_uint64 * 6),
        ("process_limit", c.c_size_t), ("job_limit", c.c_size_t),
        ("peak_process_commit", c.c_size_t), ("peak_job_commit", c.c_size_t),
    ]


class JobAccounting(c.Structure):
    _fields_ = [
        ("times", c.c_int64 * 4), ("page_faults", w.DWORD),
        ("total_processes", w.DWORD), ("active_processes", w.DWORD),
        ("terminated_processes", w.DWORD),
    ]


def win32():
    kernel = c.WinDLL("kernel32", use_last_error=True)
    for name, result, params in (
        ("CreateToolhelp32Snapshot", w.HANDLE, [w.DWORD, w.DWORD]),
        ("Process32FirstW", w.BOOL, [w.HANDLE, c.POINTER(ProcessEntry)]),
        ("Process32NextW", w.BOOL, [w.HANDLE, c.POINTER(ProcessEntry)]),
        ("OpenProcess", w.HANDLE, [w.DWORD, w.BOOL, w.DWORD]),
        ("GetProcessTimes", w.BOOL, [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4),
        ("QueryFullProcessImageNameW", w.BOOL, [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]),
        ("K32GetProcessMemoryInfo", w.BOOL, [w.HANDLE, c.POINTER(MemoryCounters), w.DWORD]),
        ("WaitForSingleObject", w.DWORD, [w.HANDLE, w.DWORD]),
        ("CreateJobObjectW", w.HANDLE, [c.c_void_p, w.LPCWSTR]),
        ("AssignProcessToJobObject", w.BOOL, [w.HANDLE, w.HANDLE]),
        ("QueryInformationJobObject", w.BOOL,
         [w.HANDLE, c.c_int, c.c_void_p, w.DWORD, c.POINTER(w.DWORD)]),
        ("TerminateJobObject", w.BOOL, [w.HANDLE, w.UINT]),
        ("ResumeThread", w.DWORD, [w.HANDLE]),
        ("GetSystemTimes", w.BOOL, [c.POINTER(w.FILETIME)] * 3),
        ("CloseHandle", w.BOOL, [w.HANDLE]),
    ):
        api = getattr(kernel, name)
        api.restype, api.argtypes = result, params
    return kernel


def process_parents(kernel):
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == w.HANDLE(-1).value:
        raise c.WinError(c.get_last_error())
    try:
        entry = ProcessEntry()
        entry.size = c.sizeof(entry)
        result = {}
        compilers = set()
        ok = kernel.Process32FirstW(snapshot, c.byref(entry))
        while ok:
            result[entry.pid] = entry.parent_pid
            image = entry.image.lower()
            if image in {"cargo.exe", "rustc.exe", "cl.exe", "link.exe", "msbuild.exe"}:
                compilers.add(image)
            ok = kernel.Process32NextW(snapshot, c.byref(entry))
        return result, compilers
    finally:
        kernel.CloseHandle(snapshot)


def process_identity(kernel, handle):
    times = [w.FILETIME() for _ in range(4)]
    if not kernel.GetProcessTimes(handle, *(c.byref(value) for value in times)):
        return None
    birth = (times[0].dwHighDateTime << 32) | times[0].dwLowDateTime
    image = c.create_unicode_buffer(32768)
    size = w.DWORD(len(image))
    if not kernel.QueryFullProcessImageNameW(handle, 0, image, c.byref(size)):
        return None
    return birth, Path(image.value)


def discover_children(kernel, cli, pid, epoch, handles, lock, started, stop, result):
    runtime = cli.parent / "runtime"
    try:
        while not stop.is_set():
            parents, compilers = process_parents(kernel)
            result["compilers"].update(compilers)
            if pid not in parents:
                break
            tree = {pid}
            while True:
                children = {child for child, parent in parents.items() if parent in tree} - tree
                if not children:
                    break
                tree.update(children)
            for member in sorted(tree - handles.keys()):
                handle = kernel.OpenProcess(0x101000, False, member)
                if not handle:
                    continue
                identity = process_identity(kernel, handle)
                if identity is None or identity[0] < epoch or not (
                        identity[1].is_relative_to(runtime)
                        or identity[1] == cli.parent / "obz-worker.exe"):
                    kernel.CloseHandle(handle)
                    continue
                with lock:
                    handles[member] = (handle, identity[1].relative_to(cli.parent).as_posix())
            result["discovery_elapsed_ms"].append(round((time.perf_counter() - started) * 1000, 3))
            time.sleep(0.001)
    except Exception as error:
        result["sampler_error"] = str(error)


def sample_tree(kernel, cli, pid, started, interval, stop, result):
    handles = {}
    discovery = None
    try:
        root_handle = kernel.OpenProcess(0x101000, False, pid)  # query and wait only
        if not root_handle:
            raise c.WinError(c.get_last_error())
        handles[pid] = (root_handle, cli.name)
        root = process_identity(kernel, root_handle)
        if root is None or root[1] != cli:
            raise RuntimeError("The sampled process does not match the launched CLI")
        lock = threading.Lock()
        discovery = threading.Thread(target=discover_children, args=(
            kernel, cli, pid, root[0], handles, lock, started, stop, result))
        discovery.start()
        next_tick = time.perf_counter()
        while not stop.is_set() and kernel.WaitForSingleObject(root_handle, 0) == 258:
            now = time.perf_counter()
            with lock:
                known = tuple(handles.values())
            row = {}
            for handle, image in known:
                if kernel.WaitForSingleObject(handle, 0) != 258:
                    continue
                counters = MemoryCounters()
                counters.size = c.sizeof(counters)
                if kernel.K32GetProcessMemoryInfo(handle, c.byref(counters), counters.size):
                    entry = row.setdefault(image, {"processes": 0, "working_set_bytes": 0,
                                                  "private_working_set_bytes": 0, "private_commit_bytes": 0})
                    entry["processes"] += 1
                    entry["working_set_bytes"] += counters.working_set
                    entry["private_working_set_bytes"] += counters.private_working_set
                    entry["private_commit_bytes"] += counters.private_commit
            if row:
                result["samples"].append({"elapsed_ms": round((now - started) * 1000, 3),
                                          "images": row})
            next_tick = max(next_tick + interval, now + interval)
            time.sleep(max(0.0, next_tick - time.perf_counter()))
    except Exception as error:
        result["sampler_error"] = str(error)
    finally:
        stop.set()
        if discovery:
            discovery.join()
        for handle, _ in handles.values():
            kernel.CloseHandle(handle)


def job_information(kernel, handle, kind, structure):
    info = structure()
    if not kernel.QueryInformationJobObject(handle, kind, c.byref(info), c.sizeof(info), None):
        raise c.WinError(c.get_last_error())
    return info


def completed_job(kernel, job):
    # A process can signal its own exit before Job accounting observes it.
    deadline = time.monotonic() + 0.25
    while True:
        counts = job_information(kernel, job, 1, JobAccounting)
        if not counts.active_processes:
            return counts.total_processes
        if time.monotonic() >= deadline:
            raise RuntimeError("The measured CLI left an active process in its observation job")
        time.sleep(0.001)


def system_times(kernel):
    fields = [w.FILETIME() for _ in range(3)]
    if not kernel.GetSystemTimes(*(c.byref(field) for field in fields)):
        raise c.WinError(c.get_last_error())
    return [(field.dwHighDateTime << 32) | field.dwLowDateTime for field in fields]


def run(kernel, cli, args, work, env, *, interval_ms, use_job):
    """Return elapsed time, process-tree samples and the optional kernel job peak."""
    with (open(os.devnull, "rb") as stdin,
          tempfile.TemporaryFile(dir=work) as stdout,
          tempfile.TemporaryFile(dir=work) as stderr):
        streams = [msvcrt.get_osfhandle(file.fileno()) for file in (stdin, stdout, stderr)]
        startup = subprocess.STARTUPINFO(
            dwFlags=subprocess.STARTF_USESTDHANDLES,
            hStdInput=streams[0], hStdOutput=streams[1], hStdError=streams[2],
            lpAttributeList={"handle_list": streams},
        )
        report = {"samples": [], "discovery_elapsed_ms": [], "compilers": set()}
        command = subprocess.list2cmdline([str(cli), *map(str, args)])
        job = kernel.CreateJobObjectW(None, None) if use_job else None
        if use_job and not job:
            raise c.WinError(c.get_last_error())
        proc, thread, pid, sampler, stopped = None, None, None, None, threading.Event()
        clean = False
        try:
            for stream in streams:
                os.set_handle_inheritable(stream, True)
            before = system_times(kernel)
            started = time.perf_counter()
            try:
                # Assign the root process before its code can launch a child.
                # The observation job applies no scheduling or memory limits.
                flags = subprocess.CREATE_NO_WINDOW | (0x4 if use_job else 0)
                proc, thread, pid, _ = _winapi.CreateProcess(str(cli), command, None, None, True,
                                                             flags, env, str(work), startup)
            finally:
                for stream in streams:
                    os.set_handle_inheritable(stream, False)
            if job and not kernel.AssignProcessToJobObject(job, proc):
                raise c.WinError(c.get_last_error())
            sampler = threading.Thread(target=sample_tree, args=(
                kernel, cli, pid, started, interval_ms / 1000, stopped, report))
            sampler.start()
            if job and kernel.ResumeThread(thread) == 0xFFFFFFFF:
                raise c.WinError(c.get_last_error())
            if _winapi.WaitForSingleObject(proc, 180_000) != _winapi.WAIT_OBJECT_0:
                raise RuntimeError("Measured CLI did not complete in time")
            report["elapsed_ms"] = round((time.perf_counter() - started) * 1000, 3)
            report["exit_code"] = _winapi.GetExitCodeProcess(proc)
            after = system_times(kernel)
            if job:
                total_processes = completed_job(kernel, job)
                peak = job_information(kernel, job, 9, JobMemory).peak_job_commit
                if not peak:
                    raise RuntimeError("Windows did not return a job memory peak")
                report.update(job_peak_private_commit_bytes=peak, job_total_processes=total_processes)
            clean = True
        finally:
            if proc and not clean:
                if job:
                    kernel.TerminateJobObject(job, 2)
                # Assignment can fail before the new process enters the job.
                if _winapi.WaitForSingleObject(proc, 0) != _winapi.WAIT_OBJECT_0:
                    _winapi.TerminateProcess(proc, 2)
                _winapi.WaitForSingleObject(proc, 10_000)
            stopped.set()
            if sampler:
                sampler.join()
            for handle in (thread, proc, job):
                if handle:
                    kernel.CloseHandle(handle)
        stdout.seek(0)
        stderr.seek(0)
        out, err = stdout.read(), stderr.read()
    if report["exit_code"] != 0:
        raise RuntimeError(f"{args[0]} returned {report['exit_code']}: {err.decode(errors='replace')}")
    if "sampler_error" in report or not report["samples"]:
        raise RuntimeError(report.get("sampler_error", "No process memory was observed"))
    idle, kernel_ticks, user = [a - b for a, b in zip(after, before)]
    report["system_cpu_percent"] = round(100 * (kernel_ticks + user - idle) / (kernel_ticks + user), 2)
    report["compilers"] = sorted(report["compilers"])
    report["discovery_ms"] = report.pop("discovery_elapsed_ms")
    for metric in ("working_set_bytes", "private_working_set_bytes", "private_commit_bytes"):
        report["peak_sampled_" + metric] = max(sum(image[metric] for image in row["images"].values())
                                                for row in report["samples"])
    report["sampled_image_names"] = sorted({image for row in report["samples"] for image in row["images"]})
    return out, err, report
