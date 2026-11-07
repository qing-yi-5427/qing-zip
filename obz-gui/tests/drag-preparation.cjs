const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const text=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const code=text.slice(text.indexOf('let dragGesture=null;'),text.indexOf('const scrollTimers='));
const run=text.slice(text.indexOf('async function run('),text.indexOf('async function finishExtraction('));
const tick=()=>new Promise(resolve=>setImmediate(resolve));
function deferred(){let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
function fixture(){
 const start=deferred(),wait=deferred(),finish=deferred(),calls=[],toasts=[],events=new Map();
 let captured=false;
 function target(prefix){return {addEventListener:(type,fn)=>events.set(prefix+type,fn)};}
 const context={
  dragCandidate:null,dragBusy:false,suppressRowClick:false,lastSelectShift:false,overlay:{hidden:true},
  s:{active:null,jobs:[],selected:new Set(['a.bin']),archive:'sample.7z',view:'files'},
  root:{...target('root:'),setPointerCapture(){captured=true},hasPointerCapture(){return captured},
   releasePointerCapture(id){captured=false;events.get('root:lostpointercapture')({pointerId:id,target:this});}},
  window:target('window:'),crypto:{randomUUID:()=> '21968181-d961-4b93-bfb1-0b36a1b9849a'},
  toast:t=>toasts.push(t),invoke:async(name,args)=>{
   calls.push({name,args});if(name==='start_drag_preparation')return start.promise;
   if(name==='wait_operation')return wait.promise;if(name==='finish_drag')return finish.promise;
   if(!['cancel_drag_preparation','forget_operation','discard_drag'].includes(name))throw Error(name);
  }
 };
 vm.runInNewContext(run+'\n'+code+'\nthis.begin=dragSelection;',context);
 return {start,wait,finish,calls,toasts,context,events};
}
const complete={status:'complete',result:{files:1,destination:'owned temporary export'},progress:{}};
test('native cancellation outside the WebView uses the job state even when the decoder reports its own error',async()=>{
 const f=fixture(),result=f.context.begin(2);f.start.resolve(8);await tick();
 f.wait.resolve({status:'cancelled',error:'decoder interrupted',progress:{}});
 await result;
 assert.equal(f.context.s.jobs[0].status,'cancelled');assert.match(f.toasts.at(-1),/已取消/);
 assert.ok(!f.calls.some(x=>x.name==='finish_drag'));
});
test('release before the native job ID arrives cancels and replays only the matching drag token',async()=>{
 const f=fixture(),result=f.context.begin(8);
 assert.equal(f.context.s.active,'starting');
 f.events.get('window:pointerup')({pointerId:5});
 assert.equal(f.calls.filter(x=>x.name==='cancel_drag_preparation').length,0);
 f.events.get('window:pointerup')({pointerId:8});f.start.resolve(19);await tick();
 const cancels=f.calls.filter(x=>x.name==='cancel_drag_preparation');
 assert.equal(cancels.length,2);assert.equal(cancels[0].args.token,cancels[1].args.token);
 assert.equal(cancels[0].args.token,f.calls[0].args.token);
 f.wait.resolve({status:'cancelled',progress:{},error:'operation cancelled'});await result;
 assert.equal(f.context.s.jobs[0].id,19);assert.equal(f.context.s.active,null);
 assert.ok(!f.calls.some(x=>['cancel_operation','finish_drag','clipboard_directory','clipboard_paths'].includes(x.name)));
 assert.equal(f.calls.at(-1).name,'discard_drag');
});
test('Escape and pointer cancellation cancel the preparing drag',async()=>{
 for(const type of ['window:keydown','window:pointercancel']){
  const f=fixture(),result=f.context.begin(2);f.start.resolve(17);await tick();
  f.events.get(type)({key:'Escape',pointerId:2,target:f.context.root,preventDefault(){},stopImmediatePropagation(){}});
  assert.ok(f.calls.some(x=>x.name==='cancel_drag_preparation'),type);
  f.wait.resolve(complete);await result;assert.ok(!f.calls.some(x=>x.name==='finish_drag'),type);
 }
});
test('handoff survives focus and capture transfer, waits for verification, and honors the completed drop',async()=>{
 const f=fixture(),result=f.context.begin(2);f.start.resolve(5);await tick();
 f.events.get('root:lostpointercapture')({pointerId:2,target:{}});
 f.events.get('root:lostpointercapture')({pointerId:2,target:f.context.root});
 f.events.get('window:blur')({});
 assert.ok(!f.calls.some(x=>x.name==='cancel_drag_preparation'));
 assert.ok(!f.calls.some(x=>x.name==='finish_drag'));
 f.wait.resolve(complete);await tick();
 assert.ok(f.calls.some(x=>x.name==='finish_drag'));
 assert.ok(!f.calls.some(x=>x.name==='cancel_drag_preparation'));
 f.events.get('window:pointerup')({pointerId:2});f.finish.resolve(true);await result;
 assert.match(f.toasts.at(-1),/复制/);assert.equal(f.calls.at(-1).name,'discard_drag');
});
test('registration, extraction and OLE failures each release only the dedicated drag export',async()=>{
 for(const when of ['start','extract','shell']){
  const f=fixture(),result=f.context.begin(2);
  if(when==='start')f.start.reject(Error('fixture failure'));
  else {f.start.resolve(7);await tick();
   if(when==='extract')f.wait.resolve({status:'failed',error:'fixture failure',progress:{}});
   else {f.wait.resolve(complete);await tick();f.finish.reject(Error('fixture failure'));}
  }
  await assert.rejects(result,/fixture failure/);assert.equal(f.context.s.active,null);
  assert.equal(f.calls.at(-1).name,'discard_drag');
  assert.equal(f.calls.filter(x=>x.name==='finish_drag').length,when==='shell'?1:0);
 }
});
