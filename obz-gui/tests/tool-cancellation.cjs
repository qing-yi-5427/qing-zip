const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.js'),'utf8');
async function run(error,status='failed',op='recover',result=null,settings={}){
 const nodes=new Map(),calls=[],requests=[];let progressVisible=false,cancelVisible=false,runVisible=false;
 const get=id=>{if(!nodes.has(id))nodes.set(id,{value:id==='operation'?op: ['parts-size','parts-unit'].includes(id)?'1':'',checked:false,disabled:false,textContent:'',className:'',classList:{contains:()=>true},replaceChildren(){},style:{setProperty(){},removeProperty(){}},hasAttribute(key){return key in this;},toggleAttribute(key,enabled){if(enabled)this[key]='';else delete this[key];},getAttribute(key){return this[key]??null;},setAttribute(key,value){this[key]=value;},removeAttribute(key){delete this[key];}});return nodes.get(id);};
 const env={archiveScope:false,fileOps:[],actions:{},automaticQueries:new Set(['capabilities']),cancellableOperations:new Set([op]),QingProgress:require('../frontend/progress.js'),passwordBook:{automaticSavingEnabled:()=>false},$:get,window:{},active:null,show(id,visible){if(id==='progress')progressVisible=visible;else if(id==='cancel')cancelVisible=visible;else if(id==='run')runVisible=visible;},lines:id=>id==='checksum-paths'?['fixture.txt']:[],invoke:async(name,args)=>{calls.push(name);if(name==='start_operation'){requests.push(args.request);return 17;}if(name==='wait_operation')return {status,error,result,progress:{percent:null}};},render(){assert.equal(status,'complete');},...settings};
 const start=source.indexOf("$('cancel').onclick=");
 vm.createContext(env);vm.runInContext(source.slice(start,source.indexOf('if (window.parent !== window)',start)),env);
 const task=get('form').onsubmit({preventDefault(){}});
 if(settings.ready)await settings.ready(get,task);
 await task;return {status:get('status'),busy:env.window.obzBusy,run:get('run'),cancel:get('cancel'),progress:get('progress'),progressVisible,cancelVisible,runVisible,calls,requests};
}
test('recovery cancellation is a normal cancelled state and restores controls',async()=>{const r=await run('operation cancelled','cancelled');assert.equal(r.status.textContent,'已取消');assert.equal(r.status.className,'');assert.equal(r.busy,false);assert.equal(r.run.disabled,false);assert.equal(r.cancel.disabled,true);assert.equal(r.cancelVisible,false);assert.deepEqual(r.calls,['start_operation','wait_operation','forget_operation']);});
test('recovery genuine errors remain visible and are not relabeled cancellation',async()=>{const r=await run('archive corrupted');assert.match(r.status.textContent,/archive corrupted/);assert.equal(r.status.className,'error');assert.equal(r.busy,false);});

test('typed cancellation works for every cancellable tool without relying on error language',async()=>{for(const op of ['recover','checksum','benchmark','file-split','file-combine','extract','test']){const r=await run('操作已中止','cancelled',op);assert.equal(r.status.textContent,'已取消',op);assert.equal(r.status.className,'',op);}});
test('a genuine failure mentioning cancelled stays an error',async()=>{const r=await run('cannot read cancelled.txt');assert.equal(r.status.className,'error');assert.match(r.status.textContent,/cannot read cancelled.txt/);});

test('successful tools persist the full track; invalid verification never appears complete',async()=>{const done=await run(null,'complete','test',{valid:true}),bad=await run(null,'complete','test',{valid:false});assert.equal(done.progressVisible,true);assert.equal(done.progress.getAttribute('aria-valuenow'),'100');assert.equal(bad.progressVisible,false);assert.equal(bad.progress.getAttribute('aria-valuenow'),null);});

test('automatic read tasks show only their result on success and permit retry on failure',async()=>{
 const op='capabilities';
  const ready=await run(null,'complete',op,[]);
  assert.deepEqual(ready.calls,['start_operation','wait_operation','forget_operation']);
  assert.equal(ready.requests[0].operation,op);
  assert.equal(ready.requests[0].path,'');
  assert.equal(ready.progressVisible,false);
  assert.equal(ready.cancelVisible,false);
  assert.equal(ready.runVisible,false);
  assert.equal(ready.status.textContent,'');
  const failed=await run('read failed','failed',op);
  assert.equal(failed.runVisible,true);
  assert.match(failed.status.textContent,/read failed/);
});

test('current-archive checksum submits the scope path even if the field changes',async()=>{
 const r=await run(null,'complete','checksum',{},{
  archiveScope:true,params:new URLSearchParams({path:'C:/archives/current.zip'})
 });
 assert.equal(r.requests[0].operation,'checksum');
 assert.equal(r.requests[0].path,'C:/archives/current.zip');
});

test('the cancellation acknowledgement cannot overwrite the terminal cancelled status',async()=>{
 let startWait,finishWait,finishCancel,finishForget;
 const started=new Promise(resolve=>{startWait=resolve;});
 const waiting=new Promise(resolve=>{finishWait=resolve;});
 const cancelling=new Promise(resolve=>{finishCancel=resolve;});
 const forgetting=new Promise(resolve=>{finishForget=resolve;});
 const r=await run(null,'cancelled','checksum',null,{
  async invoke(name){
   if(name==='start_operation')return 51;
   if(name==='wait_operation'){startWait();return waiting;}
   if(name==='cancel_operation')return cancelling;
   if(name==='forget_operation')return forgetting;
  },
  async ready(get){
   await started;
   const cancel=get('cancel').onclick();
   assert.equal(get('status').textContent,'正在取消…');
   finishWait({status:'cancelled'});
   while(get('status').textContent!=='已取消')await Promise.resolve();
   finishCancel();
   await cancel;
   finishForget();
  }
 });
 assert.equal(r.status.textContent,'已取消');
 assert.equal(r.cancelVisible,false);
});
test('an invalid integrity result has an explicit failure message',()=>{
 const start=source.indexOf('function render(op,result)');
 let message;
 const env={$:()=>({replaceChildren(){}}),text(value){message=value;}};
 vm.createContext(env);
 vm.runInContext(source.slice(start,source.indexOf("$('cancel').onclick=",start)),env);
 env.render('test',{valid:false});
 assert.equal(message,'完整性检查未通过。');
});
