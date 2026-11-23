"""Inspect and activate the real Windows 11 Explorer menu on a disposable runner."""
import argparse, hashlib, json, os, subprocess, time, zipfile
from pathlib import Path
assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
from PIL import ImageGrab
from pywinauto import Desktop, mouse, keyboard
from win32com.client import Dispatch
p=argparse.ArgumentParser(); p.add_argument('--root',type=Path,required=True);p.add_argument('--app',type=Path,required=True);a=p.parse_args()
out=Path('explorer-evidence');out.mkdir(exist_ok=True)
report={'stage':'initialize','scenarios':{},'passed':False,'not_observed':['portable_move','unregistration_menu']}
def save(): (out/'explorer.json').write_text(json.dumps(report,indent=2,ensure_ascii=False),encoding='utf8')
def wait(fn,seconds=25):
 end=time.monotonic()+seconds
 while time.monotonic()<end:
  try:
   value=fn()
   if value:return value
  except Exception:pass
  time.sleep(.2)
 raise RuntimeError('Timed out waiting for Explorer UI')
def snapshot(name): ImageGrab.grab().save(out/(name+'.png'))
def text_controls():
 items=[]
 for w in desktop.windows():
  try:
   if w.is_visible():
    for x in [w,*w.descendants()]:
     n=x.element_info.name
     if n and ('qing' in n.lower() or n in ('智能解压','解压到此处')): items.append({'name':n,'type':x.element_info.control_type,'class':x.element_info.class_name,'handle':x.handle})
  except Exception: pass
 return items
try:
 root=a.root/'ui-fixtures';root.mkdir(); file=root/'sample.txt';file.write_text('synthetic explorer fixture\n',encoding='utf8')
 archive=root/'sample.zip'
 with zipfile.ZipFile(archive,'w') as z:z.writestr('inside.txt','synthetic archive content\n')
 report['source_sha256']=hashlib.sha256(archive.read_bytes()).hexdigest()
 shell=Dispatch('Shell.Application');shell.Open(str(root));desktop=Desktop(backend='uia')
 def explorer():
  for w in shell.Windows():
   try:
    if os.path.normcase(w.Document.Folder.Self.Path)==os.path.normcase(str(root)): return w
   except Exception: pass
 target=wait(explorer); hwnd=int(target.HWND);window=desktop.window(handle=hwnd).wrapper_object();window.set_focus();report['stage']='explorer_opened';save()
 for key,filename,label in [('open','sample.zip','用 qing-zip 打开'),('extract','sample.zip','解压到此处'),('compress','sample.txt','用 qing-zip 压缩…')]:
  entry=report['scenarios'][key]={'menu_visible':False,'activated':False,'handoff_observed':False};save()
  window.set_focus();item=wait(lambda: next((x for x in window.descendants(control_type='ListItem') if x.element_info.name==filename),None))
  item.click_input(button='right');time.sleep(2); snapshot(key+'-menu');entry['menu_controls']=text_controls()
  menu=next((x for w in desktop.windows() if w.is_visible() for x in w.descendants() if x.element_info.name==label and x.is_visible()),None)
  if not menu:
   entry['error']='Command not observed in the initial Explorer context menu';keyboard.send_keys('{ESC}');save();continue
  entry['menu_visible']=True;menu.click_input();entry['activated']=True;time.sleep(3);snapshot(key+'-after');
  processes=subprocess.run(['powershell','-NoProfile','-Command',"Get-CimInstance Win32_Process -Filter \"name = 'qing-zip.exe'\" | Select-Object ProcessId,CommandLine | ConvertTo-Json -Compress"],capture_output=True,text=True,timeout=15)
  entry['gui_process_present']=bool(processes.stdout.strip());entry['handoff_observed']=entry['gui_process_present']
  save()
  for proc in subprocess.run(['powershell','-NoProfile','-Command',"Get-Process qing-zip -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id"],capture_output=True,text=True).stdout.split():
   if proc.isdigit():subprocess.run(['taskkill','/PID',proc,'/T','/F'],capture_output=True)
 report['stage']='finished';report['passed']=all(v['menu_visible'] and v['handoff_observed'] for v in report['scenarios'].values())
except Exception as e:
 report['error']=repr(e)
 try:snapshot('failure')
 except Exception:pass
finally:save()
if not report['passed']:raise SystemExit('Explorer actions not all observed; inspect receipt')
