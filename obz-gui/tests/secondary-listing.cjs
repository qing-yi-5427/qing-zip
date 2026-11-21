const assert=require('node:assert/strict'),fs=require('node:fs'),test=require('node:test'),vm=require('node:vm');
const src=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const listing=src.slice(src.indexOf('async function runSecondaryListing('),src.indexOf('async function openArchive('));
const close=src.slice(src.indexOf('function close(restoreFocus'),src.indexOf('function taskError('));

function scenario(start){
 const calls=[],progress=[];
 const primary={id:17,operation:'extract',status:'running'};
 const context={
  secondaryOpen:null,s:{active:primary.id,backgroundExtraction:true,jobs:[primary]},
  $:()=>({closest:()=>({getAttribute:()=> 'true'})}),
  performance:{now:()=>10},QingArchiveListing:{read:data=>data},
  renderHistoryJob:()=>{},updateJobProgress:(job,v)=>{job.status=v.status;},
  invoke:async(command,args)=>{
   calls.push([command,args]);
   if(command==='start_operation')return start;
   if(command==='wait_operation')return {status:'complete'};
   if(command==='take_archive_listing')return {entries:['already loaded']};
  },
 };
 vm.createContext(context);
 vm.runInContext(listing+close,context);
 return {context,calls,progress,primary};
}

test('listing uses its own operation ID while extraction remains active',async()=>{
 const f=scenario(Promise.resolve(29));
 const result=await f.context.runSecondaryListing({operation:'list',path:'other.zip'},job=>f.progress.push(job.id));
 assert.deepEqual(result.entries,['already loaded']);
 assert.deepEqual(f.progress,[29]);
 assert.deepEqual(f.calls.map(([cmd,args])=>[cmd,args.id]),[
  ['start_operation',undefined],['wait_operation',29],['take_archive_listing',29],['forget_operation',29],
 ]);
 assert.equal(f.context.s.active,17);
 assert.equal(f.primary.status,'running');
 assert.equal(f.context.secondaryOpen,null);
});

test('closing a loading read cancels only that operation, even before it has an ID',async()=>{
 let started;
 const f=scenario(new Promise(resolve=>{started=resolve;}));
 const pending=f.context.runSecondaryListing({operation:'list',path:'other.zip'});
 f.context.close();
 started(29);
 await assert.rejects(pending,/operation cancelled/);
 assert.deepEqual(f.calls.filter(([cmd])=>cmd==='cancel_operation').map(([,args])=>args.id),[29]);
 assert.ok(!f.calls.some(([cmd])=>cmd==='take_archive_listing'));
 assert.equal(f.primary.status,'running');
 assert.equal(f.context.s.active,17);
 assert.equal(f.context.secondaryOpen,null);
});
