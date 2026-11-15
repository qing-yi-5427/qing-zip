"""Enumerate only version-controlled modified dependency source files."""
import subprocess
from pathlib import Path


def source_members(root: Path) -> dict[str, bytes]:
    relative = "obz-rs/vendor/lzma-rust2"
    names = subprocess.check_output(
        ["git", "-C", str(root), "ls-files", "-z", "--", relative]
    ).decode("utf-8").split("\0")
    result = {}
    for name in filter(None, names):
        path = root / name
        suffix = Path(name).relative_to(relative).as_posix()
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"Missing or non-regular tracked dependency source: {name}")
        result["lzma-rust2-modified-0.23.0/" + suffix] = path.read_bytes()
    required = {"LICENSE", "Cargo.toml", "UPSTREAM.json", "QING-ZIP-MODIFICATIONS.md", "src/lib.rs"}
    if not required.issubset({name.split("/", 1)[1] for name in result}):
        raise ValueError("Incomplete tracked modified dependency source")
    return result
