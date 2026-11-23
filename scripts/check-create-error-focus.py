"""Real private-desktop audit of creation validation at the system DPI."""
import argparse
import hashlib
import importlib.util
import json
import shutil
import tempfile
import time
from pathlib import Path

import win32event
import win32process
from playwright.sync_api import expect

spec = importlib.util.spec_from_file_location('private_qa', Path(__file__).with_name('check-nested-multi-layer.py'))
qa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qa)
parser = argparse.ArgumentParser()
parser.add_argument('--exe', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--expect-visible', action='store_true')
parser.add_argument('--fixtures-in', type=Path, help='Existing directory for the temporary fixture (defaults to the system temp directory)')
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=False)
fixture = Path(tempfile.mkdtemp(prefix='qz-r8-', dir=args.fixtures_in))
report = {'gui_sha256': hashlib.sha256(args.exe.read_bytes()).hexdigest(), 'cases': {}, 'status': 'FAIL'}
try:
    source = fixture / 'sample.txt'
    source.write_text('ordinary test content', encoding='utf-8')
    with qa.private_process(args.exe.resolve(), fixture, None) as (page, process, pid):
        page.wait_for_selector('[data-action=compress]:visible')
        cdp = page.context.new_cdp_session(page)
        window = cdp.send('Browser.getWindowForTarget')
        cdp.send('Browser.setWindowBounds', {'windowId': window['windowId'], 'bounds': {'width': 780, 'height': 750}})
        for theme in ('light', 'dark'):
            page.locator(f'[data-theme={theme}]').click()
            page.locator('[data-action=compress]:visible').last.click()
            expect(page.locator('#qa-inputs')).to_be_attached()
            page.screenshot(path=str(args.out / f'{theme}-default.png'))
            page.locator('details').filter(has=page.locator('#qa-inputs')).locator('summary').click()
            page.locator('#qa-inputs').fill(str(source))
            page.locator('#qa-save').fill(str(fixture / 'result.zip'))
            page.locator('[data-action=compression-advanced]').click()
            timings = []
            for _ in range(5):
                page.locator('#qa-threads').fill('999')
                start = time.perf_counter()
                page.locator('[data-action=startcompress]').click()
                expect(page.locator('#qa-threads')).to_have_attribute('aria-invalid', 'true')
                expect(page.locator('#qa-formerror')).not_to_be_empty()
                timings.append(round((time.perf_counter() - start) * 1000, 2))
            geometry = page.evaluate('''() => {
              const input=document.querySelector('#qa-threads'), body=input.closest('.oa-dialog-body');
              const i=input.getBoundingClientRect(), b=body.getBoundingClientRect();
              return {dpr:devicePixelRatio, viewport:[innerWidth,innerHeight], input:[i.top,i.bottom],
                      body:[b.top,b.bottom], active:document.activeElement.id,
                      error:document.querySelector('#qa-formerror').textContent};
            }''')
            geometry['visible'] = geometry['input'][0] >= geometry['body'][0] and geometry['input'][1] <= geometry['body'][1]
            geometry['submit_to_error_ms'] = timings
            assert geometry['dpr'] == 1.5 and geometry['active'] == 'qa-threads' and geometry['error']
            page.screenshot(path=str(args.out / f'{theme}-invalid.png'))
            if args.expect_visible:
                assert geometry['visible'], geometry
            page.locator('#qa-threads').fill('2')
            expect(page.locator('#qa-formerror')).to_be_empty()
            page.keyboard.press('Tab')
            assert page.evaluate("document.activeElement.id") != 'qa-threads'
            report['cases'][theme] = geometry
            page.locator('.qa-dialog-header [data-action=close]').click()
        page.locator('[data-window=close]').click()
        if win32event.WaitForSingleObject(process, 20000) != win32event.WAIT_OBJECT_0:
            raise RuntimeError('GUI did not exit normally')
        report['gui_exit_code'] = win32process.GetExitCodeProcess(process)
        if report['gui_exit_code'] != 0:
            raise RuntimeError(f"GUI exited with code {report['gui_exit_code']}")
        report['status'] = 'PASS'
finally:
    for attempt in range(30):
        try:
            shutil.rmtree(fixture)
            break
        except OSError:
            time.sleep(.5)
    report['fixture_cleanup_verified'] = not fixture.exists()
    (args.out / 'receipt.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    if not report['fixture_cleanup_verified']:
        raise RuntimeError('Temporary profile cleanup failed')
