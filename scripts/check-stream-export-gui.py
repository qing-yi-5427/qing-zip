"""Real ADS-export WebView input and native jobs; isolated profile/owned fixtures only."""
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes,struct,msvcrt
from websockets.sync.client import connect
ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args();a.exe=a.exe.resolve();a.out=a.out.resolve();a.out.mkdir()
assert not subprocess.run(['powershell','-NoProfile','-Command','(Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue).Path'],capture_output=True).stdout.strip(),'Other GUI open'
root=Path(tempfile.mkdtemp(prefix='qing-ads-gui-'));src=root/'left';src.mkdir();right=root/'right';right.mkdir();profile=root/'profile';local=root/'local';local.mkdir()
(source:=src/'source.txt').write_bytes(b'base original');(directory:=src/'source-dir').mkdir();(directory/'child.txt').write_bytes(b'child original');(special:=src/'special.bin').write_bytes(b'special base');(cancel_source:=src/'cancel.bin').write_bytes(b'cancel base')
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

kernel=ctypes.WinDLL('kernel32',use_last_error=True)
kernel.CreateFileW.argtypes=[ctypes.c_wchar_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p];kernel.CreateFileW.restype=ctypes.c_void_p
kernel.CloseHandle.argtypes=[ctypes.c_void_p];kernel.CloseHandle.restype=ctypes.c_int
kernel.BackupWrite.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint32),ctypes.c_int,ctypes.c_int,ctypes.POINTER(ctypes.c_void_p)];kernel.BackupWrite.restype=ctypes.c_int
kernel.DeviceIoControl.argtypes=[ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint32),ctypes.c_void_p];kernel.DeviceIoControl.restype=ctypes.c_int
def put_streams(path,streams):
 handle=kernel.CreateFileW(str(path),0x40000000,7,None,3,0x02200000,None);assert handle not in (None,ctypes.c_void_p(-1).value),ctypes.get_last_error();context=ctypes.c_void_p();done=ctypes.c_uint32()
 try:
  for name,payload in streams.items():
   name_bytes=(':'+name+':$DATA').encode('utf-16le');data=struct.pack('<IIqI',4,0,len(payload),len(name_bytes))+name_bytes+payload;buffer=ctypes.create_string_buffer(data)
   assert kernel.BackupWrite(handle,buffer,len(data),ctypes.byref(done),0,0,ctypes.byref(context)),ctypes.get_last_error();assert done.value==len(data)
 finally:
  assert kernel.BackupWrite(handle,None,0,ctypes.byref(done),1,0,ctypes.byref(context)),ctypes.get_last_error();kernel.CloseHandle(handle)
