"""Inspect and activate the real Windows 11 Explorer menu on a disposable runner."""
import argparse, hashlib, json, os, subprocess, time, zipfile
from pathlib import Path
assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('RUNNER_ENVIRONMENT') == 'github-hosted'
from PIL import ImageGrab
from pywinauto import Desktop, mouse, keyboard
from win32com.client import Dispatch
import psutil
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
 root=a.root/'ui-fixtures';root.mkdir()
 data={'first.txt':b'first synthetic member\n','second.txt':b'second synthetic member\n'}
 expected={name:hashlib.sha256(value).hexdigest() for name,value in data.items()}
 report['expected_member_sha256']=expected
 desktop=Desktop(backend='uia')
 # Hosted Windows 11 ARM images can start behind first-run privacy setup.
 # Complete it only on this disposable machine before interacting with Explorer.
 report['first_run_actions']=[]
 for _ in range(12):
  candidates=[]
  for top in desktop.windows():
   try:
    if top.is_visible():
     for button in top.descendants(control_type='Button'):
      if button.is_visible() and button.element_info.name in ('Next','Accept'):
       candidates.append(button)
   except Exception: pass
  if not candidates: break
  button=candidates[0];report['first_run_actions'].append(button.element_info.name);button.click_input();time.sleep(2)
 # The first-run surface is not exposed through UI Automation on these
 # images. Recognize its fixed 1024x768 blue Next/Accept button and use
 # physical input only on the disposable runner; preserve each screenshot.
 for step in range(3):
  image=ImageGrab.grab(); image.save(out/('first-run-'+str(step)+'.png'))
  if image.size != (1024,768): break
  r,g,b=image.convert('RGB').getpixel((810,650))
  if not (b>120 and b>g and g>r+30): break
  mouse.click(coords=(867,656)); report['first_run_actions'].append('physical first-run button');time.sleep(3)
 snapshot('desktop-after-first-run');save()
 shell=Dispatch('Shell.Application')
 for key,label in [('here','解压到此处'),('smart','智能解压'),('compress','用 qing-zip 压缩…')]:
  folder=root/key;folder.mkdir(); archive=folder/'archive.zip'; plain=folder/'plain.txt'
  plain.write_bytes(b'ordinary compression source\n')
  with zipfile.ZipFile(archive,'w') as z:
   for name,contents in data.items():z.writestr(name,contents)
  archive_hash=hashlib.sha256(archive.read_bytes()).hexdigest();plain_hash=hashlib.sha256(plain.read_bytes()).hexdigest()
  shell.Open(str(folder))
  def explorer():
   for w in shell.Windows():
    try:
     if os.path.normcase(w.Document.Folder.Self.Path)==os.path.normcase(str(folder)): return w
    except Exception: pass
  target=wait(explorer);window=desktop.window(handle=int(target.HWND)).wrapper_object();window.set_focus();report['stage']='explorer_opened';save()
  filename='plain' if key=='compress' else 'archive'
  entry=report['scenarios'][key]={'menu_visible':False,'activated':False,'handoff_observed':False,'archive_sha256_before':archive_hash,'plain_sha256_before':plain_hash};save()
  item=wait(lambda: next((x for x in window.descendants(control_type='ListItem') if x.element_info.name==filename),None))
  item.click_input(button='right');time.sleep(2); snapshot(key+'-menu');entry['menu_controls']=text_controls()
  group=next((x for w in desktop.windows() if w.is_visible() for x in w.descendants() if x.element_info.name=='qing-zip' and x.element_info.control_type=='MenuItem' and x.is_visible()),None)
  if group:
   entry['top_level_visible']=True
   rect=group.rectangle();entry['group_rect']=str(rect);group.click_input();time.sleep(2);snapshot(key+'-submenu')
  menu=next((x for w in desktop.windows() if w.is_visible() for x in w.descendants() if x.element_info.name==label and x.is_visible()),None)
  if not menu:
   entry['error']='Command not observed after opening qing-zip submenu';keyboard.send_keys('{ESC}');save();continue
  entry['menu_visible']=True;menu.click_input();entry['activated']=True
  destination=folder/('archive' if key=='smart' else '')
  def valid_outputs():
   return all((destination/name).is_file() and hashlib.sha256((destination/name).read_bytes()).hexdigest()==digest for name,digest in expected.items())
  for _ in range(100):
   if key!='compress' and valid_outputs():break
   time.sleep(.2)
  snapshot(key+'-after')
  entry['output_matches_expected']=valid_outputs() if key!='compress' else None
  entry['relative_output_directory']='archive' if key=='smart' else ('.' if key=='here' else None)
  entry['archive_preserved']=hashlib.sha256(archive.read_bytes()).hexdigest()==archive_hash
  entry['plain_preserved']=hashlib.sha256(plain.read_bytes()).hexdigest()==plain_hash
  entry['gui_windows']=[w.element_info.name for w in desktop.windows() if w.is_visible() and 'qing' in w.element_info.name.lower()]
  entry['handoff_observed']=entry['output_matches_expected'] if key!='compress' else False
  if key=='compress':entry['error']='Compression form and output not yet verified'
  save()
  for proc in psutil.process_iter(['pid','exe','name']):
   try:
    if proc.info['exe'] and os.path.normcase(proc.info['exe'])==os.path.normcase(str(a.app.resolve())):proc.terminate();proc.wait(10)
   except (psutil.Error,OSError):pass
  target.Quit()
 report['stage']='finished';report['passed']=all(v['menu_visible'] and v['handoff_observed'] for v in report['scenarios'].values())
except Exception as e:
 report['error']=repr(e)
 try:snapshot('failure')
 except Exception:pass
finally:save()
if not report['passed']:raise SystemExit('Explorer actions not all observed; inspect receipt')
