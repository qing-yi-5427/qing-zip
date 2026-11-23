"""Check actual archive methods and round trips using a portable obz.exe."""
import argparse, json, subprocess, tempfile, zipfile
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--exe', required=True, type=Path)
a = p.parse_args()
exe = str(a.exe.resolve())
def run(*args, secret=None):
    command = [exe, *map(str, args)] + (['--password-stdin'] if secret else [])
    result = subprocess.run(command, input=(secret+'\n') if secret else None, text=True, capture_output=True, check=True)
    return json.loads(result.stdout)

with tempfile.TemporaryDirectory(prefix='qz-store-cli-') as tmp:
    base = Path(tmp)
    source = base/'sample.txt'
    content = b'ordinary compressible fixture\n'*8000
    source.write_bytes(content)
    for fmt, method, level, expected in [('zip','Deflate',0,'Deflate'),('zip','Store',0,'Store'),('zip','Deflate',5,'Deflate'),('7z','Copy',0,'Copy'),('7z','LZMA2',5,'LZMA2')]:
        archive = base/f'{fmt}-{method}-{level}.{fmt}'
        run('create',archive,source,'--format',fmt,'--method',method,'--level',level)
        entries = run('list',archive)['entries']
        assert len(entries) == 1 and expected.lower() in entries[0]['method'].lower(), entries
        run('test',archive)
        output = base/f'out-{fmt}-{method}-{level}'
        run('extract',archive,output)
        assert (output/'sample.txt').read_bytes() == content
        if fmt == 'zip':
            with zipfile.ZipFile(archive) as z:
                assert z.read('sample.txt') == content
                assert z.getinfo('sample.txt').compress_type == (0 if method == 'Store' else 8)
    encrypted = base/'encrypted.7z'
    run('create',encrypted,source,'--format','7z','--method','Copy','--level',0,secret='qz-test-only')
    run('test',encrypted,secret='qz-test-only')
    output = base/'out-encrypted'
    run('extract',encrypted,output,secret='qz-test-only')
    assert (output/'sample.txt').read_bytes() == content
    print('PASS: explicit CLI methods retain their method-dependent level behavior. Store/Copy and compressed controls round-trip, including encrypted 7z.')
