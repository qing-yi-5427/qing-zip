"""Exercise real encrypted ZIP retry on a private, non-input Windows desktop."""
import argparse, ctypes, hashlib, json, os, subprocess, tempfile, time
from ctypes import wintypes
from pathlib import Path
import win32api, win32con, win32event, win32process
from playwright.sync_api import Error, expect, sync_playwright


def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()


def run(exe, out, fixture, expected):
    source=fixture/'sample.txt'; source.write_bytes(('ordinary synthetic content\n'*4096).encode('utf-8'))
    archive=fixture/'sample-encrypted.zip'; target=fixture/'output';target.mkdir()
    sentinel=target/'unrelated.txt';sentinel.write_text('preserve this synthetic file\n',encoding='utf-8')
    secret='fixture-correct-only'; wrong='fixture-wrong-only'
    cli=exe.parent/'obz.exe'
    result=subprocess.run([str(cli),'create',str(archive),str(source),'--format','zip','--encryption','AES256','--password-stdin'],
                          input=secret+'\n',text=True,capture_output=True,timeout=90,check=True,
                          creationflags=subprocess.CREATE_NO_WINDOW)
    assert archive.is_file(),result.stdout
    initial={'archive':digest(archive),'source':digest(source),'sentinel':digest(sentinel)}
    profile=fixture/'profile';local=fixture/'local';appdata=fixture/'roaming'
    env={**os.environ,'LOCALAPPDATA':str(local),'APPDATA':str(appdata),'TEMP':str(fixture),'TMP':str(fixture),
         'WEBVIEW2_USER_DATA_FOLDER':str(profile),
         'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS':'--remote-debugging-port=0 --remote-debugging-address=127.0.0.1'}
    user32=ctypes.WinDLL('user32',use_last_error=True)
    user32.CreateDesktopW.argtypes=[wintypes.LPCWSTR,wintypes.LPCWSTR,wintypes.LPVOID,wintypes.DWORD,wintypes.DWORD,wintypes.LPVOID]
    user32.CreateDesktopW.restype=wintypes.HANDLE
    user32.CloseDesktop.argtypes=[wintypes.HANDLE];user32.CloseDesktop.restype=wintypes.BOOL
    desktop_name=f'QzPasswordRetry{os.getpid()}'; desktop=user32.CreateDesktopW(desktop_name,None,None,0,0x01ff,None)
    if not desktop: raise ctypes.WinError(ctypes.get_last_error())
    process=None;thread=None
    try:
        startup=win32process.STARTUPINFO();startup.lpDesktop=desktop_name
        process,thread,pid,_=win32process.CreateProcess(str(exe),subprocess.list2cmdline([str(exe),str(archive)]),None,None,False,
                win32con.CREATE_UNICODE_ENVIRONMENT,env,str(fixture),startup)
        deadline=time.monotonic()+45
        while time.monotonic()<deadline:
            if win32event.WaitForSingleObject(process,0)==win32event.WAIT_OBJECT_0: raise RuntimeError('private desktop process exited')
            ports=list(profile.rglob('DevToolsActivePort'))
            if ports: break
            time.sleep(.1)
        else: raise TimeoutError('private desktop WebView2 endpoint unavailable')
        port=int(ports[0].read_text().splitlines()[0])
        with sync_playwright() as p:
            browser=p.chromium.connect_over_cdp(f'http://127.0.0.1:{port}');page=browser.contexts[0].pages[0]
            expect(page.locator('[data-action=extract-menu]')).to_be_visible(timeout=90000)
            page.locator('[data-action=extract-menu]').click();page.locator('[data-action=extract-standard]').click()
            expect(page.locator('#qa-destination')).to_be_visible();page.locator('#qa-destination').fill(str(target))
            page.locator('#qa-smart').uncheck();page.locator('#qa-extract-password').fill(wrong)
            page.locator('[data-action=startextract]').click()
            retry=page.locator('[data-action^=extract-retry-]').first
            expect(retry).to_be_visible(timeout=60000)
            failure=page.locator('#oa-task-panel').inner_text()
            assert secret not in failure and wrong not in failure, 'task feedback exposed a fixture password'
            message='密码不正确，或加密压缩包已损坏。' if expected=='localized' else 'incorrect password or damaged encrypted archive'
            assert message in failure, f'expected task feedback not present: {failure}'
            expect(page.locator('#qa-toast')).to_contain_text(message)
            toast=page.locator('#qa-toast').inner_text()
            page.screenshot(path=out/'wrong-password.png')
            assert digest(archive)==initial['archive'] and digest(source)==initial['source'] and digest(sentinel)==initial['sentinel']
            assert not (target/source.name).exists(), 'wrong password produced a destination file'
            retry_label=retry.inner_text();retry.click()
            field=page.locator('#oa-retry-password');expect(field).to_be_visible();expect(field).to_have_value('')
            destination=page.locator('#oa-retry-dest').input_value()
            preview=page.locator('#oa-retry-output').inner_text()
            assert Path(destination)==target, 'retry destination changed'
            page.screenshot(path=out/'retry-dialog.png')
            field.fill(secret);page.locator('[data-action=extract-retry-submit]').click()
            expect(page.locator('#oa-task-compact-phase')).to_contain_text('已完成',timeout=60000)
            output=target/source.name
            assert output.is_file() and digest(output)==initial['source']
            assert digest(archive)==initial['archive'] and digest(source)==initial['source'] and digest(sentinel)==initial['sentinel']
            if not page.locator('#oa-task-output').is_visible(): page.locator('#oa-task-toggle').click()
            expect(page.locator('#oa-task-output')).to_be_visible()
            page.screenshot(path=out/'completed.png')
            receipt={'desktop':'private non-input Win32 desktop; WebView2 CDP screenshots',
                     'gui_sha256':digest(exe),'cli_sha256':digest(cli),'archive_sha256':initial['archive'],
                     'source_sha256':initial['source'],'sentinel_sha256':initial['sentinel'],
                     'wrong_password_no_output':True,'archive_and_sentinel_preserved':True,
                     'failure_text':failure.replace(str(fixture),'<test-root>'),'toast_text':toast,'retry_label':retry_label,'retry_password_initially_empty':True,
                     'retry_destination_matches_requested':Path(destination)==target,'retry_preview':preview.replace(str(fixture),'<test-root>'),
                     'output_relative_to_requested_destination':output.relative_to(target).as_posix(),
                     'output_sha256':digest(output),'output_matches_source':True,'output_button_visible':True,
                     'not_observed':['actual_open_output_action','system_foreground_desktop']}
            (out/'receipt.json').write_text(json.dumps(receipt,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
            print(json.dumps(receipt,ensure_ascii=True),flush=True)
            try: page.evaluate('window.__TAURI__.core.invoke("window_action",{action:"close"})')
            except Error: pass
    finally:
        if process:
            if win32event.WaitForSingleObject(process,8000)!=win32event.WAIT_OBJECT_0:
                win32process.TerminateProcess(process,1);win32event.WaitForSingleObject(process,10000)
            win32api.CloseHandle(process)
        if thread: win32api.CloseHandle(thread)
        if not user32.CloseDesktop(desktop): raise ctypes.WinError(ctypes.get_last_error())


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--exe',required=True,type=Path)
    parser.add_argument('--out',required=True,type=Path);parser.add_argument('--fixtures-in',required=True,type=Path)
    parser.add_argument('--expect',choices=['baseline','localized'],default='localized')
    args=parser.parse_args()
    running=subprocess.run(['powershell','-NoProfile','-Command',"[Diagnostics.Process]::GetProcessesByName('qing-zip').Id; [Diagnostics.Process]::GetProcessesByName('obz-gui').Id"],capture_output=True,text=True,check=True)
    if running.stdout.strip(): raise RuntimeError('Close other qing-zip instances before running')
    args.out.mkdir(parents=True,exist_ok=False);args.fixtures_in.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='qz-password-',dir=args.fixtures_in,ignore_cleanup_errors=True) as tmp:
        run(args.exe.resolve(strict=True),args.out.resolve(),Path(tmp).resolve(),args.expect)
