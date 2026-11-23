"""Check ordinary selected ZIP extraction in a real Windows window."""

import argparse
import json
from pathlib import Path
import runpy
import tempfile
import time
import zipfile

import win32event
import win32process
from playwright.sync_api import Error, expect

shared = runpy.run_path(str(Path(__file__).with_name("check-nested-multi-layer.py")))
digest = shared["digest"]
private_process = shared["private_process"]
verify_package = shared["verify_package"]

SELECTED = ("a-selected.txt", "z-conflict.txt")
MEMBERS = {
    "a-selected.txt": b"selected without collision\n",
    "z-conflict.txt": b"selected with collision\n",
    "excluded.txt": b"unselected member in ZIP\n",
    "excluded-folder/hidden.txt": b"unselected folder member\n",
}
STRATEGIES = {
    "error": "停止并报告冲突",
    "rename": "自动重命名",
    "skip": "跳过已有文件",
    "overwrite": "替换已有文件",
}


def close_window(page, process):
    try:
        page.locator('[data-window="close"]').click()
    except Error:
        pass
    assert win32event.WaitForSingleObject(process, 10000) == win32event.WAIT_OBJECT_0
    assert win32process.GetExitCodeProcess(process) == 0


def collect_tree(root):
    files, dirs = {}, set()
    for path in root.rglob("*"):
        name = path.relative_to(root).as_posix()
        if path.is_dir():
            dirs.add(name)
        else:
            files[name] = digest(path)
    return {"files": files, "directories": sorted(dirs)}


def expected_tree(contents):
    return {
        "files": {name: digest(data) for name, data in contents.items()},
        "directories": sorted({parent.as_posix() for name in contents for parent in Path(name).parents
                               if parent != Path(".")}),
    }


def extraction_form(page, destination, strategy, selected=SELECTED):
    for name in selected:
        page.locator(f'#qa-rows input[data-select="{name}"], #qa-rows input[data-select="{name}/"]').check()
    expect(page.locator('#qa-rows input[data-select="excluded.txt"]')).not_to_be_checked()
    page.locator('#oa-toolbar [data-action="extract-menu"]').click()
    page.locator('#oa-menu [data-action="extract-standard"]').click()
    scope = page.locator('[name="extract-scope"][value="selected"]')
    expect(scope).to_be_checked()
    expect(scope.locator("xpath=..")).to_have_text("选中项目（2）")
    page.locator("#qa-destination").fill(str(destination))
    page.locator("#qa-smart").uncheck()
    expect(page.locator("#oa-deep-extract")).not_to_be_checked()
    expect(page.locator("#oa-recycle-after")).to_be_disabled()
    expect(page.locator("#oa-recycle-after")).not_to_be_checked()
    conflict = page.locator("#qa-conflict")
    assert conflict.locator("option").all_text_contents() == list(STRATEGIES.values())
    conflict.select_option(strategy)
    expect(page.locator("#oa-expected-output")).to_have_attribute("data-path", str(destination))
    return page.locator("#qa-conflict-help").inner_text()


def expand_task(page, status):
    phase = page.locator("#oa-task-phase")
    expect(phase).to_have_text(status)
    if page.locator("#oa-task-panel").get_attribute("data-expanded") != "true":
        page.locator("#oa-task-toggle").click()


def outcome(page, destination, failure, expected_files):
    expand_task(page, "失败" if failure else "已完成")
    if failure:
        retry = page.locator('#oa-task-panel [data-action^="extract-retry-"]')
        expect(retry).to_be_visible()
        expect(page.locator("#oa-task-output")).to_be_hidden()
    else:
        expect(page.locator("#oa-task-output")).to_be_visible()
    page.locator('[data-view="history"]').click()
    card = page.locator("[data-job-card]").filter(has_text="解压选中").first
    expect(card).to_be_visible()
    summary = card.locator(".qa-jobresult").first
    if failure:
        expect(summary).to_contain_text("目标位置已有同名项目")
        expect(summary).to_contain_text("z-conflict.txt")
        return {"status": "失败", "conflict_path": "out/z-conflict.txt"}
    expect(summary).to_have_text(f"已提取 {expected_files} 个文件至 {destination}")
    return {"status": "已完成", "output_directory": "out", "output_count": expected_files}


