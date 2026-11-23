"""Ordinary local NanaZip compatibility probe for a handle-owned ZIP update."""
import argparse
from contextlib import ExitStack
import ctypes
from ctypes import wintypes
import datetime
import hashlib
import json
import msvcrt
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import struct
import subprocess
import tempfile
import time
import zipfile


kernel = ctypes.WinDLL('kernel32', use_last_error=True)
kernel.CreateFileW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                               wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE)
kernel.CreateFileW.restype = wintypes.HANDLE
kernel.SetFileInformationByHandle.argtypes = (wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD)
kernel.SetFileInformationByHandle.restype = wintypes.BOOL


def pending(file, marked):
    flag = ctypes.c_ubyte(marked)
    if not kernel.SetFileInformationByHandle(msvcrt.get_osfhandle(file.fileno()), 4, ctypes.byref(flag), 1):
        raise ctypes.WinError(ctypes.get_last_error())


def owned_file(path):
    handle = kernel.CreateFileW(str(path), 0xC0010000, 0, None, 1, 0, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    return os.fdopen(msvcrt.open_osfhandle(handle, os.O_RDWR | os.O_BINARY), 'rb+', buffering=0)


class CP936(zipfile.ZipInfo):
    def _encodeFilenameFlags(self):
        return self.filename.encode('gbk'), self.flag_bits


def fixture(path, member_type=zipfile.ZipInfo):
    member = member_type('资料/文件.txt', (2023, 2, 3, 4, 5, 6))
    member.comment = b'ordinary item comment'
    member.extra = struct.pack('<HH8s', 0xcafe, 8, b'user foo')
    with zipfile.ZipFile(path, 'w') as archive:
        archive.comment = b'ordinary archive comment'
        archive.writestr(member, b'ordinary original text')
        archive.writestr('peer.txt', b'existing peer')


def manifest(path, encoding=None):
    with zipfile.ZipFile(path, metadata_encoding=encoding) as archive:
        assert archive.testzip() is None
        return {'comment': archive.comment.hex(), 'members': {info.filename: {
            'sha256': hashlib.sha256(archive.read(info)).hexdigest(), 'comment': info.comment.hex(),
            'extra': info.extra.hex(), 'method': info.compress_type, 'date': list(info.date_time),
            'external_attr': info.external_attr, 'flags': info.flag_bits, 'crc32': info.CRC,
            'size': info.file_size, 'packed': info.compress_size,
        } for info in archive.infolist()}}


def verify_dotnet(path, entries):
    script = r'''
$ErrorActionPreference = 'Stop'
$zip = [IO.Compression.ZipFile]::OpenRead($env:QING_ZIP_INTEROP_FIXTURE)
try {
    $rows = foreach ($item in $zip.Entries) {
        $stream = $item.Open()
        try {
            @{
                name = $item.FullName
                crc32 = $item.Crc32
                date = $item.LastWriteTime.ToString('yyyy-MM-dd HH:mm:ss')
                sha256 = (Get-FileHash -InputStream $stream -Algorithm SHA256).Hash.ToLowerInvariant()
                size = $item.Length
            }
        } finally { $stream.Dispose() }
    }
    ConvertTo-Json -InputObject @($rows) -Compress
} finally { $zip.Dispose() }
'''
    result = subprocess.run(['pwsh.exe', '-NoProfile', '-NonInteractive', '-Command', script],
                            env={**os.environ, 'QING_ZIP_INTEROP_FIXTURE': str(path)},
                            capture_output=True, check=True, timeout=30)
    expected = [{'name': name, 'crc32': row['crc32'], 'date': str(datetime.datetime(*row['date'])),
                 'sha256': row['sha256'], 'size': row['size']} for name, row in entries.items()]
    rows = json.loads(result.stdout)
    assert sorted(rows, key=lambda row: row['name']) == sorted(expected, key=lambda row: row['name'])
    return True


def zip_slt(reader, path, password):
    proc = subprocess.run([str(reader), 'l', '-slt', '-p'+password, str(path)], capture_output=True, check=True, timeout=20)
    text = proc.stdout.decode('utf-8', errors='replace')
    return [line for line in text.splitlines() if line.startswith(('Method = ','Encrypted = '))]


def command(exe, op, source, target, names, stream, switches):
    args = [str(exe), op, '-y', '-sccUTF-8', '-spd', '-bsp2', *switches, '-u-', '-u!'+str(target)]
    if stream:
        args.extend(['-so', '-bso2'])
    return args + ['--', str(source), *map(str, names)]


def trial(exe, op, source, target, names, stream, switches=()):
    initial = time.perf_counter()
    if stream:
        with owned_file(target) as output:
            pending(output, True)
            process = subprocess.run(command(exe, op, source, target, names, True, switches),
                                     cwd=source.parent, stdout=output, stderr=subprocess.PIPE, timeout=30)
            bytes_at_exit = os.fstat(output.fileno()).st_size
            if process.returncode == 0:
                pending(output, False)
    else:
        process = subprocess.run(command(exe, op, source, target, names, False, switches),
                                 cwd=source.parent, capture_output=True, timeout=30)
        bytes_at_exit = target.stat().st_size if target.exists() else 0
    elapsed = round((time.perf_counter() - initial) * 1000, 3)
    magic = None
    if target.exists():
        with target.open('rb') as result:
            magic = result.read(4).hex()
    return {'exit': process.returncode, 'ms': elapsed,
            'staged_bytes_at_exit': bytes_at_exit, 'staged_exists_after_close': target.exists(),
            'zip_magic_at_zero': magic,
            'diagnostic': process.stderr.decode('utf-8','replace').replace(str(source.parent),'<fixture>').strip()[:1200]}


def product_rename(runners, source, original_member):
    original = source.read_bytes()
    rows = []
    for index in range(6):
        order = ('baseline', 'candidate') if index % 2 == 0 else ('candidate', 'baseline')
        row = {'warmup': index == 0, 'order': order}
        outputs = {}
        for side in order:
            target = source.with_name('product-edit.zip')
            shutil.copyfile(source, target)
            row[side] = runners[side](target, original_member)
            outputs[side] = manifest(target)
            target.unlink()
        assert outputs['baseline'] == outputs['candidate']
        assert 'renamed.bin' in outputs['candidate']['members']
        assert original_member not in outputs['candidate']['members']
        assert source.read_bytes() == original
        row['delta_ms'] = round(row['candidate'] - row['baseline'], 3)
        rows.append(row)
    values = [row['delta_ms'] for row in rows[1:]]
    return {'pairs': rows, 'delta_ms': {'min': min(values), 'median': statistics.median(values), 'max': max(values)}}


def cli_runner(cli):
    def rename(target, member):
        env = {**os.environ, 'TEMP': str(target.parent), 'TMP': str(target.parent)}
        started = time.perf_counter()
        subprocess.run([str(cli), 'rename', str(target), member, 'renamed.bin'],
                       cwd=target.parent, env=env, capture_output=True, check=True, timeout=30)
        return round((time.perf_counter() - started) * 1000, 3)
    return rename


def same_process_runner(exe, root, cleanup):
    env = {**os.environ, 'TEMP': str(root), 'TMP': str(root)}
    process = cleanup.enter_context(subprocess.Popen([str(exe)], cwd=root, env=env,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, encoding='utf-8'))
    cleanup.callback(process.stdin.close)

    def rename(target, _member):
        process.stdin.write(str(target) + '\n')
        process.stdin.flush()
        return float(process.stdout.readline().strip())
    return rename


def exercise(exe, reader, root, receipt, product_runners):
    addition = root/'ordinary-add.txt'
    addition.write_bytes(b'additional plain text' * 9)
    plain, legacy, encrypted = map(lambda name: root/name, ('plain.zip', 'legacy.zip', 'encrypted.zip'))
    fixture(plain)
    fixture(legacy, CP936)
    old = root/'original-encrypted.txt'
    old.write_bytes(b'original encrypted contents')
    password = 'ordinary-test-only'; secrets = ('-p'+password, '-mem=AES256')
    subprocess.run([str(exe),'a','-tzip',*secrets,str(encrypted),str(old)],cwd=root,capture_output=True,check=True,timeout=20)
    for label, op, source, names, opts, encoding in (
        ('add', 'a', plain, [addition], (), None),
        ('delete', 'd', plain, ['peer.txt'], (), None),
        ('rename', 'rn', plain, ['资料/文件.txt','资料/新名字.txt'], (), None),
        ('cp936', 'a', legacy, [addition], ('-mcp=936',), 'gbk'),
        ('aes_update', 'a', encrypted, [addition], secrets, None),
        ('missing_input','a',plain,[root/'missing-ordinary.txt'],(),None),
    ):
        original = source.read_bytes()
        results = receipt['compatibility'][label] = {}
        for side in ('named','stdout'):
            target = root/f'{label}-{side}.zip'
            result = results[side] = trial(exe, op, source, target, names, side=='stdout', opts)
            assert source.read_bytes() == original
            if result['exit'] or not target.is_file():
                continue
            if label == 'aes_update':
                test = subprocess.run([str(reader),'t','-p'+password,str(target)],cwd=root,capture_output=True,timeout=20)
                result['independent_integrity'] = test.returncode == 0
                if test.returncode == 0:
                    extracted = root / f'contents-{label}-{side}'
                    subprocess.run([str(reader),'x','-p'+password,str(target),'-y','-o'+str(extracted)],cwd=root,capture_output=True,check=True,timeout=20)
                    result['member_hashes'] = {f.name:hashlib.sha256(f.read_bytes()).hexdigest() for f in extracted.iterdir()}
                result['secure_methods'] = zip_slt(reader,target,password)
            else:
                result['proof'] = manifest(target, encoding)
        before, after = results['named'], results['stdout']
        fields = ('proof','independent_integrity','member_hashes','secure_methods')
        results['same_exit_and_content_or_metadata'] = before['exit']==after['exit'] and all(before.get(field)==after.get(field) for field in fields)
        if label in ('add', 'cp936'):
            left, right = (side['proof'] for side in (before, after))
            assert left['comment'] == right['comment']
            assert left['members'].keys() == right['members'].keys()
            for name, member in left['members'].items():
                other = right['members'][name]
                if name == addition.name:
                    assert all(member[k] == other[k] for k in ('sha256', 'crc32', 'date', 'size'))
                else:
                    assert member == other
        else:
            assert results['same_exit_and_content_or_metadata'], label
        if label == 'missing_input':
            assert before['exit'] != 0 and after['staged_bytes_at_exit'] > 0 and not after['staged_exists_after_close']
        else:
            assert before['exit'] == after['exit'] == 0
            assert before['zip_magic_at_zero'] == after['zip_magic_at_zero'] == '504b0304'

    for size in (1024, 16*1024*1024):
        data = random.Random(size).randbytes(size)
        addition.write_bytes(data)
        single = size == 1024
        archive_to_add = root / f'add-before-{size}.zip'
        archive_to_rename = root / f'rename-before-{size}.zip'
        for archive, write_subject in ((archive_to_add, False), (archive_to_rename, True)):
            with zipfile.ZipFile(archive, 'w') as target:
                if not single:
                    target.writestr('peer.txt', b'ordinary existing neighbor', compress_type=zipfile.ZIP_DEFLATED)
                if write_subject:
                    target.writestr(addition.name, data, compress_type=zipfile.ZIP_STORED)
        for key, runners in product_runners.items():
            receipt[key][str(size)] = product_rename(runners, archive_to_rename, addition.name)
        for op, archive, names, member in (
            ('a', archive_to_add, [addition], addition.name),
            ('rn', archive_to_rename, [addition.name, 'renamed.bin'], 'renamed.bin'),
        ):
            rows = []
            for i in range(6):
                row = {'warmup': i == 0, 'order': ['named','stdout'] if i % 2 == 0 else ['stdout','named']}
                for side in row['order']:
                    target = root/f'perf-{side}.zip'
                    result = trial(exe, op, archive, target, names, side == 'stdout')
                    assert result['exit'] == 0 and result['zip_magic_at_zero'] == '504b0304'
                    proof = manifest(target)
                    entries = proof['members']
                    assert entries[member]['sha256'] == hashlib.sha256(data).hexdigest()
                    with target.open('rb') as stream:
                        target_sha = hashlib.file_digest(stream, 'sha256').hexdigest()
                    row[side] = {field: result[field] for field in ('ms', 'staged_bytes_at_exit')}
                    row[side]['sha256'] = target_sha
                    if i == 0:
                        row[side]['entries'] = entries
                        row[side]['dotnet_crc_time_content_match'] = verify_dotnet(target, entries)
                    target.unlink()
                if op == 'rn':
                    assert row['named']['sha256'] == row['stdout']['sha256']
                row['delta_ms'] = round(row['stdout']['ms'] - row['named']['ms'], 3)
                rows.append(row)
            values = [row['delta_ms'] for row in rows[1:]]
            receipt['timing'][f'{op}-{size}'] = {'pairs':rows, 'delta_ms':{'min':min(values),'median':statistics.median(values),'max':max(values)}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nana',required=True,type=Path)
    parser.add_argument('--reader',required=True,type=Path)
    parser.add_argument('--out',required=True,type=Path)
    parser.add_argument('--baseline-cli', type=Path)
    parser.add_argument('--candidate-cli', type=Path)
    parser.add_argument('--baseline-process', type=Path)
    parser.add_argument('--candidate-process', type=Path)
    args=parser.parse_args()
    if bool(args.baseline_cli) != bool(args.candidate_cli):
        parser.error('--baseline-cli and --candidate-cli must be supplied together')
    if bool(args.baseline_process) != bool(args.candidate_process):
        parser.error('--baseline-process and --candidate-process must be supplied together')
    receipt={'exe_sha256':hashlib.sha256(args.nana.read_bytes()).hexdigest(), 'system':platform.platform(),
             'compatibility':{},'timing':{},'time_unit':'ms', 'scope':'engine process plus candidate handle set/clear; no publication'}
    product_runners = {}
    if args.baseline_cli:
        clis = {'baseline': args.baseline_cli.resolve(strict=True), 'candidate': args.candidate_cli.resolve(strict=True)}
        receipt['cli_sha256'] = {side: hashlib.sha256(path.read_bytes()).hexdigest() for side, path in clis.items()}
        receipt['complete_cli_timing'] = {}
        receipt['complete_cli_scope'] = 'fresh process to exit, warm filesystem cache, includes verification and commit'
        product_runners['complete_cli_timing'] = {side: cli_runner(cli) for side, cli in clis.items()}
    with tempfile.TemporaryDirectory(prefix='qing-zip-stdout-probe-') as name, ExitStack() as cleanup:
        root = Path(name)
        if args.baseline_process:
            processes = {'baseline': args.baseline_process.resolve(strict=True), 'candidate': args.candidate_process.resolve(strict=True)}
            receipt['process_sha256'] = {side: hashlib.sha256(path.read_bytes()).hexdigest() for side, path in processes.items()}
            receipt['same_process_timing'] = {}
            receipt['same_process_scope'] = 'engine::update entry to return, persistent process per version, warm filesystem cache'
            product_runners['same_process_timing'] = {side: same_process_runner(exe, root, cleanup) for side, exe in processes.items()}
        held = root/'closed-without-commit.bin'
        with owned_file(held) as file:
            pending(file, True)
            file.write(b'ordinary scratch bytes')
        assert not held.exists()
        receipt['ordinary_pending_close_reclaimed_file'] = True
        exercise(args.nana,args.reader,root,receipt,product_runners)
    receipt['owned_root_cleaned']=not root.exists()
    with args.out.open('x',encoding='utf-8') as output:
        json.dump(receipt,output,ensure_ascii=False,indent=2)


if __name__=='__main__':
    main()
