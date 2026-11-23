"""Measure ordinary ZIP listing, selected extraction, and headless directory browsing."""

import argparse
import base64
import ctypes
from ctypes import wintypes
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess
import tempfile
import threading
import time
import zipfile


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("page_faults", wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in (
            "peak_working_set", "working_set", "peak_paged_pool", "paged_pool",
            "peak_nonpaged_pool", "nonpaged_pool", "commit", "peak_commit", "private_commit",
        )
    ]


def sample_process(pid, stop, samples):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    memory = kernel.K32GetProcessMemoryInfo
    memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(ProcessMemoryCounters), wintypes.DWORD]
    memory.restype = wintypes.BOOL
    # Query only the child we started, with no write privileges.
    handle = kernel.OpenProcess(0x0400 | 0x0010, False, pid)
    if not handle:
        return
    try:
        counters = ProcessMemoryCounters()
        counters.size = ctypes.sizeof(counters)
        while not stop.is_set() and memory(handle, ctypes.byref(counters), counters.size):
            samples.append((counters.peak_working_set, counters.private_commit))
            stop.wait(0.002)
    finally:
        kernel.CloseHandle(handle)


def run_cli(cli, work, *args):
    memory_samples = []
    stop = threading.Event()
    started = time.perf_counter()
    with subprocess.Popen([cli, *args], cwd=work, stdin=subprocess.DEVNULL,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          creationflags=subprocess.CREATE_NO_WINDOW) as child:
        sampler = threading.Thread(target=sample_process, args=(child.pid, stop, memory_samples))
        sampler.start()
        try:
            stdout, stderr = child.communicate(timeout=90)
            elapsed = (time.perf_counter() - started) * 1000
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate()
            raise
        finally:
            stop.set()
            sampler.join()
    if child.returncode or not memory_samples:
        raise RuntimeError(f"{args[0]} failed or produced no memory samples: "
                           f"{child.returncode} {stderr.decode('utf-8', errors='replace')}")
    working_sets, private_commits = zip(*memory_samples)
    return stdout, {
        "ms": round(elapsed, 3),
        "peak_working_set_bytes": max(working_sets),
        "max_sampled_private_commit_bytes": max(private_commits),
        "memory_samples": len(memory_samples),
    }


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def fixture(archive, count):
    expected = {}
    target = "wide/target-needle.txt"
    payload = b"ordinary archive listing fixture\n"
    with zipfile.ZipFile(archive, "x") as output:
        for i in range(count):
            if i == count // 2 - 1:
                name = target
            else:
                parent = "wide" if i < count // 2 else f"group-{i // 500:03}/nested"
                name = f"{parent}/member-{i:06}.txt"
            entry = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            output.writestr(entry, payload, compress_type=zipfile.ZIP_DEFLATED, compresslevel=1)
            expected[name] = len(payload)
    return expected, target, sha256(payload)


def app_section(source, start, end):
    begin = source.index(start)
    return source[begin:source.index(end, begin + len(start))]


