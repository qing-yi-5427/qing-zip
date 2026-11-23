"""Check a complete, independently readable ZIP candidate after abruptly stopping its owner."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import zipfile


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def members(path):
    with zipfile.ZipFile(path) as archive:
        result = {}
        for name in archive.namelist():
            with archive.open(name) as source:
                result[name] = hashlib.file_digest(source, "sha256").hexdigest()
        return result


def trial(cli, root, reopen, profile_available, action):
    archive = root / "original.zip"
    addition = root / "added.txt"
    addition.write_bytes(b"ordinary ZIP append acceptance\n")
    sentinel = root / "neighbor.txt"
    sentinel.write_bytes(b"independent ordinary file\n")
    profile = root / "private-profile"
    temporary = root / "process-temp"
    temporary.mkdir()
    if profile_available:
        profile.mkdir()
    else:
        profile.write_bytes(b"ordinary file instead of a profile directory")
    env = os.environ | {
        "LOCALAPPDATA": str(profile),
        "APPDATA": str(profile),
        "TEMP": str(temporary),
        "TMP": str(temporary),
    }
    chunk = (b"ordinary archive member data\n" * 40000)[:1024 * 1024]
    with zipfile.ZipFile(archive, "x", zipfile.ZIP_DEFLATED, compresslevel=1) as package:
        with package.open("original.bin", "w") as member:
            for _ in range(128):
                member.write(chunk)
        if action != "add":
            package.writestr("change.txt", b"small ordinary member\n")
    original = sha256(archive)
    neighbor = sha256(sentinel)
    added = sha256(addition)
    prior_members = members(archive)
    expected = prior_members.copy()
    if action == "add":
        expected[addition.name] = added
        operands = [str(addition)]
    elif action == "delete":
        expected.pop("change.txt")
        operands = ["change.txt"]
    else:
        expected["renamed.txt"] = expected.pop("change.txt")
        operands = ["change.txt", "renamed.txt"]
    command = [str(cli), action, str(archive), *operands]
    process = {
        "cwd": root,
        "env": env,
        "stdin": subprocess.DEVNULL,
        "creationflags": subprocess.CREATE_NO_WINDOW,
    }

    def output_candidates():
        return root.glob(".qing-zip-stage-*/output/original.zip")

    def receipts():
        index = profile / "OpenBandiZip" / "edit-candidates-v1"
        return sum(1 for entry in index.glob("*/*.dpapi") if entry.is_file())

    found = None
    with subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, **process) as edit:
        try:
            deadline = time.monotonic() + 45
            while edit.poll() is None and time.monotonic() < deadline:
                for candidate in output_candidates():
                    try:
                        with zipfile.ZipFile(candidate) as reader:
                            complete = set(reader.namelist()) == set(expected)
                    except (OSError, zipfile.BadZipFile):
                        continue
                    if complete:
                        found = candidate
                        break
                if found:
                    # Close the observer before abrupt exit, so it cannot
                    # affect the updater's publication or later recovery.
                    edit.kill()
                    break
                time.sleep(0.001)
        finally:
            if edit.poll() is None and not found:
                edit.kill()
            edit.wait(timeout=20)
    if not found or not found.is_file() or sha256(archive) != original:
        raise RuntimeError("Did not observe a full private candidate before public replacement")
    if members(found) != expected:
        raise RuntimeError("Interrupted candidate did not contain the completed archive")
    size = found.stat().st_size
    before = receipts()

    def run(args):
        return subprocess.run([str(cli), *map(str, args)], check=True, timeout=90,
                              capture_output=True, **process).stdout

    if reopen != "retry":
        listing = run([reopen, archive])
        if reopen == "list":
            names = {entry["name"] for entry in json.loads(listing)["entries"]}
            if names != set(prior_members):
                raise RuntimeError("Reopen did not list the original archive")
        if sha256(archive) != original:
            raise RuntimeError("Reopen changed the public original")
    reopened = [path.stat().st_size for path in output_candidates()]
    run(command[1:])
    remaining = [path.stat().st_size for path in output_candidates()]
    if members(archive) != expected or sha256(addition) != added or sha256(sentinel) != neighbor:
        raise RuntimeError("Retry changed data outside its verified archive output")
    if profile_available:
        if before != 1 or (reopen != "retry" and reopened) or remaining or receipts():
            raise RuntimeError("Private candidate or receipt was not retired")
    elif before != 0 or remaining != [size]:
        raise RuntimeError("Unavailable profile did not preserve the previous candidate behavior")
    return {
        "action": action,
        "reopen": reopen,
        "profile_available": profile_available,
        "complete_candidate_bytes": size,
        "candidate_receipts_after_exit": before,
        "candidate_bytes_before_retry": reopened,
        "candidate_bytes_after_retry": remaining,
        "candidate_receipts_after_retry": receipts(),
        "owner_exit": edit.returncode,
        "original_sha256": original,
        "retry_sha256": sha256(archive),
        "source_and_neighbor_preserved": True,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--action", choices=("add", "rename", "delete"), default="add")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    if os.name != "nt":
        parser.error("this check requires a packaged Windows CLI on a local NTFS volume")
    with args.out.open("x", encoding="utf-8") as output:
        for reopen, available in [("list", True), ("test", True), ("retry", True), ("retry", False)]:
            with tempfile.TemporaryDirectory(prefix="qing-complete-zip-") as temporary:
                result = trial(cli, Path(temporary), reopen, available, args.action)
            print(json.dumps({"cli_sha256": sha256(cli), **result}), file=output, flush=True)


if __name__ == "__main__":
    main()
