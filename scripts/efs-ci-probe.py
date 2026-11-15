"""Native EFS feasibility probe. Never run on a developer/user machine."""
import os
if not (os.name == "nt" and os.environ.get("GITHUB_ACTIONS") == "true"
        and os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted"
        and os.environ.get("GITHUB_REPOSITORY") == "qing-yi-5427/qing-zip"):
    raise SystemExit("This probe is restricted to the disposable GitHub-hosted Windows runner.")

import ctypes as C
from ctypes import wintypes as W
import csv
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

K = C.WinDLL("kernel32", use_last_error=True)
A = C.WinDLL("advapi32", use_last_error=True)
N = C.WinDLL("ntdll")
K.CreateFileW.argtypes = [W.LPCWSTR, W.DWORD, W.DWORD, C.c_void_p, W.DWORD, W.DWORD, C.c_void_p]
K.CreateFileW.restype = W.HANDLE
K.CloseHandle.argtypes = [W.HANDLE]
K.GetFileAttributesW.argtypes = [W.LPCWSTR]
K.GetFileAttributesW.restype = W.DWORD
A.EncryptFileW.argtypes = [W.LPCWSTR]
A.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [W.LPCWSTR, W.DWORD, C.POINTER(C.c_void_p), C.c_void_p]
A.SetFileSecurityW.argtypes = [W.LPCWSTR, W.DWORD, C.c_void_p]
K.LocalFree.argtypes = [C.c_void_p]
PROGRESS = C.WINFUNCTYPE(W.DWORD, C.c_longlong, C.c_longlong, C.c_longlong,
                        C.c_longlong, W.DWORD, W.DWORD, W.HANDLE, W.HANDLE, C.c_void_p)
K.CopyFileExW.argtypes = [W.LPCWSTR, W.LPCWSTR, PROGRESS, C.c_void_p, C.POINTER(W.BOOL), W.DWORD]

class IOSB(C.Structure):
    _fields_ = [("status", C.c_size_t), ("information", C.c_size_t)]
N.NtSetEaFile.argtypes = [W.HANDLE, C.POINTER(IOSB), C.c_void_p, W.ULONG]
N.NtSetEaFile.restype = C.c_long
N.NtQueryEaFile.argtypes = [W.HANDLE, C.POINTER(IOSB), C.c_void_p, W.ULONG,
                           W.BOOLEAN, C.c_void_p, W.ULONG, C.c_void_p, W.BOOLEAN]
N.NtQueryEaFile.restype = C.c_long

receipt = {"commit": os.environ.get("GITHUB_SHA"), "run_id": os.environ.get("GITHUB_RUN_ID"),
           "runner_os": os.environ.get("RUNNER_OS"), "checks": {}, "status": "failed"}
out = Path(os.environ["GITHUB_WORKSPACE"]) / "efs-probe-result.json"

def require(value, message):
    if not value:
        raise RuntimeError(message)

def handle(path):
    h = K.CreateFileW(str(path), 0x80000000 | 0x18, 7, None, 3, 0x02200000, None)
    require(h not in (None, C.c_void_p(-1).value), f"open fixture failed: {C.get_last_error()}")
    return h

def ea(path, value=None):
    h = handle(path)
    try:
        io = IOSB()
        if value is not None:
            name = b"QING.EFS.TEST"
            data = b"\0"*4 + bytes([0x80, len(name)]) + len(value).to_bytes(2,"little") + name + b"\0" + value
            buffer = C.create_string_buffer(data)
            status = N.NtSetEaFile(h, C.byref(io), buffer, len(data))
            require(status == 0, f"NtSetEaFile: {status:#x}")
        buffer = C.create_string_buffer(65536)
        status = N.NtQueryEaFile(h, C.byref(io), buffer, len(buffer), False, None, 0, None, True)
        if status & 0xffffffff == 0xc0000052:  # STATUS_NO_EAS_ON_FILE
            return b""
        require(status == 0, f"NtQueryEaFile: {status:#x}")
        return buffer.raw[:io.information]
    finally:
        K.CloseHandle(h)

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def encrypted(path):
    attributes = K.GetFileAttributesW(str(path))
    return attributes != 0xffffffff and bool(attributes & 0x4000)

