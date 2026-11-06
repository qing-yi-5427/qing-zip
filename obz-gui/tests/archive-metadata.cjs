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
