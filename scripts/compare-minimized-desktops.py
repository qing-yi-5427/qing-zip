"""Compare two production workbenches under the same Windows UI and memory sampling script."""
import ctypes
from ctypes import wintypes as W
import hashlib
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import sys
import tempfile
import time
import zipfile

import psutil
from pywinauto.uia_defines import IUIA

assert os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
base, candidate, output = map(lambda s: Path(s).resolve(), sys.argv[1:])
output.mkdir(exist_ok=True)
ui = IUIA().iuia
win = ctypes.windll.user32
win.ShowWindow.argtypes = [W.HWND, ctypes.c_int]
win.IsIconic.argtypes = [W.HWND]
win.GetWindowThreadProcessId.argtypes = [W.HWND, ctypes.POINTER(W.DWORD)]
win.EnumWindows.argtypes = [ctypes.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM), W.LPARAM]
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
result = {"base_sha": digest(base), "candidate_sha": digest(candidate), "trials": []}
def save():
    (output/"samples.json").write_text(json.dumps(result, indent=2), "utf8")
def wait(label, read, seconds=55):
    end = time.monotonic()+seconds
    while time.monotonic()<end:
        value = read()
        if value: return value
        time.sleep(.06)
    raise RuntimeError("Timeout: "+label)
def hwnd_for(pid):
    found = []
    @ctypes.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
    def visit(h, _):
        owner=W.DWORD()
        win.GetWindowThreadProcessId(h,ctypes.byref(owner))
        if owner.value==pid and win.IsWindowVisible(h): found.append(h)
        return True
    win.EnumWindows(visit,0)
    return found[0] if found else None
def elem(h, prop, value):
    return ui.ElementFromHandle(h).FindFirst(4,ui.CreatePropertyCondition(prop,value))
def click(h,name):
    e=wait(name,lambda:elem(h,30005,name))
    e.GetCurrentPattern(10000).QueryInterface(IUIA().ui_automation_client.IUIAutomationInvokePattern).Invoke()
def sample(p):
    members=[]
    for process in [p,*p.children(recursive=True)]:
        try:
            with process.oneshot():
                m=process.memory_info()
                members.append((process.pid,process.name(),m.rss,m.private))
        except (psutil.NoSuchProcess,psutil.ZombieProcess):
            pass
    return {"pids":[r[0] for r in members],"rss":sum(r[2] for r in members),"private":sum(r[3] for r in members)}
def samples(p,n=5):
    points=[]
    for _ in range(n):
        points.append(sample(p))
        time.sleep(.8)
    return points
def show(h, state):
    win.ShowWindow(h,6 if state=="min" else 9)
    wait(state,lambda:bool(win.IsIconic(h))==(state=="min"))
def run(root,archive,label,exe,rep):
    scratch=root/f"{label}-{rep}";scratch.mkdir()
    app=subprocess.Popen([str(exe),str(archive)],env=dict(os.environ,WEBVIEW2_USER_DATA_FOLDER=str(scratch/"webview")))
    p=psutil.Process(app.pid)
    row={"name":label,"rep":rep}
    result["trials"].append(row)
    try:
        h=wait("main window",lambda:hwnd_for(app.pid))
        wait("50000 member view",lambda:elem(h,30005,"file-00000.txt"))
        time.sleep(4)
        row["visible"]=samples(p)
        show(h,"min");time.sleep(8)
        row["minimized"]=samples(p,8)
        start=time.perf_counter()
        show(h,"normal")
        click(h,"设置")
        wait("restored settings",lambda:elem(h,30011,"qa-theme") or elem(h,30005,"????"))
        row["restore_ms"]=(time.perf_counter()-start)*1000
        click(h,"文件")
        wait("restored files",lambda:elem(h,30005,"file-00000.txt"))
        row["restored"]=samples(p)
        if rep==0: background(exe,h,scratch,row)
    except Exception:
        elements=ui.ElementFromHandle(h).FindAll(4,ui.CreateTrueCondition())
        row["screen"]=[(elements.GetElement(i).CurrentControlType,elements.GetElement(i).CurrentName,elements.GetElement(i).CurrentAutomationId) for i in range(min(elements.Length,90))]
        print("window",row["screen"],flush=True)
        raise
    finally:
        children=p.children(recursive=True) if p.is_running() else []
        app.terminate();app.wait(timeout=10)
        _,still=psutil.wait_procs(children,timeout=5)
        for process in still:
            try: process.terminate()
            except psutil.NoSuchProcess: pass
        save()
