"""Exercise the registered classic Explorer action only in a disposable hosted account."""
from contextlib import contextmanager
import argparse
import ctypes
import json
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import winreg

CLSID = '{B1031E9C-F2AA-4658-8932-E2596415A942}'
CLASS = 'CLSID\\' + CLSID
VERB = r'*\shell\qing-zip-compress-files'
CLASSES = r'Software\Classes'
VIEW = winreg.KEY_WOW64_64KEY


def notify_shell():
    ctypes.windll.shell32.SHChangeNotify(0x08000000, 0, None, None)


@contextmanager
def registration(exe):
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise RuntimeError('Disposable hosted account required for shell registration')
    for root in (CLASS, VERB):
        try:
            with winreg.OpenKey(winreg.HKEY_CLASSES_ROOT, root, 0, winreg.KEY_READ | VIEW):
                raise RuntimeError('Existing shell registration refused: ' + root)
        except FileNotFoundError:
            pass

    # Match the shipping registration. The real COM server independently checks
    # ownership, LocalServer32 and DelegateExecute before launching the GUI.
    values = {
        CLASS: {'QingZipExecutable': str(exe)},
        CLASS + r'\LocalServer32': {'': f'"{exe}" --shell-compress-server'},
        VERB: {'QingZipExecutable': str(exe),'MUIVerb': '用 qing-zip 压缩…',
               'Icon': f'"{exe}",0','MultiSelectModel': 'Player'},
        VERB + r'\command': {'DelegateExecute': CLSID},
    }
    created = []
    try:
        for key, data in values.items():
            path = CLASSES + '\\' + key
            with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER,path,0,winreg.KEY_WRITE | VIEW) as h:
                created.append(path)
                for name, text in data.items():
                    winreg.SetValueEx(h,name,0,winreg.REG_SZ,text)
        notify_shell()
        yield values
    finally:
        for path in reversed(created):
            try:
                winreg.DeleteKeyEx(winreg.HKEY_CURRENT_USER,path,VIEW)
            except FileNotFoundError:
                pass
        notify_shell()

def verify_registration(values, present):
    for key, names in values.items():
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER,CLASSES+'\\'+key,0,winreg.KEY_READ|VIEW) as handle:
                assert present, f"Uninstall left {key}"
                for name, expected in names.items():
                    assert winreg.QueryValueEx(handle,name)==(expected,winreg.REG_SZ), (key,name)
        except FileNotFoundError:
            if present:
                raise

def real_dialog(exe, probe, dist):
    # All files, the browser profile and the debugging endpoint belong to this
    # hosted run. Refuse existing registrations before the actual settings click.
    for root in (CLASS, VERB):
        try:
            with winreg.OpenKey(winreg.HKEY_CLASSES_ROOT,root,0,winreg.KEY_READ|VIEW):
                raise RuntimeError('Existing shell registration refused: '+root)
        except FileNotFoundError:
            pass
    with tempfile.TemporaryDirectory(dir=os.environ['RUNNER_TEMP'],prefix='qing-shell-dialog-',ignore_cleanup_errors=True) as temp:
        scratch=Path(temp)
        source=scratch/'inputs'
        source.mkdir()
        first=source/'first 甲 & 25%.txt'
        second=source/'second [2].bin'
        first.write_text('created through the real shell and workbench',encoding='utf8')
        second.write_bytes(bytes(range(256))*37)
        env=dict(os.environ,WEBVIEW2_USER_DATA_FOLDER=str(scratch/'webview'))
        screenshot=dist.resolve()/'shell-create-dialog.png'
        app=subprocess.Popen([exe],env=env,cwd=scratch)
        try:
            driver=Path(__file__).with_name('check-real-shell-dialog.ps1')
            command=['pwsh','-NoProfile','-File',str(driver),str(app.pid),str(exe),str(probe),str(first),str(second),str(screenshot)]
            response=subprocess.run(command,capture_output=True,timeout=200)
            if response.returncode:raise RuntimeError(response.stderr.decode('utf8','replace')[-3500:])
            result=json.loads(response.stdout)
            assert result['primary_pid']==app.pid and result['dialog_count']==1 and result['created_with_integrity_check']
            archive=Path(result['output']).resolve(strict=True)
            assert archive.samefile(source/'inputs.zip') and archive.read_bytes().startswith(b'PK')
            console=list((exe.parent/'runtime').glob('nanazip*/NanaZip.Universal.Console.exe'))
            assert len(console)==1
            unpacked=scratch/'nana-output'
            subprocess.run([console[0],'x','-y','-o'+str(unpacked),archive],check=True,capture_output=True,timeout=40)
            actual={f.relative_to(unpacked).as_posix():hashlib.sha256(f.read_bytes()).hexdigest() for f in unpacked.rglob('*') if f.is_file()}
            expected={f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in [first,second]}
            assert actual==expected, (actual,expected)
            result.update(source_commit=os.environ['GITHUB_SHA'],nana_sha256=hashlib.sha256(console[0].read_bytes()).hexdigest(),
                          extracted_files=actual,screenshot_sha256=hashlib.sha256(screenshot.read_bytes()).hexdigest())
            (dist/'shell-create-dialog.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf8')
            print(json.dumps(result),flush=True)
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait(timeout=20)
            # The GUI disables the setting after successful submission. Its
            # normal uninstall command safely retires our own verb after failures.
            subprocess.run([exe,'--unregister-compress-menu'],timeout=20,check=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('portable', type=Path)
    parser.add_argument('probe', type=Path)
    args = parser.parse_args()
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        raise SystemExit('This test requires a disposable GitHub-hosted Windows runner')
    apps = list(args.portable.glob('*-portable/qing-zip.exe'))
    if len(apps) != 1:
        raise ValueError(f'Expected one freshly built portable application under {args.portable}')
    exe = apps[0].resolve(strict=True)
    probe = args.probe.resolve(strict=True)

    with registration(exe):
        with tempfile.TemporaryDirectory(prefix='qing-shell-two-') as temp:
            source = Path(temp)
            first = source/'first 甲 & 25%.txt'
            second = source/'second [2].bin'
            first.write_text('first',encoding='utf8')
            second.write_bytes(b'second')
            proc = subprocess.run([probe,exe,first,second],capture_output=True,timeout=75)
            if proc.returncode:
                raise RuntimeError(f'Shell integration failed ({proc.returncode}): {proc.stderr.decode(errors="replace")}')
            result = json.loads(proc.stdout)
            assert result['selection_items'] == result['matched_paths'] == 2
            assert result['advertised_verb'] and result['groups_received'] == 1
            result['source_commit'] = os.environ['GITHUB_SHA']
            result['receiver'] = 'test window, matching actual Tauri single-instance IPC'
            print(json.dumps(result),flush=True)
    real_dialog(exe,probe,args.portable)


if __name__ == '__main__':
    main()
