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

import argparse
parser=argparse.ArgumentParser()
for key in ['qing','nana','archive','result','binding']: parser.add_argument('--'+key,required=True,type=Path)
a=parser.parse_args()
qing,nana,archive,result=map(lambda p:p.resolve(),[a.qing,a.nana,a.archive,a.result])
assert os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted', 'Disposable hosted desktop only; no user-settings changes'
assert not result.exists();result.mkdir(parents=True)
binding=json.loads(a.binding.read_text(encoding='utf-8-sig'))
assert binding['source_commit'] and binding['nana_package_full_name']
assert binding['qing_dependencies'] and binding['nana_dependencies']
for rel,expected in binding['nana_dependencies'].items():
    target=nana.parent/rel
    assert target.resolve().is_relative_to(nana.parent)
    assert hashlib.sha256(target.read_bytes()).hexdigest()==expected
for label,path in [('qing',qing),('nana',nana),('archive',archive)]:
    assert hashlib.sha256(path.read_bytes()).hexdigest()==binding[label+'_sha256']
for rel,expected in binding['qing_dependencies'].items():
    target=qing.parent/rel
    assert target.resolve().is_relative_to(qing.parent)
    assert hashlib.sha256(target.read_bytes()).hexdigest()==expected
assert not any((p.info['name'] or '').lower() in [qing.name.lower(),nana.name.lower()] for p in psutil.process_iter(['name'])), 'Existing application instance; do not disturb'

assert qing.name.lower() in ("qing-zip.exe","obz-gui.exe")
assert nana.name.lower() == "nanazip.modern.filemanager.exe"
data = {"binding":binding,"window_request":[40,40,1200,800],"display_state":"normal, visible, foreground requested; verified rectangle required", "driver_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "platform": platform.platform(), "cpu_count": psutil.cpu_count(),
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
with zipfile.ZipFile(archive) as z:
    assert len(z.infolist())==121 and fname in z.namelist()
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
                rows.append({"pid": p.pid, "created":p.create_time(), "name": p.name(), "rss": m.rss,
                             "private": m.private, "cpu_s": cpu.user + cpu.system})
        except (psutil.NoSuchProcess, psutil.ZombieProcess):
            pass
    return {"clock": time.perf_counter(), "members": rows, "rss":sum(r["rss"] for r in rows),
            "private":sum(r["private"] for r in rows), "cpu_s":sum(r["cpu_s"] for r in rows)}

def run(label, exe, rep):
    before = set(psutil.pids())
    configured=set()
    env=dict(os.environ)
    for key in ['OPEN_ARCHIVE_PROFILE','QZ_OUTPUT_CPU_PROFILE','QZ_OUTPUT_CPU_CALIBRATE']:env.pop(key,None)
    if label=='qing':
        env['LOCALAPPDATA']=str(result/'qing-localappdata')
        env['WEBVIEW2_USER_DATA_FOLDER']=str(result/'qing-webview')
    start = time.perf_counter()
    child = subprocess.Popen([str(exe), str(archive)],env=env)
    app = None
    item = None
    row = {"app":label, "rep":rep}
    data["trials"].append(row)
    try:
        while time.perf_counter() - start < 50:
            app = app or root(exe, before)
            for hwnd in windows(app.pid) if app else []:
                try:
                    if hwnd not in configured:
                        hwnds.ShowWindow(ctypes.c_void_p(hwnd),9)
                        if not hwnds.SetWindowPos(ctypes.c_void_p(hwnd),None,40,40,1200,800,0x0040): raise RuntimeError('Window sizing failed')
                        hwnds.SetForegroundWindow(ctypes.c_void_p(hwnd))
                        configured.add(hwnd)
                    rect=(ctypes.c_long*4)()
                    assert hwnds.GetWindowRect(ctypes.c_void_p(hwnd),ctypes.byref(rect))
                    if tuple(rect)!=(40,40,1240,840):continue
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
                   root_pid=app.pid, hwnd=int(hwnd), window_bounds=list(rect), bounds=[box.left,box.top,box.right,box.bottom])
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
        if len({tuple(sorted((p["pid"],p["created"]) for p in x["members"])) for x in samples}) == 1:
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
    for label,path in [('qing',qing),('nana',nana),('archive',archive)]:
        assert hashlib.sha256(path.read_bytes()).hexdigest()==binding[label+'_sha256']
    for rel,expected in binding['qing_dependencies'].items():assert hashlib.sha256((qing.parent/rel).read_bytes()).hexdigest()==expected
    for rel,expected in binding['nana_dependencies'].items():assert hashlib.sha256((nana.parent/rel).read_bytes()).hexdigest()==expected
    data["summary"]={name:{key:statistics.median(r[key] for r in data['trials'] if r['app']==name and r['rep']>0) for key in ['visible_ms','median_rss','median_private']} for name in ['qing','nana']}
    data["complete"] = True
except Exception:
    data["error"] = traceback.format_exc()
    raise
finally:
    save()
