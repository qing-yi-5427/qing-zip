const test=require('node:test'),assert=require('node:assert/strict');
const fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require.resolve('../frontend/file-workspace.js'),'utf8');
function api(storage){const context={module:{exports:{}},localStorage:storage};vm.runInNewContext(source,context);return context.module.exports;}
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
