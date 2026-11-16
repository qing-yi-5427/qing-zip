"""Actual WebView input and native archive output; isolated profile only."""
from pathlib import Path
import argparse,os,time,json,tempfile,subprocess,hashlib,urllib.request,base64,ctypes
from websockets.sync.client import connect
ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args();a.exe=a.exe.resolve();a.out=a.out.resolve();a.out.mkdir()
assert not subprocess.run(['powershell','-NoProfile','-Command','(Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue).Path'],capture_output=True).stdout.strip(),'Other GUI open'
root=Path(tempfile.mkdtemp(prefix='qing-timestamps-gui-'));src=root/'输入';src.mkdir();(src/'data.txt').write_bytes(b'timestamp GUI proof');(src/'empty').mkdir();profile=root/'profile';local=root/'local';local.mkdir()
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
  for mode in ['rawKeyDown','keyUp']:self.call('Input.dispatchKeyEvent',{'type':mode,'key':key,'code':code,'windowsVirtualKeyCode':vk,'modifiers':mod})
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
def values(b):return b.js('Object.fromEntries(["mtime","ctime","atime"].map(k=>[k,document.querySelector("#oa-store-"+k).value]).concat([["archive",document.querySelector("#oa-archive-time").value]]))')
K=ctypes.WinDLL('kernel32',use_last_error=True);K.CreateFileW.restype=ctypes.c_void_p;K.CreateFileW.argtypes=[ctypes.c_wchar_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p];K.CloseHandle.argtypes=[ctypes.c_void_p];K.SetFileTime.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p]
def reset_times():
 for path,offset in [(src/'data.txt',0),(src/'empty',0),(src,86400)]:
  h=K.CreateFileW(str(path),0x100,7,None,3,0x02000000,None);assert h not in [None,ctypes.c_void_p(-1).value]
  vals=[ctypes.c_uint64((11644473600+v+offset)*10000000) for v in [1577836800,1609459200,1640995200]]
  try:assert K.SetFileTime(h,*[ctypes.byref(v) for v in vals])
  finally:K.CloseHandle(h)
def create(b,path):
 b.click('details:has(#qa-inputs) > summary');b.fill('#qa-inputs',src);b.fill('#qa-save',path);reset_times();b.click('[data-action=startcompress]')
 deadline=time.monotonic()+45
 while time.monotonic()<deadline:
  if path.exists():break
  time.sleep(.15)
 assert path.exists(),b.js('document.body.innerText');time.sleep(.4)
 if not Path(str(src/'data.txt')+':proof').exists():assert (src/'data.txt').stat().st_atime_ns==1609459200000000000
 b.wait('!document.querySelector("#qa-inputs")')
 nana=a.exe.parent/'runtime/nanazip/NanaZip.Universal.Console.exe'
 p=subprocess.run([str(nana),'l','-slt','-sccUTF-8',str(path)],capture_output=True,timeout=30);assert p.returncode==0,p.stderr
 (a.out/(path.name+'.metadata.txt')).write_bytes(p.stdout)
 dest=root/(path.name+'-decoded');p=subprocess.run([str(nana),'x','-y','-o'+str(dest),str(path)],capture_output=True,timeout=30);assert p.returncode==0,p.stderr
 assert (dest/src.name/'data.txt').read_bytes()==b'timestamp GUI proof'
 b.wait('document.body.innerText.includes("完整性检查通过")');b.click('.qa-dialog-footer [data-action=close]');return p
b=None
try:
 b=Browser();b.dialog();b.click('[data-action=compression-advanced]');b.select('#oa-store-mtime','false');b.select('#oa-store-ctime','true');b.select('#oa-store-atime','true');b.select('#oa-archive-time','latest');b.select('#qa-format','7z');b.click('#oa-preserve-atime');b.js('document.querySelector("#oa-store-mtime").scrollIntoView({block:"center"})');b.screenshot('time-options')
 expected={'mtime':'false','ctime':'true','atime':'true','archive':'latest'};assert values(b)==expected;assert b.js('document.querySelector("#oa-preserve-atime").checked')
 create(b,root/'gui-proof.7z');text=(a.out/'gui-proof.7z.metadata.txt').read_text(encoding='utf-8-sig');assert 'Accessed = 2021-01-02' in text and 'Created = 2020-01-02' in text
 assert (root/'gui-proof.7z').stat().st_mtime_ns==1641081600*1000000000;checked('actual_create_saved_ctime_atime_and_latest_with_content_sha')
 b.close();b=None;b=Browser();b.dialog();assert values(b)==expected;assert b.js('document.querySelector("#oa-preserve-atime").checked');checked('preferences_survive_actual_process_restart')
 b.click('[data-action=compression-advanced]');b.select('#qa-format','tar');assert b.js('["mtime","ctime","atime"].every(k=>document.querySelector("#oa-store-"+k).disabled)&&document.querySelector("#oa-archive-time").disabled&&document.querySelector("#oa-preserve-atime").disabled');checked('unsupported_format_disables_time_options');b.close_dialog()
 for bad in ['null','[]','{']:
  b.js('localStorage.setItem("qing-compression-times",'+json.dumps(bad)+')');b.dialog();assert values(b)=={'mtime':'','ctime':'','atime':'','archive':'default'};b.close_dialog()
 checked('null_array_and_malformed_preferences_recover')
 b.js('window.originalTimeStorageSet=Storage.prototype.setItem;Storage.prototype.setItem=function(k,v){if(k==="qing-compression-times")throw new DOMException("test quota","QuotaExceededError");return window.originalTimeStorageSet.call(this,k,v)}')
 b.dialog();b.click('[data-action=compression-advanced]');b.select('#oa-store-ctime','true');b.select('#oa-store-atime','true');b.click('#oa-preserve-atime');b.screenshot('storage-failure-still-usable');create(b,root/'storage-failure.zip');checked('storage_write_failure_does_not_block_actual_creation')
 b.js('Storage.prototype.setItem=window.originalTimeStorageSet')
 checked('preserved_regular_source_atime_and_postcreate_integrity_check')
 Path(str(src/'data.txt')+':proof').write_bytes(b'ADS warning fixture')
 b.dialog();b.click('[data-action=compression-advanced]')
 if not b.js('document.querySelector("#oa-preserve-atime").checked'):b.click('#oa-preserve-atime')
 b.js('document.querySelector("#oa-preserve-atime").scrollIntoView({block:"center"})');b.screenshot('preserve-switch')
 create(b,root/'ads-warning.zip')
 b.click('[data-view=history]');b.wait('document.body.innerText.includes("部分文件的访问时间未能保留")')
 b.click('details:has(summary) > summary');b.screenshot('atime-warning-details')
 assert '附加数据检查无法完整保留访问时间' in b.js('document.body.innerText')
 assert b.js('Array.from(document.querySelectorAll(".qa-jobcard")).some(c=>c.innerText.includes("ads-warning.zip")&&c.innerText.includes("已完成")&&c.innerText.includes("部分文件的访问时间未能保留"))')
 checked('actual_ads_gap_visible_with_file_reason_without_data_failure')
 assert dependencies()==bound;report['passed']=True
except Exception:
 if b and b.ws:
  try:b.screenshot('failure');report['last_ui']=b.js('document.body.innerText')
  except Exception:pass
 raise
finally:
 if b:b.close()
 save()
print(a.out/'receipt.json')
