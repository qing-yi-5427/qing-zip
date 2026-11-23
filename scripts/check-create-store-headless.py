"""Exercise the real creation form in headless Edge with mocked native calls."""
import json, os, subprocess, tempfile, threading, time, urllib.request
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from functools import partial
from websockets.sync.client import connect

root = Path(__file__).resolve().parents[1]
edge = Path(os.environ.get('EDGE_EXE', r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'))
class Quiet(SimpleHTTPRequestHandler):
    def log_message(self, *args): pass
server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Quiet, directory=str(root / 'obz-gui/frontend')))
threading.Thread(target=server.serve_forever, daemon=True).start()
with tempfile.TemporaryDirectory(prefix='qz-create-headless-', ignore_cleanup_errors=True) as profile:
    proc = subprocess.Popen([str(edge), '--headless=new', '--disable-gpu', '--no-first-run', '--remote-debugging-port=0', f'--user-data-dir={profile}', 'about:blank'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        portfile = Path(profile) / 'DevToolsActivePort'
        for _ in range(300):
            if portfile.exists(): break
            assert proc.poll() is None
            time.sleep(.1)
        port = int(portfile.read_text().splitlines()[0])
        page = next(x for x in json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json')) if x['type'] == 'page')
        with connect(page['webSocketDebuggerUrl'], max_size=8_000_000) as ws:
            seq = 0
            def call(method, params=None):
                global seq
                seq += 1; ws.send(json.dumps({'id':seq,'method':method,'params':params or {}}))
                while True:
                    result = json.loads(ws.recv())
                    if result.get('id') == seq:
                        assert 'error' not in result, result
                        return result['result']
            def js(expression):
                result = call('Runtime.evaluate', {'expression':expression,'returnByValue':True})
                assert 'exceptionDetails' not in result, result
                return result['result'].get('value')
            call('Page.enable')
            call('Page.addScriptToEvaluateOnNewDocument', {'source':'''window.testErrors=[];window.addEventListener('error',e=>testErrors.push(e.message));window.__TAURI__={core:{invoke:async (name,args)=>name==='set_appearance'?{dark:false,material:'solid'}:name==='app_version'?'0.12.23':name==='shell_integration_status'?{}:[]},event:{listen:async()=>()=>{}},window:{getCurrentWindow:()=>({onResized:async()=>()=>{},onMoved:async()=>()=>{}})}};'''})
            call('Page.navigate', {'url':f'http://127.0.0.1:{server.server_port}/index.html'})
            for _ in range(100):
                if js('!!document.querySelector("[data-action=compress]")'): break
                time.sleep(.1)
            else: raise AssertionError(js('({url:location.href,ready:document.readyState,errors:window.testErrors,body:document.body?.innerText.slice(0,500)})'))
            js('document.querySelector("[data-action=compress]").click()')
            assert js('!!document.querySelector("#qa-format")'), 'creation form did not open'
            result = js('''(()=>{let $=x=>document.querySelector(x);let set=(s,v)=>{let e=$(s);e.value=v;e.dispatchEvent(new Event('change',{bubbles:true}))};let out=[];for(let format of ['zip','7z','zipx','sfx']){set('#qa-format',format);set('#oa-method',format==='zip'?'Deflate':format==='zipx'?'XZ':'LZMA2');set('#qa-level','0');out.push([format,$('#oa-method').value,$('#qa-level').value]);set('#oa-method',format==='zip'?'Deflate':format==='zipx'?'XZ':'LZMA2');out.push([format,$('#oa-method').value,$('#qa-level').value])}return out})()''')
            assert result == [['zip','Store','0'],['zip','Deflate','5'],['7z','Copy','0'],['7z','LZMA2','5'],['zipx','Store','0'],['zipx','XZ','5'],['sfx','Copy','0'],['sfx','LZMA2','5']], result
            extra = js('''(()=>{let $=x=>document.querySelector(x),set=(s,v)=>{let e=$(s);e.value=v;e.dispatchEvent(new Event('change',{bubbles:true}))};set('#qa-format','7z');set('#oa-method','LZ4');set('#oa-method','Copy');let copy=[$('#oa-method').value,$('#qa-level').value];$('#qa-new-password').value='test-only';$('#oa-password-again').value='test-only';set('#qa-format','tar');let notice=!$('#oa-password-unsupported').hidden;set('#qa-format','zip');return {copy,notice,password:$('#qa-new-password').value,confirmation:$('#oa-password-again').value}})()''')
            assert extra == {'copy':['Copy','0'],'notice':True,'password':'test-only','confirmation':'test-only'}, extra
            # With the advanced panel collapsed, leaving "only store" by level must not keep the hidden store method.
            roundtrip = js('''(()=>{let $=x=>document.querySelector(x),set=(s,v)=>{let e=$(s);e.value=v;e.dispatchEvent(new Event('change',{bubbles:true}))};let out=[];for(let format of ['zip','7z','zipx','sfx']){set('#qa-format',format);set('#oa-method','');set('#qa-level','0');set('#qa-level','5');out.push([format,$('#oa-method').value,$('#qa-level').value,$('#oa-compression-advanced').hidden])}return out})()''')
            assert roundtrip == [[format,'','5',True] for format in ['zip','7z','zipx','sfx']], roundtrip
            print(json.dumps({'passed':True,'cases':result}))
    finally:
        proc.terminate()
        try: proc.wait(timeout=10)
        except subprocess.TimeoutExpired: proc.kill(); proc.wait()
        server.shutdown()
