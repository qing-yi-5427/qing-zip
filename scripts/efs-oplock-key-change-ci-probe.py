from pathlib import Path
exec(compile(Path(__file__).with_name('efs-oplock-ci-probe.py').read_text(encoding='utf-8-sig').split('\ntry:\n',1)[0],'defs','exec'))
import threading
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-oplock-key-change-result.json'
def stage(name):
 receipt['stage']=name;out.write_text(json.dumps(receipt,indent=2),encoding='utf-8');print(name,flush=True)
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-key-mutation-') as tmp:
  base=Path(tmp);cert=base/'additional.cer';p=base/'source';p.write_bytes(b'original');require(A.EncryptFileW(str(p)),'encrypt');before=keys(p)
  import shutil
  exe=shutil.which('openssl') or 'C:/Program Files/Git/usr/bin/openssl.exe'
  stage('certificate setup')
  setup=subprocess.run([exe,'req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(base/'private.pem'),'-out',str(cert),'-outform','DER','-days','1','-subj','/CN=qing-zip ephemeral EFS probe','-addext','extendedKeyUsage=1.3.6.1.4.1.311.10.3.4','-addext','keyUsage=keyEncipherment'],capture_output=True,text=True,timeout=30)
  receipt['certificate_setup']={'exit':setup.returncode,'stderr':setup.stderr};require(setup.returncode==0 and cert.exists(),'hosted fixture certificate failed')
  stage('trust disposable certificate')
  trust=subprocess.run(['certutil','-user','-addstore','Root',str(cert)],capture_output=True,text=True,timeout=20)
  receipt['certificate_trust']={'exit':trust.returncode,'stdout':trust.stdout,'stderr':trust.stderr};require(trust.returncode==0,'ephemeral trust failed')
  baseline=base/'baseline';require(copy(p,baseline)[0],'baseline copy failed')
  trial=subprocess.run(['cipher','/adduser','/certfile:'+str(cert),'/a',str(baseline)],capture_output=True,text=True,timeout=20)
  receipt['unlocked_baseline']={'exit':trial.returncode,'stdout':trial.stdout,'stderr':trial.stderr,'keys':len(keys(baseline))};require(trial.returncode==0 and len(keys(baseline))>len(before),'unlocked key mutation did not succeed')
  stage('acquire lease');v,info=lease(p,7);require(v,'RWH unavailable');result={}
  def mutate():
   c=subprocess.run(['cipher','/adduser','/certfile:'+str(cert),'/a',str(p)],capture_output=True,text=True,timeout=15)
   result.update(exit=c.returncode,stdout=c.stdout,stderr=c.stderr)
  stage('launch key mutation');thread=threading.Thread(target=mutate,daemon=True);thread.start();thread.join(3)
  receipt['finished_under_lease']=not thread.is_alive();receipt['lease_event']=K.WaitForSingleObject(v[1],0)
  stage('release lease');closelease(v);stage('join mutation');thread.join(20);require(not thread.is_alive(),'mutation stuck after lease release')
  receipt['mutation']=result;stage('final key query');after=keys(p);receipt['keys_before']=len(before);receipt['keys_after']=len(after);receipt['keys_changed']=before!=after;receipt['source_preserved']=p.read_bytes()==b'original';receipt['status']='observed'
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