def exercise(args, fixture, occupant, strategy):
    archive = fixture / "ordinary-selection.zip"
    with zipfile.ZipFile(archive, "w") as package:
        for name, data in MEMBERS.items():
            package.writestr(zipfile.ZipInfo(name), data, compress_type=zipfile.ZIP_DEFLATED)
    archive_hash = digest(archive)
    destination = fixture / "out"
    sentinel = fixture / "outside.txt"
    sentinel.write_bytes(b"untouched neighbor outside destination\n")
    sentinel_hash = digest(sentinel)
    existing = {
        "outside-selection.txt": b"local destination neighbor\n",
        "excluded.txt": b"local bytes at an unselected member's name\n",
        "z-conflict (1).txt": b"existing first numeric name\n",
    }
    conflict = "z-conflict.txt" if occupant == "file" else "z-conflict.txt/keep.txt"
    existing[conflict] = b"preserved local collision content\n"
    for name, data in existing.items():
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    before = collect_tree(destination)
    assert before == expected_tree(existing)

    name = f"{occupant}-{strategy}"
    expected = dict(existing)
    failure = strategy == "error" or (occupant == "directory" and strategy == "overwrite")
    if not failure and strategy != "cancel":
        expected["a-selected.txt"] = MEMBERS["a-selected.txt"]
        if strategy == "rename":
            expected["z-conflict (2).txt"] = MEMBERS["z-conflict.txt"]
        elif strategy == "overwrite":
            expected["z-conflict.txt"] = MEMBERS["z-conflict.txt"]

    with private_process(args.exe, fixture, archive) as (page, process, _):
        help_text = extraction_form(page, destination, "overwrite" if strategy == "cancel" else strategy)
        page.screenshot(path=args.out / f"{name}-preview.png")
        if strategy == "cancel":
            expect(page.locator("#oa-task-panel")).to_be_hidden()
            page.locator("#qa-overlay").get_by_role("button", name="取消", exact=True).click()
            expect(page.locator("#qa-overlay")).to_be_hidden()
            expect(page.locator("#oa-task-panel")).to_be_hidden()
            facts = {"status": "表单已取消", "task_started": False}
        else:
            page.locator('[data-action="startextract"]').click()
            files = 1 if strategy == "skip" else 2
            facts = outcome(page, destination, failure, files)
        page.screenshot(path=args.out / f"{name}-result.png")
        after = collect_tree(destination)
        assert after == expected_tree(expected), (name, after)
        assert digest(archive) == archive_hash and digest(sentinel) == sentinel_hash
        with zipfile.ZipFile(archive) as package:
            assert package.testzip() is None and set(package.namelist()) == set(MEMBERS)
            assert {entry: package.read(entry) for entry in package.namelist()} == MEMBERS
        close_window(page, process)
    return {"occupant": occupant, "strategy": strategy, "strategy_help": help_text,
            "selected": list(SELECTED), "unselected": sorted(MEMBERS.keys() - set(SELECTED)),
            "preview": "out", **facts, "archive_sha256": archive_hash, "outside_sha256": sentinel_hash,
            "destination_before": before, "destination_after": after}


