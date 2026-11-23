"""Compare installed baseline and candidate on ordinary Windows ZIP Add inputs."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import struct
import subprocess
import tempfile
import time
import zipfile

NATIVE = b'OA_ZIP_APPEND status=encoding'
OLD_TIME = 1_690_000_000
NEW_TIME = 1_700_000_000


def sha(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def archive(path, items):
    with zipfile.ZipFile(path, 'w') as target:
        target.comment = b'ordinary original archive comment'
        for ordinal, (name, data, modified) in enumerate(items):
            member = zipfile.ZipInfo(name, time.localtime(modified)[:6])
            member.comment = b'ordinary existing item comment'
            member.extra = struct.pack('<HHI', 0xcafe, 4, ordinal)
            target.writestr(member, data, compress_type=zipfile.ZIP_DEFLATED)


def fixture(root, inputs):
    for name, data, modified in inputs:
        path = root/name
        if data is None:
            path.mkdir(parents=True, exist_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    for name, _, modified in inputs:
        os.utime(root/name, (OLD_TIME, modified))


def product(cli, root, paths, options):
    env = {**os.environ, 'TEMP': str(root/'scratch'), 'TMP': str(root/'scratch'), 'OPEN_ARCHIVE_PROFILE': '1'}
    if options is None:
        return subprocess.run([cli, 'add', 'editing.zip', *paths], cwd=root, env=env, capture_output=True, timeout=60)
    request = {'operation': 'add', 'path': 'editing.zip', 'inputs': paths, 'options': options}
    return subprocess.run([cli, 'json'], input=json.dumps(request).encode(), cwd=root, env=env, capture_output=True, timeout=60)


def compare(clis, root, name, old, files, paths, options=None, candidate_native=True):
    case = root/name
    case.mkdir()
    (case/'scratch').mkdir()
    before = case/'before.zip'
    archive(before, old)
    sources = {relative: hashlib.sha256(data).hexdigest() for relative, data, _ in files if data is not None}
    rows = {}
    for side, cli in clis.items():
        fixture(case, files)
        editing = case/'editing.zip'
        shutil.copyfile(before, editing)
        proc = product(cli, case, paths, options)
        has_native = NATIVE in proc.stderr
        assert has_native == (side == 'candidate' and candidate_native), (name, side, proc.stderr.decode(errors='replace'))
        assert not list(case.glob('.qing-zip-stage-*')), (name, side)
        assert all(sha(case/relative) == digest for relative, digest in sources.items())
        rows[side] = {'exit': proc.returncode, 'seekable_worker': has_native, 'sha256': sha(editing)}
        if proc.returncode == 0:
            with zipfile.ZipFile(editing) as checked:
                assert checked.testzip() is None
                rows[side]['members'] = [(x.filename, x.compress_type, x.CRC, x.date_time) for x in checked.infolist()]
        else:
            assert rows[side]['sha256'] == sha(before), (name, side, proc.stderr.decode(errors='replace'))
    assert rows['baseline']['exit'] == rows['candidate']['exit'], name
    assert rows['baseline']['sha256'] == rows['candidate']['sha256'], (name, rows)
    return {'case': name, 'sources_unchanged': True, 'byte_equal': True, **rows}


def concurrent(cli, root, same_member=False):
    label = 'concurrent-same-member' if same_member else 'concurrent-distinct-members'
    case = root/label
    case.mkdir()
    (case/'scratch').mkdir()
    archive(case/'editing.zip', [('original.txt', b'original', OLD_TIME)])
    names = (r'first\document.txt', r'second\document.txt') if same_member else ('first.txt', 'second.txt')
    for name in names:
        (case/name).parent.mkdir(exist_ok=True)
        (case/name).write_bytes(name.encode())
    env = {**os.environ, 'TEMP': str(case/'scratch'), 'TMP': str(case/'scratch'), 'OPEN_ARCHIVE_PROFILE': '1'}
    processes = [subprocess.Popen([cli, 'add', 'editing.zip', name], cwd=case, env=env,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE) for name in names]
    try:
        for proc in processes:
            _, diagnostic = proc.communicate(timeout=60)
            assert proc.returncode == 0 and NATIVE in diagnostic, diagnostic.decode(errors='replace')
    finally:
        for proc in processes:
            if proc.poll() is None:
                proc.kill()
                proc.wait()
    with zipfile.ZipFile(case/'editing.zip') as result:
        assert result.read('original.txt') == b'original'
        if same_member:
            assert sorted(result.namelist()) == ['document.txt', 'original.txt']
            assert result.read('document.txt') in [name.encode() for name in names]
        else:
            assert sorted(result.namelist()) == ['first.txt', 'original.txt', 'second.txt']
            assert all(result.read(name) == name.encode() for name in names)
    assert all((case/name).read_bytes() == name.encode() for name in names)
    assert not list(case.glob('.qing-zip-stage-*'))
    return {'case': label, 'both_updates_succeeded_and_members_verified': True}


def fallback(cli, root, label):
    case = root/label
    case.mkdir()
    (case/'scratch').mkdir()
    archive(case/'editing.zip', [('original.txt', b'original member', OLD_TIME)])
    source = case/'additional.txt'
    source.write_bytes(b'ordinary input for named fallback')
    proc = product(cli, case, ['additional.txt'], None)
    assert proc.returncode == 0 and NATIVE not in proc.stderr, proc.stderr.decode(errors='replace')
    with zipfile.ZipFile(case/'editing.zip') as updated:
        assert sorted(updated.namelist()) == ['additional.txt', 'original.txt']
        assert updated.read('original.txt') == b'original member'
        assert updated.read('additional.txt') == source.read_bytes()
    assert not list(case.glob('.qing-zip-stage-*'))
    return {'case': label, 'cli_sha256': sha(cli), 'named_fallback_verified': True}


def occupied_sources(clis, root):
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.CreateFileW.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                                   wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE)
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = (wintypes.HANDLE,)
    case = root/'occupied'
    case.mkdir()
    (case/'scratch').mkdir()
    target = case/'editing.zip'
    archive(target, [('keep.txt', b'original', OLD_TIME)])
    source = case/'folder'/'waiting.txt'
    source.parent.mkdir()
    source.write_bytes(b'original input')
    original_sha = sha(target)
    results = []
    handle = kernel.CreateFileW(str(source), 0x80000000, 0, None, 3, 0x80, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        for logical in (r'folder\waiting.txt', 'folder'):
            for role, cli in clis.items():
                proc = product(cli, case, [logical], None)
                assert proc.returncode and proc.stderr, (role, logical)
                assert sha(target) == original_sha
                assert not list(case.glob('.qing-zip-stage-*'))
                results.append({'role': role, 'source_kind': 'folder' if logical == 'folder' else 'file',
                                'exit': proc.returncode, 'seekable_worker': NATIVE in proc.stderr})
    finally:
        kernel.CloseHandle(handle)
    assert source.read_bytes() == b'original input'
    retry = product(clis['candidate'], case, ['folder'], None)
    assert retry.returncode == 0 and NATIVE in retry.stderr, retry.stderr.decode(errors='replace')
    with zipfile.ZipFile(target) as updated:
        assert updated.read('keep.txt') == b'original' and updated.read('folder/waiting.txt') == b'original input'
    return {'case': 'occupied ordinary source', 'results': results, 'retry_verified': True}


def classic_count_limit(clis, root):
    case = root/'classic-count-limit'
    case.mkdir()
    (case/'scratch').mkdir()
    before = case/'before.zip'
    count = 65_535
    with zipfile.ZipFile(before, 'w', allowZip64=False) as initial:
        for index in range(count):
            initial.writestr(zipfile.ZipInfo(f'entry-{index:05d}.txt'), b'ordinary member')
    original = sha(before)
    source = case/'new.txt'
    source.write_bytes(b'addition')
    os.utime(source, (OLD_TIME, NEW_TIME))
    sha256 = {}
    for side, cli in clis.items():
        editing = case/'editing.zip'
        shutil.copyfile(before, editing)
        proc = product(cli, case, ['new.txt'], None)
        assert proc.returncode == 0 and NATIVE not in proc.stderr, proc.stderr.decode(errors='replace')
        assert not list(case.glob('.qing-zip-stage-*'))
        with zipfile.ZipFile(editing) as after:
            assert len(after.infolist()) == count + 1 and after.testzip() is None
            assert after.read('new.txt') == b'addition'
            assert all(after.read(f'entry-{index:05d}.txt') == b'ordinary member' for index in range(count))
        sha256[side] = sha(editing)
    assert sha(before) == original and source.read_bytes() == b'addition'
    assert sha256['baseline'] == sha256['candidate']
    return {'case': 'classic-65535-named-fallback', 'original_entries': count,
            'verified_entries': count + 1, 'seekable_worker': False, 'output_sha256': sha256}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', required=True, type=Path)
    parser.add_argument('--candidate', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--legacy-worker-bundle', type=Path, help='candidate obz.exe alongside a legacy worker')
    parser.add_argument('--backup-engine-bundle', type=Path, help='candidate obz.exe in a standalone 7-Zip bundle')
    parser.add_argument('--classic-count-limit', action='store_true', help='also verify an ordinary ZIP at 65,535 entries')
    args = parser.parse_args()
    clis = {k: getattr(args, k).resolve(strict=True) for k in ('baseline', 'candidate')}
    same_name = [('same.txt', b'old!', NEW_TIME), ('older.txt', b'previous later data', NEW_TIME),
                 ('newer.txt', b'previous earlier data', OLD_TIME), ('missing.txt', b'keep outside add', OLD_TIME),
                 ('folder/', b'', OLD_TIME), ('folder/old.txt', b'keep under selected directory', OLD_TIME)]
    incoming = [('same.txt', b'new!', NEW_TIME), ('older.txt', b'new even if older', OLD_TIME),
                ('newer.txt', b'new because newer', NEW_TIME), ('folder', None, NEW_TIME),
                ('folder/inner', None, NEW_TIME), ('folder/inner/empty.bin', b'', NEW_TIME),
                ('folder/emptydir', None, NEW_TIME), ('folder/资料.txt', '合成内容'.encode(), NEW_TIME)]
    with tempfile.TemporaryDirectory(prefix='qing-zip-add-interop-') as name:
        root = Path(name)
        rows = [compare(clis, root, 'mixed-default-add', same_name, incoming,
                        ['folder', 'newer.txt', 'older.txt', 'same.txt'])]
        rows.append(compare(clis, root, 'empty-file', [], [('empty.bin', b'', NEW_TIME)], ['empty.bin']))
        rows.append(compare(clis, root, 'incompressible-store', [('older.txt', b'old', OLD_TIME)],
                            [('data.bin', random.Random(177).randbytes(16*1024*1024), NEW_TIME)], ['data.bin']))
        rows.append(compare(clis, root, 'case-insensitive-replacement', [('Report.TXT', b'old', OLD_TIME)],
                            [('report.txt', b'new', NEW_TIME)], ['report.txt']))
        for mode in ('update', 'fresh', 'sync'):
            rows.append(compare(clis, root, f'{mode}-named-fallback', same_name, incoming,
                                ['folder','newer.txt','older.txt','same.txt'], {'update_mode': mode}, False))
        rows.append(compare(clis, root, 'explicit-codepage-fallback', [('keep.txt', b'prior', OLD_TIME)],
                            [('input.txt', b'addition', NEW_TIME)], ['input.txt'], {'codepage': 936}, False))
        rows.append(compare(clis, root, 'relative-forward-slash-fallback', [('keep.txt', b'prior', OLD_TIME)],
                            [('folder/input.txt', b'addition', NEW_TIME)], ['folder/input.txt'], candidate_native=False))
        rows.append(compare(clis, root, 'file-directory-conflict-fallback', [('folder', b'original', OLD_TIME)],
                            [('folder', None, NEW_TIME), ('folder/child.txt', b'new', NEW_TIME)], ['folder'], candidate_native=False))
        long_names = [f'{i:03d}-' + 'long-filename-'*10 + '.txt' for i in range(100)]
        rows.append(compare(clis, root, 'long-ordinary-list', [('keep.txt', b'prior', OLD_TIME)],
                            [(name, b'file', NEW_TIME) for name in long_names], long_names))
        rows.append(concurrent(clis['candidate'], root))
        rows.append(concurrent(clis['candidate'], root, same_member=True))
        rows.append(occupied_sources(clis, root))
        if args.classic_count_limit:
            rows.append(classic_count_limit(clis, root))
        for label, cli in [('legacy-worker', args.legacy_worker_bundle), ('backup-engine', args.backup_engine_bundle)]:
            if cli is not None:
                rows.append(fallback(cli.resolve(strict=True), root, label))
    with args.out.open('x', encoding='utf-8') as output:
        json.dump({'cli_sha256': {side: sha(cli) for side, cli in clis.items()}, 'results': rows}, output,
                  ensure_ascii=False, indent=2)


if __name__ == '__main__':
    main()
