"""Disposable-runner-only directory policy probe; no production routing."""
from pathlib import Path
# Load only the guarded native helper definitions, never its standalone probe.
source = Path(__file__).with_name("efs-ci-probe.py").read_text(encoding="utf-8")
assert "\ntry:\n" in source
exec(compile(source.split("\ntry:\n",1)[0], "efs-native-definitions", "exec"))
import struct
K.BackupRead.argtypes=[W.HANDLE,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),W.BOOL,W.BOOL,C.POINTER(C.c_void_p)]
K.BackupRead.restype=W.BOOL

def backup_records(path):
    h=handle(path);context=C.c_void_p();result=[]
    def read(count):
        data=C.create_string_buffer(count);actual=W.DWORD()
        if not K.BackupRead(h,data,count,C.byref(actual),False,False,C.byref(context)):
            raise RuntimeError("BackupRead error "+str(C.get_last_error()))
        return data.raw[:actual.value]
    try:
        while True:
            header=read(20)
            if not header:break
            require(len(header)==20,"short backup header")
            kind,flags,size,name_len=struct.unpack("<IIqI",header)
            require(0<=size<=16*1024*1024 and name_len<=65536,"unbounded test backup record")
            name=read(name_len).decode("utf-16-le") if name_len else ""
            total=0;sha=hashlib.sha256()
            while total<size:
                chunk=read(min(size-total,65536));require(chunk,"short backup data");sha.update(chunk);total+=len(chunk)
            result.append({"kind":kind,"flags":flags,"length":size,"name":name,"sha256":sha.hexdigest()})
        return {"records":result}
    except Exception as error:return {"error":str(error),"records":result}
    finally:
        actual=W.DWORD();K.BackupRead(h,None,0,C.byref(actual),True,False,C.byref(context));K.CloseHandle(h)

EXPORT=C.WINFUNCTYPE(W.DWORD,C.c_void_p,C.c_void_p,W.ULONG)
IMPORT=C.WINFUNCTYPE(W.DWORD,C.c_void_p,C.c_void_p,C.POINTER(W.ULONG))
A.OpenEncryptedFileRawW.argtypes=[W.LPCWSTR,W.ULONG,C.POINTER(C.c_void_p)]
A.OpenEncryptedFileRawW.restype=W.DWORD
A.ReadEncryptedFileRaw.argtypes=[EXPORT,C.c_void_p,C.c_void_p]
A.ReadEncryptedFileRaw.restype=W.DWORD
A.WriteEncryptedFileRaw.argtypes=[IMPORT,C.c_void_p,C.c_void_p]
A.WriteEncryptedFileRaw.restype=W.DWORD
A.CloseEncryptedFileRaw.argtypes=[C.c_void_p]
A.CloseEncryptedFileRaw.restype=None

