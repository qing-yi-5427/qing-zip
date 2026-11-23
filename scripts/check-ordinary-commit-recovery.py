"""Exercise two pre-publication checkpoints with ordinary ZIP files and a packaged CLI."""
import argparse
import hashlib
import json
import os
import subprocess
import tempfile
import time
import zipfile
from pathlib import Path

CHILD_TEST = "engine::edit_source::tests::framework_child_pauses_with_original_in_recovery"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def make_zip(path, entry):
    with zipfile.ZipFile(path, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(entry, entry.encode("ascii") * 256)


def inventory(root):
    return {path.relative_to(root).as_posix(): digest(path) for path in root.rglob("*")
            if path.is_file() and (path.suffix == ".zip" or path.name.startswith(".qing-zip-edit-"))}


def trial(cli, harness, root, phase, occupied):
    archive = root / "archive.zip"
    proposal = root / "proposal.zip"
    neighbor = root / "neighbor.zip"
    make_zip(archive, "original.bin")
    make_zip(proposal, "proposed.bin")
    make_zip(neighbor, "neighbor.bin")
    original, proposed, beside = map(digest, [archive, proposal, neighbor])
    ready = root / "ready-to-kill"
    env = os.environ.copy()
    env["QING_TEST_INTERRUPTED_EDIT"] = str(root)
    env.pop("QING_TEST_PUBLIC_SCRATCH", None)
    if phase == "before-publication":
        env["QING_TEST_PUBLIC_SCRATCH"] = "1"
    args = [str(harness), "--exact", CHILD_TEST, "--nocapture"]
    with subprocess.Popen(args, cwd=root, env=env, stdin=subprocess.DEVNULL,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                          creationflags=subprocess.CREATE_NO_WINDOW) as process:
        try:
            until = time.monotonic() + 25
            while not ready.exists() and process.poll() is None and time.monotonic() < until:
                time.sleep(.005)
            if not ready.exists() or process.poll() is not None:
                raise RuntimeError("Commit helper failed to reach its checkpoint")
            if archive.exists():
                raise RuntimeError("Unexpected public name at private commit checkpoint")
        finally:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=15)
    previous = inventory(root)
    private = {k: v for k, v in previous.items() if k not in {"proposal.zip", "neighbor.zip"}}
    if original not in private.values() or proposed not in private.values():
        raise RuntimeError(f"{phase}: expected {original} and {proposed}, observed {private}; files {[(p.relative_to(root).as_posix(), p.stat().st_size) for p in root.rglob(chr(42)) if p.is_file()]}")
    expected = original
    member = "original.bin"
    if occupied:
        make_zip(archive, "foreign.bin")
        expected, member = digest(archive), "foreign.bin"
    listed = subprocess.run([str(cli), "list", str(archive)], cwd=root,
                            stdin=subprocess.DEVNULL, capture_output=True, timeout=45, check=True)
    names = [entry["name"] for entry in json.loads(listed.stdout)["entries"]]
    if names != [member] or digest(archive) != expected:
        raise RuntimeError("Reopen did not preserve the intended original or independent replacement")
    checked = subprocess.run([str(cli), "test", str(archive)], cwd=root,
                             stdin=subprocess.DEVNULL, capture_output=True, timeout=45, check=True)
    if digest(proposal) != proposed or digest(neighbor) != beside:
        raise RuntimeError("Independent ZIPs changed")
    remaining = {k: v for k, v in inventory(root).items()
                 if k not in {"archive.zip", "proposal.zip", "neighbor.zip"}}
    return {"checkpoint": phase, "new_archive_at_public_name": occupied,
            "public_name_absent_before_termination": True, "original_sha256": original,
            "proposal_sha256": proposed, "public_sha256_after_reopen": digest(archive),
            "public_member_after_reopen": names, "packaged_test_exit": checked.returncode,
            "original_still_available": digest(archive) == original or original in remaining.values(),
            "private_zip_files_after_termination": len(private), "private_zip_files_after_reopen": len(remaining),
            "neighbor_and_source_proposal_preserved": True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--harness", type=Path, required=True, help="obz_core test executable from cargo test --lib --no-run")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    cli, harness = args.cli.resolve(strict=True), args.harness.resolve(strict=True)
    rows = []
    for phase in ["original-claimed", "before-publication"]:
        for occupied in [False, True]:
            with tempfile.TemporaryDirectory(prefix="qing-zip-commit-recovery-") as folder:
                result = trial(cli, harness, Path(folder), phase, occupied)
                if not result["original_still_available"]:
                    raise RuntimeError("Original archive no longer available")
                rows.append(result)
    with args.out.open("x", encoding="utf-8") as output:
        json.dump({"cli_sha256": digest(cli), "harness_sha256": digest(harness), "cases": rows}, output, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
