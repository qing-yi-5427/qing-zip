"""Real UI acceptance against the immutable published ARM64 desktop package.
No invoke mocking, direct operation calls, or DOM mutation: only CDP input and observation.
"""
from pathlib import Path
import argparse,base64,socket,winreg,ctypes,hashlib,json,os,struct,subprocess,tempfile,time,traceback,urllib.request,zipfile
from ctypes import wintypes as w
from websockets.sync.client import connect

SOURCE='1688b5e741e86cd6e347046343ec3666aafcf3bf'
GUI_SHA='8604d47b41643a8f4d89bcc01f686855c15acdd9144528101e75378a7693a936'
CLI_SHA='b2bad9f32995020c0350895369c74e4bb6a31ca3182a52d5e495f3ad2854398c'
def sha(p):
 h=hashlib.sha256()
 with Path(p).open('rb') as f:
  for b in iter(lambda:f.read(1024*1024),b''):h.update(b)
 return h.hexdigest()
def machine(p):
 with Path(p).open('rb') as f:
  f.seek(0x3c);offset=struct.unpack('<I',f.read(4))[0];f.seek(offset);assert f.read(4)==b'PE\0\0';return struct.unpack('<H',f.read(2))[0]
def native(handle):
 k=ctypes.WinDLL('kernel32',use_last_error=True);k.IsWow64Process2.argtypes=[w.HANDLE,ctypes.POINTER(w.WORD),ctypes.POINTER(w.WORD)];k.IsWow64Process2.restype=w.BOOL
 p,n=w.WORD(),w.WORD();assert k.IsWow64Process2(handle,ctypes.byref(p),ctypes.byref(n)),ctypes.get_last_error();assert p.value==0 and n.value==0xaa64,(p.value,n.value)
 return {'process_machine':p.value,'native_machine':n.value}
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--exe',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);args=ap.parse_args()
 exe=args.exe.resolve(strict=True);out=args.out.resolve();out.mkdir(parents=True,exist_ok=False)
 evidence={'product_source':SOURCE,'test_driver_sha256':sha(__file__),'exe':str(exe),'gui_sha256':sha(exe),'cli_sha256':sha(exe.with_name('obz.exe')),'passed':False,'checks':[],'automation':'CDP Input mouse/key events; Runtime.evaluate only observes DOM or reads app identity; no operation invocation or mocks'}
 assert evidence['gui_sha256']==GUI_SHA and evidence['cli_sha256']==CLI_SHA
 assert machine(exe)==machine(exe.with_name('obz.exe'))==0xaa64
 root=Path(tempfile.mkdtemp(prefix='qing-arm64-ui-'));evidence['fixture']=str(root)
 left=root/'source';left.mkdir();right=root/'copied';right.mkdir();dest=root/'extracted'
 payload={'hello.txt':b'ARM64 real GUI roundtrip\n','empty.txt':b'', '\u4e2d\u6587.txt':'unicode contents'.encode()}
 for n,b in payload.items():(left/n).write_bytes(b)
 archive=root/'sample.zip'
 with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED) as z:
  for n,b in payload.items():z.writestr(n,b)
 original=sha(archive);big=root/'cancel-source.bin'
 with big.open('wb') as f:
  for _ in range(128):f.write(os.urandom(1024*1024))
 bigsha=sha(big);profile=root/'webview';local=root/'local';local.mkdir()
 with socket.socket() as port_socket:
  port_socket.bind(('127.0.0.1',0));debug_port=port_socket.getsockname()[1]
 env=dict(os.environ,LOCALAPPDATA=str(local),WEBVIEW2_USER_DATA_FOLDER=str(profile),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f'--remote-debugging-port={debug_port} --remote-debugging-address=127.0.0.1')
 startup=subprocess.STARTUPINFO();startup.dwFlags|=subprocess.STARTF_USESHOWWINDOW;startup.wShowWindow=0
 # Elevated hosted runner ignores user-scoped WebView debug flags. Use the
 # documented per-executable HKLM test override only on this disposable runner.
 # https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security
 assert os.environ.get('GITHUB_ACTIONS')=='true' and os.environ.get('RUNNER_ARCH')=='ARM64'
 policy_path=r'SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments'
 policy=winreg.CreateKeyEx(winreg.HKEY_LOCAL_MACHINE,policy_path,0,winreg.KEY_QUERY_VALUE|winreg.KEY_SET_VALUE|winreg.KEY_WOW64_64KEY)
 try:old_policy=winreg.QueryValueEx(policy,exe.name)
 except FileNotFoundError:old_policy=None
 winreg.SetValueEx(policy,exe.name,0,winreg.REG_SZ,env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'])
 evidence['hosted_debug_override']={'scope':'HKLM per executable, restored in finally; disposable hosted ARM runner only','name':exe.name,'port':debug_port,'prior_value_present':old_policy is not None}
 proc=subprocess.Popen([str(exe)],cwd=root,env=env,startupinfo=startup,stdout=(out/'process.stdout.log').open('wb'),stderr=(out/'process.stderr.log').open('wb'))
 evidence['pid']=proc.pid
 try:
  evidence['native_process']=native(w.HANDLE(proc._handle))
  until=time.monotonic()+60
  while time.monotonic()<until:
   assert proc.poll() is None,proc.returncode
   try:
    tabs=json.load(urllib.request.urlopen(f'http://127.0.0.1:{debug_port}/json/list',timeout=2));evidence['debug_targets']=tabs;tab=next((t for t in tabs if t.get('type')=='page' and 'tauri.localhost' in t['url']),None)
    if tab:break
   except OSError as error:evidence['debug_connection_error']=str(error)
   time.sleep(.2)
  else:
   evidence['profile_files']=[str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()][:150]
   evidence['process_diagnostics']=subprocess.check_output(['pwsh','-NoProfile','-Command',"Get-CimInstance Win32_Process | Where-Object {$_.Name -in 'qing-zip.exe','msedgewebview2.exe'} | Select-Object ProcessId,ParentProcessId,Name,CommandLine | ConvertTo-Json -Depth 3"],encoding='utf-8-sig')
   raise RuntimeError('Native WebView did not expose a document: '+str(evidence))
  # Confirm actual WebView process descendants, not an unrelated installed browser.
  command='Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath | ConvertTo-Json -Compress'
  processes=json.loads(subprocess.check_output(['pwsh','-NoProfile','-Command',command],encoding='utf-8-sig'));byid={p['ProcessId']:p for p in processes};web=[]
  k=ctypes.WinDLL('kernel32',use_last_error=True);k.OpenProcess.argtypes=[w.DWORD,w.BOOL,w.DWORD];k.OpenProcess.restype=w.HANDLE;k.CloseHandle.argtypes=[w.HANDLE]
  for p in processes:
   if p['Name'].lower()!='msedgewebview2.exe':continue
   parent=p['ParentProcessId'];seen=set()
   while parent in byid and parent not in seen and parent!=proc.pid:seen.add(parent);parent=byid[parent]['ParentProcessId']
   if parent==proc.pid:
    h=k.OpenProcess(0x1000,False,p['ProcessId']);assert h
    try:web.append(dict(pid=p['ProcessId'],path=p['ExecutablePath'],**native(h)))
    finally:k.CloseHandle(h)
  assert web,'No native WebView descendant';evidence['webview_processes']=web
  with connect(tab['webSocketDebuggerUrl'],proxy=None) as ws:
   seq=0
   def call(method,params):
    nonlocal seq
    seq+=1;ws.send(json.dumps({'id':seq,'method':method,'params':params}))
    while True:
     v=json.loads(ws.recv(timeout=40))
     if v.get('id')==seq:assert 'error' not in v,v;return v['result']
   def js(code):
    v=call('Runtime.evaluate',{'expression':code,'returnByValue':True,'awaitPromise':True});assert 'exceptionDetails' not in v,v;return v['result'].get('value')
   def wait(code,timeout=90):
    until=time.monotonic()+timeout
    while time.monotonic()<until:
     if js('Boolean('+code+')'):return
     time.sleep(.1)
    raise RuntimeError(code+'\n'+str(js('document.body.innerText')))
   def click(selector):
    s=json.dumps(selector);wait('document.querySelector('+s+')');box=js('(()=>{const r=document.querySelector('+s+').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2,r.width,r.height]})()');assert box[2]>0 and box[3]>0,(selector,box)
    for t in ['mousePressed','mouseReleased']:call('Input.dispatchMouseEvent',{'type':t,'x':box[0],'y':box[1],'button':'left','buttons':int(t=='mousePressed'),'clickCount':1})
   def key(name,code,vk,mod=0):
    for t in ['keyDown','keyUp']:call('Input.dispatchKeyEvent',{'type':t,'key':name,'code':code,'windowsVirtualKeyCode':vk,'modifiers':mod})
   def fill(selector,text):click(selector);key('a','KeyA',65,2);call('Input.insertText',{'text':str(text)})
   def choose(selector,value):
    index=js('Array.from(document.querySelector('+json.dumps(selector)+').options).findIndex(o=>o.value==='+json.dumps(value)+')');assert index>=0
    click(selector);key('Home','Home',36)
    for _ in range(index):key('ArrowDown','ArrowDown',40)
    key('Enter','Enter',13)
   def shot(name):(out/(name+'.png')).write_bytes(base64.b64decode(call('Page.captureScreenshot',{'format':'png'})['data']))
   def check(name):evidence['checks'].append(name);(out/'receipt.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2),encoding='utf-8')
   def finish():
    wait('document.querySelector("#qa-overlay .qa-jobcard")')
    wait('document.querySelector("#qa-status-text")?.innerText==="就绪" && document.querySelector("#qa-overlay .qa-dialog-footer [data-action=close]") && !document.querySelector("#qa-overlay .qa-dialog-footer [data-action=close]").disabled')
    assert '失败' not in js('document.querySelector("#qa-overlay").innerText')
    click('#qa-overlay .qa-dialog-footer [data-action=close]');wait('document.querySelector("#qa-overlay").hidden')
   try:
    wait('window.__TAURI__?.core && document.querySelector("[data-action=app-menu]")')
    evidence['version']=js("window.__TAURI__.core.invoke('app_version')");assert evidence['version']=='0.11.13'
    evidence['current_executable']=js("window.__TAURI__.core.invoke('current_executable')");assert Path(evidence['current_executable']).resolve()==exe
    time.sleep(.5)
    click('[data-action=app-menu]');click('[data-menu=file]');click('[data-action=open-path]');fill('#qa-open-path',archive);click('[data-action=open-submit]')
    wait('document.querySelectorAll("[data-row]").length===3');shot('01-open');check('real menu path input opens ZIP and lists all three Unicode/empty members')
    click('[data-action=extract]');fill('#qa-destination',dest)
    for sel in ['#qa-smart','#oa-open-after','#oa-close-after','#oa-recycle-after']:
     if js('document.querySelector('+json.dumps(sel)+').checked'):click(sel)
    click('[data-action=startextract]');finish()
    assert {p.name:sha(p) for p in dest.iterdir()}=={n:hashlib.sha256(b).hexdigest() for n,b in payload.items()};assert sha(archive)==original
    shot('02-extract');check('real extraction yields exact member SHA256 and preserves archive')
    key('n','KeyN',78,2);wait('document.querySelector("#qa-inputs")');click('#qa-overlay details summary');fill('#qa-inputs',left);created=root/'created.zip';fill('#qa-save',created);choose('#qa-format','zip');click('[data-action=startcompress]');finish()
    with zipfile.ZipFile(created) as z:
     assert z.testzip() is None
     files={Path(n).name:hashlib.sha256(z.read(n)).hexdigest() for n in z.namelist() if not n.endswith('/')}
     assert files=={n:hashlib.sha256(b).hexdigest() for n,b in payload.items()},files
    shot('03-create');check('real new ZIP creation independently decoded with exact SHA256 for all inputs')
    key('n','KeyN',78,2);wait('document.querySelector("#qa-inputs")');click('#qa-overlay details summary');fill('#qa-inputs',big);cancelout=root/'cancelled.7z';fill('#qa-save',cancelout);choose('#qa-format','7z');choose('#qa-level','9');click('[data-action=startcompress]')
    wait('document.querySelector("[data-action=canceljob]") && document.querySelector("#qa-status-text")?.innerText!=="就绪"')
    evidence['cancel_before']={'monotonic':time.monotonic(),'ui':js('document.querySelector("#qa-overlay").innerText'),'cancel_enabled':js('!document.querySelector("[data-action=canceljob]").disabled')};assert evidence['cancel_before']['cancel_enabled'];shot('04-before-cancel');click('[data-action=canceljob]')
    wait('document.querySelector("#qa-status-text")?.innerText==="就绪"');evidence['cancel_after']=js('document.querySelector("#qa-overlay").innerText');assert '已取消' in evidence['cancel_after'];assert not cancelout.exists() and sha(big)==bigsha;shot('05-cancelled');finish();check('cancel clicked while actual compression is active; cancelled terminal, no output, source SHA unchanged')
    click('[data-view=tools]');click('[data-view=filemanager]');wait('document.querySelectorAll(".fm-pane").length===2')
    for index,path in [(0,left),(1,right)]:
     fill(f'[data-fm-path="{index}"] input',path);click(f'[data-fm=go][data-pane="{index}"]');wait('!document.querySelector(".fm-workspace").innerText.includes("正在读取")')
    click('[data-fm-pane="0"] [aria-label="选择 hello.txt"]');click('.fm-transfer [data-fm=copy]');finish();assert sha(right/'hello.txt')==sha(left/'hello.txt')==hashlib.sha256(payload['hello.txt']).hexdigest();shot('06-file-copy');check('dual-location paths entered and selected file copied with SHA/source retention verified')
    assert sha(archive)==original and sha(big)==bigsha
    evidence['passed']=True
   except Exception:
    evidence['last_ui']=js('document.body.innerText');shot('failure');raise
 finally:
  if proc.poll() is None:proc.terminate()
  proc.wait(timeout=15);evidence['process_terminal']=proc.returncode
  if old_policy is None:winreg.DeleteValue(policy,exe.name)
  else:winreg.SetValueEx(policy,exe.name,0,old_policy[1],old_policy[0])
  winreg.CloseKey(policy);evidence['debug_override_restored']=True
  (out/'receipt.json').write_text(json.dumps(evidence,ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':main()