def raw_directory(source,target,precreate=False,lock_target=False):
    result={"flags_export":0,"flags_import":3,"precreated":precreate}
    exported=bytearray();context=C.c_void_p()
    @EXPORT
    def take(data,unused,size):
        if len(exported)+size>4*1024*1024:return 8
        exported.extend(C.string_at(data,size));return 0
    code=A.OpenEncryptedFileRawW(str(source),0,C.byref(context));result["open_export"]=code
    if code:return result
    try:result["read_raw"]=A.ReadEncryptedFileRaw(take,None,context)
    finally:A.CloseEncryptedFileRaw(context)
    result["export_bytes"]=len(exported)
    if result["read_raw"]:return result
    sentinel_before={}
    if precreate:target.mkdir()
    if precreate=="nonempty":
        for name,secret in [("plain-sentinel.txt",False),("encrypted-sentinel.txt",True)]:
            p=target/name;p.write_bytes(("owned disposable "+name).encode())
            if secret:require(A.EncryptFileW(str(p)),"encrypt sentinel failed")
            sentinel_before[name]={"digest":digest(p),"encrypted":encrypted(p),"keys":[b.hex() for b in keys(p)] if secret else []}
    result["sentinel_before"]=sentinel_before
    locked=None
    if lock_target:
        if not target.exists():target.mkdir()
        access,sharing=lock_target if isinstance(lock_target,tuple) else (0x1f0199,0)
        result["target_pin"]={"access":hex(access),"share":sharing}
        locked=K.CreateFileW(str(target),access,sharing,None,3,0x02200000,None)
        require(locked not in (None,C.c_void_p(-1).value),"exclusive directory fixture open failed")
    context=C.c_void_p();code=A.OpenEncryptedFileRawW(str(target),3,C.byref(context));result["open_import"]=code
    if code:
        if locked is not None:K.CloseHandle(locked)
        return result
    offset=0
    @IMPORT
    def supply(data,unused,size):
        nonlocal offset
        count=min(size[0],len(exported)-offset)
        if count:C.memmove(data,bytes(exported[offset:offset+count]),count)
        offset+=count;size[0]=count;return 0
    try:result["write_raw"]=A.WriteEncryptedFileRaw(supply,None,context)
    finally:
        A.CloseEncryptedFileRaw(context)
        if locked is not None:K.CloseHandle(locked)
    result["consumed_bytes"]=offset
    if result["write_raw"]:return result
    result["native_directory"]=state(target)
    source_ea=ea(source)
    h=handle(target)
    try:
        io=IOSB();data=C.create_string_buffer(source_ea)
        code=N.NtSetEaFile(h,C.byref(io),data,len(source_ea));result["restore_ea"]=code
        require(code==0,"raw directory EA restore failed")
    finally:K.CloseHandle(h)
    result["directory"]=state(target)
    result["children_before_explicit_copy"]=[p.name for p in target.iterdir()]
    sentinel_after={name:{"digest":digest(target/name),"encrypted":encrypted(target/name),
        "keys":[b.hex() for b in keys(target/name)] if encrypted(target/name) else []} for name in sentinel_before}
    result["sentinel_after"]=sentinel_after
    result["checks"]={"no_recursive_surprise":set(p.name for p in target.iterdir())==set(sentinel_before),
      "sentinels_unchanged":sentinel_before==sentinel_after,
      "encrypted":encrypted(target),"users_equal":keys(source)==keys(target),
      "recovery_equal":keys(source,True)==keys(target,True),"ea_equal":ea(source)==ea(target),
      "ads_equal":Path(str(target)+":meta").exists() and Path(str(target)+":meta").read_bytes()==Path(str(source)+":meta").read_bytes()}
    # Preserve a pre-existing clear child when changing directory default policy.
    plain=target/"existing-plain.txt";plain.write_bytes(b"test")
    result["new_child_encrypted"]=encrypted(plain)
    return result

def raw_pinned(source,target,lock_target=False):
    pin=K.CreateFileW(str(source),0x80000000,1,None,3,0x02200000,None)
    require(pin not in (None,C.c_void_p(-1).value),"source directory read pin failed")
    try:return raw_directory(source,target,False,lock_target)
    finally:K.CloseHandle(pin)

def pin_matrix(source):
    results=[]
    for access in [0x80000000,0x20080]:
        for share in [1,3,5,7]:
            pin=K.CreateFileW(str(source),access,share,None,3,0x02200000,None)
            require(pin not in (None,C.c_void_p(-1).value),"matrix pin failed")
            context=C.c_void_p()
            try:
                code=A.OpenEncryptedFileRawW(str(source),0,C.byref(context))
                item={"access":hex(access),"share":share,"open_export":code}
                if code==0:
                    size=0
                    @EXPORT
                    def consume(data,unused,length):
                        nonlocal size
                        size+=length;return 0 if size<=4*1024*1024 else 8
                    try:item["read_raw"]=A.ReadEncryptedFileRaw(consume,None,context);item["bytes"]=size
                    finally:A.CloseEncryptedFileRaw(context)
                results.append(item)
            finally:K.CloseHandle(pin)
    return results

