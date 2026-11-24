"""Windows integration check: pass paths to new obz, old obz, 7z and WinRAR.
Passwords below are public test fixtures; temporary files are removed automatically.
"""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

new, old, seven, winrar = sys.argv[1:5]
def run(args, password=None):
    result = subprocess.run(args, input=None if password is None else (password + "\n").encode(), capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f"{args[0]} failed ({result.returncode}): {result.stderr.decode('utf-8', 'replace')}")

def check_output(path, expected):
    assert path.is_file() and hashlib.sha256(path.read_bytes()).digest() == expected

with tempfile.TemporaryDirectory(prefix="qz-aes-interop-") as tmp:
    root = Path(tmp)
    source = root / "文件.txt"
    source.write_bytes("互操作 fixture".encode())
    expected = hashlib.sha256(source.read_bytes()).digest()
    password = "中文密码"
    legacy, archive = root / "legacy.zip", root / "new.zip"
    run([old, "create", str(legacy), str(source), "--format", "zip", "--password-stdin"], password)
    run([new, "create", str(archive), str(source), "--format", "zip", "--password-stdin"], password)
    for label, source_archive in [("legacy", legacy), ("new", archive)]:
        dest = root / label
        run([new, "extract", str(source_archive), str(dest), "--password-stdin"], password)
        check_output(dest / source.name, expected)
    dest = root / "7z"
    run([seven, "x", str(archive), "-y", "-o" + str(dest), "-p" + password])
    check_output(dest / source.name, expected)
    dest = root / "rar"
    run([winrar, "x", "-ibck", "-y", "-p" + password, str(archive), str(dest) + os.sep])
    check_output(dest / source.name, expected)
    appended = root / "追加.txt"
    appended.write_bytes(b"added fixture")
    for label, source_archive in [("legacy", legacy), ("new", archive)]:
        run([new, "add", str(source_archive), str(appended), "--password-stdin"], password)
        dest = root / (label + "-updated")
        run([new, "extract", str(source_archive), str(dest), "--password-stdin"], password)
        check_output(dest / source.name, expected)
        check_output(dest / appended.name, hashlib.sha256(appended.read_bytes()).digest())
    for name, secret in [("ascii", "fixture123"), ("fallback", "密码🙂")]:
        archive = root / (name + ".zip")
        run([new, "create", str(archive), str(source), "--format", "zip", "--password-stdin"], secret)
        dest = root / name
        run([new, "extract", str(archive), str(dest), "--password-stdin"], secret)
        check_output(dest / source.name, expected)
print("AES ZIP interoperability passed")
