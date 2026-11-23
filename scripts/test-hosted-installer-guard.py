"""Ensure the installer harness refuses a non-hosted environment before I/O."""
import os, subprocess, sys, tempfile
from pathlib import Path

script=Path(__file__).with_name('check-nsis-upgrade-hosted.py')
env=dict(os.environ)
for key in ('GITHUB_ACTIONS','RUNNER_ENVIRONMENT','RUNNER_OS','RUNNER_ARCH','RUNNER_TEMP'): env.pop(key,None)
with tempfile.TemporaryDirectory(prefix='qz-installer-guard-') as root:
    out=Path(root)/'must-not-be-created'
    args=[sys.executable,str(script)]
    for name in ('old-setup','old-portable','candidate-setup','candidate-stage'):
        args.extend(['--'+name,str(Path(root)/'nonexistent')])
    args.extend(['--out',str(out)])
    for arch in ('X64','ARM64'):
        for location_options in ([], ['--use-default-location']):
            result=subprocess.run([*args,'--arch',arch,*location_options],env=env,capture_output=True,text=True,timeout=15)
            assert result.returncode and 'requires a disposable GitHub-hosted' in result.stderr and not out.exists(),result.stderr
        print(f'PASS: {arch} rejected both location modes before resolving binaries or writing evidence')
