from pathlib import Path
import hashlib,json,subprocess,sys
root=Path.cwd(); out=Path('receipt-rust-evidence');out.mkdir(exist_ok=True)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
source=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(); names=subprocess.check_output(['git','ls-files','-z']).decode().split('\0');inputs={n:sha(root/n) for n in names if n and (root/n).is_file()}
cmd=['cargo','test','--locked','--manifest-path','obz-rs/Cargo.toml','--target-dir','target/receipt-hosted','--lib','private_stage::output_receipt::tests','--','--nocapture']
with (out/'tests.log').open('w') as log:p=subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
log=(out/'tests.log').read_text();print(log)
receipt={'source':source,'inputs':inputs,'command':cmd,'exit_code':p.returncode,'compiler':subprocess.check_output(['rustc','-Vv'],text=True),'test_executables':{str(p):sha(p) for p in Path('target/receipt-hosted/debug/deps').glob('*.exe')},'test_log_sha256':sha(out/'tests.log'),'source_unchanged':inputs=={n:sha(root/n) for n in inputs},'owner_transition_actually_tested':False}
(out/'receipt.json').write_text(json.dumps(receipt,indent=2))
assert receipt['source_unchanged'];assert p.returncode==0;assert '6 passed; 0 failed' in log
