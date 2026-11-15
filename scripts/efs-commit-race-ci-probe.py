from pathlib import Path
exec(compile(Path(__file__).with_name('efs-oplock-native-ci-probe.py').read_text(encoding='utf-8-sig').split('\ntry:\n',1)[0],'defs','exec'))
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-commit-race-result.json'
K.ReadFile.argtypes=[W.HANDLE,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.POINTER(Overlap)];K.ReadFile.restype=W.BOOL
K.GetFileSizeEx.argtypes=[W.HANDLE,C.POINTER(C.c_longlong)];K.GetFileSizeEx.restype=W.BOOL

def read_pin(v):
 h=v[0];size=C.c_longlong();require(K.GetFileSizeEx(h,C.byref(size)),'size failed');require(0<=size.value<=1048576,'fixture size')
 if not size.value:return b''
 b=C.create_string_buffer(size.value);count=W.DWORD();ov=Overlap();ov.event=K.CreateEventW(None,True,False,None)
 try:
  ok=K.ReadFile(h,b,size.value,C.byref(count),C.byref(ov))
  if not ok:require(C.get_last_error()==997 and K.GetOverlappedResult(h,C.byref(ov),C.byref(count),True),'read failed')
  require(count.value==size.value,'short read');return b.raw[:count.value]
 finally:K.CloseHandle(ov.event)
def ea_pin(v):
 b=C.create_string_buffer(65536);io=IOSB();status=N.NtQueryEaFile(v[0],C.byref(io),b,len(b),False,None,0,None,True)&0xffffffff
 if status==259:
  require(K.WaitForSingleObject(v[0],5000)==0,'EA pending timeout');status=io.status&0xffffffff
 require(status==0,f'EA query {status:#x}');return b.raw[:io.information]
def pending(v,value):
 flag=W.BOOL(value);ok=K.SetFileInformationByHandle(v[0],4,C.byref(flag),C.sizeof(flag));require(ok,f'pending {value}: {C.get_last_error()}')
def snapshot(v,ads):return [read_pin(v).hex(),read_pin(ads).hex(),ea_pin(v).hex()]
def mutate_ea(h):
 name=b'QING.EFS.TEST';value=b'changed after final target check';data=b'\0'*4+bytes([0x80,len(name)])+len(value).to_bytes(2,'little')+name+b'\0'+value;b=C.create_string_buffer(data);io=IOSB();return N.NtSetEaFile(h,C.byref(io),b,len(data))&0xffffffff

def run(base,scenario):
 base.mkdir();s=base/'source';t=base/'target';s.write_bytes(b'primary payload');Path(str(s)+':ads').write_bytes(b'alternate payload');require(A.EncryptFileW(str(s)),'encrypt');ea(s,b'original ea');original_keys=keys(s)
 require(copy(s,t)[0],'copy');ea(t,b'original ea');require(keys(t)==original_keys,'target keys differ')
 sv=sa=tv=ta=None;writer=None;is_pending=False;committed=False;r={'scenario':scenario}
 try:
  sv,_=lease(s,7);sa,_=lease(str(s)+':ads',7);tv,_=lease(t,7);ta,_=lease(str(t)+':ads',7);require(all([sv,sa,tv,ta]),'lease unavailable')
  before=snapshot(sv,sa);require(before==snapshot(tv,ta),'initial full fixture proof mismatch')
  require(keys(s)==keys(t)==original_keys,'key snapshot mismatch')
  if scenario=='cancel_before_pending':return r
  pending(sv,True);is_pending=True
  r['source_pending_full_handle_read_matches']=snapshot(sv,sa)==before
  require(r['source_pending_full_handle_read_matches'],'pending handle proof mismatch')
  require(before==snapshot(tv,ta),'target changed')
  r['last_target_check_unbroken']=all(K.WaitForSingleObject(v[1],0)==258 for v in [tv,ta])
  require(r['last_target_check_unbroken'],'target already broken')
  if scenario=='target_native_after_last_check':
   writer,status=native_writer(t);r['native_status']=hex(status);require(writer,'native writer denied');r['native_ea_write_status']=hex(mutate_ea(writer));r['target_event_after_write']=K.WaitForSingleObject(tv[1],0)
  if scenario in ['cancel_after_pending','read_proof_failure']:return r
  # Deliberately reproduce the proposed final-check/close gap ONLY on generated
  # disposable fixtures. This is a negative experiment, never production logic.
  closelease(sa);sa=None;closelease(sv);sv=None;is_pending=False;committed=True
  r['source_deleted']=not s.exists();r['target_changed_after_check']=snapshot(tv,ta)!=before
  return r
 finally:
  if is_pending:
   pending(sv,False);r['rollback_succeeded']=True
  if writer:K.CloseHandle(writer)
  for v in [ta,tv,sa,sv]:closelease(v)
  r['source_preserved']=s.exists()
  if not committed:
   require(s.exists() and s.read_bytes()==b'primary payload' and keys(s)==original_keys,'rollback source mismatch')
  r['target_exists']=t.exists()
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-commit-race-') as temp:
  receipt['cases']=[run(Path(temp)/s,s) for s in ['cancel_before_pending','cancel_after_pending','read_proof_failure','clean_commit','target_native_after_last_check']]
 receipt['status']='observed';receipt['production_delete_authorized']=False
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
