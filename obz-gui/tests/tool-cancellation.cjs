const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.js'),'utf8');
async function run(error){
 const nodes=new Map(),calls=[];
 const get=id=>{if(!nodes.has(id))nodes.set(id,{value:id==='operation'?'recover':'',checked:false,disabled:false,textContent:'',className:'',classList:{contains:()=>true},replaceChildren(){},removeAttribute(){}});return nodes.get(id);};
 const env={$:get,window:{},active:null,show(){},lines:()=>[],invoke:async(name,args)=>{calls.push(name);if(name==='start_operation')return 17;if(name==='wait_operation')return {status:'failed',error,progress:{percent:null}};},render(){throw Error('unexpected success render');}};
 vm.createContext(env);vm.runInContext(source.slice(source.indexOf("$('form').onsubmit="),source.indexOf('document.documentElement.style.colorScheme')),env);
 await get('form').onsubmit({preventDefault(){}});return {status:get('status'),busy:env.window.obzBusy,run:get('run'),cancel:get('cancel'),calls};
}
test('recovery cancellation is a normal cancelled state and restores controls',async()=>{const r=await run('operation cancelled');assert.equal(r.status.textContent,'已取消');assert.equal(r.status.className,'');assert.equal(r.busy,false);assert.equal(r.run.disabled,false);assert.equal(r.cancel.disabled,true);assert.deepEqual(r.calls,['start_operation','wait_operation','forget_operation']);});
test('recovery genuine errors remain visible and are not relabeled cancellation',async()=>{const r=await run('archive corrupted');assert.match(r.status.textContent,/archive corrupted/);assert.equal(r.status.className,'error');assert.equal(r.busy,false);});
