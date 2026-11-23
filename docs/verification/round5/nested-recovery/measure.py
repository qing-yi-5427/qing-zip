"""Compare actual nested ZIP opening and F2 rename with fixed stored data."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import random
import runpy
import statistics
import tempfile
import time
import zipfile

from playwright.sync_api import expect


def fixture(root, size):
    payload = random.Random(86).randbytes(size)
    inner = io.BytesIO()
    with zipfile.ZipFile(inner, "w", zipfile.ZIP_STORED) as archive:
        archive.writestr("before.txt", b"stable editing sample")
        archive.writestr("bulk.bin", payload)
    outer = root / "outer.zip"
    with zipfile.ZipFile(outer, "w", zipfile.ZIP_STORED) as archive:
        archive.writestr("inner.zip", inner.getvalue())
        archive.writestr("outside.txt", b"unmodified outer archive member")
    return outer, hashlib.sha256(payload).hexdigest()


def begin_observation(page, settings):
    page.evaluate("""({eventName, startSelector, stopSelector}) => {
        window.qaDuration = null;
        const start = event => {
            if (!event.isTrusted || !event.target.closest(startSelector)) return;
            document.removeEventListener(eventName, start, true);
            const when = performance.now();
            const observer = new MutationObserver(() => {
                if (document.querySelector(stopSelector)) {
                    window.qaDuration = performance.now() - when;
                    observer.disconnect();
                }
            });
            observer.observe(document.body, {childList:true, subtree:true, attributes:true});
        };
        document.addEventListener(eventName, start, true);
    }""", settings)


def timing(page):
    result = page.evaluate("() => window.qaDuration")
    assert result > 0
    return round(result, 3)


def sample(harness, exe, source, payload_sha, output, renames, expected_digest):
    with tempfile.TemporaryDirectory(prefix="qing-zip-recovery-timing-", dir=output, ignore_cleanup_errors=True) as folder:
        root = Path(folder)
        original_sha = hashlib.sha256(source.read_bytes()).hexdigest()
        outer = root / source.name
        outer.write_bytes(source.read_bytes())
        with harness['private_process'](exe, root, outer) as (page, process, _):
            entry = page.locator('[data-entry="inner.zip"]')
            expect(entry).to_be_visible(timeout=60_000)
            before = page.locator('[data-entry="before.txt"]')
            begin_observation(page, {"eventName":"dblclick", "startSelector":'[data-entry="inner.zip"]', "stopSelector":'[data-entry="before.txt"]'})
            entry.dblclick()
            expect(before).to_be_visible(timeout=60_000)
            open_ms = timing(page)
            copies = harness['sessions'](page, root)
            working = Path(copies['inner.zip']['path'])
            recovery_record = root / 'local' / 'qing-zip' / 'EditedFiles' / 'index.json'
            versions = []
            for index in range(renames):
                new_name = f'after-{index}.txt'
                selector = f'[data-entry="{new_name}"]'
                renamed = page.locator(selector)
                before.click()
                page.keyboard.press('F2')
                page.locator('#qa-rename').fill(new_name)
                begin_observation(page, {"eventName":"click", "startSelector":'[data-action="rename-submit"]', "stopSelector":selector})
                page.locator('[data-action="rename-submit"]').click()
                expect(renamed).to_be_visible(timeout=60_000)
                rename_ms = timing(page)
                expect(page.locator('.oa-nested-bar')).to_contain_text('更改尚未写回')
                proof = None
                if recovery_record.exists():
                    records = json.loads(recovery_record.read_text(encoding='utf-8'))
                    if records:
                        assert len(records) == 1
                        assert Path(records[0]['path']).resolve() == working.resolve()
                        proof = records[0]['nested']['proof']
                if expected_digest:
                    assert proof and proof['committed'] and proof.get('digest', 'sha256') == expected_digest
                    if versions:
                        assert proof['committed'][1] != versions[-1]['proof_digest']
                else:
                    assert proof is None
                with zipfile.ZipFile(working) as archive:
                    assert sorted(archive.namelist()) == [new_name, 'bulk.bin']
                    assert archive.read(new_name) == b'stable editing sample'
                    assert hashlib.sha256(archive.read('bulk.bin')).hexdigest() == payload_sha
                versions.append({'duration_ms':rename_ms, 'name':new_name,
                                 'proof_digest':proof['committed'][1] if proof else None})
                before = renamed
            page.locator('[data-window="close"]').click()
            confirmation = page.locator('#qa-overlay:not([hidden])')
            expect(confirmation).to_contain_text('inner.zip')
            confirmation.locator('[data-action="nested-close-discard"]').click()
            events = harness['win32event']
            assert events.WaitForSingleObject(process, 10_000) == events.WAIT_OBJECT_0
        time.sleep(0.25)
        assert not working.exists()
        assert hashlib.sha256(outer.read_bytes()).hexdigest() == original_sha
        rename_total = sum(version['duration_ms'] for version in versions)
        return {'open_ms':open_ms, 'rename_ms':versions[0]['duration_ms'],
                'open_plus_first_rename_ms':round(open_ms + versions[0]['duration_ms'], 3),
                'rename_total_ms':round(rename_total, 3), 'versions':versions}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--driver', type=Path, required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--sha-recovery', type=Path)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--pairs', type=int, default=5)
    parser.add_argument('--fixture-mib', type=int, help='Run one size with #36 and the candidate only')
    args = parser.parse_args()
    if args.fixture_mib is None and args.sha_recovery is None:
        parser.error('--sha-recovery is required for the standard three-version groups')
    if args.fixture_mib is not None and args.fixture_mib < 1:
        parser.error('--fixture-mib must be positive')
    args.out.mkdir(parents=True, exist_ok=False)
    harness = runpy.run_path(str(args.driver))
    binaries = {'baseline':args.baseline.resolve(strict=True)}
    if args.fixture_mib is None:
        binaries['sha_recovery'] = args.sha_recovery.resolve(strict=True)
    binaries['candidate'] = args.candidate.resolve(strict=True)
    expected_digests = {'baseline':None, 'sha_recovery':'sha256', 'candidate':'blake3'}
    hashes = {label:{name:hashlib.sha256(exe.with_name(name).read_bytes()).hexdigest()
                    for name in ('qing-zip.exe','obz.exe','obz-worker.exe')} for label, exe in binaries.items()}
    assert all(len({spec[name] for spec in hashes.values()}) == 1 for name in ('obz.exe','obz-worker.exe'))
    report = {'status':'running','metric':'trusted production WebView action until the target entry is rendered',
              'binaries_sha256':hashes,'sample_groups':{}}
    receipt = args.out.resolve() / 'timing.json'

    def save():
        receipt.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')

    labels = tuple(binaries)
    if args.fixture_mib is None:
        groups = [('4', 4, 1), ('64', 64, 1), ('64-continuous', 64, 3)]
        orders = (labels, labels[::-1], labels[1:]+labels[:1], labels[:1]+labels[:0:-1],
                  labels[2:]+labels[:2], labels[1:2]+labels[:1]+labels[2:])
    else:
        groups = [(str(args.fixture_mib), args.fixture_mib, 1)]
        orders = (labels, labels[::-1])
    for name, size, renames in groups:
        group = {'stored_member_mib':size,'renames_per_open':renames,'samples':[], 'warmup':{}}
        report['sample_groups'][name] = group
        with tempfile.TemporaryDirectory(prefix='qing-zip-recovery-perf-input-', dir=args.out) as folder:
            source, payload_sha = fixture(Path(folder), size * 1024 * 1024)
            group['payload_sha256'] = payload_sha
            for iteration in range(args.pairs + 1):
                order = orders[iteration % len(orders)]
                pair = {label:sample(harness, binaries[label], source, payload_sha, args.out.resolve(), renames,
                                     expected_digests[label]) for label in order}
                if iteration == 0:
                    group['warmup'] = pair
                else:
                    group['samples'].append({'order':order, 'metrics':pair})
                summary = {label:[pair[label]['open_ms'], *[v['duration_ms'] for v in pair[label]['versions']]] for label in order}
                print(f'{name} MiB / {iteration}: {summary}', flush=True)
                save()
        group['summary'] = {}
        selectors = {metric:(lambda sample, key=metric: sample[key])
                     for metric in ('open_ms','rename_ms','open_plus_first_rename_ms','rename_total_ms')}
        if renames > 1:
            for index in range(renames):
                selectors[f'rename_{index+1}_ms'] = lambda sample, slot=index: sample['versions'][slot]['duration_ms']
        for metric, read in selectors.items():
            by_label = {label:[read(sample['metrics'][label]) for sample in group['samples']] for label in labels}
            for ref in (label for label in labels if label != 'candidate'):
                by_label[f'candidate_minus_{ref}'] = [a - b for a, b in zip(by_label['candidate'], by_label[ref], strict=True)]
            group['summary'][metric] = {label:{'median':statistics.median(values),'min':min(values),'max':max(values)}
                                        for label, values in by_label.items()}
        save()
    report['status'] = 'passed'
    save()


if __name__ == '__main__':
    main()
