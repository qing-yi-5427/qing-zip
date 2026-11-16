import argparse,ctypes,subprocess,tempfile,json,hashlib,os
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('--cli',type=Path,required=True);ap.add_argument('--workspace',type=Path,required=True);a=ap.parse_args();a.cli=a.cli.resolve()
r=Path(tempfile.mkdtemp(prefix='preserve-denied-',dir=a.workspace)).resolve();f=r/'readable.txt';data=b'ordinary read remains permitted';f.write_bytes(data);os.utime(f,ns=(1609459200000000000,1640995200000000000))
P=ctypes.c_void_p;U=ctypes.c_uint32;adv=ctypes.WinDLL('advapi32',use_last_error=True);k=ctypes.WinDLL('kernel32',use_last_error=True)
adv.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes=[ctypes.c_wchar_p,U,ctypes.POINTER(P),ctypes.POINTER(U)];adv.SetFileSecurityW.argtypes=[ctypes.c_wchar_p,U,P];adv.GetFileSecurityW.argtypes=[ctypes.c_wchar_p,U,P,U,ctypes.POINTER(U)];k.LocalFree.argtypes=[P]
sd=P();assert adv.ConvertStringSecurityDescriptorToSecurityDescriptorW('D:P(D;;0x100;;;WD)(A;;FA;;;OW)',1,ctypes.byref(sd),None)
try:assert adv.SetFileSecurityW(str(f),0x80000004,sd)
finally:k.LocalFree(sd)
def acl():
 n=U();adv.GetFileSecurityW(str(f),4,None,0,ctypes.byref(n));b=ctypes.create_string_buffer(n.value);assert adv.GetFileSecurityW(str(f),4,b,n.value,ctypes.byref(n));return hashlib.sha256(b.raw[:n.value]).hexdigest()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();dependencies={str(p):sha(p) for p in a.cli.parent.rglob('*') if p.is_file()};before=acl();out=r/'output.7z';cmd=[str(a.cli),'create',str(out),str(f),'--format','7z','--preserve-input-atime'];p=subprocess.run(cmd,capture_output=True,timeout=60);assert p.returncode==0,(p.stdout,p.stderr);result=json.loads(p.stdout);assert result['input_atime']['unavailable_handles']==1 and result['input_atime']['unconfirmed_sources']==0,result;assert result['input_atime']['warnings'][0]['os_error']==5,result;assert acl()==before
nana=a.cli.parent/'runtime/nanazip/NanaZip.Universal.Console.exe';dest=r/'decoded';x=subprocess.run([str(nana),'x','-y','-o'+str(dest),str(out)],capture_output=True);assert x.returncode==0 and (dest/f.name).read_bytes()==data
assert dependencies=={str(p):sha(p) for p in a.cli.parent.rglob('*') if p.is_file()}
receipt={'argv':cmd,'exit':p.returncode,'result':result,'source_acl_unchanged':True,'source_atime_ns':f.stat().st_atime_ns,'decoded_sha256':sha(dest/f.name),'dependency_sha256':dependencies,'script_sha256':sha(Path(__file__))};(r/'receipt.json').write_text(json.dumps(receipt,indent=2));print(r/'receipt.json')
