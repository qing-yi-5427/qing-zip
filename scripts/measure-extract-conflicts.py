"""Compare two staged CLIs on the same ordinary ZIP extraction workloads."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import tempfile
import time
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--baseline", required=True, type=Path, help="Baseline obz.exe with its worker and runtimes")
parser.add_argument("--candidate", required=True, type=Path, help="Candidate obz.exe with its worker and runtimes")
args = parser.parse_args()
executables = {"base": args.baseline.resolve(strict=True), "fixed": args.candidate.resolve(strict=True)}

def check(condition, context):
    if not condition:
        raise RuntimeError(context)

def quartiles(samples):
    return [round(v, 2) for v in statistics.quantiles(samples, n=4, method='inclusive')]

with tempfile.TemporaryDirectory(prefix='qing-zip-m2-a-b-') as scratch:
    root = Path(scratch)
    env = dict(os.environ, LOCALAPPDATA=str(root / 'profile'), TEMP=str(root), TMP=str(root))
    scenarios = [('new-smart-zip', 512, 512), ('existing-rename-zip', 96, 96), ('selected-rename-zip', 512, 320)]
    payloads = []
    random_bytes = random.Random(23014)
    for i in range(scenarios[0][1]):
        payloads.append(random_bytes.randbytes(2048) + b'archive sample\n' * 128)
    for scenario, count, selected_count in scenarios:
        paths = [f'group-{i%8}/item-{i:03}.txt' for i in range(count)]
        source = root / (scenario + '.zip')
        with zipfile.ZipFile(source, 'w', zipfile.ZIP_DEFLATED) as archive:
            for path, payload in zip(paths, payloads):
                archive.writestr(path, payload)
        original = hashlib.sha256(source.read_bytes()).hexdigest()
        samples = {key: [] for key in executables}
        effect = []
        for pair in range(14):
            times = {}
            order = ('base', 'fixed') if pair % 2 == 0 else ('fixed', 'base')
            for label in order:
                original_dest = root / f'{scenario}-{pair}-{label}'
                actual_dest = original_dest / scenario if scenario == 'new-smart-zip' else original_dest
                original_dest.mkdir()
                options = {}
                if scenario != 'new-smart-zip':
                    options['overwrite'] = 'rename'
                    if selected_count != count:
                        options['selected'] = paths[:selected_count]
                    previous = actual_dest / paths[0]
                    previous.parent.mkdir()
                    previous.write_bytes(b'preexisting file')
                request = {
                    'operation': 'extract', 'path': str(source), 'dest': str(original_dest),
                    'smart': scenario == 'new-smart-zip', 'options': options,
                }
                begin = time.perf_counter()
                result = subprocess.run([executables[label], 'json'], input=json.dumps(request), capture_output=True, text=True, encoding='utf-8', env=env, cwd=root, timeout=120)
                elapsed = (time.perf_counter() - begin) * 1000
                check(result.returncode == 0, result.stderr)
                output = json.loads(result.stdout)
                check(output['files'] == selected_count and Path(output['destination']) == actual_dest, output)
                check(sum(1 for path in actual_dest.rglob('*') if path.is_file()) == selected_count + (scenario != 'new-smart-zip'), actual_dest)
                for i, name in enumerate(paths[:selected_count]):
                    if i == 0 and scenario != 'new-smart-zip':
                        check((actual_dest / name).read_bytes() == b'preexisting file', actual_dest)
                        name = 'group-0/item-000 (1).txt'
                    check((actual_dest / name).read_bytes() == payloads[i], name)
                if pair >= 2:
                    samples[label].append(elapsed)
                    times[label] = elapsed
            if times:
                effect.append(times['fixed'] - times['base'])
        check(hashlib.sha256(source.read_bytes()).hexdigest() == original, original)
        print(json.dumps(dict(scenario=scenario, entries=count, extracted=selected_count, archive_bytes=source.stat().st_size, pairs=len(effect),
            medians_ms={label: round(statistics.median(values), 2) for label, values in samples.items()},
            q1_median_q3_ms={label: quartiles(values) for label, values in samples.items()},
            paired_fixed_minus_base_ms=quartiles(effect), raw_samples_ms={label: [round(value, 2) for value in values] for label, values in samples.items()},
            verified_source_sha256=original)))
