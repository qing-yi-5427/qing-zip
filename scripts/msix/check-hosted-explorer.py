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
    port=9227 if on else 9228
    env={**os.environ,'WEBVIEW2_USER_DATA_FOLDER':str(profile),
         'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS':f'--remote-debugging-port={port} --remote-debugging-address=127.0.0.1'}
    app=subprocess.Popen([exe,archive],env=env)
    try:
        def target():
            if app.poll() is not None:
                raise RuntimeError(f'GUI exited before settings {app.returncode}; existing={processes()}')
            try:
                tabs=json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list',timeout=2))
                report['cdp_pages']=[x.get('url') for x in tabs]
                return next((x for x in tabs if x.get('type')=='page' and ('tauri.localhost' in x['url'] or 'tauri://' in x['url'])),None)
            except (OSError,ValueError,IndexError):return
        tab=wait(target,5)
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
    except Exception:
        report.setdefault('gui_setting_error',{})[str(on)] = traceback.format_exc()
        report.setdefault('gui_setting_windows',{})[str(on)] = [(w.element_info.process_id,w.class_name(),w.window_text()) for w in desktop.windows()][:40]
        shots("settings-diagnostic-"+str(on));save()
        try:
            clear_lab_overlays()
            main=desktop.window(handle=next(w.handle for w in desktop.windows(process=app.pid) if w.class_name()=='Tauri Window')).wrapper_object()
            def named(text):
                return next((w for w in main.descendants() if text in w.window_text()),None)
            tab=wait(lambda:named("设置"),12)
            try:tab.iface_invoke.Invoke()
            except Exception:tab.click_input()
            checkbox=wait(lambda:named("文件夹右键显示"),12)
            if bool(command(folder_reg))!=on:
                try:checkbox.iface_toggle.Toggle()
                except Exception:checkbox.click_input()
            wait(lambda:bool(command(folder_reg))==on)
            report.setdefault('folder_setting_method',{})[str(on)]='actual qing-zip settings via Windows accessibility'
        except Exception:
            report.setdefault('folder_ui_error',{})[str(on)]=traceback.format_exc()
            if on:
                with winreg.CreateKey(winreg.HKEY_CURRENT_USER,folder_reg) as h:
                    winreg.SetValueEx(h,None,0,winreg.REG_SZ,f'"{exe}" --compress-folder "%1"')
            elif command(folder_reg)==f'"{exe}" --compress-folder "%1"':
                winreg.DeleteKey(winreg.HKEY_CURRENT_USER,folder_reg)
            report.setdefault('folder_setting_method',{})[str(on)]='manual owned-command fixture; folder settings UI unverified'
    finally:
        if app.poll() is None: app.terminate()
        app.wait(timeout=15)
    report.setdefault('folder_setting_method',{}).setdefault(str(on),'actual qing-zip settings switch')
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
        if action:
            action.click_input()
        else:
            # Win11 OOBE's blue Next/Accept control has no accessible name on
            # some builds. Verify the identified foreground prompt and its
            # visible button before clicking this same observed control.
            screen=ImageGrab.grab()
            w,h=screen.size
            x,y=round(.895*w),round(.856*h)
            r,g,b=screen.convert('RGB').getpixel((x,y))
            if not (r<70 and 35<g<175 and b>120 and b>g+45):
                raise RuntimeError("Identified onboarding window has no blue continuation button")
            mouse.click(coords=(x,y))
        time.sleep(1.1)
    if shown():raise RuntimeError("Hosted onboarding still covers Explorer")

