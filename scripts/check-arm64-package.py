"""Build the ARM64 trial installer; run both distributions on the native CI host."""
import argparse
import ctypes
import hashlib
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import winreg
import zipfile

from arm64_package_runtime import ROOT, NSIS, fetch, machine, sha, verify_layout, verify_private_imports

def run(args, *, data=None, check=True, cwd=None, timeout=120):
    p=subprocess.run(list(map(str,args)),input=data,capture_output=True,cwd=cwd,timeout=timeout)
    if check and p.returncode:
        raise RuntimeError(f'{Path(args[0]).name} failed ({p.returncode}): {p.stderr[-1300:]!r} {p.stdout[-500:]!r}')
    return p

def files(root):
    return {p.relative_to(root).as_posix():sha(p) for p in sorted(root.rglob('*')) if p.is_file()}

def key(root,name):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,root,0,winreg.KEY_READ|winreg.KEY_WOW64_64KEY) as k:
            return winreg.QueryValueEx(k,name)[0]
    except FileNotFoundError:return None

def seed_runner_associations(application):
    """Model candidates left by the installed version on a disposable hosted runner."""
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        return None
    prog = r'Software\Classes\OpenArchive.Archive'
    caps = r'Software\OpenArchive\Capabilities'
    openwith = r'Software\Classes\.zip\OpenWithProgids'
    owner = r'Software\OpenArchive\Associations'
    registered = r'Software\RegisteredApplications'
    entries = [
        (owner, 'ManagedBy', winreg.REG_SZ, 'OpenArchive.Associations.v1'),
        (prog, '', winreg.REG_SZ, 'qing-zip 压缩文件'),
        (prog+r'\shell\open\command', '', winreg.REG_SZ, f'"{application}" "%1"'),
        (prog+r'\DefaultIcon', '', winreg.REG_SZ, f'"{application}",0'),
        (openwith, 'OpenArchive.Archive', winreg.REG_NONE, b''),
        (caps+r'\FileAssociations', '.zip', winreg.REG_SZ, 'OpenArchive.Archive'),
        (caps, 'ApplicationName', winreg.REG_SZ, 'qing-zip'),
        (caps, 'ApplicationDescription', winreg.REG_SZ, '本地压缩与解压工具'),
        (caps, 'ApplicationIcon', winreg.REG_SZ, f'"{application}",0'),
        (registered, 'qing-zip', winreg.REG_SZ, caps),
    ]
    external = (openwith, 'QingZipTest.OtherArchive', winreg.REG_SZ, 'preserve')
    for path, name, _, _ in [*entries, external]:
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER,path) as h:
                winreg.QueryValueEx(h,name)
        except FileNotFoundError:
            continue
        raise ValueError(f'Runner already contains {path}:{name}')
    for path, name, kind, value in [*entries, external]:
        with winreg.CreateKeyEx(winreg.HKEY_CURRENT_USER,path,0,winreg.KEY_SET_VALUE | winreg.KEY_WOW64_64KEY) as h:
            winreg.SetValueEx(h,name,0,kind,value)
    return entries, external

def check_runner_associations(fixture):
    entries, (path, name, _, value) = fixture
    for candidate, setting, _, _ in entries:
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER,candidate) as h:
                winreg.QueryValueEx(h,setting)
        except FileNotFoundError:
            continue
        raise ValueError(f'Uninstall left an advertised association at {candidate}:{setting}')
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,path,0,winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE) as h:
        assert winreg.QueryValueEx(h,name)[0] == value
        winreg.DeleteValue(h,name)
    return {'owned_values_removed':len(entries),'other_candidate_preserved':True}

def shortcut_target(link):
    env=dict(os.environ,QING_CI_SHORTCUT=str(link))
    command="[Console]::OutputEncoding=[Text.Encoding]::UTF8; (New-Object -ComObject WScript.Shell).CreateShortcut($env:QING_CI_SHORTCUT).TargetPath"
    return Path(subprocess.check_output(['powershell','-NoProfile','-Command',command],env=env).decode('utf8').strip())

