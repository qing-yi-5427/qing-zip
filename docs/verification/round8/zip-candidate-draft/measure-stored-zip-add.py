"""Paired read/verify/commit cost for adding a small file to an ordinary stored ZIP."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import tempfile
import time
import zipfile


def digest_file(p):
    with p.open("rb") as f:
        return hashlib.file_digest(f,"sha256").hexdigest()


def sample(exe, root, archive, source, note, env, member_hash):
    shutil.copyfile(source, archive)
    started=time.perf_counter_ns()
    subprocess.run([exe,"add",archive,note],cwd=root,env=env,stdin=subprocess.DEVNULL,
                   capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW,timeout=180,check=True)
    elapsed=(time.perf_counter_ns()-started)/1e6
    with zipfile.ZipFile(archive) as z:
        assert set(z.namelist())=={"existing.bin",note.name}
        with z.open("existing.bin") as f:
            assert hashlib.file_digest(f,"sha256").hexdigest()==member_hash
        assert z.read(note.name)==note.read_bytes()
    return {"ms":round(elapsed,3),"zip_sha256":digest_file(archive)}


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--previous",required=True,type=Path)
    ap.add_argument("--candidate",required=True,type=Path)
    ap.add_argument("--out",required=True,type=Path)
    args=ap.parse_args()
    engines={k:getattr(args,k).resolve(strict=True) for k in ("previous","candidate")}
    report={"os":platform.platform(),"cli_sha256":{k:digest_file(exe) for k,exe in engines.items()},
            "cache":"warm-up pair, alternating five pairs, OS cache retained; verify outside timer","cases":[]}
    with args.out.open("x",encoding="utf-8") as receipt,tempfile.TemporaryDirectory(prefix="qing-zip-bulk-append-") as folder:
        root=Path(folder)
        normal=bytearray(random.Random(312).randbytes(1024*1024))
        note=root/"added-note.txt"
        note.write_bytes(b"normal text\n"*128)
        os.utime(note,(1700000000,1700000000))
        archive=root/"working.zip"
        env=os.environ.copy()
        for var in ("LOCALAPPDATA","APPDATA","TEMP","TMP"):
            sub=root/var
            sub.mkdir()
            env[var]=str(sub)
        for size in (128,512):
            src=root/f"source-{size}.zip"
            check=hashlib.sha256()
            with zipfile.ZipFile(src,"x",zipfile.ZIP_STORED) as z:
                info=zipfile.ZipInfo("existing.bin",(2024,1,1,10,0,0))
                with z.open(info,"w") as output:
                    for _ in range(size):
                        output.write(normal);check.update(normal)
            sha=check.hexdigest()
            before=digest_file(src)
            for exe in engines.values():sample(exe,root,archive,src,note,env,sha)
            pairs=[]
            for n in range(5):
                order=list(engines) if n%2==0 else list(reversed(engines))
                row={k:sample(engines[k],root,archive,src,note,env,sha) for k in order}
                if row["previous"]["zip_sha256"]!=row["candidate"]["zip_sha256"]:raise RuntimeError("Archive bytes diverged")
                pairs.append(row)
            assert digest_file(src)==before
            deltas=[r["candidate"]["ms"]-r["previous"]["ms"] for r in pairs]
            report["cases"].append({"mib":size,"source_sha256":before,"pairs":pairs,
                                    "medians_ms":{v:statistics.median(row[v]["ms"] for row in pairs) for v in engines},
                                    "paired_delta_ms":{"min":min(deltas),"median":statistics.median(deltas),"max":max(deltas)}})
            print(size,report["cases"][-1]["medians_ms"],flush=True)
        json.dump(report,receipt,indent=2)

if __name__=="__main__":main()