def copy(source, target, cancel=False):
    # No ALLOW_DECRYPTED_DESTINATION. This is Nana's ordinary-copy flag.
    callback = PROGRESS(lambda *unused: 1 if cancel else 0)
    cancelled = W.BOOL(False)
    result = K.CopyFileExW(str(source), str(target), callback, None, C.byref(cancelled), 1)
    return bool(result), C.get_last_error() if not result else 0

try:
    with tempfile.TemporaryDirectory(prefix="qing-efs-ci-", dir=os.environ["RUNNER_TEMP"]) as directory:
        base = Path(directory)
        private = base / "private"
        private.mkdir()
        # New fixture directory only: do not touch runner account or other paths.
        sid = next(csv.reader(subprocess.check_output(["whoami", "/user", "/fo", "csv", "/nh"], text=True).splitlines()))[1]
        sd = C.c_void_p()
        require(A.ConvertStringSecurityDescriptorToSecurityDescriptorW(
            f"D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)", 1, C.byref(sd), None), "private descriptor failed")
        try:
            require(A.SetFileSecurityW(str(private), 0x80000004, sd), "private directory protection failed")
        finally:
            K.LocalFree(sd)

        source = base / "source.bin"
        source.write_bytes(bytes(range(256)) * 4096)
        Path(str(source)+":custom").write_bytes(b"independent alternate stream")
        Path(str(source)+":Zone.Identifier").write_bytes(b"[ZoneTransfer]\r\nZoneId=3\r\n")
        expected_ea = ea(source, b"opaque EA bytes\0\xff")
        original = digest(source)
        # This can create an EFS certificate, exclusively inside this disposable runner.
        if not A.EncryptFileW(str(source)):
            receipt["unsupported_error"] = C.get_last_error()
            receipt["status"] = "unsupported"
            raise RuntimeError(f"Runner EFS is unavailable: {receipt['unsupported_error']}; not a pass")
        require(encrypted(source), "source did not become encrypted")
        receipt["checks"]["native_efs_fixture"] = True
        receipt["encryption_conversion_preserved_ea"] = ea(source) == expected_ea
        # Conversion and copying are distinct operations. Construct an already
        # encrypted file with EA even if Windows EncryptFile discarded old EA.
        expected_ea = ea(source, b"opaque EA bytes\0\xff")
        require(expected_ea, "could not construct encrypted EA fixture")

        target = private / "copied.bin"
        ok, error = copy(source, target)
        require(ok, f"CopyFileEx without plaintext fallback failed: {error}")
        require(encrypted(target), "copy silently lost EFS")
        require(digest(target) == original and digest(source) == original, "primary content changed")
        native_ea_preserved = ea(target) == expected_ea
        receipt["native_copy_preserved_ea"] = native_ea_preserved
        for stream in ["custom", "Zone.Identifier"]:
            require(Path(str(source)+":"+stream).read_bytes() == Path(str(target)+":"+stream).read_bytes(), "copy lost ADS")
        receipt["checks"]["copy_preserves_efs_primary_ads"] = True
        # Probe the necessary extra private-output step without hiding a native
        # metadata loss. Final result remains failed if native copying lost EA.
        require(ea(target, b"opaque EA bytes\0\xff") == expected_ea, "explicit EA restore failed")
        require(encrypted(target) and digest(target) == original, "EA restore changed encryption/content")
        receipt["checks"]["explicit_private_ea_restore_preserves_efs"] = True

        existing = private / "existing.bin"
        existing.write_bytes(b"existing target must survive")
        before = digest(existing)
        ok, error = copy(source, existing)
        require(not ok and error in (80,183), f"existing-target conflict was not rejected: {error}")
        require(digest(existing) == before and digest(source) == original and encrypted(source), "failed copy changed original")
        receipt["checks"]["conflict_preserves_source_and_target"] = True

        cancelled_target = private / "cancelled.bin"
        ok, error = copy(source, cancelled_target, cancel=True)
        require(not ok and error == 1235, f"cancel did not abort: {error}")
        require(not cancelled_target.exists(), "cancel left partial target")
        require(digest(source) == original and encrypted(source) and ea(source) == expected_ea, "cancel changed source")
        receipt["checks"]["cancel_removes_partial_preserves_source"] = True
        require(native_ea_preserved, "CopyFileEx EFS path lost EA; explicit private EA restore works, native-only parity is incomplete")
        receipt["status"] = "passed"
except Exception as error:
    receipt["error"] = str(error)
finally:
    out.write_text(json.dumps(receipt, indent=2)+"\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))
if receipt["status"] != "passed":
    raise SystemExit(1)
