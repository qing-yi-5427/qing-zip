"""Synthetic timestamp parity: exact output metadata against official Nana + all content SHA.
No machine configuration changes. Not a performance benchmark.
"""
import argparse,ctypes,hashlib,itertools,json,os,subprocess,tempfile
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('--cli',type=Path,required=True);ap.add_argument('--reference',type=Path,required=True);ap.add_argument('--old-cli',type=Path,required=True);ap.add_argument('--workspace',type=Path,required=True);a=ap.parse_args()
OUT=Path(tempfile.mkdtemp(prefix='create-times-',dir=a.workspace));sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
# Bind every executable/runtime file actually used; do not infer mixed-package
# provenance from the top-level CLI alone. Freeze manifests, when present, are
# included in the byte binding and declarations checked before any fixture work.
def bind_dependencies():
 files={Path(__file__).resolve(),a.cli.resolve(),a.old_cli.resolve(),a.reference.resolve()}
 for parent in {a.cli.resolve().parent,a.old_cli.resolve().parent}:
  helper=parent/'obz-worker.exe'
  if helper.exists():files.add(helper)
  runtime=parent/'runtime'
  if runtime.exists():files.update(x for x in runtime.rglob('*') if x.is_file())
  manifest_file=parent/'freeze-manifest.json'
  if manifest_file.exists():
   files.add(manifest_file);m=json.loads(manifest_file.read_text(encoding='utf-8-sig'))
   b=m['binaries'];decl=dict(b) if isinstance(b,dict) else {x['name']:x['sha256'] for x in b}
   for name,digest in m.get('runtime_inputs',{}).items():
    assert name not in decl or decl[name]==digest;decl[name]=digest
   for name,digest in decl.items():
    f=(parent/name).resolve(strict=True);assert f.is_relative_to(parent) and sha(f)==digest,(name,'freeze mismatch');files.add(f)
 # The reference console dynamically loads siblings from its own directory.
 files.update(x for x in a.reference.resolve().parent.rglob('*') if x.is_file())
 for f in files:assert not (f.lstat().st_file_attributes&0x400),f
 return {str(f):sha(f) for f in sorted(files)}
