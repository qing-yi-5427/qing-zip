"""Compare actual published programs; all tree construction and byte checks are untimed."""
import hashlib,itertools,json,os,platform,shutil,statistics,subprocess,sys,tempfile,time,zipfile
from pathlib import Path
qing=Path(sys.argv[1]).resolve(); result=Path(sys.argv[2]).resolve();result.mkdir()
nana=qing.parent/'runtime/nanazip/NanaZip.Universal.Console.exe'
def sha(p):
 with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
provenance=json.loads(Path('runtime/nanazip/provenance.json').read_bytes())
assert all(sha(nana.parent/n)==h for n,h in provenance['files'].items())
report={'host':platform.platform(),'cpu_count':os.cpu_count(),'qing_sha':sha(qing),'nana_sha':sha(nana),'archives':{},'rows':[],'summary':{}}
def save():(result/'measurements.json').write_text(json.dumps(report,indent=2))
def run(args,**kw):
 done=subprocess.run(args,capture_output=True,stdin=subprocess.DEVNULL,timeout=120,**kw)
 if done.returncode:raise RuntimeError((args,done.returncode,done.stderr[-1500:],done.stdout[-900:]))
 return done
SENTINEL=b'preexisting sentinel, never replace'
with tempfile.TemporaryDirectory() as tmp:
 root=Path(tmp);source=root/'source';source.mkdir();expected={};conflicts=[]
 for i in range(2400):
  n=f'd{i%24:02}/f{i:05}.txt';p=source/n;p.parent.mkdir(exist_ok=True)
  b=(f'archive row {i}\n'.encode()*512)[:4096];p.write_bytes(b);expected[n]=hashlib.sha256(b).hexdigest()
  if i%20==0:conflicts.append(n)
 files=sorted(expected);old_hash=hashlib.sha256(SENTINEL).hexdigest()
 def seed(dst,mode):
  dst.mkdir();spec=dict(expected)
  if mode!='empty':
   p=dst/'unrelated.txt';p.write_bytes(SENTINEL);spec[p.name]=old_hash
   # All existing directories must be preserved, including the unrelated empty one.
   (dst/'empty unrelated').mkdir()
   for i in range(24):(dst/f'd{i:02}').mkdir()
  if mode in ['skip','overwrite','rename']:
   for n in conflicts:(dst/n).write_bytes(SENTINEL)
   if mode=='skip':
    for n in conflicts:spec[n]=old_hash
  return spec
 def verify(dst,spec,mode,tool):
  if mode=='rename':
   for n in conflicts:
    p=Path(n)
    q=p.with_name(p.stem+(' (1)' if tool=='qing' else '_1')+p.suffix).as_posix()
    spec[q]=spec[n];spec[n]=old_hash
  actual={p.relative_to(dst).as_posix():sha(p) for p in dst.rglob('*') if p.is_file()}
  assert actual==spec,(mode,tool,len(actual),len(spec),sorted(actual.keys()^spec.keys())[:10])
  dirs={p.relative_to(dst).as_posix() for p in dst.rglob('*') if p.is_dir()}
  assert dirs=={f'd{i:02}' for i in range(24)}|({'empty unrelated'} if mode!='empty' else set()),dirs
  return len(actual)
 for fmt in ['zip','7z']:
  archive=root/('test.'+fmt)
  args=[nana,'a','-t'+fmt,'-mx=5']
  if fmt=='7z':args+=['-ms=on','-m0=LZMA2']
  else:args+=['-mm=Deflate']
  run([*args,'--',archive,'.'],cwd=source)
  tech=run([nana,'l','-slt','-slmu','--',archive]).stdout.decode('utf8')
  if fmt=='7z':assert 'Solid = +' in tech and 'Blocks = 1' in tech
  report['archives'][fmt]={'sha256':sha(archive),'bytes':archive.stat().st_size,'file_count':len(expected),'conflicts':len(conflicts)}
  for mode in ['empty','nonconflict','skip','overwrite','rename']:
   for rep in range(5):
    for tool in (['qing','nana'] if rep%2==0 else ['nana','qing']):
     dest=root/'output';spec=seed(dest,mode);archive.read_bytes()
     option={'empty':'error','nonconflict':'error'}.get(mode,mode)
     if tool=='qing':cmd=[qing,'extract',archive,dest,'--overwrite',option]
     else:cmd=[nana,'x','-y',{'rename':'-aou','skip':'-aos'}.get(mode,'-aoa'),'-o'+str(dest),'--',archive]
     env=dict(os.environ);env.pop('OPEN_ARCHIVE_PROFILE',None)
     if rep==4 and tool=='qing':env['OPEN_ARCHIVE_PROFILE']='1'
     tick=time.perf_counter();proc=run(cmd,env=env);wall=time.perf_counter()-tick
     count=verify(dest,spec,mode,tool)
     phases=[s for s in proc.stderr.decode(errors='replace').splitlines() if s.startswith('OA_PHASE')]
     report['rows'].append(dict(format=fmt,mode=mode,tool=tool,rep=rep,seconds=wall,verified_files=count,phases=phases))
     save();shutil.rmtree(dest)
   summary={tool:statistics.median(r['seconds'] for r in report['rows'] if r['format']==fmt and r['mode']==mode and r['tool']==tool and 1<=r['rep']<=3) for tool in ['qing','nana']}
   report['summary'][f'{fmt}/{mode}']=summary; print(fmt,mode,summary,flush=True);save()
 report['complete']=True;save()