def background(exe,h,scratch,row):
    work=scratch/"sources";work.mkdir()
    one=work/"one.bin";two=work/"two.txt"
    with one.open("wb") as out:
        rng=random.Random(451);out.write(rng.randbytes(64*1024*1024))
    two.write_bytes(b"second file")
    subprocess.run([exe,"--compress-files",str(one),str(two)],timeout=20,check=True)
    wait("real create form",lambda:elem(h,30005,"2 个来源"))
    click(h,"创建压缩文件");show(h,"min")
    dest=work/"sources.zip"
    def complete():
        try:
            with zipfile.ZipFile(dest) as z:
                return len(z.infolist())==2
        except (FileNotFoundError,PermissionError,zipfile.BadZipFile):return False
    wait("create output while minimized",complete,90)
    assert win.IsIconic(h)
    with zipfile.ZipFile(dest) as z:
        assert set(z.namelist())=={one.name,two.name}
        assert hashlib.sha256(z.read(one.name)).hexdigest()==digest(one)
        assert z.read(two.name)==two.read_bytes()
    show(h,"normal")
    wait("post-create integrity check",lambda:elem(h,30005,"完整性检查通过"))
    click(h,"关闭")
    queue=[]
    for i in range(2):
        folder=scratch/f"queue-{i}";folder.mkdir()
        source=folder/f"archive{i}.zip"
        with zipfile.ZipFile(source,"w",compression=zipfile.ZIP_DEFLATED,compresslevel=1) as z:
            for j in range(350): z.writestr(f"entry-{j}.txt",b"data"*32)
            z.writestr(f"end-{i}.bin", b"last"+bytes([i]))
        queue.append(source)
    subprocess.run([exe,"--extract-here",*map(str,queue)],timeout=20,check=True)
    assert not (queue[1].parent/"end-1.bin").exists()
    show(h,"min")
    wait("first queued extraction while minimized",lambda:(queue[0].parent/"end-0.bin").exists(),45)
    assert win.IsIconic(h)
    second=queue[1].parent/"end-1.bin"
    queue_fix=os.environ.get("CANDIDATE")=="42459789971" and row["name"]=="candidate"
    if queue_fix:
        wait("second queued extraction while minimized",second.exists,60)
        assert win.IsIconic(h)
        row["queue_behavior"]="both completed while minimized"
    else:
        time.sleep(4)
        row["queue_behavior"]="both completed while minimized" if second.exists() else "first completed; second needed progress dialog dismissal"
        if not second.exists():
            show(h,"normal")
            click(h,"??")
            wait("queue continuation after dismissing progress",second.exists)
    for i,source in enumerate(queue):
        assert (source.parent/f"end-{i}.bin").read_bytes()==b"last"+bytes([i])
        assert len(list(source.parent.glob('entry-*.txt')))==350
    show(h,"normal")
    result.setdefault("background",[]).append(str(exe))
with tempfile.TemporaryDirectory(dir=os.environ["RUNNER_TEMP"],ignore_cleanup_errors=True) as tmp:
    root=Path(tmp);archive=root/"large.zip"
    with zipfile.ZipFile(archive,"w",zipfile.ZIP_STORED) as z:
        for i in range(50000): z.writestr(f"file-{i:05d}.txt",str(i))
    for rep in range(1 if os.environ.get("CANDIDATE") in ("baseline","42459789971") else 3):
        order=[("base",base),("candidate",candidate)]
        if rep%2:order.reverse()
        for label,exe in order:run(root,archive,label,exe,rep)
result["complete"]=True;save()
