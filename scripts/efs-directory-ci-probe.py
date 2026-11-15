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

def raw_directory(source,target,precreate=False):
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
    if precreate:target.mkdir()
    context=C.c_void_p();code=A.OpenEncryptedFileRawW(str(target),3,C.byref(context));result["open_import"]=code
    if code:return result
    offset=0
    @IMPORT
    def supply(data,unused,size):
        nonlocal offset
        count=min(size[0],len(exported)-offset)
        if count:C.memmove(data,bytes(exported[offset:offset+count]),count)
        offset+=count;size[0]=count;return 0
    try:result["write_raw"]=A.WriteEncryptedFileRaw(supply,None,context)
    finally:A.CloseEncryptedFileRaw(context)
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
    result["checks"]={"empty_import_no_recursive_surprise":not any(target.iterdir()),
      "encrypted":encrypted(target),"users_equal":keys(source)==keys(target),
      "recovery_equal":keys(source,True)==keys(target,True),"ea_equal":ea(source)==ea(target),
      "ads_equal":Path(str(target)+":meta").exists() and Path(str(target)+":meta").read_bytes()==Path(str(source)+":meta").read_bytes()}
    # Preserve a pre-existing clear child when changing directory default policy.
    plain=target/"existing-plain.txt";plain.write_bytes(b"test")
    result["new_child_encrypted"]=encrypted(plain)
    return result

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
        receipt["source_after_raw"]=state(src)
        receipt["raw_checks"]={label:bool(item.get("checks")) and all(item["checks"].values()) and item.get("new_child_encrypted") is True
            for label,item in [("new",receipt["raw_directory_new"]),("existing",receipt["raw_directory_existing"])]}
        require(all(receipt["raw_checks"].values()),"raw directory full preservation failed")
        require(receipt["source_after_raw"]==receipt["encrypted_source"]["directory"],"source directory changed")
        receipt["status"]="passed"
except Exception as error:receipt["error"]=str(error)
finally:out.write_text(json.dumps(receipt,ensure_ascii=False,indent=2),encoding="utf-8")
if receipt["status"]!="passed":raise SystemExit(1)
