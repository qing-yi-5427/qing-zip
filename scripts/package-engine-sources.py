"""Publish exact upstream source archives alongside the unmodified runtimes.

The reviewed manifests pin the downloads. Sources are a separate release asset,
so normal installation does not add a source checkout to the application folder.
"""
import hashlib
import json
import shutil
import tempfile
import urllib.request
import zipfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
version = json.loads((root / 'obz-gui/tauri.conf.json').read_text())['version']
components = [
    ('nanazip', 'nanazip-7.0.1882.0/NanaZip-7.0.1882.0-source.zip'),
    ('7zip', '7zip-26.05/7z2605-src.7z'),
]
sources = []
for component, relative in components:
    manifest = json.loads((root / 'runtime' / component / 'provenance.json').read_text())
    url = manifest['source_archive_url']
    expected = manifest['source_archive_sha256']
    if not url.startswith(('https://github.com/ip7z/7zip/', 'https://api.github.com/repos/M2Team/NanaZip/')):
        raise SystemExit('Unrecognized source origin')
    cached = root / 'tools' / relative
    cached.parent.mkdir(parents=True, exist_ok=True)
    if not cached.is_file():
        with tempfile.NamedTemporaryFile(dir=cached.parent, delete=False) as staged:
            staged_path = Path(staged.name)
            try:
                with urllib.request.urlopen(url, timeout=120) as response:
                    shutil.copyfileobj(response, staged)
            except BaseException:
                staged.close()
                staged_path.unlink(missing_ok=True)
                raise
        if hashlib.sha256(staged_path.read_bytes()).hexdigest() != expected:
            staged_path.unlink()
            raise SystemExit(f'Source integrity mismatch: {component}')
        staged_path.replace(cached)
    if hashlib.sha256(cached.read_bytes()).hexdigest() != expected:
        raise SystemExit(f'Cached source integrity mismatch: {component}')
    sources.append((component, cached, manifest))

output = root / 'dist' / f'qing-zip-{version}-engine-sources.zip'
output.parent.mkdir(exist_ok=True)
readme = (
    'Corresponding source for qing-zip bundled archive engines.\n'
    'These are the exact upstream source archives; the bundled binaries are unmodified.\n'
    'See each archive for its licenses, build instructions, and upstream credits.\n'
    'The modified ZPAQ helper source and build instructions are in the portable package runtime/zpaq/.\n'
)
expected_members = {'README.txt': readme.encode('utf-8')}
for component, source, manifest in sources:
    expected_members[source.name] = source.read_bytes()
    expected_members[f'{component}-provenance.json'] = (json.dumps(manifest, indent=2) + '\n').encode('utf-8')
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
    for component, source, manifest in sources:
        archive.write(source, source.name)
        archive.writestr(f'{component}-provenance.json', json.dumps(manifest, indent=2) + '\n')
    archive.writestr('README.txt', readme)
print(output)