def verify_foreign_shortcuts(setup, installed, shortcuts, portable, arp):
    # Only the disposable native runner may create test shortcuts on its Desktop.
    if os.environ.get('GITHUB_ACTIONS') != 'true' or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted':
        return None
    desktop = Path(run(['powershell','-NoProfile','-Command',
                        "[Console]::OutputEncoding=[Text.Encoding]::UTF8;[Environment]::GetFolderPath('DesktopDirectory')"]).stdout.decode('utf8').strip())
    desktop_link = desktop/'qing-zip-arm64.lnk'
    start = shortcuts/'qing-zip.lnk'
    uninstall_link = shortcuts/'卸载.lnk'
    assert desktop.is_dir() and not shortcuts.exists() and not desktop_link.exists()
    shortcuts.mkdir()
    def create_link(path, arguments):
        env=dict(os.environ,QING_CI_SHORTCUT=str(path),QING_CI_TARGET=str(portable),QING_CI_ARGUMENTS=arguments)
        command="$link=(New-Object -ComObject WScript.Shell).CreateShortcut($env:QING_CI_SHORTCUT);$link.TargetPath=$env:QING_CI_TARGET;$link.Arguments=$env:QING_CI_ARGUMENTS;$link.Save()"
        subprocess.run(['powershell','-NoProfile','-Command',command],check=True,env=env)
        return sha(path)
    before = {start:create_link(start,'--user-start'), desktop_link:create_link(desktop_link,'--user-desktop')}
    run([setup,'/S'])
    assert all(sha(path)==digest for path,digest in before.items()) and uninstall_link.is_file()
    before[uninstall_link]=create_link(uninstall_link,'--portable-takeover')
    run([installed/'Uninstall.exe','/S'])
    until=time.monotonic()+90
    while time.monotonic()<until and key(arp,'InstallLocation') is not None:time.sleep(.2)
    assert key(arp,'InstallLocation') is None
    for path,digest in before.items():
        assert sha(path)==digest, f'Uninstall altered user shortcut {path.name}'
        path.unlink()
    shortcuts.rmdir()
    return {'preexisting_start_preserved':True,'unselected_desktop_preserved':True,'reassigned_uninstall_preserved':True}

