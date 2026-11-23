"""Capture a running Tauri menu using an isolated WebView2 profile.

Requires Python websockets; outputs contain no user files or paths.
"""
import argparse, base64, json, os, subprocess, tempfile, time, urllib.request
from pathlib import Path
from websockets.sync.client import connect

p = argparse.ArgumentParser()
p.add_argument('--exe', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--candidate', action='store_true')
p.add_argument('--launch-sample', action='store_true',
               help='Measure one fresh launch and two process-tree private-byte samples (requires psutil)')
a = p.parse_args()
if a.launch_sample:
    import psutil
    app_names = {a.exe.name.lower(), 'qing-zip.exe', 'obz-gui.exe', 'menu-baseline.exe'}
    if any((p.info['name'] or '').lower() in app_names for p in psutil.process_iter(['name'])):
        raise RuntimeError('Close the existing qing-zip instance before collecting a launch sample')
a.out.mkdir(parents=True, exist_ok=False)
with tempfile.TemporaryDirectory(prefix='qz-menu-', ignore_cleanup_errors=True) as tmp:
    root = Path(tmp)
    profile = root / 'profile'
    env = dict(os.environ, LOCALAPPDATA=str(root / 'local'), WEBVIEW2_USER_DATA_FOLDER=str(profile), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=0 --remote-debugging-address=127.0.0.1')
    started = time.perf_counter()
    proc = subprocess.Popen([str(a.exe.resolve())], cwd=root, env=env)
    ws = None
    try:
        for _ in range(450):
            assert proc.poll() is None, 'application exited'
            ports = list(profile.rglob('DevToolsActivePort'))
            if ports:
                try:
                    port = int(ports[0].read_text().splitlines()[0])
                    pages = json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json', timeout=1))
                    ws = connect(next(x['webSocketDebuggerUrl'] for x in pages if x.get('type') == 'page'), max_size=20_000_000)
                    break
                except Exception:
                    pass
            time.sleep(.1)
        assert ws, 'WebView2 debugging endpoint unavailable'
        seq = [0]
        def call(method, params=None):
            seq[0] += 1
            ws.send(json.dumps({'id': seq[0], 'method': method, 'params': params or {}}))
            while True:
                r = json.loads(ws.recv())
                if r.get('id') == seq[0]:
                    assert 'error' not in r, r
                    return r['result']
        def js(expression, promise=False):
            r = call('Runtime.evaluate', {'expression': expression, 'returnByValue': True, 'awaitPromise': promise})
            assert 'exceptionDetails' not in r, r
            return r['result'].get('value')
        for _ in range(400):
            if js('!!document.querySelector("[data-action=app-menu]") && !!window.__TAURI__?.core'): break
            time.sleep(.1)
        else: raise RuntimeError('UI not ready')
        if a.launch_sample:
            for _ in range(250):
                if js("document.visibilityState==='visible' && !!document.querySelector('#oa-home-smart-drop')?.getBoundingClientRect().width"):
                    break
                time.sleep(.02)
            else: raise RuntimeError('Home action did not become visible')
            js('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve(true))))', True)
            sample = {'homeReadyMs': round((time.perf_counter() - started) * 1000, 1)}
            app = psutil.Process(proc.pid)
            def memory():
                tree = [app] + app.children(recursive=True)
                return {'processes': len(tree), 'privateBytes': sum(p.memory_info().private for p in tree)}
            time.sleep(1)
            sample['home'] = memory()
            js("document.querySelector('[data-action=app-menu]').click()")
            if not js("!document.querySelector('#oa-menu').hidden && document.querySelector('#oa-menu').getBoundingClientRect().height>0"):
                raise RuntimeError('Menu did not open')
            time.sleep(1)
            sample['menu'] = memory()
            with (a.out / 'launch.json').open('x', encoding='utf-8') as file:
                json.dump(sample, file, indent=2)
            raise SystemExit(0)
        call('Page.enable')
        time.sleep(1)
        report = {'userAgent': js('navigator.userAgent'), 'dpr': js('devicePixelRatio'), 'viewport': js('[innerWidth,innerHeight]'), 'initialMaterial': js('document.querySelector("#qing-archive").dataset.material'), 'backdropFilterSupported': js('CSS.supports("backdrop-filter", "blur(1px)")'), 'cases': {}}
        for name, theme, features, material in [('light','light',[],'acrylic'), ('dark','dark',[],'acrylic'), ('mica','light',[],'mica'), ('solid','light',[],'solid'), ('forced','light',[{'name':'forced-colors','value':'active'}],'acrylic'), ('reduced','light',[{'name':'prefers-reduced-motion','value':'reduce'}],'acrylic')]:
            call('Emulation.setEmulatedMedia', {'features': []})
            js('document.querySelector("[data-view=settings]").click()')
            js(f'''(()=>{{let s=document.querySelector('#qa-material');s.value='{material}';s.dispatchEvent(new Event('change',{{bubbles:true}}))}})()''')
            time.sleep(1.5)  # set_appearance returns the effective native material asynchronously
            call('Emulation.setEmulatedMedia', {'features': features})
            js(f'document.querySelector("[data-theme={theme}]").click()')
            time.sleep(1.5)
            js('document.querySelector("[data-view=files]").click()')
            js('document.querySelector("[data-action=app-menu]").click()')
            time.sleep(.3)
            case = js('''(()=>{let e=document.querySelector('#oa-menu'),s=getComputedStyle(e),r=e.getBoundingClientRect();return {background:s.backgroundColor,filter:s.backdropFilter,color:s.color,border:s.borderColor,rect:{x:r.x,y:r.y,width:r.width,height:r.height},items:e.querySelectorAll('button').length,material:document.querySelector('#qing-archive').dataset.material,forced:matchMedia('(forced-colors: active)').matches,reduced:matchMedia('(prefers-reduced-motion: reduce)').matches}})()''')
            assert case['items'] == 7 and case['rect']['width'] > 0, case
            assert case['rect']['x'] >= 0 and case['rect']['y'] >= 0 and case['rect']['x'] + case['rect']['width'] <= report['viewport'][0] and case['rect']['y'] + case['rect']['height'] <= report['viewport'][1], case
            if name == 'solid': assert case['material'] == 'solid', case
            if name == 'forced': assert case['material'] == 'solid', case
            case['surfaceImage'] = js("getComputedStyle(document.querySelector('#oa-menu')).backgroundImage")
            assert case['filter'] == 'none', case
            if a.candidate and case['material'] in ('acrylic', 'mica') and not case['forced']:
                assert case['surfaceImage'].startswith('linear-gradient('), case
            else:
                assert case['surfaceImage'] == 'none', case
            (a.out / (name + '.png')).write_bytes(base64.b64decode(call('Page.captureScreenshot', {'format':'png'})['data']))
            case['frameIntervalsMs'] = js('''new Promise(resolve=>{let t=[],last;function tick(n){requestAnimationFrame(now=>{if(last!==undefined)t.push(now-last);last=now;if(n)tick(n-1);else resolve(t)})}tick(120)})''', True)
            case['openToTwoFramesMs'] = js('''(async()=>{let times=[];for(let i=0;i<20;i++){document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));if(!document.querySelector('#oa-menu').hidden)throw Error('Escape did not close menu');await new Promise(r=>requestAnimationFrame(r));let start=performance.now();document.querySelector('[data-action=app-menu]').click();if(document.querySelector('#oa-menu').hidden)throw Error('menu did not open');await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));times.push(performance.now()-start)}return times})()''', True)
            report['cases'][name] = case
            js('document.querySelector("[data-action=app-menu]").click()')
        call('Emulation.setEmulatedMedia', {'features': []})
        report['simulatedDpi'] = {}
        for scale in (1, 2):
            call('Emulation.setDeviceMetricsOverride', {'width':780, 'height':620, 'deviceScaleFactor':scale, 'mobile':False})
            js('''(()=>{let e=document.querySelector('#oa-menu');if(!e.hidden)document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));if(!e.hidden)throw Error('menu did not close')})()''')
            js('document.querySelector("[data-action=app-menu]").click()')
            geometry = js('''(()=>{let e=document.querySelector('#oa-menu'),r=e.getBoundingClientRect();return {hidden:e.hidden,x:r.x,y:r.y,width:r.width,height:r.height,viewport:[innerWidth,innerHeight],dpr:devicePixelRatio}})()''')
            assert not geometry['hidden'] and geometry['width'] > 0 and geometry['height'] > 0 and geometry['x'] >= 0 and geometry['y'] >= 0 and geometry['x'] + geometry['width'] <= 780 and geometry['y'] + geometry['height'] <= 620, geometry
            report['simulatedDpi'][str(scale)] = geometry
        call('Emulation.clearDeviceMetricsOverride')
        (a.out / 'metrics.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    finally:
        if ws:
            try: js('window.__TAURI__.core.invoke("window_action",{action:"close"})')
            except Exception: pass
            ws.close()
        try: proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.terminate(); proc.wait(timeout=10)
        # WebView2 children release their profile locks asynchronously.
        time.sleep(3)
