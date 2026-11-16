"""Actual WebView input and native archive output; isolated profile only."""
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes
from websockets.sync.client import connect
ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args();a.exe=a.exe.resolve();a.out=a.out.resolve();a.out.mkdir()
assert not subprocess.run(['powershell','-NoProfile','-Command','(Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue).Path'],capture_output=True).stdout.strip(),'Other GUI open'
root=Path(tempfile.mkdtemp(prefix='qing-timestamps-gui-'));src=root/'left';src.mkdir();(src/'existing.txt').write_bytes(b'keep');right=root/'right';right.mkdir();profile=root/'profile';local=root/'local';local.mkdir()
for i in range(2):(src/f'p{i:04d}.txt').write_bytes(b'original')
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
 from pywinauto import Desktop
 nested=root/'hash-tree';(nested/'one'/'two').mkdir(parents=True);empty=root/'empty';empty.mkdir()
 files={nested/'a.txt':b'alpha',nested/'one'/'two'/'b.bin':b'nested bytes',root/'outside.txt':b'outside'}
 for path,payload in files.items():path.write_bytes(payload)
 b=Browser();b.click('[data-view="tools"]');b.click('[data-action="checksum-tool"]');b.wait('!!document.querySelector("iframe")')
 url=b.js('document.querySelector("iframe").src');b.call('Page.navigate',{'url':url});b.wait('!!document.querySelector("#checksum-folder")')
 b.select('#hash-algorithm','SHA256')
 b.click('#checksum-folder')
 desktop=Desktop(backend='uia');dialog=desktop.window(process=b.proc.pid,class_name='#32770');dialog.wait('visible',timeout=15)
 report['native_picker_title']=dialog.window_text();dialog.child_window(control_type='Edit',found_index=0).set_edit_text(str(nested))
 buttons=dialog.descendants(control_type='Button');report['native_picker_buttons']=[x.window_text() for x in buttons]
 # Standard IFileDialog folder picker accepts its filename field with Enter.
 dialog.child_window(control_type='Edit',found_index=0).type_keys('{ENTER}')
 try:dialog.wait_not('visible',timeout=2)
 except Exception:
  [x for x in dialog.descendants(control_type='Button') if x.window_text() in ['选择文件夹','Select Folder','选择文件夹(&S)']][0].click_input()
 b.wait('document.querySelector("#checksum-paths").value==='+json.dumps(str(nested)))
 checked('real-native-folder-picker-populates-tool')
 b.click('#run');b.wait('document.querySelector("#status").textContent==="已完成"')
 result=b.js('document.querySelector("#result").textContent')
 for path,payload in files.items():
  if path.is_relative_to(nested):assert str(path) in result and hashlib.sha256(payload).hexdigest().lower() in result.lower(),result
 b.screenshot('recursive-nested');checked('nested-directory-results-match-independent-sha256')
 b.fill('#checksum-paths',empty);b.click('#run');b.wait('document.querySelector("#status").textContent==="已完成"');assert '没有可校验的文件' in b.js('document.querySelector("#result").textContent');checked('empty-directory-is-explicit')
 b.fill('#checksum-paths',str(nested)+'\n'+str(root/'outside.txt'));b.click('#run');b.wait('document.querySelector("#status").textContent==="已完成"')
 result=b.js('document.querySelector("#result").textContent')
 for path,payload in files.items():assert str(path) in result and hashlib.sha256(payload).hexdigest().lower() in result.lower()
 b.screenshot('mixed-results');checked('mixed-file-and-directory-results-match-independent-sha256')
 before=b.js('document.querySelector("#checksum-paths").value');b.click('#checksum-folder');dialog.wait('visible',timeout=15);dialog.type_keys('{ESC}');dialog.wait_not('visible',timeout=10);time.sleep(.2);assert b.js('document.querySelector("#checksum-paths").value')==before;checked('native-folder-picker-cancel-preserves-input')
 large=root/'large';large.mkdir();chunk=b'z'*(1024*1024)
 with (large/'large.bin').open('wb') as f:
  for _ in range(512):f.write(chunk)
 b.fill('#checksum-paths',large);b.select('#hash-algorithm','*');b.click('#run');b.wait('!document.querySelector("#cancel").disabled');b.click('#cancel');b.wait('document.querySelector("#status").textContent==="已取消"');assert not b.js('document.querySelector("#result").textContent');b.screenshot('cancelled');checked('recursive-hash-task-cancels-without-partial-success')
 assert dependencies()==bound;report['passed']=True;save()
except Exception as error:
 report['error']=str(error)
 if b and b.ws:
  try:b.screenshot('failure')
  except Exception:pass
 save();raise
finally:
 if b:b.close()
 report['process_terminal']=b is None or b.proc is None or b.proc.poll() is not None;save()
