import ctypes,os,subprocess,tempfile,time
from pathlib import Path
from win32com.client import Dispatch
from pywinauto import mouse
assert os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted'
u=ctypes.windll.user32;u.SetProcessDPIAware()
assert not u.FindWindowW('QingSignalFixture',None)
def wait(f,timeout=20):
 end=time.monotonic()+timeout
 while time.monotonic()<end:
  hit=f()
  if hit:return hit
  time.sleep(.1)
 raise TimeoutError()
with tempfile.TemporaryDirectory(dir=os.environ['RUNNER_TEMP'],ignore_cleanup_errors=True) as td:
 root=Path(td);dest=root/'Output';dest.mkdir()
 shell=Dispatch('Shell.Application');shell.Open(str(dest))
 def target():
  for w in shell.Windows():
   try:
    if os.path.normcase(w.Document.Folder.Self.Path)==os.path.normcase(str(dest)):return w
   except Exception:pass
 win=wait(target)
 try:
  target=int(win.HWND);u.ShowWindow(target,9);u.SetWindowPos(target,-1,290,0,700,650,0x40)
  for size in [32,64*1024*1024]:
   source=root/f'{size}.bin';source.write_bytes(b'T'*size)
   p=subprocess.Popen(['target/probe/Release/probe.exe',str(source)],stdout=subprocess.PIPE)
   try:
    hwnd=wait(lambda:u.FindWindowW('QingSignalFixture',None));u.SetForegroundWindow(hwnd)
    mouse.press(coords=(120,130));time.sleep(.3)
    for step in range(1,16):mouse.move(coords=(120+step*47,130+step*17));time.sleep(.08)
    time.sleep(.35);mouse.release(coords=(825,385))
    logs=p.communicate(timeout=15)[0].decode()
    print(size,logs,flush=True)
    copied=dest/source.name;wait(lambda:copied.exists() and copied.stat().st_size==size)
    assert copied.read_bytes()==source.read_bytes()
   finally:
    mouse.release()
    if p.poll() is None:p.kill();p.wait()
 finally:win.Quit()
