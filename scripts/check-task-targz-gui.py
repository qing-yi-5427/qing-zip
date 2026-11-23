"""Capture real Tauri task feedback while extracting an ordinary synthetic tar.gz.

Requires Python Playwright and complete portable distributions. No system settings
are changed; each run uses an isolated LOCALAPPDATA and WebView2 profile.
"""
import argparse, hashlib, json, os, random, subprocess, tarfile, tempfile, time
from contextlib import contextmanager
from pathlib import Path
from playwright.sync_api import Error, expect, sync_playwright


def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while chunk:=f.read(1024*1024): h.update(chunk)
    return h.hexdigest()


@contextmanager
def live_page(exe, archive, fixture):
    profile=fixture/'profile'
    env={**os.environ,'LOCALAPPDATA':str(fixture/'local'),'TEMP':str(fixture),'TMP':str(fixture),
         'WEBVIEW2_USER_DATA_FOLDER':str(profile),
         'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS':'--remote-debugging-port=0 --remote-debugging-address=127.0.0.1'}
    proc=subprocess.Popen([str(exe),str(archive)],cwd=fixture,env=env)
    try:
        deadline=time.monotonic()+45
        while time.monotonic()<deadline:
            if proc.poll() is not None: raise RuntimeError('GUI exited; close other qing-zip instances first')
            ports=list(profile.rglob('DevToolsActivePort'))
            if ports: break
            time.sleep(.1)
        else: raise TimeoutError('WebView2 debugging port unavailable')
        port=int(ports[0].read_text().splitlines()[0])
        with sync_playwright() as p:
            browser=p.chromium.connect_over_cdp(f'http://127.0.0.1:{port}')
            page=browser.contexts[0].pages[0]
            try:
                expect(page.locator('[data-action=extract-menu]')).to_be_visible(timeout=90000)
                yield page
            finally:
                try: page.evaluate('window.__TAURI__.core.invoke("window_action", {action:"close"})')
                except Error: pass
    finally:
        try: proc.wait(timeout=8)
        except subprocess.TimeoutExpired: proc.terminate(); proc.wait(timeout=10)


def extraction_dialog(page, destination):
    page.locator('[data-action=extract-menu]').click()
    page.locator('[data-action=extract-standard]').click()
    expect(page.locator('#qa-destination')).to_be_visible()
    page.locator('#qa-destination').fill(str(destination))
    page.locator('#qa-smart').uncheck()
    expect(page.locator('#oa-recycle-after')).not_to_be_checked()


def open_output_in_explorer(page, destination):
    from pywintypes import com_error
    from win32com.client import Dispatch
    from win32gui import EnumWindows, GetClassName, GetWindowText, IsWindowVisible

    def windows():
        handles = set()

        def collect(handle, _):
            if IsWindowVisible(handle) and GetClassName(handle) == 'CabinetWClass':
                handles.add(handle)
            return True

        EnumWindows(collect, None)
        return handles

    existing = windows()
    page.locator('#oa-task-output').click()
    shell = Dispatch('Shell.Application')
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        opened = {handle for handle in windows() - existing if destination.name in GetWindowText(handle)}
        if opened:
            for window in shell.Windows():
                if window.HWND not in opened:
                    continue
                try:
                    actual = Path(window.Document.Folder.Self.Path)
                except com_error:
                    continue
                if actual != destination:
                    continue
                window.Quit()
                return {'opened_matching_directory': True, 'output_file': 'sample.bin'}
        time.sleep(.15)
    raise AssertionError('No new Explorer window opened the requested test directory.')


