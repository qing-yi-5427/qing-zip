"""Exercise common archive operations using only the packaged Windows binaries."""
import argparse
import json
import subprocess
import tempfile
import zipfile
from pathlib import Path


def run_archive(cli, work, *args, returns_error=False):
    result = subprocess.run(
        [cli, *args], cwd=work, stdin=subprocess.DEVNULL, capture_output=True,
        encoding="utf-8", errors="replace", timeout=60,
    )
    if returns_error:
        if result.returncode == 0:
            raise RuntimeError(f"Expected {args[0]} to refuse an existing file")
        return
    if result.returncode:
        raise RuntimeError(f"{args[0]} failed ({result.returncode}): {result.stderr}")
    return json.loads(result.stdout)


def smoke(portable):
    with tempfile.TemporaryDirectory(prefix="qing-zip-package-test-") as directory:
        work = Path(directory).resolve()
        app = work / "app"
        with zipfile.ZipFile(portable) as package:
            entries = {Path(name) for name in package.namelist()}
            required = {
                Path("qing-zip.exe"), Path("obz.exe"), Path("obz-worker.exe"),
                Path("docs/deep-extraction.md"), Path("licenses/THIRD-PARTY-NOTICES.txt"),
                Path("runtime/python/python.exe"), Path("runtime/python/provenance.json"),
                Path("runtime/7zip/7z.dll"), Path("runtime/nanazip/NanaZip.Core.dll"),
            }
            if not required <= entries:
                raise RuntimeError(f"Incomplete portable package: {required - entries}")
            package.extractall(app)
        cli = app / "obz.exe"
        sample = work / "交付 样本.txt"
        contents = b"qing-zip release round trip\n" * 64
        sample.write_bytes(contents)
        for suffix in ("zip", "7z"):
            archive = work / f"交付.{suffix}"
            target = work / f"output-{suffix}"
            run_archive(cli, work, "create", archive, sample, "--format", suffix)
            source_bytes = archive.read_bytes()
            listing = run_archive(cli, work, "list", archive)
            if [entry["name"] for entry in listing["entries"]] != [sample.name]:
                raise RuntimeError(f"Unexpected {suffix} archive directory")
            if run_archive(cli, work, "test", archive)["valid"] is not True:
                raise RuntimeError(f"{suffix} integrity test did not pass")
            run_archive(cli, work, "extract", archive, target)
            output = target / sample.name
            if output.read_bytes() != contents:
                raise RuntimeError(f"{suffix} bytes changed during extraction")

            output.write_bytes(b"existing destination")
            run_archive(cli, work, "extract", archive, target, "--overwrite", "error", returns_error=True)
            run_archive(cli, work, "create", archive, sample, "--format", suffix, returns_error=True)
            if output.read_bytes() != b"existing destination" or archive.read_bytes() != source_bytes:
                raise RuntimeError(f"{suffix} conflict changed existing data")
            print(f"{suffix.upper()}: create, list, test, extract and conflict handling passed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("portable", type=Path)
    smoke(parser.parse_args().portable.resolve(strict=True))
