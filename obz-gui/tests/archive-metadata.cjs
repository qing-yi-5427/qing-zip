// Frontend metadata regressions, independent of native window behavior.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
function fixture(){
 const prefixes=['const opts=','const filename=','const entrySize='];
 const code=prefixes.map(prefix=>source.split('\n').find(line=>line.startsWith(prefix))).join('\n')+source.slice(source.indexOf('function entryType('),source.indexOf('function toast('))+source.slice(source.indexOf('const folderViews ='),source.indexOf('function archive()'));
 const context={ArchiveColumns:require('../frontend/archive-columns.js'),window:{},s:{entries:[],folder:'',search:'',sort:'size',reverse:false}};
 vm.createContext(context);vm.runInContext(fs.readFileSync(require('node:path').join(__dirname,'../frontend/compression-options.js'),'utf8'),context);vm.runInContext('const compressionLevels=window.qingCompression.levels;\n'+code+'\nthis.api={opts,entrySize,visibleEntries,compressionLevels};',context);
 return context;
}
test('unknown decoded size is distinct from empty file and legacy known size',()=>{
 const {api}=fixture();
 assert.equal(api.entrySize({size:0,size_known:false}),'未知');
 assert.equal(api.entrySize({size:0,size_known:true}),'0 B');
 assert.equal(api.entrySize({size:1024}),'1.0 KB');
});
test('size sorting keeps unknown lengths last in either direction and directories first',()=>{
 const f=fixture();
 f.s.entries=[{name:'unknown.txt',size:0,size_known:false},{name:'empty.txt',size:0,size_known:true},{name:'data.txt',size:4096},{name:'nested/file.txt',size:8}];
 assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.leaf),['nested','empty.txt','data.txt','unknown.txt']);
 f.s.reverse=true;
 assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.leaf),['nested','data.txt','empty.txt','unknown.txt']);
});

