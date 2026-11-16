"""Actual WebView input and native archive output; isolated profile only."""
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes
from websockets.sync.client import connect
ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args();a.exe=a.exe.resolve();a.out=a.out.resolve();a.out.mkdir()
assert not subprocess.run(['powershell','-NoProfile','-Command','(Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue).Path'],capture_output=True).stdout.strip(),'Other GUI open'
root=Path(tempfile.mkdtemp(prefix='qing-timestamps-gui-'));src=root/'left';src.mkdir();(src/'existing.txt').write_bytes(b'keep');right=root/'right';right.mkdir();profile=root/'profile';local=root/'local';local.mkdir()
for i in range(6000):(src/f'p{i:04d}.txt').write_bytes(b'original')
(src/'A').mkdir();(src/'B').mkdir();ancestor=root/'rename-parent';(ancestor/'child').mkdir(parents=True);(ancestor/'child'/'proof.txt').write_bytes(b'kept')
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
manifest_file=a.exe.parent/'freeze-manifest.json';frozen=json.loads(manifest_file.read_text(encoding='utf-8'))
binaries=frozen['binaries'];declared=dict(binaries) if isinstance(binaries,dict) else {x['name']:x['sha256'] for x in binaries}
declared.update(frozen.get('runtime_inputs',{}))
def dependencies():
 values={str(manifest_file):sha(manifest_file),str(Path(__file__).resolve()):sha(Path(__file__))}
 for name,digest in declared.items():
  path=(a.exe.parent/name).resolve(strict=True);assert path.is_relative_to(a.exe.parent)
  actual=sha(path);assert actual==digest,(name,'frozen byte mismatch');values[str(path)]=actual
 return values
