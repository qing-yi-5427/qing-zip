const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.js'),'utf8');
async function run(error,status='failed',op='recover'){
 const nodes=new Map(),calls=[];
 const get=id=>{if(!nodes.has(id))nodes.set(id,{value:id==='operation'?op: ['parts-size','parts-unit'].includes(id)?'1':'',checked:false,disabled:false,textContent:'',className:'',classList:{contains:()=>true},replaceChildren(){},setAttribute(){},removeAttribute(){}});return nodes.get(id);};
 const env={$:get,window:{},active:null,show(){},lines:id=>id==='checksum-paths'?['fixture.txt']:[],invoke:async(name,args)=>{calls.push(name);if(name==='start_operation')return 17;if(name==='wait_operation')return {status,error,progress:{percent:null}};},render(){throw Error('unexpected success render');}};
 const start=source.indexOf("$('form').onsubmit=");
 vm.createContext(env);vm.runInContext(source.slice(start,source.indexOf('if (window.parent !== window)',start)),env);
 await get('form').onsubmit({preventDefault(){}});return {status:get('status'),busy:env.window.obzBusy,run:get('run'),cancel:get('cancel'),calls};
}
test('recovery cancellation is a normal cancelled state and restores controls',async()=>{const r=await run('operation cancelled','cancelled');assert.equal(r.status.textContent,'已取消');assert.equal(r.status.className,'');assert.equal(r.busy,false);assert.equal(r.run.disabled,false);assert.equal(r.cancel.disabled,true);assert.deepEqual(r.calls,['start_operation','wait_operation','forget_operation']);});
test('recovery genuine errors remain visible and are not relabeled cancellation',async()=>{const r=await run('archive corrupted');assert.match(r.status.textContent,/archive corrupted/);assert.equal(r.status.className,'error');assert.equal(r.busy,false);});

test('typed cancellation works for every cancellable tool without relying on error language',async()=>{for(const op of ['recover','checksum','benchmark','file-split','file-combine','extract','test']){const r=await run('操作已中止','cancelled',op);assert.equal(r.status.textContent,'已取消',op);assert.equal(r.status.className,'',op);}});
test('a genuine failure mentioning cancelled stays an error',async()=>{const r=await run('cannot read cancelled.txt');assert.equal(r.status.className,'error');assert.match(r.status.textContent,/cannot read cancelled.txt/);});