dependencies=bind_dependencies()
report={'dependency_sha256':dependencies,'passed':False,'binaries':{k:sha(getattr(a,k)) for k in ['cli','reference','old_cli']},'script_sha256':sha(Path(__file__)),'rows':[]}
def save():(OUT/'receipt.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
def run(cmd,input=None):
 p=subprocess.run([str(x) for x in cmd],input=input,capture_output=True,timeout=180);assert p.returncode==0,(cmd,p.returncode,p.stderr.decode(errors='replace'));return p
K=ctypes.WinDLL('kernel32',use_last_error=True);K.CreateFileW.restype=ctypes.c_void_p;K.CreateFileW.argtypes=[ctypes.c_wchar_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p];K.SetFileTime.argtypes=[ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p,ctypes.c_void_p];K.CloseHandle.argtypes=[ctypes.c_void_p]
def set_times(p,offset=0):
 h=K.CreateFileW(str(p),0x100,7,None,3,0x02000000,None);assert h not in [None,ctypes.c_void_p(-1).value]
 vals=[ctypes.c_uint64((11644473600+v+offset)*10000000) for v in [1577836800,1609459200,1640995200]]
 try:assert K.SetFileTime(h,*[ctypes.byref(v) for v in vals]),ctypes.get_last_error()
 finally:K.CloseHandle(h)
def manifest(root):return {str(p.relative_to(root)).replace('\\','/'):sha(p) for p in root.rglob('*') if p.is_file()}
def metadata(p,password=None):
 text=run([a.reference,'l','-slt','-sccUTF-8',*(['-p'+password] if password else []),'--',p]).stdout.decode('utf-8-sig');assert '----------' in text
 blocks=text.split('----------',1)[1].strip().split('\r\n\r\n' if '\r\n' in text else '\n\n');out={}
 for block in blocks:
  d=dict(line.split(' = ',1) for line in block.splitlines() if ' = ' in line)
  if 'Path' in d:out[d['Path']]={k:d.get(k) for k in ['Folder','Size','Modified','Created','Accessed']}
 return out
for count in [3,2050]:
 src=OUT/f'source-{count}';src.mkdir();(src/'空目录').mkdir()
 for i in range(count):(src/f'{i:04}.txt').write_bytes(b'' if i==0 else (f'fixed-content-{i}'*10).encode())
 expected={src.name+'/'+k:v for k,v in manifest(src).items()}
 for fmt in ['zip','7z']:
  for choice in [None,*itertools.product([False,True],repeat=3)]:
   for latest in [False,True]:
    outputs=[];tag=f'{count}-{fmt}-{choice}-{latest}'.replace(' ','')
    for tool in ['reference','candidate']+(['old'] if choice is None and not latest else []):
     for p in src.rglob('*'):set_times(p)
     set_times(src,86400) # latest directory, not latest regular file
     archive=OUT/f'{tag}-{tool}.{fmt}';q=[];n=[]
     if choice is not None:
      for name,value in zip(['mtime','ctime','atime'],choice):q+=['--store-'+name if value else '--no-store-'+name];n+=['-m'+{'mtime':'tm','ctime':'tc','atime':'ta'}[name]+'='+('on' if value else 'off')]
     if latest:q+=['--archive-time','latest'];n+=['-stl']
     if tool=='reference':cmd=[a.reference,'a','-t'+fmt,'-mx=5','-mmt=4',*n,'--',archive,src]
     else:cmd=[a.cli if tool=='candidate' else a.old_cli,'create',archive,src,'--format',fmt,'--level','5','--threads','4',*q]
     env=os.environ.copy();env['OPEN_ARCHIVE_PROFILE']='1'
     p=subprocess.run([str(x) for x in cmd],capture_output=True,timeout=180,env=env);assert p.returncode==0,p.stderr
     if tool=='candidate':
      log=p.stderr.decode(errors='replace');native='OA_CREATE_WORKER status=complete' in log
      assert native==(count==2050),(tag,'unexpected creation route',log)
     md=metadata(archive);dest=OUT/(tag+'-'+tool+'-decoded');run([a.reference,'x','-y','-o'+str(dest),'--',archive]);assert manifest(dest)==expected
     row={'case':tag,'tool':tool,'argv':list(map(str,cmd)),'metadata':md,'archive_mtime_ns':archive.stat().st_mtime_ns,'archive_sha256':sha(archive),'verified_content':True,'stderr':p.stderr.decode(errors='replace')};report['rows'].append(row);save();outputs.append(row)
    assert all(x['metadata']==outputs[0]['metadata'] for x in outputs),tag
    if choice is None and not latest:
     bytool={x['tool']:x for x in outputs};assert bytool['candidate']['archive_sha256']==bytool['old']['archive_sha256'],(tag,'default archive bytes changed')
    if latest:assert all(x['archive_mtime_ns']==outputs[0]['archive_mtime_ns'] for x in outputs),tag
# Additional console publication routes: volumes, encrypted headers, and SFX.
for mode,fmt in [('split','zip'),('split','7z'),('encrypted','7z'),('sfx','sfx')]:
 src=OUT/'source-3';expected={src.name+'/'+k:v for k,v in manifest(src).items()};results=[]
 for tool in ['reference','candidate']:
  for p in src.rglob('*'):set_times(p)
  set_times(src,86400)
  directory=OUT/(mode+'-'+fmt+'-'+tool);directory.mkdir();archive=directory/('output.exe' if fmt=='sfx' else 'output.'+fmt)
  q=['--store-ctime','--store-atime','--archive-time','latest'];n=['-mtc=on','-mta=on','-stl']
  if mode=='split':q+=['--volume','128'];n+=['-v128b']
  if mode=='encrypted':q+=['--password-stdin','--header-encryption'];n+=['-pKnown-Time-Password','-mhe=on']
  if mode=='sfx':n+=['-sfx'+str(a.cli.parent/'runtime/7zip/7z.sfx')]
  cmd=[a.reference,'a','-t'+('7z' if fmt=='sfx' else fmt),'-mx=5','-mmt=4',*n,'--',archive,src] if tool=='reference' else [a.cli,'create',archive,src,'--format',fmt,'--level','5','--threads','4',*q]
  run(cmd,input=b'Known-Time-Password\n' if mode=='encrypted' and tool=='candidate' else None);files=sorted(directory.iterdir());assert files
  dest=OUT/(mode+'-'+fmt+'-'+tool+'-decoded');run([a.reference,'x','-y','-pKnown-Time-Password','-o'+str(dest),'--',files[0]]);assert manifest(dest)==expected
  times=[p.stat().st_mtime_ns for p in files];md=metadata(files[0],'Known-Time-Password' if mode=='encrypted' else None);report['rows'].append({'case':mode+'-'+fmt,'tool':tool,'metadata':md,'argv':list(map(str,cmd)),'published_files':{p.name:{'sha256':sha(p),'mtime_ns':p.stat().st_mtime_ns} for p in files},'verified_content':True});save();results.append({'times':times,'metadata':md})
 assert results[0]==results[1],(mode,fmt,results)
assert bind_dependencies()==dependencies,'runtime/script changed during acceptance'
report['passed']=True;save();print(OUT/'receipt.json')
