import ctypes, pathlib, subprocess, os, tempfile, random, hashlib, json, struct, argparse
ap=argparse.ArgumentParser();ap.add_argument('--cli',type=pathlib.Path,required=True);ap.add_argument('--workspace',type=pathlib.Path,required=True);a=ap.parse_args();cli=a.cli.resolve(strict=True);out=pathlib.Path(tempfile.mkdtemp(prefix='direct-reparse-',dir=a.workspace.resolve(strict=True)))
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
deps=[pathlib.Path(__file__).resolve(),cli,cli.with_name('obz-worker.exe'),*list((cli.parent/'runtime').rglob('*'))];bound={str(p):sha(p) for p in deps if p.is_file()}
k=ctypes.WinDLL('kernel32',use_last_error=True);k.CreateFileW.argtypes=[ctypes.c_wchar_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p];k.CreateFileW.restype=ctypes.c_void_p;k.CloseHandle.argtypes=[ctypes.c_void_p];k.DeviceIoControl.argtypes=[ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.c_void_p,ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint32),ctypes.c_void_p]
ancestor=out/'ancestor';source=ancestor/'source';source.mkdir(parents=True);target=out/'replacement';target.mkdir();(target/'source').mkdir();(target/'source'/'intruder').write_bytes(b'must not be archived')
data=random.Random(33).randbytes(8*1024*1024);file=source/'data.bin';file.write_bytes(data);expected=hashlib.sha256(data).hexdigest();dest=out/'result.7z'
cmd=[str(cli),'create',str(dest),str(source),'--format','7z','--store-atime','--threads','1'];p=subprocess.Popen(cmd,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env={**os.environ,'OPEN_ARCHIVE_PROFILE':'1'})
report={'dependency_sha256':bound,'cli_sha256':hashlib.sha256(cli.read_bytes()).hexdigest(),'argv':cmd,'source_sha256':expected};log=[];writer=None;installed=False
try:
 for line in iter(p.stderr.readline,b''):
  log.append(line.decode(errors='replace'))
  if b'OA_CREATE_WORKER status=encoding' in line:
   report['mutation_after_encoding']=True
   try:ancestor.rename(out/'renamed');report['rename_blocked']=False
   except OSError as e:report['rename_blocked']=True;report['rename_error']=e.winerror
   assert report['rename_blocked']
   writer=k.CreateFileW(str(ancestor),0x40000000,7,None,3,0x02200000,None);assert writer!=ctypes.c_void_p(-1).value,ctypes.get_last_error()
   text=str(target);sub=('\\??\\'+text).encode('utf-16-le');printed=text.encode('utf-16-le');body=struct.pack('<HHHH',0,len(sub),len(sub)+2,len(printed))+sub+b'\0\0'+printed+b'\0\0';buffer=struct.pack('<IHH',0xa0000003,len(body),0)+body
   n=ctypes.c_uint32();buf=ctypes.create_string_buffer(buffer);installed=bool(k.DeviceIoControl(writer,0x900a4,buf,len(buffer),None,0,ctypes.byref(n),None));report.update(reparse_installed=installed,reparse_error=0 if installed else ctypes.get_last_error())
 stdout=p.stdout.read();code=p.wait(timeout=60);report.update(exit=code,stderr=''.join(log),stdout=stdout.decode(errors='replace'),output_exists=dest.exists())
 assert report.get('mutation_after_encoding')
 if installed:assert code!=0 and not dest.exists(),report
 else:assert code==0,report
finally:
 if p.poll() is None:p.kill();p.wait(timeout=10)
 if installed:
  tag=ctypes.create_string_buffer(struct.pack('<IHH',0xa0000003,0,0));n=ctypes.c_uint32();assert k.DeviceIoControl(writer,0x900ac,tag,8,None,0,ctypes.byref(n),None),ctypes.get_last_error()
 if writer and writer!=ctypes.c_void_p(-1).value:k.CloseHandle(writer)
 report['source_sha_after']=hashlib.sha256(file.read_bytes()).hexdigest();report['source_unchanged']=report['source_sha_after']==expected
 (out/'receipt.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
assert report['source_unchanged'];
if dest.exists():
 decoded=out/'decoded';subprocess.run([str(cli.parent/'runtime/nanazip/NanaZip.Universal.Console.exe'),'x','-y','-o'+str(decoded),str(dest)],capture_output=True,check=True);files=[p for p in decoded.rglob('*') if p.is_file()];assert len(files)==1 and sha(files[0])==expected;report['decoded_full_sha_verified']=True
assert {str(p):sha(p) for p in deps if p.is_file()}==bound;report['dependencies_unchanged']=True;(out/'receipt.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(out/'receipt.json');print(json.dumps({k:v for k,v in report.items() if k not in ['stderr','stdout','argv']},indent=2))
