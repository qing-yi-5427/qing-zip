from pathlib import Path
exec(compile(Path(__file__).with_name('efs-oplock-ci-probe.py').read_text(encoding='utf-8-sig').split('\ntry:\n',1)[0],'defs','exec'))
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-oplock-native-result.json'
class Unicode(C.Structure):
 _fields_=[('length',W.WORD),('maximum',W.WORD),('buffer',C.c_void_p)]
class Attributes(C.Structure):
 _fields_=[('length',W.ULONG),('root',W.HANDLE),('name',C.POINTER(Unicode)),('flags',W.ULONG),('sd',C.c_void_p),('qos',C.c_void_p)]
N.NtCreateFile.argtypes=[C.POINTER(W.HANDLE),W.DWORD,C.POINTER(Attributes),C.POINTER(IOSB),C.c_void_p,W.DWORD,W.DWORD,W.DWORD,W.DWORD,C.c_void_p,W.DWORD];N.NtCreateFile.restype=C.c_long

def native_writer(p):
 text='\\??\\'+str(p);buf=C.create_unicode_buffer(text);name=Unicode(len(text)*2,len(text)*2,C.cast(buf,C.c_void_p));attrs=Attributes(C.sizeof(Attributes),None,C.pointer(name),0,None,None);io=IOSB();h=W.HANDLE()
 status=N.NtCreateFile(C.byref(h),0x18,C.byref(attrs),C.byref(io),None,0,7,1,0x200100,None,0)
 return h.value,status&0xffffffff

def experiment(p,before):
 p.write_bytes(b'original');Path(str(p)+':ads').write_bytes(b'alternate');require(A.EncryptFileW(str(p)),'encrypt');ea(p,b'old')
 v,info=lease(p,7);require(v,'main RWH unavailable');adsv,adsinfo=lease(str(p)+':ads',7);require(adsv,'ADS RWH unavailable')
 writer=None;pending=False;r={'native_before_pending':before,'main':info,'ads':adsinfo}
 try:
  if before:
   writer,status=native_writer(p);r['native_status']=hex(status)
   r['main_event_after_native']=K.WaitForSingleObject(v[1],0)
  flag=W.BOOL(True);ok=K.SetFileInformationByHandle(v[0],4,C.byref(flag),C.sizeof(flag));pending=bool(ok);r['set_pending']={'ok':bool(ok),'error':0 if ok else C.get_last_error()}
  if not before:
   writer,status=native_writer(p);r['native_status']=hex(status)
  r['main_event_after_pending']=K.WaitForSingleObject(v[1],0);r['ads_event_after_pending']=K.WaitForSingleObject(adsv[1],0)
  if writer:
   name=b'QING.EFS.TEST';value=b'native changed';data=b'\0'*4+bytes([0x80,len(name)])+len(value).to_bytes(2,'little')+name+b'\0'+value;b=C.create_string_buffer(data);io=IOSB()
   r['native_ea_write_status']=hex(N.NtSetEaFile(writer,C.byref(io),b,len(data))&0xffffffff)
 finally:
  if pending:
   flag=W.BOOL(False);require(K.SetFileInformationByHandle(v[0],4,C.byref(flag),C.sizeof(flag)),'undo failed')
  if writer:K.CloseHandle(writer)
  closelease(adsv);closelease(v)
  require(p.exists(),'source lost');r['source_preserved']=True
 return r
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-native-gate-') as tmp:
  receipt['cases']=[experiment(Path(tmp)/str(before),before) for before in [False,True]]
 receipt['status']='observed'
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
