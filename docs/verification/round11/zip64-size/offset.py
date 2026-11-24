"""Test a valid STORE ZIP whose late local header and central directory exceed 4 GiB."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import stat
import struct
import subprocess
import tempfile
import time
import zipfile
import zlib


BOUNDARY = 1 << 32
BIG_SIZE = BOUNDARY - 32
BIG = "large.bin"
SMALL = "after-boundary.txt"
PAYLOAD = b"Ordinary selected member after the 4 GiB boundary.\n"
CHUNK = 8 * 1024 * 1024
PATTERN = bytes(range(256)) * (CHUNK // 256)
STAMP = (2021, 2, 4, 12, 14, 16)
DOS_DATE = ((STAMP[0] - 1980) << 9) | (STAMP[1] << 5) | STAMP[2]
DOS_TIME = (STAMP[3] << 11) | (STAMP[4] << 5) | (STAMP[5] // 2)
CENTRAL = struct.Struct("<4s6H3I5H2I")
LOCAL = struct.Struct("<4s5H3I2H")
ZIP64_END = struct.Struct("<4sQ2H2I4Q")
ZIP64_LOCATOR = struct.Struct("<4sIQI")
CLASSIC_END = struct.Struct("<4s4H2IH")


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def local_header(name, size, crc, version):
    info = zipfile.ZipInfo(name, STAMP)
    info.create_version = info.extract_version = version
    info.compress_type = zipfile.ZIP_STORED
    info.file_size = info.compress_size = size
    info.CRC = crc
    return info.FileHeader(zip64=False)


def central_record(name, size, crc, local_offset):
    encoded = name.encode("ascii")
    if local_offset >= BOUNDARY:
        extra = struct.pack("<HHQ", 1, 8, local_offset)
        offset_field = 0xFFFFFFFF
        version = 45
    else:
        extra = b""
        offset_field = local_offset
        version = 20
    return CENTRAL.pack(
        b"PK\x01\x02", version, version, 0, 0, DOS_TIME, DOS_DATE, crc, size, size,
        len(encoded), len(extra), 0, 0, 0, 0x20, offset_field,
    ) + encoded + extra


def make_archive(path):
    # Both member sizes fit in the ordinary 32-bit fields. Only offsets need ZIP64.
    checksum = 0
    hasher = hashlib.sha256()
    with path.open("x+b") as archive:
        archive.write(local_header(BIG, BIG_SIZE, 0, 20))
        remaining = BIG_SIZE
        while remaining:
            data = PATTERN[:min(CHUNK, remaining)]
            archive.write(data)
            checksum = zlib.crc32(data, checksum)
            hasher.update(data)
            remaining -= len(data)
        late_header = archive.tell()
        assert late_header == BOUNDARY + 7
        archive.seek(0)
        archive.write(local_header(BIG, BIG_SIZE, checksum, 20))
        archive.seek(late_header)

        small_crc = zlib.crc32(PAYLOAD)
        archive.write(local_header(SMALL, len(PAYLOAD), small_crc, 45))
        archive.write(PAYLOAD)
        directory_offset = archive.tell()
        directory = central_record(BIG, BIG_SIZE, checksum, 0)
        directory += central_record(SMALL, len(PAYLOAD), small_crc, late_header)
        archive.write(directory)
        end_offset = archive.tell()
        archive.write(ZIP64_END.pack(
            b"PK\x06\x06", 44, 45, 45, 0, 0, 2, 2, len(directory), directory_offset,
        ))
        archive.write(ZIP64_LOCATOR.pack(b"PK\x06\x07", 0, end_offset, 1))
        archive.write(CLASSIC_END.pack(b"PK\x05\x06", 0, 0, 2, 2, len(directory), 0xFFFFFFFF, 0))
    return {
        "central_directory_offset": directory_offset,
        "central_directory_bytes": len(directory),
        "zip64_end_offset": end_offset,
        "members": {
            BIG: {"size": BIG_SIZE, "crc32": f"{checksum:08x}", "sha256": hasher.hexdigest(),
                  "local_header_offset": 0},
            SMALL: {"size": len(PAYLOAD), "crc32": f"{small_crc:08x}",
                    "sha256": hashlib.sha256(PAYLOAD).hexdigest(), "local_header_offset": late_header},
        },
    }


def stream_info(stream):
    count, checksum = 0, 0
    hasher = hashlib.sha256()
    while block := stream.read(CHUNK):
        count += len(block)
        checksum = zlib.crc32(block, checksum)
        hasher.update(block)
    return {"size": count, "crc32": f"{checksum:08x}", "sha256": hasher.hexdigest()}


def verify_standard_library(path, expected):
    qualified = {}
    with zipfile.ZipFile(path) as archive, path.open("rb") as raw:
        assert archive.namelist() == [BIG, SMALL]
        assert archive.start_dir == expected["central_directory_offset"] > BOUNDARY
        for name, member in expected["members"].items():
            info = archive.getinfo(name)
            assert info.header_offset == member["local_header_offset"]
            assert info.compress_type == zipfile.ZIP_STORED and info.flag_bits == 0
            assert info.file_size == info.compress_size == member["size"] < 0xFFFFFFFF
            assert info.CRC == int(member["crc32"], 16)
            assert info.extra == (b"" if name == BIG else struct.pack("<HHQ", 1, 8, info.header_offset))
            raw.seek(info.header_offset)
            signature, _, flags, method, _, _, crc, compressed, size, names, extra_len = LOCAL.unpack(
                raw.read(LOCAL.size)
            )
            assert (signature, flags, method, crc, compressed, size) == (
                b"PK\x03\x04", 0, 0, info.CRC, info.file_size, info.file_size,
            )
            assert extra_len == 0 and raw.read(names).decode("ascii") == name
            with archive.open(info) as member_data:
                observed = stream_info(member_data)
            assert observed == {key: member[key] for key in ("size", "crc32", "sha256")}
            qualified[name] = observed
        raw.seek(-CLASSIC_END.size, os.SEEK_END)
        classic = CLASSIC_END.unpack(raw.read(CLASSIC_END.size))
        assert classic == (b"PK\x05\x06", 0, 0, 2, 2, expected["central_directory_bytes"], 0xFFFFFFFF, 0)
        raw.seek(-CLASSIC_END.size - ZIP64_LOCATOR.size, os.SEEK_END)
        assert ZIP64_LOCATOR.unpack(raw.read(ZIP64_LOCATOR.size)) == (
            b"PK\x06\x07", 0, expected["zip64_end_offset"], 1,
        )
        raw.seek(expected["zip64_end_offset"])
        assert ZIP64_END.unpack(raw.read(ZIP64_END.size)) == (
            b"PK\x06\x06", 44, 45, 45, 0, 0, 2, 2,
            expected["central_directory_bytes"], expected["central_directory_offset"],
        )
    return qualified


def run(cli, root, environment, args):
    start = time.perf_counter()
    done = subprocess.run(
        [cli, *args], cwd=root, env=environment, stdin=subprocess.DEVNULL,
        capture_output=True, timeout=180,
    )
    elapsed = round(time.perf_counter() - start, 3)
    if done.returncode != 0:
        detail = done.stderr.decode("utf-8", errors="replace").replace(str(root), "<fixture>")
        raise RuntimeError(f"{args[0]} exit {done.returncode}: {detail[-1200:]}")
    return json.loads(done.stdout), elapsed


def exercise(cli, root, report):
    environment = os.environ.copy()
    for key in ("LOCALAPPDATA", "APPDATA", "TEMP", "TMP"):
        directory = root / key
        directory.mkdir()
        environment[key] = str(directory)
    archive = root / "offset-store.zip"
    report["layout"] = make_archive(archive)
    source = archive.stat()
    assert source.st_size > BOUNDARY and not source.st_file_attributes & stat.FILE_ATTRIBUTE_SPARSE_FILE
    report["archive_bytes"] = source.st_size
    report["archive_sparse"] = False
    before = digest(archive)
    report["archive_sha256_before"] = before
    print("Fixture written and archive SHA-256 checked.", flush=True)

    report["zipfile_full_stream_checks"] = verify_standard_library(archive, report["layout"])
    print("zipfile read both complete members and verified their CRC and SHA-256.", flush=True)
    list_result, list_time = run(cli, root, environment, ["list", archive])
    actual = [{"name": entry["name"], "size": entry["size"], "is_dir": entry["is_dir"]}
              for entry in list_result["entries"]]
    report["qing_zip_list"] = {"exit": 0, "seconds": list_time, "entries": actual}
    assert actual == [
        {"name": BIG, "size": BIG_SIZE, "is_dir": False},
        {"name": SMALL, "size": len(PAYLOAD), "is_dir": False},
    ]

    dest = root / "out"
    dest.mkdir()
    extraction, extract_time = run(cli, root, environment, ["extract", archive, dest, "--include", SMALL])
    written = sorted(path.relative_to(dest).as_posix() for path in dest.rglob("*"))
    assert written == [SMALL] and extraction["files"] == 1
    with (dest / SMALL).open("rb") as data:
        extracted = stream_info(data)
    assert extracted == report["zipfile_full_stream_checks"][SMALL]
    report["qing_zip_selected_extract"] = {
        "exit": 0, "seconds": extract_time, "included": SMALL, "outputs": written, "output": extracted,
    }
    after = digest(archive)
    report["archive_sha256_after"] = after
    assert before == after and archive.stat().st_size == source.st_size
    report["archive_unchanged"] = True


def main():
    if not __debug__:
        raise SystemExit("Run without python -O so the verification assertions stay enabled.")
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "work-root", "out"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--source", required=True)
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    work = args.work_root.resolve(strict=True)
    if not work.is_dir() or shutil.disk_usage(work).free < BOUNDARY + 1024**3:
        parser.error("Provide an existing scratch volume with at least 5 GiB free")
    if args.out.exists():
        parser.error("use a new output JSON")
    report = {
        "status": "FAIL", "source": args.source, "platform": platform.platform(),
        "python": platform.python_version(), "work_drive": work.drive, "cli_sha256": digest(cli),
        "worker_sha256": digest(cli.with_name("obz-worker.exe")), "selection_via": "installed x64 CLI",
    }
    root = None
    try:
        with tempfile.TemporaryDirectory(prefix="zip64-offset-", dir=work) as directory:
            root = Path(directory)
            exercise(cli, root, report)
        report["status"] = "PASS"
    except Exception as error:
        detail = str(error)
        for path, label in ((root, "<fixture>"), (work, "<work-root>")):
            if path is not None:
                literal = str(path)
                detail = detail.replace(literal, label).replace(repr(literal)[1:-1], label)
        report["error"] = f"{type(error).__name__}: {detail}"
        raise
    finally:
        report["owned_fixture_removed"] = root is not None and not root.exists()
        with args.out.open("x", encoding="utf-8") as receipt:
            json.dump(report, receipt, ensure_ascii=False, indent=2)
            receipt.write("\n")


if __name__ == "__main__":
    main()
