"""Exercise qing-zip's ZIP64 writer and append path on a generated ordinary file."""
import argparse
import hashlib
import json
import os
import platform
import shutil
import struct
import subprocess
import tempfile
import zipfile
from pathlib import Path


GIB = 1024**3
LARGE = "large.bin"
SMALL = "append.txt"


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def make_large(path):
    # Each MiB starts with its sequence number so reordered blocks change the digest.
    size = 1024**2
    pattern = bytes(range(256)) * (size // 256)
    tail = b"qing-zip: the end is beyond 4 GiB\n"
    checksum = hashlib.sha256()
    with path.open("xb") as stream:
        for index in range(4096):
            block = struct.pack("<Q", index) + pattern[8:]
            stream.write(block)
            checksum.update(block)
        stream.write(tail)
        checksum.update(tail)
    require(path.stat().st_size == 4 * GIB + len(tail), "Wrong source size")
    return checksum.hexdigest()


def verify_archive(archive, expected, *, created_with_store=False):
    entries = []
    with zipfile.ZipFile(archive) as package:
        require(sorted(package.namelist()) == sorted(expected), "Unexpected members")
        for info in package.infolist():
            with package.open(info) as stream:
                # Reading to EOF also makes zipfile validate the stored CRC.
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            size, wanted = expected[info.filename]
            require((info.file_size, digest) == (size, wanted), "Member mismatch")
            if created_with_store and info.filename == LARGE:
                require(info.compress_type == zipfile.ZIP_STORED, "Expected STORE")
            entries.append(dict(name=info.filename, size=info.file_size,
                                method=info.compress_type, header_offset=info.header_offset, crc32=f"{info.CRC:08x}", sha256=digest))
        if created_with_store:
            require(package.start_dir > 4 * GIB, "Unexpected central directory offset")
        return dict(archive_bytes=archive.stat().st_size, central_offset=package.start_dir, entries=entries)


def command(cli, root, env, *args):
    result = subprocess.run([cli, *args], cwd=root, env=env, stdin=subprocess.DEVNULL, capture_output=True, encoding="utf8",
                            errors="replace", timeout=1200, check=False)
    require(result.returncode == 0, f"{args[0]} failed ({result.returncode}): {result.stderr[-500:]}")
    return json.loads(result.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--scratch-parent", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    scratch = args.scratch_parent.resolve(strict=True)
    require(shutil.disk_usage(scratch).free >= 16 * GIB, "Provide at least 16 GiB of free temporary space")
    receipt = dict(platform=platform.platform(), cli_sha256=sha(cli), results={})
    # Exclusive creation protects old evidence and keeps cleanup scoped to our temporary directory.
    with args.receipt.open("x", encoding="utf8") as record:
        try:
            with tempfile.TemporaryDirectory(prefix="qing-zip64-writer-", dir=scratch) as directory:
                private = Path(directory)
                root = private / "work"
                root.mkdir()
                env = os.environ.copy()
                for key in ("TEMP", "TMP", "LOCALAPPDATA", "APPDATA"):
                    folder = private / key
                    folder.mkdir()
                    env[key] = str(folder)
                archive = root / "ordinary.zip"
                large = root / LARGE
                small = root / SMALL
                large_hash = make_large(large)
                expected = {LARGE: (large.stat().st_size, large_hash)}
                small.write_bytes(b"second member after the ZIP64 boundary\n")
                small_hash = sha(small)
                command(cli, root, env, "create", archive.name, LARGE, "--format", "zip", "--store", "--threads", "1")
                receipt["results"]["create"] = verify_archive(archive, expected, created_with_store=True)
                command(cli, root, env, "add", archive.name, SMALL)
                expected[SMALL] = (small.stat().st_size, small_hash)
                receipt["results"]["append"] = verify_archive(archive, expected)
                listing = command(cli, root, env, "list", archive.name)
                actual = [(e["name"], e["size"]) for e in listing["entries"]]
                require(sorted(actual) == sorted((k, v[0]) for k, v in expected.items()), "Wrong listing")
                require(sha(large) == expected[LARGE][1] and sha(small) == small_hash, "Source changed")
                require(set(p.name for p in root.iterdir()) == {archive.name, LARGE, SMALL}, "Temporary residue")
                receipt["source_preserved"] = True
            receipt["status"] = "PASS"
        except Exception as exc:
            receipt.update(status="FAIL", error=str(exc))
            raise
        finally:
            receipt["fixture_removed"] = "directory" in locals() and not Path(directory).exists()
            json.dump(receipt, record, indent=2)
            record.write("\n")


if __name__ == "__main__":
    main()
