"""Hosted readout of native/Win32 mask7 SD layouts; synthetic temp objects only."""
import ctypes as C, ctypes.wintypes as W, json, os, platform, struct, tempfile, hashlib
from pathlib import Path
H=C.c_void_p; U=W.DWORD; P=C.POINTER
k=C.WinDLL('kernel32',use_last_error=True); a=C.WinDLL('advapi32',use_last_error=True); n=C.WinDLL('ntdll')
def bind(lib,name,args,result):
 f=getattr(lib,name);f.argtypes=args;f.restype=result;return f
create=bind(k,'CreateFileW',[W.LPCWSTR,U,U,H,U,U,H],H)
close=bind(k,'CloseHandle',[H],W.BOOL);free=bind(k,'LocalFree',[H],H)
nt=bind(n,'NtQuerySecurityObject',[H,U,H,U,P(U)],C.c_long)
get=bind(a,'GetSecurityInfo',[H,U,U,P(H),P(H),P(H),P(H),P(H)],U)
length=bind(a,'GetSecurityDescriptorLength',[H],U)
convert=bind(a,'ConvertStringSecurityDescriptorToSecurityDescriptorW',[W.LPCWSTR,U,P(H),P(U)],W.BOOL)
getacl=bind(a,'GetSecurityDescriptorDacl',[H,P(W.BOOL),P(H),P(W.BOOL)],W.BOOL)
setinfo=bind(a,'SetSecurityInfo',[H,U,U,H,H,H,H],U)
getgroup=bind(a,'GetSecurityDescriptorGroup',[H,P(H),P(W.BOOL)],W.BOOL)
def parts(raw):
 revision,rm,control,owner,group,sacl,dacl=struct.unpack_from('<BBHIIII',raw)
 def sid(off):
  if not off:return None
  size=8+4*raw[off+1];assert off+size<=len(raw);return raw[off:off+size].hex()
 def acl(off):
  if not off:return None
  rev,sbz,size,count,sbz2=struct.unpack_from('<BBHHH',raw,off);assert off+size<=len(raw)
  at=off+8;aces=[]
  for _ in range(count):
   typ,flags,sz=struct.unpack_from('<BBH',raw,at);assert sz>=4 and at+sz<=off+size
   aces.append({'type':typ,'flags':flags,'bytes':raw[at:at+sz].hex()});at+=sz
  return {'revision':rev,'sbz':sbz,'size':size,'count':count,'sbz2':sbz2,'aces_ordered':aces,'trailing':raw[at:off+size].hex()}
 return {'revision':revision,'rm':rm,'control':control,'owner_group_offsets_alias':bool(owner and owner==group),'offsets':{'owner':owner,'group':group,'sacl':sacl,'dacl':dacl},'owner':sid(owner),'group':sid(group),'dacl':acl(dacl),'sacl':acl(sacl)}
def capture(handle,api):
 if api=='native':
  buf=(C.c_uint32*16384)();required=U();status=nt(handle,7,buf,C.sizeof(buf),C.byref(required));assert status>=0,(status,required.value)
  size=length(buf);assert 20<=size<=required.value<=C.sizeof(buf);raw=C.string_at(buf,size)
  extra={'status':status,'required':required.value}
 else:
  sd=H();status=get(handle,1,7,None,None,None,None,C.byref(sd));assert status==0,status
  try:raw=C.string_at(sd,length(sd))
  finally:free(sd)
  extra={'status':status}
 return dict(extra,length=len(raw),sha256=hashlib.sha256(raw).hexdigest(),raw_hex=raw.hex(),structure=parts(raw))
def apply(handle,sddl,mask):
 sd=H();assert convert(sddl,1,C.byref(sd),None),C.get_last_error()
 try:
  value=H();defaulted=W.BOOL();present=W.BOOL()
  if mask==2:assert getgroup(sd,C.byref(value),C.byref(defaulted));args=(None,value,None,None)
  else:assert getacl(sd,C.byref(present),C.byref(value),C.byref(defaulted));args=(None,None,value,None)
  status=setinfo(handle,1,mask,*args);assert status==0,status
 finally:free(sd)
report={'platform':platform.platform(),'python':platform.python_version(),'mask':7,'source':os.environ.get('GITHUB_SHA'),'cases':[]}
with tempfile.TemporaryDirectory(prefix='qing-receipt-sd-') as tmp:
 for directory in [False,True]:
  path=Path(tmp)/('directory' if directory else 'file')
  if directory:path.mkdir()
  else:path.write_bytes(b'synthetic')
  h=create(str(path),0xe0080,7,None,3,0x02000000,None);assert h not in (None,C.c_void_p(-1).value),C.get_last_error()
  try:
   for name,sddl,mask in [('initial',None,0),('group-BU','G:BU',2),('protected-allow','D:P(A;;FA;;;OW)(A;;FR;;;BU)',0x80000004),('deny-first','D:P(D;;WD;;;BU)(A;;FA;;;OW)(A;;FR;;;WD)',0x80000004),('deny-last','D:P(A;;FA;;;OW)(D;;WD;;;BU)(A;;FR;;;WD)',0x80000004)]:
    if sddl:apply(h,sddl,mask)
    samples=[{api:capture(h,api) for api in ['native','win32']} for _ in range(8)]
    report['cases'].append({'directory':directory,'name':name,'samples':samples})
    Path('receipt-security-diagnostic.json').write_text(json.dumps(report,indent=2))
  finally:close(h)
Path('receipt-security-diagnostic.json').write_text(json.dumps(report,indent=2))
for case in report['cases']:
 sample=case['samples'][0];print(case['directory'],case['name'],'raw_equal',sample['native']['raw_hex']==sample['win32']['raw_hex'],'structures',sample['native']['structure'],sample['win32']['structure'])
