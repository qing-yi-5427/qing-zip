"""Stage only the reviewed ARM64 NanaZip files after checking the official archive."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import zipfile

MANIFEST = Path(__file__).resolve().parents[1] / 'obz-rs/native/nana/provenance-arm64.json'
FILES = {'NanaZip.Universal.Console.exe', 'NanaZip.Core.dll', 'NanaZip.Codecs.dll',
         'NanaZip.ZS.Codecs.dll', 'K7Base.dll', 'K7User.dll'}


def is_arm64_pe(data):
    if len(data) < 64 or data[:2] != b'MZ':
        return False
    pos = struct.unpack_from('<I', data, 60)[0]
    return (pos + 26 <= len(data) and data[pos:pos+4] == b'PE\0\0'
            and struct.unpack_from('<H', data, pos+4)[0] == 0xaa64
            and struct.unpack_from('<H', data, pos+24)[0] == 0x20b)


def stage(package, directory):
    manifest = json.loads(MANIFEST.read_text(encoding='utf-8'))
    if manifest['architecture'] != 'arm64' or set(manifest['files']) != FILES:
        raise ValueError('incorrect ARM64 runtime manifest')
    with package.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != manifest['package_sha256']:
            raise ValueError('upstream package SHA-256 mismatch')
        with zipfile.ZipFile(source) as archive:
            payloads = {}
            for name, digest in manifest['files'].items():
                member = 'arm64/' + name
                if archive.namelist().count(member) != 1:
                    raise ValueError(f'missing or duplicated upstream member: {member}')
                data = archive.read(member)
                if hashlib.sha256(data).hexdigest() != digest or not is_arm64_pe(data):
                    raise ValueError(f'invalid ARM64 component: {name}')
                payloads[name] = data
    directory.mkdir(parents=True, exist_ok=False)
    for name, data in payloads.items():
        (directory / name).write_bytes(data)
        print(f'ARM64 {name} {manifest["files"][name]}')
    print('Verified official package SHA-256:', manifest['package_sha256'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    stage(args.package, args.directory)
