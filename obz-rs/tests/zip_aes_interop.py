"""Windows x64 integration: python zip_aes_interop.py NEW_OBZ 7Z [OLD_OBZ] [WINRAR].
A non-ASCII password is chosen for the current OEM codepage. Passwords are
public test fixtures; temporary files are removed automatically.
"""
import ctypes
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile

if len(sys.argv) < 3 or len(sys.argv) > 5:
    raise SystemExit(__doc__)
new, seven = sys.argv[1:3]
old = sys.argv[3] if len(sys.argv) >= 4 else None
winrar = sys.argv[4] if len(sys.argv) >= 5 else None

def run(args, password=None):
    result = subprocess.run(args, input=None if password is None else (password + "\n").encode(), capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(f"{args[0]} failed ({result.returncode}): {result.stderr.decode('utf-8', 'replace')}")

def check_output(path, expected):
    assert path.is_file() and hashlib.sha256(path.read_bytes()).digest() == expected

def create(tool, archive, source, password):
    run([tool, "create", str(archive), str(source), "--format", "zip", "--password-stdin"], password)

def extract(tool, archive, dest, password):
    run([tool, "extract", str(archive), str(dest), "--password-stdin"], password)

with tempfile.TemporaryDirectory(prefix="qz-aes-interop-") as tmp:
    root = Path(tmp)
    source = root / "file.txt"
    source.write_bytes("互操作 fixture".encode())
    expected = hashlib.sha256(source.read_bytes()).digest()
    cases = [("ascii", "fixture123"), ("fallback", "密码🙂")]
    codepage = ctypes.windll.kernel32.GetOEMCP()
    if codepage == 65001:
        print("SKIP non-ASCII encoding difference: OEM is UTF-8")
    else:
        candidates = ["中文密码", "café", "日本語", "한국어", "пароль", "κωδικός", "şifre", "סיסמה", "كلمة", "รหัส"]
        for candidate in candidates:
            try:
                encoded = candidate.encode(f"cp{codepage}", errors="strict")
                if encoded.decode(f"cp{codepage}", errors="strict") == candidate and encoded != candidate.encode("utf-8"):
                    cases.append(("oem", candidate))
                    print(f"Testing non-ASCII password with OEM codepage {codepage}")
                    break
            except (LookupError, UnicodeError):
                continue
        else:
            raise RuntimeError(f"No representable non-ASCII fixture for OEM codepage {codepage}")
    for name, password in cases:
        archive = root / (name + ".zip")
        create(new, archive, source, password)
        dest = root / (name + "-new")
        extract(new, archive, dest, password)
        check_output(dest / source.name, expected)
        if name != "fallback":
            dest = root / (name + "-7z")
            run([seven, "x", str(archive), "-y", "-o" + str(dest), "-p" + password])
            check_output(dest / source.name, expected)
            if winrar and (name == "ascii" or (name == "oem" and codepage == 936)):
                dest = root / (name + "-rar")
                run([winrar, "x", "-ibck", "-y", "-p" + password, str(archive), str(dest) + os.sep])
                check_output(dest / source.name, expected)
        if name == "oem":
            archives = [("new", archive)]
            if old:
                legacy = root / "legacy.zip"
                create(old, legacy, source, password)
                archives.append(("legacy", legacy))
            appended = root / "appended.txt"
            appended.write_bytes(b"added fixture")
            for label, source_archive in archives:
                run([new, "add", str(source_archive), str(appended), "--password-stdin"], password)
                dest = root / (label + "-updated")
                extract(new, source_archive, dest, password)
                check_output(dest / source.name, expected)
                check_output(dest / appended.name, hashlib.sha256(appended.read_bytes()).digest())
print("Available AES ZIP interoperability checks passed")
