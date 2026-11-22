const test=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs'),vm=require('node:vm');
const source=['input-atime.js','file-workspace.js'].map(name=>fs.readFileSync(require.resolve('../frontend/'+name),'utf8')).join('\n');
function api(storage){const context={localStorage:storage};vm.runInNewContext(source,context);return context.QingFileWorkspace;}
const local={kind:'local',path:'C:/source',selected:new Set(['C:/source/a.txt'])};
const archive={kind:'archive',archive:'C:/target.zip',path:'folder/',selected:new Set(['folder/a.txt'])};
test('preserve applies only to local additions and moves into archives',()=>{
 const {request}=api();
 for(const move of [false,true]){
  assert.equal(request(local,archive,move,'error',true).options.preserve_input_atime,true);
  assert.equal(request(local,archive,move,'error',false).options.preserve_input_atime,undefined);
  assert.equal(request(local,local,move,'error',true).options.preserve_input_atime,undefined);
 }
 assert.equal(request(archive,local,false,'error',true).options.preserve_input_atime,undefined);
 assert.equal(request(archive,archive,false,'error',true).options.preserve_input_atime,undefined);
 assert.equal(request(local,archive,false,'error','true').options.preserve_input_atime,undefined);
});
test('remembered preference is shared, validated and survives storage failure in session',()=>{
 const values=new Map([['unrelated','keep']]);
 const storage={getItem:k=>values.get(k),setItem:(k,v)=>values.set(k,v)};
 let current=api(storage);assert.equal(current.preservePreference(),false);
 current.preservePreference(true);assert.equal(api(storage).preservePreference(),true);
 current.preservePreference(false);assert.equal(api(storage).preservePreference(),false);
 assert.equal(values.get('unrelated'),'keep');
 for(const value of ['broken','"true"','{}','[]']){values.set('qing-add-preserve-atime',value);assert.equal(api(storage).preservePreference(),false);}
 current=api({getItem(){throw Error('denied')},setItem(){throw Error('full')}});
 assert.equal(current.preservePreference(),false);current.preservePreference(true);assert.equal(current.preservePreference(),true);
});
test('advanced preserve switch is shown only for local to archive transfers',()=>{
 const current=api(),workspace=current.create({esc:String,icon:()=>''});
 assert.equal(workspace.page().includes('id="fm-preserve-atime"'),false);
 Object.assign(workspace.state.panes[1],archive);
 assert.equal(workspace.page().includes('id="fm-preserve-atime"'),true);
 workspace.state.active=1;
 assert.equal(workspace.page().includes('id="fm-preserve-atime"'),false);
});
test('preservation reports distinguish unavailable protection from unconfirmed protection',()=>{
 const {preservationNotice}=api();
 assert.equal(preservationNotice({}), '');
 assert.equal(preservationNotice({input_atime:{applied_handles:5}}), '');
 assert.match(preservationNotice({input_atime:{unavailable_handles:1}}), /未能启用/);
 assert.match(preservationNotice({input_atime:{backend_best_effort:true}}), /无法逐项确认/);
 assert.doesNotMatch(preservationNotice({input_atime:{unconfirmed_sources:1}}), /未能保留/);
});
test('archive add warning is delivered after refresh rather than hidden by the list job',async()=>{
 const events=[],handlers={},current=api();current.preservePreference(true);
 const workspace=current.create({esc:String,icon:()=>'',render(){},error:e=>{throw e},
  run:async body=>{events.push(body.operation);return body.operation==='archive-add'?{input_atime:{backend_best_effort:true}}:{entries:[]};},
  updated:async()=>events.push('refreshed'),notice:message=>events.push(message)});
 Object.assign(workspace.state.panes[0],local,{loaded:true});
 Object.assign(workspace.state.panes[1],archive,{loaded:true});
 workspace.mount({querySelector:()=>({addEventListener:(key,fn)=>handlers[key]=fn})});
 handlers.click({preventDefault(){},target:{closest:selector=>selector==='[data-fm]'?{dataset:{fm:'copy',pane:'0'}}:null}});
 await new Promise(setImmediate);
 assert.deepEqual(events.slice(0,3),['archive-add','list','refreshed']);
 assert.match(events[3],/无法逐项确认/);assert.equal(workspace.state.busy,false);
});
