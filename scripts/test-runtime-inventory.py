"""Release packaging must take only verified x64 runtime inputs."""
import shutil
import tempfile
import unittest
from pathlib import Path

from x64_runtime import x64_runtime_files


class RuntimeInventory(unittest.TestCase):
    def test_python_is_pinned_and_unreviewed_runtime_files_stay_out_of_the_package(self):
        source = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            for relative in ("runtime", "obz-rs/native/nana"):
                shutil.copytree(
                    source / relative, root / relative,
                    ignore=shutil.ignore_patterns("__pycache__", "nanazip-arm64"),
                )
            extra = Path("python/unreviewed.txt")
            (root / "runtime" / extra).write_text("local build artifact", encoding="utf-8")
            verified = x64_runtime_files(root)
            self.assertNotIn(extra, verified)
            self.assertNotIn(Path("zpaq/zpaq64.exe"), verified)
            self.assertIn(Path("python/python314._pth"), verified)
            self.assertIn(Path("python/ace_worker.py"), verified)

            (root / "runtime/python/python314._pth").write_text("changed", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "Runtime integrity mismatch: python/python314._pth"):
                x64_runtime_files(root)


if __name__ == "__main__":
    unittest.main()
