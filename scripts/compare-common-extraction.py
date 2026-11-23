"""Compare the output of real Windows archivers on seven ordinary archive formats."""
import argparse
import bz2
import gzip
import hashlib
import io
import json
import lzma
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import zipfile


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(args, cwd):
    result = subprocess.run(args, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=90)
    if result.returncode:
        raise RuntimeError(f"{Path(args[0]).name}: {result.returncode}\n"
                           + result.stdout.decode(errors="replace")[-1200:])


def cases(root, sevenzip):
    data = "压缩互读\n".encode() * 230
    files = {"资料/item.txt": data, "other/item.txt": b"other bytes", "empty.txt": b""}
    expected = {name: sha(value) for name, value in files.items()}
    sources = root / "sources"
    sources.mkdir()
    for name, value in files.items():
        path = sources / name
        path.parent.mkdir(exist_ok=True)
        path.write_bytes(value)
    with zipfile.ZipFile(root / "sample.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for name, value in files.items():
            archive.writestr(name, value)
    with tarfile.open(root / "sample.tar", "w", format=tarfile.PAX_FORMAT) as archive:
        for name, value in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(value)
            archive.addfile(info, io.BytesIO(value))
    run([sevenzip, "a", "-t7z", root / "sample.7z", *files], sources)
    for ext in ("zip", "7z", "tar"):
        yield root / f"sample.{ext}", expected
    for ext, encoded in (("gz", gzip.compress(data, mtime=0)), ("bz2", bz2.compress(data)),
                         ("xz", lzma.compress(data))):
        archive = root / f"payload.txt.{ext}"
        archive.write_bytes(encoded)
        yield archive, {"payload.txt": sha(data)}
    fixture = Path(__file__).resolve().parents[1] / "obz-rs/tests/fixtures/deep/rar5-m3.rar"
    provenance = json.loads(fixture.with_name("SOURCE.json").read_text(encoding="utf-8"))
    if sha(fixture.read_bytes()) != provenance["sha256"]:
        raise ValueError("Unexpected RAR5 fixture")
    yield Path(shutil.copyfile(fixture, root / fixture.name)), None


def compare(args):
    with tempfile.TemporaryDirectory(prefix="qing-zip-common-formats-") as work:
        root = Path(work)
        for archive, expected in cases(root, args.sevenzip):
            before = sha(archive.read_bytes())
            reference = expected
            tools = ["7-Zip", "qing-zip"] + (["Bandizip"] if args.bandizip else [])
            for tool in tools:
                output = root / (archive.name + "-" + tool)
                output.mkdir()
                if tool == "7-Zip":
                    command = [args.sevenzip, "x", "-y", f"-o{output}", archive]
                elif tool == "Bandizip":
                    command = [args.bandizip, "x", "-y", f"-o:{output}", archive]
                else:
                    command = [args.cli, "extract", archive, output]
                run(command, root)
                actual = {p.relative_to(output).as_posix(): sha(p.read_bytes())
                          for p in output.rglob("*") if p.is_file()}
                if reference is None:  # Published RAR5 fixture; use the independently decoded bytes.
                    if set(actual) != {"bigtext_64k.bin"} or (output / "bigtext_64k.bin").stat().st_size != 65536:
                        raise ValueError("Unexpected RAR5 reference content")
                    reference = actual
                if actual != reference or sha(archive.read_bytes()) != before:
                    raise ValueError(f"Unexpected output or changed source: {tool}, {archive.name}, {actual}")
            print(json.dumps({"archive": archive.name, "engines": tools, "files": reference},
                             ensure_ascii=True))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=lambda p: Path(p).resolve(strict=True))
    parser.add_argument("--sevenzip", required=True, type=lambda p: Path(p).resolve(strict=True))
    parser.add_argument("--bandizip", type=lambda p: Path(p).resolve(strict=True), help="Optional bz console")
    compare(parser.parse_args())
