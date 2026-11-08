from pathlib import Path
import os,sys,tempfile,ctypes as c,ctypes.wintypes as w,importlib.util
here=Path(__file__).resolve().parent
ctx={'__file__':str(here/'check-move-tree-acl.py')};exec((here/'check-move-tree-acl.py').read_text(encoding='utf-8-sig').split("\nexe=Path(sys.argv[1])")[0],ctx);globals().update(ctx)
if len(sys.argv)>1:
 target=Path(sys.argv[1]);h=dbg.func(dbg.k,'CreateFileW',dbg.P,w.LPCWSTR,w.DWORD,w.DWORD,dbg.P,w.DWORD,w.DWORD,dbg.P)(str(target),0x60080,7,None,3,0x02200000,None)
 empty=(w.DWORD*2)(0x80002,0)
 q=adv.SetSecurityInfo;q.argtypes=[dbg.P,w.DWORD,w.DWORD,dbg.P,dbg.P,dbg.P,dbg.P]
 rc=q(h,1,0x20000004,None,None,c.byref(empty),None);assert rc==0,rc;dbg.close(h);sys.exit(0)
with tempfile.TemporaryDirectory() as tmp:
 d=Path(tmp);model(d,True);target=d/'leaf';target.write_bytes(b'test');apply_acl(target,False)
 ordinary=d/'ordinary';ordinary.write_bytes(b'test');print('ordinary before',grants_ls(ordinary),'private',grants_ls(target))
 def match(path):return os.path.normcase(path.removeprefix('\\\\?\\'))==os.path.normcase(str(target))
 def revoke():apply_acl(d,False)
 hit,rc,msg=dbg.observe([sys.executable,here/'test-system-reset.py',target],match,revoke,os.environ.copy())
 print('system reset result',hit,rc,msg,'parent',grants_ls(d),'leaf',grants_ls(target))

