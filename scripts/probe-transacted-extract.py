"""Probe publication ACLs and reader visibility on unmodified native Windows NTFS."""
import ctypes as c
from ctypes import wintypes as w
from pathlib import Path
import csv
import json
import os
import subprocess
import sys
import tempfile
import platform

k=c.WinDLL('kernel32',use_last_error=True)
ktm=c.WinDLL('ktmw32',use_last_error=True)
adv=c.WinDLL('advapi32',use_last_error=True)
def api(dll,name,restype,*args):
 f=getattr(dll,name);f.restype=restype;f.argtypes=args;return f
ptr=c.c_void_p
ctx=api(ktm,'CreateTransaction',w.HANDLE,ptr,ptr,w.DWORD,w.DWORD,w.DWORD,w.DWORD,w.LPCWSTR)
commit=api(ktm,'CommitTransaction',w.BOOL,w.HANDLE)
rollback=api(ktm,'RollbackTransaction',w.BOOL,w.HANDLE)
mkdir=api(k,'CreateDirectoryTransactedW',w.BOOL,w.LPCWSTR,w.LPCWSTR,ptr,w.HANDLE)
create=api(k,'CreateFileTransactedW',w.HANDLE,w.LPCWSTR,w.DWORD,w.DWORD,ptr,w.DWORD,w.DWORD,w.HANDLE,w.HANDLE,ptr,ptr)
open_w=api(k,'CreateFileW',w.HANDLE,w.LPCWSTR,w.DWORD,w.DWORD,ptr,w.DWORD,w.DWORD,w.HANDLE)
close=api(k,'CloseHandle',w.BOOL,w.HANDLE)
write=api(k,'WriteFile',w.BOOL,w.HANDLE,ptr,w.DWORD,c.POINTER(w.DWORD),ptr)
invalid=w.HANDLE(-1).value
sddl=api(adv,'ConvertStringSecurityDescriptorToSecurityDescriptorW',w.BOOL,w.LPCWSTR,w.DWORD,c.POINTER(ptr),ptr)
get_dacl=api(adv,'GetSecurityDescriptorDacl',w.BOOL,ptr,c.POINTER(w.BOOL),c.POINTER(ptr),c.POINTER(w.BOOL))
set_sd=api(adv,'SetNamedSecurityInfoW',w.DWORD,w.LPWSTR,w.DWORD,w.DWORD,ptr,ptr,ptr,ptr)
get_sd=api(adv,'GetNamedSecurityInfoW',w.DWORD,w.LPWSTR,w.DWORD,w.DWORD,ptr,ptr,ptr,ptr,c.POINTER(ptr))
as_string=api(adv,'ConvertSecurityDescriptorToStringSecurityDescriptorW',w.BOOL,ptr,w.DWORD,w.DWORD,c.POINTER(w.LPWSTR),ptr)
free=api(k,'LocalFree',ptr,ptr)
sid=next(csv.reader(subprocess.check_output(['whoami','/user','/fo','csv','/nh']).decode(errors='replace').splitlines()))[1]
def check(x):
 if not x:raise c.WinError(c.get_last_error())
 return x
def set_grant(path,allow):
 sd=ptr();check(sddl('D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;'+sid+')'+('(A;OICI;FR;;;LS)' if allow else ''),1,c.byref(sd),None))
 try:
  present=w.BOOL();acl=ptr();default=w.BOOL();check(get_dacl(sd,c.byref(present),c.byref(acl),c.byref(default)))
  err=set_sd(str(path),1,0x80000004,None,None,acl,None)
  if err:raise c.WinError(err)
 finally:free(sd)
def grant(path):
 sd=ptr();ret=get_sd(str(path),1,4,None,None,None,None,c.byref(sd))
 if ret:raise c.WinError(ret)
 text=w.LPWSTR()
 try:
  check(as_string(sd,1,4,c.byref(text),None))
  return ';;;LS)' in text.value or ';;;S-1-5-19)' in text.value
 finally:
  if text:free(text)
  free(sd)

if sys.argv[1]=='revoke':
 set_grant(Path(sys.argv[2]),False);sys.exit()
report=Path(sys.argv[1]);report.parent.mkdir(parents=True,exist_ok=True)
def save():report.write_text(json.dumps({'system':platform.platform(),'rows':rows},indent=2))
rows=[]
for fresh in (True,False):
 for action in ('cancel','commit','revoke','viewer','existing-viewer'):
  if fresh and action=='existing-viewer':continue
  with tempfile.TemporaryDirectory() as tmp:
   parent=Path(tmp)/'parent';parent.mkdir();set_grant(parent,True)
   dest=parent/'new' if fresh else parent
   leaf=dest/'leaf.txt'
   if action=='existing-viewer':leaf.write_bytes(b'original')
   tx=ctx(None,None,0,0,0,0,'qing isolated publication fixture');assert tx and tx!=invalid
   row={'fresh':fresh,'action':action}
   try:
    if fresh:check(mkdir(None,str(dest),None,tx))
    h=create(str(leaf),0xc0000000,7,None,2 if action=='existing-viewer' else 1,0,None,tx,None,None)
    assert h!=invalid,c.WinError(c.get_last_error())
    n=w.DWORD();check(write(h,b'verified',8,c.byref(n),None));check(close(h))
    row['leaf_visible_before']=leaf.is_file()
    reader=open_w(str(leaf),0x80000000,1,None,3,0,None)
    row['external_reader_open']=reader!=invalid;row['reader_error']=c.get_last_error() if reader==invalid else 0
    if row['external_reader_open']:row['reader_initial']=leaf.read_bytes().decode()
    if action=='revoke':
     run=subprocess.Popen([sys.executable,__file__,'revoke',parent],stdout=subprocess.PIPE,stderr=subprocess.PIPE)
     try:
      _,err=run.communicate(timeout=5);row['permission_changed_before_commit']=run.returncode==0;row['acl_error']=err.decode(errors='replace')[-260:]
     except subprocess.TimeoutExpired:row['permission_changed_before_commit']=False
     row['parent_grant_before_commit']=grant(parent)
    fun=rollback if action in ('viewer','cancel','existing-viewer') else commit
    ok=fun(tx);row['finished']=bool(ok);row['finish_error']=0 if ok else c.get_last_error()
    if reader!=invalid:close(reader)
    if action=='revoke' and run.poll() is None:run.communicate(timeout=10)
    row['leaf_exists_after']=leaf.exists();row['root_exists_after']=dest.exists()
    if leaf.exists():row['bytes']=leaf.read_bytes().decode();row['leaf_grant']=grant(leaf)
    witness=parent/'normal';witness.write_bytes(b'');row['ordinary_grant']=grant(witness)
    row['destination_grant']=grant(dest) if dest.exists() else None
    rows.append(row);save();print(row,flush=True)
   finally:close(tx)

for row in rows:
 assert row['finished'],row
 if row['action']=='revoke':
  assert row['permission_changed_before_commit'] and not row['parent_grant_before_commit']
  assert not row['ordinary_grant'] and row['leaf_grant'] and row['bytes']=='verified'
  if row['fresh']:assert row['destination_grant']
 if row['action'] in ('cancel','viewer'):
  assert not row['external_reader_open'] and not row['leaf_exists_after']
  if row['fresh']:assert not row['root_exists_after']
 if row['action']=='existing-viewer':
  assert row['bytes']=='original'
