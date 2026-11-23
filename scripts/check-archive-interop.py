"""Cross-check ordinary ZIP and 7z files using a staged qing-zip build."""

import argparse
import hashlib
import json
from pathlib import Path
import random
import statistics
import subprocess
import tempfile
import time
import zipfile


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(stream):
    sha256 = hashlib.sha256()
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        sha256.update(block)
    return sha256.hexdigest()


def file_digest(path):
    with path.open("rb") as stream:
        return digest(stream)


def manifest(root):
    return {
        path.relative_to(root).as_posix(): None if path.is_dir() else file_digest(path)
        for path in root.rglob("*")
    }


def run(executable, work, *args, request=None):
    started = time.perf_counter()
    result = subprocess.run(
        [executable, *args], cwd=work, input=request, capture_output=True,
        stdin=subprocess.DEVNULL if request is None else None, encoding="utf-8", timeout=120,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    elapsed_ms = (time.perf_counter() - started) * 1000
    require(result.returncode == 0, f"{executable.name} {args[0]} ({result.returncode}): "
            f"{result.stderr or result.stdout}")
    return result.stdout, elapsed_ms


def qingzip(cli, work, *args, request=None):
    output, elapsed_ms = run(cli, work, *args, request=request)
    return json.loads(output), elapsed_ms


def create(cli, source, archive, format_name, *, volume=None):
    settings = ["--format", format_name, "--level", "5", "--threads", "4"]
    if format_name == "7z":
        settings.extend(["--method", "lzma2", "--solid"])
    else:
        settings.extend(["--method", "deflate"])
    if volume is not None:
        settings.extend(["--volume", str(volume)])
    return qingzip(cli, source.parent, "create", archive, source, *settings)[1]


def exercise(cli, archive, sizes, destination):
    work = destination.parent
    listing, list_ms = qingzip(cli, work, "list", archive)
    files = [(entry["name"].replace("\\", "/"), entry["size"])
             for entry in listing["entries"] if not entry["is_dir"]]
    require(len(files) == len(sizes) and dict(files) == sizes, f"Wrong listing: {archive.name}")
    tested, test_ms = qingzip(cli, work, "test", archive, "--threads", "4")
    require(tested["valid"] is True, f"Integrity check failed: {archive.name}")
    _, extract_ms = qingzip(cli, work, "extract", archive, destination, "--threads", "4")
    return {"list": list_ms, "test": test_ms, "extract": extract_ms}


def sizes_in(root, expected):
    return {name: (root / name).stat().st_size for name, value in expected.items()
            if value is not None}


def add_file(root, name, data):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def interop(cli, reference, work):
    source_root = work / "inputs"
    source = source_root / "样本 files"
    selected = "样本 files/子目录/重名.txt"
    add_file(source_root, selected, b"second name, unique bytes\n")
    add_file(source, "重名.txt", b"first name\n")
    add_file(source, "空文件.bin", b"")
    add_file(source, "media/seeded.bin", random.Random(245).randbytes(160 * 1024))
    (source / "空目录").mkdir()
    expected = manifest(source_root)
    sizes = sizes_in(source_root, expected)

    qzip = work / "qing.zip"
    create(cli, source, qzip, "zip")
    zip_before = file_digest(qzip)
    with zipfile.ZipFile(qzip) as archive:
        rows = archive.infolist()
        from_qing = {}
        for row in rows:
            name = row.filename.rstrip("/")
            if row.is_dir():
                from_qing[name] = None
            else:
                with archive.open(row) as stream:
                    from_qing[name] = digest(stream)
        require(len(rows) == len(from_qing) and from_qing == expected,
                "Python did not read the expected qing-zip ZIP tree")

    python_zip = work / "python.zip"
    with zipfile.ZipFile(python_zip, "x", zipfile.ZIP_DEFLATED) as archive:
        for name in sorted(expected):
            archive.write(source_root / name, name)
    python_zip_before = file_digest(python_zip)
    python_zip_out = work / "from-python"
    exercise(cli, python_zip, sizes, python_zip_out)
    require(manifest(python_zip_out) == expected, "Wrong tree extracted from Python ZIP")

    selection = work / "selection"
    original = b"existing destination file"
    add_file(selection, selected, original)
    add_file(selection, "untouched.txt", b"unrelated existing file")
    selection_before = manifest(selection)
    qingzip(cli, work, "extract", python_zip, selection,
            "--include", selected, "--overwrite", "skip")
    require(manifest(selection) == selection_before, "Skip changed destination content")
    qingzip(cli, work, "extract", python_zip, selection,
            "--include", selected, "--overwrite", "overwrite")
    require(manifest(selection) == {**selection_before, selected: expected[selected]},
            "Selective overwrite affected unexpected destination paths")

    q7z = work / "qing.7z"
    create(cli, source, q7z, "7z", volume=65536)
    volumes = sorted(work.glob("qing.7z.*"))
    require(len(volumes) > 1 and volumes[0].suffix == ".001", "Missing qing-zip 7z volumes")
    reference_7z = work / "7zip.7z"
    run(reference, source_root, "a", "-t7z", "-m0=LZMA2", "-ms=on", "-mx=5", "-mmt=4",
        "-sccUTF-8", "-bso0", reference_7z, source.name)
    archive_hashes = {path: file_digest(path) for path in [qzip, python_zip, *volumes, reference_7z]}
    reference_out = work / "from-qing"
    run(reference, work, "t", "-sccUTF-8", "-bso0", volumes[0])
    run(reference, work, "x", "-sccUTF-8", "-bso0", f"-o{reference_out}", volumes[0])
    require(manifest(reference_out) == expected, "Wrong tree extracted from qing-zip split 7z")
    qing_out = work / "from-7zip"
    exercise(cli, reference_7z, sizes, qing_out)
    require(manifest(qing_out) == expected, "Wrong tree extracted from official 7-Zip archive")
    require(manifest(source_root) == expected, "Input tree changed")
    require(file_digest(qzip) == zip_before and file_digest(python_zip) == python_zip_before,
            "ZIP source changed")
    require(all(file_digest(path) == original_hash for path, original_hash in archive_hashes.items()),
            "Reading or extraction modified an archive or volume")
    return {"status": "PASS", "source_manifest_sha256": expected,
            "archives_sha256": {path.name: value for path, value in archive_hashes.items()},
            "zip_directions": ["qing-zip to Python", "Python to qing-zip"],
            "sevenz_directions": ["qing-zip split to 7-Zip", "7-Zip solid to qing-zip"],
            "selected_overwrite_policies": ["skip", "overwrite"]}


def populate(source, profile):
    if profile == "tiny-unicode":
        add_file(source, "目录/示例.txt", ("示例\n" * 128).encode())
        add_file(source, "empty.txt", b"")
        (source / "empty-directory").mkdir()
    elif profile == "many-small-files":
        row = b"timestamp, category, index, value\n" * 120
        for i in range(1024):
            add_file(source, f"group-{i // 64:02}/row-{i:04}.csv", row + str(i).encode())
    elif profile == "few-large-random":
        rng = random.Random(8246)
        for i in range(4):
            add_file(source, f"blob-{i}.bin", rng.randbytes(8 * 1024 * 1024))
    else:
        rng = random.Random(9771)
        for i in range(64):
            row = (f"table {i:02d} id status display-name\n" * 1375).encode()
            add_file(source, f"level-{i // 16}/mixed-{i}.dat", row + rng.randbytes(64 * 1024))


def measurements(cli, work, rounds):
    profiles = [("tiny-unicode", "zip"), ("many-small-files", "zip"),
                ("few-large-random", "zip"), ("solid-mixed", "7z")]
    report = {}
    for profile, format_name in profiles:
        case = work / profile
        source_root = case / "inputs"
        source = source_root / "sample"
        populate(source, profile)
        expected = manifest(source_root)
        sizes = sizes_in(source_root, expected)
        samples = []
        for i in range(rounds + 1):
            archive = case / f"round-{i}.{format_name}"
            destination = case / f"output-{i}"
            create_ms = create(cli, source, archive, format_name)
            archive_before = file_digest(archive)
            times = exercise(cli, archive, sizes, destination)
            require(manifest(destination) == expected, f"Output tree changed: {profile}, round {i}")
            require(file_digest(archive) == archive_before, f"Archive changed: {profile}, round {i}")
            require(manifest(source_root) == expected, f"Input tree changed: {profile}, round {i}")
            samples.append({"create": create_ms, **times, "archive_bytes": archive.stat().st_size})
        measured = samples[1:]
        stats = {}
        for key in ("create", "list", "test", "extract", "archive_bytes"):
            values = [sample[key] for sample in measured]
            stats[key] = {"median": statistics.median(values), "min": min(values), "max": max(values)}
        report[profile] = {"format": format_name, "files": len(sizes), "bytes": sum(sizes.values()),
                           "warmup": samples[0], "samples": measured, "stats": stats}
    return report


def synthetic_benchmark(cli, work, rounds):
    settings = {"threads": 4, "dictionary_log": 20, "iterations": 1}
    request = json.dumps({"operation": "benchmark", "benchmark": settings})
    samples = []
    for _ in range(rounds + 1):
        result, wall_ms = qingzip(cli, work, "json", request=request)
        require(result["parameters"] == settings, "Engine benchmark returned different parameters")
        samples.append({"cli_ms": wall_ms, "engine_seconds": result["elapsed_seconds"],
                        "compressing": result["compressing"], "decompressing": result["decompressing"]})
    return {"settings": settings, "warmup": samples[0], "samples": samples[1:]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True, help="Portable staging directory's obz.exe")
    parser.add_argument("--reference", type=Path, help="Official 7z.exe; defaults to the staged runtime")
    parser.add_argument("--rounds", type=int, choices=range(10), default=0, metavar="0-9",
                        help="Benchmark repetitions per workload after one warm-up; 0 tests only")
    parser.add_argument("--receipt", type=Path, help="Optional new JSON output file")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    reference = args.reference or cli.parent / "runtime/7zip/7z.exe"
    reference = reference.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="qing-zip-interop-") as directory:
        work = Path(directory).resolve()
        report = {"cli_sha256": file_digest(cli), "reference_sha256": file_digest(reference),
                  "interop": interop(cli, reference, work)}
        if args.rounds:
            report["benchmark"] = measurements(cli, work, args.rounds)
            report["synthetic_benchmark"] = synthetic_benchmark(cli, work, args.rounds)
    receipt = json.dumps(report, ensure_ascii=False, indent=2)
    if args.receipt:
        with args.receipt.open("x", encoding="utf-8") as output:
            print(receipt, file=output)
    else:
        print(receipt)


if __name__ == "__main__":
    main()
