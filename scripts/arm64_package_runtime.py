"""Build the ARM64 runtime from fixed releases in a native MSVC developer shell."""
import argparse
import hashlib
import json
import shutil
import struct
import subprocess
import tempfile
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PYTHON = ('https://www.python.org/ftp/python/3.14.9/python-3.14.9-embed-arm64.zip',
          'a8212664f7622ebc0cb0f9a42d7beee36cebedab97ff47c6bba668cfd85b676d')
SEVEN = ('https://github.com/ip7z/7zip/releases/download/26.05/7z2605-arm64.exe',
         '86d5790d5dd84cab4636c3bb7fab10d3ed7cd3e57e6119401bd2769e28165268')
SEVEN_FILES = {'7z.dll':'311d0a45182401e10c71a12b33a5c4a7171f6a8c1d67eeec9ef5dc2a5c962fe8',
               '7z.exe':'08c3868b294fd12984bd1537a939ae7200ad695420dc65a2eeeec9f8fdf99772',
               'License.txt':'519ac0a4bded9c18ea02e0afb71f663d8c47373bd9facd3ac96a79f51d77765d'}
NSIS = ('https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13.zip',
        'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8')
SUFFIXES = {'.exe','.dll','.pyd','.sfx'}

def sha(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file,'sha256').hexdigest()

def machine(path):
    with path.open('rb') as f:
        dos=f.read(64)
        if len(dos)!=64 or dos[:2]!=b'MZ': raise ValueError(f'invalid PE: {path}')
        f.seek(struct.unpack_from('<I',dos,60)[0])
        header=f.read(6)
        if len(header)!=6 or header[:4]!=b'PE\0\0': raise ValueError(f'invalid PE: {path}')
        return struct.unpack_from('<H',header,4)[0]

def fetch(directory, source):
    url, digest = source
    target = directory / url.rsplit('/',1)[1]
    if not target.is_file():
        for attempt in range(3):
            try:
                req = urllib.request.Request(url, headers={'User-Agent':'qing-zip-build'})
                with urllib.request.urlopen(req,timeout=90) as response, tempfile.NamedTemporaryFile(dir=directory,delete=False) as output:
                    temp=Path(output.name)
                    shutil.copyfileobj(response,output)
                if sha(temp)!=digest: raise ValueError(f'upstream archive hash mismatch: {url}')
                temp.replace(target)
                break
            except OSError:
                if attempt==2: raise
    if sha(target)!=digest: raise ValueError(f'upstream archive hash mismatch: {url}')
    return target

def inventory(root):
    return {p.relative_to(root).as_posix():sha(p) for p in sorted(root.rglob('*'))
            if p.is_file() and p!=root/'ARM64-RUNTIME.json'}

def imports(path):
    """Return ordinary and delay-loaded PE module names for a reviewed binary."""
    data=path.read_bytes()
    def u16(i):return struct.unpack_from('<H',data,i)[0]
    def u32(i):return struct.unpack_from('<I',data,i)[0]
    pe=u32(60);opt=pe+24
    directories=opt+(112 if u16(opt)==0x20b else 96)
    start=opt+u16(pe+20)
    sections=[(u32(i+12),max(u32(i+8),u32(i+16)),u32(i+20))
              for i in range(start,start+40*u16(pe+6),40)]
    def offset(addr):
        return next(raw+addr-rva for rva,size,raw in sections if rva<=addr<rva+size)
    names=set()
    for index,stride,field in [(1,20,12),(13,32,4)]:
        addr=u32(directories+8*index)
        if not addr:continue
        cursor=offset(addr)
        while u32(cursor+field):
            name=offset(u32(cursor+field))
            names.add(data[name:data.index(0,name)].decode('ascii').lower())
            cursor+=stride
    return sorted(names)

def verify_private_imports(path):
    deps=imports(path)
    # Windows 11 includes UCRT and API sets, but the optional VC++ redistributable
    # on a developer machine must not conceal a missing app-local dependency.
    for name in deps:
        if name.startswith(('msvcp1','vcruntime1','concrt1','vcomp1','webview2loader.dll')):
            local=path.parent/name
            if not local.is_file() or machine(local)!=0xaa64:raise ValueError(f'{path.name}: missing native {name}')
    return deps

def verify_layout(root):
    record=json.loads((root/'ARM64-RUNTIME.json').read_text(encoding='utf8'))
    if inventory(root)!=record['files']: raise ValueError('ARM64 runtime file inventory mismatch')
    pins=json.loads((ROOT/'obz-rs/native/nana/provenance-arm64.json').read_text())['files']
    pyfiles=json.loads((ROOT/'scripts/arm64-python-files.json').read_text())
    zsrc=json.loads((ROOT/'runtime/zpaq/SOURCE.json').read_text())['patched_source_sha256']
    for folder,expected in [('nanazip-arm64',pins),('7zip',SEVEN_FILES),('python',pyfiles),('zpaq',zsrc)]:
        for name,digest in expected.items():
            if sha(root/folder/name)!=digest:raise ValueError(f'invalid pinned {folder}/{name}')
    for script in ['acefile.py','ace_worker.py','disc_worker.py','LICENSE-ACEFILE.md']:
        if sha(root/'python'/script)!=sha(ROOT/'runtime/python'/script):raise ValueError(f'invalid Python adapter: {script}')
    files={p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p.suffix.lower() in SUFFIXES}
    if not {'nanazip-arm64/NanaZip.Universal.Console.exe','7zip/7z.exe','7zip/7z.dll','7zip/7z.sfx','python/python.exe','zpaq/obz-zpaq.exe'} <= files:
        raise ValueError('incomplete ARM64 runtime')
    for name in files:
        if machine(root/name)!=0xaa64: raise ValueError(f'non-native runtime binary: {name}')
        verify_private_imports(root/name)
    print(f'Verified {len(files)} ARM64 PE files and {len(record["files"])} runtime files')