test('all date columns sort their own timestamp, keep unknown dates last, and preserve directory priority',()=>{
 for(const key of ['modified','created','accessed']) {
  const f=fixture();f.s.sort=key;
  f.s.entries=[{name:'unknown.txt'},{name:'invalid.txt',[key]:'invalid'},{name:'late.txt',[key]:'2026-01-01 00:00:00Z'},{name:'early.txt',[key]:'2025-01-01 00:00:00Z'},{name:'folder/',is_dir:true}];
  assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.leaf),['folder','early.txt','late.txt','invalid.txt','unknown.txt']);
  f.s.reverse=true;
  assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.leaf),['folder','late.txt','early.txt','unknown.txt','invalid.txt']);
 }
 const f=fixture();f.s.entries=[{name:'a',created:'2025-01-01 00:00:00Z',accessed:'2026-01-01 00:00:00Z'},{name:'b',created:'2026-01-01 00:00:00Z',accessed:'2025-01-01 00:00:00Z'}];
 f.s.sort='created';assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.leaf),['a','b']);
 f.s.sort='accessed';assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.leaf),['b','a']);
});
test('nested metadata retains unknown size after folder navigation and filtering',()=>{
 const f=fixture();f.s.entries=[{name:'folder/log.txt',size:0,size_known:false},{name:'elsewhere.txt',size:8}];
 f.s.folder='folder/';f.s.search='LOG';
 const entries=f.api.visibleEntries();assert.equal(entries.length,1);assert.equal(f.api.entrySize(entries[0]),'未知');
});
test('an empty scoped search can expand, locate the matching member and restore directory selection',()=>{
 const context=fixture(),{s,api}=context;
 const target='乙目录/版本/报告.txt';
 const elements=Object.fromEntries(['qa-search','oa-search-results','oa-search-count','oa-search-clear','oa-search-whole','oa-search-locate','oa-search-scope'].map(id=>[id,{}]));
 elements['qa-search'].focus=()=>{elements['qa-search'].focused=true;};
 context.$=id=>elements[id];
 context.main={scrollTop:450,scrollLeft:80};
 context.positionsFor=list=>new Map(list.map((entry,index)=>[entry.name,index]));
 context.rows=()=>context.syncArchiveSearch(api.visibleEntries());
 context.render=()=>context.rows();context.updateColumnScroll=()=>{};
 context.focusEntry=name=>{context.focused=name;return true;};
 Object.assign(s,{folder:'甲目录/',searchScope:'folder',selected:new Set(['甲目录/草稿.txt']),anchor:'甲目录/草稿.txt',collapsed:new Set(['乙目录/'])});
 s.entries=[{name:'甲目录/草稿.txt',size:1},{name:'乙目录\\版本\\报告.txt',size:2},{name:'乙目录/版本/图片.png',size:3}];
 elements['qa-search'].value='报告';
 context.updateArchiveSearch(elements['qa-search']);
 assert.equal(elements['oa-search-whole'].hidden,false);
 assert.match(elements['oa-search-count'].textContent,/找到 0 项/);
 assert.equal(s.selected.size,0);
 assert.equal(context.main.scrollTop,0);
 assert.equal(context.main.scrollLeft,0);
 context.searchWholeArchive();
 assert.equal(elements['oa-search-scope'].value,'archive');
 assert.equal(elements['oa-search-whole'].hidden,true);
 assert.deepEqual(Array.from(api.visibleEntries(),entry=>entry.name),[target]);
 context.locateSearchResult(target);
 assert.equal(s.folder,'乙目录/版本/');
 assert.equal(s.search,'');
 assert.deepEqual([...s.selected],[target]);
 assert.equal(s.anchor,target);
 assert.equal(context.focused,target);
 assert.equal(s.collapsed.has('乙目录/'),false);
 assert.equal(elements['oa-search-clear'].hidden,true);
});
test('numeric compression preference selects standard instead of silently choosing store',()=>{
 const {api}=fixture();const options=api.opts([[0,'Store'],[5,'Standard'],[9,'Maximum']],'5');
 assert.match(options,/<option value="5" selected>Standard/);
 assert.doesNotMatch(options,/<option value="0" selected>/);
});
test('extended methods present only real application levels and distinguish fast from store',()=>{
 const {api}=fixture();
 assert.equal(api.compressionLevels('zstd').at(-1)[0],9);
 assert.equal(api.compressionLevels('7z','ZSTD').at(-1)[0],9);
 assert.equal(api.compressionLevels('lz5').at(-1)[0],15);
 assert.equal(api.compressionLevels('7z','LZ4')[0][0],1);
 assert.equal(api.compressionLevels('7z','Lizard')[0][0],10);
 assert.equal(api.compressionLevels('7z','Lizard').at(-1)[0],49);
 assert.match(api.compressionLevels('zip')[0][1],/仅存储/);
 assert.doesNotMatch(api.compressionLevels('brotli')[0][1],/仅存储/);
});

// The engine keeps exact UTC ticks; only the visible timestamp is localized.
test('UTC listing display follows local timezone without changing old local strings',()=>{
 const code=source.split('\n').find(line=>line.startsWith('function displayModified('));
 const {spawnSync}=require('node:child_process');
 for(const [zone,expected] of [['Asia/Hong_Kong','2026-11-08 02:00:00'],['America/New_York','2026-11-07 13:00:00'],['UTC','2026-11-07 18:00:00']]){
  const script=code+`\nprocess.stdout.write(JSON.stringify([displayModified('2026-11-07 18:00:00.3370638Z'),displayModified('2020-01-02 03:04:05.125'),displayModified('invalid'),displayModified(null)]));`;
  const result=spawnSync(process.execPath,['-e',script],{env:{...process.env,TZ:zone},encoding:'utf8'});
  assert.equal(result.status,0,result.stderr);assert.deepEqual(JSON.parse(result.stdout),[expected,'2020-01-02 03:04:05.125','invalid','']);
 }
});

const thumbnailPump=source.slice(source.indexOf('function pumpThumbnails()'),source.indexOf('function watchTile('));

