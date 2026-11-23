"""Capture real Tauri task feedback while extracting an ordinary synthetic tar.gz.

Requires Python Playwright and complete portable distributions. No system settings
are changed; each run uses an isolated LOCALAPPDATA and WebView2 profile.
"""
import argparse, hashlib, json, os, random, subprocess, tarfile, tempfile, time
from pathlib import Path
from playwright.sync_api import Error, expect, sync_playwright


def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while chunk:=f.read(1024*1024): h.update(chunk)
    return h.hexdigest()


def run(exe, archive, fixture, out, baseline):
    profile=fixture/'profile'; destination=fixture/'output'
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
            expect(page.locator('[data-action=extract-menu]')).to_be_visible(timeout=90000)
            page.locator('[data-action=extract-menu]').click()
            page.locator('[data-action=extract-standard]').click()
            expect(page.locator('#qa-destination')).to_be_visible()
            page.locator('#qa-destination').fill(str(destination))
            page.locator('#qa-smart').uncheck()
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
            (out/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
            print(json.dumps(receipt),flush=True)
            try: page.evaluate('window.__TAURI__.core.invoke("window_action", {action:"close"})')
            except Error: pass
    finally:
        try: proc.wait(timeout=8)
        except subprocess.TimeoutExpired: proc.terminate(); proc.wait(timeout=10)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--baseline',required=True,type=Path);p.add_argument('--candidate',required=True,type=Path)
    p.add_argument('--out',required=True,type=Path);p.add_argument('--fixtures-in',required=True,type=Path)
    args=p.parse_args()
    running=subprocess.run(['powershell','-NoProfile','-Command','Get-Process qing-zip,obz-gui -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id'],capture_output=True,text=True,check=True)
    if running.stdout.strip(): raise RuntimeError('Close other qing-zip instances before running this test')
    args.out.mkdir(parents=True,exist_ok=False); args.fixtures_in.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='qz-task-gui-',dir=args.fixtures_in,ignore_cleanup_errors=True) as tmp:
        root=Path(tmp).resolve(); source=root/'sample.bin'; block=random.Random(42).randbytes(1024*1024)
        with source.open('wb') as f:
            for _ in range(256): f.write(block)
        archive=root/'sample.tar.gz'
        with tarfile.open(archive,'w:gz',compresslevel=1,format=tarfile.USTAR_FORMAT) as tar: tar.add(source,arcname=source.name)
        for name,exe,baseline in [('before',args.baseline,True),('after',args.candidate,False)]:
            fixture=root/name; fixture.mkdir(); out=args.out/name; out.mkdir()
            run(exe.resolve(strict=True),archive,fixture,out,baseline)
