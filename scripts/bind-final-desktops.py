"""Bind an exact same-run hardened distribution and registered reference; no launch."""
import argparse,hashlib,json,subprocess
from pathlib import Path
p=argparse.ArgumentParser()
for key in ['product','stage','nana','package','output','fixture','build']:p.add_argument('--'+key,type=Path,required=True)
p.add_argument('--commit',required=True);a=p.parse_args()
def sha(f):
 with f.open('rb') as h:return hashlib.file_digest(h,'sha256').hexdigest()
head=subprocess.check_output(['git','-C',str(a.product),'rev-parse','HEAD'],text=True).strip()
assert head==a.commit
package=json.loads(a.package.read_text(encoding='utf-8-sig'))
assert package['Version']=='7.0.1882.0' and package['Name']=='40174MouriNaruto.NanaZip'
qing=a.stage/'qing-zip.exe';build=json.loads(a.build.read_text(encoding='utf-8-sig'))
for item in build['binaries']:
 actual=a.stage/('qing-zip.exe' if item['name']=='obz-gui.exe' else item['name'])
 assert sha(actual)==item['sha256']
assert sha(a.fixture)=='5e61cbc8904d336e624fe4375e9d3e7a847d6c3ce17cc8f13486c465a5809312'
r={'source_commit':head,'qing_sha256':sha(qing),'nana_sha256':sha(a.nana),'archive_sha256':sha(a.fixture),'nana_package_full_name':package['PackageFullName'],'package':package,'hardened_build':build,'hardened_build_sha256':sha(a.build),'qing_dependencies':{f.relative_to(a.stage).as_posix():sha(f) for f in [a.stage/'obz.exe',a.stage/'obz-worker.exe',*sorted((a.stage/'runtime').rglob('*'))] if f.is_file()},'nana_dependencies':{f.relative_to(a.nana.parent).as_posix():sha(f) for f in sorted(a.nana.parent.rglob('*')) if f.is_file() and f.suffix.lower() in ['.exe','.dll']}}
a.output.write_text(json.dumps(r,indent=2),encoding='utf-8')