def run_headless(edge, work, listing, count, rounds):
    root = Path(__file__).resolve().parents[1]
    frontend = root / "obz-gui/frontend"
    source = (frontend / "app.js").read_text(encoding="utf-8")
    sections = [
        "const " + app_section(source, "esc=v=>", ";\n") + ";",
        app_section(source, "const toolIcons=", "const btn="),
        app_section(source, "const filename=", "function sortableColumn("),
        app_section(source, "const folderViews =", "function applyArchiveFilter("),
        app_section(source, "const windowThreshold =", "function releaseListingWindow("),
        app_section(source, "const entryPositions =", "function rowButtons("),
        app_section(source, "const IMAGE_EXT=", "const previewArgs="),
    ]
    blob = base64.b64encode(listing).decode("ascii")
    data = json.dumps({"base64": blob, "count": count, "rounds": rounds})
    (work / "browser-input.js").write_text(
        "const sample = " + data + ";\n" + "\n".join(sections), encoding="utf-8"
    )
    script_tags = "\n".join(
        f'<script src="{html.escape(path.resolve().as_uri(), quote=True)}"></script>'
        for path in (
            frontend / "archive-listing.js", frontend / "archive-columns.js",
            frontend / "archive-row-window.js", work / "browser-input.js",
            root / "scripts/benchmark-archive-browse.js",
        )
    )
    page = work / "browser.html"
    page.write_text(f"""<!doctype html>
<meta charset="utf-8">
<title>Archive browser timing</title>
<style>
#viewport {{width: 980px; height: 576px; overflow: auto; --qa-row: 32px}}
table {{width: 100%; border-collapse: collapse}}
tr[data-row] {{height: 32px}}
svg {{width: 12px; height: 12px}}
</style>
<div id="viewport"><table><thead><tr><th>Files</th></tr></thead><tbody></tbody></table></div>
<pre id="result"></pre>
{script_tags}
""", encoding="utf-8")
    rendered = subprocess.run([
        edge, "--headless=new", "--disable-gpu", "--no-first-run", "--disable-background-networking",
        "--disable-extensions", "--allow-file-access-from-files", "--enable-precise-memory-info",
        "--window-size=1200,900", f"--user-data-dir={work / 'edge-profile'}", "--dump-dom",
        page.as_uri(),
    ], capture_output=True, stdin=subprocess.DEVNULL, encoding="utf-8", errors="replace",
        timeout=90, creationflags=subprocess.CREATE_NO_WINDOW)
    result = re.search(r'<pre id="result">([^<]+)</pre>', rendered.stdout)
    if rendered.returncode or result is None:
        raise RuntimeError(f"Headless browser failed: {rendered.returncode} {rendered.stderr[-1600:]}")
    report = json.loads(html.unescape(result.group(1)))
    if "error" in report:
        raise RuntimeError(report["error"])
    return report


def sample_archive(cli, edge, directory, count, rounds):
    work = directory / f"count-{count}"
    work.mkdir()
    archive = work / "browse.zip"
    expected, target, target_hash = fixture(archive, count)
    archive_hash = sha256(archive.read_bytes())
    samples = []
    for i in range(rounds + 1):
        listing, list_stats = run_cli(cli, work, "list", archive)
        entries = json.loads(listing)["entries"]
        actual = {entry["name"].replace("\\", "/"): entry["size"]
                  for entry in entries if not entry["is_dir"]}
        if len(entries) != count or actual != expected:
            raise RuntimeError(f"Unexpected directory listing for {count} entries")
        destination = work / f"out-{i}"
        _, extract_stats = run_cli(cli, work, "extract", archive, destination,
                                   "--include", target, "--threads", "4")
        output_files = [p.relative_to(destination).as_posix()
                        for p in destination.rglob("*") if p.is_file()]
        if output_files != [target] or sha256((destination / target).read_bytes()) != target_hash:
            raise RuntimeError("Selected extraction did not publish exactly the expected bytes")
        if sha256(archive.read_bytes()) != archive_hash:
            raise RuntimeError("Archive bytes changed")
        samples.append({"list": list_stats, "extract_one": extract_stats})
    return {"members": count, "archive_bytes": archive.stat().st_size, "archive_sha256": archive_hash,
            "selected_sha256": target_hash, "listing_json_bytes": len(listing),
            "native_warmup": samples[0], "native_samples": samples[1:],
            "browser": run_headless(edge, work, listing, count, rounds)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True, help="Staged obz.exe")
    parser.add_argument("--edge", type=Path, required=True, help="Microsoft Edge executable")
    parser.add_argument("--rounds", type=int, choices=range(3, 10), default=5, metavar="3-9")
    parser.add_argument("--receipt", type=Path, required=True, help="New file to receive JSON timings")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    edge = args.edge.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="qing-archive-browse-perf-") as temporary:
        directory = Path(temporary).resolve()
        report = {"cli_sha256": sha256(cli.read_bytes()), "status": "PASS", "cases": [
            sample_archive(cli, edge, directory, count, args.rounds) for count in (10000, 50000)
        ]}
    with args.receipt.open("x", encoding="utf-8") as output:
        print(json.dumps(report, indent=2), file=output)


if __name__ == "__main__":
    main()