test('an in-flight thumbnail cannot repaint or cache plaintext under a different policy',async()=>{
 const run=async(token,returned,revisionAfterStart)=>{
  const paints=[],cache=[];
  let finish;
  const context={
   thumbnailQueue:new Map([['a.png',{batch:true,generation:1,policyRevision:2,args:{}}]]),
   previewGeneration:1,thumbnailPolicyRevision:2,thumbnailPolicyToken:token,
   thumbnailActive:0,thumbnailInflight:new Set(),
   invoke:()=>new Promise(resolve=>{finish=resolve;}),
   paintMatchingThumbnails:(_,record)=>paints.push(record),rememberThumbnail:(_,record)=>cache.push(record)
  };
  vm.runInNewContext(thumbnailPump+'\npumpThumbnails();',context);
  context.thumbnailPolicyRevision=revisionAfterStart;
  finish([{name:'a.png',data_url:'data:image/png;base64,AA==',format_policy:returned}]);
  await new Promise(resolve=>setImmediate(resolve));
  return {paints,cache};
 };
 const mismatch=await run('new','old',2);
 assert.equal(mismatch.cache.length,0);
 assert.equal(mismatch.paints.length,1);
 assert.equal(mismatch.paints[0].data_url,undefined);
 assert.equal((await run('old','old',3)).paints.length,0);
 const current=await run('new','new',2);
 assert.equal(current.cache.length,1);
 assert.equal(current.paints[0].data_url,'data:image/png;base64,AA==');
});

test('visible thumbnails use bounded batches and one corrupt image does not discard valid neighbors', async()=>{
 const names=Array.from({length:20},(_,i)=>i+'.png'), requests=[], paints=new Map(), cache=new Map();
 const context={
  thumbnailQueue:new Map(names.map(name=>[name,{name,batch:true,generation:2,policyRevision:3,args:{path:'archive.zip'}}])),
  previewGeneration:2,thumbnailPolicyRevision:3,thumbnailPolicyToken:'token',thumbnailActive:0,thumbnailInflight:new Set(),
  invoke:(cmd,args)=>{assert.equal(cmd,'preview_thumbnails');assert.equal(args.path,'archive.zip');
    requests.push([...args.names]);return Promise.resolve(args.names.map(name=>name==='1.png'?{name,error:'corrupt'}:
      {name,data_url:'data:image/png;base64,AA==',format_policy:'token'}));},
  paintMatchingThumbnails:(name,value)=>paints.set(name,value), rememberThumbnail:(name,value)=>cache.set(name,value)
 };
 vm.runInNewContext(thumbnailPump+'\npumpThumbnails();',context);
 assert.deepEqual(requests.map(batch=>batch.length),[8]);
 await new Promise(resolve=>setImmediate(resolve));
 assert.deepEqual(requests.flat(),names);assert.equal(requests[2].length,4);
 assert.equal(cache.size,19);assert.equal(cache.has('1.png'),false);assert.match(paints.get('1.png').error,/corrupt/);
 assert.equal(context.thumbnailActive,0);assert.equal(context.thumbnailInflight.size,0);
});

test('small or password-protected archives retain the individual thumbnail calls', async()=>{
 const names=['a.png','b.png','c.png'], sent=[], painted=[];
 const context={
  thumbnailQueue:new Map(names.map(name=>[name,{name,batch:false,generation:1,policyRevision:1,args:{path:'a.zip',password:'pw'}}])),
  previewGeneration:1,thumbnailPolicyRevision:1,thumbnailPolicyToken:'default',thumbnailActive:0,thumbnailInflight:new Set(),
  invoke:(cmd,args)=>{assert.equal(cmd,'preview_entry');assert.equal(args.password,'pw');
    sent.push(args.name);return Promise.resolve({name:args.name,data_url:'data:image/png;base64,QQ==',format_policy:'default'});},
  paintMatchingThumbnails:name=>painted.push(name),rememberThumbnail:()=>{}
 };
 vm.runInNewContext(thumbnailPump+'\npumpThumbnails();',context);
 assert.deepEqual(sent,['a.png','b.png']);
 await new Promise(resolve=>setImmediate(resolve));
 assert.deepEqual(painted,names);assert.equal(context.thumbnailActive,0);
});