def smoke_desktop(application, archive, scratch):
    """Observe the trial app starting its own WebView2 process. No UI injection."""
    from ctypes import wintypes as w
    class Process(ctypes.Structure):
        _fields_=[('size',w.DWORD),('usage',w.DWORD),('pid',w.DWORD),('heap',ctypes.c_size_t),
                  ('module',w.DWORD),('threads',w.DWORD),('parent',w.DWORD),('priority',w.LONG),
                  ('flags',w.DWORD),('name',w.WCHAR*260)]
    k=ctypes.WinDLL('kernel32',use_last_error=True)
    k.CreateToolhelp32Snapshot.argtypes=(w.DWORD,w.DWORD);k.CreateToolhelp32Snapshot.restype=w.HANDLE
    for method in (k.Process32FirstW,k.Process32NextW):method.argtypes=(w.HANDLE,ctypes.POINTER(Process));method.restype=w.BOOL
    k.OpenProcess.argtypes=(w.DWORD,w.BOOL,w.DWORD);k.OpenProcess.restype=w.HANDLE
    k.CloseHandle.argtypes=(w.HANDLE,);k.CloseHandle.restype=w.BOOL
    k.IsWow64Process2.argtypes=(w.HANDLE,ctypes.POINTER(w.WORD),ctypes.POINTER(w.WORD));k.IsWow64Process2.restype=w.BOOL
    env=dict(os.environ,WEBVIEW2_USER_DATA_FOLDER=str(scratch/'webview-profile'))
    startup=subprocess.STARTUPINFO();startup.dwFlags|=subprocess.STARTF_USESHOWWINDOW;startup.wShowWindow=0
    proc=subprocess.Popen([str(application),str(archive)],cwd=scratch,env=env,startupinfo=startup,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        process_machine,native_machine=w.WORD(),w.WORD()
        if not k.IsWow64Process2(w.HANDLE(proc._handle),ctypes.byref(process_machine),ctypes.byref(native_machine)) or process_machine.value!=0 or native_machine.value!=0xaa64:
            raise ValueError('desktop app did not run as native ARM64')
        deadline=time.monotonic()+45
        while time.monotonic()<deadline:
            if proc.poll() is not None: raise RuntimeError(f'desktop exited before WebView2 initialization ({proc.returncode})')
            snapshot=k.CreateToolhelp32Snapshot(2,0)
            if snapshot==ctypes.c_void_p(-1).value:raise ctypes.WinError(ctypes.get_last_error())
            processes={}
            try:
                entry=Process();entry.size=ctypes.sizeof(entry)
                more=k.Process32FirstW(snapshot,ctypes.byref(entry))
                while more:
                    processes[entry.pid]=(entry.parent,entry.name.lower())
                    more=k.Process32NextW(snapshot,ctypes.byref(entry))
            finally:k.CloseHandle(snapshot)
            for pid,(parent,name) in processes.items():
                if name!='msedgewebview2.exe':continue
                ancestors=set()
                while parent and parent not in ancestors:
                    if parent==proc.pid:
                        child=k.OpenProcess(0x1000,False,pid)
                        if not child:break
                        try:
                            child_machine,native=w.WORD(),w.WORD()
                            if k.IsWow64Process2(child,ctypes.byref(child_machine),ctypes.byref(native)):
                                return {'native_process':True,'webview2_child':True,'system_webview_machine':hex(child_machine.value or native.value),'scope':'process startup, no UI interaction'}
                        finally:k.CloseHandle(child)
                        break
                    ancestors.add(parent);parent=processes.get(parent,(0,''))[0]
            time.sleep(.25)
        raise RuntimeError('desktop process did not start a WebView2 child within 45s')
    finally:
        if proc.poll() is None:proc.terminate()
        proc.wait(timeout=15)

def exercise(app, scratch):
    scratch.mkdir()
    verify_layout(app/'runtime')
    if any(machine(app/n)!=0xaa64 for n in ['obz.exe','qing-zip.exe']):raise ValueError('wrong app architecture')
    dependencies={name:verify_private_imports(app/name) for name in ['obz.exe','qing-zip.exe']}
    cli=app/'obz.exe'
    cap=json.loads(run([cli,'capabilities']).stdout)['engine']
    assert Path(cap['engine']).resolve()==(app/'runtime/nanazip-arm64/NanaZip.Universal.Console.exe').resolve()
    py=app/'runtime/python/python.exe'
    code="""import ctypes,sys,pathlib,importlib,ssl,sqlite3
from ctypes import wintypes
p=n=0
p=ctypes.c_ushort();n=ctypes.c_ushort()
assert ctypes.windll.kernel32.IsWow64Process2(ctypes.c_void_p(-1),ctypes.byref(p),ctypes.byref(n)) and p.value==0 and n.value==0xaa64
root=pathlib.Path(sys.executable).parent
for f in root.glob('*.pyd'):
 mod=importlib.import_module(f.stem)
 assert pathlib.Path(mod.__file__).resolve()==f.resolve()
print('native Python',sys.version,ssl.OPENSSL_VERSION,sqlite3.sqlite_version)
"""
    runtime=run([py,'-I','-B','-c',code]).stdout.decode().strip()
    checks={}
    fixtures=ROOT/'obz-rs/tests/fixtures'
    expected={'legacy/license2.ace':{'LICENSE':'c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4'},
      'disc/method2.isz':{'hello.txt':'0c9f0bc3d5c3c9f057a3403cd0e334968cfd2c96b8bb162db83327c48944e070'},
      'disc/deflate.daa':{'hello.txt':'0c9f0bc3d5c3c9f057a3403cd0e334968cfd2c96b8bb162db83327c48944e070'},
      'zpaq/encrypted.zpaq':{'hello.txt':'caad595e2efb367f3599b541ab96834f72dcf1d6f71e99efe65e928828c0db87','second.txt':'ee730c5a2e41032d0c296b26b958ff26c81e710e7c30271184a4f5019aca4eff'}}
    for fixture,manifest in expected.items():
        output=scratch/Path(fixture).stem
        args=[cli,'extract',fixtures/fixture,output];encrypted=fixture.endswith('.zpaq')
        if encrypted:args+=['--password-stdin']
        run(args,data=b'test-password\n' if encrypted else None)
        assert files(output)==manifest,fixture
        checks[fixture]=manifest
    text=scratch/'sample.txt';text.write_bytes(b'native ARM64 sample\n')
    archive=scratch/'sample.7z'
    run([cli,'create',archive,'--format','7z',text])
    run([app/'runtime/7zip/7z.exe','t',archive])
    desktop=smoke_desktop(app/'qing-zip.exe',archive,scratch)
    # Force the bundled fallback in a disposable layout, independently of any
    # globally installed 7-Zip. Restore the same owned directory before cleanup.
    nana=app/'runtime/nanazip-arm64';hidden=app/'runtime/temporarily-disabled-nana'
    assert nana.is_dir() and not hidden.exists()
    nana.rename(hidden)
    try:
        c=json.loads(run([cli,'capabilities']).stdout)['engine']
        assert Path(c['engine']).resolve()==(app/'runtime/7zip/7z.exe').resolve()
        run([cli,'extract',archive,scratch/'fallback']);assert sha(scratch/'fallback/sample.txt')==sha(text)
    finally:hidden.rename(nana)
    sfx=scratch/'self-extract.exe'
    run([cli,'create',sfx,'--format','sfx',text])
    assert machine(sfx)==0xaa64
    run([sfx,'-y','-o'+str(scratch/'sfx-out')],cwd=scratch,timeout=45)
    assert sha(scratch/'sfx-out/sample.txt')==sha(text)
    verify_layout(app/'runtime')
    return {'python':runtime,'desktop':desktop,'authored_imports':dependencies,'full_manifest_checks':checks,'native_sfx_executed':True,'native_7zip_fallback':True}

def main():
    ap=argparse.ArgumentParser();ap.add_argument('dist',type=Path);ap.add_argument('downloads',type=Path);a=ap.parse_args()
    out=a.dist.resolve();downloads=a.downloads.resolve()
    version=json.loads((ROOT/'obz-gui/tauri.conf.json').read_text())['version']
    prefix=f'qing-zip-{version}-arm64'
    portable=out/(prefix+'-portable.zip');stage=out/portable.stem;setup=out/(prefix+'-setup.exe')
    assert portable.is_file() and not setup.exists()
    with tempfile.TemporaryDirectory() as td:
        tmp=Path(td)
        with zipfile.ZipFile(fetch(downloads,NSIS)) as archive:archive.extractall(tmp/'nsis')
        nsis=next((tmp/'nsis').glob('*/makensis.exe'))
        run([nsis,'/INPUTCHARSET','UTF8','/DARM64_PAYLOAD',f'/DVERSION={version}',f'/DSTAGING={stage}',f'/DOUTPUT={setup}',ROOT/'scripts/installer.nsi'])
        assert machine(setup)==0x14c,'unexpected installer bootstrap'
        exported=files(stage)
        with zipfile.ZipFile(portable) as z:z.extractall(tmp/'portable')
        assert files(tmp/'portable')==exported
        report={'version':version,'source_commit':os.environ.get('GITHUB_SHA'),'workflow_run_id':os.environ.get('GITHUB_RUN_ID'),'scope':'native Windows ARM64, official x86 NSIS bootstrap for setup and uninstall only',
                'portable_files':len(exported),'portable':exercise(tmp/'portable',tmp/'portable-test')}
        inst=Path(os.environ['LOCALAPPDATA'])/'Programs/qing-zip-arm64'
        arp=r'Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip-arm64'
        shortcuts=Path(os.environ['APPDATA'])/'Microsoft/Windows/Start Menu/Programs/qing-zip-arm64'
        assert not inst.exists() and key(arp,'InstallLocation') is None and not shortcuts.exists()
        x64_install=r'Software\qing-zip\Installer'
        x64_arp=r'Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip'
        previous_x64=(key(x64_install,'InstallDir'),key(x64_arp,'InstallLocation'))
        run([setup,'/S'])
        assert inst.is_dir() and Path(key(arp,'InstallLocation')).resolve()==inst.resolve()
        uninstall=inst/'Uninstall.exe'
        assert machine(uninstall)==0x14c and (shortcuts/'qing-zip.lnk').is_file()
        assert shortcut_target(shortcuts/'qing-zip.lnk').samefile(inst/'qing-zip.exe')
        assert shortcut_target(shortcuts/'卸载.lnk').samefile(uninstall)
        installed=files(inst);del installed['Uninstall.exe'];assert installed==exported
        report['installed']=exercise(inst,tmp/'installed-test')
        keep=inst/'user-sentinel.txt';keep.write_text('retain this local file',encoding='utf8')
        assoc_fixture=seed_runner_associations(inst/'qing-zip.exe')
        run([uninstall,'/S'])
        deadline=time.monotonic()+90
        while time.monotonic()<deadline and (uninstall.exists() or key(arp,'InstallLocation') is not None):time.sleep(.2)
        remaining=files(inst)
        if remaining!={'user-sentinel.txt':sha(keep)} or keep.read_text(encoding='utf8')!='retain this local file':
            raise ValueError('uninstall residue or changed sentinel: '+str(sorted(remaining)))
        assert key(arp,'InstallLocation') is None and not shortcuts.exists()
        keep.unlink();inst.rmdir()
        assert previous_x64==(key(x64_install,'InstallDir'),key(x64_arp,'InstallLocation'))
        report['uninstall']={'bootstrap':'x86','registration_removed':True,'shortcuts_removed':True,'user_file_preserved':True}
        if assoc_fixture:
            report['uninstall']['file_associations']=check_runner_associations(assoc_fixture)
            report['uninstall']['foreign_shortcuts']=verify_foreign_shortcuts(setup,inst,shortcuts,tmp/'portable/qing-zip.exe',arp)
    report['assets']={}
    for p in [setup,portable]:
        digest=sha(p)
        report['assets'][p.name]={'sha256':digest,'bytes':p.stat().st_size}
        (out/(p.name+'.sha256')).write_text(digest+'  '+p.name+'\n',encoding='ascii')
    (out/(prefix+'-verification.json')).write_text(json.dumps(report,indent=2),encoding='utf8')
    print(json.dumps(report))

if __name__=='__main__':main()
