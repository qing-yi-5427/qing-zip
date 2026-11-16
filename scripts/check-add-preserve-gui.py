"""Actual WebView input and native archive output; isolated profile only."""
import zipfile
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes
from websockets.sync.client import connect
ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args();a.exe=a.exe.resolve();a.out=a.out.resolve();a.out.mkdir()
assert not subprocess.run(['powershell','-NoProfile','-Command','(Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue).Path'],capture_output=True).stdout.strip(),'Other GUI open'
root=Path(tempfile.mkdtemp(prefix='qing-timestamps-gui-'));src=root/'left';src.mkdir();(src/'existing.txt').write_bytes(b'keep');right=root/'right';right.mkdir();profile=root/'profile';local=root/'local';local.mkdir()
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
(src/'data.txt').write_bytes(b'timestamp GUI proof')
archive=root/"target.zip"
with zipfile.ZipFile(archive,"w") as z:z.writestr("base.txt",b"base")
bound=dependencies()
report={'gui':str(a.exe),'sha256':hashlib.sha256(a.exe.read_bytes()).hexdigest(),'fixture':str(root),'dependency_sha256':bound,'source_commit':frozen.get('source_commit',frozen.get('commit')),'checks':[],'passed':False}
def save():(a.out/'receipt.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
def checked(name):report['checks'].append(name);save()
class Browser:
 def __init__(self):
  self.proc=None;self.ws=None;self.seq=0
  env=dict(os.environ,LOCALAPPDATA=str(local),WEBVIEW2_USER_DATA_FOLDER=str(profile),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=0 --remote-debugging-address=127.0.0.1')
  self.proc=subprocess.Popen([str(a.exe),str(archive)],env=env,cwd=root)
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
  self.key('Enter','Enter',13);assert self.js('document.querySelector('+json.dumps(selector)+').value')==value
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
 b=Browser();b.wait('!document.querySelector("[data-action=add]").disabled');b.click('[data-action=add]');b.fill('#qa-edit-inputs',src/'data.txt');b.click('.qa-advanced > summary');b.click('#oa-add-preserve-atime');assert b.js('document.querySelector("#oa-add-preserve-atime").checked');b.click('[data-action=add-submit]')
 b.wait('document.querySelector(".qa-dialog")?.innerText.includes("来源访问时间") && !document.querySelector(".qa-dialog")?.innerText.includes("任务进度")');b.screenshot('main-add-warning');report['main_notice']=b.js('document.querySelector(".qa-dialog").innerText')
 with zipfile.ZipFile(archive) as z:assert z.read('data.txt')==b'timestamp GUI proof' and z.read('base.txt')==b'base'
 checked('main-add-completes-and-notice-remains-after-refresh');b.click('.qa-dialog-header [data-action=close]')
 b.click('[data-action=add]');b.click('.qa-advanced > summary');assert b.js('document.querySelector("#oa-add-preserve-atime").checked');b.click('.qa-dialog-header [data-action=close]');checked('main-add-remembers-choice')
 b.click('[data-view="tools"]');b.click('[data-view="filemanager"]');b.wait('document.querySelectorAll(".fm-pane").length===2');b.fill('[data-fm-path="0"] input',src);b.click('[data-fm="go"][data-pane="0"]');b.wait('document.querySelector(`[data-fm-pane="0"]`).innerText.includes("data.txt")');b.click('[data-fm="archive"][data-pane="1"]')
 b.click('[data-fm="entry"][data-pane="0"][data-index="0"]');b.wait('!!document.querySelector("#fm-preserve-atime")');b.js('document.querySelector("#fm-preserve-atime").closest("details").open=true');assert b.js('document.querySelector("#fm-preserve-atime").checked');b.screenshot('fm-remembered-choice');checked('file-manager-shares-choice')
 # Deny only WRITE_ATTRIBUTES on our own fixture; preserve the descriptor.
 denied=src/'denied.txt';denied.write_bytes(b'preserve denied fixture')
 adv=ctypes.WinDLL('advapi32',use_last_error=True);K=ctypes.WinDLL('kernel32',use_last_error=True);P=ctypes.c_void_p;U=ctypes.c_uint32
 adv.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes=[ctypes.c_wchar_p,U,ctypes.POINTER(P),ctypes.POINTER(U)];adv.SetFileSecurityW.argtypes=[ctypes.c_wchar_p,U,P];adv.GetFileSecurityW.argtypes=[ctypes.c_wchar_p,U,P,U,ctypes.POINTER(U)];K.LocalFree.argtypes=[P]
 def sd():
  needed=U();adv.GetFileSecurityW(str(denied),4,None,0,ctypes.byref(needed));buf=ctypes.create_string_buffer(needed.value);assert adv.GetFileSecurityW(str(denied),4,buf,needed.value,ctypes.byref(needed));return buf
 original=sd();descriptor=P();assert adv.ConvertStringSecurityDescriptorToSecurityDescriptorW('D:P(D;;0x100;;;WD)(A;;FA;;;OW)',1,ctypes.byref(descriptor),None)
 try:assert adv.SetFileSecurityW(str(denied),0x80000004,descriptor)
 finally:K.LocalFree(descriptor)
 try:
  before=sd().raw;b.click('[data-fm="entry"][data-pane="0"][data-index="0"]');b.click('[data-fm="refresh"][data-pane="0"]');b.wait('Array.from(document.querySelectorAll(`[data-fm="entry"][data-pane="0"]`)).some(e=>e.textContent.includes("denied.txt"))')
  index=b.js('Array.from(document.querySelectorAll(`[data-fm="entry"][data-pane="0"]`)).find(e=>e.textContent.includes("denied.txt")).dataset.index');b.click(f'[data-fm="entry"][data-pane="0"][data-index="{index}"]');b.click('[data-fm="copy"]')
  b.wait('document.querySelector(".qa-dialog")?.innerText.includes("未能启用访问时间保护")');report['fm_notice']=b.js('document.querySelector(".qa-dialog").innerText');b.screenshot('fm-denied-warning')
  with zipfile.ZipFile(archive) as z:assert z.read('denied.txt')==b'preserve denied fixture' and z.read('data.txt')==b'timestamp GUI proof' and z.read('base.txt')==b'base'
  assert denied.read_bytes()==b'preserve denied fixture';assert sd().raw==before;report['denied_acl_sha256']=hashlib.sha256(before).hexdigest();checked('file-manager-real-attribute-denial-success-with-persistent-warning-and-unchanged-acl')
 finally:assert adv.SetFileSecurityW(str(denied),0x20000004,original)
 b.click('.qa-dialog-header [data-action=close]')
 moving=src/'move-me.txt';moving.write_bytes(b'move preservation GUI proof')
 # Navigate through the actual path form to clear the previous selection.
 b.fill('[data-fm-path="0"] input',src);b.click('[data-fm="go"][data-pane="0"]');b.wait('Array.from(document.querySelectorAll(`[data-fm="entry"][data-pane="0"]`)).some(e=>e.textContent.includes("move-me.txt"))')
 index=b.js('Array.from(document.querySelectorAll(`[data-fm="entry"][data-pane="0"]`)).find(e=>e.textContent.includes("move-me.txt")).dataset.index');b.click(f'[data-fm="entry"][data-pane="0"][data-index="{index}"]');b.click('[data-fm="move"]')
 b.wait('Array.from(document.querySelectorAll(`[data-fm="entry"][data-pane="1"]`)).some(e=>e.textContent.includes("move-me.txt")) && !document.querySelector(`[data-fm="move"]`).disabled')
 assert not moving.exists()
 with zipfile.ZipFile(archive) as z:
  expected={'base.txt':b'base','data.txt':b'timestamp GUI proof','denied.txt':b'preserve denied fixture','move-me.txt':b'move preservation GUI proof'};assert set(z.namelist())==set(expected)
  for name,payload in expected.items():assert hashlib.sha256(z.read(name)).digest()==hashlib.sha256(payload).digest()
 b.screenshot('fm-move-completed');checked('real-file-manager-move-removes-only-source-after-full-archive-verification')
 b.close();assert b.proc.poll() is not None;b=Browser();b.wait('!document.querySelector("[data-action=add]").disabled');b.click('[data-action=add]');b.click('.qa-advanced > summary');assert b.js('document.querySelector("#oa-add-preserve-atime").checked');b.screenshot('remembered-after-process-restart');checked('choice-persists-after-real-process-restart-in-isolated-profile')
 assert dependencies()==bound;report['passed']=True
except Exception as error:
 report['error']=str(error)
 if b and b.ws:
  try:b.screenshot('failure');report['ui']=b.js('document.body.innerText')
  except Exception:pass
 raise
finally:
 if b:b.close()
 report['process_terminal']=b is None or b.proc is None or b.proc.poll() is not None;save()
