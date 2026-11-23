"""Install, upgrade and uninstall only on an asserted disposable hosted runner."""
import argparse, ctypes, hashlib, json, os, shutil, subprocess, tempfile, time, winreg, zipfile
from ctypes import wintypes
from pathlib import Path

OLD_SETUP='83a08671edde7bb53ff37582f4d82fd880c968029ddcd39001119efc11a979e1'
OLD_PORTABLE='6acef9b5a4ae726b6e6bd9bad4eecda5b4238d5f60296cc4b5d74e7671efb84a'
ARP=r'Software\Microsoft\Windows\CurrentVersion\Uninstall\qing-zip'
INSTALL=r'Software\qing-zip\Installer'

def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while chunk:=f.read(1024*1024): h.update(chunk)
    return h.hexdigest()

def executable_version(path):
    version=ctypes.WinDLL('version',use_last_error=True)
    version.GetFileVersionInfoSizeW.argtypes=[wintypes.LPCWSTR,ctypes.POINTER(wintypes.DWORD)]
    version.GetFileVersionInfoSizeW.restype=wintypes.DWORD
    version.GetFileVersionInfoW.argtypes=[wintypes.LPCWSTR,wintypes.DWORD,wintypes.DWORD,wintypes.LPVOID]
    version.GetFileVersionInfoW.restype=wintypes.BOOL
    version.VerQueryValueW.argtypes=[wintypes.LPCVOID,wintypes.LPCWSTR,ctypes.POINTER(ctypes.c_void_p),ctypes.POINTER(wintypes.UINT)]
    version.VerQueryValueW.restype=wintypes.BOOL
    length=version.GetFileVersionInfoSizeW(str(path),None)
    if not length: raise ctypes.WinError(ctypes.get_last_error())
    data=ctypes.create_string_buffer(length)
    if not version.GetFileVersionInfoW(str(path),0,length,data): raise ctypes.WinError(ctypes.get_last_error())
    pointer=ctypes.c_void_p();size=wintypes.UINT()
    if not version.VerQueryValueW(data,'\\',ctypes.byref(pointer),ctypes.byref(size)) or size.value<16:
        raise RuntimeError('missing fixed executable version')
    fields=ctypes.cast(pointer,ctypes.POINTER(wintypes.DWORD))
    if fields[0]!=0xFEEF04BD: raise RuntimeError('invalid executable version signature')
    ms,ls=fields[2],fields[3]
    return '.'.join(str(v) for v in (ms>>16,ms&0xffff,ls>>16,ls&0xffff))

def key_exists(name):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,name,0,winreg.KEY_READ|winreg.KEY_WOW64_64KEY): return True
    except FileNotFoundError: return False

def value(name,field):
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER,name,0,winreg.KEY_READ|winreg.KEY_WOW64_64KEY) as key:
            return winreg.QueryValueEx(key,field)[0]
    except FileNotFoundError: return None

