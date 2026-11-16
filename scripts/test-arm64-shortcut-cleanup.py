"""Tiny ordinary-file tests of the hosted shortcut fixture cleanup only.

Extract the real function via AST: never import the package/installer driver,
resolve Desktop, run an installer, or touch any user shortcut.
"""
import argparse
import ast
import ctypes
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import threading
import time
from unittest.mock import patch

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--receipt', type=Path, required=True)
    args = parser.parse_args()
    assert os.name == 'nt' and not args.receipt.exists()
    driver = Path(__file__).with_name('check-arm64-package.py')
    module = ast.parse(driver.read_text(encoding='utf-8'))
    function = next(n for n in module.body if isinstance(n, ast.FunctionDef)
                    and n.name == 'remove_fixture_shortcut')
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    context = dict(Path=Path, time=time, stat=stat, sha=digest)
    exec(compile(ast.Module(body=[function], type_ignores=[]), str(driver), 'exec'), context)
    remove = context['remove_fixture_shortcut']
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                                   ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    kernel.CreateFileW.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]

    def lock(path):
        handle = kernel.CreateFileW(str(path), 0x80000000, 3, None, 3, 0x00200000, None)
        assert handle not in (None, ctypes.c_void_p(-1).value), ctypes.get_last_error()
        return handle

    rows = []
    with tempfile.TemporaryDirectory(prefix='qing-shortcut-cleanup-unit-') as temporary:
        root = Path(temporary)
        for mode in ('transient-lock', 'persistent-lock', 'changed-digest', 'unknown-error'):
            path = root / (mode + '.lnk')
            path.write_bytes(b'ordinary synthetic shortcut bytes')
            original = digest(path)
            started = time.monotonic()
            if mode == 'transient-lock':
                handle = lock(path)
                worker = threading.Thread(target=lambda: (time.sleep(0.15), kernel.CloseHandle(handle)))
                worker.start()
                try:
                    remove(path, original, timeout=2, interval=0.02)
                finally:
                    worker.join()
                assert not path.exists() and time.monotonic() - started >= 0.1
                rows.append({'case': mode, 'deleted_after_release': True})
            elif mode == 'persistent-lock':
                handle = lock(path)
                try:
                    try:
                        remove(path, original, timeout=0.12, interval=0.02)
                        raise AssertionError('Persistent lock was swallowed')
                    except OSError as error:
                        assert error.winerror in (32, 33)
                    assert path.exists() and digest(path) == original
                finally:
                    kernel.CloseHandle(handle)
                rows.append({'case': mode, 'retained_with_error': True})
            elif mode == 'changed-digest':
                handle = lock(path)
                changed = threading.Event()
                def replace_contents():
                    time.sleep(0.06)
                    path.write_bytes(b'changed fixture, never owned for deletion')
                    changed.set()
                    kernel.CloseHandle(handle)
                worker = threading.Thread(target=replace_contents)
                worker.start()
                try:
                    try:
                        remove(path, original, timeout=2, interval=0.02)
                        raise AssertionError('Changed content was deleted')
                    except RuntimeError as error:
                        assert 'contents changed' in str(error)
                finally:
                    worker.join()
                assert changed.is_set() and path.read_bytes() == b'changed fixture, never owned for deletion'
                rows.append({'case': mode, 'changed_bytes_preserved': True})
            else:
                error = OSError('injected non-sharing error')
                error.winerror = 5
                with patch.object(Path, 'unlink', side_effect=error) as unlink:
                    try:
                        remove(path, original, timeout=2)
                        raise AssertionError('Access error was swallowed')
                    except OSError as caught:
                        assert caught is error
                    assert unlink.call_count == 1
                assert path.exists() and digest(path) == original
                rows.append({'case': mode, 'single_attempt_and_original_retained': True})
            rows[-1]['seconds'] = time.monotonic() - started
    report = {'passed': True, 'scope': 'Synthetic ordinary files in temporary directory; no Desktop access',
              'driver_sha256': digest(driver), 'test_sha256': digest(Path(__file__)), 'cases': rows}
    with args.receipt.open('x', encoding='utf-8') as output:
        json.dump(report, output, indent=2)
    print(json.dumps(report))

if __name__ == '__main__':
    main()
