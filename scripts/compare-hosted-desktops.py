"""Measure visible archive content and an idle process tree on a disposable x64 runner."""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time
import traceback
import zipfile

import psutil
from PIL import ImageGrab, ImageStat
from pywinauto.uia_defines import IUIA

qing, nana, result = map(lambda x: Path(x).resolve(), sys.argv[1:])
assert qing.name.lower() == "qing-zip.exe"
assert nana.name.lower() == "nanazip.modern.filemanager.exe"
data = {"platform": platform.platform(), "cpu_count": psutil.cpu_count(),
        "executables": {x.name: hashlib.sha256(x.read_bytes()).hexdigest() for x in (qing, nana)},
        "trials": [], "notes": [
            "Startup measures request to first UIA descendant with matching filename, visible bounds and IsOffscreen false.",
            "First launch per app is distinguished. Subsequent samples are warm, not disk-cold.",
            "Working set sums double count shared pages. Private bytes are committed private memory, not resident bytes.",
            "Descendants of the file manager only. Unrelated WebView processes, Explorer, DWM and other shared services excluded.",
            "The accessibility probe runs outside either process tree. Display capture and idle samples follow the timed observation."]}
def save():
    (result / "measurements.json").write_text(json.dumps(data, indent=2))
save()
machine = IUIA().iuia
# UIA client setup is complete before either application's first launch.
fname = "A001-archive-content-visible"
archive = result / "gui-probe.zip"
with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr(fname, "make visible in both file managers")
    for i in range(120):
        z.writestr(f"item-{i:03}.txt", f"sample {i}")
data["archive_sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
data["fixture_members"] = 121
data["marker"] = fname
name_filter = machine.CreatePropertyCondition(30005, fname)
hwnds = ctypes.windll.user32
def windows(pid):
    found = []
    @ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_ssize_t)
    def visit(hwnd, ignored):
        owner = ctypes.c_ulong()
        hwnds.GetWindowThreadProcessId(ctypes.c_void_p(hwnd), ctypes.byref(owner))
        if owner.value == pid and hwnds.IsWindowVisible(ctypes.c_void_p(hwnd)):
            found.append(hwnd)
        return True
    hwnds.EnumWindows(visit, 0)
    return found

def root(exe, seen):
    for p in psutil.process_iter(["pid", "name"]):
        if p.pid not in seen and (p.info["name"] or "").lower() == exe.name.lower():
            try:
                if os.path.samefile(exe, p.exe()): return p
            except (OSError, psutil.Error):
                pass

def snap(proc):
    rows = []
    for p in [proc, *proc.children(recursive=True)]:
        try:
            with p.oneshot():
                m, cpu = p.memory_info(), p.cpu_times()
                rows.append({"pid": p.pid, "name": p.name(), "rss": m.rss,
                             "private": m.private, "cpu_s": cpu.user + cpu.system})
        except (psutil.NoSuchProcess, psutil.ZombieProcess):
            pass
    return {"clock": time.perf_counter(), "members": rows, "rss":sum(r["rss"] for r in rows),
            "private":sum(r["private"] for r in rows), "cpu_s":sum(r["cpu_s"] for r in rows)}

def run(label, exe, rep):
    before = set(psutil.pids())
    start = time.perf_counter()
    child = subprocess.Popen([str(exe), str(archive)])
    app = None
    item = None
    row = {"app":label, "rep":rep}
    data["trials"].append(row)
    try:
        while time.perf_counter() - start < 50:
            app = app or root(exe, before)
            for hwnd in windows(app.pid) if app else []:
                try:
                    win = machine.ElementFromHandle(hwnd)
                    item = win.FindFirst(4, name_filter)
                    if item and not item.CurrentIsOffscreen:
                        box = item.CurrentBoundingRectangle
                        if box.right > box.left and box.bottom > box.top: break
                except Exception:
                    pass
                item = None
            if item: break
            time.sleep(.04)
        if not item:
            row["visible_pids"] = [(p.pid, p.info["name"]) for p in psutil.process_iter(["name"])
                                   if p.pid not in before and p.info["name"]]
            raise RuntimeError(f"{label} archive entry never observed within 50s")
        row.update(visible_ms=(time.perf_counter()-start)*1000, marker=item.CurrentName,
                   root_pid=app.pid, hwnd=int(hwnd), bounds=[box.left,box.top,box.right,box.bottom])
        if rep == 0:
            wr = win.CurrentBoundingRectangle
            screen = ImageGrab.grab((wr.left,wr.top,wr.right,wr.bottom), all_screens=True)
            row["screenshot_channel_stddev"] = ImageStat.Stat(screen.convert("RGB")).stddev
            screen.save(result / (label + "-desktop.png"))
        time.sleep(3)
        samples = [snap(app)]
        for i in range(4):
            time.sleep(1)
            samples.append(snap(app))
        row.update(idle=samples, median_rss=statistics.median(x["rss"] for x in samples),
                   median_private=statistics.median(x["private"] for x in samples))
        # Process identities must be stable before reporting a CPU delta.
        if len({tuple(sorted(p["pid"] for p in x["members"])) for x in samples}) == 1:
            row["idle_cpu_one_core_percent"] = (samples[-1]["cpu_s"]-samples[0]["cpu_s"])*100/(samples[-1]["clock"]-samples[0]["clock"])
        print(label, rep, row["visible_ms"], row["median_rss"], row["median_private"], flush=True)
    finally:
        if app:
            children = app.children(recursive=True)
            for h in windows(app.pid): hwnds.PostMessageW(ctypes.c_void_p(h),0x10,0,0)
            _, stubborn = psutil.wait_procs([app, *children], timeout=6)
            for p in stubborn:
                try: p.terminate()
                except psutil.NoSuchProcess: pass
            psutil.wait_procs(stubborn, timeout=3)
        if child.poll() is None: child.terminate()
        save()
apps = [("qing", qing), ("nana", nana)]
try:
    for rep in range(4):
        for name, exe in (apps if rep % 2 == 0 else reversed(apps)):
            run(name, exe, rep)
    data["complete"] = True
except Exception:
    data["error"] = traceback.format_exc()
    raise
finally:
    save()
