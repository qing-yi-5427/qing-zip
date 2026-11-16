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
data = {"binding":binding,"window_request":None,"display_state":"normal, visible, foreground requested; verified rectangle required", "driver_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "platform": platform.platform(), "cpu_count": psutil.cpu_count(),
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

def desktop_state():
    from ctypes import wintypes as W
    u=ctypes.WinDLL('user32',use_last_error=True)
    k=ctypes.WinDLL('kernel32',use_last_error=True)
    wts=ctypes.WinDLL('wtsapi32',use_last_error=True)
    u.OpenInputDesktop.argtypes=[W.DWORD,W.BOOL,W.DWORD];u.OpenInputDesktop.restype=W.HANDLE
    u.GetUserObjectInformationW.argtypes=[W.HANDLE,ctypes.c_int,W.LPVOID,W.DWORD,ctypes.POINTER(W.DWORD)]
    u.CloseDesktop.argtypes=[W.HANDLE]
    k.ProcessIdToSessionId.argtypes=[W.DWORD,ctypes.POINTER(W.DWORD)]
    wts.WTSQuerySessionInformationW.argtypes=[W.HANDLE,W.DWORD,ctypes.c_int,ctypes.POINTER(W.LPVOID),ctypes.POINTER(W.DWORD)]
    wts.WTSFreeMemory.argtypes=[W.LPVOID]
    sid=W.DWORD();assert k.ProcessIdToSessionId(os.getpid(),ctypes.byref(sid))
    ptr=W.LPVOID();size=W.DWORD()
    assert wts.WTSQuerySessionInformationW(None,sid.value,8,ctypes.byref(ptr),ctypes.byref(size))
    try:
        assert size.value>=4;state=ctypes.cast(ptr,ctypes.POINTER(ctypes.c_int)).contents.value
    finally:wts.WTSFreeMemory(ptr)
    desktop=u.OpenInputDesktop(0,False,1)
    if not desktop:raise RuntimeError('No readable interactive input desktop')
    try:
        name=ctypes.create_unicode_buffer(256);needed=W.DWORD()
        assert u.GetUserObjectInformationW(desktop,2,name,ctypes.sizeof(name),ctypes.byref(needed))
        monitors=[]
        monitor_callback=ctypes.WINFUNCTYPE(W.BOOL,W.HANDLE,W.HDC,ctypes.POINTER(W.RECT),W.LPARAM)
        @monitor_callback
        def visit_monitor(handle,dc,rect,context):
            r=rect.contents;monitors.append([r.left,r.top,r.right,r.bottom]);return True
        u.EnumDisplayMonitors.argtypes=[W.HDC,ctypes.POINTER(W.RECT),monitor_callback,W.LPARAM]
        if not u.EnumDisplayMonitors(None,None,visit_monitor,0):raise RuntimeError('Cannot enumerate display layout')
        value={'session_id':sid.value,'wts_connection_state':state,'input_desktop':name.value,'screen':[u.GetSystemMetrics(0),u.GetSystemMetrics(1)],'layout_metrics':{'metrics':{str(i):u.GetSystemMetrics(i) for i in [0,1,76,77,78,79,80]},'monitor_rectangles':sorted(monitors)}}
    finally:u.CloseDesktop(desktop)
    if state!=0 or name.value.lower()!='default':raise RuntimeError('Desktop not active/default: '+str(value))
    if data.get('display_layout') is not None and value['layout_metrics']!=data['display_layout']:
        raise RuntimeError('Display layout changed during comparison: '+str(value))
    return value

def remember(proc,owned):
    for p in [proc,*proc.children(recursive=True)]:
        try:owned[(p.pid,p.create_time())]=p
        except psutil.NoSuchProcess:pass

def snap(proc,owned):
    remember(proc,owned)
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
    initial_desktop=desktop_state()
    before = set(psutil.pids())
    configured=set()
    env=dict(os.environ)
    for key in ['OPEN_ARCHIVE_PROFILE','QZ_OUTPUT_CPU_PROFILE','QZ_OUTPUT_CPU_CALIBRATE']:env.pop(key,None)
    if label=='qing':
        env['LOCALAPPDATA']=str(result/'qing-localappdata')
        env['WEBVIEW2_USER_DATA_FOLDER']=str(result/'qing-webview')
    start = time.perf_counter()
    child = subprocess.Popen([str(exe), str(archive)],env=env)
    owned={}
    try:remember(psutil.Process(child.pid),owned)
    except psutil.NoSuchProcess:pass
    app = None
    item = None
    row = {"app":label, "rep":rep,"initial_desktop":initial_desktop}
    data["trials"].append(row)
    try:
        while time.perf_counter() - start < 50:
            app = app or root(exe, before)
            if app:remember(app,owned)
            for hwnd in windows(app.pid) if app else []:
                try:
                    if hwnd not in configured:
                        hwnds.ShowWindow(ctypes.c_void_p(hwnd),9)
                        if not hwnds.SetWindowPos(ctypes.c_void_p(hwnd),None,*data["window_request"],0x0040): raise RuntimeError('Window sizing failed')
                        hwnds.SetForegroundWindow(ctypes.c_void_p(hwnd))
                        configured.add(hwnd)
                    rect=(ctypes.c_long*4)()
                    assert hwnds.GetWindowRect(ctypes.c_void_p(hwnd),ctypes.byref(rect))
                    x,y,width,height=data['window_request']
                    if tuple(rect)!=(x,y,x+width,y+height):continue
                    screen=desktop_state()['screen']
                    if rect[0]<0 or rect[1]<0 or rect[2]>screen[0] or rect[3]>screen[1]:
                        raise RuntimeError('Window outside primary display')
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
        row["visible_desktop"]=desktop_state()
        if rep == 0:
            wr = win.CurrentBoundingRectangle
            screen = ImageGrab.grab((wr.left,wr.top,wr.right,wr.bottom), all_screens=True)
            row["screenshot_channel_stddev"] = ImageStat.Stat(screen.convert("RGB")).stddev
            screen.save(result / (label + "-desktop.png"))
            rgb=screen.convert('RGB');extrema=rgb.getextrema()
            row['screenshot_size']=list(rgb.size);row['screenshot_extrema']=extrema
            # Reject blank/black captures, but preserve original PNG for human review.
            if min(rgb.size)<600 or max(row['screenshot_channel_stddev'])<3 or max(hi-lo for lo,hi in extrema)<30:
                raise RuntimeError('Blank or invalid desktop screenshot; original retained')
        time.sleep(3)
        samples = [snap(app,owned)]
        for i in range(4):
            time.sleep(1)
            desktop_state()
            samples.append(snap(app,owned))
        row.update(idle=samples, median_rss=statistics.median(x["rss"] for x in samples),
                   median_private=statistics.median(x["private"] for x in samples))
        # Process identities must be stable before reporting a CPU delta.
        if len({tuple(sorted((p["pid"],p["created"]) for p in x["members"])) for x in samples}) == 1:
            row["idle_cpu_one_core_percent"] = (samples[-1]["cpu_s"]-samples[0]["cpu_s"])*100/(samples[-1]["clock"]-samples[0]["clock"])
        print(label, rep, row["visible_ms"], row["median_rss"], row["median_private"], flush=True)
    finally:
        cleanup_error=None
        try:
            if app:
                try:remember(app,owned)
                except psutil.NoSuchProcess:pass
                for h in windows(app.pid):hwnds.PostMessageW(ctypes.c_void_p(h),0x10,0,0)
            # Only exact Process objects captured in our owned tree are signalled.
            def observe_until(deadline):
                while True:
                    for proc in list(owned.values()):
                        try:
                            if proc.is_running():remember(proc,owned)
                        except psutil.NoSuchProcess:pass
                    live=[p for p in owned.values() if p.is_running()]
                    if not live or time.perf_counter()>=deadline:return live
                    time.sleep(.05)
            stubborn=observe_until(time.perf_counter()+6)
            for proc in stubborn:
                try:proc.terminate()
                except psutil.NoSuchProcess:pass
            survivors=observe_until(time.perf_counter()+3)
            row['cleanup_survivors']=[{'pid':p.pid,'created':p.create_time()} for p in survivors if p.is_running()]
            try:child.wait(timeout=3)
            except subprocess.TimeoutExpired:row['launcher_still_live']=True
            if row['cleanup_survivors'] or child.poll() is None:
                raise RuntimeError('Owned process cleanup incomplete; aborting further rounds')
            row['cleanup_complete']=True
        except Exception as error:
            cleanup_error=error;row['cleanup_error']=repr(error)
        finally:save()
        if cleanup_error:raise cleanup_error
apps = [("qing", qing), ("nana", nana)]
try:
    display=desktop_state()
    width=min(1200,display['screen'][0]-80)
    height=min(800,display['screen'][1]-80)
    if width<780 or height<620:
        raise RuntimeError('Display cannot fit common qing minimum780x620 with margins; no display changes attempted')
    data['display_layout']=display['layout_metrics']
    data['window_request']=[40,40,width,height]
    data['initial_display']=display
    save()
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
