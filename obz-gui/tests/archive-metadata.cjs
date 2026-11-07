// Frontend metadata regressions, independent of native window behavior.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
function fixture(){
 const prefixes=['const opts=','const filename=','const entrySize=','function compareEntries(','function visibleEntries('];
 const code=prefixes.map(prefix=>source.split('\n').find(line=>line.startsWith(prefix))).join('\n');
 const context={window:{},s:{entries:[],folder:'',search:'',sort:'size',reverse:false}};
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
test('nested metadata retains unknown size after folder navigation and filtering',()=>{
 const f=fixture();f.s.entries=[{name:'folder/log.txt',size:0,size_known:false},{name:'elsewhere.txt',size:8}];
 f.s.folder='folder/';f.s.search='LOG';
 const entries=f.api.visibleEntries();assert.equal(entries.length,1);assert.equal(f.api.entrySize(entries[0]),'未知');
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

test('an in-flight thumbnail cannot repaint or cache plaintext under a different policy',async()=>{
 const pump=source.slice(source.indexOf('function pumpThumbnails()'),source.indexOf('function observeThumbnails()'));
 const run=async(token,returned,revisionAfterStart)=>{
  const paints=[],cache=[];
  let finish;
  const context={
   thumbnailQueue:new Map([['a.png',{generation:1,policyRevision:2,args:{}}]]),
   previewGeneration:1,thumbnailPolicyRevision:2,thumbnailPolicyToken:token,
   thumbnailActive:0,thumbnailInflight:new Set(),
   invoke:()=>new Promise(resolve=>{finish=resolve;}),
   paintMatchingThumbnails:(_,record)=>paints.push(record),rememberThumbnail:(_,record)=>cache.push(record)
  };
  vm.runInNewContext(pump+'\npumpThumbnails();',context);
  context.thumbnailPolicyRevision=revisionAfterStart;
  finish({data_url:'data:image/png;base64,AA==',format_policy:returned});
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
