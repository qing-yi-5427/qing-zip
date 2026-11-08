"""Race real Windows archive edits against writers in a disposable directory."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import tempfile
import time
import zipfile

def accepted_zip(sequence):
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w") as archive:
        archive.writestr("external.txt", f"External writer {sequence}")
    return stream.getvalue()

def trial(exe, directory, template, operation, writer, repeated):
    directory.mkdir()
    path = directory / "archive.zip"
    shutil.copyfile(template, path)
    added = directory.parent / "new.txt"
    added.write_bytes(b"source")
    edits = {"add":[added], "delete":["target.txt"], "rename":["target.txt","updated.txt"]}
    child = subprocess.Popen([exe,operation,path,*edits[operation]],
                             stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    attempts = 0
    accepted = []
    deadline = time.monotonic() + 45
    try:
        while not any(item.is_dir() for item in directory.iterdir()):
            if child.poll() is not None or time.monotonic()>deadline:
                raise RuntimeError("Did not observe the real editing window")
            time.sleep(.001)
        temp = directory.parent / (directory.name + "-external-write.zip")
        content = accepted_zip(0)
        while child.poll() is None and time.monotonic()<deadline:
            attempts += 1
            try:
                if writer == "replace":
                    if not temp.exists():
                        temp.write_bytes(content)
                    os.replace(temp,path)
                else:
                    with path.open("r+b") as output:
                        output.write(content)
                        output.truncate()
            except (PermissionError, FileNotFoundError):
                continue
            accepted.append(hashlib.sha256(content).hexdigest())
            if not repeated:
                break
            content = accepted_zip(len(accepted))
        _, err = child.communicate(timeout=30)
        final = hashlib.sha256(path.read_bytes()).hexdigest()
        if accepted:
            assert final == accepted[-1], (operation,writer,attempts,accepted[-1],final)
        else:
            assert child.returncode == 0, err.decode("utf-8","replace")
            with zipfile.ZipFile(path) as archive:
                expected = {"keep.bin", "target.txt", "new.txt"} if operation=="add" else {"keep.bin","updated.txt"} if operation=="rename" else {"keep.bin"}
                assert set(archive.namelist()) == expected
        if child.returncode:
            message = err.decode("utf-8","replace")
            stages = [item for item in directory.iterdir() if item.is_dir()]
            for stage in stages:
                assert str(stage) in message
                originals = [stage/"original"/path.name,stage/"merge"/path.name]
                assert any(p.exists() and hashlib.sha256(p.read_bytes()).digest()==hashlib.sha256(template.read_bytes()).digest() for p in originals)
        return {"edit":operation,"writer":writer,"repeat":repeated,"attempts":attempts,"accepted":len(accepted),"last_external":accepted[-1] if accepted else None,"final_sha256":final,"exit":child.returncode}
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("obz",type=Path)
    parser.add_argument("--output",type=Path)
    parser.add_argument("--root",type=Path)
    args=parser.parse_args()
    exe=args.obz.resolve(strict=True)
    rows=[]
    with tempfile.TemporaryDirectory(dir=args.root) as temp:
        root=Path(temp)
        source=root/"source.zip"
        with zipfile.ZipFile(source,"w",compression=zipfile.ZIP_STORED) as archive:
            archive.writestr("keep.bin", random.Random(983).randbytes(48*1024*1024))
            archive.writestr("target.txt", b"old")
        for repeated in (False,True):
            for operation in ("add","rename","delete"):
                for writer in ("replace","inplace"):
                    rows.append(trial(exe,root/f"try-{len(rows)}",source,operation,writer,repeated))
    result=json.dumps(rows,indent=2)
    if args.output:
        args.output.write_text(result,encoding="utf-8")
    print(result)
if __name__=="__main__":
    main()
