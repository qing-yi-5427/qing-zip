"""Synthetic timestamp parity: exact output metadata against official Nana + all content SHA.
No machine configuration changes. Not a performance benchmark.
"""
import argparse,ctypes,hashlib,itertools,json,os,subprocess,tempfile,time,datetime
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('--cli',type=Path,required=True);ap.add_argument('--reference',type=Path,required=True);ap.add_argument('--old-cli',type=Path,required=True);ap.add_argument('--workspace',type=Path,required=True);ap.add_argument('--scope',choices=['full','without-atime'],default='full');a=ap.parse_args()
OUT=Path(tempfile.mkdtemp(prefix='create-times-',dir=a.workspace)).resolve();sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
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
report={'scope':a.scope,'dependency_sha256':dependencies,'passed':False,'binaries':{k:sha(getattr(a,k)) for k in ['cli','reference','old_cli']},'script_sha256':sha(Path(__file__)),'rows':[]}
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

# Each source inventory is captured before resetting timestamps. No source lookup
# is performed between the final root reset and launching each encoder.
sources=[]
for label in ['alpha','beta']:
 p=OUT/label;p.mkdir();(p/'child').mkdir();(p/'child'/'unicode-文.txt').write_bytes(label.encode()*100);(p/'empty').mkdir();sources.append(p)
file=OUT/'loose.txt';file.write_bytes(b'loose');sources.append(file)
large=OUT/'many-directories';large.mkdir()
for i in range(4100):(large/f'd{i:04}').mkdir()
empty=OUT/'empty.dat';empty.write_bytes(b'')
cases=[]
for fmt in ['zip','7z']:
 for label,roots in [('single-file',[file]),('empty-file',[empty]),('overlap-file',[sources[0],sources[0]/'child'/'unicode-文.txt']),('overlap-dir',[sources[0],sources[0]/'child'])]:cases.append((label+'-'+fmt,fmt,roots,[],[],None))
for label,fmt,roots,q,n,password in cases:
 expected={}
 paths=[]
 for root in roots:
  if root.is_dir():
   expected.update({root.name+'/'+k:v for k,v in manifest(root).items()});paths.extend(root.rglob('*'))
  else:expected[root.name]=sha(root)
 paths=sorted(paths,key=lambda p:(p.is_dir(),-len(p.parts)))
 results=[]
 for tool in ['reference','candidate']:
  reset=sorted(set(paths+roots),key=lambda p:(p.is_dir(),-len(p.parts)))
  for p in reset:set_times(p,86400 if p in roots else 0)
  archive=OUT/(label+'-'+tool+'.'+fmt)
  cmd=[a.reference,'a','-t'+fmt,'-mx=5','-mmt=4','-mtc=on','-mta=on','-stl',*n,'--',archive,*roots] if tool=='reference' else [a.cli,'create',archive,*roots,'--format',fmt,'--level','5','--threads','4','--store-ctime','--store-atime','--archive-time','latest',*q]
  began=time.time();p=run(cmd,input=(password+'\n').encode() if password and tool=='candidate' else None);ended=time.time()
  md=metadata(archive,password);dest=OUT/(label+'-'+tool+'-decoded')
  run([a.reference,'x','-y',*(['-p'+password] if password else []),'-o'+str(dest),'--',archive]);assert manifest(dest)==expected
  row={'case':label,'tool':tool,'argv':list(map(str,cmd)),'metadata':md,'archive_mtime_ns':archive.stat().st_mtime_ns,'verified_content':True,'began':began,'ended':ended,'stderr':p.stderr.decode(errors='replace')}
  report['rows'].append(row);save();results.append(row)
 for name,first in results[0]['metadata'].items():
  second=results[1]['metadata'][name]
  assert {k:v for k,v in first.items() if k!='Accessed'}=={k:v for k,v in second.items() if k!='Accessed'},(label,name)
  if first['Accessed']!=second['Accessed']:
   assert label.startswith('overlap-'),(label,name,'unexpected atime variation')
   physical=next(root for root in roots if name.split('\\')[0]==root.name).parent/Path(name)
   ancestor=any(root!=physical and root.is_relative_to(physical) for root in roots)
   projection_root=next(root for root in roots if name.split('\\')[0]==root.name)
   alias=any(root!=projection_root and physical.is_relative_to(root) for root in roots)
   assert ancestor or alias,(label,name,'not an explicit-root ancestor or overlapping directory alias')
   for row in results:
    if alias and row['tool']=='candidate':
     original=next(root for root in roots if root!=projection_root and physical.is_relative_to(root))
     original_name=str(physical.relative_to(original.parent))
     assert row['metadata'][name]['Accessed']==row['metadata'][original_name]['Accessed'],(label,'immutable alias mismatch')
    else:
     timestamp=datetime.datetime.fromisoformat(row['metadata'][name]['Accessed']).timestamp()
     assert row['began']-0.02<=timestamp<=row['ended']+0.02,(label,name,'atime outside own execution interval',timestamp,row['began'],row['ended'])
   report.setdefault('reference_dynamic_ancestor_atime',[]).append({'case':label,'name':name,'reference':first['Accessed'],'candidate':second['Accessed'],'criterion':'ancestor: own invocation interval; duplicate directory: candidate exact first physical snapshot, reference own later scan interval'})
 for row in results:
  physical_times={}
  for name,md in row['metadata'].items():
   physical=next(root for root in roots if name.split('\\')[0]==root.name).parent/Path(name)
   key=str(physical).lower()
   if key in physical_times and row['tool']=='candidate':assert physical_times[key]==md['Accessed'],(label,row['tool'],name,'physical alias atime differs')
   physical_times[key]=md['Accessed']
 assert results[0]['archive_mtime_ns']==results[1]['archive_mtime_ns'],label
# Same-basename roots must not silently overwrite or merge snapshot records.
pairs=[]
for name in ['collision-a','collision-b']:
 p=OUT/name/'same';p.mkdir(parents=True);(p/'item').write_bytes(name.encode());pairs.append(p)
dest=OUT/'collision.7z';p=subprocess.run([str(a.cli),'create',str(dest),*[str(x) for x in pairs],'--format','7z','--store-atime'],capture_output=True,timeout=180)
assert p.returncode!=0 and not dest.exists()
report['collision']={'exit':p.returncode,'stderr':p.stderr.decode(errors='replace'),'source_sha':[manifest(x) for x in pairs]}
assert bind_dependencies()==dependencies
report['passed']=True;save();print(OUT/'receipt.json')
