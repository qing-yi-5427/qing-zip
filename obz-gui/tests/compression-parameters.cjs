const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const ctx={window:{}};vm.createContext(ctx);vm.runInContext(fs.readFileSync(require('node:path').join(__dirname,'../frontend/compression-options.js'),'utf8'),ctx);
const range=(f,m)=>{const r=ctx.window.qingCompression.wordRange(f,m);return r&&Array.from(r)};
test('word range follows the actual encoder and ZIP PPMd limit',()=>{
 assert.deepEqual(range('zip'),[3,258]);assert.deepEqual(range('7z'),[5,273]);assert.deepEqual(range('sfx','LZMA'),[5,273]);
 assert.deepEqual(range('zip','Deflate64'),[3,257]);assert.deepEqual(range('zip','PPMd'),[2,16]);assert.deepEqual(range('7z','PPMd'),[2,32]);
 for(const f of ['tar','zipx','gzip','zstd'])assert.equal(range(f),null);
 for(const m of ['ZSTD','Copy','BZip2','Brotli'])assert.equal(range('7z',m),null);
});
test('single-file requirements and split availability are independent',()=>{
 const {singleFile,volumeSupported}=ctx.window.qingCompression;
 for(const format of ['gzip','bzip2','xz','zstd','brotli','lz4','lz5','lizard'])assert.equal(singleFile(format),true);
 for(const format of ['zstd','brotli','lz4','lz5','lizard','tgz','tbz2','txz','iso','lzh'])assert.equal(volumeSupported(format),false);
 for(const format of ['zip','zipx','7z','tar','gzip'])assert.equal(volumeSupported(format),true);
 assert.equal(singleFile('tgz'),false);
});
test('store level and explicit methods stay consistent in both directions',()=>{
 const reconcile=ctx.window.qingCompression.reconcileStore;
 for(const [format,store] of [['zip','Store'],['zipx','Store'],['7z','Copy'],['sfx','Copy']]){
  assert.deepEqual({...reconcile(format,'Deflate','0','level')},{method:store,level:'0'});
  assert.deepEqual({...reconcile(format,store,'5','method')},{method:store,level:'0'});
  assert.deepEqual({...reconcile(format,'LZMA','0','method')},{method:'LZMA',level:'5'});
  // Leaving "only store" by level drops the hidden store method; other
  // explicit methods and the explicit store choice itself are kept.
  for(const level of ['1','5','9'])assert.deepEqual({...reconcile(format,store,level,'level')},{method:'',level});
  assert.deepEqual({...reconcile(format,'BZip2','9','level')},{method:'BZip2',level:'9'});
  assert.deepEqual({...reconcile(format,'','5','level')},{method:'',level:'5'});
 }
 assert.deepEqual({...reconcile('brotli','','0','level')},{method:'',level:'0'});
});
