"""Actual WebView input and native archive output; isolated profile only."""
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes
from websockets.sync.client import connect
ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args();a.exe=a.exe.resolve();a.out=a.out.resolve();a.out.mkdir()
assert not subprocess.run(['powershell','-NoProfile','-Command','(Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue).Path'],capture_output=True).stdout.strip(),'Other GUI open'
root=Path(tempfile.mkdtemp(prefix='qing-links-gui-'));src=root/'left';src.mkdir();(src/'existing.txt').write_bytes(b'keep');(src/'source.txt').write_bytes(b'original bytes');(src/'source-dir').mkdir();right=root/'right';right.mkdir();profile=root/'profile';local=root/'local';local.mkdir()
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
  self.proc=subprocess.Popen([str(a.exe)],env=env,cwd=root,creationflags=subprocess.CREATE_NO_WINDOW)
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
 b=Browser.__new__(Browser);Browser.__init__(b)
 b.call('Page.enable')
 b.call('Page.addScriptToEvaluateOnNewDocument',{'source':"""Object.defineProperty(window,'QingFileWorkspace',{configurable:true,set(api){const original=api.create;api.create=function(ui){window.__hookCreated=true;const run=ui.run;ui.run=async function(...args){if(args[0].operation==='fs-create-link'){window.__linkRequests=(window.__linkRequests||0)+1;window.__lastLinkRequest=args[0];}const result=await run(...args);if(window.__holdLink&&args[0].operation==='fs-create-link'){window.__realLinkFinished=true;await new Promise(r=>window.__releaseLink=r);}return result;};return original(ui);};Object.defineProperty(window,'QingFileWorkspace',{value:api,writable:true,configurable:true});}});"""})
 b.call('Page.reload');b.wait('window.__hookCreated===true && !!document.querySelector("[data-action=compress]")');time.sleep(.5)
 b.click('[data-view="tools"]');b.click('[data-view="filemanager"]');b.wait('document.querySelectorAll(".fm-pane").length===2')
 for index,path in enumerate([src,right]):
  selector=f'[data-fm-path="{index}"] input';b.fill(selector,str(path));b.click(f'[data-fm="go"][data-pane="{index}"]');b.wait('document.querySelector('+json.dumps(selector)+').value==='+json.dumps(str(path)))
 def open_link(name):
  # Check the desired actual local row, not a synthetic selection/model.
  check='[data-fm-check="0"][aria-label='+json.dumps('选择 '+name,ensure_ascii=False)+']'
  b.click(check);b.click('[data-fm-pane="0"] .fm-pane-actions details summary');b.click('[data-fm="link"][data-pane="0"]');b.wait('!!document.querySelector("[data-fm-link]")')
 open_link('source.txt')
 assert b.js('document.querySelector("[data-fm-link-parent]").value')==str(right)
 assert b.js('Array.from(document.querySelector("[data-fm-link-kind]").options).map(v=>v.value)')==['hard','symbolic-file']
 assert str(src/'source.txt') in b.js('document.querySelector("[data-fm-link]").innerText')
 assert '共享同一文件内容' in b.js('document.querySelector("[data-fm-link]").innerText')
 b.fill('[data-fm-link-name]','硬链接.txt');b.js('window.__holdLink=true');b.click('[data-fm-link] [type=submit]');b.wait('window.__realLinkFinished===true')
 assert (right/'硬链接.txt').read_bytes()==b'original bytes';assert os.stat(src/'source.txt').st_ino==os.stat(right/'硬链接.txt').st_ino
 assert b.js('document.querySelector("[data-fm-link-name]").disabled')
 b.key('Enter','Enter',13);b.key('Escape','Escape',27)
 assert b.js('window.__linkRequests')==1;assert b.js('!!document.querySelector("[data-fm-link]")')
 b.js('window.__holdLink=false;window.__releaseLink()');b.wait('!document.querySelector("[data-fm-link]")')
 b.wait('Array.from(document.querySelectorAll(`[data-fm-pane="1"] .fm-row`)).some(r=>r.innerText.includes("硬链接.txt"))')
 checked('actual-hardlink-shared-identity-refresh-and-duplicate-submit-blocked');b.screenshot('hardlink-success')
 # Source remains selected; reopen the second-level action without toggling it.
 b.click('[data-fm-pane="0"] .fm-pane-actions details summary');b.click('[data-fm="link"][data-pane="0"]');b.fill('[data-fm-link-name]','硬链接.txt');b.click('[data-fm-link] [type=submit]');b.wait('!!document.querySelector(".fm-link-error")')
 assert '同名项目已存在' in b.js('document.querySelector(".fm-link-error").innerText');assert b.js('document.querySelector("[data-fm-link-name]").value')=='硬链接.txt';assert (right/'硬链接.txt').read_bytes()==b'original bytes';checked('actual-conflict-preserves-existing-and-input')
 b.select('[data-fm-link-kind]','symbolic-file');b.fill('[data-fm-link-name]','file-symbolic.txt');b.click('[data-fm-link] [type=submit]');b.wait('!!document.querySelector(".fm-link-error")')
 assert '1314' in b.js('document.querySelector(".fm-link-error").innerText');assert not (right/'file-symbolic.txt').exists();assert b.js('document.querySelector("[data-fm-link-name]").value')=='file-symbolic.txt';checked('actual-file-symbolic-permission-failure-no-residue');b.screenshot('file-symbolic-permission-failure')
 b.click('[data-fm="link-close"]');b.click('[data-fm-check="0"][aria-label="选择 source.txt"]');open_link('source-dir')
 assert b.js('Array.from(document.querySelector("[data-fm-link-kind]").options).map(v=>v.value)')==['symbolic-directory']
 b.fill('[data-fm-link-name]','dir-symbolic');b.click('[data-fm-link] [type=submit]');b.wait('!!document.querySelector(".fm-link-error")')
 assert '1314' in b.js('document.querySelector(".fm-link-error").innerText');assert not (right/'dir-symbolic').exists();checked('actual-directory-symbolic-permission-failure-no-residue');b.screenshot('directory-symbolic-permission-failure')
 assert (src/'existing.txt').read_bytes()==b'keep';assert (src/'source.txt').read_bytes()==b'original bytes'
 assert sorted(p.name for p in right.iterdir())==['硬链接.txt'];assert dependencies()==bound
 report['limitations']=['No symbolic-link success evidence on this token; native CI with existing permission is required.','TxF-unavailable and source READ_DATA-denied compatibility remain open.','Junction/WSL/edit-existing-links are out of this phase.'];report['passed']=True;save()
except Exception as error:
 report['error']=str(error)
 if b and b.ws:
  try:b.screenshot('failure')
  except Exception:pass
 save();raise
finally:
 if b:b.close()
 report['process_terminal']=b is None or b.proc is None or b.proc.poll() is not None;save()