def blocked_parent(args, fixture, strategy):
    archive = fixture / "ordinary-folder-selection.zip"
    members = {"a-selected.txt": MEMBERS["a-selected.txt"], "picked/child.txt": b"selected folder member\n",
               "excluded.txt": MEMBERS["excluded.txt"]}
    with zipfile.ZipFile(archive, "w") as package:
        for name, data in members.items():
            package.writestr(zipfile.ZipInfo(name), data, compress_type=zipfile.ZIP_DEFLATED)
    archive_hash = digest(archive)
    destination = fixture / "out"
    destination.mkdir()
    blocker = destination / "picked"
    blocker.write_bytes(b"local file named like a required directory\n")
    neighbor = destination / "outside-selection.txt"
    neighbor.write_bytes(b"unrelated local neighbor\n")
    sentinel = fixture / "outside.txt"
    sentinel.write_bytes(b"untouched neighbor outside destination\n")
    sentinel_hash = digest(sentinel)
    before = collect_tree(destination)
    alternate = fixture / "retry-out"
    with private_process(args.exe, fixture, archive) as (page, process, _):
        help_text = extraction_form(page, destination, strategy, ("a-selected.txt", "picked"))
        page.locator('[data-action="startextract"]').click()
        expand_task(page, "失败")
        assert collect_tree(destination) == before
        message = page.locator("#oa-task-panel .qa-jobresult").first
        if args.expect_legacy_block_message:
            expect(message).to_contain_text("create output directory:")
            expect(message).to_contain_text("(os error 183)")
        else:
            expect(message).to_contain_text("解压需要创建文件夹，但这个位置已有同名文件。")
            expect(message).to_contain_text(str(blocker))
            expect(message).to_contain_text("请选择其他保存位置后重试。")
            expect(message).not_to_contain_text("os error")
        observed = message.text_content()
        page.screenshot(path=args.out / f"parent-{strategy}-result.png")
        retry = page.locator('#oa-task-panel [data-action^="extract-retry-"]')
        expect(retry).to_have_text("处理冲突并重试")
        retry.click()
        expect(page.get_by_text("原选定范围（2 项）")).to_be_visible()
        expect(page.locator(".oa-extract-problem")).to_have_text(observed)
        page.locator("#oa-retry-dest").fill(str(alternate))
        expect(page.locator("#oa-retry-output")).to_have_attribute("data-path", str(alternate))
        page.screenshot(path=args.out / f"parent-{strategy}-retry.png")
        page.locator('[data-action="extract-retry-submit"]').click()
        facts = outcome(page, alternate, False, 2)
        assert collect_tree(alternate) == expected_tree({name: data for name, data in members.items()
                                                         if name != "excluded.txt"})
        after = collect_tree(destination)
        assert after == before
        assert digest(archive) == archive_hash and digest(sentinel) == sentinel_hash
        assert shared["checked_members"](archive) == members
        close_window(page, process)
    return {"occupant": "parent", "strategy": strategy, "strategy_help": help_text,
            "selected": ["a-selected.txt", "picked"], "unselected": ["excluded.txt"], "preview": "out",
            "blocked_message": observed.replace(str(fixture), "<fixture>"), "retry": {**facts, "output_directory": "retry-out"},
            "archive_sha256": archive_hash, "outside_sha256": sentinel_hash,
            "destination_before": before, "destination_after": after, "retry_contents": collect_tree(alternate)}


def clear_fixture(fixture):
    deadline = time.monotonic() + 15
    while True:
        try:
            fixture.cleanup()
            return
        except OSError:  # WebView2 can hold its own cache briefly after window exit.
            if time.monotonic() >= deadline:
                raise
            time.sleep(.25)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("exe", "package", "hardening", "out", "fixtures-in"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--source", required=True)
    cases = [("file", "cancel"), *((kind, policy) for kind in ("file", "directory") for policy in STRATEGIES),
             ("parent", "skip"), ("parent", "rename")]
    parser.add_argument("--case", choices=[f"{kind}-{policy}" for kind, policy in cases])
    parser.add_argument("--expect-legacy-block-message", action="store_true",
                        help="Check the original Windows directory-creation message in the two parent cases")
    args = parser.parse_args()
    for name in ("exe", "package", "hardening", "fixtures_in"):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    args.out.mkdir(parents=True, exist_ok=False)
    args.staged_sha256 = None
    receipt = {"source": args.source, **verify_package(args), "strategies": STRATEGIES, "cases": []}
    for occupant, strategy in cases:
        name = f"{occupant}-{strategy}"
        if args.case and args.case != name:
            continue
        fixture = tempfile.TemporaryDirectory(prefix=f"{name}-", dir=args.fixtures_in)
        try:
            if occupant == "parent":
                result = blocked_parent(args, Path(fixture.name), strategy)
            else:
                result = exercise(args, Path(fixture.name), occupant, strategy)
            receipt["cases"].append(result)
        finally:
            clear_fixture(fixture)
        (args.out / "receipt.json").write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(f"PASS {name}", flush=True)


if __name__ == "__main__":
    main()
