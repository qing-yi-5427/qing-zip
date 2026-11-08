"""Exercise the context menu on a disposable, active Windows 11 desktop."""
from pathlib import Path
import ctypes, json, os, subprocess, sys, tempfile, time, traceback, urllib.request, winreg, zipfile
from PIL import ImageGrab
from pywinauto import Desktop, mouse, keyboard
from pywinauto.uia_defines import IUIA
from websockets.sync.client import connect

exe, results = [Path(x).resolve() for x in sys.argv[1:]]
assert os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
assert int(sys.getwindowsversion().build) >= 22000
root = Path(tempfile.mkdtemp(prefix="Qing Explorer visible "))
report = {"architecture":os.environ["RUNNER_ARCH"], "folder":str(root), "menus":{}, "actions":{}}
def save():
    (results/'observations.json').write_text(json.dumps(report,indent=2,ensure_ascii=False),encoding='utf8')
def wait(fn,timeout=12):
    stop=time.monotonic()+timeout
    while time.monotonic()<stop:
        value=fn()
        if value:return value
        time.sleep(.2)
    raise RuntimeError("Timed out: "+fn.__name__)
def command(key):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,key) as h:return winreg.QueryValueEx(h,None)[0]
    except FileNotFoundError:return None
archive_reg=r'Software\Classes\*\shell\qing-zip\command'
folder_reg=r'Software\Classes\Directory\shell\qing-zip-compress\command'
archive=root/'Archive sample.zip'; folder=root/'Folder sample';folder.mkdir()
(folder/'source.txt').write_text('compression marker',encoding='utf8')
payload=b'modern context menu extraction marker'
with zipfile.ZipFile(archive,'w',zipfile.ZIP_STORED) as z:z.writestr('extracted-marker.txt',payload)
ui=IUIA().iuia
desktop=Desktop(backend="uia")
def shots(label):
    try: ImageGrab.grab(all_screens=True).save(results/(label+'.png'))
    except Exception as exc:report.setdefault('capture_errors',[]).append(str(exc))

