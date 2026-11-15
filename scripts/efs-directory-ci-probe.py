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
        receipt["status"]="passed"
except Exception as error:receipt["error"]=str(error)
finally:out.write_text(json.dumps(receipt,ensure_ascii=False,indent=2),encoding="utf-8")
if receipt["status"]!="passed":raise SystemExit(1)