def run(args, cwd, *, timeout=180):
    result=subprocess.run([str(x) for x in args],cwd=cwd,stdin=subprocess.DEVNULL,capture_output=True,
                          timeout=timeout,creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode: raise RuntimeError(f'{Path(args[0]).name} exited {result.returncode}')
    return result

def check(condition,message):
    if not condition: raise AssertionError(message)

def installed_version(expected,inst):
    check(value(ARP,'DisplayVersion')==expected,'unexpected registered version')
    check(os.path.normcase(os.path.normpath(value(ARP,'InstallLocation') or ''))==os.path.normcase(str(inst)),'unexpected registration location')
    check(os.path.normcase(os.path.normpath(value(INSTALL,'InstallDir') or ''))==os.path.normcase(str(inst)),'unexpected installer location')

def uninstall(inst,root):
    run([inst/'Uninstall.exe','/S'],root)
    deadline=time.monotonic()+90
    while time.monotonic()<deadline and ((inst/'Uninstall.exe').exists() or key_exists(ARP) or key_exists(INSTALL)):
        time.sleep(.2)
    check(not (inst/'Uninstall.exe').exists() and not key_exists(ARP) and not key_exists(INSTALL),'uninstall did not clear executable and registration')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('old-setup','old-portable','candidate-setup','candidate-stage','out'):
        parser.add_argument('--'+name,required=True,type=Path)
    args=parser.parse_args()
    # This is a defensive environment assertion, not cryptographic attestation.
    check(os.environ.get('GITHUB_ACTIONS')=='true' and os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted'
          and os.environ.get('RUNNER_OS')=='Windows' and os.environ.get('RUNNER_ARCH')=='X64',
          'requires a disposable GitHub-hosted Windows x64 runner')
    runner_temp=Path(os.environ['RUNNER_TEMP']).resolve(strict=True)
    old_setup=args.old_setup.resolve(strict=True);old_portable=args.old_portable.resolve(strict=True)
    candidate_setup=args.candidate_setup.resolve(strict=True);stage=args.candidate_stage.resolve(strict=True)
    check(sha(old_setup)==OLD_SETUP and sha(old_portable)==OLD_PORTABLE,'official v0.12.20 SHA-256 mismatch')
    check(not key_exists(ARP) and not key_exists(INSTALL),'existing qing-zip registration; refuse to install')
    programs=value(r'Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders','Programs')
    check(bool(programs),'cannot locate the current user Programs folder')
    shortcuts=Path(programs)/'qing-zip'
    check(not shortcuts.exists(),'existing qing-zip Start menu directory; refuse to install')
    out=args.out.resolve();out.mkdir(parents=True,exist_ok=False)
    report={'status':'starting','release':'v0.12.20','old_setup_sha256':OLD_SETUP,'old_portable_sha256':OLD_PORTABLE,
            'candidate_setup_sha256':sha(candidate_setup),'candidate_gui_sha256':sha(stage/'qing-zip.exe'),
            'candidate_cli_sha256':sha(stage/'obz.exe'),'checks':[],'owned_root_cleaned':False}
    def save(): (out/'receipt.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    def passed(name): report['checks'].append(name);save()
    save();root=Path(tempfile.mkdtemp(prefix='qznsis-',dir=runner_temp)).resolve();inst=root/'app';uninstalled=False
    try:
        check(root.is_relative_to(runner_temp) and inst.parent==root and not inst.exists(),'installation target must be a fresh owned temporary path')
        check(not any(char.isspace() for char in str(inst)),'NSIS /D path must not require quoting')
        with zipfile.ZipFile(old_portable) as archive:
            old_hashes={name:hashlib.sha256(archive.read(name)).hexdigest() for name in ('qing-zip.exe','obz.exe','obz-worker.exe')}
        run([old_setup,'/S',f'/D={inst}'],root)
        installed_version('0.12.20',inst)
        check(executable_version(inst/'qing-zip.exe')=='0.12.20.0','unexpected old executable version')
        check(all(sha(inst/name)==digest for name,digest in old_hashes.items()),'installed old binaries differ from official portable')
        check((shortcuts/'qing-zip.lnk').is_file() and (shortcuts/'卸载.lnk').is_file(),'expected Start menu shortcuts missing')
        passed('official_release_installed_to_owned_directory')
        user=inst/'用户保留';user.mkdir();source=root/'样本.txt';source.write_bytes(('普通输入与个人数据哨兵\n'*4096).encode('utf-8'))
        sentinel=user/'无关文件.txt';sentinel.write_bytes(b'unrelated user file, preserve\n')
        external=root/'外部哨兵.txt';external.write_bytes(b'external file, preserve\n')
        archive=user/'原始归档.zip';run([inst/'obz.exe','create',archive,source,'--format','zip'],root)
        original_sha=sha(archive);source_sha=sha(source);sentinel_sha=sha(sentinel);external_sha=sha(external)
        with zipfile.ZipFile(archive) as z: check(z.read(source.name)==source.read_bytes(),'old archive content mismatch')
        passed('old_archive_and_user_sentinels_created')
        run([candidate_setup,'/S',f'/D={inst}'],root)
        installed_version('0.12.23',inst)
        check(executable_version(inst/'qing-zip.exe')=='0.12.23.0','unexpected candidate executable version')
        shipped={p.relative_to(stage).as_posix():sha(p) for p in stage.rglob('*') if p.is_file()}
        check(all((inst/name).is_file() and sha(inst/name)==digest for name,digest in shipped.items()),'upgraded payload differs from candidate portable stage')
        check(sha(archive)==original_sha and sha(source)==source_sha and sha(sentinel)==sentinel_sha and sha(external)==external_sha,'upgrade changed user data')
        passed('in_place_upgrade_payload_and_user_data_verified')
        output=root/'解压结果';run([inst/'obz.exe','test',archive],root)
        run([inst/'obz.exe','extract',archive,output,'--threads','1'],root)
        extracted=output/source.name
        check(sha(extracted)==source_sha and sha(archive)==original_sha,'extraction or source archive changed')
        passed('upgraded_cli_tests_and_extracts_original_archive')
        uninstall(inst,root);uninstalled=True
        check(not (shortcuts/'qing-zip.lnk').exists() and not (shortcuts/'卸载.lnk').exists(),'owned Start menu shortcuts remained')
        check(all(not (inst/name).exists() for name in shipped),'candidate shipped file survived uninstall')
        remaining={p.relative_to(inst).as_posix() for p in inst.rglob('*') if p.is_file()}
        check(remaining=={sentinel.relative_to(inst).as_posix(),archive.relative_to(inst).as_posix()},'unexpected remaining files after uninstall')
        check(sha(archive)==original_sha and sha(source)==source_sha and sha(extracted)==source_sha
              and sha(sentinel)==sentinel_sha and sha(external)==external_sha,'uninstall changed user data')
        report.update({'old_executable_version':'0.12.20.0','candidate_executable_version':'0.12.23.0',
                       'source_sha256':source_sha,'archive_sha256':original_sha,'extracted_sha256':sha(extracted),
                       'inside_sentinel_sha256':sentinel_sha,'outside_sentinel_sha256':external_sha,
                       'candidate_shipped_file_count':len(shipped),'remaining_user_file_count':len(remaining)})
        passed('uninstall_removed_payload_and_registration_preserved_user_files')
        report['status']='passed'
    except Exception as error:
        report['status']='failed';report['error']=f'{type(error).__name__}: {error}'.replace(str(root),'<owned-root>')[:1500]
    finally:
        if not uninstalled and (inst/'Uninstall.exe').is_file() and value(INSTALL,'InstallDir') and os.path.normcase(os.path.normpath(value(INSTALL,'InstallDir')))==os.path.normcase(str(inst)):
            try: uninstall(inst,root);uninstalled=True;report['cleanup_uninstall']='completed'
            except Exception as error: report['cleanup_uninstall']=f'failed: {type(error).__name__}'
        if uninstalled or (not key_exists(ARP) and not key_exists(INSTALL) and not (inst/'qing-zip.exe').exists()):
            try: shutil.rmtree(root);report['owned_root_cleaned']=not root.exists()
            except Exception as error: report['cleanup_error']=type(error).__name__
        save()
    print(json.dumps(report,ensure_ascii=True),flush=True)
    if report['status']!='passed' or not report['owned_root_cleaned']: raise SystemExit(1)

if __name__=='__main__': main()
