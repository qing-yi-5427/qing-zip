const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.js'),'utf8');
async function run(error,status='failed',op='recover',result=null){
 const nodes=new Map(),calls=[];let progressVisible=false;
 const get=id=>{if(!nodes.has(id))nodes.set(id,{value:id==='operation'?op: ['parts-size','parts-unit'].includes(id)?'1':'',checked:false,disabled:false,textContent:'',className:'',classList:{contains:()=>true},replaceChildren(){},style:{setProperty(){},removeProperty(){}},hasAttribute(key){return key in this;},toggleAttribute(key,enabled){if(enabled)this[key]='';else delete this[key];},getAttribute(key){return this[key]??null;},setAttribute(key,value){this[key]=value;},removeAttribute(key){delete this[key];}});return nodes.get(id);};
 const env={QingProgress:require('../frontend/progress.js'),$:get,window:{},active:null,show(id,visible){if(id==='progress')progressVisible=visible;},lines:id=>id==='checksum-paths'?['fixture.txt']:[],invoke:async(name,args)=>{calls.push(name);if(name==='start_operation')return 17;if(name==='wait_operation')return {status,error,result,progress:{percent:null}};},render(){assert.equal(status,'complete');}};
 const start=source.indexOf("$('form').onsubmit=");
 vm.createContext(env);vm.runInContext(source.slice(start,source.indexOf('if (window.parent !== window)',start)),env);
 await get('form').onsubmit({preventDefault(){}});return {status:get('status'),busy:env.window.obzBusy,run:get('run'),cancel:get('cancel'),progress:get('progress'),progressVisible,calls};
}
test('recovery cancellation is a normal cancelled state and restores controls',async()=>{const r=await run('operation cancelled','cancelled');assert.equal(r.status.textContent,'已取消');assert.equal(r.status.className,'');assert.equal(r.busy,false);assert.equal(r.run.disabled,false);assert.equal(r.cancel.disabled,true);assert.deepEqual(r.calls,['start_operation','wait_operation','forget_operation']);});
test('recovery genuine errors remain visible and are not relabeled cancellation',async()=>{const r=await run('archive corrupted');assert.match(r.status.textContent,/archive corrupted/);assert.equal(r.status.className,'error');assert.equal(r.busy,false);});

test('typed cancellation works for every cancellable tool without relying on error language',async()=>{for(const op of ['recover','checksum','benchmark','file-split','file-combine','extract','test']){const r=await run('操作已中止','cancelled',op);assert.equal(r.status.textContent,'已取消',op);assert.equal(r.status.className,'',op);}});
test('a genuine failure mentioning cancelled stays an error',async()=>{const r=await run('cannot read cancelled.txt');assert.equal(r.status.className,'error');assert.match(r.status.textContent,/cannot read cancelled.txt/);});

test('successful tools persist the full track; invalid verification never appears complete',async()=>{const done=await run(null,'complete','test',{valid:true}),bad=await run(null,'complete','test',{valid:false});assert.equal(done.progressVisible,true);assert.equal(done.progress.getAttribute('aria-valuenow'),'100');assert.equal(bad.progressVisible,false);assert.equal(bad.progress.getAttribute('aria-valuenow'),null);});
