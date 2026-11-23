"""Enable the real GUI file-compression switch in a disposable hosted profile."""
import json, os, subprocess, sys, time, urllib.request
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from arm_gui_lifecycle import hosted_debug_session
from websockets.sync.client import connect
exe=Path(sys.argv[1]).resolve();root=Path(sys.argv[2]);receipt=Path('explorer-evidence/settings-session.json')
with hosted_debug_session(exe,receipt) as debug:
 env=dict(os.environ,LOCALAPPDATA=str(root/'app-local'),WEBVIEW2_USER_DATA_FOLDER=str(root/'app-webview'))
 proc=subprocess.Popen([str(exe)],cwd=root,env=env);debug['proc']=proc;ws=None
 for _ in range(400):
  if proc.poll() is not None:raise RuntimeError('GUI exited during settings')
  try:
   pages=json.load(urllib.request.urlopen(f"http://127.0.0.1:{debug['port']}/json",timeout=1))
   ws=connect(next(x['webSocketDebuggerUrl'] for x in pages if x.get('type')=='page' and 'tauri.localhost' in x.get('url','')),max_size=10_000_000);break
  except Exception:time.sleep(.1)
 if not ws:raise RuntimeError('GUI debug endpoint unavailable')
 seq=0
 def js(expr,promise=False):
  global seq
  seq+=1;ws.send(json.dumps({'id':seq,'method':'Runtime.evaluate','params':{'expression':expr,'returnByValue':True,'awaitPromise':promise}}))
  while True:
   r=json.loads(ws.recv())
   if r.get('id')==seq:
    if 'error' in r or 'exceptionDetails' in r.get('result',{}):raise RuntimeError(str(r))
    return r['result']['result'].get('value')
 for _ in range(300):
  if js("!!window.__TAURI__?.core && !!document.querySelector('[data-action=app-menu]')"):break
  time.sleep(.1)
 else:raise RuntimeError('GUI not ready')
 for _ in range(150):
  if js("!!document.querySelector('#oa-file-menu')"):break
  js("document.querySelector('[data-view=settings]').click()")
  time.sleep(.2)
 else:raise RuntimeError('Settings page missing')
 js("document.querySelector('#oa-file-menu').click()")
 for _ in range(100):
  if js("window.__TAURI__.core.invoke('file_compression_menu_enabled')",True):break
  time.sleep(.1)
 else:raise RuntimeError('Product switch failed to enable')
 Path('explorer-evidence/product-settings.json').write_text(json.dumps({'file_compression_enabled':True,'method':'actual GUI settings switch'},indent=2),encoding='utf8')
 ws.close()