def native(request):
 result=subprocess.run([str(a.exe.parent/'obz.exe'),'json'],input=json.dumps(request,ensure_ascii=False).encode('utf-8'),capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
 assert result.returncode==0,(result.stdout.decode('utf-8','replace'),result.stderr.decode('utf-8','replace'));return json.loads(result.stdout)
def source_inventory(path):return native({'operation':'fs-streams','path':str(path)})
file_streams={'notes.txt':b'notes original\n','empty':b''};directory_streams={'dir-note.log':b'directory notes'};special_streams={'bad?.txt':b'question original','bad*.txt':b'star original','trail. ':b'trailing original','CON':b'device original'}
put_streams(source,file_streams);put_streams(directory,directory_streams);put_streams(special,special_streams);put_streams(cancel_source,{'a-first':b'first retained'})
large_path=str(cancel_source)+':z-large';large_size=8*1024**3
kernel.SetFilePointerEx.argtypes=[ctypes.c_void_p,ctypes.c_longlong,ctypes.c_void_p,ctypes.c_uint32];kernel.SetEndOfFile.argtypes=[ctypes.c_void_p]
handle=kernel.CreateFileW(large_path,0xc0000000,7,None,2,0x00200000,None);assert handle not in (None,ctypes.c_void_p(-1).value),ctypes.get_last_error()
try:
 returned=ctypes.c_uint32();assert kernel.DeviceIoControl(handle,0x900c4,None,0,None,0,ctypes.byref(returned),None),ctypes.get_last_error();assert kernel.SetFilePointerEx(handle,large_size,None,0);assert kernel.SetEndOfFile(handle)
 query=ctypes.create_string_buffer(struct.pack('<qq',0,large_size));ranges=ctypes.create_string_buffer(65536);assert kernel.DeviceIoControl(handle,0x940cf,query,16,ranges,len(ranges),ctypes.byref(returned),None),ctypes.get_last_error();assert returned.value==0,'cancellation source must have real sparse holes';report['native_sparse_ranges_before']=[];save()
finally:kernel.CloseHandle(handle)
original_inventory={str(path):source_inventory(path) for path in [source,directory,special,cancel_source]}
native({'operation':'fs-create-link','dest':str(src),'name':'existing-wsl','link_kind':'wsl','wsl_target':'../original-target'})
b=None
try:
 b=Browser.__new__(Browser);Browser.__init__(b);b.call('Page.enable')
 b.click('[data-view="tools"]');b.click('[data-view="filemanager"]');b.wait('document.querySelectorAll(".fm-pane").length===2')
 def go(index,path):
  selector=f'[data-fm-path="{index}"] input';b.fill(selector,str(path));b.click(f'[data-fm="go"][data-pane="{index}"]');b.wait('document.querySelector('+json.dumps(selector)+').value==='+json.dumps(str(path))+' && !document.querySelector('+json.dumps(selector)+').disabled')
 def clear_selection(index=0):
  for label in b.js('Array.from(document.querySelectorAll('+json.dumps(f'[data-fm-check="{index}"]:checked')+')).map(e=>e.getAttribute("aria-label"))'):b.click(f'[data-fm-check="{index}"][aria-label='+json.dumps(label,ensure_ascii=False)+']')
 def open_streams(name):
  clear_selection();b.click('[data-fm-check="0"][aria-label='+json.dumps('选择 '+name,ensure_ascii=False)+']');b.click('[data-fm-pane="0"] .fm-pane-actions details summary');b.click('[data-fm="streams"][data-pane="0"]');b.wait('!!document.querySelector(".fm-stream-table")')
  assert not b.js('!!document.querySelector(".fm-panes")'),'ADS must be a secondary page'
 def choose(name):b.click('[data-fm-stream-check][aria-label='+json.dumps('选择数据流 '+name,ensure_ascii=False)+']')
 def rename_output(name,output):b.fill('[data-fm-stream-name][aria-label='+json.dumps('输出文件名：'+name,ensure_ascii=False)+']',output)
 def return_files():b.click('[data-fm="streams-close"]');b.wait('document.querySelectorAll(".fm-pane").length===2')
 def run_export(status='已完成'):
  b.click('[data-fm-stream-export] [type=submit]');finish_job(status)
 def finish_job(status):
  b.wait('!!document.querySelector(".qa-dialog-footer [data-action=close]") && !document.querySelector(".qa-dialog-footer [data-action=close]").disabled')
  assert status in b.js('document.querySelector(".qa-jobcard").innerText');b.screenshot('job-'+str(len(report['checks'])));b.click('.qa-dialog-footer [data-action="close"]');b.wait('!!document.querySelector(".fm-streams")')
 go(0,src);go(1,right)
 open_streams(source.name);assert b.js('document.querySelectorAll(".fm-stream-table th").length')==3;assert not b.js('!!document.querySelector("[data-fm-stream-export]")');checked('existing-readonly-inventory-entry-and-metadata-retained')
 choose('notes.txt');rename_output('notes.txt','single-note.txt');run_export();assert (right/'single-note.txt').read_bytes()==file_streams['notes.txt'];checked('real-file-ADS-single-export-with-custom-output-name')
 # Real collision: the previous output remains byte-identical and inputs survive.
 run_export('失败');assert (right/'single-note.txt').read_bytes()==file_streams['notes.txt'];assert b.js('document.querySelector("[data-fm-stream-name]").value')=='single-note.txt';assert '已导出 0 / 1' in b.js('document.querySelector(".fm-stream-export-error").innerText');checked('existing-output-refused-with-input-and-original-bytes-retained')
 return_files();open_streams(directory.name);choose('dir-note.log');rename_output('dir-note.log','directory-note.txt');run_export();assert (right/'directory-note.txt').read_bytes()==directory_streams['dir-note.log'];checked('real-directory-ADS-single-export')
 return_files();open_streams(special.name);b.click('[data-fm="streams-select-all"]');before={p.name:sha(p) for p in right.iterdir()};b.click('[data-fm-stream-export] [type=submit]');b.wait('!!document.querySelector(".fm-stream-export-error")');assert '同一输出文件名' in b.js('document.querySelector(".fm-stream-export-error").innerText');assert {p.name:sha(p) for p in right.iterdir()}==before
 assert b.js('Array.from(document.querySelectorAll("[data-fm-stream-name]")).map(e=>e.value)').count('bad_.txt')==2
 rename_output('bad*.txt','star-edited.txt');run_export();mapping={'bad?.txt':'bad_.txt','bad*.txt':'star-edited.txt','trail. ':'trail__','CON':'_CON'}
 for name,output in mapping.items():assert (right/output).read_bytes()==special_streams[name]
 checked('real-multi-ADS-corrected-name-collision-edit-and-export');b.screenshot('special-name-mapping')
 # Read actual output directory in both panes, with automatic refresh disabled.
 return_files();go(1,src)
 for index in [0,1]:
  if b.js(f'document.querySelector(`[data-fm-auto="{index}"]`).checked'):b.click(f'[data-fm-auto="{index}"]')
 open_streams(source.name);choose('notes.txt');rename_output('notes.txt','same-directory.txt');run_export();assert (src/'same-directory.txt').read_bytes()==file_streams['notes.txt'];return_files()
 for index in [0,1]:
  b.wait(f'Array.from(document.querySelectorAll(`[data-fm-pane="{index}"] .fm-row`)).some(r=>r.innerText.includes("same-directory.txt"))');assert b.js(f'document.querySelector(`[data-fm-auto="{index}"]`).checked') is False
 checked('both-same-directory-panes-force-refresh-with-auto-refresh-off')
 # Existing link creation and inspect entrances remain functional.
 clear_selection();b.click('[data-fm-check="0"][aria-label="选择 source.txt"]');b.click('[data-fm-pane="0"] .fm-pane-actions details summary');b.click('[data-fm="link"][data-pane="0"]');b.wait('!!document.querySelector("[data-fm-link]")');b.click('[data-fm="link-close"]')
 inspect='[data-fm="link-inspect"][data-pane="0"][data-path='+json.dumps(str(src/'existing-wsl'),ensure_ascii=False)+']';b.click(inspect);b.wait('!!document.querySelector("[data-fm-link-edit-target]")');assert b.js('document.querySelector("[data-fm-link-edit-target]").value')=='../original-target';b.click('[data-fm="link-editor-close"]');checked('existing-link-create-and-inspect-secondary-entrances-retained')
 # First selected item finishes; cancel the next real native stream copy.
 cancel_dest=root/'cancel-output';cancel_dest.mkdir();go(1,cancel_dest);open_streams(cancel_source.name);b.click('[data-fm="streams-select-all"]');b.click('[data-fm-stream-export] [type=submit]')
 deadline=time.monotonic()+30
 while not (cancel_dest/'a-first').exists():
  assert time.monotonic()<deadline,'first ADS did not publish';time.sleep(.01)
 b.click('[data-action="canceljob"]');finish_job('已取消');text=b.js('document.querySelector(".fm-stream-export-error").innerText');assert '已导出 1 / 2' in text and 'a-first' in text,text;assert (cancel_dest/'a-first').read_bytes()==b'first retained';assert sorted(p.name for p in cancel_dest.iterdir())==['a-first'];checked('real-job-cancel-after-first-publication-shows-partial-count-and-path')
 return_files();b.wait('Array.from(document.querySelectorAll(`[data-fm-pane="1"] .fm-row`)).some(r=>r.innerText.includes("a-first"))');checked('returned-file-view-shows-real-partial-output')
 assert source.read_bytes()==b'base original';assert (directory/'child.txt').read_bytes()==b'child original';assert special.read_bytes()==b'special base';assert cancel_source.read_bytes()==b'cancel base'
 for path in [source,directory,special,cancel_source]:assert source_inventory(path)==original_inventory[str(path)]
 assert dependencies()==bound;report['limitations']=['Read access may update last-access time.','The batch is not atomic; cancellation preserves already completed outputs.'];report['passed']=True;save()
except Exception as error:
 report['error']=str(error)
 if b and b.ws:
  try:b.screenshot('failure')
  except Exception:pass
 save();raise
finally:
 if b:b.close()
 report['process_terminal']=b is None or b.proc is None or b.proc.poll() is not None;save()
