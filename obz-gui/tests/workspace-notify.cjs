const test=require('node:test'),assert=require('node:assert/strict');
const {create}=require('../frontend/file-workspace.js');
const tick=()=>new Promise(setImmediate),settle=()=>new Promise(r=>setTimeout(r,220));
function harness(watching=false){
 const handlers={},calls=[],errors=[];let entries=[],failure=null,renders=0,stamp='initial';
 const ui={esc:String,icon:()=>'',render(){renders++},error:e=>errors.push(e),invoke:async(name,args)=>{
  calls.push([name,args]);if(name==='watch_local')return;
  if(failure)throw Error(failure);return {path:args.path,parent:'C:/',entries,revision:stamp};
 }};
 if(watching)ui.probe=async(path,names)=>{calls.push(['probe',{path,names}]);if(failure)throw Error(failure);return {revision:stamp,entries:[]};};
 const ws=create(ui),section={addEventListener:(k,f)=>handlers[k]=f};
 for(const p of ws.state.panes)Object.assign(p,{loaded:true,path:'C:/initial',revision:'initial'});
 const mount=()=>ws.mount({querySelector:()=>section});mount();
 const click=(action,path,index=0)=>handlers.click({preventDefault(){},target:{closest:s=>s==='[data-fm]'?{dataset:{fm:action,pane:String(index),path}}:null}});
 return {ws,calls,errors,click,mount,setEntries(v){entries=v},fail(v){failure=v},stamp(v){stamp=v},get renders(){return renders}};
}
test('per-pane history is bounded, deduplicated and forward branch is replaced',async()=>{
 const h=harness();for(let i=0;i<55;i++){h.click('tree','C:/'+i);await tick();}
 const p=h.ws.state.panes[0];assert.equal(p.history.length,50);assert.equal(p.history[0],'C:/5');assert.equal(p.historyIndex,49);
 h.click('tree','C:/54');await tick();assert.equal(p.history.length,50);
 h.click('back');await tick();assert.equal(p.path,'C:/53');assert.equal(p.historyIndex,48);
 h.click('forward');await tick();assert.equal(p.path,'C:/54');
 h.click('back');await tick();h.click('tree','C:/branch');await tick();assert.equal(p.history.at(-1),'C:/branch');assert.ok(!p.history.includes('C:/54'));assert.equal(h.ws.state.panes[1].history.length,0);
 h.fail('missing directory');h.click('back');await tick();assert.equal(p.path,'C:/branch');assert.equal(p.historyIndex,p.history.length-1);assert.match(p.error,/missing/);
});

test('metadata polling is bounded, preserves selection and does not browse unchanged directories',async()=>{
 const h=harness(true),p=h.ws.state.panes[0],first={path:'C:/initial/a',name:'a',size:1,is_dir:false};h.setEntries([first]);p.entries=[first];await settle();
 p.selected.add(first.path);p.cursor=p.anchor=first.path;const reads=h.calls.filter(x=>x[0]==='browse_local').length;
 await new Promise(r=>setTimeout(r,2100));assert.equal(h.calls.filter(x=>x[0]==='browse_local').length,reads);
 h.setEntries([{...first,size:5}]);h.stamp('grown');await new Promise(r=>setTimeout(r,2100));assert.equal(p.entries[0].size,5);assert.deepEqual([...p.selected],[first.path]);assert.equal(p.cursor,first.path);
 h.ws.suspend();const stopped=h.calls.length;await new Promise(r=>setTimeout(r,2100));assert.equal(h.calls.length,stopped);
});
test('removed directories are reported once and automatically stop probes',async()=>{
 const h=harness(true);await settle();h.fail('directory disappeared');await new Promise(r=>setTimeout(r,2100));const p=h.ws.state.panes[0];assert.match(p.error,/无法读取.*directory disappeared/);assert.equal(p.entries.length,0);const stopped=h.calls.length;await new Promise(r=>setTimeout(r,2100));assert.equal(h.calls.length,stopped);h.ws.suspend();
});
test('a delayed metadata result cannot trigger a browse after suspension',async()=>{
 let done;const handlers={},calls=[];const ws=create({esc:String,icon:()=>'',render(){},error:e=>{throw e},probe:()=>new Promise(r=>done=r),invoke:async()=>{calls.push('browse');return {path:'C:/a',parent:'C:/',entries:[]}}});
 Object.assign(ws.state.panes[0],{loaded:true,path:'C:/a'});Object.assign(ws.state.panes[1],{loaded:true,path:''});ws.mount({querySelector:()=>({addEventListener:(k,f)=>handlers[k]=f})});await settle();ws.suspend();done('changed');await tick();assert.deepEqual(calls,[]);
});

 test('unchanged manual refresh resumes polling without a render',async()=>{
 const h=harness(true);await settle();const p=h.ws.state.panes[0];h.click('refresh');await tick();assert.ok(p.timer,'manual refresh must schedule next metadata probe');
 const probes=h.calls.filter(x=>x[0]==='probe').length;await new Promise(r=>setTimeout(r,2100));assert.ok(h.calls.filter(x=>x[0]==='probe').length>probes);h.ws.suspend();assert.equal(p.timer,null);
});
