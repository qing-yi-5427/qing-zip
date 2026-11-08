"""Cancel a shipping GUI extract after its first published file on the disposable FAT volume."""
import hashlib,json,os,subprocess,sys,tempfile,time,urllib.request,zipfile
from pathlib import Path
from websockets.sync.client import connect

def main():
    assert os.environ.get("RUNNER_ENVIRONMENT")=="github-hosted" and os.environ.get("GITHUB_ACTIONS")=="true"
    root=Path(os.environ["QING_VHD_ROOT"])
    assert root.is_dir() and root.parent==Path(root.anchor) and root.name.startswith("case-")
    fs=os.environ["QING_VHD_FS"]
    assert fs in ("exFAT","FAT32")
    report=Path("fat-evidence")/("cancel-"+fs+".json")
    gui=Path(json.loads(Path("fat-evidence/tools.json").read_bytes())["candidate"]["gui"])
    result={"gui":hashlib.sha256(gui.read_bytes()).hexdigest(),"filesystem":fs}
    dest=root/"cancel-new-target"
    assert not dest.exists()
    with tempfile.TemporaryDirectory(dir=os.environ["RUNNER_TEMP"],ignore_cleanup_errors=True) as temp:
        temp=Path(temp)
        archive=temp/"cancel-input.zip"
        with zipfile.ZipFile(archive,"w",compression=zipfile.ZIP_STORED) as z:
            for i in range(10000):z.writestr(f"file-{i:05d}.txt",b"known content"*4)
        result["source_before"]=hashlib.sha256(archive.read_bytes()).hexdigest()
        profile=temp/"browser";local=temp/"local";local.mkdir()
        env=dict(os.environ,LOCALAPPDATA=str(local),WEBVIEW2_USER_DATA_FOLDER=str(profile),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=0 --remote-debugging-address=127.0.0.1")
        proc=subprocess.Popen([gui,archive],env=env,cwd=temp)
        try:
            deadline=time.monotonic()+30
            while time.monotonic()<deadline:
                assert proc.poll() is None,proc.returncode
                portfile=next(profile.rglob('DevToolsActivePort'),None)
                if portfile:
                    port=int(portfile.read_text().splitlines()[0])
                    tabs=json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json/list',timeout=3))
                    tab=next((x for x in tabs if x.get('type')=='page' and 'tauri.localhost' in x['url']),None)
                    if tab:break
                time.sleep(.25)
            else:raise RuntimeError('No app document endpoint in the disposable profile')
            with connect(tab['webSocketDebuggerUrl'],proxy=None) as ws:
                seq=0
                def call(method,params):
                    nonlocal seq
                    seq+=1;ws.send(json.dumps(dict(id=seq,method=method,params=params)))
                    while True:
                        msg=json.loads(ws.recv(timeout=8))
                        if msg.get('id')==seq:
                            assert 'error' not in msg,msg
                            return msg['result']
                def js(code):
                    val=call('Runtime.evaluate',{'expression':code,'returnByValue':True})
                    assert 'exceptionDetails' not in val,val
                    return val['result'].get('value')
                def wait(code):
                    until=time.monotonic()+20
                    while time.monotonic()<until:
                        if js(code):return
                        time.sleep(.12)
                    raise RuntimeError(code+'\n'+str(js('document.body.innerText.slice(-1500)')))
                def click(selector,mod=0,count=1):
                    sel=json.dumps(selector);wait('!!document.querySelector('+sel+')')
                    x,y=js('(()=>{const r=document.querySelector('+sel+').getBoundingClientRect();return [r.x+r.width*.55,r.y+r.height*.5]})()')
                    for n in range(1,count+1):
                        for mode in ['mousePressed','mouseReleased']:
                            call('Input.dispatchMouseEvent',dict(type=mode,x=x,y=y,button='left',buttons=int(mode=='mousePressed'),clickCount=n,modifiers=mod))
                wait('!!document.querySelector("[data-row]")')
                click('[data-action=extract]')
                js('document.querySelector("#qa-destination").value='+json.dumps(str(dest)))
                js('document.querySelector("#qa-smart").checked=false')
                click('[data-action=startextract]')
                wait('!!document.querySelector("[data-action=canceljob]")')
                until=time.monotonic()+100
                while time.monotonic()<until:
                    if dest.exists():
                        first=next((p for p in dest.iterdir() if p.name.startswith("file-") and p.is_file()),None)
                        if first:
                            result["first_observed"]=first.name
                            break
                    status=js('document.querySelector(".qa-jobcard .qa-badge")?.textContent')
                    if status and status!='处理中':raise RuntimeError("Operation ended before cancel: "+str(js('document.querySelector(".qa-jobcard")?.innerText')))
                    time.sleep(.015)
                assert result.get("first_observed"), "No visible file before cancellation"
                click('[data-action=canceljob]')
                wait('document.querySelector(".qa-jobcard .qa-badge")?.textContent==="已取消"')
                result["cancelled"]=True
                result["destination_removed"]=not dest.exists()
                result["source_after"]=hashlib.sha256(archive.read_bytes()).hexdigest()
                assert result["destination_removed"] and result["source_before"]==result["source_after"],result
        except Exception as e:
            result["error"]=repr(e)
            if dest.exists():result["surviving"]=sorted(v.name for v in dest.iterdir())[:20]
            raise
        finally:
            report.write_text(json.dumps(result,indent=2),encoding="utf8")
            if proc.poll() is None:proc.terminate();proc.wait(timeout=15)
if __name__=="__main__":main()
