const test=require('node:test'),assert=require('node:assert/strict');
const {create,localDirectoryKey}=require('../frontend/file-workspace.js');
const tick=()=>new Promise(setImmediate);
function setup(run,{streams=[{name:'a<.txt',size:5,allocated_size:4096,export_name:'a_.txt'},{name:'empty',size:0,allocated_size:0,export_name:'empty'}],directory=false}={}){
 const handlers={},requests=[],refreshes=[],errors=[];
 const ws=create({esc:s=>String(s).replaceAll('<','&lt;'),icon:()=>'',render(){},error:e=>errors.push(e),run:async body=>{requests.push(body);return run(body);},invoke:async(name,value)=>{
  if(name==='local_streams')return {path:value.path,revision:'identity:revision',streams};
  if(name==='pick_folder')return 'E:/chosen';refreshes.push(value.path);return {path:value.path,parent:'C:/',entries:[]};
 }});
 ws.state.panes.forEach((p,i)=>Object.assign(p,{loaded:true,path:i?'D:/out':'C:/source',autoRefresh:false}));
 Object.assign(ws.state.panes[0],{entries:[{name:'base',path:'C:/source/base',is_dir:directory,linked:false}],selected:new Set(['C:/source/base'])});
 ws.mount({querySelector:()=>({addEventListener:(name,fn)=>handlers[name]=fn})});
 const click=action=>handlers.click({preventDefault(){},target:{closest:s=>s==='[data-fm]'?{dataset:{fm:action,pane:'0'}}:null}});
 const check=(index,checked=true)=>handlers.change({target:{dataset:{fmStreamCheck:String(index)},checked}});
 const edit=(field,value,index=0)=>handlers.input({target:{value,hasAttribute:key=>key==='data-fm-stream-'+field,closest:()=>null,dataset:field==='name'?{fmStreamName:String(index)}:{}}});
 const submit=()=>handlers.submit({preventDefault(){},target:{closest:s=>s==='[data-fm-stream-export]'?{}:null}});
 return {ws,handlers,requests,refreshes,errors,click,check,edit,submit};
}
test('single stream export has editable name, opposite local destination, receipt and no-overwrite typed request',async()=>{
 const t=setup(async()=>({summary:'已导出 1 项'}));t.ws.state.overwrite='overwrite';t.click('streams');await tick();t.check(0);
 assert.match(t.ws.page(),/a&lt;\.txt/);assert.match(t.ws.page(),/输出文件名/);assert.doesNotMatch(t.ws.page(),/identity:revision/);
 t.edit('name','renamed.txt');t.submit();await tick();assert.deepEqual(t.requests,[{operation:'fs-export-streams',path:'C:/source/base',stream_revision:'identity:revision',stream_exports:[{name:'a<.txt',size:5,output_name:'renamed.txt'}],dest:'D:/out',options:{overwrite:'error'}}]);
 assert.deepEqual(t.refreshes,['D:/out']);assert.match(t.ws.page(),/已导出 1 项/);assert.equal(t.ws.state.busy,false);
});
test('directory ADS and multi-select export show exact output mapping and zero-length streams',async()=>{
 const t=setup(async()=>({summary:'ok'}),{directory:true});t.click('streams');await tick();t.click('streams-select-all');assert.match(t.ws.page(),/class="fm-stream-source">a&lt;\.txt<\/span>.*value="a_\.txt"/);assert.match(t.ws.page(),/class="fm-stream-source">empty<\/span>.*value="empty"/);assert.equal((t.ws.page().match(/class="fm-stream-mapping"/g)||[]).length,2);assert.equal((t.ws.page().match(/data-fm-stream-name=/g)||[]).length,2);
 t.click('streams-pick');await tick();t.submit();await tick();assert.equal(t.requests[0].dest,'E:/chosen');assert.equal(t.requests[0].stream_exports.length,2);assert.deepEqual(t.requests[0].stream_exports[1],{name:'empty',size:0,output_name:'empty'});
 assert.equal(t.requests[0].stream_exports[0].output_name,'a_.txt');
});
test('top export control reaches configuration for large selections without submitting and preserves selected names',async()=>{
 const streams=Array.from({length:129},(_,i)=>({name:'stream'+i,size:0,allocated_size:0,export_name:'stream'+i}));const t=setup(async()=>({summary:'ok'}),{streams});t.click('streams');await tick();assert.match(t.ws.page(),/data-fm="streams-configure"[^>]*disabled/);t.click('streams-select-all');
 assert.ok(t.ws.page().indexOf('data-fm="streams-configure"')<t.ws.page().indexOf('<table'));assert.doesNotMatch(t.ws.page(),/4096/);assert.match(t.ws.page(),/class="fm-stream-destination-controls"/);
 const calls=[];const previous=global.document;global.document={querySelector:selector=>{assert.equal(selector,'[data-fm-stream-export]');return {scrollIntoView:options=>calls.push(['scroll',options.block]),querySelector:selector=>{assert.equal(selector,'[data-fm-stream-dest]');return {focus:options=>calls.push(['focus',options.preventScroll])};}};}};
 try{t.click('streams-configure');}finally{if(previous===undefined)delete global.document;else global.document=previous;}
 assert.deepEqual(calls,[['scroll','start'],['focus',true]]);assert.equal(t.requests.length,0);assert.equal(t.ws.state.streams.selected.size,129);
});
test('directory comparison preserves volume roots and refreshes either trailing-separator spelling',async()=>{
 assert.equal(localDirectoryKey('C:\\'),'c:/');assert.equal(localDirectoryKey('c:////'),'c:/');assert.notEqual(localDirectoryKey('C:'),localDirectoryKey('C:/'));
 for(const [left,right,dest] of [['C:\\','c:/','C:/'],['D:/out/','d:\\out\\','D:/out'],['D:/out','d:\\out','d:\\out\\']]){
  const t=setup(async()=>({summary:'ok'}));t.ws.state.panes[0].path=left;t.ws.state.panes[1].path=right;t.click('streams');await tick();t.check(0);t.edit('dest',dest);t.submit();await tick();assert.deepEqual(t.refreshes,[left,right]);
 }
});
test('default corrected-name collisions require edits and each mapping survives selection changes and errors',async()=>{
 const streams=[{name:'a?.txt',size:1,allocated_size:1,export_name:'a_.txt'},{name:'a*.txt',size:2,allocated_size:2,export_name:'a_.txt'}];
 const t=setup(async()=>{throw Error('existing output');},{streams});t.click('streams');await tick();t.click('streams-select-all');t.submit();assert.equal(t.requests.length,0);assert.match(t.ws.state.streams.exportError,/同一输出文件名/);
 t.edit('name','second.txt',1);t.check(1,false);t.check(1,true);assert.equal(t.ws.state.streams.outputNames.get('a*.txt'),'second.txt');t.submit();await tick();assert.equal(t.requests.length,1);assert.deepEqual(t.requests[0].stream_exports.map(e=>e.output_name),['a_.txt','second.txt']);assert.equal(t.ws.state.streams.outputNames.get('a*.txt'),'second.txt');assert.match(t.ws.state.streams.exportError,/existing output/);
 t.edit('name','A_.TXT',1);t.submit();assert.equal(t.requests.length,1);assert.match(t.ws.state.streams.exportError,/同一输出文件名/);
});
test('failed or cancelled partial export keeps selection and inputs and refreshes both matching panes with auto-refresh off',async()=>{
 let reject;const t=setup(()=>new Promise((_,r)=>reject=r));t.ws.state.panes[1].path='C:\\source';t.click('streams');await tick();t.click('streams-select-all');t.edit('dest','C:/source/');t.submit();t.submit();t.click('streams-close');t.click('streams-refresh');
 assert.equal(t.requests.length,1);assert.equal(t.ws.state.busy,true);assert.ok(t.ws.state.streams);reject(Error('operation cancelled\n已导出 1 / 2 项；first.txt 已保留'));await tick();
 assert.match(t.ws.state.streams.exportError,/已导出 1 \/ 2/);assert.equal(t.ws.state.streams.selected.size,2);assert.equal(t.ws.state.streams.dest,'C:/source/');assert.deepEqual(t.refreshes,['C:/source','C:\\source']);assert.equal(t.ws.state.panes[0].autoRefresh,false);
});
test('stale completion does not refresh a pane that navigated away, invalid output cannot submit',async()=>{
 let done;const t=setup(()=>new Promise(resolve=>done=resolve));t.click('streams');await tick();t.check(0);t.edit('name','../outside');t.submit();assert.equal(t.requests.length,0);assert.match(t.ws.state.streams.exportError,/文件名/);
 t.edit('name','safe.txt');t.submit();t.ws.state.panes[1].path='E:/new';t.ws.state.panes[1].requestId++;done({summary:'ok'});await tick();assert.deepEqual(t.refreshes,[]);
});
test('129 streams still support select-all and export; close invalidates a pending listing',async()=>{
 const streams=Array.from({length:129},(_,i)=>({name:'stream'+i,size:0,allocated_size:0}));const t=setup(async()=>({summary:'ok'}),{streams});t.click('streams');await tick();t.click('streams-select-all');assert.equal(t.ws.state.streams.selected.size,129);t.submit();await tick();assert.equal(t.requests[0].stream_exports.length,129);
 t.click('streams-close');assert.equal(t.ws.state.streams,null);
});
