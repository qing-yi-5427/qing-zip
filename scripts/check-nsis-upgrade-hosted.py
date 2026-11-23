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

def installed_version(expected,inst,arp=ARP,install=INSTALL):
    check(value(arp,'DisplayVersion')==expected,'unexpected registered version')
    check(os.path.normcase(os.path.normpath(value(arp,'InstallLocation') or ''))==os.path.normcase(str(inst)),'unexpected registration location')
    check(os.path.normcase(os.path.normpath(value(install,'InstallDir') or ''))==os.path.normcase(str(inst)),'unexpected installer location')

def uninstall(inst,root,arp=ARP,install=INSTALL):
    run([inst/'Uninstall.exe','/S'],root)
    deadline=time.monotonic()+90
    while time.monotonic()<deadline and ((inst/'Uninstall.exe').exists() or key_exists(arp) or key_exists(install)):
        time.sleep(.2)
    check(not (inst/'Uninstall.exe').exists() and not key_exists(arp) and not key_exists(install),'uninstall did not clear executable and registration')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('old-setup','old-portable','candidate-setup','candidate-stage','out'):
        parser.add_argument('--'+name,required=True,type=Path)
    parser.add_argument('--arch',choices=('X64','ARM64'),default='X64')
    parser.add_argument('--use-default-location', action='store_true',
                        help='Let the official installer select its own folder on a fresh hosted VM')
    args=parser.parse_args()
    # This is a defensive environment assertion, not cryptographic attestation.
    check(os.environ.get('GITHUB_ACTIONS')=='true' and os.environ.get('RUNNER_ENVIRONMENT')=='github-hosted'
          and os.environ.get('RUNNER_OS')=='Windows' and os.environ.get('RUNNER_ARCH')==args.arch,
          f'requires a disposable GitHub-hosted Windows {args.arch} runner')
    if args.arch=='ARM64':
        product='qing-zip-arm64'
        previous='0.11.27'
        old_setup_sha='46f3a8a091dd7b5119e12a3e0e5da4fabd0bbd6e9aaf50a6fb6356c719a22c4c'
        old_zip_sha='9aafd4acd77a9a3eea535ae1beecb922ac8ef7ddd44a2c91875a93e87487f113'
    else:
        product='qing-zip'
        previous='0.12.20'
        old_setup_sha,old_zip_sha=OLD_SETUP,OLD_PORTABLE
    arp=rf'Software\Microsoft\Windows\CurrentVersion\Uninstall\{product}'
    install=rf'Software\{product}\Installer'
    current=json.loads((Path(__file__).resolve().parents[1]/'obz-gui/tauri.conf.json').read_text(encoding='utf-8'))['version']
    runner_temp=Path(os.environ['RUNNER_TEMP']).resolve(strict=True)
    old_setup=args.old_setup.resolve(strict=True);old_portable=args.old_portable.resolve(strict=True)
    candidate_setup=args.candidate_setup.resolve(strict=True);stage=args.candidate_stage.resolve(strict=True)
    check(sha(old_setup)==old_setup_sha and sha(old_portable)==old_zip_sha,f'official v{previous} SHA-256 mismatch')
    check(not key_exists(arp) and not key_exists(install),'existing qing-zip registration; refuse to install')
    programs=value(r'Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders','Programs')
    check(bool(programs),'cannot locate the current user Programs folder')
    shortcuts=Path(programs)/product
    check(not shortcuts.exists(),'existing qing-zip Start menu directory; refuse to install')
    default_location=Path(os.environ['LOCALAPPDATA'])/'Programs'/product
    check(default_location.is_absolute() and not os.path.lexists(default_location),'existing default installation directory; refuse to install')
    out=args.out.resolve();out.mkdir(parents=True,exist_ok=False)
    report={'install_location_mode':'default' if args.use_default_location else 'custom',
            'status':'starting','release':'v'+previous,'runner_arch':args.arch,'source':os.environ.get('GITHUB_SHA'),
            'old_setup_sha256':old_setup_sha,'old_portable_sha256':old_zip_sha,
            'candidate_setup_sha256':sha(candidate_setup),'candidate_gui_sha256':sha(stage/'qing-zip.exe'),
            'candidate_cli_sha256':sha(stage/'obz.exe'),'checks':[],'owned_root_cleaned':False}
    def save(): (out/'receipt.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    def passed(name): report['checks'].append(name);save()
    save()
    root=Path(tempfile.mkdtemp(prefix='qznsis-',dir=runner_temp)).resolve()
    inst=default_location if args.use_default_location else root/'app'
    old_options=['/S']
    uninstalled=False
    try:
        check(root.parent==runner_temp and not os.path.lexists(inst),'installation target must be unused')
        if not args.use_default_location:
            check(not any(char.isspace() for char in str(inst)),'NSIS /D path must not require quoting')
            old_options.append(f'/D={inst}')
        shipped={p.relative_to(stage).as_posix():sha(p) for p in stage.rglob('*') if p.is_file()}
        with zipfile.ZipFile(old_portable) as archive:
            old_hashes={name:hashlib.sha256(archive.read(name)).hexdigest() for name in ('qing-zip.exe','obz.exe','obz-worker.exe')}
            old_only={name:hashlib.sha256(archive.read(name)).hexdigest() for name in archive.namelist()
                      if not name.endswith('/') and name not in shipped}
        run([old_setup,*old_options],root)
        installed_version(previous,inst,arp,install)
        check(executable_version(inst/'qing-zip.exe')==previous+'.0','unexpected old executable version')
        check(all(sha(inst/name)==digest for name,digest in old_hashes.items()),'installed old binaries differ from official portable')
        check((shortcuts/'qing-zip.lnk').is_file() and (shortcuts/'卸载.lnk').is_file(),'expected Start menu shortcuts missing')
        check(all((inst/name).is_file() and sha(inst/name)==digest for name,digest in old_only.items()),'old-only release files missing or changed immediately after old install')
        report['old_only_installed']=[{'relative_path':name,'sha256':digest} for name,digest in sorted(old_only.items())]
        report['first_install_used_directory_override']=len(old_options)>1
        report['registered_install_directory_matches_default']=Path(value(arp,'InstallLocation')) == default_location
        passed('official_release_installed_to_owned_directory')
        user=inst/'用户保留';user.mkdir();source=root/'样本.txt';source.write_bytes(('普通输入与个人数据哨兵\n'*4096).encode('utf-8'))
        sentinel=user/'无关文件.txt';sentinel.write_bytes(b'unrelated user file, preserve\n')
        external=root/'外部哨兵.txt';external.write_bytes(b'external file, preserve\n')
        archive=user/'原始归档.zip';run([inst/'obz.exe','create',archive,source,'--format','zip'],root)
        original_sha=sha(archive);source_sha=sha(source);sentinel_sha=sha(sentinel);external_sha=sha(external)
        with zipfile.ZipFile(archive) as z: check(z.read(source.name)==source.read_bytes(),'old archive content mismatch')
        passed('old_archive_and_user_sentinels_created')
        run([candidate_setup,'/S'],root)
        installed_version(current,inst,arp,install)
        if not args.use_default_location:
            check(not os.path.lexists(default_location),'custom-directory upgrade also wrote to default location')
        check(executable_version(inst/'qing-zip.exe')==current+'.0','unexpected candidate executable version')
        check(all((inst/name).is_file() and sha(inst/name)==digest for name,digest in shipped.items()),'upgraded payload differs from candidate portable stage')
        check(sha(archive)==original_sha and sha(source)==source_sha and sha(sentinel)==sentinel_sha and sha(external)==external_sha,'upgrade changed user data')
        passed('in_place_upgrade_payload_and_user_data_verified')
        output=root/'解压结果';run([inst/'obz.exe','test',archive],root)
        run([inst/'obz.exe','extract',archive,output,'--threads','1'],root)
        extracted=output/source.name
        check(sha(extracted)==source_sha and sha(archive)==original_sha,'extraction or source archive changed')
        passed('upgraded_cli_tests_and_extracts_original_archive')
        data_paths={'source':source,'archive':archive,'extracted':extracted,
                    'inside_sentinel':sentinel,'outside_sentinel':external}
        before_uninstall={name:sha(path) for name,path in data_paths.items()}
        report['user_data_before_uninstall']=before_uninstall;save()
        uninstall(inst,root,arp,install);uninstalled=True
        after_uninstall={name:sha(path) if path.is_file() else None for name,path in data_paths.items()}
        report['user_data_after_uninstall']=after_uninstall;save()
        check(after_uninstall==before_uninstall,'uninstall changed or removed synthetic user data')
        passed('uninstall_preserved_all_synthetic_user_data')
        check(not (shortcuts/'qing-zip.lnk').exists() and not (shortcuts/'卸载.lnk').exists(),'owned Start menu shortcuts remained')
        report['candidate_files_remaining']=[name for name in sorted(shipped) if (inst/name).exists()]
        save()
        check(not report['candidate_files_remaining'],'candidate shipped file survived uninstall; see receipt')
        passed('uninstall_removed_candidate_payload_registration_and_shortcuts')
        remaining={p.relative_to(inst).as_posix():sha(p) for p in inst.rglob('*') if p.is_file()}
        expected_user={sentinel.relative_to(inst).as_posix():sentinel_sha,archive.relative_to(inst).as_posix():original_sha}
        # Silent upgrades never consent to moving historical files. Only the
        # old-only files verified during the clean install may remain alongside
        # the two synthetic user files, and every hash must still match.
        expected_remaining={**old_only,**expected_user}
        report['remaining_after_uninstall']=[]
        for name,digest in sorted(remaining.items()):
            verified_old=old_only.get(name)==digest
            classification=('synthetic_user_file' if expected_user.get(name)==digest else
                            'unchanged_previous_release_file' if verified_old else 'unexpected')
            report['remaining_after_uninstall'].append({'relative_path':name,'sha256':digest,
                'classification':classification,'verified_old_only_at_install':verified_old})
        save()
        check(remaining==expected_remaining,'silent upgrade and uninstall did not preserve exactly the expected files and hashes; see receipt')
        passed('silent_upgrade_and_uninstall_preserved_verified_previous_release_files')
        check(sha(archive)==original_sha and sha(source)==source_sha and sha(extracted)==source_sha
              and sha(sentinel)==sentinel_sha and sha(external)==external_sha,'uninstall changed user data')
        report.update({'old_executable_version':previous+'.0','candidate_executable_version':current+'.0',
                       'source_sha256':source_sha,'archive_sha256':original_sha,'extracted_sha256':sha(extracted),
                       'inside_sentinel_sha256':sentinel_sha,'outside_sentinel_sha256':external_sha,
                       'candidate_shipped_file_count':len(shipped),'remaining_user_file_count':len(expected_user),
                       'remaining_previous_release_file_count':len(old_only)})
        passed('uninstall_removed_payload_and_registration_preserved_user_files')
        report['status']='passed'
    except Exception as error:
        report['status']='failed';report['error']=f'{type(error).__name__}: {error}'.replace(str(root),'<owned-root>')[:1500]
    finally:
        if not uninstalled and (inst/'Uninstall.exe').is_file() and value(install,'InstallDir') and os.path.normcase(os.path.normpath(value(install,'InstallDir')))==os.path.normcase(str(inst)):
            try: uninstall(inst,root,arp,install);uninstalled=True;report['cleanup_uninstall']='completed'
            except Exception as error: report['cleanup_uninstall']=f'failed: {type(error).__name__}'
        if uninstalled or (not key_exists(arp) and not key_exists(install) and not (inst/'qing-zip.exe').exists()):
            try: shutil.rmtree(root);report['owned_root_cleaned']=not root.exists()
            except Exception as error: report['cleanup_error']=type(error).__name__
        save()
    print(json.dumps(report,ensure_ascii=True),flush=True)
    if report['status']!='passed' or not report['owned_root_cleaned']: raise SystemExit(1)

if __name__=='__main__': main()
