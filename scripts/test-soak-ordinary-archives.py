"""Verify setup failure still emits a receipt and cleans the owned root."""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

script=Path(__file__).with_name('soak-ordinary-archives.py')
with tempfile.TemporaryDirectory(prefix='qz-soak-receipt-test-') as work:
    out=Path(work)/'evidence'
    run=subprocess.run([sys.executable,str(script),'--cli',sys.executable,'--out',str(out),
                        '--duration-seconds','1'],capture_output=True,text=True,timeout=30)
    assert run.returncode!=0
    summary=json.loads((out/'summary.json').read_text(encoding='utf-8'))
    assert summary['status']=='failed' and summary['owned_root_cleaned'] and summary['rounds']==0,summary
    assert (out/'rounds.jsonl').exists()
    print('PASS: setup failure preserved summary and cleaned the owned root')
