from pathlib import Path
exec(compile(Path(__file__).with_name('efs-ci-probe.py').read_text(encoding='utf-8').split('\ntry:\n',1)[0],'defs','exec'))
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-oplock-result.json'
class Overlap(C.Structure):
 _fields_=[('internal',C.c_size_t),('high',C.c_size_t),('offset',W.DWORD),('offset_high',W.DWORD),('event',W.HANDLE)]
class Input(C.Structure):
 _fields_=[('version',W.WORD),('length',W.WORD),('level',W.DWORD),('flags',W.DWORD)]
class Output(C.Structure):
 _fields_=[('version',W.WORD),('length',W.WORD),('original',W.DWORD),('new',W.DWORD),('flags',W.DWORD),('access',W.DWORD),('share',W.WORD)]
K.CreateEventW.argtypes=[C.c_void_p,W.BOOL,W.BOOL,W.LPCWSTR];K.CreateEventW.restype=W.HANDLE
K.DeviceIoControl.argtypes=[W.HANDLE,W.DWORD,C.c_void_p,W.DWORD,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.POINTER(Overlap)];K.DeviceIoControl.restype=W.BOOL
K.CancelIoEx.argtypes=[W.HANDLE,C.POINTER(Overlap)];K.CancelIoEx.restype=W.BOOL
K.GetOverlappedResult.argtypes=[W.HANDLE,C.POINTER(Overlap),C.POINTER(W.DWORD),W.BOOL];K.GetOverlappedResult.restype=W.BOOL
K.WaitForSingleObject.argtypes=[W.HANDLE,W.DWORD];K.WaitForSingleObject.restype=W.DWORD
K.SetFileInformationByHandle.argtypes=[W.HANDLE,C.c_int,C.c_void_p,W.DWORD];K.SetFileInformationByHandle.restype=W.BOOL

def openh(p,access,share=7,async_io=False):
 h=K.CreateFileW(str(p),access,share,None,3,0x02200000|(0x40000000 if async_io else 0),None)
 return (None,C.get_last_error()) if h in (None,C.c_void_p(-1).value) else (h,0)
def lease(p,level):
 h,e=openh(p,0x80010000,7,True)
 if h is None:return None,{'open_error':e}
 event=K.CreateEventW(None,True,False,None);ov=Overlap();ov.event=event
 inp=Input(1,C.sizeof(Input),level,1);output=Output();count=W.DWORD()
 ok=K.DeviceIoControl(h,0x90240,C.byref(inp),C.sizeof(inp),C.byref(output),C.sizeof(output),C.byref(count),C.byref(ov))
 err=0 if ok else C.get_last_error()
 if err!=997:
  K.CloseHandle(event);K.CloseHandle(h);return None,{'request_error':err,'immediate':bool(ok)}
 return (h,event,ov,inp,output),{'pending':True,'event':K.WaitForSingleObject(event,0)}
def closelease(v):
 if v:
  h,event,ov,_,_=v;K.CancelIoEx(h,C.byref(ov));count=W.DWORD();K.GetOverlappedResult(h,C.byref(ov),C.byref(count),True);K.CloseHandle(h);K.CloseHandle(event)
def run(p,directory,old,level):
 if directory:p.mkdir()
 else:p.write_bytes(b'original')
 Path(str(p)+':ads').write_bytes(b'alternate')
 require(A.EncryptFileW(str(p)),f'encrypt {C.get_last_error()}');ea(p,b'old')
 writer=None;v=None;adsv=None;pending=False;r={'directory':directory,'old_writer':old,'level':level}
 try:
  if old!='none':writer,e=openh(str(p)+':ads' if old=='ads' else p,0x40000000 if old=='ads' else 0x18);r['writer_open_error']=e
  v,r['main']=lease(p,level)
  if v:
   adsv,r['ads']=lease(str(p)+':ads',7)
   flag=W.BOOL(True);ok=K.SetFileInformationByHandle(v[0],4,C.byref(flag),C.sizeof(flag));pending=bool(ok)
   r['pending']={'ok':bool(ok),'error':0 if ok else C.get_last_error()}
   r['main_event_after_pending']=K.WaitForSingleObject(v[1],0)
   if adsv:r['ads_event_after_pending']=K.WaitForSingleObject(adsv[1],0)
 finally:
  if pending:
   flag=W.BOOL(False);require(K.SetFileInformationByHandle(v[0],4,C.byref(flag),C.sizeof(flag)),'undo failed')
  closelease(adsv);closelease(v)
  if writer:K.CloseHandle(writer)
  require(p.exists(),'fixture lost')
 return r
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-oplock-') as tmp:
  receipt['cases']=[]
  for d in [False,True]:
   for old in ['none','ea','ads']:
    for level in ([7] if not d else [7,5]):
     receipt['cases'].append(run(Path(tmp)/f'{d}-{old}-{level}',d,old,level))
 receipt['status']='observed'
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
