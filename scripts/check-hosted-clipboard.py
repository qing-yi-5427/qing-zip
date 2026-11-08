"""Physical Windows clipboard export and paste on a disposable hosted desktop."""
from contextlib import contextmanager
from pathlib import Path
import ctypes, hashlib, json, os, subprocess, tempfile, time, zipfile

assert os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted" and os.environ.get("GITHUB_ACTIONS") == "true"
from PIL import ImageGrab
from pywinauto import Desktop, keyboard, mouse
import win32clipboard
from win32com.client import Dispatch

u = ctypes.WinDLL("user32")
u.SetProcessDPIAware()
evidence = Path("clipboard-evidence")
evidence.mkdir()
exe, = (Path(os.environ["RUNNER_TEMP"])/"qing-trial").rglob("qing-zip.exe")
digest = lambda b:hashlib.sha256(b).hexdigest()
assert digest(exe.read_bytes()) == os.environ["QING_CLIP_GUI_SHA"]
report = {"binary":digest(exe.read_bytes()), "tested_source":os.environ["QING_CLIP_SOURCE_SHA"],
          "source_run":os.environ["QING_CLIP_SOURCE_RUN"],"source_artifact":os.environ["QING_CLIP_ARTIFACT_DIGEST"]}
if "QING_QA_ASSET_ID" in os.environ:
    report.update(actual_release_asset=os.environ["QING_QA_ASSET_ID"],architecture=os.environ["QING_QA_ARCH"])
desktop = Desktop(backend="uia")
def wait(fn, description, seconds=40):
    deadline=time.monotonic()+seconds
    while time.monotonic()<deadline:
        value=fn()
        if value:return value
        time.sleep(.08)
    raise RuntimeError("Timed out: "+description)
def manifest(where):
    return {p.relative_to(where).as_posix():digest(p.read_bytes()) for p in where.rglob("*") if p.is_file()}
def clip(action):
    def attempt():
        try:win32clipboard.OpenClipboard()
        except Exception:return None
        try:return True,action()
        finally:win32clipboard.CloseClipboard()
    return wait(attempt,"access clipboard",8)[1]
def set_sentinel():
    def seed():
        win32clipboard.EmptyClipboard()
        win32clipboard.SetClipboardText("qing hosted clipboard sentinel",13)
    clip(seed)
def sentinel_preserved():
    return clip(lambda:win32clipboard.IsClipboardFormatAvailable(13) and win32clipboard.GetClipboardData(13)=="qing hosted clipboard sentinel" and not win32clipboard.IsClipboardFormatAvailable(15))
def drop_paths():
    return clip(lambda:list(win32clipboard.GetClipboardData(15)) if win32clipboard.IsClipboardFormatAvailable(15) else [])
def find(win, title, kind):
    return next((v for v in win.descendants(control_type=kind) if v.element_info.name==title and v.is_visible()),None)
def click(button):
    r=button.rectangle()
    mouse.click(coords=(int((r.left+r.right)/2),int((r.top+r.bottom)/2)))
@contextmanager
def application(root, archive):
    tmp=root/"temp";tmp.mkdir()
    profile=root/"profile";local=root/"local";local.mkdir()
    env={**os.environ,"TMP":str(tmp),"TEMP":str(tmp),"LOCALAPPDATA":str(local),
         "WEBVIEW2_USER_DATA_FOLDER":str(profile)}
    process=subprocess.Popen([str(exe),str(archive)],env=env)
    try:
        if "QING_QA_ARCH" in os.environ:
            from ctypes import wintypes as wt
            k=ctypes.WinDLL("kernel32",use_last_error=True)
            k.IsWow64Process2.argtypes=[wt.HANDLE,ctypes.POINTER(wt.WORD),ctypes.POINTER(wt.WORD)]
            process_machine,native_machine=wt.WORD(),wt.WORD()
            assert k.IsWow64Process2(wt.HANDLE(process._handle),ctypes.byref(process_machine),ctypes.byref(native_machine))
            assert process_machine.value==0 and native_machine.value=={"x64":0x8664,"arm64":0xaa64}[os.environ["QING_QA_ARCH"]]
            report["native_machine"]=hex(native_machine.value)
        win=wait(lambda:next((w for w in desktop.windows(process=process.pid) if w.is_visible() and w.window_text()=="qing-zip"),None),"visible main qing-zip window")
        win.set_focus()
        yield process,win,tmp
    except Exception:
        ImageGrab.grab().save(evidence/(root.name+"-failure.png"))
        if "win" in locals():
            report[root.name+"_controls"]=[(v.element_info.control_type,v.element_info.name) for v in win.descendants() if v.is_visible()][:100]
        raise
    finally:
        if process.poll() is None:
            process.terminate();process.wait(timeout=10)
