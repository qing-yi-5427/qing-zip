"""Exercise a live destination ACL change through the normal CLI using disposable files only."""
from pathlib import Path
import ctypes as c,ctypes.wintypes as w,sys,json,os,tempfile,hashlib,subprocess,time,csv,tarfile
adv=c.WinDLL('advapi32',use_last_error=True);k=c.WinDLL('kernel32',use_last_error=True)
adv.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes=[w.LPCWSTR,w.DWORD,c.POINTER(c.c_void_p),c.POINTER(w.DWORD)]
adv.GetSecurityDescriptorDacl.argtypes=[c.c_void_p,c.POINTER(w.BOOL),c.POINTER(c.c_void_p),c.POINTER(w.BOOL)]
adv.SetNamedSecurityInfoW.argtypes=[w.LPWSTR,w.DWORD,w.DWORD,c.c_void_p,c.c_void_p,c.c_void_p,c.c_void_p]
k.LocalFree.argtypes=[c.c_void_p]
who=next(csv.reader(subprocess.check_output(['whoami','/user','/fo','csv','/nh']).decode(errors='replace').splitlines()))[1];assert who.startswith('S-1-')
def apply_acl(path,allow):
 text='D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;'+who+')'+('(A;OICI;FR;;;LS)' if allow else '')
 sd=c.c_void_p();assert adv.ConvertStringSecurityDescriptorToSecurityDescriptorW(text,1,c.byref(sd),None)
 try:
  ok=w.BOOL();default=w.BOOL();acl=c.c_void_p();assert adv.GetSecurityDescriptorDacl(sd,c.byref(ok),c.byref(acl),c.byref(default))
  e=adv.SetNamedSecurityInfoW(str(path),1,0x80000004,None,None,acl,None);assert not e,(path,e)
 finally:k.LocalFree(sd)
def grants_ls(path):
 sd=c.c_void_p();s=c.c_wchar_p()
 adv.GetNamedSecurityInfoW.argtypes=[w.LPWSTR,w.DWORD,w.DWORD,c.c_void_p,c.c_void_p,c.c_void_p,c.c_void_p,c.POINTER(c.c_void_p)]
 e=adv.GetNamedSecurityInfoW(str(path),1,4,None,None,None,None,c.byref(sd));assert not e,e
 try:
  assert adv.ConvertSecurityDescriptorToStringSecurityDescriptorW(sd,1,4,c.byref(s),None)
  sddl=s.value
 finally:
  if s:k.LocalFree(c.cast(s,c.c_void_p))
  k.LocalFree(sd)
 return ';;;LS)' in sddl or ';;;S-1-5-19)' in sddl

exe=Path(sys.argv[1]).resolve();result=Path(sys.argv[2]);result.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory() as tmp:
 d=Path(tmp);src=d/'source.bin';payload=hashlib.shake_256(b'qzip security race').digest(1<<20)
 with src.open('wb') as f:
  for i in range(160):f.write(payload)
 archive=d/'probe.tar'
 with tarfile.open(archive,'w') as t:t.add(src,arcname='output.bin')
 tar_sha=hashlib.sha256(archive.read_bytes()).hexdigest();rows=[]
 for mode in ['error','overwrite']:
  dest=d/mode;dest.mkdir();apply_acl(dest,True);out=dest/'output.bin'
  if mode=='overwrite':out.write_bytes(b'original')
  p=subprocess.Popen([exe,'extract',archive,dest,'--overwrite',mode],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
  try:
   t0=time.perf_counter();scratch=None
   while time.perf_counter()-t0<60 and p.poll() is None:
    scratch=next(dest.glob('.qing-zip-output-*'),None)
    if scratch:break
    time.sleep(.001)
   assert scratch,(mode,'did not observe a private output')
   apply_acl(dest,False)
   wtn=dest/'ordinary-reference.txt';wtn.write_bytes(b'now')
   stdout,stderr=p.communicate(timeout=90)
   rows.append({'mode':mode,'exit':p.returncode,'output_len':out.stat().st_size if out.exists() else None,
    'parent_read_grant':grants_ls(dest),'ordinary_child_read_grant':grants_ls(wtn),'output_read_grant':grants_ls(out) if out.exists() else None,
    'leftovers':len(list(dest.glob('.qing-zip-output-*'))),'error':stderr.decode(errors='replace')[-600:]})
  finally:
   if p.poll() is None:p.kill();p.wait()
 result.joinpath('acl-race.json').write_text(json.dumps({'exe':hashlib.sha256(exe.read_bytes()).hexdigest(),'archive':tar_sha,'rows':rows},indent=2));print(rows)
 assert all(not x['parent_read_grant'] and not x['ordinary_child_read_grant'] for x in rows)
