"""Verify the reviewed x64 runtime and enumerate the files allowed in a release."""
import hashlib
import json


def read_manifest(path):
    return json.loads(path.read_text(encoding="utf-8"))


def checked_files(directory, expected):
    files = []
    for name, digest in expected.items():
        if name in ("", ".", "..") or any(separator in name for separator in "/\\:"):
            raise ValueError(f"Invalid runtime filename: {name}")
        path = directory / name
        if path.resolve() != path or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f"Runtime integrity mismatch: {directory.name}/{name}")
        files.append(path)
    return files


def x64_runtime_files(root):
    runtime = root / "runtime"
    seven = runtime / "7zip"
    seven_manifest = read_manifest(seven / "provenance.json")
    if (not seven_manifest["url"].startswith("https://github.com/ip7z/7zip/releases/download/")
            or len(seven_manifest["sha256"]) != 64
            or set(seven_manifest["files"]) != {"7z.exe", "7z.dll", "7z.sfx", "License.txt"}):
        raise ValueError("Invalid 7-Zip runtime manifest")
    files = checked_files(seven, seven_manifest["files"])
    files += [seven / "README.txt", seven / "provenance.json"]

    nana = runtime / "nanazip"
    nana_manifest = read_manifest(nana / "provenance.json")
    if (not nana_manifest["package_url"].startswith("https://github.com/M2Team/NanaZip/releases/download/")
            or len(nana_manifest["package_sha256"]) != 64
            or set(nana_manifest["files"]) != {"NanaZip.Universal.Console.exe", "NanaZip.Core.dll",
                                              "NanaZip.Codecs.dll", "NanaZip.ZS.Codecs.dll",
                                              "K7Base.dll", "K7User.dll"}):
        raise ValueError("Invalid NanaZip runtime manifest")
    files += checked_files(nana, nana_manifest["files"])
    files += [nana / "README.txt", nana / "provenance.json"]

    interfaces = root / "obz-rs/native/nana"
    headers = read_manifest(interfaces / "interfaces.json")
    if (headers["version"] != nana_manifest["version"]
            or headers["source_archive_sha256"] != nana_manifest["source_archive_sha256"]):
        raise ValueError("NanaZip interface source and runtime releases differ")
    actual_headers = {
        path.relative_to(interfaces).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for directory in ("vendor", "licenses")
        for path in (interfaces / directory).rglob("*") if path.is_file()
    }
    if actual_headers != headers["files"]:
        raise ValueError("NanaZip interface header or license inventory differs from upstream")

    python = runtime / "python"
    python_manifest = read_manifest(python / "provenance.json")
    version = python_manifest["version"]
    if (python_manifest["url"] != f"https://www.python.org/ftp/python/{version}/python-{version}-embed-amd64.zip"
            or len(python_manifest["sha256"]) != 64
            or not {"python.exe", "LICENSE.txt"} <= python_manifest["files"].keys()):
        raise ValueError("Invalid embedded Python runtime manifest")
    files += checked_files(python, python_manifest["files"])
    files += [python / name for name in (
        "provenance.json", "acefile.py", "ace_worker.py", "disc_worker.py", "LICENSE-ACEFILE.md",
    )]

    zpaq = runtime / "zpaq"
    zpaq_manifest = read_manifest(zpaq / "SOURCE.json")
    if (zpaq_manifest["source_archive"] != "https://mattmahoney.net/dc/zpaq715.zip"
            or set(zpaq_manifest["patched_source_sha256"]) != {"zpaq.cpp", "libzpaq.cpp", "libzpaq.h"}):
        raise ValueError("Invalid ZPAQ runtime manifest")
    files += checked_files(zpaq, {
        "obz-zpaq.exe": zpaq_manifest["runtime_sha256"], **zpaq_manifest["patched_source_sha256"],
    })
    files += [zpaq / name for name in ("SOURCE.json", "COPYING", "Makefile", "readme.txt", "zpaq.pod")]

    for path in files:
        if not path.is_file() or path.resolve() != path:
            raise ValueError(f"Missing or redirected runtime input: {path.relative_to(runtime)}")
    return sorted(path.relative_to(runtime) for path in files)