def guarded_export(source,inject_new=False):
    before=state(source)
    result={"injected_new_stream":inject_new};pins=[];context=C.c_void_p()
    K.MoveFileExW.argtypes=[W.LPCWSTR,W.LPCWSTR,W.DWORD];K.MoveFileExW.restype=W.BOOL
    K.DeviceIoControl.argtypes=[W.HANDLE,W.DWORD,C.c_void_p,W.DWORD,C.c_void_p,W.DWORD,C.POINTER(W.DWORD),C.c_void_p];K.DeviceIoControl.restype=W.BOOL
    try:
        for p,access,share in [(source,0x20088,1)]:
            h=K.CreateFileW(str(p),access,share,None,3,0x02200000,None)
            require(h not in (None,C.c_void_p(-1).value),"guarded source pin failed")
            pins.append(h)
        code=A.OpenEncryptedFileRawW(str(source),0,C.byref(context));result["open_export"]=code
        if code:return result
        rename=bool(K.MoveFileExW(str(source),str(source)+"-renamed",0));result["rename_blocked"]=not rename;result["rename_error"]=C.get_last_error() if not rename else 0
        if rename:require(K.MoveFileExW(str(source)+"-renamed",str(source),0),"restore disposable rename failed")
        writer=K.CreateFileW(str(source)+":meta",0x40000000,7,None,3,0,None)
        result["ads_write_blocked"]=writer in (None,C.c_void_p(-1).value);result["ads_write_error"]=C.get_last_error() if result["ads_write_blocked"] else 0
        if not result["ads_write_blocked"]:K.CloseHandle(writer)
        # Empty and nonempty directories are tested separately. This mounts only
        # a sibling disposable target and is removed by its original handle.
        junction=source.parent/(source.name+"-junction-target");junction.mkdir()
        writer=K.CreateFileW(str(source),0x40000000,7,None,3,0x02200000,None)
        if writer in (None,C.c_void_p(-1).value):result["reparse_blocked"]=True;result["reparse_error"]=C.get_last_error()
        else:
            substitute=("\\??\\"+str(junction)).encode("utf-16-le");display=str(junction).encode("utf-16-le")
            names=substitute+b"\0\0"+display+b"\0\0";payload=struct.pack("<HHHH",0,len(substitute),len(substitute)+2,len(display))+names
            data=C.create_string_buffer(struct.pack("<IHH",0xA0000003,len(payload),0)+payload);returned=W.DWORD()
            changed=bool(K.DeviceIoControl(writer,0x900a4,data,len(data)-1,None,0,C.byref(returned),None))
            result["reparse_blocked"]=not changed;result["reparse_error"]=C.get_last_error() if not changed else 0
            if changed:
                deletion=C.create_string_buffer(struct.pack("<IHH",0xA0000003,0,0));require(K.DeviceIoControl(writer,0x900ac,deletion,8,None,0,C.byref(returned),None),"remove disposable reparse failed")
            K.CloseHandle(writer)
        total=0
        @EXPORT
        def consume(data,unused,length):
            nonlocal total
            total+=length;return 0 if total<=4*1024*1024 else 8
        result["read_raw"]=A.ReadEncryptedFileRaw(consume,None,context);result["bytes"]=total
        writer=K.CreateFileW(str(source)+":meta",0x40000000,7,None,3,0,None)
        result["ads_write_after_read_blocked"]=writer in (None,C.c_void_p(-1).value)
        if not result["ads_write_after_read_blocked"]:K.CloseHandle(writer)
        if inject_new:
            writer=K.CreateFileW(str(source)+":new-during-raw",0x40000000,7,None,1,0,None)
            result["new_ads_blocked"]=writer in (None,C.c_void_p(-1).value)
            if not result["new_ads_blocked"]:K.CloseHandle(writer)
        result["crypto_queries_during_raw"]={}
        for recovery in [False,True]:
            try:result["crypto_queries_during_raw"][str(recovery)]=[v.hex() for v in keys(source,recovery)]
            except Exception as e:result["crypto_queries_during_raw"][str(recovery)]=str(e)
        info=IOSB();buffer=C.create_string_buffer(65536)
        result["ea_query_during_raw"]=N.NtQueryEaFile(pins[0],C.byref(info),buffer,len(buffer),False,None,0,None,True)
        # Raw owns existing stream data locks, but does not freeze stream enumeration.
        # Reacquire read locks and compare the entire source after closing its lease.
        A.CloseEncryptedFileRaw(context);context=C.c_void_p()
        for entry in before["backup"]["records"]:
            if entry["kind"]!=4:continue
            h=K.CreateFileW(str(source)+entry["name"],0x80000000,1,None,3,0,None)
            require(h not in (None,C.c_void_p(-1).value),"source ADS repin failed")
            pins.append(h)
        after=state(source)
        result["source_equal_after_repin"]=before==after
        result["copy_eligible"]=all(result[k] for k in ["rename_blocked","ads_write_blocked","reparse_blocked","ads_write_after_read_blocked","source_equal_after_repin"]) and result["read_raw"]==0
        result["source_retained"]=source.is_dir()
        return result
    finally:
        if context.value:A.CloseEncryptedFileRaw(context)
        for h in reversed(pins):K.CloseHandle(h)

def state(path):
    value={"encrypted":encrypted(path),"attributes":K.GetFileAttributesW(str(path)),"backup":backup_records(path),"ea":ea(path).hex()}
    for recovery in [False,True]:
        try:value["recovery" if recovery else "users"]=[b.hex() for b in keys(path,recovery)]
        except Exception as error:value["recovery_error" if recovery else "users_error"]=str(error)
    return value

out=Path(os.environ["GITHUB_WORKSPACE"])/"efs-directory-result.json"
receipt={"run":os.environ["GITHUB_RUN_ID"],"commit":os.environ["GITHUB_SHA"],"status":"failed",
 "reference_kind":"source-algorithm reproduction: CreateComplexDir + CopyFileEx FAIL_IF_EXISTS; not Nana GUI binary",
 "source_evidence":"Modern-FSFolderCopy.cpp CopyFolder:497-564 and CopyFile_NT:278-284"}
