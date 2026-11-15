"""Observe native ARM64 GUI/Explorer/WebView descendants; never controls operations."""
import ctypes as c,json,subprocess
from ctypes import wintypes as w
def verify(app_pid,explorer_hwnd):
 k=c.WinDLL('kernel32',use_last_error=True);u=c.WinDLL('user32',use_last_error=True)
 k.OpenProcess.argtypes=[w.DWORD,w.BOOL,w.DWORD];k.OpenProcess.restype=w.HANDLE
 k.CloseHandle.argtypes=[w.HANDLE];k.CloseHandle.restype=w.BOOL
 k.IsWow64Process2.argtypes=[w.HANDLE,c.POINTER(w.WORD),c.POINTER(w.WORD)];k.IsWow64Process2.restype=w.BOOL
 u.GetWindowThreadProcessId.argtypes=[w.HWND,c.POINTER(w.DWORD)];u.GetWindowThreadProcessId.restype=w.DWORD
 def native(pid):
  h=k.OpenProcess(0x1000,False,pid);assert h,(pid,c.get_last_error())
  try:
   process,host=w.WORD(),w.WORD();assert k.IsWow64Process2(h,c.byref(process),c.byref(host)),c.get_last_error()
   assert process.value==0 and host.value==0xaa64,(pid,process.value,host.value)
   return {'pid':pid,'process_machine':process.value,'native_machine':host.value}
  finally:k.CloseHandle(h)
 explorer_pid=w.DWORD();assert u.GetWindowThreadProcessId(explorer_hwnd,c.byref(explorer_pid))
 report={'native_gui':native(app_pid),'native_explorer':native(explorer_pid.value)}
 command='Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath | ConvertTo-Json -Compress'
 processes=json.loads(subprocess.check_output(['pwsh','-NoProfile','-Command',command],encoding='utf-8-sig'));byid={p['ProcessId']:p for p in processes};web=[]
 for item in processes:
  if item['Name'].lower()!='msedgewebview2.exe':continue
  parent=item['ParentProcessId'];seen=set()
  while parent in byid and parent not in seen and parent!=app_pid:
   seen.add(parent);parent=byid[parent]['ParentProcessId']
  if parent==app_pid:web.append(dict(path=item['ExecutablePath'],**native(item['ProcessId'])))
 assert web,'No actual native WebView descendant'
 report['native_webview_processes']=web
 return report
