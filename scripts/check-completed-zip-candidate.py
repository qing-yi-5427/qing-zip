"""Observe an independently readable, complete ZIP candidate before publication."""
import argparse
import hashlib
import json
import subprocess
import tempfile
import time
import zipfile
from pathlib import Path


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def member_hashes(path):
    with zipfile.ZipFile(path) as archive:
        result = {}
        for name in archive.namelist():
            with archive.open(name) as member:
                result[name] = hashlib.file_digest(member, "sha256").hexdigest()
        return result


def trial(exe, root):
    archive = root / "example.zip"
    addition = root / "note.txt"
    addition.write_bytes(b"ordinary ZIP append acceptance\n")
    sentinel = root / "neighbor.txt"
    sentinel.write_bytes(b"independent ordinary file\n")
    chunk = (b"ordinary archive member data\n" * 40000)[:1024 * 1024]
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=1) as package:
        with package.open("original.bin", "w") as member:
            for _ in range(128):
                member.write(chunk)
    old = digest(archive)
    before = member_hashes(archive)
    expected = {**before, addition.name: digest(addition)}
    neighbor = digest(sentinel)
    command = [str(exe), "add", str(archive), str(addition)]
    staged_pattern = ".qing-zip-stage-*/output/example.zip"
    seen = False
    with subprocess.Popen(command, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                          stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW) as edit:
        deadline = time.monotonic() + 45
        while edit.poll() is None and time.monotonic() < deadline:
            for candidate in root.glob(staged_pattern):
                try:
                    with zipfile.ZipFile(candidate) as reader:
                        seen = set(reader.namelist()) == set(expected)
                except (OSError, zipfile.BadZipFile):
                    continue
                if seen:
                    break
            if seen:
                edit.kill()
                break
            time.sleep(.001)
        if edit.poll() is None and not seen:
            edit.kill()
        edit.wait(timeout=20)
    if not seen:
        raise RuntimeError("Did not observe a readable new central directory before exit")
    if edit.returncode == 0 or digest(archive) != old:
        raise RuntimeError("Interruption did not precede publication")
    def inventory():
        files = []
        for path in root.glob(staged_pattern):
            files.append({"bytes": path.stat().st_size, "sha256": digest(path),
                          "member_hashes_match_completed_proposal": member_hashes(path) == expected})
        return files
    after_exit = inventory()
    def run(args):
        subprocess.run([str(exe), *map(str, args)], cwd=root, check=True, timeout=90,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=subprocess.CREATE_NO_WINDOW)
    run(["test", archive])
    if digest(archive) != old:
        raise RuntimeError("Reopening the original changed its bytes")
    after_reopen = inventory()
    run(command[1:])
    if member_hashes(archive) != expected or digest(addition) != expected[addition.name] or digest(sentinel) != neighbor:
        raise RuntimeError("Retry output, source or neighbor changed unexpectedly")
    return {"observed_readable_new_zip_directory": seen, "owner_exit": edit.returncode,
            "old_archive_preserved_on_exit": True, "source_and_neighbor_preserved": True,
            "reopened_original_verified": True, "retry_content_verified": True,
            "retained_after_exit": after_exit, "retained_after_reopen": after_reopen, "retained_after_retry": inventory()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--require-reclaimed", action="store_true",
                        help="fail after recording all trials if a completed candidate remains after retry")
    args = parser.parse_args()
    exe = args.cli.resolve(strict=True)
    retained = False
    with args.out.open("x", encoding="utf-8") as receipt:
        for n in range(1, 4):
            with tempfile.TemporaryDirectory(prefix="qing-zip-finalized-") as folder:
                facts = trial(exe, Path(folder))
            retained |= bool(facts["retained_after_retry"])
            print(json.dumps({"round": n, "cli_sha256": digest(exe), **facts}), file=receipt, flush=True)
    if retained and args.require_reclaimed:
        raise SystemExit("Completed ZIP candidate remains after reopening and retrying")


if __name__ == "__main__":
    main()
