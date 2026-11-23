"""Compare real extraction of ordinary ZIP/RAR controls in two qing-zip builds."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('checkout', 'baseline', 'candidate', 'sevenzip', 'receipt'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    cases_path = args.checkout.resolve(strict=True) / 'scripts/compare-common-extraction.py'
    spec = importlib.util.spec_from_file_location('archive_controls', cases_path)
    fixtures = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixtures)
    binaries = {'base': args.baseline.resolve(strict=True), 'candidate': args.candidate.resolve(strict=True)}
    receipt = {'windows': platform.version(), 'binaries': {k: sha(v) for k, v in binaries.items()}, 'cases': {}}
    with tempfile.TemporaryDirectory(prefix='qz-rar-ordinary-pairs-') as folder:
        root = Path(folder)
        names = {'sample.zip', 'compressed.rar', 'rar5-m3.rar', 'multivol_oldnaming_rar300.rar'}
        for volumes, expected in fixtures.cases(root, args.sevenzip.resolve(strict=True), False):
            name = volumes[0].name
            if name not in names:
                continue
            original = {v.name: sha(v) for v in volumes}
            rounds = []
            for i in range(8):
                pair = {}
                order = ['base','candidate'] if i % 2 == 0 else ['candidate','base']
                for role in order:
                    with tempfile.TemporaryDirectory(dir=root) as destination:
                        started = time.perf_counter_ns()
                        subprocess.run([binaries[role],'extract',volumes[0],destination], cwd=root,
                                       stdin=subprocess.DEVNULL, capture_output=True, check=True, timeout=60)
                        pair[role] = round((time.perf_counter_ns() - started) / 1e6, 3)
                        tree = {p.relative_to(destination).as_posix(): sha(p) for p in Path(destination).rglob('*') if p.is_file()}
                        if tree != expected or {v.name: sha(v) for v in volumes} != original:
                            raise RuntimeError('Content changed during ' + name)
                rounds.append(pair)
            measurements = rounds[1:]
            deltas = [p['candidate'] - p['base'] for p in measurements]
            receipt['cases'][name] = {'source_hashes': original, 'expected_files': expected,
                                       'warmup_ms': rounds[0], 'pairs_ms': measurements,
                                       'medians_ms': {k: statistics.median(i[k] for i in measurements) for k in binaries},
                                       'paired_delta_range_ms': [round(min(deltas),3), round(max(deltas),3)]}
        if set(receipt['cases']) != names:
            raise RuntimeError('Missing archive controls')
    with args.receipt.open('x', encoding='utf-8') as out:
        print(json.dumps(receipt, ensure_ascii=False, indent=2),file=out)


if __name__ == '__main__':
    main()