test('switching folder, query, sort and refreshed listings invalidates displayed rows',()=>{
 const f=fixture(),s=f.s,get=f.api.visibleEntries;
 s.entries=[{name:'dir/alpha.txt',size:5},{name:'dir/bravo.txt',size:1},{name:'other/beta.txt',size:4}];
 s.folder='dir/';
 assert.deepEqual(Array.from(get(),e=>e.leaf),['bravo.txt','alpha.txt']);
 s.search='ALP';assert.deepEqual(Array.from(get(),e=>e.leaf),['alpha.txt']);
 s.search='';s.sort='name';assert.deepEqual(Array.from(get(),e=>e.leaf),['alpha.txt','bravo.txt']);
 s.folder='other/';assert.equal(get()[0].leaf,'beta.txt');
 s.entries=[{name:'other/new.txt',size:9}];assert.equal(get()[0].leaf,'new.txt');
});

test('type and modification date sorts keep directories first and missing times last',()=>{
 const f=fixture(),s=f.s;
 s.entries=[{name:'folder/item',modified:'2030-01-01 00:00:00Z'},{name:'folder/',is_dir:true,modified:'2023-10-01 00:00:00Z'},
  {name:'a.txt',modified:'2024-01-01 00:00:00Z'},{name:'b.ZIP',modified:'2023-01-01 00:00:00Z'},{name:'c.txt'}];
 s.sort='modified';
 const rows=()=>Array.from(f.api.visibleEntries(),e=>e.name);
 assert.deepEqual(rows(),['folder/','b.ZIP','a.txt','c.txt']);
 assert.equal(f.api.visibleEntries()[0].modified,'2023-10-01 00:00:00Z');
 s.reverse=true;assert.deepEqual(rows(),['folder/','a.txt','b.ZIP','c.txt']);
 s.sort='type';s.reverse=false;assert.deepEqual(rows(),['folder/','a.txt','c.txt','b.ZIP']);
});

test('flat view includes implied and empty directories, scopes search, and preserves full member identity',()=>{
 const {api,s}=fixture();
 s.entries=[
  {name:'outside.txt',size:42},{name:'top/b/same.txt',size:2},{name:'top/a/same.txt',size:1},
  {name:'top/b/empty/',is_dir:true,modified:'2026-01-01'},{name:'top/a/',is_dir:true,modified:'2026-04-01'},
 ];
 Object.assign(s,{folder:'top/',flatView:true,sort:'name'});
 const names=()=>Array.from(api.visibleEntries(),e=>e.name);
 assert.deepEqual(names(),['top/a/','top/b/','top/b/empty/','top/a/same.txt','top/b/same.txt']);
 assert.equal(api.visibleEntries()[0].modified,'2026-04-01');
 s.reverse=true;
 assert.deepEqual(names(),['top/b/empty/','top/b/','top/a/','top/b/same.txt','top/a/same.txt']);
 s.search='b/same';assert.deepEqual(names(),['top/b/same.txt']);
 s.folder='top/a/';s.search='';assert.deepEqual(names(),['top/a/same.txt']);
 s.folder='top/';s.flatView=false;s.reverse=false;
 assert.deepEqual(names(),['top/a/','top/b/']);
});

test('flat filenames sort embedded numbers naturally while ordinary browsing keeps its ordering',()=>{
 const {api,s}=fixture();
 s.entries=['item10','item2','item1'].map(name=>({name,size:1}));
 Object.assign(s,{sort:'name',flatView:true});
 assert.deepEqual(Array.from(api.visibleEntries(),e=>e.name),['item1','item2','item10']);
 s.flatView=false;
 assert.deepEqual(Array.from(api.visibleEntries(),e=>e.name),['item1','item10','item2']);
});

test('optional metadata sorts and omitted packed sizes remain meaningful in both directions',()=>{
 const f=fixture();f.s.entries=[{name:'unknown',size:9,packed_size:0,crc:''},{name:'empty',size:0,packed_size:0,crc:'11111111'},{name:'data',size:9,packed_size:4,crc:'FFFFFFFF'}];
 f.s.sort='packed_size';assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.name),['empty','data','unknown']);f.s.reverse=true;assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.name),['data','empty','unknown']);
 f.s.sort='crc';assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.name),['data','empty','unknown']);f.s.reverse=false;assert.deepEqual(Array.from(f.api.visibleEntries(),e=>e.name),['empty','data','unknown']);
});
