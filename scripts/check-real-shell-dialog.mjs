// Drive only the isolated WebView2 instance launched by check_classic_shell.py.
// CDP reads rendered controls and sends mouse input. Archive operations still go
// through the shipping GUI, shell registration, single-instance path and engine.
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {setTimeout as delay} from 'node:timers/promises';
import {writeFile} from 'node:fs/promises';
import path from 'node:path';

const [exe, probe, port, pid, first, second, screenshotPath] = process.argv.slice(2);
assert.equal(process.env.GITHUB_ACTIONS, 'true');
assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
assert.ok(Number.isInteger(+port) && +port > 0 && +port < 65536);
const endpoint = 'http://127.0.0.1:'+port;
async function until(read, label, timeout=45000) {
  const deadline = Date.now()+timeout;
  while (Date.now()<deadline) {
    const result = await read();
    if (result) return result;
    await delay(120);
  }
  throw new Error('Timed out waiting for '+label);
}
const target = await until(async()=>{
  let pages;
  try { pages=await (await fetch(endpoint+'/json/list',{signal:AbortSignal.timeout(1500)})).json(); }
  catch { return false; }
  return pages.find(p=>p.type==='page' && /^https?:\/\/tauri\.localhost(?:\/|$)/i.test(p.url));
},'isolated Tauri WebView2 target',60000);
const address=new URL(target.webSocketDebuggerUrl);
assert.ok(['127.0.0.1','localhost','[::1]'].includes(address.hostname) && address.port===port);
const socket=new WebSocket(address);
let id=0;
const pending=new Map();
function call(method,params={}) {
  const key=++id;
  return new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{pending.delete(key);reject(new Error('CDP timed out: '+method));},12000);
    pending.set(key,msg=>{clearTimeout(timer);if(msg.error) reject(new Error(JSON.stringify(msg.error)));else resolve(msg.result);});
    socket.send(JSON.stringify({id:key,method,params}));
  });
}
socket.addEventListener('message',event=>{
  const msg=JSON.parse(event.data);
  if(pending.has(msg.id)){const finish=pending.get(msg.id);pending.delete(msg.id);finish(msg);}
});
await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
async function read(expression) {
  const r=await call('Runtime.evaluate',{expression,returnByValue:true});
  if(r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
}
async function click(selector) {
  const s=JSON.stringify(selector);
  await until(()=>read('Boolean(document.querySelector('+s+'))'), selector);
  const {root}=await call('DOM.getDocument');
  const {nodeId}=await call('DOM.querySelector',{nodeId:root.nodeId,selector});
  assert.ok(nodeId,selector);
  await call('DOM.scrollIntoViewIfNeeded',{nodeId});
  const point=await read('(()=>{const e=document.querySelector('+s+'), b=e.getBoundingClientRect();const x=b.left+b.width/2,y=b.top+b.height/2;return e.contains(document.elementFromPoint(x,y))?{x,y}:null})()');
  assert.ok(point,'Control is covered: '+selector);
  await call('Input.dispatchMouseEvent',{type:'mouseMoved',...point});
  await call('Input.dispatchMouseEvent',{type:'mousePressed',button:'left',buttons:1,clickCount:1,...point});
  await call('Input.dispatchMouseEvent',{type:'mouseReleased',button:'left',buttons:0,clickCount:1,...point});
}
const menu='#oa-file-menu';
const setting='[data-view="settings"]';
const fileNav='[data-view="files"]';
const canonical=p=>path.win32.normalize(p.replace(/^\\\\\?\\/,'')).toLowerCase();
try {
  await until(()=>read('Boolean(document.querySelector("#qing-archive #oa-toolbar [data-action=open]"))'),'workbench startup');
  await click(setting);
  await until(()=>read('Boolean(document.querySelector("#oa-file-menu"))'),'menu setting');
  assert.equal(await read('document.querySelector("#oa-file-menu").checked'),false);
  await click(menu);
  await until(()=>read('document.querySelector("#oa-file-menu")?.checked && !document.querySelector("#oa-file-menu").disabled'),'persisted menu setting');
  await click(fileNav);
  assert.equal(await read('document.querySelectorAll("#qa-overlay:not([hidden]) [role=dialog]").length'),0);

  const invocation=JSON.parse(execFileSync(probe,[exe,first,second,pid],{encoding:'utf8',timeout:60000}));
  assert.equal(invocation.real_primary_pid,Number(pid));assert.equal(invocation.invoked_for_files,2);
  const state=await until(()=>read(`(()=>{
    const overlay=document.querySelector('#qa-overlay:not([hidden])');
    if(!overlay || !overlay.querySelector('#oa-source-count'))return null;
    return {dialogs:overlay.querySelectorAll('[role=dialog]').length,title:overlay.querySelector('h2')?.textContent,
      rows:[...overlay.querySelectorAll('#oa-sources .qa-source-row small')].map(e=>e.textContent),
      inputs:overlay.querySelector('#qa-inputs')?.value.split(/\\r?\\n/),save:overlay.querySelector('#qa-save')?.value,
      format:overlay.querySelector('#qa-format')?.value,verify:overlay.querySelector('#oa-test-after')?.checked};
  })()`),'one populated creation dialog');
  const expected=[first,second].map(canonical).sort();
  assert.equal(state.dialogs,1);assert.equal(state.title,'新建压缩文件');
  assert.deepEqual(state.rows.map(canonical).sort(),expected);assert.deepEqual(state.inputs.map(canonical).sort(),expected);
  assert.equal(state.format,'zip');assert.equal(state.verify,true);
  assert.equal(canonical(state.save),canonical(path.join(path.dirname(first),path.basename(path.dirname(first))+'.zip')));
  const image=await call('Page.captureScreenshot',{format:'png',fromSurface:true});
  await writeFile(screenshotPath,Buffer.from(image.data,'base64'));
  await click('[data-action="startcompress"]');
  await until(()=>read(`(()=>{const job=document.querySelector('#qa-overlay[data-page="jobs"]');
    return job?.querySelector('.qa-jobhead strong')?.textContent==='测试新压缩文件' &&
      job.querySelector('.qa-jobresult')?.textContent==='完整性检查通过' &&
      !job.querySelector('.qa-dialog-footer [data-action=close]').disabled;})()`),'successful create and post-create validation',90000);
  await click('#qa-overlay .qa-dialog-footer [data-action=close]');
  await until(()=>read('document.querySelector("#qa-overlay").hidden'),'closed progress dialog');
  await delay(900);
  assert.equal(await read('document.querySelector("#qa-overlay").hidden'),true,'an extra source group opened another dialog');
  await click(setting);
  await until(()=>read('document.querySelector("#oa-file-menu")?.checked===true'),'enabled right-click setting');
  await click(menu);
  await until(()=>read('document.querySelector("#oa-file-menu")?.checked===false && !document.querySelector("#oa-file-menu").disabled'),'removal of right-click setting');
  console.log(JSON.stringify({primary_pid:invocation.real_primary_pid,dialog_count:state.dialogs,title:state.title,sources:state.rows,
    output:state.save,format:state.format,created_with_integrity_check:true}));
} catch(error) {
  const ui=await read('({title:document.title,overlay:document.querySelector("#qa-overlay")?.innerText?.slice(-1400),main:document.querySelector("#qa-main")?.innerText?.slice(-1300),toast:document.querySelector("#qa-toast")?.innerText})').catch(()=>null);
  console.error('GUI state:',JSON.stringify(ui));throw error;
} finally { socket.close(); }
