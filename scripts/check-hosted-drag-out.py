"""Exercise WebView2 pointer events and Windows OLE using a disposable, live Explorer."""
from pathlib import Path
import ctypes as c, hashlib, json, os, subprocess, tempfile, time, urllib.parse, zipfile

assert os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted' and os.environ.get('GITHUB_ACTIONS')=='true'
from ctypes import wintypes as w
u=c.WinDLL('user32',use_last_error=True)
u.SetWindowPos.argtypes=[w.HWND,w.HWND,c.c_int,c.c_int,c.c_int,c.c_int,w.UINT]
u.SetWindowPos.restype=w.BOOL
u.GetAncestor.argtypes=[w.HWND,w.UINT];u.GetAncestor.restype=w.HWND
u.WindowFromPoint.argtypes=[w.POINT];u.WindowFromPoint.restype=w.HWND
def place(hwnd,z,x,y,width,height):
    if not u.SetWindowPos(hwnd,z,x,y,width,height,0x50):raise c.WinError(c.get_last_error())

u.SetProcessDPIAware()
from PIL import ImageGrab
from pywinauto import Desktop, mouse
from win32com.client import Dispatch

result=Path('drag-evidence');result.mkdir()
exe=next((Path(os.environ['RUNNER_TEMP'])/'qing-trial').rglob('qing-zip.exe'))
assert hashlib.sha256(exe.read_bytes()).hexdigest()=='319112564bbeb8fbb18f6a9a2f45576dd05d7e215ec4b458250b373f48884ac7'
report={'binary':hashlib.sha256(exe.read_bytes()).hexdigest()}
def wait(fn,why,seconds=30):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        obj=fn()
        if obj:return obj
        time.sleep(.15)
    raise RuntimeError('Timed out: '+why)
def save(label):
    ImageGrab.grab().save(result/(label+'.png'))
def owner(point):
    x,y=point
    return u.GetAncestor(u.WindowFromPoint(w.POINT(x,y)),2)
def center(rect):
    return (int((rect.left+rect.right)/2),int((rect.top+rect.bottom)/2))
def find(win,name,kind):
    hits=[a for a in win.descendants(control_type=kind) if a.element_info.name==name and a.is_visible()]
    return hits[0] if hits else None
def manifest(p):
    return {f.relative_to(p).as_posix():hashlib.sha256(f.read_bytes()).hexdigest() for f in p.rglob('*') if f.is_file()}
