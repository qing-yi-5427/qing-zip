from pathlib import Path
import os,sys,importlib.util,subprocess,time,tempfile,json,zipfile,ctypes as c,ctypes.wintypes as w
here=Path(__file__).resolve().parent
ctx={};exec((here/'check-live-output-acl.py').read_text(encoding='utf-8-sig').split('exe=Path(sys.argv[1])')[0],ctx)
for key in ('apply_acl','grants_ls','who','k','adv'):globals()[key]=ctx[key]
spec=importlib.util.spec_from_file_location('watch',here/'probe-native-acl-debug.py');dbg=importlib.util.module_from_spec(spec);spec.loader.exec_module(dbg)
def model(path,allow,legacy=False):
 import msvcrt
 text='D:'+('P' if legacy else 'AI')+'(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;'+who+')'+('(A;OICI;FR;;;LS)' if allow else '')
 sd=c.c_void_p();assert adv.ConvertStringSecurityDescriptorToSecurityDescriptorW(text,1,c.byref(sd),None)
 create=dbg.func(dbg.k,'CreateFileW',dbg.P,w.LPCWSTR,w.DWORD,w.DWORD,dbg.P,w.DWORD,w.DWORD,dbg.P)
 h=create(str(path),0x60081,3,None,3,0x02200000,None);assert h and h!=c.c_void_p(-1).value,c.get_last_error()
 try:
  if not legacy:assert adv.SetSecurityDescriptorControl(sd,0x500,0x500)
  nt=c.WinDLL('ntdll').NtSetSecurityObject;nt.argtypes=[dbg.P,w.DWORD,dbg.P];nt.restype=c.c_int32
  code=nt(h,4|(0x80000000 if legacy else 0x20000000),sd);assert code>=0,hex(code&0xffffffff)
 finally:dbg.close(h);k.LocalFree(sd)
exe=Path(sys.argv[1]).resolve();result=Path(sys.argv[2]).resolve();result.mkdir(exist_ok=True,parents=True)
rows=[]
for kind in sys.argv[3:] or ['file','tree','legacy-tree']:
 with tempfile.TemporaryDirectory() as tmp:
  root=Path(tmp);model(root,True,'legacy' in kind)
  zpath=root/'sample.zip'
  with zipfile.ZipFile(zpath,'w',zipfile.ZIP_STORED) as z:
   if kind.startswith('file'):
    with z.open('payload.bin','w') as f:
     for _ in range(96):f.write(b'q'*(1<<20))
   else:
    for i in range(120):z.writestr(f'f{i:03}.txt',b'normal')
  out=root/'out'
  if kind.startswith('file'):
   out.mkdir();model(out,True,'legacy' in kind)
   if 'overwrite' in kind:(out/'payload.bin').write_bytes(b'original')
  leaf=out/('payload.bin' if kind.startswith('file') else 'f010.txt')
  parent=out if kind!='legacy-tree' else out
  rename='rename' in kind
  if kind.startswith('copy'):
   import tarfile
   src=root/'input.txt';src.write_text('ordinary copy')
   zpath=root/'copy.tar'
   with tarfile.open(zpath,'w') as t:t.add(src,arcname='payload.bin')
   out.mkdir();model(out,True);leaf=out/'payload.bin';parent=out
  if rename and kind.startswith(('tree','legacy','private')):parent=out if 'local-leaf' in kind else root
  if kind.startswith('private'):
   model(root,False,'legacy' in kind)
  def match(p, new=None):
   if rename:
    if 'leaf' in kind:return bool(new) and Path(new).name=='f010.txt'
    return bool(new) and Path(new).name==('out' if parent==root else 'payload.bin')
   return os.path.normcase(p.removeprefix('\\\\?\\'))==os.path.normcase(str(leaf))
  def revoke():
   if kind.startswith('private'):
    remove=ctx['apply_acl'].__code__
    # Real native parent propagation, with the test user's full access retained.
    source=(here/'check-live-output-acl.py').read_text(encoding='utf-8-sig')
    sd=source[source.index('def apply_acl'):source.index('def grants_ls')]
    env=dict(ctx);exec(sd.replace('(A;OICI;FA;;;BA)',''),env);env['apply_acl'](parent,False)
   else:apply_acl(parent,False)
  hit,rc,err=dbg.observe([exe,'extract',zpath,out,'--overwrite','overwrite'],match,revoke,{**os.environ,'OPEN_ARCHIVE_PROFILE':'1'},'NtSetInformationFile' if rename else 'NtSetSecurityObject')
  witness=parent/'current-policy-check';witness.write_bytes(b'')
  grants=grants_ls
  if kind.startswith('private'):
   source=(here/'check-live-output-acl.py').read_text(encoding='utf-8-sig');definition=source[source.index('def grants_ls'):source.index('\nexe=Path')]
   env=dict(ctx);exec(definition.replace(';;;LS)',';;;BA)').replace('S-1-5-19)','S-1-5-32-544)'),env);grants=env['grants_ls']
  row=dict(kind=kind,stopped_at=hit,status=rc,leaf_exists=leaf.exists(),parent_read_grant=grants(parent),ordinary_read_grant=grants(witness),actual_read_grant=grants(leaf) if leaf.exists() else None,log=err);rows.append(row);print(row,flush=True)
  if 'leaf' in kind:
   row['leaf_count']=len(list(out.glob('f*.txt')))
   row['all_leaf_grants']=sum(grants(q) for q in out.glob('f*.txt'))
   row['actual_directory_grant']=grants(out)
  (result/'move-tree.json').write_text(json.dumps(rows,indent=2))
