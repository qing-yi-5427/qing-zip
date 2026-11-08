import tempfile,ctypes as c,os
from ctypes import wintypes as w
from pathlib import Path
k=c.WinDLL('kernel32',use_last_error=True);k.ReplaceFileW.argtypes=[w.LPCWSTR,w.LPCWSTR,w.LPCWSTR,w.DWORD,w.LPVOID,w.LPVOID]
with tempfile.TemporaryDirectory() as t:
 d=Path(t);other=d/'other';other.write_bytes(b'outside');link=d/'link';src=d/'proposed';src.write_bytes(b'qing');backup=d/'backup'
 try:link.symlink_to(other)
 except OSError as e:print(e);quit()
 r=k.ReplaceFileW(str(link),str(src),str(backup),0,None,None); print(r,c.get_last_error(),[(p.name,p.is_symlink(),p.read_bytes() if p.exists() else None) for p in d.iterdir()])