try:
    with tempfile.TemporaryDirectory(prefix="qing-efs-dir-") as base:
        base=Path(base);src=base/"source";src.mkdir()
        before=src/"before.txt";before.write_bytes(b"created before directory flag")
        Path(str(src)+":meta").write_bytes(b"directory named data")
        ea(src,b"directory-ea")
        receipt["before_encrypt"]=state(src)
        require(A.EncryptFileW(str(src)),"EncryptFileW directory failed "+str(C.get_last_error()))
        after=src/"after.txt";after.write_bytes(b"created after directory flag")
        receipt["encrypted_source"]={"directory":state(src),"before":state(before),"after":state(after)}
        dst=base/"reference-copy";dst.mkdir()
        for child in [before,after]:
            ok,error=copy(child,dst/child.name);require(ok,"reference CopyFileEx failed "+str(error))
        receipt["reference_copy"]={"directory":state(dst),"before":state(dst/before.name),"after":state(dst/after.name)}
        require(A.EncryptFileW(str(dst)),"restore directory encryption policy failed")
        receipt["after_directory_policy_restore"]={"directory":state(dst),"before":state(dst/before.name),"after":state(dst/after.name)}
        future=dst/"future.txt";future.write_bytes(b"future default policy")
        receipt["future_child"]=state(future)
        receipt["checks"]={"directory_policy_flag":encrypted(src),"existing_plain_child_unchanged":not encrypted(before),
          "new_child_encrypted":encrypted(after),"reference_existing_plain_child_unchanged":not encrypted(dst/before.name),
          "reference_encrypted_child_preserved":encrypted(dst/after.name),"future_child_encrypted":encrypted(future),
          "source_directory_ads_preserved":Path(str(src)+":meta").read_bytes()==b"directory named data",
          "all_child_bytes_match":all((dst/p.name).read_bytes()==p.read_bytes() for p in [before,after])}
        require(all(receipt["checks"].values()),"directory policy expectation failed")
        receipt["raw_directory_new"]=raw_directory(src,base/"raw-new")
        receipt["raw_directory_existing"]=raw_directory(src,base/"raw-existing",True)
        receipt["raw_directory_nonempty"]=raw_directory(src,base/"raw-nonempty","nonempty")
        receipt["raw_source_pinned"]=raw_pinned(src,base/"raw-source-pinned")
        receipt["raw_exclusive_target"]=raw_pinned(src,base/"raw-exclusive-target",True)
        receipt["target_metadata_pins"]={str(share):raw_directory(src,base/("target-meta-"+str(share)),False,(0x20088,share)) for share in [1,3,7]}
        receipt["source_pin_matrix"]=pin_matrix(src)
        receipt["guarded_nonempty"]=guarded_export(src)
        empty=base/"guarded-empty";empty.mkdir();require(A.EncryptFileW(str(empty)),"encrypt empty directory failed")
        Path(str(empty)+":meta").write_bytes(b"empty directory ads")
        receipt["guarded_empty"]=guarded_export(empty)
        injected=base/"guarded-injected";injected.mkdir()
        require(A.EncryptFileW(str(injected)),"encrypt injected directory failed")
        Path(str(injected)+":meta").write_bytes(b"injected fixture existing ADS")
        receipt["guarded_injected"]=guarded_export(injected,True)
        require(receipt["guarded_nonempty"].get("copy_eligible"),"clean nonempty handoff failed")
        require(receipt["guarded_empty"].get("copy_eligible"),"clean empty handoff failed")
        require(receipt["guarded_injected"].get("source_retained") and not receipt["guarded_injected"].get("copy_eligible"),"changed stream inventory not rejected")
        receipt["source_after_raw"]=state(src)
        receipt["raw_checks"]={label:bool(item.get("checks")) and all(item["checks"].values()) and item.get("new_child_encrypted") is True
            for label,item in [("new",receipt["raw_directory_new"]),("existing",receipt["raw_directory_existing"]),("nonempty",receipt["raw_directory_nonempty"])]}
        require(all(receipt["raw_checks"].values()),"raw directory full preservation failed")
        require(receipt["source_after_raw"]==receipt["encrypted_source"]["directory"],"source directory changed")
        receipt["status"]="passed"
except Exception as error:receipt["error"]=str(error)
finally:out.write_text(json.dumps(receipt,ensure_ascii=False,indent=2),encoding="utf-8")
if receipt["status"]!="passed":raise SystemExit(1)
