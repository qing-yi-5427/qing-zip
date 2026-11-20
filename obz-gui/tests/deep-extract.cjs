const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const app=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8'),moduleSource=fs.readFileSync(path.join(__dirname,'../frontend/deep-extract.js'),'utf8');
function fixture(){
 const elements={},calls=[],handlers={},submit={};
 const context={
  window:{},Map,Number,Set,archiveParent:p=>path.win32.dirname(p),archiveStem:p=>path.win32.parse(p).name,
  root:{addEventListener:(name,fn)=>handlers[name]=fn,querySelector:()=>submit},
  s:{jobs:[],openAfter:true,closeAfter:true,sameFolder:true},
  pref:{},esc:String,btn:()=>'',dialog(title,body,footer){
   calls.push(['dialog',title,body,footer]);
   for(const id of ['inputs','destination','count','list','preview','recycle'])elements['oa-deep-'+id]={value:'',checked:false};
   elements['oa-deep-recycle'].checked=context.s.deepRecycleAfter;
  },$:id=>elements[id],run:async (...args)=>{calls.push(['run',...args]);return {deep_session:1,finished:false};},
  invoke:async(...args)=>calls.push(['invoke',...args]),appearance:()=>calls.push(['appearance']),toast:value=>calls.push(['toast',value])
 };
 vm.createContext(context);vm.runInContext(moduleSource,context);
 return {context,api:context.window.QingDeepExtract(context),elements,calls,handlers,submit};
}
test('actual app initialization reaches deep module without a temporal-dead-zone error',()=>{const element={addEventListener:()=>{}};const context={window:{__TAURI__:{core:{invoke:async()=>{}}}},document:{getElementById:()=>element},localStorage:{getItem:()=>null},dialog:()=>{},run:async()=>{},appearance:()=>{},toast:()=>{},renderProgress:()=>{}};vm.createContext(context);vm.runInContext(moduleSource,context);const start=app.slice(0,app.indexOf('let columns=ArchiveColumns.normalize'))+app.slice(app.indexOf('function archiveParent('),app.indexOf('function updateExtractPreview('))+'})();';assert.doesNotThrow(()=>vm.runInContext(start,context));});
test('batch drop appends once and settings never persist passwords',()=>{const f=fixture();f.api.batch();f.elements['oa-deep-inputs']={value:'C:\\a.zip'};assert.ok(f.api.acceptDrop(['C:\\a.zip','C:\\b.jpg']));assert.equal(f.elements['oa-deep-inputs'].value,'C:\\a.zip\nC:\\b.jpg');assert.ok(f.api.switchMarkup().includes('oa-deep-extract'));assert.ok(!JSON.stringify(f.context.s.deepLimits).includes('password'));});
test('home drop uses native physical coordinates and preloads the batch dialog',()=>{
 const f=fixture();
 const logic=app.slice(app.indexOf('function homeDeepDropAt('),app.indexOf('function refreshSources()'));
 Object.assign(f.context,{overlay:{hidden:true},deepExtract:f.api,enqueueArchives:()=>assert.fail('deep zone routed as normal open'),document:{elementFromPoint:(x,y)=>{
  assert.equal(x,40);assert.equal(y,60);
  return {closest:selector=>selector.includes('#oa-home-deep-drop')?{id:'oa-home-deep-drop'}:null};
 }}});
 f.context.window.devicePixelRatio=1.5;
 vm.runInContext(logic,f.context);
 f.context.acceptDroppedPaths(['C:\\a','D:\\b.7z'],{x:60,y:90});
 const dialog=f.calls.find(c=>c[0]==='dialog');
 assert.equal(f.elements['oa-deep-inputs'].value,['C:\\a','D:\\b.7z'].join('\n'));
 assert.match(f.elements['oa-deep-list'].innerHTML,/data-deep-action="remove-input"/);
 assert.match(dialog[2],/id="oa-deep-smart"[^>]*checked/);
 assert.doesNotMatch(dialog[2],/id="oa-deep-recycle"[^>]*checked/);
});
test('batch forwards optional initial password and dedicated per-source destination',async()=>{
 const f=fixture();
 for(const [field,value] of Object.entries({inputs:'C:\\folder\\a\nD:\\b.7z',destination:'',conflict:'error',password:'test-only'})){
  f.elements['oa-deep-'+field]={value};
 }
 f.elements['oa-deep-recycle']={checked:true};
 f.elements['oa-deep-smart']={checked:true};
 for(const id of ['open-after','close-after','remember'])f.elements['oa-deep-'+id]={checked:true};
 await f.handlers.click({target:{closest:()=>({dataset:{deepAction:'start-batch'}})},preventDefault(){}});
 const run=f.calls.find(c=>c[0]==='run');
 assert.equal(run[1].options.password,'test-only');
 assert.equal(run[1].dest,'');
 assert.equal(run[1].smart,true);
 assert.equal(run[1].deep.dedicated_output,true);
 assert.equal(run[3].recycleAfter,true);
 assert.equal(f.elements['oa-deep-password'].value,'');
 f.api.batch();
 assert.match(f.calls.at(-1)[2],/id="oa-deep-recycle"[^>]*checked/);
});
test('cancelled session remains manageable and recovery is directly reachable',()=>{const f=fixture(),html=f.api.jobActions({deep:{deep_session:4,finished:false,cancelled:true,pending:[],recovery_path:'C:\\Recovery'}});assert.match(html,/data-session="4"/);assert.match(html,/打开恢复目录/);assert.match(html,/继续保存或结束会话/);});
test('retained ordinary companions are shown without a failure or new retry action',()=>{const f=fixture(),html=f.api.jobActions({deep:{deep_session:4,finished:true,partial:false,retained_matching_files:['outer.zip → bundle.002']}});assert.match(html,/保留的同名匹配文件/);assert.match(html,/bundle.002/);assert.doesNotMatch(html,/data-deep-action="manage"/);});
test('completion preferences survive password pause and later resume',async()=>{const f=fixture();await f.api.finish({deep_session:1,finished:false,outputs:['C:\\out']},{openAfter:true,closeAfter:true});assert.equal(f.calls.filter(c=>c[0]==='invoke').length,0);await f.api.finish({deep_session:1,finished:true,partial:false,outputs:['C:\\out']});assert.deepEqual(f.calls.filter(c=>c[0]==='invoke').map(c=>c[1]),['reveal_path','window_action']);});
test('discard ignores invalid edited limits and clears password field',async()=>{const f=fixture();f.elements['oa-deep-session']={value:'1'};f.elements['oa-deep-password']={value:'private'};f.elements['oa-deep-depth']={value:'invalid'};await f.handlers.click({target:{closest:()=>({dataset:{deepAction:'discard'}})},preventDefault(){}});const run=f.calls.find(c=>c[0]==='run');assert.equal(run[1].operation,'deep-discard');assert.equal(f.elements['oa-deep-password'].value,'');assert.equal(run[1].deep,undefined);});
test('archive-tab recycling follows the dialog-local intent and valid scope',()=>{
 const source=app.slice(app.indexOf('function syncRecycleOption()'),app.indexOf('function refreshJobContent('));
 let deep=true,scope=[],policy='error';
 const box={dataset:{wanted:'false'},disabled:false,checked:true,closest:()=>({title:''})},submit={lastChild:{textContent:''}};
 const context={s:{archive:'a.zip',recycleAfter:true,deepRecycleAfter:false},archiveTabs:[],overlay:{querySelector:()=>submit},
  $:id=>id==='oa-recycle-after'?box:id==='oa-deep-extract'?{checked:deep}:id==='qa-conflict'?{value:policy}:null,
  canRecycleArchive:(_path,overwrite,partial)=>overwrite!=='skip'&&!partial,
  extractSelection:()=>scope,updateExtractPreview:()=>{}};
 vm.createContext(context);vm.runInContext(source,context);
 context.syncRecycleOption();assert.equal(box.disabled,false);assert.equal(box.checked,false);
 box.dataset.wanted='true';
 context.syncRecycleOption();assert.equal(box.checked,true);assert.match(submit.lastChild.textContent,/回收/);
 scope=['a.txt'];
 context.syncRecycleOption();assert.equal(box.disabled,true);assert.equal(box.checked,false);
 scope=[];policy='skip';
 context.syncRecycleOption();assert.equal(box.disabled,true);assert.equal(box.checked,false);
 deep=false;policy='error';
 context.syncRecycleOption();assert.equal(box.disabled,false);assert.equal(box.checked,true);
 assert.equal(context.s.deepRecycleAfter,false);
});
