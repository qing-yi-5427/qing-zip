"""Compare normal ZIP creation, safe compatibility dispatch and native Windows presentation."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile


def run(args, success=True, profile=False):
    env = dict(os.environ)
    if profile:
        env["OPEN_ARCHIVE_PROFILE"] = "1"
    result = subprocess.run(list(map(str, args)), capture_output=True, env=env, timeout=180)
    if (result.returncode == 0) != success:
        raise RuntimeError((args, result.returncode, result.stderr.decode(errors="replace")))
    return result.stderr.decode("utf-8", "replace")


def digest(p):
    with p.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--console", type=Path, required=True)
    parser.add_argument("--require-file-symlink", action="store_true")
    parser.add_argument("--workspace", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    console = args.console.resolve(strict=True)
    repo = Path(__file__).resolve().parents[1]
    cases = []
    with tempfile.TemporaryDirectory(prefix="qing-zip-create-", dir=args.workspace) as tmp:
        root = Path(tmp).resolve()
        src = root / "source"
        (src / "empty").mkdir(parents=True)
        (src / "empty.txt").touch()
        bulk = src / "wrappers" / "level-2" / "bulk"
        bulk.mkdir(parents=True)
        for n in range(2048):
            (bulk / f"f{n:04d}.txt").write_bytes(b"file")
        small = root / "small"; small.mkdir()
        (small / "one.txt").write_bytes(b"ordinary")
        little = root / "little.zip"
        errors = run([cli, "create", little, small], profile=True)
        assert "OA_CREATE_WORKER status=encoding" not in errors and "OA_PHASE engine-process" in errors
        cases.append(["small-directory", "ordinary console"])

        nested = src / "folder"; nested.mkdir()
        (src / "folder.rs").write_bytes(b"directory prefix ordering")
        (nested / ("mixed-" + chr(0x4E2D) + chr(0x1F4E6) + ".txt")).write_bytes(b"real data\n" * 1000)
        # Actual source files exercise nontrivial content without any reference installer.
        shutil.copy2(repo / "obz-rs/src/engine_stream.rs", src / "stream.rs")
        shutil.copy2(repo / "obz-rs/native/nana/process.cpp", nested / "process.cpp")
        flags = src / "hidden-sys-ro.txt"
        flags.write_bytes(b"stored Windows attributes")
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        assert kernel.SetFileAttributesW(str(flags), 0x27)
        options = [
            ("default", [], []),
            ("level-1", ["--level", "1"], ["-mx=1"]),
            ("deflate-4", ["--method", "Deflate", "--level", "5", "--threads", "4"], ["-mm=Deflate", "-mx=5", "-mmt=4"]),
            ("deflate-9", ["--method", "Deflate", "--level", "9", "--threads", "1"], ["-mm=Deflate", "-mx=9", "-mmt=1"]),
        ]
        try:
            for label, extra, reference in options:
                out, official = root / (label + ".zip"), root / (label + "-official.zip")
                stderr = run([cli, "create", out, src, "--format", "zip", *extra], profile=True)
                assert "OA_CREATE_WORKER status=complete" in stderr, (label, stderr)
                run([console, "a", "-tzip", "-y", "-sse", "-spd", *reference, "--", official, src])
                assert digest(out) == digest(official), (label, out.stat().st_size, official.stat().st_size)
                with zipfile.ZipFile(out) as archive:
                    assert archive.testzip() is None
                    observed = {p.filename for p in archive.infolist() if not p.is_dir()}
                    expected = {p.relative_to(root).as_posix() for p in src.rglob("*") if p.is_file()}
                    assert observed == expected
                    assert all(archive.read(name) == (root/name).read_bytes() for name in expected)
                    stored = archive.getinfo("source/hidden-sys-ro.txt")
                    assert stored.external_attr & 7 == 7
                cases.append([label, digest(out)])
            # Plain uncompressed bytes fit ZIP32, but the *combined* conservative
            # payload/metadata budget does not. The old writer must still succeed.
            budget_file = src / "budget.bin"
            length = 480 << 20
            with budget_file.open("wb") as f:
                f.truncate(length)
            assert kernel.GetFileAttributesW(str(budget_file)) & 0x200 == 0
            out = root / "budget.zip"
            errors = run([cli, "create", out, src, "--level","1","--threads","1"], profile=True)
            assert "OA_CREATE_WORKER status=encoding" not in errors and "OA_PHASE engine-process" in errors
            with zipfile.ZipFile(out) as archive:
                assert archive.getinfo("source/budget.bin").file_size == length
                assert archive.testzip() is None
            budget_file.unlink()
            cases.append(["classic-layout-upper-bound","console before encoding; valid full payload"])
            # A replaceable directory's writer handle can block our stronger pin.
            # The ordinary console still works; assert this with both processes.
            kernel.CreateFileW.restype = ctypes.c_void_p
            kernel.CloseHandle.argtypes = [ctypes.c_void_p]
            for label, path in [("root-write-sharing", src), ("nested-write-sharing", nested)]:
                pin = kernel.CreateFileW(str(path), 0x2, 7, None, 3, 0x02000000, None)
                assert pin != ctypes.c_void_p(-1).value, ctypes.get_last_error()
                try:
                    out = root / (label+".zip")
                    stderr = run([cli, "create", out, src, "--format", "zip"], profile=True)
                    assert "OA_CREATE_WORKER status=complete" not in stderr, stderr
                    other = root / (label+"-console.zip")
                    run([console, "a", "-tzip", "--", other, src])
                    assert digest(out) == digest(other)
                    cases.append([label, "console fallback"])
                finally:
                    kernel.CloseHandle(pin)
            # A locked *file* is different: both engines reject it. Once native
            # encoding starts, do not try the console a second time.
            file_lock = kernel.CreateFileW(str(src/"stream.rs"), 0x40000000, 7, None, 3, 0, None)
            assert file_lock != ctypes.c_void_p(-1).value, ctypes.get_last_error()
            try:
                target = root/"writer-conflict.zip"
                errors = run([cli, "create", target, src], False, True)
                assert "OA_CREATE_WORKER status=encoding" in errors and "OA_PHASE engine-process" not in errors
                assert not target.exists()
                run([console, "a", "-sse", "-tzip", "--", root/"ref-conflict.zip", src], False)
                cases.append(["file-write-sharing", "both reject, no retry"])
            finally:
                kernel.CloseHandle(file_lock)
            for label, extra in [("store", ["--method", "Store"]), ("split",["--volume","5000"])]:
                out = root/(label+".zip"); errors=run([cli,"create",out,src,"--format","zip",*extra],profile=True)
                assert "OA_CREATE_WORKER status=complete" not in errors
                run([console,"t",out if label=="store" else Path(str(out)+".001")])
                cases.append([label,"console fallback"])
            link=src/"outside-link"; outside=root/"outside.txt"; outside.write_bytes(b"outside")
            try:
                link.symlink_to(outside)
            except OSError as e:
                if args.require_file_symlink or e.winerror!=1314:raise
                cases.append(["file-symlink","unavailable (1314)"])
            else:
                try:
                    target=root/"unsafe.zip";errors=run([cli,"create",target,src],False,True)
                    assert "linked input path rejected" in errors and "OA_PHASE engine-process" not in errors and not target.exists(),errors
                    cases.append(["file-symlink","rejected before encoding, no retry"])
                finally:link.unlink()
            if args.require_file_symlink:
                # Replace a regular manifest entry after the native encoder starts.
                # This tests the worker even on hosts whose enumeration happens to
                # discover the static link during the bounded routing sample.
                head = src / "000-first.bin"
                head.write_bytes(hashlib.shake_256(b"native-create-symlink").digest(8 << 20))
                tail = src / "zzz-last.txt"; tail.write_bytes(b"original")
                bad = root / "in-flight-link.zip"
                with subprocess.Popen(list(map(str, [cli, "create", bad, src, "--level", "9", "--threads", "1"])),
                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, env={**os.environ, "OPEN_ARCHIVE_PROFILE":"1"}) as worker:
                    for line in worker.stderr:
                        if b"OA_CREATE_WORKER status=encoding" in line:
                            break
                    else:
                        raise AssertionError("worker did not begin encoding")
                    tail.unlink(); tail.symlink_to(outside)
                    try:
                        _, errors = worker.communicate(timeout=90)
                        assert worker.returncode and not bad.exists() and b"OA_PHASE engine-process" not in errors
                        cases.append(["in-flight-file-symlink","native rejected after encoding began, no retry"])
                    finally:
                        tail.unlink()
            target=root/"default.zip"; before=digest(target)
            run([cli,"create",target,src],False)
            assert before==digest(target)
            run([cli,"create",src/"inside.zip",src],False)
            assert not (src/"inside.zip").exists()
            assert not list(root.glob(".qing-zip-create-*"))
            cases.append(["existing-target-and-containment","passed"])
        finally:
            assert kernel.SetFileAttributesW(str(flags), 0x20)
    record = {"cli_sha256":digest(cli), "cases":cases}
    if args.report:
        args.report.write_text(json.dumps(record,indent=2),encoding="utf-8")
    print(json.dumps(record,ensure_ascii=True))


if __name__=="__main__":
    main()
