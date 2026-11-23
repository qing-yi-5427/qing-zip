"""Compare the output of real Windows archivers, including classic split RAR volumes."""
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


def cases(root, sevenzip, include_rar15_split):
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
        yield (root / f"sample.{ext}",), expected
    for ext, encoded in (("gz", gzip.compress(data, mtime=0)), ("bz2", bz2.compress(data)),
                         ("xz", lzma.compress(data))):
        archive = root / f"payload.txt.{ext}"
        archive.write_bytes(encoded)
        yield (archive,), {"payload.txt": sha(data)}
    fixture = Path(__file__).resolve().parents[1] / "obz-rs/tests/fixtures/deep/rar5-m3.rar"
    provenance = json.loads(fixture.with_name("SOURCE.json").read_text(encoding="utf-8"))
    if sha(fixture.read_bytes()) != provenance["sha256"]:
        raise ValueError("Unexpected RAR5 fixture")
    bigtext = {"bigtext_64k.bin": "ed70a3812333daa70dfdbc167cb009049fd6aed3aeef0902ad5326d245892209"}
    yield (Path(shutil.copyfile(fixture, root / fixture.name)),), bigtext
    classic = fixture.parent / "rar15"
    hashes = json.loads((classic / "SOURCE.json").read_text(encoding="utf-8"))["sha256"]
    payload = {"payload.txt": sha(b"rar15 oracle payload\n")}
    samples = [
        (("compressed.rar",), payload),
        (("multivol_oldnaming_rar300.rar", "multivol_oldnaming_rar300.r00"), bigtext),
    ]
    if include_rar15_split:
        samples.append((("split-store.rar", "split-store.r00", "split-store.r01"), payload))
    for names, expected_files in samples:
        for name in names:
            member = classic / name
            if sha(member.read_bytes()) != hashes[name]:
                raise ValueError(f"Unexpected classic RAR fixture: {name}")
            shutil.copyfile(member, root / name)
        yield tuple(root / name for name in names), expected_files


def compare(args):
    failures = 0
    with tempfile.TemporaryDirectory(prefix="qing-zip-common-formats-") as work:
        root = Path(work)
        for volumes, expected in cases(root, args.sevenzip, args.include_rar15_split):
            archive = volumes[0]
            before = {part.name: sha(part.read_bytes()) for part in volumes}
            tools = ["7-Zip", "qing-zip"] + (["Bandizip"] if args.bandizip else [])
            if args.unrar and archive.suffix == ".rar":
                tools.append("RARLAB UnRAR")
            outcomes = {}
            for tool in tools:
                output = root / (archive.name + "-" + tool)
                output.mkdir()
                if tool == "7-Zip":
                    command = [args.sevenzip, "x", "-y", f"-o{output}", archive]
                elif tool == "Bandizip":
                    command = [args.bandizip, "x", "-y", f"-o:{output}", archive]
                elif tool == "RARLAB UnRAR":
                    command = [args.unrar, "x", "-y", archive, str(output) + "\\"]
                else:
                    command = [args.cli, "extract", archive, output]
                result = subprocess.run(command, cwd=root, stdin=subprocess.DEVNULL, capture_output=True, timeout=90)
                actual = {p.relative_to(output).as_posix(): sha(p.read_bytes())
                          for p in output.rglob("*") if p.is_file()}
                if any(sha(part.read_bytes()) != before[part.name] for part in volumes):
                    raise ValueError(f"Source changed: {tool}, {archive.name}")
                passed = result.returncode == 0 and actual == expected
                failures += int(not passed)
                outcomes[tool] = {"status": "PASS" if passed else "FAIL", "exit": result.returncode}
                if not passed:
                    outcomes[tool]["written"] = actual
            print(json.dumps({"archive": archive.name, "engines": tools, "files": expected, "sources": before, "outcomes": outcomes},
                             ensure_ascii=True))
    return int(failures > 0)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=lambda p: Path(p).resolve(strict=True))
    parser.add_argument("--sevenzip", required=True, type=lambda p: Path(p).resolve(strict=True))
    parser.add_argument("--bandizip", type=lambda p: Path(p).resolve(strict=True), help="Optional bz console")
    parser.add_argument("--unrar", type=lambda p: Path(p).resolve(strict=True), help="Optional RARLAB Windows UnRAR")
    parser.add_argument("--include-rar15-split", action="store_true", help="Also test the legacy three-volume sample with a known decoder discrepancy")
    raise SystemExit(compare(parser.parse_args()))
