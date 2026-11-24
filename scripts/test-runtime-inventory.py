"""Verify x64 runtime inventory and default Windows build selection."""
import re
import shutil
import tempfile
import unittest
from itertools import takewhile
from pathlib import Path

from x64_runtime import x64_runtime_files


ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"


def yaml_block(document, name, indent=0):
    lines = document.splitlines()
    start = lines.index(" " * indent + name + ":") + 1
    return "\n".join(takewhile(lambda line: not line or line.startswith(" " * (indent + 1)), lines[start:]))


def workflow_events(document):
    return set(re.findall(r"(?m)^  (\w+):", yaml_block(document, "on")))


class DefaultWindowsBuild(unittest.TestCase):
    def test_arm64_workflows_have_no_automatic_triggers(self):
        dedicated = {"windows-arm64.yml", "arm64-gui-acceptance.yml"}
        arm64 = re.compile(r"\bwindows-(?:\d+|latest)-arm\b|(?:--target|-Target)\s+aarch64-pc-windows-msvc")
        for path in sorted(WORKFLOWS.iterdir()):
            if path.suffix not in (".yml", ".yaml"):
                continue
            document = path.read_text(encoding="utf-8")
            if path.name in dedicated or (path.name != "shell-package.yml" and arm64.search(document)):
                with self.subTest(workflow=path.name):
                    self.assertEqual(workflow_events(document), {"workflow_dispatch"})
                    self.assertRegex(document, arm64)
        for name in dedicated:
            self.assertTrue((WORKFLOWS / name).is_file())

    def test_explorer_pr_and_default_dispatch_select_x64(self):
        document = (WORKFLOWS / "shell-package.yml").read_text(encoding="utf-8")
        self.assertEqual(workflow_events(document), {"pull_request", "workflow_dispatch"})
        architecture = yaml_block(yaml_block(document, "on"), "architecture", indent=6)
        self.assertIn("options: [x64, arm64]", architecture)
        self.assertIn("default: x64", architecture)
        unsigned = yaml_block(document, "unsigned-package", indent=2)
        manual_arm = "github.event_name == 'workflow_dispatch' && inputs.architecture == 'arm64'"
        self.assertIn("if: github.event_name != 'workflow_dispatch' || !inputs.hosted_explorer", unsigned)
        self.assertIn(f"runs-on: ${{{{ {manual_arm} && 'windows-11-arm' || 'windows-latest' }}}}", unsigned)
        self.assertIn(f"SHELL_ARCH: ${{{{ {manual_arm} && 'arm64' || 'x64' }}}}", unsigned)

    def test_signed_explorer_requires_dispatch_and_explicit_arm64(self):
        document = (WORKFLOWS / "shell-package.yml").read_text(encoding="utf-8")
        signed = yaml_block(document, "hosted-explorer", indent=2)
        self.assertIn("github.event_name == 'workflow_dispatch' && inputs.hosted_explorer", signed)
        self.assertIn("runs-on: ${{ inputs.architecture == 'arm64' && 'windows-11-arm' || 'windows-latest' }}", signed)
        self.assertIn("if: inputs.architecture != 'arm64'", signed)
        self.assertIn("if: inputs.architecture == 'arm64'", signed)
        self.assertIn("run: throw 'Select arm64 architecture", signed)

    def test_normal_ci_and_local_packaging_keep_x64_defaults(self):
        document = (WORKFLOWS / "windows-ci.yml").read_text(encoding="utf-8")
        self.assertEqual(workflow_events(document), {"push", "pull_request", "workflow_dispatch"})
        self.assertIn("runs-on: windows-latest", yaml_block(document, "public-regressions", indent=2))
        for script, default in {
            "build.ps1": r"\$Target\s*=\s*'x86_64-pc-windows-msvc'",
            "build-hardened.ps1": r"\$Target\s*=\s*'x86_64-pc-windows-msvc'",
            "msix/Build-Component.ps1": r"\$Architecture\s*=\s*'x64'",
            "msix/Prepare-Package.ps1": r"\$Architecture\s*=\s*'x64'",
            "package.py": r'add_argument\("--architecture"[^\n]*default="x64"',
        }.items():
            with self.subTest(script=script):
                self.assertRegex((ROOT / "scripts" / script).read_text(encoding="utf-8"), default)


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
