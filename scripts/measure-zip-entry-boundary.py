"""Measure selected extraction near the classic ZIP end-record count boundary."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time
import zipfile


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f,"sha256").hexdigest()


def measure(cli,archive,output,target,expected,env):
    start=time.perf_counter()
    proc=subprocess.run([cli,"extract",archive,output,"--include",target,"--threads","4"],
                        stdin=subprocess.DEVNULL,capture_output=True,env=env,
                        cwd=archive.parent,timeout=240,creationflags=subprocess.CREATE_NO_WINDOW)
    elapsed=time.perf_counter()-start
    assert proc.returncode==0, proc.stderr[-500:]
    assert json.loads(proc.stdout)["files"]==1
    files=[p.relative_to(output).as_posix() for p in output.rglob("*") if p.is_file()]
    assert files==[target] and (output/target).read_bytes()==expected
    return round(elapsed*1000,2)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline",type=Path,required=True)
    parser.add_argument("--candidate",type=Path)
    parser.add_argument("--out",required=True,type=Path)
    parser.add_argument("--counts",nargs="+",type=int,choices=[65534,65535,65536],default=[65534,65535,65536])
    parser.add_argument("--rounds",type=int,choices=range(1,8),default=3)
    args=parser.parse_args()
    engines={"baseline":args.baseline.resolve(strict=True)}
    if args.candidate: engines["candidate"]=args.candidate.resolve(strict=True)
    result={"system":platform.platform(),"cpu":os.environ.get("PROCESSOR_IDENTIFIER"),
            "cli_sha256":{k:digest(v) for k,v in engines.items()},
            "cache":"one warm-up per engine and workload; alternate run order; OS cache retained","cases":[]}
    with tempfile.TemporaryDirectory(prefix="qz-entry-count-timing-") as d, args.out.open("x",encoding="utf8") as out:
        root=Path(d)
        env=os.environ.copy()
        for key in ("TEMP","TMP","APPDATA","LOCALAPPDATA"):
            path=root/key
            path.mkdir()
            env[key]=str(path)
        for count in args.counts:
            archive=root/f"receipts-{count}.zip"
            with zipfile.ZipFile(archive,"x",zipfile.ZIP_STORED) as z:
                for i in range(count):
                    target=f"records/{i//256:03d}/item-{i:05d}.txt"
                    data=f"record {i}\n".encode("ascii")
                    z.writestr(zipfile.ZipInfo(target,(2024,6,1,10,0,0)),data)
            before=digest(archive)
            samples=[]
            for n in range(args.rounds+1):
                order=list(engines) if n%2==1 else list(reversed(engines))
                row={k:measure(engines[k],archive,root/f"out-{count}-{k}-{n}",target,data,env) for k in order}
                assert digest(archive)==before
                print(count,"warm" if n==0 else n,row,flush=True)
                if n:samples.append(row)
            result["cases"].append({"count":count,"source_sha256":before,"samples_ms":samples,
                                    "medians_ms":{k:statistics.median(row[k] for row in samples) for k in engines}})
            out.seek(0);json.dump(result,out,indent=2);out.truncate();out.flush()

if __name__=="__main__":
    main()
