"""Reject QA evidence and large new files in a change: python check-tracked-artifacts.py BASE [HEAD] [--direct].

--direct compares the two trees (BASE..HEAD) for shallow CI checkouts without a merge base.

Screenshots, receipts and logs belong in PR comments or Actions artifacts, not in Git.
Only paths added, renamed or copied by the change are size-checked, so existing large
sources (for example engine.rs) can still be edited.
"""
import subprocess
import sys
from pathlib import PurePosixPath

LIMIT = 200 * 1024
EVIDENCE = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".json", ".jsonl", ".log", ".zip", ".7z"}
IMAGES = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"}
# Product artwork and test fixtures that legitimately carry images or large files.
IMAGE_HOMES = ("assets/", "obz-gui/icons/", "obz-gui/frontend/", "qing-shell/")
LARGE_HOMES = ("runtime/", "obz-rs/tests/fixtures/", "scripts/fixtures/", "assets/", "licenses/", "obz-rs/vendor/")
FIXTURE_PART = "/fixtures/"

HINT = "截图直接拖进 PR 评论，CI 回执和截图上传为 Actions artifact，长期保存的大文件放 Release 附件；docs/ 只放长期有效的说明。"


def violations(changes):
    """changes: (status letter, path, size in bytes or None for deletions)."""
    found = []
    for status, path, size in changes:
        if status == "D":
            continue
        suffix = PurePosixPath(path).suffix.lower()
        added = status in "ARC"
        small_doc_example = (path.startswith("docs/examples/") and suffix == ".json"
                             and size is not None and size <= LIMIT)
        if path.startswith("docs/") and suffix in EVIDENCE and not small_doc_example:
            found.append(f"{path}: docs/ 不保存截图、回执、日志或压缩包")
        elif added and suffix in IMAGES and not path.startswith(IMAGE_HOMES) and FIXTURE_PART not in f"/{path}":
            found.append(f"{path}: 新增图片只允许放在产品素材或测试样本目录")
        if added and size is not None and size > LIMIT and not path.startswith(LARGE_HOMES):
            found.append(f"{path}: 新增文件 {size // 1024} KiB，超过 {LIMIT // 1024} KiB 上限")
    return found


def changed(base, head, direct=False):
    out = subprocess.run(["git", "diff", "--name-status", "-z", "--find-renames", f"{base}{'..' if direct else '...'}{head}"],
                         check=True, capture_output=True).stdout.decode("utf-8").split("\0")
    items, i = [], 0
    while i < len(out) - 1:
        status = out[i][0]
        if status in "RC":
            path, i = out[i + 2], i + 3
        else:
            path, i = out[i + 1], i + 2
        size = None
        if status != "D":
            size = int(subprocess.run(["git", "cat-file", "-s", f"{head}:{path}"], check=True,
                                      capture_output=True, text=True).stdout)
        items.append((status, path, size))
    return items


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--direct"]
    if len(args) not in (1, 2):
        raise SystemExit(__doc__)
    problems = violations(changed(args[0], args[1] if len(args) == 2 else "HEAD", "--direct" in sys.argv))
    if problems:
        print("以下文件不应提交到仓库：", *problems, HINT, sep="\n")
        raise SystemExit(1)
    print("No QA evidence or oversized new files in this change")
