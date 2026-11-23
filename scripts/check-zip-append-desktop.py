"""Verify ordinary ZIP Add and mid-write cancellation in a private Windows desktop."""
import argparse
import importlib.util
import json
from pathlib import Path
import random
import tempfile
import time

import psutil


def load_desktop_helper():
    spec = importlib.util.spec_from_file_location('zip_add_desktop', Path(__file__).with_name('check-nested-multi-layer.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def running_writer(pid, image):
    editor = psutil.Process(pid)
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        for child in editor.children():
            try:
                if Path(child.exe()).resolve() != image or child.cmdline()[1:] != ['--internal-zip-append-worker']:
                    continue
                io = child.io_counters()
                if io.write_bytes >= 1024 * 1024 and io.read_bytes >= 2 * 1024 * 1024:
                    return child, io.write_bytes
            except (psutil.NoSuchProcess, psutil.AccessDenied):
                pass
        time.sleep(0.005)
    raise RuntimeError('The owned worker was not observed writing its ordinary ZIP candidate')


def exercise(desktop, args, temporary):
    archive = temporary/'sample.zip'
    before = desktop.archive({'retained.txt': b'ordinary original member'})
    archive.write_bytes(before)
    source = temporary/'larger.bin'
    rng = random.Random(618)
    with source.open('wb') as file:
        for _ in range(192):
            file.write(rng.randbytes(1024 * 1024))
    source_hash = desktop.digest(source)
    other = temporary/'nearby.txt'
    other.write_bytes(b'nearby ordinary file')
    with desktop.private_process(args.exe, temporary, archive) as (page, process, pid):
        desktop.expect(page.locator('[data-entry="retained.txt"]')).to_be_visible(timeout=45000)
        page.locator('[data-action="add"]:visible').first.click()
        dialog = page.locator('.oa-add-dialog')
        desktop.expect(dialog.locator('#qa-update-mode')).to_have_value('add')
        dialog.locator('.oa-source-paths summary').click()
        paths = dialog.locator('#qa-edit-inputs')
        paths.fill(str(source))
        dialog.locator('[data-action="add-submit"]').click()
        writer, bytes_before_cancel = running_writer(pid, args.exe.with_name('obz-worker.exe'))
        dialog.locator('[data-action="add-cancel-task"]').click()
        desktop.expect(dialog.locator('#qa-formerror')).to_contain_text('任务已取消', timeout=45000)
        writer.wait(timeout=20)
        assert archive.read_bytes() == before and desktop.digest(source) == source_hash
        assert other.read_bytes() == b'nearby ordinary file'
        assert not list(temporary.glob('.qing-zip-stage-*'))

        first, second = temporary/'one.txt', temporary/'two.txt'
        first.write_bytes(b'first ordinary input')
        second.write_bytes(b'')
        paths.fill(f'{first}\n{second}')
        dialog.locator('[data-action="add-submit"]').click()
        desktop.expect(dialog).not_to_be_visible(timeout=45000)
        desktop.expect(page.locator('[data-entry="one.txt"]')).to_be_visible(timeout=45000)
        desktop.expect(page.locator('[data-entry="two.txt"]')).to_be_visible()
        assert desktop.checked_members(archive) == {'one.txt': first.read_bytes(), 'two.txt': second.read_bytes(),
                                                    'retained.txt': b'ordinary original member'}
        assert not list(temporary.glob('.qing-zip-stage-*'))
        descendants = psutil.Process(pid).children(recursive=True)
        try:
            page.locator('[data-window="close"]').click()
        except desktop.Error:
            pass  # A successful native close can disconnect WebView2 first.
        assert desktop.win32event.WaitForSingleObject(process, 10000) == desktop.win32event.WAIT_OBJECT_0
        assert desktop.win32process.GetExitCodeProcess(process) == 0
        _, alive = psutil.wait_procs(descendants, timeout=10)
        assert not alive, 'A test GUI child still holds the private profile'
    return {'cancelled_after_actual_worker_write_bytes': bytes_before_cancel, 'writer_exited': True,
            'original_sources_and_sibling_preserved': True, 'normal_add_verified': True, 'private_stage_cleaned': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--package', type=Path)
    parser.add_argument('--hardening', type=Path)
    parser.add_argument('--staged-sha256', nargs=3, metavar=('GUI', 'CLI', 'WORKER'))
    args = parser.parse_args()
    if not args.staged_sha256 and (not args.hardening or not args.package):
        parser.error('Supply staged hashes, or both the exact portable package and its hardening receipt')
    args.exe = args.exe.resolve(strict=True)
    desktop = load_desktop_helper()
    package = desktop.verify_package(args)
    with tempfile.TemporaryDirectory(prefix='qing-zip-add-desktop-') as folder:
        report = exercise(desktop, args, Path(folder))
    with args.out.open('x', encoding='utf-8') as output:
        json.dump({**package, **report}, output, indent=2)


if __name__ == '__main__':
    main()