def build(out, cache):
    out.mkdir(parents=True,exist_ok=False); cache.mkdir(parents=True,exist_ok=True)
    nana=out/'nanazip-arm64';nana.mkdir()
    provenance=ROOT/'obz-rs/native/nana/provenance-arm64.json'
    meta=json.loads(provenance.read_text())
    for name,expected in meta['files'].items():
        src=ROOT/'runtime/nanazip-arm64'/name
        if sha(src)!=expected: raise ValueError(f'invalid NanaZip binary: {name}')
        shutil.copyfile(src,nana/name)
    shutil.copyfile(provenance,nana/'provenance.json')
    exe=nana/'NanaZip.Universal.Console.exe'
    seven=out/'7zip';seven.mkdir()
    pkg=fetch(cache,SEVEN)
    subprocess.run([exe,'x','-y',f'-o{seven}',pkg,*SEVEN_FILES],check=True)
    for name,expected in SEVEN_FILES.items():
        if sha(seven/name)!=expected: raise ValueError(f'invalid official 7-Zip file: {name}')
    meta7=json.loads((ROOT/'runtime/7zip/provenance.json').read_text())
    archive=fetch(cache,(meta7['source_archive_url'],meta7['source_archive_sha256']))
    with tempfile.TemporaryDirectory(dir=cache) as temp:
        src=Path(temp)
        subprocess.run([exe,'x','-y',f'-o{src}',archive],check=True)
        sfx=src/'CPP/7zip/Bundles/SFXWin'
        subprocess.run(['nmake','/nologo','PLATFORM=arm64'],cwd=sfx,check=True)
        shutil.copyfile(sfx/'arm64/7z.sfx',seven/'7z.sfx')
    meta7.update(url=SEVEN[0],sha256=SEVEN[1],architecture='arm64',files={**SEVEN_FILES,'7z.sfx':sha(seven/'7z.sfx')},sfx_build='Official SFXWin source, nmake PLATFORM=arm64')
    (seven/'provenance.json').write_text(json.dumps(meta7,indent=2),encoding='utf8')
    py=out/'python';py.mkdir()
    with zipfile.ZipFile(fetch(cache,PYTHON)) as z:
        for name in z.namelist():
            if Path(name).name!=name: raise ValueError('unexpected embedded Python layout')
            # This ancillary x64/ARM64EC object is not imported by any native
            # extension or interpreter. Verify every included extension on Arm.
            if name!='vcruntime140_1.dll': (py/name).write_bytes(z.read(name))
    for name in ['ace_worker.py','disc_worker.py','acefile.py','LICENSE-ACEFILE.md']:
        shutil.copyfile(ROOT/'runtime/python'/name,py/name)
    zp=out/'zpaq';zp.mkdir()
    zmeta=json.loads((ROOT/'runtime/zpaq/SOURCE.json').read_text())
    for name,expected in zmeta['patched_source_sha256'].items():
        path=ROOT/'runtime/zpaq'/name
        if sha(path)!=expected:raise ValueError(f'ZPAQ source differs: {name}')
        shutil.copyfile(path,zp/name)
    for name in ['COPYING','readme.txt']:shutil.copyfile(ROOT/'runtime/zpaq'/name,zp/name)
    with tempfile.TemporaryDirectory(dir=cache) as work:
        subprocess.run(['cl','/nologo','/O2','/EHsc','/MT','/DNOJIT','/guard:cf',zp/'zpaq.cpp',zp/'libzpaq.cpp',
            'advapi32.lib','/Fe:'+str(zp/'obz-zpaq.exe'),'/link','/DYNAMICBASE','/NXCOMPAT','/HIGHENTROPYVA','/guard:cf'],cwd=work,check=True)
    zmeta.update(architecture='arm64',runtime_sha256=sha(zp/'obz-zpaq.exe'),build='Native MSVC ARM64 /MT /DNOJIT /O2 /EHsc /guard:cf')
    zmeta['modifications']=[line.replace('MSVC x64','MSVC ARM64') for line in zmeta['modifications']]
    (zp/'SOURCE.json').write_text(json.dumps(zmeta,indent=2),encoding='utf8')
    with zipfile.ZipFile(fetch(cache,NSIS)) as installer_tools:
        (out/'NSIS-COPYING.txt').write_bytes(installer_tools.read('nsis-3.13/COPYING'))
    (out/'ARM64-RUNTIME.json').write_text(json.dumps({'architecture':'arm64','python_package':PYTHON,'files':inventory(out)},indent=2),encoding='utf8')
    verify_layout(out)

if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('runtime',type=Path);parser.add_argument('downloads',type=Path)
    args=parser.parse_args();build(args.runtime.resolve(),args.downloads.resolve())
