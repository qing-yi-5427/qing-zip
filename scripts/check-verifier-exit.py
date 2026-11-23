"""Require the exact external ZIP integrity checker to stop when its owning CLI exits."""
import argparse
import hashlib
import json
from pathlib import Path
import random
import subprocess
import tempfile
import time
import zipfile

import psutil


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def trial(cli, root, action):
    archive = root / "ordinary.zip"
    addition = root / "note.txt"
    addition.write_bytes(b"ordinary addition\n")
    chunk = random.Random(370).randbytes(512 * 1024) + b"zip data" * 65536
    content = hashlib.sha256()
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED, compresslevel=1) as bundle:
        with bundle.open("original.bin", "w") as member:
            for _ in range(96):
                member.write(chunk)
                content.update(chunk)
    before = digest(archive)
    command = [str(cli), action, str(archive)]
    if action == "add":
        command.append(str(addition))
    verifier = None
    suspended = False
    with subprocess.Popen(command, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW) as owner:
        parent = psutil.Process(owner.pid)
        try:
            deadline = time.monotonic() + 45
            while owner.poll() is None and verifier is None and time.monotonic() < deadline:
                for child in parent.children():
                    try:
                        args = child.cmdline()
                        if len(args) < 3 or args[1] != "t" or child.name() not in (
                            "7z.exe", "NanaZip.Universal.Console.exe"
                        ):
                            continue
                        target = Path(args[-1])
                        if action == "add":
                            if (target.name != archive.name or target.parent.name != "output"
                                    or not target.parent.parent.name.startswith(".qing-zip-stage-")
                                    or target.parent.parent.parent != root):
                                continue
                        elif target != archive:
                            continue
                        child.suspend()
                        suspended = True
                        verifier = child
                        break
                    except psutil.NoSuchProcess:
                        continue
                if verifier is None:
                    time.sleep(.002)
            if verifier is None or owner.poll() is not None:
                raise RuntimeError(f"Failed to observe an active verifier for {action}")
            owner.kill()
            owner.wait(timeout=15)
            try:
                verifier.wait(timeout=2)
                stopped_with_owner = True
            except psutil.TimeoutExpired:
                stopped_with_owner = False
        finally:
            if owner.poll() is None:
                owner.kill()
                owner.wait(timeout=15)
            if suspended and verifier.is_running():
                try:
                    verifier.resume()
                    verifier.wait(timeout=30)
                except psutil.NoSuchProcess:
                    pass
    if digest(archive) != before:
        raise RuntimeError("Interrupted verification changed the original archive")
    subprocess.run(command, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, timeout=90,
                   creationflags=subprocess.CREATE_NO_WINDOW, check=True)
    with zipfile.ZipFile(archive) as result:
        expected_names = ["original.bin", "note.txt"] if action == "add" else ["original.bin"]
        if sorted(result.namelist()) != sorted(expected_names):
            raise RuntimeError("Retry produced unexpected ZIP entries")
        with result.open("original.bin") as member:
            if hashlib.file_digest(member, "sha256").hexdigest() != content.hexdigest():
                raise RuntimeError("Original member changed")
        if action == "add" and result.read("note.txt") != addition.read_bytes():
            raise RuntimeError("Retry did not retain the added file")
    return {"operation": action, "suspended_verifier_exited_with_owner": stopped_with_owner,
            "original_archive_preserved": True, "retry_and_member_hashes_verified": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path, help="new JSON receipt")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    rows = []
    with args.out.open("x", encoding="utf-8") as receipt:
        for action in ("add", "test"):
            with tempfile.TemporaryDirectory(prefix="qing-zip-verify-exit-") as folder:
                rows.append(trial(cli, Path(folder), action))
        json.dump({"cli_sha256": digest(cli), "cases": rows}, receipt, indent=2)
        receipt.write("\n")
    if any(not row["suspended_verifier_exited_with_owner"] for row in rows):
        raise SystemExit("An archive integrity checker survived its owner")


if __name__ == "__main__":
    main()