bound=dependencies()
report={'gui':str(a.exe),'sha256':hashlib.sha256(a.exe.read_bytes()).hexdigest(),'fixture':str(root),'dependency_sha256':bound,'source_commit':frozen.get('source_commit',frozen.get('commit')),'checks':[],'passed':False}
def save():(a.out/'receipt.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
def checked(name):report['checks'].append(name);save()
class Browser:
 def __init__(self):
  self.proc=None;self.ws=None;self.seq=0
  env=dict(os.environ,LOCALAPPDATA=str(local),WEBVIEW2_USER_DATA_FOLDER=str(profile),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=0 --remote-debugging-address=127.0.0.1')
  self.proc=subprocess.Popen([str(a.exe)],env=env,cwd=root)
  deadline=time.monotonic()+45
  while time.monotonic()<deadline:
   assert self.proc.poll() is None
   pf=next(profile.rglob('DevToolsActivePort'),None)
   if pf:
    try:
     port=int(pf.read_text().splitlines()[0]);tabs=json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json',timeout=1));tab=next(t for t in tabs if t.get('type')=='page');self.ws=connect(tab['webSocketDebuggerUrl'],max_size=16*1024*1024);break
    except Exception:pass
   time.sleep(.15)
  assert self.ws is not None
  self.wait('!!window.__TAURI__?.core && !!document.querySelector("[data-action=compress]")');time.sleep(.5)
 def call(self,method,params={}):
  self.seq+=1;self.ws.send(json.dumps({'id':self.seq,'method':method,'params':params}))
  while True:
   result=json.loads(self.ws.recv())
   if result.get('id')==self.seq:
    assert 'error' not in result,result;return result['result']
 def js(self,code):
  r=self.call('Runtime.evaluate',{'expression':code,'returnByValue':True});assert 'exceptionDetails' not in r,r;return r['result'].get('value')
 def wait(self,code):
  deadline=time.monotonic()+40
  while time.monotonic()<deadline:
   if self.js(code):return
   time.sleep(.12)
  raise AssertionError(code+'\n'+str(self.js('document.body.innerText.slice(-2500)')))
 def click(self,selector):
  q=json.dumps(selector);self.wait('!!document.querySelector('+q+')');self.js('document.querySelector('+q+').scrollIntoView({block:"center"})');time.sleep(.1)
  x,y=self.js('(()=>{let r=document.querySelector('+q+').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]})()')
  for mode in ['mousePressed','mouseReleased']:self.call('Input.dispatchMouseEvent',{'type':mode,'x':x,'y':y,'button':'left','buttons':int(mode=='mousePressed'),'clickCount':1})
 def key(self,key,code,vk,mod=0):
  for mode in ['keyDown' if key=='Enter' else 'rawKeyDown','keyUp']:self.call('Input.dispatchKeyEvent',{'type':mode,'key':key,'code':code,'windowsVirtualKeyCode':vk,'modifiers':mod,**({'text':'\r','unmodifiedText':'\r'} if key=='Enter' and mode=='keyDown' else {})})
 def fill(self,selector,value):
  self.click(selector);self.key('a','KeyA',65,2);self.call('Input.insertText',{'text':str(value)})
 def select(self,selector,value):
  options=self.js('Array.from(document.querySelector('+json.dumps(selector)+').options).map(o=>o.value)');index=options.index(value)
  self.click(selector);self.key('Home','Home',36)
  for _ in range(index):self.key('ArrowDown','ArrowDown',40)
  self.key('Enter','Enter',13);self.wait('document.querySelector('+json.dumps(selector)+').value==='+json.dumps(value))
 def screenshot(self,name):(a.out/(name+'.png')).write_bytes(base64.b64decode(self.call('Page.captureScreenshot',{'format':'png'})['data']))
 def dialog(self):self.click('[data-action=compress]');self.wait('!!document.querySelector("#qa-inputs")')
 def close_dialog(self):self.click('.qa-dialog-header [data-action=close]');self.wait('!document.querySelector("#qa-inputs")')
 def close(self):
  if self.ws:
   try:self.js('window.__TAURI__.core.invoke("window_action",{action:"close"})')
   except Exception:pass
   self.ws.close()
  if self.proc:
   try:self.proc.wait(timeout=12)
   except subprocess.TimeoutExpired:self.proc.terminate();self.proc.wait(timeout=10)
b=None
try:
 b=Browser.__new__(Browser);Browser.__init__(b);b.call('Page.enable')
 b.call('Page.addScriptToEvaluateOnNewDocument',{'source':"""Object.defineProperty(window,'QingFileWorkspace',{configurable:true,set(api){const original=api.create;api.create=function(ui){const m=window.__metrics={probes:[],browses:0,inflight:0,maxInflight:0,maxNames:0};const probe=ui.probe,invoke=ui.invoke;ui.probe=async function(path,names){m.inflight++;m.maxInflight=Math.max(m.maxInflight,m.inflight);m.maxNames=Math.max(m.maxNames,names.length);const log={path,names:names.slice(),start:performance.now()};m.probes.push(log);try{return await probe(path,names);}finally{log.ms=performance.now()-log.start;m.inflight--;}};ui.invoke=async function(name,args){if(name==='browse_local')m.browses++;return invoke(name,args);};window.__metricReady=true;return original(ui);};Object.defineProperty(window,'QingFileWorkspace',{value:api,writable:true,configurable:true});}});"""})
 b.call('Page.reload');b.wait('window.__metricReady===true && !!document.querySelector("[data-action=compress]")');time.sleep(.5)
 b.click('[data-view="tools"]');b.click('[data-view="filemanager"]')
 def goto(index,path):
  b.fill(f'[data-fm-path="{index}"] input',str(path));b.click(f'[data-fm="go"][data-pane="{index}"]');b.wait('!document.querySelector('+json.dumps(f'[data-fm-path="{index}"] input')+').disabled');b.wait('document.querySelector('+json.dumps(f'[data-fm-path="{index}"] input')+').value==='+json.dumps(str(path)))
 def has(index,name):return 'Array.from(document.querySelectorAll('+json.dumps(f'[data-fm-pane="{index}"] .fm-row button span')+')).some(e=>e.textContent==='+json.dumps(name)+')'
 def metrics():return b.js('window.__metrics')
 goto(0,src);goto(1,right);b.wait('document.hasFocus()');time.sleep(2.3)
 assert b.js('document.querySelectorAll(`[data-fm-pane="0"] .fm-row`).length')==200
 from ctypes import wintypes as W
 K=ctypes.WinDLL('kernel32',use_last_error=True)
 class ProcessEntry(ctypes.Structure):_fields_=[('size',W.DWORD),('usage',W.DWORD),('pid',W.DWORD),('heap',ctypes.c_size_t),('module',W.DWORD),('threads',W.DWORD),('parent',W.DWORD),('priority',W.LONG),('flags',W.DWORD),('exe',W.WCHAR*260)]
 K.CreateToolhelp32Snapshot.argtypes=[W.DWORD,W.DWORD];K.CreateToolhelp32Snapshot.restype=W.HANDLE;K.Process32FirstW.argtypes=[W.HANDLE,ctypes.POINTER(ProcessEntry)];K.Process32NextW.argtypes=K.Process32FirstW.argtypes
 K.OpenProcess.argtypes=[W.DWORD,W.BOOL,W.DWORD];K.OpenProcess.restype=W.HANDLE;K.CloseHandle.argtypes=[W.HANDLE];K.GetProcessTimes.argtypes=[W.HANDLE]+[ctypes.POINTER(W.FILETIME)]*4
 def cpu():
  snap=K.CreateToolhelp32Snapshot(2,0);assert snap!=ctypes.c_void_p(-1).value;item=ProcessEntry();item.size=ctypes.sizeof(item);parents={}
  try:
   valid=K.Process32FirstW(snap,ctypes.byref(item))
   while valid:parents[item.pid]=item.parent;valid=K.Process32NextW(snap,ctypes.byref(item))
  finally:K.CloseHandle(snap)
  owned={b.proc.pid}
  while True:
   more={pid for pid,parent in parents.items() if parent in owned};new=owned|more
   if new==owned:break
   owned=new
  out={};number=lambda t:(t.dwHighDateTime<<32)|t.dwLowDateTime
  for pid in owned:
   handle=K.OpenProcess(0x1000,False,pid)
   if not handle:continue
   try:
    born,exit,kernel,user=(W.FILETIME() for _ in range(4))
    if K.GetProcessTimes(handle,ctypes.byref(born),ctypes.byref(exit),ctypes.byref(kernel),ctypes.byref(user)):out[(pid,number(born))]=(number(kernel)+number(user))/1e7
   finally:K.CloseHandle(handle)
  return out
 start=metrics();start_cpu=cpu();start_wall=time.monotonic();time.sleep(12.2);end=metrics();end_cpu=cpu();wall=time.monotonic()-start_wall
 assert b.js('document.hasFocus()');assert end['browses']==start['browses'],(start['browses'],end['browses']);assert 8<=len(end['probes'])-len(start['probes'])<=14
 report['idle_6000_entries']={'wall_seconds':wall,'metadata_calls':len(end['probes'])-len(start['probes']),'full_browse_calls':end['browses']-start['browses'],'max_names':end['maxNames'],'max_global_inflight':end['maxInflight'],'gui_live_tree_cpu_seconds':sum(end_cpu.get(pid,value)-value for pid,value in start_cpu.items()),'process_set_unchanged':set(start_cpu)==set(end_cpu),'cpu_scope':'GUI and live descendants present in both snapshots; test process excluded; lower bound if process set changes','metadata_ms':[x['ms'] for x in end['probes'][len(start['probes']):] if 'ms' in x]};checked('idle-no-directory-rescan-bounded-metadata')
 n=b.js('Array.from(document.querySelectorAll(`[data-fm-pane="0"] [data-fm="entry"]`)).find(e=>e.textContent.includes("p0000.txt")).dataset.index');b.click(f'[data-fm="entry"][data-pane="0"][data-index="{n}"]')
 x,y=b.js('(()=>{const r=document.querySelector(`[data-fm-pane="0"] .fm-list`).getBoundingClientRect();return [r.x+50,Math.min(innerHeight-40,r.y+80)]})()');b.call('Input.dispatchMouseEvent',{'type':'mouseWheel','x':x,'y':y,'deltaX':0,'deltaY':250});time.sleep(.2)
 scroll=b.js('document.querySelector(`[data-fm-pane="0"] .fm-list`).scrollTop');(src/'p0000.txt').write_bytes(b'grown-content-123456789')
 b.wait('Array.from(document.querySelectorAll(`[data-fm-pane="0"] .fm-row`)).some(e=>e.textContent.includes("p0000.txt")&&e.textContent.includes("23 字节"))')
 assert b.js('Array.from(document.querySelectorAll(`[data-fm-pane="0"] .fm-row.selected`)).some(e=>e.textContent.includes("p0000.txt"))');assert abs(b.js('document.querySelector(`[data-fm-pane="0"] .fm-list`).scrollTop')-scroll)<2;checked('growth-refresh-retains-selection-and-scroll')
 added=src/'000-added.txt';added.write_bytes(b'new');b.wait(has(0,added.name));renamed=src/'000-renamed.txt';added.rename(renamed);b.wait(has(0,renamed.name));renamed.unlink();b.wait('!('+has(0,renamed.name)+')');checked('external-create-rename-delete')
 mark=b.js('performance.now()');b.click('[data-fm="next"][data-pane="0"]');b.wait('window.__metrics.probes.some(p=>p.start>='+str(mark)+'&&p.names.length===200&&!p.names.includes("p0000.txt"))');checked('page-change-probes-new-page')
 goto(0,src/'A');goto(0,src/'B');b.click('[data-fm="back"][data-pane="0"]');b.wait('document.querySelector(`[data-fm-path="0"] input`).value==='+json.dumps(str(src/'A')));b.click('[data-fm="forward"][data-pane="0"]');b.wait('document.querySelector(`[data-fm-path="0"] input`).value==='+json.dumps(str(src/'B')))
 value=b.js('Array.from(document.querySelector(`[data-fm-history="0"]`).options).find(e=>e.textContent==='+json.dumps(str(src))+').value');b.select('[data-fm-history="0"]',value);b.wait('document.querySelector(`[data-fm-path="0"] input`).value==='+json.dumps(str(src)));checked('back-forward-history-select')
 b.click('[data-view="tools"]');time.sleep(.3);stopped=len(metrics()['probes']);time.sleep(4.2);assert len(metrics()['probes'])==stopped;checked('leaving-page-stops-probes');b.click('[data-view="filemanager"]');b.wait('window.__metrics.probes.length>'+str(stopped))
 b.wait('window.__metrics.inflight===0');newsrc=src.with_name('left-renamed');src.rename(newsrc);b.wait('document.querySelector(`[data-fm-pane="0"]`).textContent.includes("无法读取此文件夹")');goto(0,newsrc);checked('watched-root-rename-is-not-blocked-and-old-path-errors')
 goto(1,ancestor/'child');b.wait('window.__metrics.inflight===0');moved=ancestor.with_name('renamed-parent');ancestor.rename(moved);b.wait('document.querySelector(`[data-fm-pane="1"]`).textContent.includes("无法读取此文件夹")');goto(1,moved/'child');assert (moved/'child'/'proof.txt').read_bytes()==b'kept';checked('ancestor-rename-is-not-blocked')
 path_values=b.js('Array.from(document.querySelectorAll("[data-fm-path] input")).map(e=>e.value)')
 b.click('[data-fm-auto="1"]');assert b.js('Array.from(document.querySelectorAll("[data-fm-path] input")).map(e=>e.value)')==path_values;newfile=moved/'child'/'manual.txt';newfile.write_bytes(b'manual');time.sleep(2.3);assert not b.js(has(1,newfile.name));b.click('[data-fm="refresh"][data-pane="1"]');b.wait(has(1,newfile.name));b.click('[data-fm-auto="1"]');checked('auto-off-manual-refresh-and-resume')
 assert b.js('Array.from(document.querySelectorAll("[data-fm-path] input")).map(e=>e.value)')==path_values
 b.click('[data-fm="refresh"][data-pane="1"]');time.sleep(.3);before_probe=len(metrics()['probes']);time.sleep(2.2);assert len(metrics()['probes'])>before_probe;checked('unchanged-manual-refresh-keeps-polling-and-control-values')
 # Inspect only this live GUI process; the largest client area excludes its
 # small hidden/auxiliary windows. Restore no unrelated application.
 U=ctypes.WinDLL('user32');from ctypes import wintypes as W
 U.GetWindowThreadProcessId.argtypes=[W.HWND,ctypes.POINTER(W.DWORD)];U.GetClientRect.argtypes=[W.HWND,ctypes.POINTER(W.RECT)];U.ShowWindow.argtypes=[W.HWND,ctypes.c_int];U.SetForegroundWindow.argtypes=[W.HWND]
 found=[];CB=ctypes.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM)
 @CB
 def own(hwnd,_):
  pid=W.DWORD();U.GetWindowThreadProcessId(hwnd,ctypes.byref(pid));rect=W.RECT()
  if pid.value==b.proc.pid and U.GetClientRect(hwnd,ctypes.byref(rect)):found.append(((rect.right-rect.left)*(rect.bottom-rect.top),hwnd))
  return True
 U.EnumWindows(own,0);hwnd=max(found)[1];b.click('[data-window="minimize"]');b.wait('!document.hasFocus()');time.sleep(.3);stopped=len(metrics()['probes']);time.sleep(4.2);assert len(metrics()['probes'])==stopped
 U.ShowWindow(hwnd,9);U.SetForegroundWindow(hwnd);b.wait('document.hasFocus()');b.wait('window.__metrics.probes.length>'+str(stopped));checked('background-stops-and-foreground-resumes')
 b.screenshot('local-navigation');report['final_metrics']=metrics();assert dependencies()==bound;report['passed']=True;save()
except Exception as error:
 report['error']=str(error)
 if b and b.ws:
  try:b.screenshot('failure')
  except Exception:pass
 save();raise
finally:
 if b:b.close()
 report['process_terminal']=b is None or b.proc is None or b.proc.poll() is not None;save()
