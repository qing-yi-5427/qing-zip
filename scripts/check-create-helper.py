import os,sys,json,hashlib,shutil,subprocess,time,argparse,tempfile
from pathlib import Path
ap=argparse.ArgumentParser(description='Native create helper handshake, safe fallback and mixed-version acceptance; synthetic files only')
ap.add_argument('--cli',type=Path,required=True);ap.add_argument('--old-cli',type=Path,required=True);ap.add_argument('--old-helper',type=Path,required=True);ap.add_argument('--workspace',type=Path,required=True)
a=ap.parse_args();BIN=a.cli.resolve(strict=True).parent;OLDCLI=a.old_cli.resolve(strict=True);OLDHELPER=a.old_helper.resolve(strict=True)
OUT=Path(tempfile.mkdtemp(prefix='create-helper-proof-',dir=a.workspace.resolve(strict=True)))
PKG=OUT/'package';PKG.mkdir();shutil.copy2(BIN/'obz.exe',PKG/'obz.exe');shutil.copytree(BIN/'runtime',PKG/'runtime')
NEW=OUT/'new-helper.exe';shutil.copy2(BIN/'obz-worker.exe',NEW)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
src=OUT/'source';src.mkdir()
for i in range(2050):(src/f'{i:04}.txt').write_bytes((f'content{i}'*10).encode())
report={'passed':False,'cli_sha256':sha(PKG/'obz.exe'),'script_sha256':sha(Path(__file__)),'old_cli_sha256':sha(OLDCLI),'helper_sha256':sha(NEW),'old_helper_sha256':sha(OLDHELPER),'rows':[]}
def save():(OUT/'receipt.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
def run(cmd,**kw):return subprocess.run([str(x) for x in cmd],capture_output=True,timeout=90,**kw)
# Actual published helper, not a reconstructed stub, must decline the new entry cleanly.
p=run([OLDHELPER,'--internal-create-worker'],input=b'');assert p.returncode==3 and not p.stdout;report['old_entry']={'exit':p.returncode,'stdout_hex':p.stdout.hex()};save()
rust=OUT/'fault.rs';rust.write_text(r"""use std::{io::{self,Read,Write},fs,env,time::Duration};
fn send(k:u8,b:&[u8]) {let mut o=io::stdout().lock();o.write_all(&[k]).unwrap();o.write_all(&(b.len() as u32).to_le_bytes()).unwrap();o.write_all(b).unwrap();o.flush().unwrap();}
fn recv()->(u8,Vec<u8>) {let mut i=io::stdin().lock();let mut h=[0;5];i.read_exact(&mut h).unwrap();let n=u32::from_le_bytes(h[1..5].try_into().unwrap()) as usize;assert!(n<=1024);let mut b=vec![0;n];i.read_exact(&mut b).unwrap();let path=env::var("QING_CREATE_TRACE").unwrap();let mut f=fs::OpenOptions::new().append(true).create(true).open(path).unwrap();writeln!(f,"{} {}",h[0],n).unwrap();(h[0],b)}
fn main(){let m=env::var("QING_CREATE_FAULT").unwrap();if m=="exit0"{return}if m=="exit1"{std::process::exit(1)}if m=="exit3"{std::process::exit(3)}if m=="timeout"{std::thread::sleep(Duration::from_secs(30));return}
let mut hello=[1u32.to_le_bytes(),3u32.to_le_bytes()].concat();if m=="version"{hello[0]=2}if m=="caps"{hello[4]=1}send(if m=="unknown"{99}else{49},&hello);
let (_,mut nonce)=recv();if m=="nonce"{nonce[0]^=1}if m=="longack"{nonce.push(0)}send(51,&nonce);
if ["version","caps","unknown","nonce","longack"].contains(&m.as_str()){std::thread::sleep(Duration::from_secs(30));return}
let _=recv();send(47,&[]);if m=="afterbegin"{send(41,&[])}else{send(43,b"fixture error after encoding began")};}
""",encoding='utf-8')
stub=OUT/'stub.exe';p=run(['rustc','--edition','2021','-O',rust,'-o',stub]);assert p.returncode==0,p.stderr
for mode in ['new','missing','old','version','caps','unknown','nonce','longack','timeout','exit0','exit1','exit3','afterbegin','aftererror']:
 helper=PKG/'obz-worker.exe'
 if helper.exists():helper.unlink() # exact test-owned ordinary file, no recursive deletion
 if mode!='missing':shutil.copy2(NEW if mode=='new' else OLDHELPER if mode=='old' else stub,helper)
 trace=OUT/(mode+'.trace');dest=OUT/(mode+'.7z');env={**os.environ,'OPEN_ARCHIVE_PROFILE':'1','QING_CREATE_FAULT':mode,'QING_CREATE_TRACE':str(trace)}
 p=run([PKG/'obz.exe','create',dest,src,'--format','7z','--level','5','--threads','4'],env=env)
 expected=mode in ['new','missing','old','exit3'];err=p.stderr.decode(errors='replace');frames=trace.read_text().splitlines() if trace.exists() else []
 row={'mode':mode,'exit':p.returncode,'stderr':err,'received_frames':frames,'expected_success':expected};report['rows'].append(row);save()
 assert (p.returncode==0)==expected,row
 if expected:
  assert 'status=complete' in err and dest.is_file();row['archive_sha256']=sha(dest)
 else:
  assert not dest.exists() and 'OA_PHASE engine-process' not in err,row
  assert not list(OUT.glob('.qing-zip-create-*')),row
  if mode in ['version','caps','unknown','timeout','exit0','exit1']:assert not frames,row
  if mode in ['nonce','longack']:assert frames==['50 16'],row
  if mode in ['afterbegin','aftererror']:assert len(frames)==2 and frames[0]=='50 16' and frames[1].startswith('48 '),row
 save();print(mode,'passed',flush=True)
# Old parent continues using its original create entry despite a new helper beside it.
shutil.copy2(NEW,PKG/'obz-worker.exe');shutil.copy2(OLDCLI,PKG/'obz.exe')
p=run([PKG/'obz.exe','create',OUT/'old-parent.zip',src,'--format','zip','--threads','4']);assert p.returncode==0,p.stderr
report['old_parent_new_helper_create']=True
# Its full-worker extraction path is unchanged by adding the create entry.
small=OUT/'small';small.mkdir();(small/'a.txt').write_bytes(b'full worker compatibility')
nana=PKG/'runtime/nanazip/NanaZip.Universal.Console.exe';p=run([nana,'a','-t7z',OUT/'small.7z',small]);assert p.returncode==0
p=run([PKG/'obz.exe','extract',OUT/'small.7z',OUT/'decoded','--threads','4']);assert p.returncode==0,p.stderr;assert (OUT/'decoded/small/a.txt').read_bytes()==b'full worker compatibility'
report['old_parent_new_helper_full_extract']=True
assert len({r['archive_sha256'] for r in report['rows'] if r['expected_success']})==1
report['passed']=True;save();print(OUT/'receipt.json')
