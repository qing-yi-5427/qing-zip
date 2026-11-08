"""Cancel the real GUI after an archive member has reached the disposable volume."""
import hashlib,json,os,subprocess,tempfile,time,zipfile
from pathlib import Path
from pywinauto import Desktop,keyboard
import win32clipboard

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
    def wait(get,desc,seconds=30):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            v=get()
            if v:return v
            time.sleep(.08)
        raise RuntimeError(desc)
    with tempfile.TemporaryDirectory(dir=os.environ["RUNNER_TEMP"],ignore_cleanup_errors=True) as temp:
        temp=Path(temp)
        archive=temp/"cancel-input.zip"
        with zipfile.ZipFile(archive,"w",compression=zipfile.ZIP_STORED) as z:
            for i in range(10000):z.writestr(f"file-{i:05d}.txt",b"known content"*4)
        result["source_before"]=hashlib.sha256(archive.read_bytes()).hexdigest()
        profile=temp/"browser";local=temp/"local";local.mkdir()
        env=dict(os.environ,LOCALAPPDATA=str(local),WEBVIEW2_USER_DATA_FOLDER=str(profile))
        proc=subprocess.Popen([gui,archive],env=env,cwd=temp)
        desktop=Desktop(backend="uia")
        try:
            window=wait(lambda:next((v for v in desktop.windows(process=proc.pid) if v.is_visible() and v.window_text()=="qing-zip"),None),"main window")
            window.set_focus()
            def controls():return [v for v in window.descendants() if v.is_visible()]
            def button(label):return next((v for v in controls() if v.element_info.control_type=="Button" and v.element_info.name==label and v.is_enabled()),None)
            wait(lambda:button("解压"),"extract toolbar").click_input()
            edit=wait(lambda:next((v for v in controls() if v.element_info.control_type=="Edit" and v.element_info.automation_id=="qa-destination"),None),"destination textbox")
            edit.click_input()
            win32clipboard.OpenClipboard()
            try:
                win32clipboard.EmptyClipboard();win32clipboard.SetClipboardText(str(dest),13)
            finally:win32clipboard.CloseClipboard()
            keyboard.send_keys("^a^v")
            assert edit.get_value()==str(dest)
            wait(lambda:button("开始解压"),"start extract").click_input()
            cancel=wait(lambda:button("取消任务"),"running cancel")
            def published():
                for parent in (dest,dest/archive.stem):
                    if parent.exists():
                        first=next((p for p in parent.iterdir() if p.name.startswith("file-") and p.is_file()),None)
                        if first:return first
            first=wait(published,"no file published before cancellation",80)
            result["first_observed"]=first.relative_to(dest).as_posix()
            cancel.click_input()
            wait(lambda:any(v.element_info.name=="已取消" for v in controls()),"GUI did not report cancelled",60)
            result["cancelled"]=True
            result["destination_removed"]=not dest.exists()
            result["source_after"]=hashlib.sha256(archive.read_bytes()).hexdigest()
            assert result["destination_removed"] and result["source_before"]==result["source_after"],result
        except Exception as e:
            result["error"]=repr(e)
            if "window" in locals():
                result["controls"]=[(v.element_info.control_type,v.element_info.name,v.element_info.automation_id) for v in controls()][:90]
            if dest.exists():result["surviving"]=sorted(v.name for v in dest.iterdir())[:20]
            raise
        finally:
            report.write_text(json.dumps(result,indent=2,ensure_ascii=False),encoding="utf8")
            if proc.poll() is None:proc.terminate();proc.wait(timeout=15)
if __name__=="__main__":main()
