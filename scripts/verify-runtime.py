"""Verify the committed x64 runtime against the reviewed release manifests."""
from pathlib import Path

from x64_runtime import x64_runtime_files


if __name__ == "__main__":
    try:
        files = x64_runtime_files(Path(__file__).resolve().parents[1])
    except (KeyError, OSError, ValueError) as error:
        raise SystemExit(str(error)) from error
    print(f"Verified {len(files)} x64 runtime files (7-Zip, NanaZip, Python, ZPAQ) and NanaZip headers")
