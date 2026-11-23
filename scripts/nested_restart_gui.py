"""Observe real nested ZIP close and restart on a disposable Windows runner."""
from contextlib import suppress
import argparse
import hashlib
import io
import json
import os
import random
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import winreg
import zipfile

from playwright.sync_api import expect, sync_playwright
from arm_gui_lifecycle import guarded_run


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def members(payload):
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        return {name: hashlib.sha256(archive.read(name)).hexdigest() for name in archive.namelist()}


def fixture(root, large=False):
    inner = io.BytesIO()
    with zipfile.ZipFile(inner, 'w', zipfile.ZIP_STORED) as archive:
        archive.writestr('before.txt', b'editable nested content\n')
        archive.writestr('unchanged.txt', b'preserve nested sibling\n')
        if large:
            archive.writestr('bulk.bin', random.Random(86).randbytes(64 * 1024 * 1024))
    payload = inner.getvalue()
    outer = root / 'outer.zip'
    with zipfile.ZipFile(outer, 'w', zipfile.ZIP_STORED) as archive:
        archive.writestr('inner.zip', payload)
        archive.writestr('sibling.zip', payload)
        archive.writestr('outer-only.txt', b'preserve outer sibling\n')
    (root / 'neighbour.txt').write_bytes(b'preserve ordinary neighbour\n')
    return outer


def inspect_outer(path):
    with zipfile.ZipFile(path) as archive:
        return {'outer_sha256': digest(path),
                'inner': members(archive.read('inner.zip')),
                'sibling_sha256': hashlib.sha256(archive.read('sibling.zip')).hexdigest(),
                'outer_only_sha256': hashlib.sha256(archive.read('outer-only.txt')).hexdigest()}


def workcopies(temp):
    return sorted(temp.rglob('inner.zip'))


def connect(app, port, driver):
    endpoint = f'http://127.0.0.1:{port}'
    deadline = time.monotonic() + 75
    while time.monotonic() < deadline:
        if app.poll() is not None:
            raise RuntimeError(f'GUI exited during startup: {app.returncode}')
        with suppress(OSError, urllib.error.URLError):
            with urllib.request.urlopen(endpoint + '/json/version', timeout=1):
                browser = driver.chromium.connect_over_cdp(endpoint)
                while time.monotonic() < deadline:
                    if browser.contexts and browser.contexts[0].pages:
                        return browser, browser.contexts[0].pages[0]
                    time.sleep(.1)
                raise RuntimeError('WebView2 page unavailable')
        time.sleep(.2)
    raise RuntimeError('WebView2 debugging endpoint unavailable')


def launch(exe, outer, root, env, port, driver, lifetime):
    process = subprocess.Popen([str(exe), str(outer)], cwd=root, env=env, stdin=subprocess.DEVNULL)
    lifetime['proc'] = process
    browser, page = connect(process, port, driver)
    expect(page.locator('[data-entry="inner.zip"]')).to_be_visible(timeout=60000)
    return process, browser, page


def edit(page, temp, password=None):
    page.locator('[data-entry="inner.zip"]').dblclick()
    if password is not None:
        password_field = page.locator('#oa-nested-password')
        expect(password_field).to_be_visible(timeout=30000)
        password_field.fill(password)
        page.locator('[data-action="nested-open-submit"]').click()
    page.locator('[data-entry="before.txt"]').click(timeout=60000)
    page.keyboard.press('F2')
    page.locator('#qa-rename').fill('after.txt')
    page.locator('[data-action="rename-submit"]').click()
    expect(page.locator('[data-entry="after.txt"]')).to_be_visible(timeout=60000)
    expect(page.locator('.oa-nested-bar')).to_contain_text('更改尚未写回')
    copies = workcopies(temp)
    if len(copies) != 1:
        raise RuntimeError(f'Expected one working copy, got {len(copies)}')
    return copies[0]


def stop_own(process, browser):
    if process.poll() is None:
        process.terminate()
    process.wait(timeout=20)
    with suppress(Exception):
        browser.close()


