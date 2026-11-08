import ctypes as c, importlib.util, itertools, json, os, statistics, subprocess, sys, tempfile, time, zipfile, hashlib
from pathlib import Path
root=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('accounting',root/'hosted-list-accounting.py')
acc=importlib.util.module_from_spec(spec);spec.loader.exec_module(acc)
exe1,exe2,result=map(lambda s:Path(s).resolve(),sys.argv[1:]);exe={'bounded':exe1,'typed':exe2,'official':exe1.parent/'runtime/nanazip/NanaZip.Universal.Console.exe'}
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
data={'binaries':{k:sha(v) for k,v in exe.items()},'rows':[],'summary':{}}
def save():(result/'measurements.json').write_text(json.dumps(data,indent=2))
def run(k,src,out,resources):
 job=None;api=acc.k
 if resources:
  job=api.CreateJobObjectW(None,None);acc.check(job);limits=acc.Extended();limits.limits.flags=0x2000
  acc.check(api.SetInformationJobObject(job,9,c.byref(limits),c.sizeof(limits)))
 try:
  src.read_bytes()
  with out.open('wb') as f:
   cmd=[exe[k],'list',src] if k!='official' else [exe[k],'l','-slt','-slmu','-ba','-sccUTF-8','--',src]
   t=time.perf_counter();p=subprocess.Popen(cmd,stdin=subprocess.DEVNULL,stdout=f,stderr=subprocess.PIPE,creationflags=0x08000000)
   if job:
    h=api.OpenProcess(0x1101,False,p.pid);acc.check(h)
    try:acc.check(api.AssignProcessToJobObject(job,h))
    finally:api.CloseHandle(h)
   _,err=p.communicate(timeout=100);ms=(time.perf_counter()-t)*1000;assert p.returncode==0,err[-500:]
  row={'ms':ms,'sha':sha(out)}
  if job:
   usage=acc.Usage();acc.check(api.QueryInformationJobObject(job,8,c.byref(usage),c.sizeof(usage),None));acc.check(api.QueryInformationJobObject(job,9,c.byref(limits),c.sizeof(limits),None))
   row.update(peak_mib=limits.peak_job_commit/2**20,cpu_ms=(usage.cpu.user+usage.cpu.kernel)/1e4,processes=usage.cpu.processes)
  return row
 finally:
  if job:api.CloseHandle(job)
with tempfile.TemporaryDirectory() as tmp:
 d=Path(tmp);regular=d/'large.zip'
 with zipfile.ZipFile(regular,'w') as z:
  for i in range(50000):
   z.writestr(zipfile.ZipInfo(f'file-{i:05d}.txt',(2025,1,1,0,0,0)),f'row {i:05d}\n'.encode())
 assert sha(regular)=='f7243bf7f9448c1187613759a9f0be60ff4064f57ce025ba321a8345ae483e00'
 jdk=Path(os.environ['JAVA_HOME_21_X64'])/'lib/src.zip';assert jdk.exists()
 for label,src in [('zip-50000',regular),('actual-jdk21',jdk)]:
  outs={k:d/(k+'.out') for k in exe};oracle={}
  for profile,reps in [(False,6),(True,3)]:
   for r in range(reps+1):
    for k in list(itertools.permutations(exe))[max(r-1,0)%6]:
     row=run(k,src,outs[k],profile)
     if k not in oracle:oracle[k]=row['sha']
     assert row['sha']==oracle[k]
     data['rows'].append(dict(case=label,rep=r,profile=profile,tool=k,**row))
   save()
  assert oracle['bounded']==oracle['typed']
  entries=json.loads(outs['typed'].read_bytes())['entries']
  text=outs['official'].read_text(encoding='utf8').strip().split('\n\n')
  assert len(text)==len(entries)
  for e,b in zip(entries,text):
   f=dict(l.split(' = ',1) for l in b.splitlines());attrs=f.get('Attributes','')
   assert e==dict(name=f['Path'],size=int(f['Size']),size_known=True,packed_size=int(f['Packed Size']),is_dir=f.get('Folder')=='+' or attrs.startswith(('D','d')) or f['Path'].endswith('/'),encrypted=f['Encrypted']=='+',modified=f['Modified'],method=f['Method'],comment=f.get('Comment',''),attributes=attrs,crc=f.get('CRC','')),(e,f)
  summary={'source_sha':sha(src),'members':len(entries),'tools':{}}
  for k in exe:
   rows=[r for r in data['rows'] if r['case']==label and r['rep'] and r['tool']==k]
   summary['tools'][k]={m:statistics.median(r[m] for r in rows if r['profile']==pr) for m,pr in [('ms',False),('peak_mib',True),('cpu_ms',True)]}
  data['summary'][label]=summary;save();print(label,summary)