def run(exe, archive, fixture, out, baseline, *, open_output=False):
    directory = 'qz-task-output-' + fixture.parent.name if open_output else 'output'
    destination = fixture / directory
    with live_page(exe, archive, fixture) as page:
        extraction_dialog(page, destination)
        start=time.monotonic(); page.locator('[data-action=startextract]').click()
        expected='正在处理' if baseline else '解压压缩流'
        found=None
        deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            found=page.evaluate('''async ()=>{let out=document.querySelector('#oa-task-output');let id=Number(out?.dataset.jobOutput);if(!id)return null;try{let v=await window.__TAURI__.core.invoke('operation_status',{id});return {id,phase:v.progress.phase,status:v.status,compact:document.querySelector('#oa-task-compact-phase')?.textContent,expanded:document.querySelector('#oa-task-phase')?.textContent}}catch{return null}}''')
            if found and found['phase']=='decompressing' and found['status']=='running' and expected in (found['compact']+' '+found['expanded']):
                page.screenshot(path=out/'decompressing.png')
                after=page.evaluate('''async id=>{try{let v=await window.__TAURI__.core.invoke('operation_status',{id});return {phase:v.progress.phase,status:v.status,compact:document.querySelector('#oa-task-compact-phase')?.textContent,expanded:document.querySelector('#oa-task-phase')?.textContent}}catch{return null}}''',found['id'])
                if after and after['status']=='running' and after['phase']=='decompressing' and expected in (after['compact']+' '+after['expanded']):
                    capture_seconds=time.monotonic()-start
                    break
            time.sleep(.025)
        else: raise AssertionError(f'decompressing phase was not observed with matching UI: {found}')
        expect(page.locator('#oa-task-compact-phase')).to_contain_text('已完成',timeout=120000)
        output=destination/'sample.bin'
        assert output.exists() and sha(output)==sha(fixture.parent/'sample.bin')
        if not page.locator('#oa-task-output').is_visible(): page.locator('#oa-task-toggle').click()
        expect(page.locator('#oa-task-output')).to_be_visible(timeout=5000)
        page.screenshot(path=out/'complete.png')
        receipt={'mode':'baseline' if baseline else 'candidate','gui_sha256':sha(exe),'archive_sha256':sha(archive),
                 'observed_during':found,'observed_after_screenshot':after,'elapsed_to_capture_seconds':round(capture_seconds,3),
                 'output_sha256':sha(output),'completed':True,'output_action_visible':True,
                 'not_observed':['physical_cancel','actual_open_output_action','unknown_file_total_with_byte_progress']}
        if open_output:
            receipt['explorer'] = open_output_in_explorer(page, destination)
            receipt['not_observed'].remove('actual_open_output_action')
        (out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8',newline='\n')
        print(json.dumps(receipt),flush=True)


def run_cancel(exe, archive, fixture, out):
    destination = fixture / 'output'
    destination.mkdir()
    existing = destination / 'sample.bin'
    existing.write_bytes(b'pre-existing destination data')
    untouched = destination / 'keep.txt'
    untouched.write_bytes(b'unrelated output')
    archive_hash = sha(archive)
    previous = {p.name: sha(p) for p in destination.iterdir()}
    with live_page(exe, archive, fixture) as page:
        extraction_dialog(page, destination)
        page.locator('#qa-conflict').select_option('overwrite')
        page.evaluate('''() => {
            window.cancelTrace = [];
            const panel = document.querySelector('#oa-task-panel');
            const observer = new MutationObserver(() => {
                const phase = panel.querySelector('#oa-task-phase').textContent;
                const metric = panel.querySelector('#oa-task-metric').textContent;
                const value = panel.querySelector('#oa-task-progress').getAttribute('aria-valuenow');
                if (phase !== '正在取消' && phase !== '已取消') return;
                const snapshot = {phase, metric, value, estimate: /本阶段约/.test(panel.innerText)};
                const last = window.cancelTrace.at(-1);
                if (!last || JSON.stringify(last) !== JSON.stringify(snapshot)) window.cancelTrace.push(snapshot);
            });
            observer.observe(panel, {attributes:true, characterData:true, childList:true, subtree:true});
        }''')
        page.locator('[data-action=startextract]').click()
        expect(page.locator('#oa-task-phase')).to_contain_text('解压压缩流', timeout=15000)
        cancel = page.locator('#oa-task-cancel')
        expect(cancel).to_be_visible(timeout=3000)
        running = page.evaluate('''async () => {
            const panel = document.querySelector('#oa-task-panel');
            const id = Number(document.querySelector('#oa-task-output').dataset.jobOutput);
            const job = await window.__TAURI__.core.invoke('operation_status', {id});
            return {status:job.status, phase:job.progress.phase, native_percent:job.progress.percent ?? null,
                metric:panel.querySelector('#oa-task-metric').textContent,
                value:panel.querySelector('#oa-task-progress').getAttribute('aria-valuenow'),
                estimate:/本阶段约/.test(panel.innerText)};
        }''')
        assert running['status'] == 'running' and running['phase'] == 'decompressing', running
        if running['native_percent'] is None and running['value'] is None:
            assert running['metric'] == '进度待定' and not running['estimate'], running
        cancel.click(timeout=2000)
        expect(page.locator('#oa-task-compact-phase')).to_contain_text('已取消', timeout=30000)
        trace = page.evaluate('window.cancelTrace')
        assert {item['phase'] for item in trace} >= {'正在取消', '已取消'}, trace
        assert all(item['value'] is None and not item['estimate'] for item in trace), trace
        expect(page.locator('#oa-task-output')).to_be_hidden()
        page.locator('[data-view=history]').click()
        expect(page.locator('.qa-badge[data-state=cancelled]')).to_be_visible()
        page.screenshot(path=out/'cancelled.png')
    final = {p.name: sha(p) for p in destination.iterdir()}
    assert final == previous and sha(archive) == archive_hash
    receipt = {'candidate_gui_sha256': sha(exe), 'archive_sha256': archive_hash,
               'running': running, 'cancel_transitions': trace, 'terminal': 'cancelled',
               'output_action_hidden': True, 'input_preserved': True,
               'destination_before': previous, 'destination_after': final}
    (out/'receipt.json').write_text(json.dumps(receipt, ensure_ascii=False, indent=2)+'\n', encoding='utf-8', newline='\n')
    print(json.dumps(receipt), flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--baseline',type=Path);p.add_argument('--candidate',required=True,type=Path)
    p.add_argument('--out',required=True,type=Path);p.add_argument('--fixtures-in',required=True,type=Path)
    p.add_argument('--cancel',action='store_true',help='Click Cancel in a running candidate extraction.')
    p.add_argument('--open-output',action='store_true',help='Verify the candidate output action opens the actual test directory in Explorer.')
    p.add_argument('--size-mib',type=int,metavar='64..2048')
    args=p.parse_args()
    if not args.cancel and not args.baseline: p.error('--baseline is required for comparison mode')
    if args.size_mib is not None and not 64 <= args.size_mib <= 2048:
        p.error('--size-mib must be between 64 and 2048')
    running=subprocess.run(['powershell','-NoProfile','-Command',
        "'qing-zip','obz-gui' | ForEach-Object { [Diagnostics.Process]::GetProcessesByName($_).Id }"],
        capture_output=True,text=True,check=True)
    if running.stdout.strip(): raise RuntimeError('Close other qing-zip instances before running this test')
    args.out.mkdir(parents=True,exist_ok=False); args.fixtures_in.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='qz-task-gui-',dir=args.fixtures_in,ignore_cleanup_errors=True) as tmp:
        root=Path(tmp).resolve(); source=root/'sample.bin'; block=random.Random(42).randbytes(1024*1024)
        with source.open('wb') as f:
            for _ in range(args.size_mib or (1024 if args.cancel else 256)): f.write(block)
        archive=root/'sample.tar.gz'
        with tarfile.open(archive,'w:gz',compresslevel=1,format=tarfile.USTAR_FORMAT) as tar: tar.add(source,arcname=source.name)
        if args.cancel:
            fixture=root/'cancel'; fixture.mkdir()
            run_cancel(args.candidate.resolve(strict=True), archive, fixture, args.out)
            if args.open_output:
                fixture=root/'complete'; fixture.mkdir()
                out=args.out/'complete'; out.mkdir()
                run(args.candidate.resolve(strict=True), archive, fixture, out, False, open_output=True)
        else:
            for name,exe,baseline in [('before',args.baseline,True),('after',args.candidate,False)]:
                fixture=root/name; fixture.mkdir(); out=args.out/name; out.mkdir()
                run(exe.resolve(strict=True),archive,fixture,out,baseline,open_output=args.open_output and not baseline)
