from pathlib import Path
exec(compile(Path(__file__).with_name('efs-oplock-ci-probe.py').read_text(encoding='utf-8-sig').split('\ntry:\n',1)[0],'defs','exec'))
import threading
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-oplock-keys-result.json'
def query(p,recovery):
 p.write_bytes(b'original');Path(str(p)+':ads').write_bytes(b'alternate');require(A.EncryptFileW(str(p)),'encrypt');expected=keys(p,recovery)
 v,info=lease(p,7);require(v,'lease failed');adsv,adsinfo=lease(str(p)+':ads',7);require(adsv,'ADS lease failed');result={}
 def worker():
  try:result['keys']=[x.hex() for x in keys(p,recovery)]
  except Exception as exc:result['error']=repr(exc)
 thread=threading.Thread(target=worker,daemon=True);thread.start();thread.join(2)
 r={'recovery':recovery,'finished_under_lease':not thread.is_alive(),'main_event':K.WaitForSingleObject(v[1],0),'ads_event':K.WaitForSingleObject(adsv[1],0)}
 closelease(adsv);closelease(v);thread.join(10);require(not thread.is_alive(),'key query stuck after release')
 r['result']=result;r['matches']=result.get('keys')==[x.hex() for x in expected];return r
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-key-lease-') as tmp:receipt['cases']=[query(Path(tmp)/str(x),x) for x in [False,True]]
 receipt['status']='observed'
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
