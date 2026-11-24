"""Unit checks for check-tracked-artifacts.py."""
import importlib.util
from pathlib import Path

spec = importlib.util.spec_from_file_location("gate", Path(__file__).with_name("check-tracked-artifacts.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

big = 300 * 1024
rejected = [
    ("A", "docs/verification/run.png", 1000),
    ("A", "docs/report/screen.webp", 1000),
    ("M", "docs/notes/receipt.json", 1000),
    ("A", "docs/examples/nested/oversized.json", big),
    ("A", "docs/examples-old/receipt.json", 1000),
    ("A", "docs/examples/run.jsonl", 1000),
    ("A", "docs/x/output.log", 10),
    ("A", "scripts/proof.png", 1000),
    ("R", "evidence-r12/window.jpg", 1000),
    ("A", "scripts/receipt.bin", big),
    ("A", "obz-rs/src/new_table.rs", big),
]
accepted = [
    ("A", "assets/screenshots/workspace.png", big),
    ("A", "obz-gui/icons/icon.png", 1000),
    ("A", "obz-rs/tests/fixtures/sample.zip", big),
    ("A", "scripts/fixtures/sample/photo.jpg", 1000),
    ("A", "runtime/nanazip/NanaZip.Core.dll", big),
    ("M", "obz-rs/src/engine.rs", 500 * 1024),
    ("A", "docs/deep-extraction.md", 1000),
    ("A", "docs/examples/settings.json", 1000),
    ("M", "docs/examples/nested/options.json", gate.LIMIT),
    ("A", "runtime/nanazip/provenance.json", 1000),
    ("D", "docs/verification/old.png", None),
]
for change in rejected:
    assert gate.violations([change]), change
for change in accepted:
    assert not gate.violations([change]), change
print(f"Artifact gate: {len(rejected)} rejected and {len(accepted)} accepted cases passed")
