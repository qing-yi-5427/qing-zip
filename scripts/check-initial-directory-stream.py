import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import zipfile

spec = importlib.util.spec_from_file_location('probe', Path(__file__).with_name('probe-native-acl-debug.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
exe, report = [Path(arg).resolve() for arg in sys.argv[1:]]
rows = []
for which in ('root', 'nested'):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        archive = root / 'test.zip'
        output = root / 'output'
        target = output if which == 'root' else output / 'nested'
        with zipfile.ZipFile(archive, 'w') as z:
            z.writestr('nested/file.txt', b'payload')
        def matches(name):
            return name.lower().removeprefix('\\\\?\\') == str(target).lower()
        def add_stream():
            Path(str(target) + ':outside').write_bytes(b'foreign')
        hit, status, log = probe.observe([exe, 'extract', archive, output], matches,
                                         add_stream, os.environ.copy(), 'NtQueryInformationFile')
        preserved = target.is_dir() and Path(str(target) + ':outside').read_bytes() == b'foreign'
        rows.append(dict(which=which, hit=hit, status=status, preserved=preserved, log=log))
        report.write_text(json.dumps(rows, indent=2))
        assert hit and status and preserved and 'partial destination retained' in log, rows[-1]