def retained_dialog(page, evidence, case):
    page.locator('[data-action="app-menu"]').click()
    page.locator('[data-menu="edit"]').click()
    page.locator('[data-action="external-copies"]').click()
    dialog = page.locator('#qa-overlay:not([hidden])')
    expect(dialog).to_be_visible(timeout=10000)
    text = dialog.inner_text()
    page.screenshot(path=str(evidence / f'{case}-retained.png'))
    return {'text': text, 'reports_no_retained_copies': '没有保留的编辑副本' in text}


def save_retained_copy(page, app, destination, evidence):
    from pywinauto import Desktop, keyboard
    initial_count = page.locator('#oa-retained-edit option').count()
    page.locator('[data-action="external-retained-save"]').click()
    picker = Desktop(backend="uia").window(process=app.pid, class_name="#32770")
    picker.wait("visible", timeout=20)
    picker.set_focus()
    keyboard.send_keys("^l")
    keyboard.send_keys(str(destination.parent), with_spaces=True, pause=.004)
    keyboard.send_keys("{ENTER}")
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        toolbars = [bar.window_text() for bar in picker.descendants(control_type="ToolBar")]
        if any(str(destination.parent).casefold() in title.casefold() for title in toolbars):
            break
        time.sleep(.2)
    else:
        raise RuntimeError("Save As did not navigate to the empty recovery directory")
    filename = picker.child_window(auto_id="FileNameControlHost").child_window(control_type="Edit")
    filename.wait("visible", timeout=10)
    filename.click_input()
    keyboard.send_keys("^a")
    keyboard.send_keys(destination.name, with_spaces=True, pause=.02)
    if filename.get_value() != destination.name:
        raise RuntimeError("Unexpected native Save As filename")
    picker.capture_as_image().save(evidence / "recovered-save-as.png")
    picker.child_window(auto_id="1", control_type="Button").click_input()
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if destination.is_file():
            try:
                with zipfile.ZipFile(destination) as archive:
                    if archive.testzip() is None:
                        remaining = page.evaluate("window.__TAURI__.core.invoke('retained_edits')")
                        if len(remaining) < initial_count:
                            return
            except (OSError, zipfile.BadZipFile):
                pass
        time.sleep(.2)
    raise RuntimeError("Native Save As did not produce a readable ZIP and consume its record")