def clear_lab_overlays():
    # The hosted ARM image automatically opens Start and an unrelated wsl.exe
    # upgrade prompt after first logon. Close only these disposable-session processes.
    script = """
    $session=(Get-Process -Id $PID).SessionId
    Get-Process -Name StartMenuExperienceHost,wsl -ErrorAction SilentlyContinue |
      Where-Object { $_.SessionId -eq $session -and ($_.Path -like "$env:SystemRoot\SystemApps\*\StartMenuExperienceHost.exe" -or $_.Path -eq "$env:SystemRoot\System32\wsl.exe") } |
      ForEach-Object { $_.ProcessName; Stop-Process -Id $_.Id -Force }
    """
    out=subprocess.run(['powershell','-NoProfile','-Command',script],capture_output=True)
    report.setdefault('session_overlays_closed',[]).append(out.stdout.decode(errors='replace').splitlines())
    for window in desktop.windows():
        if window.window_text().lower().endswith(r"\system32\wsl.exe") and window.class_name() in ("ConsoleWindowClass","CASCADIA_HOSTING_WINDOW_CLASS"):
            ctypes.windll.user32.PostMessageW(window.handle,0x10,0,0)
    time.sleep(.4)

def focus_item(window,item):
    from ctypes import wintypes as w
    user=ctypes.windll.user32;kernel=ctypes.windll.kernel32
    hwnd=window.handle
    user.WindowFromPoint.argtypes=[w.POINT]
    user.WindowFromPoint.restype=w.HWND
    user.GetAncestor.argtypes=[w.HWND,w.UINT];user.GetAncestor.restype=w.HWND
    rect=item.rectangle()
    pos=w.POINT(rect.left+min(40,max(4,(rect.right-rect.left)//4)),(rect.top+rect.bottom)//2)
    # Clear an already open Explorer flyout by clicking its own tab header.
    # Escape is not consistently routed to this shell popup in hosted sessions.
    frame=window.rectangle()
    ctypes.windll.user32.PostMessageW(hwnd,0x001f,0,0)
    mouse.click(coords=(frame.left+110,frame.top+17))
    time.sleep(.12)
    def visible():
        hit=user.WindowFromPoint(w.POINT(pos.x,pos.y))
        return bool(hit and user.GetAncestor(hit,2)==hwnd)
    for attempt in range(6):
        if visible() and user.GetForegroundWindow()==hwnd:return
        if attempt in (0,3):clear_lab_overlays()
        keyboard.send_keys('{ESC}{ESC}')
        fg=user.GetForegroundWindow();foreign=user.GetWindowThreadProcessId(fg,None);local=kernel.GetCurrentThreadId()
        attached=foreign!=local and user.AttachThreadInput(local,foreign,True)
        try:
            user.ShowWindow(hwnd,9);user.SetWindowPos(hwnd,-1,0,0,0,0,3)
            user.BringWindowToTop(hwnd);user.SetForegroundWindow(hwnd)
        finally:
            if attached:user.AttachThreadInput(local,foreign,False)
        time.sleep(.25)
        if visible():
            user.SetWindowPos(hwnd,-2,0,0,0,0,3);time.sleep(.15)
            if visible():return
    raise RuntimeError('Explorer item remains occluded at its actual screen position')
def explorer_item(path):
    def selected():
        for window in desktop.windows(class_name='CabinetWClass'):
            if root.name not in window.window_text(): continue
            for item in window.descendants(control_type="ListItem"):
                if item.window_text() in (path.name,path.stem):
                    focus_item(window,item)
                    return item
    return wait(selected,24)
def menuitems():
    foreground=desktop.window(handle=ctypes.windll.user32.GetForegroundWindow()).wrapper_object()
    items=[]
    for w in foreground.descendants():
        if w.element_info.control_type!='MenuItem':continue
        name=w.window_text()
        xy=None
        try:
            box=w.rectangle()
            if box.right>box.left and box.bottom>box.top:xy=((box.left+box.right)//2,(box.top+box.bottom)//2)
        except Exception as exc:report.setdefault('geometry_errors',{})[name]=str(exc)
        items.append((name,xy))
    return items
def relevant(items):
    return [n for n,xy in items if 'qing' in n.lower() or '智能解压' in n or '解压到此处' in n]
def first_menu(path,label):
    keyboard.send_keys('{ESC}{ESC}')
    item=explorer_item(path)
    rect=item.rectangle()
    mouse.click(button='right',coords=(rect.left+min(40,max(4,(rect.right-rect.left)//4)),(rect.top+rect.bottom)//2))
    def is_modern(items):
        return any("more options" in n.lower() or "\u66f4\u591a\u9009\u9879" in n for n,_ in items)
    until=time.monotonic()+8
    while True:
        items=menuitems()
        if is_modern(items) or time.monotonic()>=until:break
        time.sleep(.35)
    names=[n for n,xy in items]
    report['menus'][label]=names;shots(label);save()
    if not is_modern(items):
        fg=desktop.window(handle=ctypes.windll.user32.GetForegroundWindow()).wrapper_object()
        report.setdefault('incomplete_menu_snapshot',{})[label]={'class':fg.class_name(),'title':fg.window_text(),'elements':[(w.element_info.control_type,w.window_text()) for w in fg.descendants()][:140]};save()
    return items
def act(items,label,target):
    def find(parts):return next(((n,xy) for n,xy in parts if target in n),None)
    def trigger(name,xy):
        if xy:
            mouse.click(coords=xy);return
        win=desktop.window(handle=ctypes.windll.user32.GetForegroundWindow()).wrapper_object()
        item=next(w for w in win.descendants() if w.element_info.control_type=="MenuItem" and w.window_text()==name)
        item.iface_invoke.Invoke()
    entry=find(items)
    if not entry:
        group=next(((n,xy) for n,xy in items if 'qing' in n.lower()),None)
        assert group,(label,items)
        def wait_child(seconds):
            end=time.monotonic()+seconds
            current=[]
            while time.monotonic()<end:
                current=menuitems()
                if find(current):return current
                time.sleep(.3)
            return current
        if group[1]:mouse.move(coords=group[1])
        items=wait_child(5)
        if not find(items):
            # Do not click the row again during XAML's asynchronous expansion:
            # that can close exactly the popup whose children are appearing.
            win=desktop.window(handle=ctypes.windll.user32.GetForegroundWindow()).wrapper_object()
            node=next((w for w in win.descendants() if w.element_info.control_type=="MenuItem" and w.window_text()==group[0]),None)
            try:node.iface_expand_collapse.Expand()
            except Exception:
                if node:
                    try:node.set_focus()
                    except Exception:pass
                keyboard.send_keys('{RIGHT}')
            items=wait_child(5)
        if not find(items):
            trigger(*group);items=wait_child(4)
        report['menus'][label+'-expanded']=[n for n,xy in items];shots(label+'-expanded');save()
        entry=find(items)
    assert entry,(label,items)
    trigger(*entry)
def processes():
    cmd="Get-CimInstance Win32_Process -Filter \"Name='qing-zip.exe'\" | Select-Object ProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress"
    out=subprocess.check_output(['powershell','-NoProfile','-Command',cmd])
    if not out.strip():return []
    val=json.loads(out.decode('utf8',errors='replace'));return val if isinstance(val,list) else [val]
def stop_own():
    for p in processes():
        if os.path.normcase(p['ExecutablePath'] or '')==os.path.normcase(str(exe)):
            subprocess.run(['taskkill','/F','/PID',str(p['ProcessId'])],stdout=subprocess.DEVNULL)
def registered_commands(enabled, exercise=False):
    # This packaged COM probe is separately labelled; it is never a substitute
    # for the Explorer menu clicks and screenshots.
    import uuid
    ole=ctypes.OleDLL('ole32')
    shell=ctypes.OleDLL('shell32')
    P=ctypes.c_void_p
    def guid(text):return (ctypes.c_ubyte*16).from_buffer_copy(uuid.UUID(text).bytes_le)
    interface=guid('A08CE4D0-FA25-44AB-B57C-C7B1C323E0B9')
    def method(ptr,idx,types,*args):
        slots=ctypes.cast(ptr,ctypes.POINTER(ctypes.POINTER(P))).contents
        f=ctypes.WINFUNCTYPE(ctypes.c_long,P,*types)(slots[idx])
        hr=f(ptr,*args)
        if hr<0:raise OSError(hex(hr&0xffffffff))
        return hr
    result={}
    probe_archive=root/'Registered COM sample.zip'
    if exercise:
        with zipfile.ZipFile(probe_archive,'w') as z:z.writestr('registered-marker.txt',payload)
    for key,cls,path in [('open','9A406E20-1F79-49F3-94F9-179D18761D4A',archive),
                          ('compress','A65CB9C7-E692-4D0B-A5A1-4F19530EE312',folder),
                          ('smart','8148C339-AF9A-4E1E-8BE9-469F28DA1B75',archive),
                          ('here','0C0C73E7-2D76-4823-AB70-F9BE332B1360',archive)]:
        if exercise and key=='here':path=probe_archive
        obj=P();pidl=P();selection=P();title=P()
        try:
            ole.CoCreateInstance(ctypes.byref(guid(cls)),None,5,ctypes.byref(interface),ctypes.byref(obj))
            shell.SHParseDisplayName(ctypes.c_wchar_p(str(path)),None,ctypes.byref(pidl),0,None)
            shell.SHCreateShellItemArrayFromIDLists(1,ctypes.byref(pidl),ctypes.byref(selection))
            method(obj,3,[P,ctypes.POINTER(P)],selection,ctypes.byref(title))
            state=ctypes.c_uint()
            method(obj,7,[P,ctypes.c_int,ctypes.POINTER(ctypes.c_uint)],selection,1,ctypes.byref(state))
            result[key]={'title':ctypes.wstring_at(title),'state':state.value}
            assert state.value==(0 if enabled else 2),(key,state.value)
            if exercise and key=='here':
                method(obj,8,[P,P],selection,None)
                result[key]['invoked']=True
                marker=root/'registered-marker.txt'
                try:
                    wait(lambda:marker.is_file() and marker.read_bytes()==payload,25)
                    result[key]['extracted_expected_bytes']=True
                except RuntimeError:
                    result[key]['extracted_expected_bytes']=False
                    result[key]['processes']=processes()
                    shots('registered-action')

        finally:
            if title:ole.CoTaskMemFree(title)
            if selection:method(selection,2,[])
            if pidl:ole.CoTaskMemFree(pidl)
            if obj:method(obj,2,[])
    return result

try:
    assert not command(archive_reg) and not command(folder_reg)
    report["registered_off"]=registered_commands(False);save()
    dismiss_runner_onboarding()
    keyboard.send_keys('{ESC}{ESC}');time.sleep(1)
    clear_lab_overlays()
    os.startfile(root)
    # The off state must be checked against the same installed sparse package.
    for p,tag in [(archive,'file-off'),(folder,'folder-off')]:
        try:
            items=first_menu(p,tag);assert not relevant(items),(tag,items)
        except Exception:report.setdefault('separate_off_probe_errors',{})[tag]=traceback.format_exc();save()
    keyboard.send_keys('{ESC}');settings(True)
    report['registered_on']=registered_commands(True);save()
    clear_lab_overlays()
    try:
        items=first_menu(archive,'file-on')
        if not relevant(items):
            # Microsoft documents restarting Explorer to refresh a newly registered
            # shell extension. Preserve the first observation separately.
            script='$s=(Get-Process -Id $PID).SessionId; Get-Process -Name explorer | Where-Object { $_.SessionId -eq $s -and $_.Path -eq "$env:SystemRoot\explorer.exe" } | Stop-Process -Force'
            subprocess.run(['powershell','-NoProfile','-Command',script],capture_output=True)
            time.sleep(1.5);os.startfile(root);time.sleep(1)
            items=first_menu(archive,'file-on-after-shell-restart')
        act(items,'file-on','\u89e3\u538b\u5230\u6b64\u5904')
        marker=root/'extracted-marker.txt';wait(lambda: marker.is_file() and marker.read_bytes()==payload,30)
        report['actions']['actual_archive_extraction']=True
    except Exception:
        report['actions']['extract_error']=traceback.format_exc()
    finally:
        save();keyboard.send_keys('{ESC}{ESC}');stop_own()
    for category in ("smart","open"):
        try:
            testfile=archive
            if category=="smart":
                testfile=root/"Smart sample.zip"
                with zipfile.ZipFile(testfile,"w") as z:
                    z.writestr("alpha.txt","alpha")
                    z.writestr("beta.txt","beta")
            items=first_menu(testfile,category+"-on")
            act(items,category+"-on","\u667a\u80fd\u89e3\u538b" if category=="smart" else "\u7528 qing-zip \u6253\u5f00")
            if category=="smart":
                target=root/"Smart sample"
                wait(lambda:(target/"beta.txt").exists(),35)
                assert (target/"alpha.txt").read_bytes()==b"alpha" and (target/"beta.txt").read_bytes()==b"beta"
                assert not (root/"alpha.txt").exists() and not (root/"beta.txt").exists()
                report['actions']['actual_smart_extraction']=True
            else:
                def displayed_entry():
                    for win in desktop.windows():
                        if win.class_name()=='Tauri Window' and any(w.window_text()=='extracted-marker.txt' for w in win.descendants()):
                            return True
                wait(displayed_entry,22)
                report['actions']['actual_archive_open']=True;shots("opened-archive")
        except Exception:report['actions'][category+"_error"]=traceback.format_exc()
        finally:
            save();keyboard.send_keys('{ESC}{ESC}');stop_own()
    try:
        items=first_menu(folder,'folder-on')
        act(items,'folder-on','\u538b\u7f29')
        def genuine_folder_activation():
            for p in processes():
                cmd=p['CommandLine'] or ''
                if '--compress-folder' in cmd:
                    selected=cmd.split('--compress-folder',1)[1].strip().strip('"')
                    if os.path.exists(selected) and os.path.samefile(selected,folder):return p
            return None
        report['actions']['folder_activation']=wait(genuine_folder_activation,22)
        time.sleep(2);shots('compression-dialog')
        def create_button():
            for win in desktop.windows():
                if win.class_name()!='Tauri Window':continue
                for w in win.descendants():
                    if w.window_text()=='\u521b\u5efa\u538b\u7f29\u6587\u4ef6':return w
        button=wait(create_button,15)
        try:button.iface_invoke.Invoke()
        except Exception:button.click_input()
        output=root/(folder.name+'.zip')
        def correct_archive():
            if not output.is_file():return False
            try:
                with zipfile.ZipFile(output) as z:
                    return z.testzip() is None and z.read('Folder sample/source.txt')==b'compression marker'
            except (OSError,KeyError,zipfile.BadZipFile):return False
        wait(correct_archive,35)
        report['actions']['actual_folder_zip']={'path':str(output),'member':'Folder sample/source.txt','crc_ok':True}
        shots('completed-compression')
    except Exception:
        report['actions']['folder_error']=traceback.format_exc()
    finally:
        save();keyboard.send_keys('{ESC}{ESC}');stop_own()
    settings(False)
    for p,tag in [(archive,'file-disabled-again'),(folder,'folder-disabled-again')]:
        try:
            items=first_menu(p,tag);assert not relevant(items),(tag,items)
        except Exception:report.setdefault('separate_off_probe_errors',{})[tag]=traceback.format_exc();save()
    assert report['actions'].get('actual_archive_extraction') and all(report['actions'].get(k) for k in ('actual_folder_zip','actual_smart_extraction','actual_archive_open')),report['actions']
    report['complete']=True
except Exception:
    report['error']=traceback.format_exc();shots('last-desktop')
    try:
        stop_own();settings(True);report['registered_on']=registered_commands(True,exercise=True);save();stop_own()
        settings(False);report['registered_off_again']=registered_commands(False)
    except Exception:report['independent_probe_error']=traceback.format_exc()
    raise
finally:
    save()
