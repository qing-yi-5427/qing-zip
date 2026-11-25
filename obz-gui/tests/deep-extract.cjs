const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm');
const app=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8'),moduleSource=fs.readFileSync(path.join(__dirname,'../frontend/deep-extract.js'),'utf8');
const appSection=(start,end)=>app.slice(app.indexOf(start),app.indexOf(end));
const layout=appSection('function archiveParent(', 'function updateExtractPreview(');
const recoveryState=appSection('const ordinaryRetries=', 'function updateJobProgress(');
function fixture(){
 const elements={},calls=[],handlers={},submit={focus(){}};
 const element=()=>({value:'',checked:false,hidden:false,dataset:{},focus(){},querySelector:()=>submit});
 const context={
  window:{},Map,Number,Set,filename:p=>path.win32.basename(p),
  root:{addEventListener:(name,fn)=>handlers[name]=fn,querySelector:()=>submit},
  s:{jobs:[],openAfter:true,closeAfter:true,sameFolder:true,overwrite:'error'},
  pref:{},esc:String,btn:()=>'',dialog(title,body,footer){
   calls.push(['dialog',title,body,footer]);
   for(const id of Object.keys(elements))delete elements[id];
   for(const [tag,id] of (body+footer).matchAll(/<[^>]+ id="([^"]+)"[^>]*>/g)){
    elements[id]={...element(),id,checked:/\bchecked\b/.test(tag),disabled:/\bdisabled\b/.test(tag),hidden:/\bhidden\b/.test(tag),value:tag.match(/\bvalue="([^"]*)"/)?.[1]||''};
   }
   for(const [,id,options] of body.matchAll(/<select id="([^"]+)"[^>]*>([\s\S]*?)<\/select>/g)){
    elements[id].value=options.match(/<option value="([^"]+)" selected/)?.[1]||options.match(/<option value="([^"]+)"/)?.[1];
   }
   if(elements['oa-deep-password']){
    const label={hidden:true};
    elements['oa-deep-password'].closest=()=>label;
   }
   if(elements['oa-deep-list']){
    Object.defineProperty(elements['oa-deep-list'],'innerHTML',{set(html){
     for(const id of Object.keys(elements).filter(id=>id.startsWith('oa-deep-path-')))delete elements[id];
     for(const [,id] of html.matchAll(/id="(oa-deep-path-\d+)"/g))elements[id]=element();
    }});
   }
  },
  passwordRequiredForSource:()=>false,$:id=>elements[id],
  run:async (...args)=>{calls.push(['run',...args]);return {deep_session:args[0].deep_session??1,finished:false,session_closed:args[0].operation==='deep-discard'};},
  invoke:async(...args)=>calls.push(['invoke',...args]),appearance:()=>calls.push(['appearance']),toast:value=>calls.push(['toast',value]),
  error:assert.fail,
 };
 vm.createContext(context);vm.runInContext(layout+recoveryState+moduleSource,context);
 context.folderForSource=context.archiveStem;
 context.currentReport=context.currentDeepReport;
 context.canContinue=vm.runInContext('deepCanContinue',context);
 const click=(action,attributes={})=>handlers.click({target:{closest:()=>({dataset:{deepAction:action,...attributes}})},preventDefault(){}});
 return {context,api:context.window.QingDeepExtract(context),elements,calls,handlers,submit,click};
}
test('actual app initialization reaches deep module without a temporal-dead-zone error',()=>{const element={addEventListener:()=>{}};const context={QingAppearance:{preferredMode:()=> 'system'},window:{__TAURI__:{core:{invoke:async()=>{}}}},document:{getElementById:()=>element},localStorage:{getItem:()=>null},dialog:()=>{},run:async()=>{},appearance:()=>{},toast:()=>{},error:()=>{},renderProgress:()=>{}};vm.createContext(context);vm.runInContext(moduleSource,context);vm.runInContext(fs.readFileSync(path.join(__dirname,'../frontend/password-book.js'),'utf8'),context);const start=app.slice(0,app.indexOf('let columns=ArchiveColumns.normalize'))+layout+recoveryState+'})();';assert.doesNotThrow(()=>vm.runInContext(start,context));});
test('batch drop appends once and settings never persist passwords',()=>{const f=fixture();f.api.batch();f.elements['oa-deep-inputs']={value:'C:\\a.zip'};assert.ok(f.api.acceptDrop(['C:\\a.zip','C:\\b.jpg']));assert.equal(f.elements['oa-deep-inputs'].value,'C:\\a.zip\nC:\\b.jpg');assert.ok(f.api.switchMarkup().includes('oa-deep-extract'));assert.ok(!JSON.stringify(f.context.s.deepLimits).includes('password'));});
test('batch previews complete paths and live recycle eligibility',()=>{
 const f=fixture();
 f.context.s.deepRecycleAfter=true;
 f.api.batch(['C:\\a.zip','D:\\archives\\b.zip']);
 const fields=f.elements,output=index=>fields['oa-deep-path-'+index].textContent,disposition=fields['oa-deep-source-status'];
 assert.equal(fields['oa-deep-recycle'].checked,false);
 fields['oa-deep-recycle'].checked=true;
 f.handlers.change({target:fields['oa-deep-recycle']});
 assert.equal(output(0),'C:\\a');
 assert.equal(output(1),'D:\\archives\\b');
 assert.equal(disposition.dataset.recycle,'true');
 fields['oa-deep-destination'].value='E:\\long output directory';
 f.handlers.input({target:fields['oa-deep-destination']});
 assert.equal(output(0),'E:\\long output directory\\a');
 assert.equal(output(1),'E:\\long output directory\\b');
 fields['oa-deep-conflict'].value='skip';
 f.handlers.change({target:{id:'oa-deep-conflict'}});
 assert.equal(f.submit.disabled,true);
 assert.equal(disposition.dataset.recycle,'false');
 fields['oa-deep-recycle'].checked=false;
 f.handlers.change({target:{id:'oa-deep-recycle'}});
 assert.equal(f.submit.disabled,false);
 assert.match(disposition.textContent,/保留/);
 fields['oa-deep-inputs'].value=' C:\\a.zip \nC:\\a.zip\n';
 f.handlers.input({target:fields['oa-deep-inputs']});
 assert.match(output(1),/重复路径/);
 fields['oa-deep-inputs'].value=' ';
 f.handlers.input({target:fields['oa-deep-inputs']});
 assert.equal(f.submit.disabled,true);
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
 await f.click('start-batch');
 const run=f.calls.find(c=>c[0]==='run');
 assert.equal(run[1].options.password,'test-only');
 assert.equal(run[1].dest,'');
 assert.equal(run[1].smart,true);
 assert.equal(run[1].deep.dedicated_output,true);
 assert.equal(run[3].recycleAfter,true);
 assert.equal(f.elements['oa-deep-password'].value,'');
 f.api.batch();
 assert.equal(f.elements['oa-deep-recycle'].checked,false);
});
test('cancelled session remains manageable and recovery is directly reachable',()=>{const f=fixture(),html=f.api.jobActions({deep:{deep_session:4,finished:false,cancelled:true,pending:[],recovery_path:'C:\\Recovery'}});assert.match(html,/data-session="4"/);assert.match(html,/打开恢复目录/);assert.match(html,/继续保存或结束会话/);});
test('retained ordinary companions are shown without a failure or new retry action',()=>{const f=fixture(),html=f.api.jobActions({deep:{deep_session:4,finished:true,partial:false,retained_matching_files:['outer.zip → bundle.002']}});assert.match(html,/保留的同名匹配文件/);assert.match(html,/bundle.002/);assert.doesNotMatch(html,/data-deep-action="manage"/);});
test('completion preferences survive password pause and later resume',async()=>{const f=fixture();await f.api.finish({deep_session:1,finished:false,outputs:['C:\\out']},{openAfter:true,closeAfter:true});assert.equal(f.calls.filter(c=>c[0]==='invoke').length,0);await f.api.finish({deep_session:1,finished:true,partial:false,outputs:['C:\\out']});assert.deepEqual(f.calls.filter(c=>c[0]==='invoke').map(c=>c[1]),['reveal_path','window_action']);});
test('reviewing an end-session action preserves the active session until confirmed',async()=>{
 const f=fixture(),deep={deep_session:1,finished:false,pending:[],recovery_path:'C:\\Recovery'};
 f.context.recordDeepReport({},deep);
 await f.click('manage',{session:'1'});
 f.elements['oa-deep-password']={value:'private'};
 f.elements['oa-deep-depth']={value:'invalid'};
 assert.equal(f.elements['oa-deep-discard-review'].hidden,true);
 await f.click('discard-review');
 await f.click('discard-cancel');
 assert.equal(f.elements['oa-deep-manage-actions'].hidden,false);
 assert.equal(f.calls.some(c=>c[0]==='run'),false);
 await f.click('discard-review');
 await f.click('discard');
 const run=f.calls.find(c=>c[0]==='run');
 assert.equal(run[1].operation,'deep-discard');
 assert.equal(f.elements['oa-deep-password'].value,'');
 assert.equal(run[1].deep,undefined);
});
test('resume retains per-session limits and policy and clears a password when switching projects',async()=>{
 const f=fixture(),deep={deep_session:7,finished:false,recovery_path:'C:\\Recovery',pending:[
  {node:2,name:'first.zip',source:'C:\\outer.zip → first.zip',reason:'最大层数'},
  {node:3,name:'second.zip',source:'C:\\outer.zip → second.zip',reason:'需要密码'},
 ]};
 f.context.recordDeepReport({},deep);
 const limits={...f.context.s.deepLimits,depth:42};
 await f.api.finish(deep,{limits,overwrite:'rename'});
 await f.click('manage',{session:'7'});
 const fields=f.elements,password=fields['oa-deep-password'],selector=fields['oa-deep-node'];
 assert.equal(fields['oa-deep-depth'].value,'42');
 assert.equal(fields['oa-deep-conflict'].value,'rename');
 assert.equal(selector.value,'3');
 assert.equal(password.disabled,false);
 password.value='fixture-only';
 selector.value='2';
 f.handlers.change({target:selector});
 assert.equal(fields['oa-deep-node-reason'].textContent,'最大层数');
 assert.equal(fields['oa-deep-node-source'].textContent,'C:\\outer.zip → first.zip');
 assert.equal(password.disabled,true);
 assert.equal(password.value,'');
 await f.click('resume');
 const request=f.calls.find(c=>c[0]==='run')[1];
 assert.equal(request.deep_node,2);
 assert.equal(request.deep.depth,42);
 assert.equal(request.options.overwrite,'rename');
 assert.equal(request.options.password,null);
});
for(const deep of [false,true]){
 test(`archive-tab recycle status follows scope and conflict choices (${deep?'deep':'smart'})`,()=>{
  const code=appSection('function extractSelection(', 'function extractPasswordRequired(')+layout+
   appSection('function updateExtractConflict(', 'async function submitExtraction(')+
   appSection('function recycleBlockReason(', 'function replaceSiblings(')+
   appSection('function handleExtractionChange(', 'function handleExtractionShortcut(');
  const box={id:'oa-recycle-after',dataset:{}},status={dataset:{}},submit={lastChild:{},setAttribute(){}},scope={name:'extract-scope',dataset:{}};
  const conflict={id:'qa-conflict',value:'error',dataset:{}};
  const toggle={id:'oa-deep-extract',checked:deep};
  const elements={'oa-recycle-after':box,'oa-extract-source-status':status,'oa-deep-extract':toggle,
   'qa-conflict':conflict,'qa-conflict-help':{dataset:{}},'qa-destination':{value:'',focus(){}},'oa-destination-details':{addEventListener(){}}};
  const state={archive:'one.zip',selected:new Set(['only.txt']),overwrite:'error',sameFolder:true,recycleAfter:true,deepRecycleAfter:true};
  const context={s:state,archiveTabs:[],archiveKey:String,$:id=>elements[id],
   overlay:{querySelector:selector=>selector.includes('extract-scope')?scope:submit},
   dialog(_title,body,footer){
    scope.value=body.match(/name="extract-scope" value="([^"]+)" checked/)[1];
    box.checked=/id="oa-recycle-after"[^>]+checked/.test(footer);
    assert.match(footer,/aria-describedby="oa-extract-source-status"/);
   },deepExtract:{passwordMarkup:()=>'',switchMarkup:()=>'',limitsMarkup:()=>'',sync(){}},
   esc:String,btn:()=>'',filename:String,conflicts:()=>'',invoke:()=>Promise.resolve({}),error:assert.fail,
   syncExtractPassword(){},updateExtractPreview(){}};
  vm.createContext(context);vm.runInContext(code,context);
  context.extract({forceDeep:deep});
  const onChange=target=>context.handleExtractionChange({target});
  assert.equal(scope.value,'selected');
  assert.equal(box.disabled,true);
  assert.equal(box.checked,false);
  assert.match(status.textContent,/请选择「整个压缩包」/);

  scope.value='all';onChange(scope);
  assert.equal(box.disabled,false);
  assert.equal(box.checked,false);
  box.checked=true;onChange(box);
  assert.match(submit.lastChild.textContent,/回收/);
  conflict.value='skip';onChange(conflict);
  assert.equal(box.disabled,true);
  assert.equal(box.checked,false);
  assert.match(status.textContent,/跳过已有文件/);
  conflict.value='rename';onChange(conflict);
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

  state.archive='single.zip';
  box.checked=false;onChange(box);
  toggle.checked=!deep;onChange(toggle);
  assert.equal(box.checked,false);
  toggle.checked=deep;onChange(toggle);
  assert.equal(box.checked,false);
  context.extract({recycleOnce:true,forceDeep:deep});
  assert.equal(scope.value,'all');
  assert.equal(box.checked,true);
 });
}
test('single extraction previews a preserved wrapper, selected members, and a direct destination',()=>{
 const context={filename:name=>path.win32.basename(name)};
 vm.createContext(context);vm.runInContext(layout,context);
 const names=[{name:'Folder/',is_dir:true},{name:'Folder/sub/a.txt',is_dir:false},{name:'loose.txt',is_dir:false}];
 const location=(selected,smart,deep)=>context.extractionLocation('C:\\files\\folder.zip',names,selected,smart,deep);
 assert.equal(location(['Folder/sub/a.txt'],true,false).relative,'Folder\\sub\\a.txt');
 assert.equal(location(['Folder/'],true,false).relative,'Folder');
 assert.equal(location(['loose.txt'],true,false).relative,'folder\\loose.txt');
 assert.equal(location(['loose.txt'],false,false).relative,'loose.txt');
 assert.equal(location([],false,false).relative,'');
 assert.equal(location(['Folder/'],true,true).relative,'Folder');
 assert.equal(location(['Folder/'],false,true).relative,'folder');
 assert.equal(context.joinExtractionPath('C:\\','folder'),'C:\\folder');
 assert.equal(context.joinExtractionPath('C:','folder'),'C:folder');
});
test('out-of-order folder responses never replace the current destination listing',async()=>{
 const requests=new Map(),list={},input={value:'C:\\first'},up={};
 const details={open:true,querySelector:()=>up};
 const fields={'oa-destination-details':details,'oa-destination-folders':list,'qa-destination':input};
 const context={clearTimeout,$:id=>fields[id],esc:String,icon:()=>'',invoke:(_,args)=>new Promise(resolve=>requests.set(args.path,resolve))};
 vm.createContext(context);vm.runInContext(layout+appSection('let destinationRequest=0', 'const baseAction='),context);
 const older=context.refreshDestinationFolders();
 input.value='C:\\second';
 const current=context.refreshDestinationFolders();
 requests.get('C:\\second')([{path:'C:\\second\\new',name:'new'}]);
 await current;
 requests.get('C:\\first')([{path:'C:\\first\\stale',name:'stale'}]);
 await older;
 assert.match(list.innerHTML,/new/);
 assert.doesNotMatch(list.innerHTML,/stale/);
 assert.equal(context.extractionParent('C:\\'),'');
 assert.equal(context.extractionParent('\\\\server\\'),'');
 assert.equal(context.extractionParent('\\\\server\\share\\'),'');
 assert.equal(context.extractionParent('\\\\server\\share\\child'),'\\\\server\\share');
});
test('keyboard submit respects IME composition, the password picker, and disabled actions',()=>{
 let submitted=0,prevented=0;
 const button={disabled:false,getClientRects:()=>[{}],click:()=>submitted++};
 const dialog={querySelector:()=>button},context={isCompositionKey:event=>event.isComposing};
 vm.createContext(context);vm.runInContext(appSection('function handleExtractionShortcut(', "root.addEventListener('input',handleExtractionInput);"),context);
 const event={key:'Enter',ctrlKey:true,preventDefault:()=>prevented++,target:{closest:selector=>selector==='.oa-extraction-form'?dialog:null}};
 context.handleExtractionShortcut({...event,isComposing:true});
 context.handleExtractionShortcut({...event,target:{closest:selector=>selector==='.oa-password-choices'?{}:dialog}});
 button.disabled=true;context.handleExtractionShortcut(event);
 assert.equal(submitted,0);
 button.disabled=false;context.handleExtractionShortcut(event);
 assert.equal(submitted,1);
 assert.equal(prevented,1);
});
test('ordinary retry keeps the archive and allows a supported conflict strategy after opting out of recycling',async()=>{
 const job={id:11,status:'failed'};
 const retries=new Map([[11,{job,source:'C:\\source.zip',selected:[],recycle:true,sameFolder:true,deep:false}]]),calls=[];
 const fields={
  'oa-retry-id':{value:'11'},'oa-retry-dest':{value:'D:\\chosen'},
  'oa-retry-conflict':{value:'skip'},'oa-retry-recycle':{checked:true},
 };
 const context={ordinaryRetries:retries,s:{page:'files'},$:id=>fields[id],runExtraction:async (config,from)=>calls.push({config,from})};
 vm.createContext(context);vm.runInContext(appSection('async function submitExtractionRetry(', 'async function quickExtract('),context);
 await assert.rejects(context.submitExtractionRetry(),/关闭成功后回收/);
 assert.equal(calls.length,0);
 fields['oa-retry-recycle'].checked=false;
 await context.submitExtractionRetry();
 assert.equal(calls[0].config.source,'C:\\source.zip');
 assert.equal(calls[0].config.dest,'D:\\chosen');
 assert.equal(calls[0].config.overwrite,'skip');
 assert.equal(calls[0].config.recycle,false);
 assert.equal(calls[0].from,job);
});
