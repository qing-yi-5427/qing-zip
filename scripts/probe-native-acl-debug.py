"""Stop only the isolated benchmark child at a real native ACL assignment.
The debugger restores each original instruction and never changes its arguments/results."""
import ctypes as c, ctypes.wintypes as w, os,struct,subprocess,time
from pathlib import Path
k=c.WinDLL('kernel32',use_last_error=True); P=c.c_void_p; D=w.DWORD; Z=c.c_size_t; B=w.BOOL
def func(lib,name,ret,*args):
 f=getattr(lib,name);f.argtypes=args;f.restype=ret;return f
def ok(result):
 if not result:raise c.WinError(c.get_last_error())
 return result
wait=func(k,'WaitForDebugEvent',B,P,D);cont=func(k,'ContinueDebugEvent',B,D,D,D)
getmodule=func(k,'GetModuleHandleW',P,w.LPCWSTR);addr=func(k,'GetProcAddress',P,P,c.c_char_p)
thread=func(k,'OpenThread',P,D,B,D);getctx=func(k,'GetThreadContext',B,P,P);setctx=func(k,'SetThreadContext',B,P,P)
close=func(k,'CloseHandle',B,P);dup=func(k,'DuplicateHandle',B,P,P,P,c.POINTER(P),D,B,D)
path=func(k,'GetFinalPathNameByHandleW',D,P,w.LPWSTR,D,D)
read=func(k,'ReadProcessMemory',B,P,P,P,Z,c.POINTER(Z));write=func(k,'WriteProcessMemory',B,P,P,P,Z,c.POINTER(Z));flush=func(k,'FlushInstructionCache',B,P,P,Z)
protect=func(k,'VirtualProtectEx',B,P,P,Z,D,c.POINTER(D))
def pathname(handle):
 b=c.create_unicode_buffer(32768);n=path(handle,b,len(b),0);return b.value if n and n<len(b) else ''
def observe(args,match,action,env,probe='NtSetSecurityObject'):
 proc=subprocess.Popen(list(map(str,args)),creationflags=2|0x08000000,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 handle=int(proc._handle);ev=c.create_string_buffer(208);bp=None;old=None;hit=None;single=set();initial=True
 rva=addr(getmodule('ntdll.dll'),probe.encode())-getmodule('ntdll.dll')
 def byte(code):
  prev=D();ok(protect(handle,bp,1,0x40,c.byref(prev)))
  try:ok(write(handle,bp,code,1,None));ok(flush(handle,bp,1))
  finally:ok(protect(handle,bp,1,prev.value,c.byref(D())))
 try:
  end=time.monotonic()+120
  while time.monotonic()<end:
   ok(wait(ev,120000));code,pid,tid=struct.unpack_from('<III',ev.raw)
   status=0x10002
   if code==6:
    file,base=struct.unpack_from('<QQ',ev.raw,16)
    if file:
     if pathname(file).lower().endswith('\\ntdll.dll'):
      bp=base+rva;var=c.create_string_buffer(1);ok(read(handle,bp,var,1,None));old=var.raw
     close(file)
   elif code==3:
    f=struct.unpack_from('<Q',ev.raw,16)[0]
    if f:close(f)
   elif code==1:
    exc=struct.unpack_from('<I',ev.raw,16)[0];location=struct.unpack_from('<Q',ev.raw,32)[0]
    if initial and exc==0x80000003:
     assert bp and old is not None,'Failed to resolve native DLL';byte(b'\xcc');initial=False
    elif (exc==0x80000003 and location==bp) or (exc==0x80000004 and tid in single):
     h=ok(thread(0x1a,False,tid));raw=c.create_string_buffer(1328);ctx=(c.addressof(raw)+15)&~15
     try:
      c.c_uint32.from_address(ctx+48).value=0x100003;ok(getctx(h,ctx))
      flags=c.c_uint32.from_address(ctx+68)
      if exc==0x80000003:
       obj=P();arg=c.c_uint64.from_address(ctx+128).value;name=''
       if dup(handle,arg,P(-1),c.byref(obj),0,False,2):
        name=pathname(obj);close(obj)
       byte(old);c.c_uint64.from_address(ctx+248).value=bp
       extra=None
       if probe=="NtSetInformationFile":
        temp=c.create_string_buffer(4)
        sp=c.c_uint64.from_address(ctx+152).value
        ok(read(handle,sp+0x28,temp,4,None))
        info_class=struct.unpack("<I",temp.raw)[0]
        if info_class in (10,65):
         hdr=c.create_string_buffer(20);info=c.c_uint64.from_address(ctx+184).value
         ok(read(handle,info,hdr,20,None));length=struct.unpack_from("<I",hdr.raw,16)[0]
         if length<=65536:
          text=c.create_string_buffer(length);ok(read(handle,info+20,text,length,None));extra=text.raw.decode('utf-16-le')
       if not hit and (match(name,extra) if probe=="NtSetInformationFile" else match(name)):
        hit=name;action()
       else:
        flags.value |= 0x100;single.add(tid)
      else:
       flags.value &= ~0x100;single.remove(tid)
       if not hit:byte(b'\xcc')
      ok(setctx(h,ctx))
     finally:close(h)
    else:status=0x80010001
   ok(cont(pid,tid,status))
   if code==5:break
  else:raise RuntimeError('debug timeout')
  stdout,stderr=proc.communicate(timeout=15)
  return hit,proc.returncode,stderr.decode(errors='replace')
 finally:
  if proc.poll() is None:proc.kill()
