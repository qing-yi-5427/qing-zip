const test=require('node:test'),assert=require('node:assert/strict');
const {create}=require('../frontend/file-workspace.js');
function setup(run){
 const handlers={},errors=[],calls=[];
 const ws=create({esc:String,icon:()=>'',render(){},error:e=>errors.push(e),run,
 invoke:async(_,v)=>{calls.push(v.path);return {path:v.path,parent:'C:/',entries:[{name:'新文件.txt',path:v.path+'/新文件.txt',size:0,is_dir:false}]};}});
 for(const [i,p] of ws.state.panes.entries())Object.assign(p,{loaded:true,path:i?'D:/right':'C:/left'});
 ws.mount({querySelector:()=>({addEventListener:(k,f)=>handlers[k]=f})});
 const click=(action,index)=>handlers.click({preventDefault(){},target:{closest:s=>s==='[data-fm]'?{dataset:{fm:action,pane:String(index)}}:null}});
 const input=(index,value)=>handlers.input({target:{dataset:{fmNewName:String(index)},value}});
 const submit=index=>handlers.submit({preventDefault(){},target:{closest:s=>s==='[data-fm-create]'?{dataset:{fmCreate:String(index)}}:null}});
 return {ws,handlers,errors,calls,click,input,submit};
}
const tick=()=>new Promise(setImmediate);
test('new local item binds pane, ignores transfer overwrite, then selects published item',async()=>{
 const requests=[];const t=setup(async body=>{requests.push(body);return {path:body.path+'/'+body.name};});
 t.ws.state.overwrite='overwrite';t.click('new-file',1);t.input(1,'新文件.txt');t.submit(1);await tick();
 assert.deepEqual(requests,[{operation:'fs-create-file',path:'D:/right',name:'新文件.txt',options:{overwrite:'error'}}]);
 assert.deepEqual(t.calls,['D:/right']);assert.deepEqual([...t.ws.state.panes[1].selected],['D:/right/新文件.txt']);
 assert.equal(t.ws.state.panes[0].selected.size,0);assert.equal(t.ws.state.busy,false);
});
test('failed create keeps name and folder mode for correction',async()=>{
 const t=setup(async()=>{throw Error('commit private output: exists (os error 183)');});t.click('new-directory',0);t.input(0,'existing');t.submit(0);await tick();
 assert.equal(t.ws.state.panes[0].creating,'directory');assert.equal(t.ws.state.panes[0].newName,'existing');assert.equal(t.errors.length,0);assert.equal(t.ws.state.panes[0].createError,'同名项目已存在，请换一个名称。');assert.equal(t.calls.length,0);
});
test('a completed create cannot navigate or select in a newer pane path',async()=>{
 let done;const t=setup(()=>new Promise(r=>done=r));t.click('new-file',0);t.input(0,'新文件.txt');t.submit(0);
 const p=t.ws.state.panes[0];p.path='E:/new';p.requestId++;done({path:'C:/left/新文件.txt'});await tick();
 assert.equal(p.path,'E:/new');assert.equal(p.selected.size,0);assert.equal(t.calls.length,0);
});
test('Escape during an active create does not clear pending creation',async()=>{
 let done;const t=setup(()=>new Promise(r=>done=r));t.click('new-file',0);t.input(0,'新文件.txt');t.submit(0);
 t.handlers.keydown({key:'Escape',preventDefault(){},target:{closest:()=>({dataset:{fmCreate:'0'}})}});
 assert.equal(t.ws.state.panes[0].creating,'file');done({path:'C:/left/新文件.txt'});await tick();
});
