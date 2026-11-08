from pathlib import Path
import tempfile,sys,os,ctypes as c,ctypes.wintypes as w,json,zipfile,subprocess
here=Path(__file__).resolve().parent
ns={'__file__':str(here/'check-move-tree-acl.py')}
exec((here/'check-move-tree-acl.py').read_text(encoding='utf-8-sig').split("\nexe=Path(sys.argv[1])")[0],ns)
globals().update(ns)
exe=Path(sys.argv[1]).resolve();reports=Path(sys.argv[2]).resolve();reports.mkdir(exist_ok=True,parents=True)
results=[]
for foreign in (False,True):
 with tempfile.TemporaryDirectory() as tmp:
  root=Path(tmp); model(root,True);archive=root/'case.zip'
  with zipfile.ZipFile(archive,'w') as z:
   for name in ('good','bad'):
    z.writestr(name+'/',b'')
    for i in range(16):z.writestr(f'{name}/f{i:03}.txt',b'our contents')
  dest=root/'output';bad=dest/'bad'
  def inject():
   if foreign:
    (dest/'foreign.txt').write_bytes(b'outside')
    (dest/'foreign empty folder').mkdir()
    e=bad/'f000.txt';assert e.is_file()
    stream=dbg.func(dbg.k,'CreateFileW',dbg.P,w.LPCWSTR,w.DWORD,w.DWORD,dbg.P,w.DWORD,w.DWORD,dbg.P)(str(e)+':user-note',0x40000000,7,None,1,0,None)
    assert stream!=c.c_void_p(-1).value,c.get_last_error()
    try:
     n=w.DWORD();assert dbg.k.WriteFile(dbg.P(stream),b'user change',11,c.byref(n),None) and n.value==11
    finally:dbg.close(stream)
   # Alter just this directory, without propagating replacement ACLs to files.
   sd=c.c_void_p();text=f'D:P(D;;0x0002;;;{who})(A;OICI;FA;;;{who})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)'
   assert adv.ConvertStringSecurityDescriptorToSecurityDescriptorW(text,1,c.byref(sd),None)
   h=dbg.func(dbg.k,'CreateFileW',dbg.P,w.LPCWSTR,w.DWORD,w.DWORD,dbg.P,w.DWORD,w.DWORD,dbg.P)(str(bad),0x60080,7,None,3,0x02200000,None)
   nt=c.WinDLL('ntdll').NtSetSecurityObject;nt.argtypes=[dbg.P,w.DWORD,dbg.P];nt.restype=c.c_int32
   try:assert nt(h,0x80000004,sd)>=0
   finally:dbg.close(h);k.LocalFree(sd)
  def match(path,new):return bool(new) and Path(new).name=='f005.txt' and Path(path).parent.name=='bad'
  hit,rc,err=dbg.observe([exe,'extract',archive,dest,'--overwrite','overwrite'],match,inject,{**os.environ,'OPEN_ARCHIVE_PROFILE':'1'},'NtSetInformationFile')
  paths=sorted(x.relative_to(dest).as_posix() for x in dest.rglob('*'))
  result=dict(foreign=foreign,stopped_at=hit,status=rc,paths=paths,error=err)
  results.append(result);print(result,flush=True)
  (reports/'permission-rollback.json').write_text(json.dumps(results,indent=2))
  assert hit and rc and 'partial destination retained' in err
  expected=['bad']+(['foreign empty folder','foreign.txt','bad/f000.txt'] if foreign else [])
  assert paths==sorted(expected),paths
  if foreign:
   assert (dest/'foreign.txt').read_bytes()==b'outside' and Path(str(bad/'f000.txt')+':user-note').read_bytes()==b'user change'
  apply_acl(bad,True)
