"""Disposable hosted-runner experiment; never a production deletion path."""
from pathlib import Path
exec(compile(Path(__file__).with_name('efs-ci-probe.py').read_text(encoding='utf-8').split('\ntry:\n',1)[0], 'efs-definitions', 'exec'))
out = Path(os.environ['GITHUB_WORKSPACE']) / 'efs-delete-pending-result.json'
K.SetFileInformationByHandle.argtypes = [W.HANDLE, C.c_int, C.c_void_p, W.DWORD]
K.SetFileInformationByHandle.restype = W.BOOL
K.WriteFile.argtypes = [W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p]
K.WriteFile.restype = W.BOOL

def open_native(path, access, share=7):
    h = K.CreateFileW(str(path), access, share, None, 3, 0x02200000, None)
    return (None, C.get_last_error()) if h in (None, C.c_void_p(-1).value) else (h, 0)

def set_ea(h, value):
    name=b'QING.EFS.TEST'
    data=b'\0'*4+bytes([0x80,len(name)])+len(value).to_bytes(2,'little')+name+b'\0'+value
    b=C.create_string_buffer(data); io=IOSB()
    return N.NtSetEaFile(h,C.byref(io),b,len(data)) & 0xffffffff

def disposition(h, pending):
    flag=W.BOOL(pending)
    ok=K.SetFileInformationByHandle(h,4,C.byref(flag),C.sizeof(flag))
    return {'ok':bool(ok),'error':0 if ok else C.get_last_error()}

def probe(base, directory, writer_kind):
    p=base / ('dir' if directory else 'file')
    if directory: p.mkdir()
    else: p.write_bytes(b'original primary content')
    Path(str(p)+':existing').write_bytes(b'original alternate content')
    require(A.EncryptFileW(str(p)), f'encrypt failed {C.get_last_error()}')
    ea(p,b'original ea')
    initial_ea=ea(p); initial_keys=keys(p)
    rec={'directory':directory,'writer':writer_kind,'encrypted':encrypted(p),'keys':len(initial_keys)}
    writer=None; source=None; pending=False
    try:
        if writer_kind=='ea': writer,err=open_native(p,0x18)
        elif writer_kind=='ads': writer,err=open_native(str(p)+':existing',0x40000000)
        else: err=0
        rec['existing_writer_open_error']=err
        source,err=open_native(p,0x80010000,1)
        rec['source_pin_open_error']=err
        if source is None: return rec
        rec['set_pending']=disposition(source,True)
        pending=rec['set_pending']['ok']
        if not pending: return rec
        for name,access,path in [('new_ea',0x18,p),('new_ads',0x40000000,str(p)+':new')]:
            h=K.CreateFileW(str(path),access,7,None,2 if name=='new_ads' else 3,0x02200000,None)
            valid=h not in (None,C.c_void_p(-1).value)
            rec[name]={'opened':valid,'error':0 if valid else C.get_last_error()}
            if valid: K.CloseHandle(h)
        if writer_kind=='ea' and writer:
            rec['existing_writer_status']=set_ea(writer,b'changed while pending')
        if writer_kind=='ads' and writer:
            data=C.create_string_buffer(b'changed while pending'); written=W.DWORD()
            ok=K.WriteFile(writer,data,len(data.raw)-1,C.byref(written),None)
            rec['existing_writer_write']={'ok':bool(ok),'error':0 if ok else C.get_last_error(),'bytes':written.value}
        pointer=C.POINTER(HashList)()
        status=A.QueryUsersOnEncryptedFile(str(p),C.byref(pointer))
        rec['pending_path_key_query_status']=status
        if pointer: A.FreeEncryptionCertificateHashList(pointer)
    finally:
        if pending:
            rec['undo_pending']=disposition(source,False)
            require(rec['undo_pending']['ok'], 'fixture rollback failed; retain handles until process exits')
        if source: K.CloseHandle(source)
        if writer: K.CloseHandle(writer)
        rec['source_preserved']=p.exists()
        require(p.exists(),'source was unexpectedly deleted')
        rec['ea_changed']=ea(p)!=initial_ea
        rec['keys_preserved']=keys(p)==initial_keys
        rec['ads_content']=Path(str(p)+':existing').read_bytes().decode('ascii')
    return rec

try:
    with tempfile.TemporaryDirectory(prefix='qing-efs-delete-pending-') as temp:
        root=Path(temp); receipt['cases']=[]
        for directory in [False,True]:
            for writer in ['none','ea','ads']:
                base=root/f'{directory}-{writer}'; base.mkdir()
                receipt['cases'].append(probe(base,directory,writer))
    receipt['status']='observed'
    receipt['production_delete_authorized']=False
except Exception as exc:
    receipt['error']=repr(exc)
    raise
finally:
    out.write_text(json.dumps(receipt,indent=2),encoding='utf-8')
