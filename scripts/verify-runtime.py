"""Check shipped 7-Zip files against the reviewed, committed release manifest.

The manifest records the upstream installer URL and GitHub release SHA-256.
Changing the runtime requires reviewing that upstream release and updating the
manifest; this check deliberately does not silently download or update binaries.
"""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parents[1]
runtime = root / 'runtime' / '7zip'
manifest = json.loads((runtime / 'provenance.json').read_text(encoding='utf-8'))
assert manifest['url'].startswith('https://github.com/ip7z/7zip/releases/download/')
assert len(manifest['sha256']) == 64
assert set(manifest['files']) == {'7z.exe', '7z.dll', '7z.sfx', 'License.txt'}
for name, expected in manifest['files'].items():
    actual = hashlib.sha256((runtime / name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f'Runtime integrity mismatch: {name}')
print(f"Verified shipped 7-Zip {manifest['version']} runtime hashes")


runtime = root / 'runtime' / 'nanazip'
manifest = json.loads((runtime / 'provenance.json').read_text(encoding='utf-8'))
assert manifest['package_url'].startswith('https://github.com/M2Team/NanaZip/releases/download/')
assert len(manifest['package_sha256']) == 64
assert set(manifest['files']) == {
    'NanaZip.Universal.Console.exe', 'NanaZip.Core.dll', 'NanaZip.Codecs.dll',
    'NanaZip.ZS.Codecs.dll', 'K7Base.dll', 'K7User.dll',
}
for name, expected in manifest['files'].items():
    actual = hashlib.sha256((runtime / name).read_bytes()).hexdigest()
    if actual != expected:
        raise SystemExit(f'Runtime integrity mismatch: {name}')
print(f"Verified shipped NanaZip {manifest['version']} runtime hashes")

interfaces = root / 'obz-rs' / 'native' / 'nana'
headers = json.loads((interfaces / 'interfaces.json').read_text(encoding='utf-8'))
if (headers['version'] != manifest['version'] or
        headers['source_archive_sha256'] != manifest['source_archive_sha256']):
    raise SystemExit('NanaZip interface source and runtime releases differ')
actual_files = {p.relative_to(interfaces).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
                for directory in ('vendor', 'licenses')
                for p in (interfaces / directory).rglob('*') if p.is_file()}
if actual_files != headers['files']:
    raise SystemExit('NanaZip interface header or license inventory differs from upstream')
print(f"Verified {len(actual_files)} NanaZip interface and license files")

runtime = root / 'runtime' / 'zpaq'
manifest = json.loads((runtime / 'SOURCE.json').read_text(encoding='utf-8'))
assert manifest['source_archive'] == 'https://mattmahoney.net/dc/zpaq715.zip'
assert set(manifest['patched_source_sha256']) == {'zpaq.cpp', 'libzpaq.cpp', 'libzpaq.h'}
expected_files = {'obz-zpaq.exe': manifest['runtime_sha256'], **manifest['patched_source_sha256']}
for name, expected in expected_files.items():
    if hashlib.sha256((runtime / name).read_bytes()).hexdigest() != expected:
        raise SystemExit(f'ZPAQ runtime/source integrity mismatch: {name}')
print('Verified shipped ZPAQ helper and corresponding modified source hashes')
