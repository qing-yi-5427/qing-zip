"""OpenBandiZip ACE codec worker. Never execute or open archive members as programs."""
import json, sys, pathlib, os
import acefile
MAX_ENTRY=512*1024*1024
MAX_TOTAL=2*1024*1024*1024

def main():
    req=json.loads(sys.stdin.buffer.readline(1024*1024+1))
    path=pathlib.Path(req['path'])
    if path.stat().st_size>MAX_TOTAL: raise ValueError('ACE input exceeds 2 GiB limit')
    # Supply explicit sibling volumes to avoid scanning arbitrary files. The
    # library checks ACE volume numbers and missing continuation segments.
    volumes=[str(path)]
    total=path.stat().st_size
    for i in range(1000):
        p=path.with_suffix('.c%02d'%i)
        if not p.exists(): break
        if p.is_symlink(): raise ValueError('linked ACE volume rejected')
        total+=p.stat().st_size
        if total>MAX_TOTAL: raise ValueError('ACE volumes exceed 2 GiB limit')
        volumes.append(str(p))
    if len(volumes)>1000: raise ValueError('ACE volume count exceeds limit')
    with acefile.open(volumes,encoding=req.get('encoding','cp437'),strict_multivolume=True) as archive:
        entries=[]; total=0; seen=set(); files=set()
        for i,m in enumerate(archive):
            if i>=100000: raise ValueError('ACE entry count exceeds limit')
            name=m.filename.replace('\\','/')
            p=pathlib.PureWindowsPath(name)
            if p.is_absolute() or p.drive or any(x in ('..','.') for x in name.split('/')): raise ValueError('unsafe ACE filename')
            key=name.rstrip('/').lower()
            if key in seen: raise ValueError('duplicate ACE filename')
            seen.add(key)
            if not m.is_dir(): files.add(key)
            total+=m.size
            if m.size>MAX_ENTRY or total>MAX_TOTAL: raise ValueError('ACE decoded size exceeds limit')
            entries.append(dict(name=name,size=m.size,packed_size=m.packsize,is_dir=m.is_dir(),crc='%08X'%m.crc32,encrypted=m.is_enc(),modified=str(m.datetime),method='ACE%d'%m.comptype,comment=m.comment,attributes=''))
        for name in seen:
            parts=name.split('/')
            if any('/'.join(parts[:i]) in files for i in range(1,len(parts))): raise ValueError('ACE file/directory collision')
        if req['action']=='list':
            out=json.dumps(entries,ensure_ascii=True)
            if len(out)>32*1024*1024: raise ValueError('ACE listing too large')
            print(out)
            return
        wanted=set(req['selected'])
        stage=pathlib.Path(req['stage'])
        for i,m in enumerate(archive):
            if entries[i]['name'] not in wanted or m.is_dir(): continue
            size=0
            with (stage/('%d.bin'%i)).open('xb') as f:
                # Streaming also verifies the complete member CRC at end.
                for block in archive.readblocks(m,pwd=req.get('password')):
                    size+=len(block)
                    if size>m.size or size>MAX_ENTRY: raise ValueError('ACE decoded size overflow')
                    f.write(block)
            if size!=m.size: raise ValueError('ACE decoded size mismatch')
        print('{"ok":true}')

try:
    main()
except Exception as exc:
    # No password or request echo in diagnostics.
    print(json.dumps({'error':type(exc).__name__+': '+str(exc)[:1024]}))
    sys.exit(1)
