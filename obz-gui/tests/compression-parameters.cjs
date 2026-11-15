const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const ctx={window:{}};vm.createContext(ctx);vm.runInContext(fs.readFileSync(require('node:path').join(__dirname,'../frontend/compression-options.js'),'utf8'),ctx);
const range=(f,m)=>{const r=ctx.window.qingCompression.wordRange(f,m);return r&&Array.from(r)};
test('word range follows the actual encoder and ZIP PPMd limit',()=>{
 assert.deepEqual(range('zip'),[3,258]);assert.deepEqual(range('7z'),[5,273]);assert.deepEqual(range('sfx','LZMA'),[5,273]);
 assert.deepEqual(range('zip','Deflate64'),[3,257]);assert.deepEqual(range('zip','PPMd'),[2,16]);assert.deepEqual(range('7z','PPMd'),[2,32]);
 for(const f of ['tar','zipx','gzip','zstd'])assert.equal(range(f),null);
 for(const m of ['ZSTD','Copy','BZip2','Brotli'])assert.equal(range('7z',m),null);
});