def settings(on):
    subprocess.run([exe.parent/'obz.exe','shell-enable' if on else 'shell-disable',exe],
                   capture_output=True,check=True,timeout=25)
    profile=root.parent/('qing-cdp-'+str(on)+'-'+str(os.getpid()))
    env={**os.environ,'WEBVIEW2_USER_DATA_FOLDER':str(profile),
         'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS':'--remote-debugging-port=0 --remote-debugging-address=127.0.0.1'}
    app=subprocess.Popen([exe],env=env)
    try:
        def target():
            file=next(profile.rglob('DevToolsActivePort'),None)
            if not file:return
            try:
                port=int(file.read_text().splitlines()[0])
                return next((x for x in json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list',timeout=2)) if 'tauri.localhost' in x['url']),None)
            except (OSError,ValueError,IndexError):return
        tab=wait(target,35)
        with connect(tab['webSocketDebuggerUrl'],proxy=None) as ws:
            seq=0
            def js(code):
                nonlocal seq
                seq+=1;ws.send(json.dumps({'id':seq,'method':'Runtime.evaluate','params':{'expression':code,'awaitPromise':True,'returnByValue':True}}))
                while True:
                    msg=json.loads(ws.recv(timeout=8))
                    if msg.get('id')==seq:
                        val=msg['result'];assert 'exceptionDetails' not in val,val
                        return val['result'].get('value')
            wait(lambda: js("!!document.querySelector('[data-view=settings]')"))
            js("document.querySelector('[data-view=settings]').click()")
            wait(lambda: js("!!document.querySelector('#oa-folder-menu')"))
            js("if(document.querySelector('#oa-folder-menu').checked !== "+str(on).lower()+"){document.querySelector('#oa-folder-menu').click()}")
            wait(lambda: bool(command(folder_reg))==on)
            assert js("document.querySelector('#oa-folder-menu').checked")==on
    finally:
        if app.poll() is None: app.terminate()
        app.wait(timeout=15)
    state={"file":command(archive_reg),"directory":command(folder_reg)}
    assert bool(state['file'])==on and bool(state['directory'])==on
    if on:
        assert state['file'].casefold()==f'"{exe}" "%1"'.casefold()
        assert state['directory'].casefold()==f'"{exe}" --compress-folder "%1"'.casefold()
    report.setdefault('settings',{})[str(on)]=state;save()

def dismiss_runner_onboarding():
    # Hosted Windows 11 VMs sometimes display first-logon privacy over Explorer.
    # Scope the handling to the actual onboarding heading in this disposable session.
    condition=ui.CreatePropertyCondition(30005,"Choose privacy settings for your device")
    def shown():return bool(ui.GetRootElement().FindFirst(4,condition))
    if not shown():return
    shots("runner-onboarding")
    keyboard.send_keys("%{F4}")
    time.sleep(.6)
    if not shown():
        report["onboarding"]="dismissed with Alt+F4";return
    for attempt in range(8):
        if not shown():
            report["onboarding"]="finished disposable profile with optional toggles off";save();return
        foreground=ctypes.windll.user32.GetForegroundWindow()
        window=desktop.window(handle=foreground).wrapper_object()
        descendants=window.descendants()
        if not any(w.window_text()=="Choose privacy settings for your device" for w in descendants):
            raise RuntimeError("Refusing to act outside the identified hosted onboarding window")
        for w in descendants:
            try:
                toggle=w.iface_toggle
                if toggle.CurrentToggleState==1:toggle.Toggle()
            except Exception:pass
        action=next((w for w in descendants if w.window_text() in ("Next","Accept") and w.is_visible()),None)
        if not action:raise RuntimeError("Hosted privacy prompt has no known continuation")
        action.click_input();time.sleep(.65)
    if shown():raise RuntimeError("Hosted onboarding still covers Explorer")

def explorer_item(path):
    def selected():
        for window in desktop.windows(class_name='CabinetWClass'):
            if root.name not in window.window_text(): continue
            for item in window.descendants(control_type="ListItem"):
                if item.window_text() in (path.name,path.stem):
                    try:window.set_focus()
                    except Exception:pass
                    return item
    return wait(selected,24)
def menuitems():
    typ=ui.CreatePropertyCondition(30003,50011)
    array=ui.GetRootElement().FindAll(4,typ)
    items=[]
    for i in range(array.Length):
        try:
            e=array.GetElement(i); b=e.CurrentBoundingRectangle
            if not e.CurrentIsOffscreen and b.right>b.left and b.bottom>b.top:
                items.append((e.CurrentName,((b.left+b.right)//2,(b.top+b.bottom)//2)))
        except Exception:pass
    return items
def relevant(items):
    return [n for n,xy in items if 'qing' in n.lower() or '智能解压' in n or '解压到此处' in n]
def first_menu(path,label):
    keyboard.send_keys('{ESC}{ESC}')
    item=explorer_item(path);item.click_input(button='right')
    time.sleep(.65)
    items=menuitems(); names=[n for n,xy in items]
    report['menus'][label]=names;shots(label);save()
    assert any('more options' in n.lower() or '更多选项' in n for n in names),(label,names)
    return items
def act(items,label,target):
    def named(parts):return next((xy for n,xy in parts if target in n),None)
    where=named(items)
    if not where:
        group=next(((n,xy) for n,xy in items if 'qing' in n.lower()),None)
        assert group,(label,items)
        mouse.move(coords=group[1]);time.sleep(.8)
        items=menuitems()
        if not named(items):
            mouse.click(coords=group[1]);time.sleep(.65);items=menuitems()
        report['menus'][label+'-expanded']=[n for n,xy in items];shots(label+'-expanded');save()
        where=named(items)
    assert where,(label,items)
    mouse.click(coords=where)
def processes():
    cmd="Get-CimInstance Win32_Process -Filter \"Name='qing-zip.exe'\" | Select-Object ProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress"
    out=subprocess.check_output(['powershell','-NoProfile','-Command',cmd])
    if not out.strip():return []
    val=json.loads(out.decode('utf8',errors='replace'));return val if isinstance(val,list) else [val]
def stop_own():
    for p in processes():
        if os.path.normcase(p['ExecutablePath'] or '')==os.path.normcase(str(exe)):
            subprocess.run(['taskkill','/F','/PID',str(p['ProcessId'])],stdout=subprocess.DEVNULL)
try:
    assert not command(archive_reg) and not command(folder_reg)
    dismiss_runner_onboarding()
    os.startfile(root)
    # The off state must be checked against the same installed sparse package.
    for p,tag in [(archive,'file-off'),(folder,'folder-off')]:
        items=first_menu(p,tag);assert not relevant(items),(tag,items)
    keyboard.send_keys('{ESC}');settings(True)
    items=first_menu(archive,'file-on')
    act(items,'file-on','解压到此处')
    marker=root/'extracted-marker.txt';wait(lambda: marker.is_file() and marker.read_bytes()==payload,30)
    report['actions']['actual_archive_extraction']=True;save();stop_own()
    items=first_menu(folder,'folder-on')
    act(items,'folder-on','压缩')
    def genuine_folder_activation():
        return next((p for p in processes() if '--compress-folder' in (p['CommandLine'] or '') and str(folder) in p['CommandLine']),None)
    report['actions']['folder_activation']=wait(genuine_folder_activation,22)
    time.sleep(2);shots('compression-dialog');save();stop_own()
    settings(False)
    for p,tag in [(archive,'file-disabled-again'),(folder,'folder-disabled-again')]:
        items=first_menu(p,tag);assert not relevant(items),(tag,items)
    report['complete']=True
except Exception:
    report['error']=traceback.format_exc();shots('last-desktop');raise
finally:
    save()