def select_and_copy(main,names):
    for name in names:
        box=wait(lambda:find(main,"选择 "+name,"CheckBox"),"checkbox "+name)
        click(box)
    def checked():
        current=[find(main,"选择 "+name,"CheckBox") for name in names]
        return all(box is not None and box.get_toggle_state()==1 for box in current)
    wait(checked,"selected checkbox states")
    # A focused checkbox is an HTML input; the application intentionally leaves
    # keyboard editing shortcuts alone there. Focus the selected name button.
    button=wait(lambda:find(main,names[-1],"Button"),"selected row control")
    button.set_focus()
    keyboard.send_keys("^c")
def stages(tmp):
    return sorted(p.name for p in tmp.glob("qing-zip-drag-*"))
def job_status(main,status):
    return find(main,status,"Text")
def exercise_negative(base,case):
    root=base/case;root.mkdir();archive=root/"input.zip"
    with zipfile.ZipFile(archive,"w",compression=zipfile.ZIP_STORED) as z:
        if case=="cancel":
            for i in range(12000):
                z.writestr(f"bulk/f{i:05d}.txt",("payload"+str(i)).encode()*32)
        else:
            z.writestr("bad.txt",b"qing-damaged-copy-payload")
    if case=="corrupt":
        original=archive.read_bytes();assert original.count(b"qing-damaged-copy-payload")==1
        archive.write_bytes(original.replace(b"qing-damaged-copy-payload",b"Qing-damaged-copy-payload"))
    set_sentinel()
    with application(root,archive) as (proc,main,tmp):
        before=stages(tmp);assert before==[]
        select_and_copy(main,["bulk" if case=="cancel" else "bad.txt"])
        if case=="cancel":
            active=wait(lambda:stages(tmp),"private export appeared")
            assert len(active)==1
            click(wait(lambda:find(main,"取消任务","Button"),"real cancel job"))
            status="已取消"
        else:status="失败"
        wait(lambda:job_status(main,status),"durable copy status "+status)
        wait(lambda:stages(tmp)==before,"discard private export")
        assert sentinel_preserved()
        report[case]={"status":status,"stages_before":before,"stages_after":stages(tmp),"clipboard_unchanged":True}
        if case=="cancel":report[case]["active_stages"]=active
def exercise_paste(base):
    root=base/"paste";root.mkdir();archive=root/"mixed.zip";dest=root/"Destination";dest.mkdir()
    files={"文档 & 1.txt":b"selected plain","嵌套目录/子层/#one.txt":b"nested payload","未选择.txt":b"ignored"}
    selected_names=["文档 & 1.txt","嵌套目录"]
    expected={k:digest(v) for k,v in files.items() if k!="未选择.txt"}
    with zipfile.ZipFile(archive,"w") as z:
        for name,data in files.items():z.writestr(name,data)
    set_sentinel()
    with application(root,archive) as (proc,main,tmp):
        select_and_copy(main,selected_names)
        paths=wait(drop_paths,"CF_HDROP")
        assert sorted(Path(p).name for p in paths)==sorted(selected_names),paths
        stage_names=stages(tmp);assert len(stage_names)==1
        stage=tmp/stage_names[0]
        assert all(Path(p).parent.samefile(stage) for p in paths)
        assert manifest(stage)==expected
        wait(lambda:job_status(main,"已完成"),"successful copy")
        ImageGrab.grab().save(evidence/"copied.png")
        main.set_focus();keyboard.send_keys("%{F4}")
        assert proc.wait(timeout=20)==0
        assert drop_paths()==paths and manifest(stage)==expected
    shell=Dispatch("Shell.Application")
    shell.Open(str(dest))
    def target():
        for wnd in shell.Windows():
            try:
                if Path(wnd.Document.Folder.Self.Path).samefile(dest):return wnd
            except (OSError,AttributeError):pass
    explorer=wait(target,"real Explorer destination")
    try:
        win=desktop.window(handle=int(explorer.HWND)).wrapper_object()
        win.set_focus()
        view=next((ctrl for ctrl in win.descendants() if ctrl.element_info.class_name=="UIItemsView" and ctrl.is_visible()),None)
        if view is not None:
            click(view)
        else:
            # The folder is empty. Focus its content pane, below the address bar
            # and to the right of Explorer navigation, on either Explorer layout.
            rect=win.rectangle()
            mouse.click(coords=(int(rect.left+rect.width()*.72),int(rect.top+rect.height()*.66)))
        keyboard.send_keys("^v")
        wait(lambda:manifest(dest)==expected,"post-exit Explorer paste",30)
        ImageGrab.grab().save(evidence/"pasted.png")
        report["paste"]={"clipboard_basenames":[Path(p).name for p in paths],"exited_before_paste":True,"destination":manifest(dest),"private_export_retained":stage_names}
    finally:explorer.Quit()
try:
    with tempfile.TemporaryDirectory(prefix="qing-clipboard-",dir=os.environ["RUNNER_TEMP"],ignore_cleanup_errors=True) as temp:
        root=Path(temp)
        exercise_negative(root,"cancel")
        exercise_negative(root,"corrupt")
        exercise_paste(root)
except Exception as exc:
    report["error"]=repr(exc);ImageGrab.grab().save(evidence/"failure.png");raise
finally:
    (evidence/"receipt.json").write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding="utf8")