with tempfile.TemporaryDirectory(dir=os.environ['RUNNER_TEMP'],prefix='qing-drag-study-',ignore_cleanup_errors=True) as tmp:
    scratch=Path(tmp);dest=scratch/'Destination';dest.mkdir()
    archive=scratch/'selection.zip'
    contents={'take.txt':b'first file', 'nested/one.txt':b'inside selected folder','dont-copy.txt':b'not selected'}
    with zipfile.ZipFile(archive,'w') as z:
        for name,data in contents.items():z.writestr(name,data)
    shell=Dispatch('Shell.Application')
    shell.Open(str(dest))
    def target_window():
        for w in shell.Windows():
            try:
                if os.path.normcase(w.Document.Folder.Self.Path)==os.path.normcase(str(dest)):return w
            except Exception:pass
    target=wait(target_window,'Explorer destination'); target_hwnd=int(target.HWND)
    drag_temp=scratch/"drag-temp";drag_temp.mkdir()
    def drag_stages():return {p.name:manifest(p) for p in drag_temp.glob("qing-zip-drag-*")}
    app=subprocess.Popen([exe,archive],env={**os.environ,'TEMP':str(drag_temp),'TMP':str(drag_temp),'WEBVIEW2_USER_DATA_FOLDER':str(scratch/'profile')})
    desktop=Desktop(backend='uia')
    try:
        win=wait(lambda:next((w for w in desktop.windows(process=app.pid) if w.is_visible()),None),'application')
        width,height=u.GetSystemMetrics(0),u.GetSystemMetrics(1)
        left_width=min(840,width-150)
        u.ShowWindow(target_hwnd,9);u.SetWindowPos(target_hwnd,-1,200,5,width-205,height-65,0x40)
        u.ShowWindow(win.handle,9);u.SetWindowPos(win.handle,-1,0,0,left_width,height-65,0x40)
        u.SetForegroundWindow(win.handle)
        row=wait(lambda:find(win,'take.txt','Button'),'archive file')
        for name in ['take.txt','nested']:
            cb=wait(lambda:find(win,'选择 '+name,'CheckBox'),'checkbox '+name)
            coords=center(cb.rectangle());assert owner(coords)==win.handle,(coords,owner(coords),win.handle)
            mouse.click(coords=coords)
        # Place windows after WebView startup and Explorer navigation have settled.
        # Explorer remains topmost so clicking the source cannot cover the drop.
        u.ShowWindow(win.handle,9);place(win.handle,-2,0,0,left_width,height-65)
        u.ShowWindow(target_hwnd,9);place(target_hwnd,-1,width-320,5,750,height-65)
        time.sleep(.6)
        point=center(row.rectangle())
        xp=desktop.window(handle=target_hwnd).wrapper_object()
        report['desktop']=[width,height];report['source_rect']=str(win.rectangle());report['destination_rect']=str(xp.rectangle())
        # The right part of the folder's item view stays exposed beside the archiver.
        qing_right=win.rectangle().right
        xr=xp.rectangle();drop=(width-55,int(xr.top+xr.height()*.65))
        report.update(source=point,destination=drop,source_owner=owner(point),target_owner=owner(drop),expected_hwnds=[win.handle,target_hwnd])
        assert owner(point)==win.handle and owner(drop)==target_hwnd,report
        report['drop_control']=desktop.from_point(*drop).element_info.name
        save('before')
        def move_selection():
            mouse.press(coords=point)
            for distance in (3,8,15,22):
                mouse.move(coords=(point[0]+distance,point[1]));time.sleep(.16)
            time.sleep(1.0)
            for step in range(1,15):
                x=int((point[0]+22)*(1-step/14)+drop[0]*step/14);y=int(point[1]*(1-step/14)+drop[1]*step/14)
                mouse.move(coords=(x,y));time.sleep(.075)
            time.sleep(.5)
        move_selection()
        # Hold Escape through a mouse move while the primary button remains
        # pressed, then inspect the durable job record before starting again.
        u.keybd_event(0x1b,1,0,0)
        try:
            time.sleep(.15)
            assert u.GetAsyncKeyState(0x1b)&0x8000
            mouse.move(coords=(drop[0]-4,drop[1]))
            time.sleep(.3)
        finally:u.keybd_event(0x1b,1,2,0)
        mouse.release(coords=drop);time.sleep(.6)
        report['after_escape']=manifest(dest)
        task=wait(lambda:find(win,'任务','Button'),'task navigation')
        mouse.click(coords=center(task.rectangle()))
        wait(lambda:find(win,'已取消拖放。','Text'),'cancelled drag acknowledgement',6)
        assert manifest(dest)=={}
        mouse.click(coords=center(find(win,'文件','Button').rectangle()))
        wait(lambda:find(win,'take.txt','Button'),'return to archive')
        report['after_escape_stages']=drag_stages()
        report['escape_left_empty_destination']=True
        move_selection();save('dragging');mouse.release(coords=drop)
        expected={name:hashlib.sha256(data).hexdigest() for name,data in contents.items() if name!='dont-copy.txt'}
        wait(lambda:manifest(dest)==expected,'exact dropped files',25)
        report['verified']=manifest(dest)
        time.sleep(.75);report['after_copy_stages']=drag_stages()
        save('after')
    except Exception as exc:
        report['error']=repr(exc);save('failure')
        if 'win' in locals():
            report['visible_controls']=[(a.element_info.control_type,a.element_info.name) for a in win.descendants() if a.is_visible()][:160]
        raise
    finally:
        mouse.release();(result/'receipt.json').write_text(json.dumps(report,indent=2,ensure_ascii=False),encoding='utf8')
        if app.poll() is None:app.terminate();app.wait(timeout=10)
        target.Quit()
