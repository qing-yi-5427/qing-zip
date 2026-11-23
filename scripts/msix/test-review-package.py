"""Build an unsigned review package and check its external payload without registering it."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET
import zipfile


def verify(component, architecture, sdk, work):
    prepare = Path(__file__).with_name("Prepare-Package.ps1")

    def package(source, output):
        return subprocess.run([
            "pwsh", "-NoProfile", "-NonInteractive",
            "-File", str(prepare), "-ComponentDirectory", str(source),
            "-OutputDirectory", str(output), "-Publisher", "CN=QingZip QA",
            "-Version", "1.0.0.0", "-Architecture", architecture.upper(),
            "-MakeAppxPath", str(sdk / "makeappx.exe"), "-ManifestToolPath", str(sdk / "mt.exe"),
        ], stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    output = work / "review"
    result = package(component, output)
    if result.returncode:
        raise RuntimeError(result.stdout.decode(errors="replace"))
    with zipfile.ZipFile(output / f"qing-zip-shell-1.0.0.0-{architecture}.msix") as archive:
        manifest = ET.fromstring(archive.read("AppxManifest.xml"))
        identity = manifest.find("{*}Identity")
        assert identity.attrib["ProcessorArchitecture"] == architecture
        commands = {item.attrib["Type"]: {verb.attrib["Id"]: verb.attrib["Clsid"] for verb in item}
                    for item in manifest.findall(".//{*}ItemType")}
        files_id = commands["*"]["QingZipCompressFiles"]
        assert files_id == "182148F9-276F-4DB8-8136-A181FB81353F"
        assert commands["Directory"] == {"QingZipCompress": "A65CB9C7-E692-4D0B-A5A1-4F19530EE312"}
        assert any(entry.attrib == {"Id": files_id, "Path": "qing-shell.dll", "ThreadingModel": "STA"}
                   for entry in manifest.findall(".//{*}Class"))
        expected = json.loads(archive.read("external-payload.json"))
    assert set(expected) == {"qing-shell.dll", "qing-shell-host.exe", r"Assets\AppIcon.png"}
    for name, digest in expected.items():
        assert hashlib.sha256((output / "shell" / name).read_bytes()).hexdigest() == digest.lower()

    names = ("qing-shell.dll", "qing-shell-host.exe")
    for mismatched in names:
        source = work / mismatched
        source.mkdir()
        for name in names:
            shutil.copyfile(component / name, source / name)
        # Model a mixed build directory. These copies are only read by the packager.
        with (source / mismatched).open("r+b") as stream:
            stream.seek(0x3C)
            offset, = struct.unpack("<I", stream.read(4))
            stream.seek(offset + 4)
            stream.write(struct.pack("<H", 0xAA64 if architecture == "x64" else 0x8664))
        rejected = work / (mismatched + "-output")
        result = package(source, rejected)
        assert result.returncode and (mismatched + " has PE machine").encode() in result.stdout
        assert not rejected.exists()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("component", type=Path, help="Release build directory")
    parser.add_argument("architecture", choices=("x64", "arm64"))
    parser.add_argument("sdk", type=Path, help="SDK bin directory containing makeappx.exe and mt.exe")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="qing-shell-package-") as directory:
        verify(args.component.resolve(), args.architecture, args.sdk.resolve(), Path(directory))
    print(f"PASS {args.architecture}: unsigned MSIX, file and folder verbs, three payload hashes, two mixed-build rejections")
