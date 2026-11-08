"""Try revocation with an already-open administrative handle and inherited policy."""
import time
prefix=__import__('pathlib').Path(__file__).with_name('probe-transacted-extract.py').read_text(encoding='utf-8')
exec(prefix[:prefix.index("if sys.argv[1]=='revoke':")])
set_handle_sd=api(adv,'SetSecurityInfo',w.DWORD,w.HANDLE,w.DWORD,w.DWORD,ptr,ptr,ptr,ptr)
if sys.argv[1]=='preopened':
 parent=Path(sys.argv[2])
 h=open_w(str(parent),0x60000,7,None,3,0x02200000,None)
 if h==invalid:raise c.WinError(c.get_last_error())
 print('ready',flush=True);sys.stdin.readline()
 sd=ptr();check(sddl('D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;'+sid+')',1,c.byref(sd),None))
 present=w.BOOL();acl=ptr();default=w.BOOL();check(get_dacl(sd,c.byref(present),c.byref(acl),c.byref(default)))
 err=set_handle_sd(h,1,0x80000004,None,None,acl,None)
 print(err,flush=True);free(sd);close(h);sys.exit()
rows=[];out=Path(sys.argv[1]);out.parent.mkdir(parents=True,exist_ok=True)
for fresh in (True,False):
 for how in ('preopened','ancestor'):
  with tempfile.TemporaryDirectory() as tmp:
   grand=Path(tmp)/'policy';grand.mkdir();set_grant(grand,True)
   parent=grand/'target';parent.mkdir()
   sibling=grand/'ordinary-sibling';sibling.mkdir()
   assert grant(parent) and grant(sibling)
   child=None
   if how=='preopened':
    child=subprocess.Popen([sys.executable,__file__,how,parent],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    assert child.stdout.readline().strip()=='ready'
   tx=ctx(None,None,0,0,0,0,'qing limited metadata guard');assert tx and tx!=invalid
   held=None
   row={'fresh':fresh,'kind':how}
   try:
    dest=parent/'new' if fresh else parent
    if fresh:check(mkdir(None,str(dest),None,tx))
    leaf=dest/'leaf';h=create(str(leaf),0xc0000000,7,None,1,0,None,tx,None,None);assert h!=invalid,c.WinError(c.get_last_error())
    n=w.DWORD();check(write(h,b'proof',5,c.byref(n),None));check(close(h))
    held=create(str(parent),0x60180,7,None,3,0x02200000,None,tx,None,None)
    row['guard_opened']=held!=invalid;row['guard_open_error']=0 if held!=invalid else c.get_last_error()
    if held!=invalid:
     row['before']=grant(parent)
     if child:
      stdout,err=child.communicate(input='go\n',timeout=8);row['set_error']=int(stdout.strip())
     else:
      try:set_grant(grand,False);row['set_error']=0
      except OSError as e:row['set_error']=e.winerror
     row['live_grand']=grant(grand);row['live_parent']=grant(parent);row['live_sibling']=grant(sibling)
     tick=time.perf_counter_ns();done=commit(tx);row['commit_us']=(time.perf_counter_ns()-tick)//1000
     row['commit_ok']=bool(done);row['commit_error']=0 if done else c.get_last_error()
     close(held);held=None
     if leaf.exists():row['result_leaf']=grant(leaf)
     row['result_parent']=grant(parent);row['result_sibling']=grant(sibling)
    else:check(rollback(tx))
   finally:
    if held and held!=invalid:close(held)
    close(tx)
    if child and child.poll() is None:child.kill();child.wait()
   rows.append(row);out.write_text(json.dumps(rows,indent=2));print(row,flush=True)
for row in rows:
 if row['kind']=='preopened':
  assert not row['guard_opened'] and row['guard_open_error']==6800,row
 else:
  assert row['guard_opened'] and row['set_error']==0 and row['commit_ok'],row
  assert not row['live_grand'] and not row['live_sibling'] and not row['result_sibling'],row
  assert row['live_parent'] and row['result_parent'] and row['result_leaf'],row
