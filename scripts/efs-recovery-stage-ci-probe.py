from pathlib import Path
exec(compile(Path(__file__).with_name('efs-commit-race-ci-probe.py').read_text(encoding='utf-8-sig').split('\ntry:\n',1)[0],'defs','exec'))
import sys
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-recovery-stage-result.json'
class Rename(C.Structure):
 _fields_=[('replace',W.BOOL),('root',W.HANDLE),('length',W.DWORD),('name',W.WCHAR*1)]
def rename_pin(v,path):
 name=str(path).encode('utf-16-le');b=C.create_string_buffer(Rename.name.offset+len(name));r=Rename.from_buffer(b);r.replace=False;r.root=None;r.length=len(name);C.memmove(C.addressof(b)+Rename.name.offset,name,len(name));ok=K.SetFileInformationByHandle(v[0],3,b,len(b));require(ok,f'rename original handle failed {C.get_last_error()}')
def durable_json(p,data):
 with p.open('w',encoding='utf-8') as f:json.dump(data,f);f.flush();os.fsync(f.fileno())
def child(base,scenario):
 s=base/'source';t=base/'target';recovery=base/'recovery'/'captured';sv=sa=tv=ta=None;captured=False;target_pending=False;committed=False
 try:
  sv,_=lease(s,7);sa,_=lease(str(s)+':ads',7);tv,_=lease(t,7);ta,_=lease(str(t)+':ads',7);require(all([sv,sa,tv,ta]),'leases unavailable')
  proof=snapshot(sv,sa);require(proof==snapshot(tv,ta),'copy proof mismatch');certs=keys(s);require(certs==keys(t),'cert mismatch')
  durable_json(base/'journal.json',{'source':str(s),'recovery':str(recovery),'target':str(t),'proof':proof,'state':'prepared'})
  pending(tv,True);target_pending=True
  require(all(K.WaitForSingleObject(v[1],0)==258 for v in [sv,sa,tv,ta]),'lease broke before capture')
  rename_pin(sv,recovery);captured=True
  require(proof==snapshot(sv,sa)==snapshot(tv,ta),'post-capture proof mismatch')
  if scenario=='crash_before_publish':os._exit(73)
  if scenario=='lost_target_handle':
   closelease(ta);ta=None;closelease(tv);tv=None;target_pending=False
   flag=W.BOOL(False);ok=K.SetFileInformationByHandle(W.HANDLE(-1),4,C.byref(flag),C.sizeof(flag));require(not ok and C.get_last_error()==6,'expected lost-handle error')
   durable_json(base/'failure.json',{'clear_pending_error':6});return
  pending(tv,False);target_pending=False;committed=True
  if scenario=='crash_after_publish':os._exit(74)
  durable_json(base/'journal.json',{'source':str(s),'recovery':str(recovery),'target':str(t),'proof':proof,'state':'published'})
  # Retain the captured source in this feasibility probe. No cleanup algorithm
  # is implemented until recovery-state durability and namespace policy exist.
 finally:
  if target_pending and tv:pending(tv,False)
  for v in [ta,tv,sa,sv]:closelease(v)
  if captured and not committed and recovery.exists() and not s.exists():os.rename(recovery,s)
if len(sys.argv)>1 and sys.argv[1]=='child':
 child(Path(sys.argv[2]),sys.argv[3]);raise SystemExit(0)
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-recovery-stage-') as temp:
  receipt['cases']=[]
  for scenario in ['normal_publish','lost_target_handle','crash_before_publish','crash_after_publish']:
   base=Path(temp)/scenario;base.mkdir();(base/'recovery').mkdir();s=base/'source';t=base/'target';recovery=base/'recovery'/'captured';s.write_bytes(b'primary payload');Path(str(s)+':ads').write_bytes(b'alternate payload');require(A.EncryptFileW(str(s)),'encrypt');ea(s,b'original ea');expected_keys=keys(s);expected_ea=ea(s)
   require(copy(s,t)[0],'copy');ea(t,b'original ea')
   proc=subprocess.run([sys.executable,__file__,'child',str(base),scenario],capture_output=True,text=True,timeout=30)
   r={'scenario':scenario,'exit':proc.returncode,'stderr':proc.stderr,'source':s.exists(),'target':t.exists(),'recovery':recovery.exists(),'journal':json.loads((base/'journal.json').read_text()) if (base/'journal.json').exists() else None}
   require(proc.returncode in [0,73,74],'child failed')
   survivor=s if s.exists() else recovery
   require(survivor.exists(),'no recovery source');require(survivor.read_bytes()==b'primary payload' and Path(str(survivor)+':ads').read_bytes()==b'alternate payload' and ea(survivor)==expected_ea and keys(survivor)==expected_keys,'recovery object proof failed');r['recovery_proof_matches']=True
   if scenario=='crash_before_publish':require(not t.exists(),'pending target unexpectedly survived')
   if scenario=='crash_after_publish':require(t.exists(),'published target lost')
   receipt['cases'].append(r)
 receipt['status']='observed';receipt['power_loss_recovery_proven']=False;receipt['production_delete_authorized']=False
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
