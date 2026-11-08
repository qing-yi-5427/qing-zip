"""Run only inside a uniquely verified scratch VHD on a hosted Windows worker."""
from pathlib import Path
import ctypes,hashlib,json,os,subprocess,time,zipfile
from pywinauto import Desktop, keyboard

assert os.environ.get("RUNNER_ENVIRONMENT")=="github-hosted" and os.environ.get("GITHUB_ACTIONS")=="true"
root=Path(os.environ["QING_VHD_ROOT"])
assert root.is_dir() and root.name.startswith("case-") and root.parent==Path(root.anchor)
fs=os.environ["QING_VHD_FS"]
assert fs in ("FAT32","exFAT")
receipt=Path("fat-evidence")/(fs+".json")
tools=json.loads(Path("fat-evidence/tools.json").read_text(encoding="utf-8-sig"))
data={"keep.txt":b"keep original","remove.txt":b"remove only on delete","nested/old.txt":b"old filename"}
digest=lambda v:hashlib.sha256(v).hexdigest()
def members(p):
    with zipfile.ZipFile(p) as z:
        assert z.testzip() is None
        return {n.filename:z.read(n) for n in z.infolist() if not n.is_dir()}
def save_as(app,src,dest,scope):
    env=dict(os.environ,WEBVIEW2_USER_DATA_FOLDER=str(scope/"browser"),LOCALAPPDATA=str(scope/"local"))
    (scope/"local").mkdir(parents=True,exist_ok=True)
    child=subprocess.Popen([str(app),str(src)],env=env)
    desktop=Desktop(backend="uia")
    def wait(fn,label,sec=15):
        end=time.monotonic()+sec
        while time.monotonic()<end:
            v=fn()
            if v:return v
            time.sleep(.15)
        raise TimeoutError(label)
    def controls(w):return [c for c in w.descendants() if c.is_visible()]
    def named(win,kind,term):
        return next((x for x in controls(win) if x.element_info.control_type in kind and x.element_info.name.startswith(term)),None)
    try:
        main=wait(lambda:next((v for v in desktop.windows(process=child.pid) if v.is_visible() and v.window_text()=="qing-zip"),None),"main window")
        main.set_focus()
        wait(lambda:named(main,["Button"],"keep.txt"),"archive listing")
        wait(lambda:named(main,["Button"],"所有菜单"),"menu trigger").click_input()
        wait(lambda:named(main,["MenuItem"],"文件"),"File menu").click_input()
        wait(lambda:named(main,["MenuItem"],"另存为"),"Save As item").click_input()
        dialog=wait(lambda:next((w for w in desktop.windows(process=child.pid,class_name="#32770") if w.is_visible()),None),"native save dialog")
        dialog.set_focus()
        field=next((w for w in controls(dialog) if w.element_info.control_type=="Edit" and w.element_info.automation_id=="1001"),None)
        if field:
            field.set_edit_text(str(dest));field.set_focus()
        else:
            keyboard.send_keys("%n");keyboard.send_keys("^a");keyboard.send_keys(str(dest),with_spaces=True)
        keyboard.send_keys("{ENTER}")
        wait(lambda:not ctypes.windll.user32.IsWindowVisible(dialog.handle),"dialog submitted",5)
        end=time.monotonic()+6
        while not dest.exists() and time.monotonic()<end:time.sleep(.1)
        hints=[c.element_info.name for c in controls(main) if c.element_info.control_type in ("Text","Alert")][-35:]
        return {"method":"real GUI and common save dialog", "hints":hints, "ok":dest.exists() and dest.read_bytes()==src.read_bytes()}
    except Exception as e:
        visible=[] if 'main' not in locals() else [(c.element_info.control_type,c.element_info.name,c.element_info.automation_id) for c in controls(main)][:180]
        return {"driver_error":repr(e),"visible":visible,"ok":False}
    finally:
        if child.poll() is None:child.terminate();child.wait(timeout=15)

all_rows=[]
for role,tool in tools.items():
    base=root/role;base.mkdir(); cli=Path(tool["cli"]);gui=Path(tool["gui"])
    assert digest(cli.read_bytes())==tool["cli_sha"] and digest(gui.read_bytes())==tool["gui_sha"]
    def request(op,arc,**fields):
        return subprocess.run([cli,"json"],input=json.dumps(dict(operation=op,path=str(arc),**fields)).encode(),capture_output=True,timeout=60)
    for op in ("create","add","delete","rename","comment-set","save-as","extract","extract-existing"):
        work=base/op;work.mkdir();arc=work/"sample.zip";dest=work/"out"
        if op!="create":
            with zipfile.ZipFile(arc,"w",compression=zipfile.ZIP_DEFLATED) as zipped:
                for n,v in data.items():zipped.writestr(n,v)
        before=digest(arc.read_bytes()) if arc.exists() else None
        row={"role":role,"op":op,"filesystem":fs}
        try:
            expected=dict(data)
            if op in ("add","create"):
                extra=work/"added & one.txt";extra.write_bytes(b"added content")
                r=request(op,arc,inputs=[str(extra)],format="zip")
                if op=="create":expected={}
                expected[extra.name]=extra.read_bytes()
            elif op=="delete":
                r=request(op,arc,inputs=["remove.txt"]);del expected["remove.txt"]
            elif op=="rename":
                r=request(op,arc,inputs=["nested/old.txt","nested/new.txt"]);expected["nested/new.txt"]=expected.pop("nested/old.txt")
            elif op=="comment-set":r=request(op,arc,name="comment on "+fs)
            elif op=="save-as":
                target=work/"copied.zip"
                result=save_as(gui,arc,target,Path(os.environ["RUNNER_TEMP"])/("qing-fat-ui-"+role+"-"+fs))
                row.update(result)
                if target.exists():row["dest_sha"]=digest(target.read_bytes())
            else:
                if op=="extract-existing":dest.mkdir();(dest/"sentinel.txt").write_bytes(b"user file");expected["sentinel.txt"]=b"user file"
                r=request("extract",arc,dest=str(dest))
            if op!="save-as":
                row.update(rc=r.returncode,stdout=r.stdout.decode("utf8","replace")[-1600:],stderr=r.stderr.decode("utf8","replace")[-800:])
                if op.startswith("extract"):
                    observed={f.relative_to(dest).as_posix():f.read_bytes() for f in dest.rglob("*") if f.is_file()} if dest.exists() else {}
                    row["ok"]=r.returncode==0 and observed==expected
                else:
                    row["ok"]=r.returncode==0 and arc.is_file() and members(arc)==expected
                    if op=="comment-set" and row["ok"]:
                        c=request("comment-get",arc)
                        row["ok"]=c.returncode==0 and json.loads(c.stdout)["comment"]=="comment on "+fs
            row["source_unchanged"]=before==digest(arc.read_bytes()) if before and arc.exists() else None
            if (not row["ok"] or op.startswith("extract") or op=="save-as") and before:assert row["source_unchanged"],"Unexpected source mutation"
        except Exception as e: row.update(ok=False,probe_error=repr(e))
        all_rows.append(row)
        receipt.write_text(json.dumps({"filesystem":fs,"tools":tools,"rows":all_rows},indent=2,ensure_ascii=False),encoding="utf8")
        print(fs,role,op,row["ok"],row.get("driver_error",""),row.get("stderr",""),flush=True)