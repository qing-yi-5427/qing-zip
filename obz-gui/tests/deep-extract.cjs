const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const app=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8'),moduleSource=fs.readFileSync(path.join(__dirname,'../frontend/deep-extract.js'),'utf8');
function fixture(){
 const elements={},calls=[],handlers={},submit={};
 const context={
  window:{},Map,Number,Set,archiveParent:p=>path.win32.dirname(p),archiveStem:p=>path.win32.parse(p).name,
  root:{addEventListener:(name,fn)=>handlers[name]=fn,querySelector:()=>submit},
  s:{jobs:[],openAfter:true,closeAfter:true,sameFolder:true},
  pref:{},esc:String,btn:()=>'',extractionSummary:({recycleId,recycle})=>`<input id="${recycleId}" ${recycle?'checked':''}>`,sourceDisposition:recycle=>recycle?'回收':'保留',dialog(title,body,footer){
   calls.push(['dialog',title,body,footer]);
   for(const id of ['inputs','destination','count','list','preview','recycle','smart','conflict','source-status'])elements['oa-deep-'+id]={value:'',checked:false,dataset:{}};
   const label={hidden:true};
   elements['oa-deep-password']={id:'oa-deep-password',value:'',disabled:true,closest:()=>label};
   elements['oa-deep-password-reveal']={hidden:true,dataset:{}};
   elements['oa-deep-recycle'].checked=context.s.deepRecycleAfter;
  },passwordRequiredForSource:()=>false,$:id=>elements[id],run:async (...args)=>{calls.push(['run',...args]);return {deep_session:1,finished:false};},
  invoke:async(...args)=>calls.push(['invoke',...args]),appearance:()=>calls.push(['appearance']),toast:value=>calls.push(['toast',value])
 };
 vm.createContext(context);vm.runInContext(moduleSource,context);
 return {context,api:context.window.QingDeepExtract(context),elements,calls,handlers,submit};
}
test('actual app initialization reaches deep module without a temporal-dead-zone error',()=>{const element={addEventListener:()=>{}};const context={QingAppearance:{preferredMode:()=> 'system'},window:{__TAURI__:{core:{invoke:async()=>{}}}},document:{getElementById:()=>element},localStorage:{getItem:()=>null},dialog:()=>{},run:async()=>{},appearance:()=>{},toast:()=>{},renderProgress:()=>{}};vm.createContext(context);vm.runInContext(moduleSource,context);vm.runInContext(fs.readFileSync(path.join(__dirname,'../frontend/password-book.js'),'utf8'),context);const start=app.slice(0,app.indexOf('let columns=ArchiveColumns.normalize'))+app.slice(app.indexOf('function archiveParent('),app.indexOf('function updateExtractPreview('))+'})();';assert.doesNotThrow(()=>vm.runInContext(start,context));});
test('batch drop appends once and settings never persist passwords',()=>{const f=fixture();f.api.batch();f.elements['oa-deep-inputs']={value:'C:\\a.zip'};assert.ok(f.api.acceptDrop(['C:\\a.zip','C:\\b.jpg']));assert.equal(f.elements['oa-deep-inputs'].value,'C:\\a.zip\nC:\\b.jpg');assert.ok(f.api.switchMarkup().includes('oa-deep-extract'));assert.ok(!JSON.stringify(f.context.s.deepLimits).includes('password'));});
test('batch previews complete paths and live recycle eligibility',()=>{
 const f=fixture();
 f.context.s.deepRecycleAfter=true;
 f.api.batch(['C:\\a.zip','D:\\archives\\b.zip']);
 const fields=f.elements,output=fields['oa-deep-preview'],disposition=fields['oa-deep-source-status'];
 assert.equal(output.value,'C:\\a\nD:\\archives\\b');
 assert.equal(disposition.dataset.recycle,'true');
 fields['oa-deep-destination'].value='E:\\long output directory';
 f.handlers.input({target:{id:'oa-deep-destination',dataset:{}}});
 assert.equal(output.value,'E:\\long output directory\\a\nE:\\long output directory\\b');
 fields['oa-deep-conflict'].value='skip';
 f.handlers.change({target:{id:'oa-deep-conflict'}});
 assert.equal(f.submit.disabled,true);
 assert.equal(disposition.dataset.recycle,'false');
 fields['oa-deep-recycle'].checked=false;
 f.handlers.change({target:{id:'oa-deep-recycle'}});
 assert.equal(f.submit.disabled,false);
 assert.match(disposition.textContent,/保留/);
});
test('batch forwards optional initial password and dedicated per-source destination',async()=>{
 const f=fixture();
 f.api.batch(['C:\\folder\\a','D:\\b.7z'],{password:'test-only'});
 assert.equal(f.elements['oa-deep-password'].disabled,false);
 assert.equal(f.elements['oa-deep-password-reveal'].hidden,true);
 f.elements['oa-deep-conflict'].value='error';
 f.elements['oa-deep-recycle'].checked=true;
 f.elements['oa-deep-smart'].checked=true;
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
 assert.match(f.calls.at(-1)[3],/id="oa-deep-recycle"[^>]*checked/);
});
test('cancelled session remains manageable and recovery is directly reachable',()=>{const f=fixture(),html=f.api.jobActions({deep:{deep_session:4,finished:false,cancelled:true,pending:[],recovery_path:'C:\\Recovery'}});assert.match(html,/data-session="4"/);assert.match(html,/打开恢复目录/);assert.match(html,/继续保存或结束会话/);});
test('retained ordinary companions are shown without a failure or new retry action',()=>{const f=fixture(),html=f.api.jobActions({deep:{deep_session:4,finished:true,partial:false,retained_matching_files:['outer.zip → bundle.002']}});assert.match(html,/保留的同名匹配文件/);assert.match(html,/bundle.002/);assert.doesNotMatch(html,/data-deep-action="manage"/);});
test('completion preferences survive password pause and later resume',async()=>{const f=fixture();await f.api.finish({deep_session:1,finished:false,outputs:['C:\\out']},{openAfter:true,closeAfter:true});assert.equal(f.calls.filter(c=>c[0]==='invoke').length,0);await f.api.finish({deep_session:1,finished:true,partial:false,outputs:['C:\\out']});assert.deepEqual(f.calls.filter(c=>c[0]==='invoke').map(c=>c[1]),['reveal_path','window_action']);});
test('discard ignores invalid edited limits and clears password field',async()=>{const f=fixture();f.elements['oa-deep-session']={value:'1'};f.elements['oa-deep-password']={value:'private'};f.elements['oa-deep-depth']={value:'invalid'};await f.handlers.click({target:{closest:()=>({dataset:{deepAction:'discard'}})},preventDefault(){}});const run=f.calls.find(c=>c[0]==='run');assert.equal(run[1].operation,'deep-discard');assert.equal(f.elements['oa-deep-password'].value,'');assert.equal(run[1].deep,undefined);});
for(const deep of [false,true]){
 test(`archive-tab recycle status follows scope and conflict choices (${deep?'deep':'smart'})`,()=>{
  const section=(start,end)=>app.slice(app.indexOf(start),app.indexOf(end));
  const code=section('function extractSelection(', 'function extractPasswordRequired(')+
   section('function extractionSummary(', 'function updateExtractPreview(')+
   section('function extract({', 'async function submitExtraction(')+
   section('function recycleBlockReason(', 'function replaceSiblings(')+
   app.split('\n').find(line=>line.startsWith("root.addEventListener('change'")&&line.includes("t.id==='oa-add-preserve-atime'"));
  const box={id:'oa-recycle-after',dataset:{}},status={dataset:{}},submit={lastChild:{}},scope={name:'extract-scope',dataset:{}};
  const conflict={id:'qa-conflict',value:'error',dataset:{}};
  const elements={'oa-recycle-after':box,'oa-extract-source-status':status,'oa-deep-extract':{checked:deep},'qa-conflict':conflict,'qa-destination':{value:''}};
  const state={archive:'one.zip',selected:new Set(['only.txt']),overwrite:'error',sameFolder:true,recycleAfter:true,deepRecycleAfter:true};
  let onChange;
  const context={s:state,archiveTabs:[],archiveKey:String,$:id=>elements[id],root:{addEventListener:(_event,fn)=>{onChange=fn;}},
   overlay:{querySelector:selector=>selector.includes('extract-scope')?scope:submit},timePreferenceIds:[],
   dialog(_title,body,footer){
    scope.value=body.match(/name="extract-scope" value="([^"]+)" checked/)[1];
    box.checked=/id="oa-recycle-after"[^>]+checked/.test(footer);
    assert.match(footer,/aria-describedby="oa-extract-source-status"/);
   },deepExtract:{passwordMarkup:()=>'',switchMarkup:()=>'',limitsMarkup:()=>'',sync(){}},
   esc:String,btn:()=>'',filename:String,archiveParent:()=>'',conflicts:()=>'',invoke:()=>Promise.resolve({}),error:assert.fail,
   syncExtractPassword(){},updateExtractPreview(){},setDestination(){}};
  vm.createContext(context);vm.runInContext(code,context);
  context.extract({forceDeep:deep});
  assert.equal(scope.value,'selected');
  assert.equal(box.disabled,true);
  assert.equal(box.checked,false);
  assert.match(status.textContent,/请选择「整个压缩包」/);

  scope.value='all';onChange({target:scope});
  assert.equal(box.disabled,false);
  assert.equal(box.checked,true);
  assert.match(submit.lastChild.textContent,/回收/);
  conflict.value='skip';onChange({target:conflict});
  assert.equal(box.disabled,true);
  assert.equal(box.checked,false);
  assert.match(status.textContent,/跳过已有文件/);
  conflict.value='rename';onChange({target:conflict});
  assert.equal(box.disabled,false);
  assert.equal(box.checked,true);
  assert.match(status.textContent,/全部成功后.*移入回收站/);

  state.nestedId='inner';context.syncRecycleOption();
  assert.equal(box.disabled,true);
  assert.match(status.textContent,/关闭内层标签/);
  delete state.nestedId;
  state.archive='split.7z.001';context.syncRecycleOption();
  assert.equal(box.disabled,true);
  assert.match(status.textContent,/分卷压缩包/);
  state.archive='renamed.dat';context.syncRecycleOption();
  assert.equal(box.disabled,!deep);
  if(!deep)assert.match(status.textContent,/此格式在普通解压时无法自动回收/);
 });
}
