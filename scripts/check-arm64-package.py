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
    return {'python':runtime,'authored_imports':dependencies,'full_manifest_checks':checks,'native_sfx_executed':True,'native_7zip_fallback':True}

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
        installed=files(inst);del installed['Uninstall.exe'];assert installed==exported
        report['installed']=exercise(inst,tmp/'installed-test')
        keep=inst/'user-sentinel.txt';keep.write_text('retain this local file',encoding='utf8')
        run([uninstall,'/S'])
        deadline=time.monotonic()+90
        while time.monotonic()<deadline and (uninstall.exists() or key(arp,'InstallLocation') is not None):time.sleep(.2)
        assert files(inst)=={'user-sentinel.txt':sha(keep)} and keep.read_text(encoding='utf8')=='retain this local file'
        assert key(arp,'InstallLocation') is None and not shortcuts.exists()
        keep.unlink();inst.rmdir()
        assert previous_x64==(key(x64_install,'InstallDir'),key(x64_arp,'InstallLocation'))
        report['uninstall']={'bootstrap':'x86','registration_removed':True,'shortcuts_removed':True,'user_file_preserved':True}
    for p in [setup,portable]: (out/(p.name+'.sha256')).write_text(sha(p)+'  '+p.name+'\n',encoding='ascii')
    (out/(prefix+'-verification.json')).write_text(json.dumps(report,indent=2),encoding='utf8')
    print(json.dumps(report))

if __name__=='__main__':main()
