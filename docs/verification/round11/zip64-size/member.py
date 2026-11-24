"""Exercise the installed Windows x64 CLI with ordinary ZIP64 members around 4 GiB."""

import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import struct
import subprocess
import tempfile
import threading
import time
import zipfile
import zlib


GIB = 1 << 30
LIMIT = 4 * GIB
CHUNK = 8 << 20
MEMBER = "ordinary-member.bin"
PROFILE = (("below", LIMIT - 65_537), ("at", LIMIT), ("above", LIMIT + 65_537))
LOCAL = struct.Struct("<4s5H3I2H")
CENTRAL = struct.Struct("<4s6H3I5H2I")
EOCD = struct.Struct("<4s4H2IH")


def require(condition, description):
    if not condition:
        raise RuntimeError(description)


def json_file(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def checksum(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def digest(stream):
    sha = hashlib.sha256()
    crc, size = 0, 0
    for block in iter(lambda: stream.read(CHUNK), b""):
        sha.update(block)
        crc = zlib.crc32(block, crc)
        size += len(block)
    return {"bytes": size, "sha256": sha.hexdigest(), "crc32": f"{crc:08x}"}


def fixture(path, size):
    seeds = random.Random(190596).randbytes(128 << 10)
    pattern = hashlib.sha256(b"ordinary ZIP64 content").digest()
    body = seeds + pattern * ((CHUNK - len(seeds)) // len(pattern))
    records = {
        0: b"ZIP64 boundary QA HEAD\n",
        (2 * GIB) - 8: b"ZIP64 signed 32-bit offset boundary\n",
        LIMIT - (128 << 10): b"ZIP64 pre-boundary marker\n",
        size - 64: b"ZIP64 boundary QA TAIL\n",
    }
    if size > LIMIT:
        records[LIMIT - 16] = b"ZIP64 marker straddling the 4 GiB offset\n"
        records[LIMIT + 32_768] = b"ZIP64 member content after 4 GiB\n"
    sha, crc = hashlib.sha256(), 0
    with path.open("xb") as file:
        for offset in range(0, size, CHUNK):
            length = min(CHUNK, size - offset)
            block = body[:length]
            patches = [(pos, mark) for pos, mark in records.items()
                       if pos < offset + length and pos + len(mark) > offset]
            if patches:
                block = bytearray(block)
                for pos, mark in patches:
                    a, b = max(pos, offset), min(pos + len(mark), offset + length)
                    block[a - offset:b - offset] = mark[a - pos:b - pos]
            file.write(block)
            sha.update(block)
            crc = zlib.crc32(block, crc)
    return {"bytes": size, "sha256": sha.hexdigest(), "crc32": f"{crc:08x}"}, records


def create_zip(source, archive):
    info = zipfile.ZipInfo(MEMBER, date_time=(2024, 1, 1, 0, 0, 0))
    info.compress_type = zipfile.ZIP_DEFLATED
    info.compress_level = 1
    info.file_size = source.stat().st_size
    with zipfile.ZipFile(archive, "x", allowZip64=True) as zf:
        with source.open("rb") as content, zf.open(info, "w", force_zip64=True) as member:
            shutil.copyfileobj(content, member, length=CHUNK)


def zip64_field(extra):
    at, fields = 0, []
    while at < len(extra):
        tag, length = struct.unpack_from("<HH", extra, at)
        at += 4
        require(at + length <= len(extra), "ZIP extra field overrun")
        if tag == 1:
            fields.append(extra[at:at + length])
        at += length
    require(len(fields) == 1, "Expected a single ZIP64 extra field")
    return fields[0]


def inspect_zip64(path, expected):
    with path.open("rb") as stream:
        stream.seek(-EOCD.size, os.SEEK_END)
        end = EOCD.unpack(stream.read(EOCD.size))
        require(end[:5] == (b"PK\x05\x06", 0, 0, 1, 1) and end[7] == 0,
                "Expected a small single-member central directory and ordinary ZIP end record")
        stream.seek(end[6])
        cd = CENTRAL.unpack(stream.read(CENTRAL.size))
        require(cd[0] == b"PK\x01\x02" and cd[2] >= 45 and cd[3] == 0 and cd[4] == 8,
                "Unexpected central directory attributes")
        require(cd[8:10] == (0xFFFFFFFF, 0xFFFFFFFF) and cd[16] == 0,
                "Expected both central size sentinels and a normal local-header offset")
        require(stream.read(cd[10]) == MEMBER.encode(), "Wrong central member name")
        central_original, central_packed = struct.unpack("<QQ", zip64_field(stream.read(cd[11])))
        require(central_original == expected["bytes"] and 0 < central_packed < 0xFFFFFFFF,
                "Wrong central ZIP64 sizes")
        stream.seek(cd[16])
        lh = LOCAL.unpack(stream.read(LOCAL.size))
        require(lh[:4] == (b"PK\x03\x04", cd[2], 0, 8) and lh[7:9] == (0xFFFFFFFF, 0xFFFFFFFF),
                "Wrong local header/version or size sentinels")
        require(stream.read(lh[9]) == MEMBER.encode(), "Wrong local member name")
        local_original, local_packed = struct.unpack("<QQ", zip64_field(stream.read(lh[10])))
        require((local_original, local_packed) == (central_original, central_packed), "Local ZIP64 sizes differ")
        require(f"{lh[6]:08x}" == f"{cd[7]:08x}" == expected["crc32"], "Header CRC mismatch")
        require(stream.tell() + local_packed == end[6], "Wrong compressed data extent")
    return {
        "format": "single-volume ZIP, one Deflate member",
        "version_needed": cd[2], "zip64_extra_id": "0x0001", "central_original_size_32": cd[9],
        "central_original_size_64": central_original, "central_compressed_size_32": cd[8],
        "central_compressed_size_64": central_packed,
        "local_original_size_32": lh[8], "local_compressed_size_32": lh[7],
        "local_original_size_64": local_original, "local_compressed_size_64": local_packed,
        "crc32": expected["crc32"], "ordinary_archive_end_record": True,
    }


def binary_hashes(cli, manifest, build):
    binaries = (cli.name, "obz-worker.exe")
    if manifest is None:
        return {name: checksum(cli.parent / name) for name in binaries}
    require(build["inner_portable_sha256"] == manifest["portable_sha256"],
            "Package manifest and build receipt refer to different packages")
    require(len(manifest["files"]) == build["verified_payload_files"] and set(binaries) <= manifest["files"].keys(),
            "The supplied package inventory is incomplete")
    for name, value in manifest["files"].items():
        file = (cli.parent / name).resolve(strict=True)
        require(file.is_relative_to(cli.parent) and checksum(file) == value, f"Package file differs: {name}")
    return {name: manifest["files"][name] for name in (*binaries, "qing-zip.exe") if name in manifest["files"]}


def run_action(cli, action, arguments, work, env):
    started = time.perf_counter()
    result = subprocess.run([cli, action, *arguments], cwd=work, env=env, stdin=subprocess.DEVNULL,
                            capture_output=True, timeout=480, creationflags=subprocess.CREATE_NO_WINDOW)
    seconds = round(time.perf_counter() - started, 3)
    if result.returncode:
        message = (result.stderr or result.stdout).decode("utf-8", "replace")
        raise RuntimeError(f"{action} exit {result.returncode}: {message}")
    return json.loads(result.stdout), {"exit_code": result.returncode, "seconds": seconds}


def isolated_environment(work):
    env = os.environ.copy()
    for name in ("OPEN_ARCHIVE_PROFILE", "RUST_BACKTRACE"):
        env.pop(name, None)
    for name in ("LOCALAPPDATA", "APPDATA", "TEMP", "TMP"):
        area = work / name
        area.mkdir()
        env[name] = str(area)
    return env


def trial(cli, work, row):
    size = row["requested_member_bytes"]
    env = isolated_environment(work)
    original = work / "fixture.bin"
    archive = work / "input.zip"
    destination = work / "extracted"
    expected, markers = fixture(original, size)
    row["synthetic_input"] = expected
    create_zip(original, archive)
    row["zip_headers"] = inspect_zip64(archive, expected)
    row["archive_before"] = {"bytes": archive.stat().st_size, "sha256": checksum(archive)}
    with zipfile.ZipFile(archive, "r") as zf:
        with zf.open(MEMBER) as source:
            row["independent_archive_member"] = digest(source)
    require(row["independent_archive_member"] == expected, "The independent ZIP reader disagrees with the fixture")
    listed, listing = run_action(cli, "list", [archive], work, env)
    listings = listed["entries"]
    require(len(listings) == 1 and listings[0]["name"] == MEMBER and listings[0]["size"] == size,
            "Installed list did not return the complete member size")
    row["installed_list"] = {**listing, "name": listings[0]["name"], "member_bytes": listings[0]["size"]}
    tested, test = run_action(cli, "test", [archive, "--threads", "4"], work, env)
    row["installed_test"] = {**test, "valid": tested["valid"]}
    require(tested["valid"] is True, "Installed test did not validate the archive")
    extraction, extracted = run_action(cli, "extract", [archive, destination, "--threads", "4"], work, env)
    row["installed_extract"] = {**extracted, "files": extraction["files"]}
    require(extraction["files"] == 1 and list(destination.iterdir()) == [destination / MEMBER],
            "Expected the single named extracted member")
    target = destination / MEMBER
    require(target.is_file() and target.stat().st_size == size, "Extracted file has the wrong length")
    with target.open("rb") as stream:
        row["extracted_member"] = digest(stream)
        row["markers"] = []
        for offset, expected_bytes in sorted(markers.items()):
            stream.seek(offset)
            require(stream.read(len(expected_bytes)) == expected_bytes, f"Marker differs at offset {offset}")
            row["markers"].append({"offset": offset, "bytes": len(expected_bytes),
                                   "sha256": hashlib.sha256(expected_bytes).hexdigest()})
    with original.open("rb") as stream:
        row["input_after"] = digest(stream)
    row["archive_after"] = {"bytes": archive.stat().st_size, "sha256": checksum(archive)}
    require(row["extracted_member"] == row["input_after"] == expected, "Source or extracted bytes changed")
    require(row["archive_after"] == row["archive_before"], "The source archive changed")


def run_case(cli, root, case, size, row):
    row.update(case=case, requested_member_bytes=size)
    free_before = shutil.disk_usage(root).free
    require(free_before >= 24 * GIB, "Each case requires 24 GiB free on its temporary volume")
    with tempfile.TemporaryDirectory(prefix=f"{case}-", dir=root) as temp:
        work = Path(temp).resolve(strict=True)
        require(work.parent == root, "Temporary case escaped the owned root")
        observations = [free_before]
        stop = threading.Event()

        def observe_disk():
            while not stop.wait(0.25):
                observations.append(shutil.disk_usage(root).free)

        monitor = threading.Thread(target=observe_disk)
        monitor.start()
        try:
            trial(cli, work, row)
        finally:
            stop.set()
            monitor.join()
            observations.append(shutil.disk_usage(root).free)
            row["volume_observation_bytes"] = {"free_before": free_before, "lowest_sampled_free": min(observations),
                                               "free_before_cleanup": observations[-1], "samples": len(observations)}
    row["case_directory_removed"] = not work.exists()
    require(row["case_directory_removed"], "Case fixture was not cleaned")
    row["volume_observation_bytes"]["free_after_case_cleanup"] = shutil.disk_usage(root).free


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "temporary-parent", "out"):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--manifest", type=Path, help="Optional shipped-file SHA-256 inventory")
    p.add_argument("--build-receipt", type=Path, help="Build receipt matching the shipped-file inventory")
    args = p.parse_args()
    if bool(args.manifest) != bool(args.build_receipt):
        p.error("--manifest and --build-receipt are supplied together")
    require(os.name == "nt" and platform.machine().upper() in {"AMD64", "X86_64"}, "Requires Windows x64")
    cli = args.cli.resolve(strict=True)
    require(cli.is_file() and cli.name.lower() == "obz.exe" and (cli.parent / "obz-worker.exe").is_file(),
            "Point --cli at obz.exe alongside its matching obz-worker.exe and runtime")
    parent = args.temporary_parent.resolve(strict=True)
    require(parent.is_dir() and len(parent.drive) == 2 and parent.drive[1] == ":",
            "Use an existing temporary directory on a local Windows volume")
    require(shutil.disk_usage(parent).free >= 24 * GIB, "The temporary volume requires 24 GiB free")
    require(not args.out.exists() and args.out.parent.is_dir(), "Expected a new receipt path in an existing directory")
    manifest, build = (json_file(args.manifest), json_file(args.build_receipt)) if args.manifest else (None, {})
    binaries = binary_hashes(cli, manifest, build)
    report = {
        "started": datetime.now().astimezone().isoformat(timespec="seconds"),
        "platform": platform.platform(), "python": platform.python_version(),
        "zlib": zlib.ZLIB_RUNTIME_VERSION, "source": build.get("main_commit"), "tree": build.get("main_tree"),
        "build_run": build.get("windows_regression_run"),
        "installed_payloads_verified": len(manifest["files"]) if manifest is not None else None,
        "installed_binaries_sha256": binaries, "free_before_all_bytes": shutil.disk_usage(parent).free,
        "minimum_free_before_case_bytes": 24 * GIB, "cases": [],
    }
    failure = False
    with tempfile.TemporaryDirectory(prefix="qing-zip-zip64-member-", dir=parent) as owned:
        root = Path(owned).resolve(strict=True)
        require(root.parent == parent, "Unexpected temporary root")
        for case, size in PROFILE:
            row = {"status": "pending"}
            report["cases"].append(row)
            print(f"Running {case}: {size:,} bytes", flush=True)
            try:
                run_case(cli, root, case, size, row)
            except Exception as error:
                message = str(error).replace(str(root), "$WORK").replace(str(cli.parent), "$INSTALLED")
                row.update(status="failed", error=message)
                print(f"{case} FAIL: {message}", flush=True)
                failure = True
            else:
                row["status"] = "passed"
                print(f"{case} PASS: extract {row['installed_extract']['seconds']:.3f} s", flush=True)
    report["owned_temporary_root_removed"] = not root.exists()
    report["free_after_all_bytes"] = shutil.disk_usage(parent).free
    report["installed_binaries_unchanged"] = binary_hashes(cli, manifest, build) == binaries
    report["finished"] = datetime.now().astimezone().isoformat(timespec="seconds")
    failure = failure or not report["owned_temporary_root_removed"] or not report["installed_binaries_unchanged"]
    report["status"] = "failed" if failure else "passed"
    with args.out.open("x", encoding="utf-8", newline="\n") as output:
        output.write(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    return 1 if failure else 0


if __name__ == "__main__":
    raise SystemExit(main())
