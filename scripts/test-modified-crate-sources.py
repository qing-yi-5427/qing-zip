"""Small end-to-end source packager tests; never download real assets."""
import hashlib
import json
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from modified_crate_sources import source_members

class SourcePackageTests(unittest.TestCase):
    def test_untracked_outputs_excluded_and_existing_asset_reusable(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            scripts = root / "scripts"
            scripts.mkdir()
            for name in ("package-engine-sources.py", "modified_crate_sources.py"):
                shutil.copyfile(Path(__file__).parent / name, scripts / name)
            vendor = root / "obz-rs/vendor/lzma-rust2"
            for name in ("LICENSE", "Cargo.toml", "UPSTREAM.json", "QING-ZIP-MODIFICATIONS.md", "src/lib.rs", "Cargo.lock", ".cargo_vcs_info.json"):
                path = vendor / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(("synthetic " + name).encode())
            patches = root / "obz-rs/native/nana"
            patches.mkdir(parents=True)
            for name in ("zip-aes-acp.patch", "zip-aes-build.md"):
                (patches / name).write_text("fixture " + name)
            subprocess.run(["git", "-C", str(root), "add", "obz-rs"], check=True)
            (root / "obz-gui").mkdir()
            (root / "obz-gui/tauri.conf.json").write_text('{"version":"test"}')
            for component, relative, url in (
                ("nanazip", "nanazip-7.0.1882.0/NanaZip-7.0.1882.0-source.zip", "https://api.github.com/repos/M2Team/NanaZip/fixture"),
                ("7zip", "7zip-26.06/7z2606-src.7z", "https://github.com/ip7z/7zip/fixture"),
            ):
                cached = root / "tools" / relative
                cached.parent.mkdir(parents=True, exist_ok=True)
                cached.write_bytes(b"synthetic source asset")
                runtime = root / "runtime" / component
                runtime.mkdir(parents=True)
                (runtime / "provenance.json").write_text(json.dumps({"source_archive_url":url,"source_archive_sha256":hashlib.sha256(cached.read_bytes()).hexdigest()}))
            command = [__import__("sys").executable, str(scripts / "package-engine-sources.py")]
            subprocess.run(command, check=True, capture_output=True)
            archive = root / "dist/qing-zip-test-engine-sources.zip"
            with zipfile.ZipFile(archive) as zipped:
                for name, data in source_members(root).items():
                    self.assertEqual(zipped.read(name), data)
                for name in ("zip-aes-acp.patch", "zip-aes-build.md"):
                    self.assertEqual(zipped.read("nanazip-modified/" + name), (patches / name).read_bytes())
            before = archive.read_bytes()
            (vendor / "target").mkdir()
            (vendor / "target/private-build-output").write_bytes(b"must not ship")
            (vendor / "untracked.tmp").write_bytes(b"must not ship")
            subprocess.run(command, check=True, capture_output=True)
            self.assertEqual(before, archive.read_bytes())
            self.assertFalse(any("target/" in name or name.endswith(".tmp") for name in source_members(root)))
            (vendor / "LICENSE").unlink()
            with self.assertRaises(ValueError):
                source_members(root)

if __name__ == "__main__":
    unittest.main()
