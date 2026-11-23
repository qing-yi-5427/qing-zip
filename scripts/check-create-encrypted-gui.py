"""Verify encrypted archives created by the real Windows creation dialog.

Requires Playwright, pywin32, and an independent 7-Zip executable. Passwords
are random per run and never saved; the reader receives them in its arguments.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import secrets
import struct
import subprocess
import tempfile
import time
import zipfile

import win32api
import win32con
import win32event
import win32process
from playwright.sync_api import Error, expect, sync_playwright


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def reader(reference, operation, archive, password, destination=None):
    args = [str(reference), operation, '-y', '-p' + password, '-bd']
    if destination is not None:
        args.append('-o' + str(destination))
    args.extend(['--', str(archive)])
    # Never print the command output: some tools echo interactive input.
    try:
        return subprocess.run(args, input=password + '\n', text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              timeout=60, creationflags=subprocess.CREATE_NO_WINDOW)
    except subprocess.TimeoutExpired:
        raise RuntimeError('The independent reader timed out') from None


def fill_password(page, selector, password):
    try:
        page.locator(selector).fill(password)
    except Error:
        # Playwright can include fill arguments in its exception text.
        raise RuntimeError('Could not fill a password field') from None


def validate_archive(reference, archive, source, scratch, secret, wrong, encrypted_names):
    before = sha(archive)
    empty = reader(reference, 'l', archive, '')
    invalid_list = reader(reference, 'l', archive, wrong)
    correct_list = reader(reference, 'l', archive, secret)
    assert correct_list.returncode == 0 and source.name in correct_list.stdout
    if encrypted_names:
        assert empty.returncode != 0 and invalid_list.returncode != 0
        assert source.name not in empty.stdout and source.name not in invalid_list.stdout
    else:
        assert empty.returncode == 0 and source.name in empty.stdout
    results = {}
    for label, password in [('empty', ''), ('wrong', wrong), ('correct', secret)]:
        destination = scratch / (archive.stem + '-' + label)
        destination.mkdir()
        result = reader(reference, 'x', archive, password, destination)
        output = destination / source.name
        files = [item for item in destination.rglob('*') if item.is_file()]
        if label == 'correct':
            assert result.returncode == 0 and files == [output] and sha(output) == sha(source)
        else:
            assert result.returncode != 0
            assert all(item.stat().st_size == 0 for item in files)
        results[label] = {'exit_code': result.returncode, 'files': len(files),
                          'all_files_empty': all(item.stat().st_size == 0 for item in files),
                          'matches_source': output.is_file() and sha(output) == sha(source)}
    assert sha(archive) == before
    return {'archive_sha256': before, 'encrypted_names': encrypted_names,
            'empty_list_exit_code': empty.returncode, 'wrong_list_exit_code': invalid_list.returncode,
            'correct_list_exit_code': correct_list.returncode, 'extractions': results}


def run(exe, reference, root, out, scenario):
    secret, wrong = secrets.token_hex(16), secrets.token_hex(16)
    assert secret != wrong
    source, sentinel = root / 'input.txt', root / 'neighbor.txt'
    source.write_bytes(b'ordinary synthetic file\n' * 4096)
    sentinel.write_bytes(b'unrelated synthetic neighbor\n')
    initial = {'source': sha(source), 'sentinel': sha(sentinel)}
    profile = root / 'profile'
    env = {**os.environ, 'LOCALAPPDATA': str(root / 'local'), 'APPDATA': str(root / 'roaming'),
           'TEMP': str(root), 'TMP': str(root), 'WEBVIEW2_USER_DATA_FOLDER': str(profile),
           'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS': '--remote-debugging-port=0 --remote-debugging-address=127.0.0.1'}
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    user32.CreateDesktopW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID]
    user32.CreateDesktopW.restype = wintypes.HANDLE
    user32.CloseDesktop.argtypes = [wintypes.HANDLE]
    user32.CloseDesktop.restype = wintypes.BOOL
    desktop_name = f'QzEncryptedCreate{os.getpid()}'
    desktop = user32.CreateDesktopW(desktop_name, None, None, 0, 0x01ff, None)
    if not desktop:
        raise ctypes.WinError(ctypes.get_last_error())
    process = thread = None
    try:
        startup = win32process.STARTUPINFO()
        startup.lpDesktop = desktop_name
        args = [str(exe), '--compress-files', str(source)]
        process, thread, _, _ = win32process.CreateProcess(str(exe), subprocess.list2cmdline(args), None, None, False,
            win32con.CREATE_UNICODE_ENVIRONMENT, env, str(root), startup)
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if win32event.WaitForSingleObject(process, 0) == win32event.WAIT_OBJECT_0:
                raise RuntimeError('Test process exited before WebView2 was ready')
            ports = list(profile.rglob('DevToolsActivePort'))
            if ports:
                break
            time.sleep(.1)
        else:
            raise TimeoutError('WebView2 debugging endpoint unavailable')
        with sync_playwright() as automation:
            browser = automation.chromium.connect_over_cdp('http://127.0.0.1:' + ports[0].read_text().splitlines()[0])
            page = browser.contexts[0].pages[0]
            expect(page.locator('#qa-format')).to_be_visible(timeout=60000)
            page.locator('#oa-password-button').click()
            fill_password(page, '#qa-new-password', secret)
            fill_password(page, '#oa-password-again', secret)
            masks = [page.locator('#qa-inputs:visible'), page.locator('#qa-save:visible'),
                     page.locator('input[type=password]:visible')]
            if scenario == '7z':
                page.locator('#qa-format').select_option('tar')
                expect(page.locator('#oa-password-unsupported')).to_be_visible()
                tar = root / 'blocked.tar'
                page.locator('#qa-save').fill(str(tar))
                page.locator('[data-action=startcompress]').click()
                expect(page.locator('#qa-formerror')).to_contain_text('当前格式无法使用已填写的密码')
                assert not tar.exists()
                page.screenshot(path=out / 'unsupported.png', mask=masks, mask_color='#303642')
            facts = {}
            for fmt in (scenario,):
                page.locator('#qa-format').select_option(fmt)
                archive = root / ('created.' + fmt)
                page.locator('#qa-save').fill(str(archive))
                if fmt == '7z':
                    if not page.locator('#oa-password-panel').is_visible():
                        page.locator('#oa-password-button').click()
                    expect(page.locator('#oa-header-encryption')).to_be_checked()
                    assert page.locator('#qa-new-password').input_value() == secret
                page.screenshot(path=out / (fmt + '-form.png'), mask=masks, mask_color='#303642')
                completed_before = page.get_by_text('检查通过', exact=True).count()
                page.locator('[data-action=startcompress]').click()
                expect(page.get_by_text('检查通过', exact=True)).to_have_count(completed_before + 1, timeout=60000)
                assert archive.is_file()
                facts[fmt] = validate_archive(reference, archive, source, root, secret, wrong, fmt == '7z')
                if fmt == '7z':
                    page.screenshot(path=out / '7z-complete.png', mask=masks, mask_color='#303642')
                if fmt == 'zip':
                    with zipfile.ZipFile(archive) as z:
                        info = z.getinfo(source.name)
                        assert info.flag_bits & 1 and info.compress_type == 99
                        extra = info.extra
                        aes = None
                        while len(extra) >= 4:
                            kind, length = struct.unpack_from('<HH', extra)
                            payload, extra = extra[4:4 + length], extra[4 + length:]
                            if kind == 0x9901:
                                assert len(payload) == 7
                                aes = payload[4]
                        assert aes == 3, 'ZIP AES strength is not 256 bits'
                        facts[fmt]['zip_encryption_flag'] = True
                        facts[fmt]['zip_method'] = info.compress_type
                        facts[fmt]['aes_strength_bits'] = 256
                assert sha(source) == initial['source'] and sha(sentinel) == initial['sentinel']
            try:
                page.evaluate('window.__TAURI__.core.invoke("window_action",{action:"close"})')
            except Error:
                pass
            assert win32event.WaitForSingleObject(process, 15000) == win32event.WAIT_OBJECT_0
            assert win32process.GetExitCodeProcess(process) == 0
            return {'desktop': 'private Win32 desktop; real WebView2 via CDP', 'gui_sha256': sha(exe),
                    'reference_sha256': sha(reference), 'source_sha256': initial['source'],
                    'sentinel_sha256': initial['sentinel'], 'tar_submission_blocked_no_output': True if scenario == '7z' else None,
                    'source_and_sentinel_preserved': True, 'normal_gui_exit': True, 'archives': facts}
    finally:
        if process:
            if win32event.WaitForSingleObject(process, 8000) != win32event.WAIT_OBJECT_0:
                win32process.TerminateProcess(process, 1)
                win32event.WaitForSingleObject(process, 10000)
            win32api.CloseHandle(process)
        if thread:
            win32api.CloseHandle(thread)
        if not user32.CloseDesktop(desktop):
            raise ctypes.WinError(ctypes.get_last_error())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--reference', type=Path, required=True)
    parser.add_argument('--fixtures-in', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    running = subprocess.run(['powershell', '-NoProfile', '-Command',
        "[Diagnostics.Process]::GetProcessesByName('qing-zip').Id; [Diagnostics.Process]::GetProcessesByName('obz-gui').Id"],
        capture_output=True, text=True, check=True)
    if running.stdout.strip():
        raise RuntimeError('Another qing-zip window is running')
    args.out.mkdir(parents=True, exist_ok=False)
    args.fixtures_in.mkdir(parents=True, exist_ok=True)
    from runpy import run_path
    fixture = run_path(str(Path(__file__).with_name('check-password-retry-gui.py')))['private_fixture']
    with fixture(args.fixtures_in) as root:
        scenarios = {}
        for name in ('7z', 'zip'):
            case_root = root / name
            case_root.mkdir()
            scenarios[name] = run(args.exe.resolve(strict=True), args.reference.resolve(strict=True),
                                  case_root, args.out.resolve(), name)
        receipt = {'scenarios': scenarios}
    receipt['private_fixture_removed'] = True
    (args.out / 'receipt.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(receipt, ensure_ascii=True))


if __name__ == '__main__':
    main()
