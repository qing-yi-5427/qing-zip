// Pure queue model tests. These do not claim Explorer or desktop GUI clicks.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const names=['canRecycleArchive','finishExtraction','shellQueueStatus','clearShellSecret','clearShellQueue','showShellPassword','retryShellExtraction','enqueueShellExtractions','pumpShellExtractions','pumpArchiveQueue','drainNativeArchives'];
const functions=names.map(name=>source.split('\n').find(line=>line.startsWith(`function ${name}(`)||line.startsWith(`async function ${name}(`))).join('\n');
assert.equal(functions.split('\n').length,names.length);
function fixture(options={}){
 const timers=[],calls=[],errors=[],elements={},native={extractions:[],folders:[],files:[],archives:[]};
 const context={
  setTimeout:callback=>{timers.push(callback);return timers.length;},
  s:{archive:'',password:'',codepage:null,overwrite:'error',recycleAfter:false,openAfter:false,closeAfter:false,jobs:[],recent:[],view:'jobs',...options},
  pendingShellExtractions:[],pendingFolders:[],pendingFileGroups:[],pendingArchivePaths:[],archiveTabs:[],
  shellQueueRunning:false,shellQueuePaused:false,shellQueueInterrupted:false,shellActiveRequest:null,shellPasswordRequest:null,openQueueRunning:false,drainingNativeQueue:false,nativeDrainAgain:false,
  overlay:{hidden:true,dataset:{}},archiveKey:p=>(p||'').replaceAll('/','\\').toLowerCase(),filename:p=>p.split(/[\\/]/).at(-1),
  esc:x=>String(x),btn:(label,action)=>`<button data-action="${action}">${label}</button>`,
  renderProgress:()=>{},render:()=>{},renderArchiveTabs:()=>{},toast:x=>calls.push(['toast',x]),error:e=>errors.push(String(e)),
  localStorage:{setItem:(key,value)=>calls.push(['storage',key,value])},discardArchiveTab:(path)=>calls.push(['discard',path]),
  tabBusy:()=>!!context.s.active,compress:()=>calls.push(['compress']),refreshSources:()=>{},$:id=>elements[id]||null,
  dialog:(title,body,footer)=>{calls.push(['dialog',title,body,footer]);context.overlay.hidden=false;elements['qa-shell-password']={value:''};},
  close:()=>{context.overlay.hidden=true;delete elements['qa-shell-password'];},
  openArchive:async path=>calls.push(['open',path]),openDialog:()=>{},
  invoke:async(command,args)=>{calls.push([command,args]);switch(command){case'take_pending_shell_extractions':return native.extractions.splice(0);case'take_pending_compression_files':return native.files.splice(0);case'take_pending_folders':return native.folders.splice(0);case'take_pending_archives':return native.archives.splice(0);default:return null;}},
  run:async(request,label,settings)=>{calls.push(['run',JSON.parse(JSON.stringify(request)),settings]);context.s.jobs.push({summary:'done'});return {destination:request.dest,recycled:settings.recycleAfter};},
 };
 context.enqueueArchives=paths=>context.pendingArchivePaths.push(...paths);
 const compression=source.slice(source.indexOf('function showCompressionFiles('),source.indexOf('async function pumpArchiveQueue('));
 vm.createContext(context);vm.runInContext(compression+'\n'+functions,context);
 const actions=source.split('\n').find(line=>line.startsWith("case'shell-continue':"));
 assert.ok(actions,'real queue action handlers must be present');
 vm.runInContext(`async function queueAction(action){switch(action){${actions}}}`,context);
 return {context,calls,errors,elements,native,timers,queue:(path,smart=true)=>context.enqueueShellExtractions([{path,dest:'C:\\archives',smart}])};
}
test('Explorer file selections are grouped in one compression dialog, including deferred selections',async()=>{
 const f=fixture();
 f.context.compress=()=>{
  f.context.overlay.hidden=false;
  f.elements['qa-inputs']={value:''};
  f.elements['qa-save']={value:''};
 };
 const first=['C:\\sample\\one.txt','C:\\sample\\子 项.txt'];
 const second=['D:\\single.bin'];
 f.native.files.push({Ok:first},{Err:'selection invalid'},{Ok:second});
 await f.context.drainNativeArchives();
 await f.context.pumpArchiveQueue();
 assert.equal(f.elements['qa-inputs'].value,first.join('\n'));
 assert.equal(f.elements['qa-save'].value,'C:\\sample\\sample.zip');
 await f.context.pumpArchiveQueue();
 assert.equal(f.elements['qa-inputs'].value,first.join('\n'));
 f.context.overlay.hidden=true;
 await f.context.pumpArchiveQueue();
 assert.equal(f.elements['qa-inputs'].value,second[0]);
 assert.equal(f.elements['qa-save'].value,'D:\\single.bin.zip');
 assert.ok(f.calls.some(([op,label])=>op==='toast'&&label==='selection invalid'));
});
test('smart and here retain exact request rules and inherit completion preferences',async()=>{
 const f=fixture({overwrite:'rename',recycleAfter:true,openAfter:true,closeAfter:true});
 f.queue('C:\\archives\\one.zip',true);f.queue('C:\\archives\\two.7z',false);
 await f.context.pumpShellExtractions();
 const runs=f.calls.filter(c=>c[0]==='run');
 assert.deepEqual(runs.map(c=>c[1].smart),[true,false]);
 for(const [,request,settings] of runs){assert.equal(request.options.overwrite,'rename');assert.deepEqual(request.options.selected,[]);assert.equal(settings.recycleAfter,true);}
 assert.equal(f.calls.filter(c=>c[0]==='reveal_path').length,2);
 assert.equal(f.calls.filter(c=>c[0]==='discard').length,2);
 assert.equal(f.calls.filter(c=>c[0]==='window_action').length,1);
 assert.equal(f.calls.at(-1)[0],'window_action');
});
test('the first completed job progress dialog permits the next extraction but never a user confirmation',async()=>{
 for(const modal of ['jobs','close-confirm']){
  const f=fixture();
  const run=f.context.run;
  f.context.run=async(...args)=>{
   const result=await run(...args);
   f.context.overlay.hidden=false;
   f.context.overlay.dataset.page=modal;
   return result;
  };
  f.queue('C:\\archives\\one.zip');f.queue('C:\\archives\\two.zip');
  await f.context.pumpShellExtractions();
  assert.equal(f.calls.filter(c=>c[0]==='run').length,modal==='jobs'?2:1);
  assert.equal(f.context.pendingShellExtractions.length,modal==='jobs'?0:1);
 }
});
test('skip and unverified or split formats preserve source despite saved recycle preference',async()=>{
 for(const [path,overwrite] of [['a.zip','skip'],['a.rar','error'],['a.7z.001','rename']]){
  const f=fixture({recycleAfter:true,overwrite});f.queue('C:\\archives\\'+path);await f.context.pumpShellExtractions();
  assert.equal(f.calls.find(c=>c[0]==='run')[2].recycleAfter,false);
  assert.equal(f.calls.some(c=>c[0]==='discard'),false);
 }
});
test('failure pauses remaining queue and never runs completion actions',async()=>{
 const f=fixture({openAfter:true,closeAfter:true});f.queue('C:\\archives\\first.zip');f.queue('C:\\archives\\next.zip');
 f.context.run=async()=>{throw Error('existing destination file');};
 await f.context.pumpShellExtractions();
 assert.equal(f.context.pendingShellExtractions.length,2);assert.equal(f.context.shellQueuePaused,true);
 assert.equal(f.context.pendingShellExtractions[0].path,'C:\\archives\\first.zip');assert.equal(f.context.pendingShellExtractions[0].state,'failed');
 assert.equal(f.calls.some(c=>['reveal_path','window_action'].includes(c[0])),false);
 assert.match(f.errors[0],/existing destination/);assert.match(f.context.shellQueueStatus(),/shell-continue/);
});
test('cancellation stops the batch and leaves next request pending',async()=>{
 const f=fixture({closeAfter:true});f.queue('C:\\archives\\first.zip');f.queue('C:\\archives\\next.zip');
 f.context.run=async()=>{f.context.shellQueuePaused=true;f.context.shellQueueInterrupted=true;throw Error('operation cancelled');};
 await f.context.pumpShellExtractions();
 assert.equal(f.context.pendingShellExtractions.length,2);assert.equal(f.calls.some(c=>c[0]==='window_action'),false);
});
test('only the matching archive supplies password and codepage',async()=>{
 const f=fixture({archive:'C:\\archives\\current.zip',password:'current-secret',codepage:936});
 f.context.archiveTabs.push({archive:'C:\\archives\\saved.zip',password:'saved-secret',codepage:932});
 for(const path of ['current.zip','saved.zip','unknown.zip'])f.queue('C:\\archives\\'+path);
 await f.context.pumpShellExtractions();
 assert.deepEqual(f.calls.filter(c=>c[0]==='run').map(c=>c[1].options.password),['current-secret','saved-secret',null]);
 assert.deepEqual(f.calls.filter(c=>c[0]==='run').map(c=>c[1].options.codepage),[936,932,null]);
});
test('pending native opens or compression requests prevent automatic window close',async()=>{
 for(const field of ['folders','archives','extractions']){
  const f=fixture({closeAfter:true});f.queue('C:\\archives\\one.zip');
  f.native[field].push(field==='extractions'?{path:'C:\\archives\\two.zip',dest:'C:\\archives',smart:false}:'C:\\later');
  await f.context.pumpShellExtractions();assert.equal(f.calls.some(c=>c[0]==='window_action'),false);
 }
});
test('case-insensitive duplicate suppression retains different extraction modes',()=>{
 const f=fixture();f.queue('C:\\archives\\one.zip');f.queue('c:\\ARCHIVES\\ONE.zip');f.queue('C:\\archives\\one.zip',false);
 assert.equal(f.context.pendingShellExtractions.length,2);
});
test('recycle failure prevents auto-close and pauses subsequent tasks',async()=>{
 const f=fixture({recycleAfter:true,closeAfter:true});f.queue('C:\\archives\\one.zip');f.queue('C:\\archives\\two.zip');
 f.context.run=async()=>{f.context.s.jobs.push({summary:'done'});return {destination:'C:\\archives',recycled:false,recycle_error:'source identity changed'};};
 await f.context.pumpShellExtractions();assert.equal(f.context.shellQueuePaused,true);
 assert.equal(f.calls.some(c=>c[0]==='window_action'),false);assert.match(f.context.s.jobs.at(-1).summary,/source identity changed/);
});
test('an unrelated active task or open dialog blocks queue execution',async()=>{
 const f=fixture();f.queue('C:\\archives\\one.zip');f.context.s.active='123';await f.context.pumpShellExtractions();
 f.context.s.active=null;f.context.overlay.hidden=false;await f.context.pumpShellExtractions();
 assert.equal(f.calls.some(c=>c[0]==='run'),false);assert.equal(f.context.pendingShellExtractions.length,1);
});
test('last failed item remains available for explicit generic retry',async()=>{
 const f=fixture({closeAfter:true,openAfter:true});f.queue('C:\\archives\\one.zip');
 const good=f.context.run;f.context.run=async()=>{throw Error('temporary destination error');};
 await f.context.pumpShellExtractions();const item=f.context.pendingShellExtractions[0];
 assert.equal(item.state,'failed');assert.match(f.context.shellQueueStatus(),/shell-password/);
 assert.equal(f.calls.some(c=>c[0]==='dialog'),false,'no error-message guessing or automatic password dialog');
 f.context.run=good;await f.context.queueAction('shell-continue');
 assert.equal(f.context.pendingShellExtractions.length,0);assert.equal(item.state,'complete');
 assert.equal(f.calls.filter(c=>c[0]==='window_action').length,1);
});
test('explicit password retry is bound to the failed archive and cleared on success',async()=>{
 const f=fixture({archive:'C:\\archives\\unrelated.zip',password:'unrelated-secret',recycleAfter:true});
 f.queue('C:\\archives\\protected.zip');f.queue('C:\\archives\\other.zip');
 const item=f.context.pendingShellExtractions[0],good=f.context.run;let first=true;
 f.context.run=async(...args)=>{if(first){first=false;throw Error('password required');}return good(...args);};
 await f.context.pumpShellExtractions();await f.context.queueAction('shell-password');
 const input=f.elements['qa-shell-password'];input.value='only-this-archive-secret';
 assert.equal(f.calls.filter(c=>c[0]==='dialog').length,1);
 await f.context.queueAction('shell-password-submit');
 const runs=f.calls.filter(c=>c[0]==='run');
 assert.deepEqual(runs.map(c=>c[1].path),['C:\\archives\\protected.zip','C:\\archives\\other.zip']);
 assert.deepEqual(runs.map(c=>c[1].options.password),['only-this-archive-secret',null]);
 assert.equal(input.value,'');assert.equal('password' in item,false);assert.equal(f.context.shellPasswordRequest,null);
 assert.equal(f.calls.filter(c=>['storage','dialog','toast'].includes(c[0])).some(c=>JSON.stringify(c).includes('only-this-archive-secret')),false);
 assert.equal(f.context.s.password,'unrelated-secret');
});
test('explicit retry overrides only the matching tab password',async()=>{
 const f=fixture({archive:'C:\\archives\\one.zip',password:'old-tab-secret'});f.queue('C:\\archives\\one.zip');
 const good=f.context.run;f.context.run=async()=>{throw Error('wrong password');};await f.context.pumpShellExtractions();
 f.context.showShellPassword();f.elements['qa-shell-password'].value='corrected-secret';f.context.run=good;
 await f.context.retryShellExtraction(true);
 assert.equal(f.calls.find(c=>c[0]==='run')[1].options.password,'corrected-secret');
 assert.equal(f.context.s.password,'old-tab-secret','queue secret must not silently enter tab storage');
});
test('wrong password retry stays paused and clear removes secret references',async()=>{
 const f=fixture({closeAfter:true,openAfter:true,recycleAfter:true});f.queue('C:\\archives\\one.zip');
 f.context.run=async()=>{throw Error('wrong password');};await f.context.pumpShellExtractions();
 f.context.showShellPassword();f.elements['qa-shell-password'].value='temporary-password';
 await f.context.retryShellExtraction(true);const item=f.context.pendingShellExtractions[0];
 assert.equal(item.password,'temporary-password');assert.equal(item.state,'failed');assert.equal(f.context.shellQueuePaused,true);
 assert.equal(f.context.shellQueueStatus().includes('temporary-password'),false);
 f.context.showShellPassword();const field=f.elements['qa-shell-password'];field.value='not-submitted';
 await f.context.queueAction('shell-clear');
 assert.equal('password' in item,false);assert.equal(field.value,'');assert.equal(f.context.shellPasswordRequest,null);
 assert.equal(f.context.pendingShellExtractions.length,0);assert.equal(f.context.overlay.hidden,true);
 assert.equal(f.calls.some(c=>['discard','reveal_path','window_action'].includes(c[0])),false);
});
test('cancel action retains cancelled request and suppresses all completion side effects',async()=>{
 const f=fixture({recycleAfter:true,closeAfter:true,openAfter:true});f.queue('C:\\archives\\one.zip');f.queue('C:\\archives\\two.zip');
 let reject;const gate=new Promise((_,no)=>{reject=no;});
 f.context.run=async()=>{f.context.s.active=7;try{await gate;}finally{f.context.s.active=null;}};
 const pumping=f.context.pumpShellExtractions();await Promise.resolve();await f.context.queueAction('canceljob');
 reject(Error('operation cancelled'));await pumping;
 assert.equal(f.context.pendingShellExtractions.length,2);assert.equal(f.context.pendingShellExtractions[0].state,'cancelled');
 assert.equal(f.context.shellQueuePaused,true);assert.equal(f.calls.filter(c=>c[0]==='cancel_operation').length,1);
 assert.equal(f.calls.some(c=>['discard','reveal_path','window_action'].includes(c[0])),false);
});
test('clearing waiting items wipes their passwords and keeps only the current task',async()=>{
 const f=fixture({closeAfter:true});f.queue('C:\\archives\\one.zip');f.queue('C:\\archives\\two.zip');
 const first=f.context.pendingShellExtractions[0],second=f.context.pendingShellExtractions[1];
 first.password='active-secret';second.password='waiting-secret';
 const good=f.context.run;f.context.run=async(...args)=>{f.context.clearShellQueue();assert.equal(f.context.pendingShellExtractions.length,1);assert.equal(f.context.pendingShellExtractions[0],first);return good(...args);};
 await f.context.pumpShellExtractions();
 assert.equal('password' in first,false);assert.equal('password' in second,false);
 assert.equal(f.calls.filter(c=>c[0]==='run').length,1);assert.equal(f.calls.some(c=>c[0]==='window_action'),false);
});
test('native enqueue cannot seed password or overwrite failed state',async()=>{
 const f=fixture();f.context.enqueueShellExtractions([{path:'C:\\archives\\one.zip',dest:'C:\\archives',smart:true,password:'injected-secret',state:'complete'}]);
 const item=f.context.pendingShellExtractions[0];assert.equal('password' in item,false);assert.equal(item.state,'pending');
 f.context.run=async()=>{throw Error('failure');};await f.context.pumpShellExtractions();f.queue('C:\\archives\\one.zip');
 assert.equal(f.context.pendingShellExtractions.length,1);assert.equal(item.state,'failed');assert.equal(f.context.shellQueuePaused,true);
});
test('empty password is rejected and stale dialog cannot retry a different archive',async()=>{
 const f=fixture();f.queue('C:\\archives\\one.zip');f.context.run=async()=>{throw Error('failure');};await f.context.pumpShellExtractions();
 f.context.showShellPassword();await assert.rejects(f.context.retryShellExtraction(true),/请输入密码/);
 assert.equal(f.context.pendingShellExtractions[0].state,'failed');f.context.clearShellQueue();f.queue('C:\\archives\\different.zip');
 await assert.rejects(f.context.retryShellExtraction(true),/已移出队列/);
 assert.equal('password' in f.context.pendingShellExtractions[0],false);
});

test('queue retry wakes only for pending work and disarms after it drains',async()=>{
 const f=fixture();
 f.context.schedulePendingQueue();
 assert.equal(f.timers.length,0);
 f.context.overlay.hidden=false;
 f.native.archives.push('C:\\archives\\queued.zip');
 await f.context.drainNativeArchives();
 f.context.schedulePendingQueue();
 assert.equal(f.timers.length,1);
 await f.timers.shift()();
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(f.timers.length,1);
 f.context.overlay.hidden=true;
 await f.timers.shift()();
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(f.timers.length,0);
 assert.equal(f.calls.filter(([name])=>name==='open').length,1);
});
