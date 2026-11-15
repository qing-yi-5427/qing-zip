from pathlib import Path
exec(compile(Path(__file__).with_name('efs-oplock-ci-probe.py').read_text(encoding='utf-8-sig').split('\ntry:\n',1)[0],'defs','exec'))
import threading
out=Path(os.environ['GITHUB_WORKSPACE'])/'efs-oplock-key-change-result.json'
try:
 with tempfile.TemporaryDirectory(prefix='qing-efs-key-mutation-') as tmp:
  base=Path(tmp);cert=base/'additional.cer';p=base/'source';p.write_bytes(b'original');require(A.EncryptFileW(str(p)),'encrypt');before=keys(p)
  env=os.environ.copy();env['QZ_EFS_PROBE_CERT']=str(cert)
  script="$c = New-SelfSignedCertificate -Type Custom -Subject 'CN=qing-zip ephemeral EFS probe' -KeyAlgorithm RSA -KeyLength 2048 -KeyUsage KeyEncipherment -KeySpec KeyExchange -CertStoreLocation Cert:\\CurrentUser\\My -TextExtension @('2.5.29.37={text}1.3.6.1.4.1.311.10.3.4'); Export-Certificate -Cert $c -FilePath $env:QZ_EFS_PROBE_CERT | Out-Null"
  setup=subprocess.run(['powershell','-NoProfile','-Command',script],env=env,capture_output=True,text=True)
  receipt['certificate_setup']={'exit':setup.returncode,'stderr':setup.stderr};require(setup.returncode==0 and cert.exists(),'hosted fixture certificate failed')
  v,info=lease(p,7);require(v,'RWH unavailable');result={}
  def mutate():
   c=subprocess.run(['cipher','/adduser','/certfile:'+str(cert),'/a',str(p)],capture_output=True,text=True)
   result.update(exit=c.returncode,stdout=c.stdout,stderr=c.stderr)
  thread=threading.Thread(target=mutate,daemon=True);thread.start();thread.join(3)
  receipt['finished_under_lease']=not thread.is_alive();receipt['lease_event']=K.WaitForSingleObject(v[1],0)
  closelease(v);thread.join(20);require(not thread.is_alive(),'mutation stuck after lease release')
  receipt['mutation']=result;after=keys(p);receipt['keys_before']=len(before);receipt['keys_after']=len(after);receipt['keys_changed']=before!=after;receipt['source_preserved']=p.read_bytes()==b'original';receipt['status']='observed'
finally:out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
