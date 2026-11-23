"""Measure mixed-outcome ordinary ZIP batches in real private Windows application windows."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import random
import runpy
import statistics
import tempfile
import time
import zipfile

def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()

def clean(owner):
    for attempt in range(32):
        try:
            owner.cleanup()
            return
        except OSError:
            if attempt==31:raise
            time.sleep(.25)

def main():
    a=argparse.ArgumentParser()
    for key in ['scripts','baseline','candidate','fixtures-in','out']:
        a.add_argument('--'+key,type=Path,required=True)
    a.add_argument('--baseline-source',required=True)
    a.add_argument('--candidate-source',required=True)
    a.add_argument('--cpu-label',default=platform.processor(),help='CPU model to record for this machine')
    a.add_argument('--ram-gib',type=float,help='Installed RAM in GiB')
    a.add_argument('--pairs',type=int,default=5)
    a.add_argument('--case',choices=['small','medium','many','daily','all'],default='all')
    args=a.parse_args()
    h=runpy.run_path(str(args.scripts/'check-batch-extract-desktop.py'))
    gui=runpy.run_path(str(args.scripts/'check-nested-multi-layer.py'))
    exes={n:getattr(args,n).resolve(strict=True) for n in ['baseline','candidate']}
    manifest={name:{f:sha(exe.with_name(f)) for f in ('qing-zip.exe','obz.exe','obz-worker.exe')} for name,exe in exes.items()}
    assert h['snapshot'](exes['baseline'].parent/'runtime')==h['snapshot'](exes['candidate'].parent/'runtime')
    summary={'os':platform.platform(),'cpu':args.cpu_label,'ram_gib':args.ram_gib,'file_system':'local NTFS','baseline':args.baseline_source,
        'candidate':args.candidate_source,'binaries':manifest,'common_runtime_identical':True,
        'method':'Real trusted start click to first visible complete batch report; one intentional existing-file conflict for small/medium/many. Daily is all-success, click to report in DOM regardless of its visibility. Warm cache. One warm-up pair then alternating order. Per-run actual file and archive hash comparisons. Excludes startup and input generation.','cases':{}}
    owner=tempfile.TemporaryDirectory(prefix='public-batch-pair-',dir=args.fixtures_in)
    try:
        base=Path(owner.name)
        for key,n,size in [('small',3,1024),('medium',3,4*1024*1024),('many',24,1024),('daily',8,2*1024*1024)]:
            if args.case not in ('all',key):continue
            case=base/key;case.mkdir()
            paths=[]; expected={}; originals={}
            for i in range(n):
                name=f'archive-{i+1:02d}'
                arc=case/(name+'.zip')
                payload=random.Random(230400+i).randbytes(size)
                with zipfile.ZipFile(arc,'x') as z:
                    info=zipfile.ZipInfo('file.bin',(2024,6,1,10,0,0))
                    info.compress_type=zipfile.ZIP_DEFLATED
                    z.writestr(info,payload)
                paths.append(arc); originals[arc.name]=sha(arc)
                expected[f'{i+1}-{name}/file.bin']=hashlib.sha256(payload if key=='daily' or i!=n//2 else b'keep previously existing target').hexdigest()
            expected['neighbor.txt']=hashlib.sha256(b'keep neighboring output').hexdigest()
            rows=[]
            def measure(exe):
                own=tempfile.TemporaryDirectory(prefix='run-',dir=case)
                try:
                    root=Path(own.name);dest=root/'output';dest.mkdir()
                    (dest/'neighbor.txt').write_bytes(b'keep neighboring output')
                    blocked=dest/f'{n//2+1}-{paths[n//2].stem}'/'file.bin'
                    if key!='daily':
                        blocked.parent.mkdir();blocked.write_bytes(b'keep previously existing target')
                    with gui['private_process'](exe,root,None) as (page,process,_):
                        h['fill_batch'](page,paths,dest,gui['expect'])
                        final=f'成功 {n}' if key=='daily' else f'成功 {n-1} · 失败 1'
                        page.evaluate("""([summary, visible]) => {
                          document.querySelector('[data-action="batch-start"]').addEventListener('click',event=>{
                            if (!event.isTrusted) throw Error('Untrusted click');
                            const t=performance.now();
                            const watch=new MutationObserver(()=>{
                              const report=Array.from(document.querySelectorAll('.oa-batch-outcome')).find(n=>(!visible || n.getClientRects().length) && n.textContent.includes(summary));
                              if(report){window.batchElapsed=performance.now()-t; watch.disconnect();}
                            });
                            watch.observe(document.body,{childList:true,characterData:true,subtree:true});
                          },{once:true});
                        }""",[final,key!='daily'])
                        page.locator('[data-action="batch-start"]').click()
                        report=page.locator('.oa-batch-outcome' if key=='daily' else '.oa-batch-outcome:visible').filter(has_text=final).first
                        report.wait_for(state='attached' if key=='daily' else 'visible',timeout=25000)
                        elapsed=round(page.evaluate('window.batchElapsed'),2)
                        assert h['snapshot'](dest)==expected and {p.name:sha(p) for p in paths}==originals
                        page.locator('[data-window="close"]').click()
                        assert gui['win32event'].WaitForSingleObject(process,15000)==0 and gui['win32process'].GetExitCodeProcess(process)==0
                    return elapsed
                finally:clean(own)
            for i in range(args.pairs+1):
                order=['baseline','candidate'] if i%2==0 else ['candidate','baseline']
                rows.append({k:measure(exes[k]) for k in order})
            def stats(x):
                return {'median':round(statistics.median(x),2),'min':round(min(x),2),'max':round(max(x),2)}
            trials=rows[1:]
            result={'count':n,'member_bytes':size,'archive_hashes':originals,'expected_files':expected,'warmup':rows[0],'paired':trials,
                    'baseline_ms':stats([r['baseline'] for r in trials]),'candidate_ms':stats([r['candidate'] for r in trials]),
                    'paired_delta_ms':stats([r['candidate']-r['baseline'] for r in trials]),'outputs_verified_all':True,'intentional_conflict':key!='daily','report_must_be_visible':key!='daily'}
            summary['cases'][key]=result
            print(key,result['baseline_ms'],result['candidate_ms'],result['paired_delta_ms'],flush=True)
    finally:clean(owner)
    with args.out.open('x',encoding='utf8') as f:json.dump(summary,f,indent=2,ensure_ascii=False)
if __name__=='__main__':main()
