"""Bounded paired ZIP/7z soak on ordinary files; no system configuration changes."""
import argparse, concurrent.futures, datetime, hashlib, json, os, random, shutil, subprocess, tempfile, threading, time
from pathlib import Path


def file_hash(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        while chunk:=f.read(1024*1024): h.update(chunk)
    return h.hexdigest()


def manifest(root):
    return {p.relative_to(root).as_posix():file_hash(p) for p in sorted(root.rglob('*')) if p.is_file()}


class Overlap:
    def __init__(self,origin): self.lock=threading.Lock();self.active={};self.witness=None;self.origin=origin
    def register(self,role,operation,process):
        with self.lock:
            self.active[role]=(operation,process)
            if len(self.active)==2 and all(item[1].poll() is None for item in self.active.values()):
                self.witness={'seconds':round(time.monotonic()-self.origin,4),
                              'operations':{key:value[0] for key,value in self.active.items()}}
    def unregister(self,role,process):
        with self.lock:
            if self.active.get(role,(None,None))[1] is process: self.active.pop(role)


def invoke(cli, root, scratch, *args, monitor=None, role=None):
    start=time.monotonic()
    env={**os.environ,'TEMP':str(scratch),'TMP':str(scratch)}
    process=subprocess.Popen([str(cli),*map(str,args)],cwd=root,env=env,stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,encoding='utf-8',errors='replace',
        creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
    try:
        if monitor: monitor.register(role,str(args[0]),process)
        try: stdout,stderr=process.communicate(timeout=90)
        except subprocess.TimeoutExpired:
            process.kill();process.communicate(timeout=10)
            raise RuntimeError(f'{args[0]} timed out after 90 seconds')
        if process.returncode:
            detail=(stderr or stdout)[-2000:].replace(str(root),'<owned-root>')
            raise RuntimeError(f'{args[0]} exit {process.returncode}: {detail}')
        return json.loads(stdout),round(time.monotonic()-start,4)
    finally:
        if monitor: monitor.unregister(role,process)


def create(cli,root,scratch,dest,source,fmt,monitor=None):
    method='Deflate' if fmt=='zip' else 'LZMA2'
    return invoke(cli,root,scratch,'create',dest,source,'--format',fmt,'--method',method,'--level','5','--threads','1',monitor=monitor,role=fmt)


def task(cli,root,scratch,source,expected,seed,seed_hash,round_number,fmt,barrier,origin,monitor):
    directory=root/f'round-{round_number:06d}'/fmt
    directory.mkdir(parents=True)
    result={'format':fmt,'ok':False}
    try:
        barrier.wait(timeout=30)
        result['start_seconds']=round(time.monotonic()-origin,4)
        times={}
        tested,times['seed_test']=invoke(cli,root,scratch,'test',seed,'--threads','1',monitor=monitor,role=fmt)
        if tested.get('valid') is not True: raise AssertionError('seed integrity failed')
        _,times['seed_extract']=invoke(cli,root,scratch,'extract',seed,directory/'seed-output','--threads','1',monitor=monitor,role=fmt)
        if manifest(directory/'seed-output')!=expected: raise AssertionError('seed extracted content mismatch')
        candidate=directory/f'输出.{fmt}'
        _,times['create']=create(cli,root,scratch,candidate,source,fmt,monitor)
        result['created_sha256']=file_hash(candidate)
        tested,times['created_test']=invoke(cli,root,scratch,'test',candidate,'--threads','1',monitor=monitor,role=fmt)
        if tested.get('valid') is not True: raise AssertionError('created archive integrity failed')
        _,times['created_extract']=invoke(cli,root,scratch,'extract',candidate,directory/'created-output','--threads','1',monitor=monitor,role=fmt)
        outputs=manifest(directory/'created-output')
        if outputs!=expected: raise AssertionError('created archive content mismatch')
        result.update({'ok':True,'source_sha256_by_relative_path':expected,
                       'extracted_sha256_by_relative_path':outputs,'seed_sha256':seed_hash,'operation_seconds':times})
    except Exception as error:
        result['error']=(f'{type(error).__name__}: {error}').replace(str(root),'<owned-root>')[:2500]
    finally:
        result['end_seconds']=round(time.monotonic()-origin,4)
        try:
            shutil.rmtree(directory)
            result['round_directory_cleaned']=not directory.exists()
        except Exception as error:
            result['ok']=False; result['cleanup_error']=str(error).replace(str(root),'<owned-root>')[:500]
    return result


def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--cli',required=True,type=Path);ap.add_argument('--out',required=True,type=Path)
    ap.add_argument('--duration-seconds',type=int,default=1800);ap.add_argument('--interval-seconds',type=float,default=0.5)
    args=ap.parse_args()
    if not 1<=args.duration_seconds<=3600 or not 0<=args.interval_seconds<=30: ap.error('duration must be 1–3600 seconds; interval 0–30 seconds')
    cli=args.cli.resolve(strict=True); out=args.out.resolve(); out.mkdir(parents=True,exist_ok=False)
    summary={'status':'starting','requested_seconds':args.duration_seconds,'interval_seconds':args.interval_seconds,
             'cli_sha256':file_hash(cli),'script_sha256':file_hash(Path(__file__)),'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
             'rounds':0,'failed_rounds':0,'tasks':0,'failed_tasks':0,'overlapping_rounds':0,'owned_root_cleaned':False}
    def save(): (out/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    save(); root=None; start=None; temporary=None
    with (out/'rounds.jsonl').open('x',encoding='utf-8',buffering=1) as log:
        try:
            temporary=tempfile.TemporaryDirectory(prefix='qz-ordinary-soak-');root=Path(temporary.name).resolve()
            scratch=root/'temp'; scratch.mkdir(); inputs=root/'输入'; source=inputs/'样本'; (source/'子目录').mkdir(parents=True)
            (source/'说明.txt').write_text('普通归档双任务测试\n'*800,encoding='utf-8')
            (source/'子目录'/'数据.bin').write_bytes(random.Random(2104).randbytes(1024*1024))
            expected=manifest(inputs); seeds={}
            for fmt in ('zip','7z'):
                seed=root/f'原包.{fmt}';create(cli,root,scratch,seed,source,fmt)
                seeds[fmt]=(seed,file_hash(seed))
            summary['source_sha256_by_relative_path']=expected
            summary['seed_sha256']={fmt:item[1] for fmt,item in seeds.items()};summary['status']='running';save()
            start=time.monotonic()
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
                while time.monotonic()-start<args.duration_seconds:
                    number=summary['rounds']+1;barrier=threading.Barrier(2);monitor=Overlap(start)
                    jobs=[executor.submit(task,cli,root,scratch,source,expected,*seeds[fmt],number,fmt,barrier,start,monitor) for fmt in ('zip','7z')]
                    results=[j.result() for j in jobs]
                    intact=manifest(inputs)==expected and all(file_hash(path)==digest for path,digest in seeds.values())
                    overlap=monitor.witness is not None
                    round_root=root/f'round-{number:06d}'
                    try: round_root.rmdir(); round_root_cleaned=True
                    except OSError: round_root_cleaned=False
                    record={'round':number,'tasks':results,'seed_and_source_unchanged':intact,'overlap':overlap,'overlap_witness':monitor.witness,
                            'round_root_cleaned':round_root_cleaned,'elapsed_seconds':round(time.monotonic()-start,4)}
                    valid=intact and overlap and round_root_cleaned and all(r['ok'] and r['round_directory_cleaned'] for r in results)
                    record['ok']=valid
                    log.write(json.dumps(record,ensure_ascii=False)+'\n');log.flush()
                    summary['rounds']+=1;summary['tasks']+=len(results);summary['failed_tasks']+=sum(not r['ok'] for r in results)
                    summary['failed_rounds']+=int(not valid)
                    summary['overlapping_rounds']+=int(overlap);summary['elapsed_seconds']=record['elapsed_seconds'];save()
                    if not valid:
                        raise RuntimeError('task, overlap, source, seed or cleanup check failed; inspect last round')
                    remaining=args.duration_seconds-(time.monotonic()-start)
                    if remaining>0: time.sleep(min(args.interval_seconds,remaining))
            summary['elapsed_seconds']=round(time.monotonic()-start,4)
            summary['status']='passed' if summary['elapsed_seconds']>=args.duration_seconds and summary['rounds']>0 else 'incomplete'
        except Exception as error:
            summary['status']='failed';summary['error']=(f'{type(error).__name__}: {error}').replace(str(root),'<owned-root>')[:2500] if root else str(error)
            if start is not None: summary['elapsed_seconds']=round(time.monotonic()-start,4)
        finally:
            if temporary:
                try: temporary.cleanup();summary['owned_root_cleaned']=not root.exists()
                except Exception as error: summary['status']='failed';summary['cleanup_error']=str(error).replace(str(root),'<owned-root>')[:500]
            summary['finished_utc']=datetime.datetime.now(datetime.timezone.utc).isoformat();save()
    # Windows hosted runners may expose a non-UTF-8 console; the UTF-8 receipt
    # above keeps readable names while stdout remains portable ASCII.
    print(json.dumps(summary,ensure_ascii=True),flush=True)
    if summary['status']!='passed' or not summary['owned_root_cleaned']: raise SystemExit(1)


if __name__=='__main__': main()
