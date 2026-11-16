const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.js'),'utf8');
function render(result){const output=[],env={$:()=>({replaceChildren(){output.length=0;}}),text:(value,tag)=>{const node={value,tag};output.push(node);return node;}};vm.createContext(env);vm.runInContext(source.slice(source.indexOf('function render('),source.indexOf("$('cancel').onclick"))+';this.render=render;',env);env.render('checksum',result);return output;}
test('single explicit and legacy checksums retain their source and separate digest lines',()=>{assert.equal(render({path:'中文.txt',hashes:{MD5:'aa',SHA256:'bb'}})[0].value,'中文.txt\nMD5: aa\nSHA256: bb');assert.equal(render({path:'empty',crc32:'0',md5:'a',sha1:'b',sha256:'c'})[0].value,'empty\nCRC32: 0\nMD5: a\nSHA1: b\nSHA256: c');});
test('multi-file output associates each digest with its own path',()=>{assert.equal(render({checksums:[{path:'first',hashes:{SHA512:'1'}},{path:'second',hashes:{SHA512:'2'}}]})[0].value,'first\nSHA512: 1\n\nsecond\nSHA512: 2');});

test('empty directory has an explicit message without a synthetic digest',()=>{const out=render({checksums:[]});assert.equal(out.length,1);assert.equal(out[0].value,'文件夹中没有可校验的文件。');});
