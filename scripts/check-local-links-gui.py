"""Actual WebView input and native local links; isolated profile and owned fixtures only."""
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes,struct
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
 b.call('Page.addScriptToEvaluateOnNewDocument',{'source':"""Object.defineProperty(window,'QingFileWorkspace',{configurable:true,set(api){const original=api.create;api.create=function(ui){window.__hookCreated=true;const run=ui.run;ui.run=async function(...args){if(args[0].operation==='fs-create-link'){window.__linkRequests=(window.__linkRequests||0)+1;window.__lastLinkRequest=args[0];}window.__linkOperationCounts=window.__linkOperationCounts||{};window.__linkOperationCounts[args[0].operation]=(window.__linkOperationCounts[args[0].operation]||0)+1;const result=await run(...args);if(window.__holdOperation===args[0].operation){window.__realOperationFinished=true;await new Promise(r=>window.__releaseOperation=r);}if(window.__holdLink&&args[0].operation==='fs-create-link'){window.__realLinkFinished=true;await new Promise(r=>window.__releaseLink=r);}return result;};return original(ui);};Object.defineProperty(window,'QingFileWorkspace',{value:api,writable:true,configurable:true});}});"""})
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
 assert b.js('Array.from(document.querySelector("[data-fm-link-kind]").options).map(v=>v.value)')==['hard','symbolic-file','wsl']
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
 assert any(('os error '+code) in b.js('document.querySelector(".fm-link-error").innerText') for code in ('1300','1314'));assert not (right/'file-symbolic.txt').exists();assert b.js('document.querySelector("[data-fm-link-name]").value')=='file-symbolic.txt';checked('actual-file-symbolic-permission-failure-no-residue');b.screenshot('file-symbolic-permission-failure')
 b.click('[data-fm="link-close"]');b.click('[data-fm-check="0"][aria-label="选择 source.txt"]');open_link('source-dir')
 assert b.js('Array.from(document.querySelector("[data-fm-link-kind]").options).map(v=>v.value)')==['symbolic-directory','junction','wsl']
 b.fill('[data-fm-link-name]','dir-symbolic');b.click('[data-fm-link] [type=submit]');b.wait('!!document.querySelector(".fm-link-error")')
 assert any(('os error '+code) in b.js('document.querySelector(".fm-link-error").innerText') for code in ('1300','1314'));assert not (right/'dir-symbolic').exists();checked('actual-directory-symbolic-permission-failure-no-residue');b.screenshot('directory-symbolic-permission-failure')
 b.click('[data-fm="link-close"]');b.click('[data-fm-check="0"][aria-label="选择 source-dir"]')
 b.fill('[data-fm-path="1"] input',str(src));b.click('[data-fm="go"][data-pane="1"]');b.wait('document.querySelector(`[data-fm-path="1"] input`).value==='+json.dumps(str(src)))
 for index in [0,1]:
  if b.js(f'document.querySelector(`[data-fm-auto="{index}"]`).checked'):b.click(f'[data-fm-auto="{index}"]')
 open_link('source.txt');b.fill('[data-fm-link-name]','同目录链接.txt');b.click('[data-fm-link] [type=submit]');b.wait('!document.querySelector("[data-fm-link]")')
 for index in [0,1]:
  b.wait(f'Array.from(document.querySelectorAll(`[data-fm-pane="{index}"] .fm-row`)).some(r=>r.innerText.includes("同目录链接.txt"))')
  assert b.js(f'document.querySelector(`[data-fm-auto="{index}"]`).checked') is False
 assert os.stat(src/'source.txt').st_ino==os.stat(src/'同目录链接.txt').st_ino
 checked('actual-both-panes-same-directory-auto-refresh-off-update');b.screenshot('both-panes-same-directory')
 # Final 0.11.21 flows: real input, real native jobs, only owned filesystem fixtures.
 def native(request,success=True):
  result=subprocess.run([str(a.exe.parent/'obz.exe'),'json'],input=json.dumps(request,ensure_ascii=False).encode('utf-8'),capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
  assert (result.returncode==0)==success,(request,result.stdout.decode('utf-8','replace'),result.stderr.decode('utf-8','replace'))
  return json.loads(result.stdout) if success else result.stderr.decode('utf-8','replace')
 def go(index,path):
  b.fill(f'[data-fm-path="{index}"] input',str(path));b.click(f'[data-fm="go"][data-pane="{index}"]');b.wait('document.querySelector('+json.dumps(f'[data-fm-path="{index}"] input')+').value==='+json.dumps(str(path))+' && !document.querySelector('+json.dumps(f'[data-fm-path="{index}"] input')+').disabled')
 def clear_selection(index):
  for label in b.js('Array.from(document.querySelectorAll('+json.dumps(f'[data-fm-check="{index}"]:checked')+')).map(e=>e.getAttribute("aria-label"))'):
   b.click(f'[data-fm-check="{index}"][aria-label='+json.dumps(label,ensure_ascii=False)+']')
 def row_link(index,path):return f'[data-fm-pane="{index}"] [data-fm="link-inspect"][data-path='+json.dumps(str(path),ensure_ascii=False)+']'
 def inspect(index,path):
  b.click(row_link(index,path));b.wait('!!document.querySelector("[data-fm-link-editor]") && !document.querySelector("[data-fm-link-editor] [role=status]")?.textContent.includes("正在读取") && !!document.querySelector("[data-fm-link-edit-target]")')
  assert not b.js('!!document.querySelector(".fm-panes")'),'editor must be a secondary page'
 def return_files():b.click('[data-fm="link-editor-close"]');b.wait('document.querySelectorAll(".fm-pane").length===2')
 def both_show(name):
  for index in [0,1]:b.wait(f'Array.from(document.querySelectorAll(`[data-fm-pane="{index}"] .fm-row`)).some(r=>r.innerText.includes('+json.dumps(name)+'))')
 kernel=ctypes.WinDLL('kernel32',use_last_error=True)
 kernel.CreateFileW.argtypes=[ctypes.c_wchar_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p];kernel.CreateFileW.restype=ctypes.c_void_p
 kernel.DeviceIoControl.argtypes=[ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint32),ctypes.c_void_p];kernel.DeviceIoControl.restype=ctypes.c_int
 kernel.CloseHandle.argtypes=[ctypes.c_void_p];kernel.CloseHandle.restype=ctypes.c_int
 kernel.ReadFile.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint32),ctypes.c_void_p];kernel.ReadFile.restype=ctypes.c_int
 def raw_handle(path,access):
  handle=kernel.CreateFileW(str(path),access,7,None,3,0x02200000,None);assert handle not in (None,ctypes.c_void_p(-1).value),(str(path),ctypes.get_last_error());return handle
 def put_wsl(path,target):
  encoded=target.encode('utf-8');data=struct.pack('<IHHI',0xa000001d,len(encoded)+4,0,2)+encoded;handle=raw_handle(path,0x100);buffer=ctypes.create_string_buffer(data);returned=ctypes.c_uint32()
  try:assert kernel.DeviceIoControl(handle,0x900a4,buffer,len(data),None,0,ctypes.byref(returned),None),(str(path),ctypes.get_last_error())
  finally:kernel.CloseHandle(handle)
 def raw_bytes(path):
  handle=raw_handle(path,0x80000000);buffer=ctypes.create_string_buffer(4096);size=ctypes.c_uint32()
  try:assert kernel.ReadFile(handle,buffer,len(buffer),ctypes.byref(size),None);return buffer.raw[:size.value]
  finally:kernel.CloseHandle(handle)
 go(1,right);clear_selection(0);open_link('source-dir');b.select('[data-fm-link-kind]','junction')
 missing=root/'future'/'深层目录';b.fill('[data-fm-link-target]',str(missing));b.fill('[data-fm-link-name]','目录联接点');b.click('[data-fm-link] [type=submit]');b.wait('!document.querySelector("[data-fm-link]")')
 junction=right/'目录联接点';assert junction.lstat().st_file_attributes&0x410==0x410;assert not (root/'future').exists();assert native({'operation':'fs-inspect-link','path':str(junction)})['target']==str(missing)
 missing.mkdir(parents=True);(missing/'child.txt').write_bytes(b'later target bytes');assert (junction/'child.txt').read_bytes()==b'later target bytes';checked('actual-junction-dangling-target-later-real-access');b.screenshot('junction-success')
 clear_selection(0);open_link('source.txt');b.select('[data-fm-link-kind]','wsl');raw_target='../原始/用户 target';b.fill('[data-fm-link-wsl-target]',raw_target);b.fill('[data-fm-link-name]','WSL链接');b.click('[data-fm-link] [type=submit]');b.wait('!document.querySelector("[data-fm-link]")');wsl=right/'WSL链接'
 assert native({'operation':'fs-inspect-link','path':str(wsl)})['target']==raw_target;assert wsl.lstat().st_file_attributes&0x400;checked('actual-wsl-raw-unicode-relative-target-create')
 # A completed native inspection is delayed only after the real job, to check close/late completion.
 b.js('window.__holdOperation="fs-inspect-link";window.__realOperationFinished=false');b.click(row_link(1,wsl));b.wait('window.__realOperationFinished===true');return_files();b.js('window.__holdOperation=null;window.__releaseOperation()');time.sleep(.4);assert not b.js('!!document.querySelector("[data-fm-link-editor]")');checked('actual-native-inspect-late-result-cannot-reopen')
 inspect(1,wsl);assert b.js('document.querySelector("[data-fm-link-edit-target]").value')==raw_target;assert raw_target in b.js('document.querySelector("[data-fm-link-editor]").innerText');b.screenshot('wsl-actual-target-secondary-page')
 b.fill('[data-fm-link-edit-target]','/draft/旧对象');return_files();inspect(1,wsl);assert b.js('document.querySelector("[data-fm-link-edit-target]").value')=='/draft/旧对象';checked('actual-return-restores-same-object-draft-after-fresh-inspect');return_files()
 # Replace only our zero-byte created link. The replacement has hidden payload + ADS.
 wsl.unlink();wsl.write_bytes(b'foreign hidden payload');Path(str(wsl)+':keep').write_bytes(b'foreign ADS');put_wsl(wsl,'/foreign-native-target')
 inspect(1,wsl);assert b.js('document.querySelector("[data-fm-link-edit-target]").value')=='/foreign-native-target';assert '未恢复旧草稿' in b.js('document.querySelector("[data-fm-link-editor]").innerText');checked('actual-replaced-object-does-not-receive-old-draft')
 # External owned-fixture mutation leaves the displayed receipt stale.
 fresh=native({'operation':'fs-inspect-link','path':str(wsl)});native({'operation':'fs-edit-link','path':str(wsl),'link_receipt':fresh['receipt'],'link_target':'/external-change'})
 b.fill('[data-fm-link-edit-target]','/must-retain-draft');b.click('[data-fm-link-editor] [type=submit]');b.wait('!!document.querySelector(".fm-link-edit-error")');assert '查看后已改变' in b.js('document.querySelector(".fm-link-edit-error").innerText');assert b.js('document.querySelector("[data-fm-link-edit-target]").value')=='/must-retain-draft';assert native({'operation':'fs-inspect-link','path':str(wsl)})['target']=='/external-change';assert raw_bytes(wsl)==b'foreign hidden payload';checked('actual-stale-edit-failure-preserves-input-and-existing-payload');b.screenshot('stale-edit-preserves-input')
 b.click('[data-fm="link-editor-refresh"]');b.wait('document.querySelector("[data-fm-link-edit-target]")?.value==="/external-change"');return_files()
 for index in [0,1]:
  go(index,right)
  if b.js(f'document.querySelector(`[data-fm-auto="{index}"]`).checked'):b.click(f'[data-fm-auto="{index}"]')
 inspect(0,wsl);b.fill('[data-fm-link-edit-target]','/edited/用户');(right/'edit-refresh-marker.txt').write_bytes(b'edit marker')
 before_edits=b.js('window.__linkOperationCounts["fs-edit-link"]||0');b.js('window.__holdOperation="fs-edit-link";window.__realOperationFinished=false');b.click('[data-fm-link-editor] [type=submit]');b.wait('window.__realOperationFinished===true');assert b.js('document.querySelector("[data-fm-link-edit-target]").disabled');b.key('Enter','Enter',13);b.key('Escape','Escape',27);assert b.js('window.__linkOperationCounts["fs-edit-link"]')==before_edits+1;assert b.js('!!document.querySelector("[data-fm-link-editor]")');b.js('window.__holdOperation=null;window.__releaseOperation()');b.wait('!document.querySelector("[data-fm-link-editor]")');both_show('edit-refresh-marker.txt')
 assert native({'operation':'fs-inspect-link','path':str(wsl)})['target']=='/edited/用户';assert raw_bytes(wsl)==b'foreign hidden payload';checked('actual-edit-success-busy-duplicate-protection-both-panes-refresh-auto-off');b.screenshot('edited-both-panes')
 inspect(0,wsl);b.fill('[data-fm-link-edit-target]','');b.click('[data-fm-link-editor] [type=submit]');b.wait('document.querySelector(".fm-link-edit-error")?.innerText.includes("目标不能为空")');assert wsl.lstat().st_file_attributes&0x400
 (right/'clear-refresh-marker.txt').write_bytes(b'clear marker');b.click('[data-fm="link-editor-clear"]');assert '项目本身和原有数据都会保留' in b.js('document.querySelector("[data-fm-link-editor]").innerText');b.click('[data-fm="link-editor-clear-confirm"]');b.wait('!document.querySelector("[data-fm-link-editor]")');both_show('clear-refresh-marker.txt');assert wsl.read_bytes()==b'foreign hidden payload';assert Path(str(wsl)+':keep').read_bytes()==b'foreign ADS';assert not(wsl.lstat().st_file_attributes&0x400)
 for index in [0,1]:
  assert b.js(f'document.querySelector(`[data-fm-check="{index}"][aria-label="选择 WSL链接"]`).disabled') is False
  assert b.js(f'document.querySelector(`[data-fm-auto="{index}"]`).checked') is False
 checked('actual-independent-clear-keeps-entry-payload-ads-and-refreshes-both-panes');b.screenshot('clear-keeps-payload-both-panes')
 multi=right/'多链接原对象';alias=right/'多链接别名';multi.write_bytes(b'multi hidden bytes');os.link(multi,alias);put_wsl(multi,'/multi-original')
 b.click('[data-fm="refresh"][data-pane="0"]');b.wait('!!document.querySelector('+json.dumps(row_link(0,multi))+')');inspect(0,multi);assert '多硬链接' in b.js('document.querySelector("[data-fm-link-editor]").innerText');assert b.js('document.querySelector("[data-fm-link-edit-target]").disabled');assert b.js('document.querySelector("[data-fm-link-editor] [type=submit]").disabled');assert b.js('document.querySelector(`[data-fm="link-editor-clear"]`).disabled');assert os.stat(multi,follow_symlinks=False).st_nlink==2;checked('actual-multiple-hardlinks-inspect-readonly-edit-and-clear-disabled');b.screenshot('multiple-hardlinks-readonly');return_files()
 # Actual link rows still have disabled normal selection/open controls.
 for index in [0,1]:
  if index==1:b.click('[data-fm="refresh"][data-pane="1"]')
  b.wait('!!document.querySelector('+json.dumps(row_link(index,junction))+')')
  row='Array.from(document.querySelectorAll('+json.dumps(f'[data-fm-pane="{index}"] .fm-row')+')).find(r=>r.querySelector("[data-fm=link-inspect]")?.getAttribute("data-path")==='+json.dumps(str(junction))+')'
  assert b.js('('+row+').querySelector("input").disabled');assert b.js('('+row+').querySelector("[data-fm=entry]").disabled')
 copy_guard=root/'copy-guard';copy_guard.mkdir();assert native({'operation':'fs-copy','inputs':[str(junction)],'dest':str(copy_guard)},False);assert list(copy_guard.iterdir())==[];assert (junction/'child.txt').read_bytes()==b'later target bytes';checked('actual-linked-normal-selection-open-and-native-copy-protections-retained')
 assert (src/'existing.txt').read_bytes()==b'keep';assert (src/'source.txt').read_bytes()==b'original bytes';assert dependencies()==bound
 report['limitations']=['No symbolic-link success evidence on this token; native CI with existing permission is required.','TxF-unavailable and source READ_DATA-denied compatibility remain open.','WSL native reparse bytes are verified; Linux distribution traversal is not tested.'];report['passed']=True;save()
except Exception as error:
 report['error']=str(error)
 if b and b.ws:
  try:b.screenshot('failure')
  except Exception:pass
 save();raise
finally:
 if b:b.close()
 report['process_terminal']=b is None or b.proc is None or b.proc.poll() is not None;save()
