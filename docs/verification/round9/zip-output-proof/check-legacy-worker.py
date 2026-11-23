"""Verify normal publication with a compatible worker that supplies no output proof."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile


def digest(path):
    with path.open("rb") as data:
        return hashlib.file_digest(data, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("candidate", "legacy-worker", "out"):
        parser.add_argument(f"--{option}", type=Path, required=True)
    args = parser.parse_args()
    cli = args.candidate.resolve(strict=True)
    workers = {
        "matching": (cli.parent / "obz-worker.exe").resolve(strict=True),
        "legacy": args.legacy_worker.resolve(strict=True),
    }
    with tempfile.TemporaryDirectory(prefix="qing-zip-legacy-worker-") as folder:
        root = Path(folder)
        env = os.environ | {"OPEN_ARCHIVE_PROFILE": "1"}
        for variable in ("LOCALAPPDATA", "APPDATA", "TEMP", "TMP"):
            directory = root / variable
            directory.mkdir()
            env[variable] = str(directory)
        initial = root / "original.zip"
        original_bytes = b"an original ordinary stored member\n" * 75000
        with zipfile.ZipFile(initial, "x") as archive:
            archive.writestr(zipfile.ZipInfo("original.bin", (2024, 1, 1, 0, 0, 0)), original_bytes)
        input_file = root / "added.txt"
        input_bytes = b"a new ordinary member\n"
        input_file.write_bytes(input_bytes)
        os.utime(input_file, (1700000000, 1700000000))
        before = digest(initial)
        reference_input = digest(input_file)
        results = {}
        for label, worker in workers.items():
            package = root / label
            package.mkdir()
            shutil.copy2(cli, package / "obz.exe")
            shutil.copy2(worker, package / "obz-worker.exe")
            shutil.copytree(cli.parent / "runtime", package / "runtime")
            working = package / "working.zip"
            shutil.copyfile(initial, working)
            run = subprocess.run([package / "obz.exe", "add", working, input_file], cwd=package,
                                 env=env, stdin=subprocess.DEVNULL, capture_output=True,
                                 creationflags=subprocess.CREATE_NO_WINDOW, timeout=60)
            if run.returncode:
                raise RuntimeError(run.stderr.decode(errors="replace"))
            if b"OA_ZIP_APPEND status=encoding" not in run.stderr:
                raise RuntimeError("The native worker did not run")
            used_proof = b"OA_ZIP_CANDIDATE bytes=" in run.stderr
            if used_proof != (label == "matching"):
                raise RuntimeError("Unexpected output evidence capability")
            with zipfile.ZipFile(working) as archive:
                if set(archive.namelist()) != {"original.bin", "added.txt"}:
                    raise RuntimeError("Unexpected published names")
                if archive.read("original.bin") != original_bytes or archive.read("added.txt") != input_bytes:
                    raise RuntimeError("Published payload differs")
            results[label] = {"worker_sha256": digest(worker), "native_worker_used": True,
                              "output_proof_used": used_proof, "zip_sha256": digest(working)}
        if results["matching"]["zip_sha256"] != results["legacy"]["zip_sha256"]:
            raise RuntimeError("The same update produced different ZIP bytes")
        if digest(initial) != before or digest(input_file) != reference_input:
            raise RuntimeError("The source archive or incoming file changed")
        with args.out.open("x", encoding="utf-8") as output:
            json.dump({"cli_sha256": digest(cli), "source_and_input_sha256_preserved": True,
                       "cases": results}, output, indent=2)
            output.write("\n")


if __name__ == "__main__":
    main()
