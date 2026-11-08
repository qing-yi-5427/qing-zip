"""Exercise the registered classic Explorer action only in a disposable hosted account."""
import argparse
import ctypes
import json
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
    finally:
        for path in reversed(created):
            winreg.DeleteKeyEx(winreg.HKEY_CURRENT_USER,path,VIEW)
        notify_shell()


if __name__ == '__main__':
    main()