def main():
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted' or os.environ.get('RUNNER_OS') != 'Windows':
        raise RuntimeError('Disposable hosted Windows required')
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--large', action='store_true', help='Use a deterministic 64 MiB stored inner ZIP member')
    parser.add_argument('--expect-recovery', action='store_true', help='Require interrupted edit in retained copies and save it through the native dialog')
    options = parser.parse_args()
    exe = options.exe.resolve(strict=True)
    evidence = Path('nested-restart-evidence').resolve()
    evidence.mkdir(exist_ok=True)
    receipt = evidence / 'nested-restart.json'
    report = {'status': 'running', 'scenarios': {}, 'not_observed': []}
    def save():
        receipt.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf8')
    try:
        with tempfile.TemporaryDirectory(prefix='qz-nested-restart-', dir=os.environ['RUNNER_TEMP'], ignore_cleanup_errors=True) as temporary:
            top = Path(temporary)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            arguments = f'--remote-debugging-port={port} --remote-debugging-address=127.0.0.1'
            policy = winreg.CreateKeyEx(winreg.HKEY_LOCAL_MACHINE, r'SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments', 0, winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY)
            session = {'passed': False}
            with guarded_run(winreg, policy, exe.name, arguments, session, evidence / 'desktop-session.json') as lifetime, sync_playwright() as driver:
                for case in ('cancel-discard', 'save-close', 'terminate'):
                    root = top / case
                    root.mkdir()
                    temp = root / 'temp'
                    temp.mkdir()
                    fixture_started = time.perf_counter()
                    outer = fixture(root, large=options.large)
                    fixture_seconds = round(time.perf_counter() - fixture_started, 3)
                    before = inspect_outer(outer)
                    neighbour_hash = digest(root / 'neighbour.txt')
                    env = dict(os.environ, TEMP=str(temp), TMP=str(temp), LOCALAPPDATA=str(root / 'local'), APPDATA=str(root / 'roaming'), WEBVIEW2_USER_DATA_FOLDER=str(root / 'webview'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=arguments)
                    entry = report['scenarios'][case] = {'before': before, 'neighbour_sha256': neighbour_hash, 'large_member_bytes': 64 * 1024 * 1024 if options.large else 0, 'fixture_prepare_seconds': fixture_seconds}
                    save()
                    app, browser, page = launch(exe, outer, root, env, port, driver, lifetime)
                    entry['first_gui_pid'] = app.pid
                    edit_started = time.perf_counter()
                    working = edit(page, temp)
                    entry['edit_elapsed_seconds'] = round(time.perf_counter() - edit_started, 3)
                    entry['edited_sha256'] = digest(working)
                    entry['edited_members'] = members(working.read_bytes())
                    entry['outer_unchanged_after_edit'] = inspect_outer(outer) == before
                    page.screenshot(path=str(evidence / f'{case}-edited.png'))
                    if case != 'terminate':
                        if case == 'cancel-discard':
                            page.locator('[data-window=close]').click()
                        else:
                            page.keyboard.press('Control+w')
                        dialog = page.locator('#qa-overlay:not([hidden])')
                        expect(dialog).to_contain_text('更改尚未写回外层', timeout=15000)
                        entry['close_prompt'] = dialog.inner_text()
                        page.screenshot(path=str(evidence / f'{case}-prompt.png'))
                        if case == 'cancel-discard':
                            dialog.locator('[data-action="close"]').last.click()
                            expect(page.locator('[data-entry="after.txt"]')).to_be_visible()
                            entry['cancel_kept_working_copy'] = digest(working) == entry['edited_sha256']
                            page.locator('[data-window=close]').click()
                            expect(dialog).to_be_visible()
                            dialog.locator('[data-action="nested-close-discard"]').click()
                        else:
                            dialog.locator('[data-action="nested-close-save"]').click()
                        if case == 'cancel-discard':
                            app.wait(timeout=60)
                            entry['exit_method'] = 'window close with explicit discard'
                            entry['normal_window_close_exit_code'] = app.returncode
                            with suppress(Exception):
                                browser.close()
                        else:
                            expect(page.locator('[data-entry="inner.zip"]')).to_be_visible(timeout=60000)
                            page.locator('[data-window=close]').click()
                            app.wait(timeout=60)
                            entry['exit_method'] = 'save and close tab, then window close'
                            entry['normal_window_close_exit_code'] = app.returncode
                            with suppress(Exception):
                                browser.close()
                        entry['after_close_working_exists'] = working.exists()
                    else:
                        stop_own(app, browser)
                        entry['exit_method'] = 'TerminateProcess on the GUI process created by this test'
                        entry['forced_gui_exit_code'] = app.returncode
                    lifetime['proc'] = None
                    entry['after_stop_outer'] = inspect_outer(outer)
                    entry['after_stop_neighbour_unchanged'] = digest(root / 'neighbour.txt') == neighbour_hash
                    entry['after_stop_working_copies'] = [{'sha256': digest(path), 'members': members(path.read_bytes())} for path in workcopies(temp)]
                    if not entry['outer_unchanged_after_edit'] or not entry['after_stop_neighbour_unchanged']:
                        raise RuntimeError('Original data changed while editing the private copy')
                    actual_outer = entry['after_stop_outer']
                    if case in ('cancel-discard', 'terminate') and actual_outer != before:
                        raise RuntimeError('Uncommitted nested edit changed the outer archive')
                    if case == 'cancel-discard' and (not entry['cancel_kept_working_copy'] or entry['after_close_working_exists']):
                        raise RuntimeError('Cancel or explicit discard did not preserve its expected result')
                    if case == 'save-close' and (actual_outer['inner'] != entry['edited_members'] or
                            actual_outer['sibling_sha256'] != before['sibling_sha256'] or
                            actual_outer['outer_only_sha256'] != before['outer_only_sha256']):
                        raise RuntimeError('Saved nested archive or unchanged siblings do not match')
                    app, browser, page = launch(exe, outer, root, env, port, driver, lifetime)
                    entry['reopened_gui_pid'] = app.pid
                    page.screenshot(path=str(evidence / f'{case}-reopened.png'))
                    entry['reopened_root_entries'] = page.locator('[data-entry]').evaluate_all('(els)=>els.map(e=>e.getAttribute("data-entry"))')
                    page.locator('[data-entry="inner.zip"]').dblclick()
                    expect(page.locator('[data-entry="unchanged.txt"]')).to_be_visible(timeout=60000)
                    entry['reopened_inner_entries'] = page.locator('[data-entry]').evaluate_all('(els)=>els.map(e=>e.getAttribute("data-entry"))')
                    page.screenshot(path=str(evidence / f'{case}-reopened-inner.png'))
                    expected_name = 'after.txt' if case == 'save-close' else 'before.txt'
                    expected_entries = [expected_name, 'unchanged.txt'] + (['bulk.bin'] if options.large else [])
                    if sorted(entry['reopened_inner_entries']) != sorted(expected_entries):
                        raise RuntimeError('Reopened inner archive entries differ from expected')
                    entry['retained_menu'] = retained_dialog(page, evidence, case)
                    entry['retained_options_first'] = page.locator('#oa-retained-edit option').count()
                    if case == 'terminate' and options.expect_recovery:
                        if entry['retained_menu']['reports_no_retained_copies'] or entry['retained_options_first'] != 1:
                            raise RuntimeError('Expected exactly one recoverable nested edit')
                        recovered_dir = top / 'recovered-output'
                        recovered_dir.mkdir()
                        recovered = recovered_dir / 'recovered-inner.zip'
                        save_started = time.perf_counter()
                        save_retained_copy(page, app, recovered, evidence)
                        entry['recovery_save_elapsed_seconds'] = round(time.perf_counter() - save_started, 3)
                        entry['recovered_sha256'] = digest(recovered)
                        entry['recovered_members'] = members(recovered.read_bytes())
                        entry['recovered_matches_edit'] = entry['recovered_sha256'] == entry['edited_sha256'] and entry['recovered_members'] == entry['edited_members']
                        if not entry['recovered_matches_edit'] or {p.name for p in recovered_dir.iterdir()} != {'recovered-inner.zip'}:
                            raise RuntimeError('Saved recovery differs from edited inner ZIP')
                    page.locator('#qa-overlay:not([hidden]) [data-action="close"]').last.click()
                    page.keyboard.press('Control+w')
                    expect(page.locator('[data-entry="inner.zip"]')).to_be_visible(timeout=60000)
                    page.locator('[data-window=close]').click()
                    app.wait(timeout=60)
                    with suppress(Exception):
                        browser.close()
                    lifetime['proc'] = None
                    if case == 'terminate':
                        app, browser, page = launch(exe, outer, root, env, port, driver, lifetime)
                        entry['second_reopen_gui_pid'] = app.pid
                        entry['second_reopen_menu'] = retained_dialog(page, evidence, 'terminate-second')
                        entry['retained_options_second'] = page.locator('#oa-retained-edit option').count()
                        entry['no_new_copies_on_second_reopen'] = entry['retained_options_second'] <= entry['retained_options_first']
                        if options.expect_recovery:
                            entry['record_removed_after_successful_export'] = entry['retained_options_first'] == 1 and entry['retained_options_second'] == 0
                            if not entry['record_removed_after_successful_export']:
                                raise RuntimeError('Successful export did not remove the recovered record')
                        entry['outer_unchanged_after_second_reopen'] = inspect_outer(outer) == before
                        entry['neighbour_unchanged_after_second_reopen'] = digest(root / 'neighbour.txt') == neighbour_hash
                        if not (entry['no_new_copies_on_second_reopen'] and entry['outer_unchanged_after_second_reopen'] and entry['neighbour_unchanged_after_second_reopen']):
                            raise RuntimeError('Restart changed original data or added retained copies')
                        stop_own(app, browser)
                        lifetime['proc'] = None
                    save()
                encrypted = top / 'encrypted-control'
                encrypted.mkdir()
                temp = encrypted / 'temp'
                temp.mkdir()
                source = encrypted / 'source'
                source.mkdir()
                plaintext_inner = io.BytesIO()
                with zipfile.ZipFile(plaintext_inner, 'w', zipfile.ZIP_STORED) as archive:
                    archive.writestr('before.txt', b'editable nested content\n')
                    archive.writestr('unchanged.txt', b'preserve nested sibling\n')
                (source / 'inner.zip').write_bytes(plaintext_inner.getvalue())
                (source / 'outer-only.txt').write_bytes(b'preserve encrypted outer sibling\n')
                sentinel = encrypted / 'neighbour.txt'
                sentinel.write_bytes(b'preserve ordinary neighbour\n')
                password = 'fixture-zip-password-unique-7264'
                outer = encrypted / 'protected.zip'
                cli = exe.with_name('obz.exe')
                env = dict(os.environ, TEMP=str(temp), TMP=str(temp), LOCALAPPDATA=str(encrypted / 'local'), APPDATA=str(encrypted / 'roaming'), WEBVIEW2_USER_DATA_FOLDER=str(encrypted / 'webview'), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=arguments)
                create = subprocess.run([str(cli), 'create', str(outer), str(source / 'inner.zip'), str(source / 'outer-only.txt'), '--format', 'zip', '--encryption', 'AES256', '--password-stdin'], input=password+'\n', text=True, capture_output=True, timeout=90, env=env)
                if create.returncode:
                    raise RuntimeError('Could not create encrypted synthetic ZIP')
                with zipfile.ZipFile(outer) as archive:
                    if any(not item.flag_bits & 1 for item in archive.infolist()):
                        raise RuntimeError('Synthetic ZIP entries were not encrypted')
                control = report['scenarios']['encrypted-control'] = {'outer_sha256_before': digest(outer), 'neighbour_sha256': digest(sentinel)}
                save()
                app, browser, page = launch(exe, outer, encrypted, env, port, driver, lifetime)
                control['first_gui_pid'] = app.pid
                working = edit(page, temp, password=password)
                control['edited_sha256'] = digest(working)
                control['edited_members'] = members(working.read_bytes())
                page.screenshot(path=str(evidence / 'encrypted-edited.png'))
                stop_own(app, browser)
                control['exit_method'] = 'TerminateProcess on the GUI process created by this test'
                control['forced_gui_exit_code'] = app.returncode
                lifetime['proc'] = None
                app, browser, page = launch(exe, outer, encrypted, env, port, driver, lifetime)
                control['reopened_gui_pid'] = app.pid
                control['retained_menu'] = retained_dialog(page, evidence, 'encrypted')
                control['retained_options'] = page.locator('#oa-retained-edit option').count()
                stop_own(app, browser)
                lifetime['proc'] = None
                recovery_index = encrypted / 'local' / 'qing-zip' / 'EditedFiles' / 'index.json'
                control['recovery_index_exists'] = recovery_index.exists()
                control['recovery_index_contains_fixture_password'] = password.encode() in recovery_index.read_bytes() if recovery_index.exists() else False
                recovered = encrypted / 'decrypt-check'
                recovered.mkdir()
                extract = subprocess.run([str(cli), 'extract', str(outer), str(recovered), '--password-stdin'], input=password+'\n', text=True, capture_output=True, timeout=90, env=env)
                control['outer_decrypts_with_original_password'] = extract.returncode == 0 and (recovered / 'inner.zip').is_file() and (recovered / 'inner.zip').read_bytes() == plaintext_inner.getvalue() and (recovered / 'outer-only.txt').read_bytes() == b'preserve encrypted outer sibling\n'
                control['outer_unchanged'] = digest(outer) == control['outer_sha256_before']
                control['neighbour_unchanged'] = digest(sentinel) == control['neighbour_sha256']
                if control['retained_options'] != 0 or control['recovery_index_contains_fixture_password'] or not control['outer_decrypts_with_original_password'] or not control['outer_unchanged'] or not control['neighbour_unchanged']:
                    raise RuntimeError('Encrypted control failed its isolation or preservation assertions')
                save()
                session['passed'] = True
        report['status'] = 'observed'
    except Exception as error:
        report['status'] = 'failed'
        report['error'] = repr(error)
        raise
    finally:
        save()


if __name__ == '__main__':
    main()
