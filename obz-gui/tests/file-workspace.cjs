const test=require('node:test'),assert=require('node:assert/strict');
const {request,archiveRows,create}=require('../frontend/file-workspace.js');
const local=(path,selected=[])=>({kind:'local',path,selected:new Set(selected)});
const archive=(selected=[])=>({kind:'archive',archive:'C:\\sample.zip',path:'folder/',password:'secret',selected:new Set(selected)});
test('local transfer and archive handoffs retain explicit selections and conflict choices',()=>{
 assert.deepEqual(request(local('C:\\left',['C:\\left\\a']),local('D:\\right'),true,'rename'),{operation:'fs-move',inputs:['C:\\left\\a'],dest:'D:\\right',options:{overwrite:'rename'}});
 const add=request(local('C:\\left',['C:\\left\\a']),archive(),false,'error');assert.equal(add.operation,'add');assert.equal(add.path,'C:\\sample.zip');assert.equal(add.options.password,'secret');
 const extract=request(archive(['folder/a']),local('D:\\right'),false,'skip');assert.equal(extract.smart,false);assert.deepEqual(extract.options.selected,['folder/a']);assert.equal(extract.options.overwrite,'skip');
});
test('empty targets, in-flight navigation and unsupported archive moves cannot start a job',()=>{
 assert.throws(()=>request(local('C:\\left',['x']),local(''),false,'error'));
 assert.throws(()=>request({...local('C:\\left',['x']),pending:true},local('D:\\right'),false,'error'));
 assert.throws(()=>request(archive(['x']),local('D:\\right'),true,'error'));
 assert.throws(()=>request(archive(['x']),archive(),false,'error'));
});
test('archive directory browsing does not include adjacent folder prefixes and preserves raw member selections',()=>{
 const entries=[{name:'folder\\a.txt',size:1,is_dir:false},{name:'folder/deep/b',size:2,is_dir:false},{name:'folder2/no',size:3,is_dir:false}];
 const rows=archiveRows(entries,'folder/');assert.deepEqual(rows.map(v=>v.name),['deep','a.txt']);assert.equal(rows[1].path,'folder\\a.txt');assert.equal(rows[0].path,'folder/deep/');
});
test('a large local folder renders a bounded page while retaining selections outside that page',()=>{
 const ui={esc:String,icon:()=>'',invoke:()=>{},render:()=>{}};const workspace=create(ui);const p=workspace.state.panes[0];
 p.entries=Array.from({length:10000},(_,i)=>({name:'f'+i,path:'C:/'+i,size:i,is_dir:false}));p.loaded=true;p.selected.add('C:/9000');
 assert.equal((workspace.page().match(/class="fm-row /g)||[]).length,200);p.offset=200;assert.ok(workspace.page().includes('data-index="399"'));assert.deepEqual([...p.selected],['C:/9000']);
});
test('an old archive refresh cannot overwrite a newer local navigation',async()=>{
 let finish,click;const updates=[];const ui={esc:String,icon:()=>'',render:()=>{},error:e=>{throw e},updated:(...a)=>updates.push(a),run:()=>new Promise(r=>finish=r),invoke:async()=>({path:'D:/target',parent:'D:/',entries:[{name:'local.txt',path:'D:/target/local.txt',size:1,is_dir:false}]})};
 const workspace=create(ui);for(const p of workspace.state.panes)p.loaded=true;
 Object.assign(workspace.state.panes[0],{kind:'archive',archive:'C:/old.zip',path:''});
 const section={addEventListener:(type,fn)=>{if(type==='click')click=fn;}};
 workspace.mount({querySelector:()=>section});
 const trigger=action=>click({preventDefault(){},target:{closest:()=>({dataset:{fm:action,pane:'0'}})}});
 trigger('refresh');assert.equal(workspace.state.panes[0].pending,true);
 trigger('local');await new Promise(setImmediate);
 finish({entries:[{name:'stale.txt',size:1,is_dir:false}]});await new Promise(setImmediate);
 assert.equal(workspace.state.panes[0].kind,'local');assert.equal(workspace.state.panes[0].path,'D:/target');
 assert.deepEqual(workspace.state.panes[0].entries.map(e=>e.name),['local.txt']);assert.deepEqual(updates,[]);
});
test('a partially failed move refreshes both panes and preserves the operation error',async()=>{
 let click;const browsed=[],errors=[];const original=Error('one item moved; second item conflicts');
 const workspace=create({esc:String,icon:()=>'',render(){},error:e=>errors.push(e),run:async()=>{throw original},invoke:async(_,v)=>{browsed.push(v.path);return {path:v.path,parent:null,entries:[]}}});
 for(const [i,p] of workspace.state.panes.entries())Object.assign(p,{loaded:true,path:i?'D:/target':'C:/source'});
 workspace.state.panes[0].selected.add('C:/source/a.txt');
 workspace.mount({querySelector:()=>({addEventListener:(type,fn)=>{if(type==='click')click=fn;}})});
 click({preventDefault(){},target:{closest:()=>({dataset:{fm:'move',pane:'0'}})}});
 await new Promise(setImmediate);
 assert.deepEqual(browsed.sort(),['C:/source','D:/target']);assert.deepEqual(errors,[original]);assert.equal(workspace.state.busy,false);
});
test('unknown archive sizes stay unknown and folders use selection then explicit entry',async()=>{
 const handlers={},calls=[];const workspace=create({esc:String,icon:()=>'',render(){},error:e=>{throw e},invoke:async(_,v)=>{calls.push(v.path);return {path:v.path,parent:'C:/',entries:[]}}});
 for(const p of workspace.state.panes)p.loaded=true;
 Object.assign(workspace.state.panes[0],{path:'C:/',entries:[{name:'folder',path:'C:/folder',is_dir:true,size:0}]});
 workspace.mount({querySelector:()=>({addEventListener:(kind,fn)=>handlers[kind]=fn})});
 const event=()=>({preventDefault(){},stopPropagation(){},target:{closest:()=>({dataset:{fm:'entry',pane:'0',index:'0'}})}});
 handlers.click(event());assert.deepEqual([...workspace.state.panes[0].selected],['C:/folder']);assert.deepEqual(calls,[]);
 handlers.dblclick(event());await new Promise(setImmediate);assert.deepEqual(calls,['C:/folder']);
 Object.assign(workspace.state.panes[0],{entries:[{name:'next',path:'C:/folder/next',is_dir:true,size:0}]});
 handlers.keydown({...event(),key:'Enter'});await new Promise(setImmediate);assert.equal(calls[1],'C:/folder/next');
 workspace.state.panes[0].entries=archiveRows([{name:'unknown',size:0,size_known:false,is_dir:false}],'');
 assert.equal(workspace.state.panes[0].entries[0].size_known,false);assert.ok(workspace.page().includes('>未知</small>'));
});
