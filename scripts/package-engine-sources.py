"""Publish corresponding upstream sources and local patches for bundled runtimes.

The reviewed manifests pin the downloads. Sources are a separate release asset,
so normal installation does not add a source checkout to the application folder.
"""
import argparse
import hashlib
import json
import shutil
import tempfile
import urllib.request
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--arm64-runtime', type=Path, help='Verified staged ARM64 runtime to accompany the trial package')
parser.add_argument('--output-directory', type=Path)
parser.add_argument('--source-cache', type=Path, help='Reuse source archives downloaded during the runtime build')
args = parser.parse_args()
runtime = args.arm64_runtime.resolve(strict=True) if args.arm64_runtime else root / 'runtime'
if args.arm64_runtime:
    from arm64_package_runtime import verify_layout
    verify_layout(runtime)
version = json.loads((root / 'obz-gui/tauri.conf.json').read_text())['version']
components = [
    ('nanazip', 'nanazip-7.0.1882.0/NanaZip-7.0.1882.0-source.zip'),
    ('7zip', '7zip-26.06/7z2606-src.7z'),
]
sources = []
for component, relative in components:
    component_dir = 'nanazip-arm64' if component == 'nanazip' and args.arm64_runtime else component
    manifest = json.loads((runtime / component_dir / 'provenance.json').read_text(encoding='utf-8'))
    url = manifest['source_archive_url']
    expected = manifest['source_archive_sha256']
    if not url.startswith(('https://github.com/ip7z/7zip/', 'https://api.github.com/repos/M2Team/NanaZip/')):
        raise SystemExit('Unrecognized source origin')
    cached = root / 'tools' / relative
    if args.source_cache and (args.source_cache / cached.name).is_file():
        cached = args.source_cache / cached.name
    if not cached.is_file():
        cached.parent.mkdir(parents=True, exist_ok=True)
        urls = [url]
        prefix = 'https://api.github.com/repos/M2Team/NanaZip/zipball/'
        if url.startswith(prefix):
            urls.append('https://codeload.github.com/M2Team/NanaZip/legacy.zip/refs/tags/' + url[len(prefix):])
        with tempfile.TemporaryDirectory(dir=cached.parent) as temp:
            staged = Path(temp) / cached.name
            for address in urls:
                try:
                    with urllib.request.urlopen(address, timeout=120) as response, staged.open('wb') as out:
                        shutil.copyfileobj(response, out)
                    break
                except OSError:
                    if address == urls[-1]:
                        raise
            if hashlib.sha256(staged.read_bytes()).hexdigest() != expected:
                raise SystemExit(f'Source integrity mismatch: {component}')
            staged.replace(cached)
    if hashlib.sha256(cached.read_bytes()).hexdigest() != expected:
        raise SystemExit(f'Cached source integrity mismatch: {component}')
    sources.append((component, cached, manifest))

suffix = '-arm64' if args.arm64_runtime else ''
output = (args.output_directory or root / 'dist') / f'qing-zip-{version}{suffix}-engine-sources.zip'
output.parent.mkdir(exist_ok=True)
readme = (
    'Corresponding source for qing-zip bundled archive engines.\n'
    + ('These are the exact upstream source archives. The ARM64 GUI self-extractor was rebuilt from 7-Zip SFXWin source.\n'
     if args.arm64_runtime else 'The NanaZip Core x64 DLL is modified; apply nanazip-modified/zip-aes-acp.patch to the included upstream source and follow nanazip-modified/zip-aes-build.md. The other NanaZip files and 7-Zip runtime are upstream binaries.\n')
    + 'See each archive for its licenses, build instructions, and upstream credits.\n'
    'Modified lzma-rust2 0.23.0 source, original license and upstream provenance are included in lzma-rust2-modified-0.23.0/.\n'
    'The modified ZPAQ helper source and build instructions are in the portable package runtime/zpaq/.\n'
)
expected_members = {'README.txt': readme.encode('utf-8')}
for component, source, manifest in sources:
    expected_members[source.name] = source.read_bytes()
    expected_members[f'{component}-provenance.json'] = (json.dumps(manifest, indent=2) + '\n').encode('utf-8')
if not args.arm64_runtime:
    for name in ('zip-aes-acp.patch', 'zip-aes-build.md'):
        expected_members[f'nanazip-modified/{name}'] = (root / 'obz-rs/native/nana' / name).read_bytes()
# Include the exact locally modified crate, with its original license/credits.
from modified_crate_sources import source_members
expected_members.update(source_members(root))
if output.exists():
    # A previously prepared exact source asset is safe to reuse. Never replace
    # an existing release file or accept extra/mismatched archive members.
    with zipfile.ZipFile(output) as archive:
        names = archive.namelist()
        if len(names) != len(expected_members) or set(names) != set(expected_members):
            raise SystemExit('Existing engine source asset has unexpected contents')
        for name, expected_bytes in expected_members.items():
            if archive.read(name) != expected_bytes:
                raise SystemExit(f'Existing engine source asset mismatch: {name}')
    print(f'Verified existing source asset: {output}')
    raise SystemExit(0)
with zipfile.ZipFile(output, 'x', compression=zipfile.ZIP_STORED) as archive:
    for name, content in expected_members.items():
        archive.writestr(name, content)
print(output)

if args.arm64_runtime:
    digest=hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_name(output.name+'.sha256').write_text(digest+'  '+output.name+'\n',encoding='ascii')
