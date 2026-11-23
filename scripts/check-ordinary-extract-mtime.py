"""Verify ordinary ZIP and TAR extraction times with installed Windows CLI tools."""

import argparse
import bz2
from datetime import datetime, timedelta, timezone
import gzip
import hashlib
import io
import json
import lzma
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import tempfile
import zipfile


STAMP = datetime(2021, 2, 4, 12, 14)
SELECTED = "folder/notes.txt"
MEMBERS = {
    "folder": None,
    SELECTED: b"Selected member\n",
    "folder/other.txt": b"Unselected member\n",
    "empty": None,
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def make_archives(root):
    zip_path = root / "sample.zip"
    tar_bytes = io.BytesIO()
    with (
        zipfile.ZipFile(zip_path, "x") as zip_archive,
        tarfile.open(fileobj=tar_bytes, mode="w", format=tarfile.USTAR_FORMAT) as tar_archive,
    ):
        for index, (name, content) in enumerate(MEMBERS.items()):
            modified = STAMP + timedelta(seconds=2 * (index + 1))
            zip_info = zipfile.ZipInfo(name if content is not None else name + "/",
                                       modified.timetuple()[:6])
            tar_info = tarfile.TarInfo(name)
            tar_info.mtime = int(modified.replace(tzinfo=timezone.utc).timestamp())
            if content is None:
                zip_info.external_attr = (0o40755 << 16) | 0x10
                tar_info.type = tarfile.DIRTYPE
            else:
                tar_info.size = len(content)
            zip_archive.writestr(zip_info, content or b"")
            tar_archive.addfile(tar_info, None if content is None else io.BytesIO(content))

    data = tar_bytes.getvalue()
    yield zip_path
    for extension, encoded in (
        ("tar", data), ("tar.gz", gzip.compress(data, mtime=0)),
        ("tar.bz2", bz2.compress(data)), ("tar.xz", lzma.compress(data)),
    ):
        archive = root / f"sample.{extension}"
        archive.write_bytes(encoded)
        yield archive


def expected_output(zip_time, selected):
    expected = {}
    for index, (name, content) in enumerate(MEMBERS.items()):
        if selected and name not in ("folder", SELECTED):
            continue
        record = {"kind": "directory"} if content is None else {
            "kind": "file", "sha256": hashlib.sha256(content).hexdigest(),
        }
        # The selected file requires an implicit parent, which has no selected timestamp.
        if not selected or name == SELECTED:
            modified = STAMP + timedelta(seconds=2 * (index + 1))
            record["mtime"] = modified.isoformat() if zip_time else int(
                modified.replace(tzinfo=timezone.utc).timestamp()
            )
        expected[name] = record
    return expected


def extract(root, env, executable, archive, selected, reference=False):
    tool = "7-Zip" if reference else "qing-zip"
    mode = "selected" if selected else "all"
    out = root / f"{tool}-{archive.name}-{mode}"
    out.mkdir()
    if reference:
        command = [executable, "x", "-y", f"-o{out}", archive]
        if selected:
            command.append(SELECTED)
    else:
        command = [executable, "extract", archive, out]
        if selected:
            command.extend(("--include", SELECTED))
    before = digest(archive)
    result = subprocess.run(command, cwd=root, env=env, stdin=subprocess.DEVNULL,
                            capture_output=True, timeout=20, check=False)
    context = f"{tool} {archive.name} {mode}"
    if result.returncode:
        raise RuntimeError(f"{context}: exit {result.returncode}\n"
                           + result.stderr.decode(errors="replace")[-800:])
    after = digest(archive)
    if after != before:
        raise RuntimeError(f"{context}: source archive changed")

    zip_time = archive.suffix == ".zip"
    expected = expected_output(zip_time, selected)
    actual = {}
    for path in out.rglob("*"):
        name = path.relative_to(out).as_posix()
        record = {"kind": "directory"} if path.is_dir() else {
            "kind": "file", "sha256": digest(path),
        }
        if "mtime" in expected.get(name, {}):
            seconds = path.stat().st_mtime_ns // 1_000_000_000
            record["mtime"] = datetime.fromtimestamp(seconds).isoformat() if zip_time else seconds
        actual[name] = record
    if actual != expected:
        raise RuntimeError(f"{context}: expected {expected!r}, observed {actual!r}")
    return {"tool": tool, "archive": archive.name, "mode": mode, "exit_code": result.returncode,
            "source_sha256_before": before, "source_sha256_after": after, "output": actual}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True, help="Path to installed obz.exe")
    parser.add_argument("--sevenzip", type=Path, required=True, help="Path to 7z.exe for the ZIP control")
    parser.add_argument("--source", required=True, help="Source revision of installed obz.exe")
    parser.add_argument("--out", type=Path, required=True, help="Path to a new JSON receipt")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    sevenzip = args.sevenzip.resolve(strict=True)
    rows = []
    with tempfile.TemporaryDirectory(prefix="qing-zip-ordinary-mtime-") as temp:
        root = Path(temp)
        isolated = {key: root / key.lower() for key in ("LOCALAPPDATA", "APPDATA", "TEMP", "TMP")}
        for directory in isolated.values():
            directory.mkdir()
        env = os.environ | {key: str(directory) for key, directory in isolated.items()}
        for archive in make_archives(root):
            rows.extend(extract(root, env, cli, archive, selected) for selected in (False, True))
            if archive.suffix == ".zip":
                rows.extend(extract(root, env, sevenzip, archive, selected, reference=True)
                            for selected in (False, True))
    with args.out.open("x", encoding="utf-8") as receipt:
        summary = {"source": args.source, "platform": platform.platform(), "python": platform.python_version(),
                   "cli_sha256": digest(cli), "sevenzip_sha256": digest(sevenzip), "results": rows}
        json.dump(summary, receipt, ensure_ascii=False, indent=2)
        receipt.write("\n")


if __name__ == "__main__":
    main()
