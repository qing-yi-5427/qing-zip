const test=require('node:test'),assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.js'),'utf8');
const html=fs.readFileSync(require('node:path').join(__dirname,'../frontend/tools.html'),'utf8');
test('folder picker appends selected folder without losing files or duplicating it',async()=>{
 assert.match(html,/id="checksum-folder"/);let selected='C:/test/folder';const nodes={'checksum-folder':{},'checksum-paths':{value:'C:/test/file'}};
 let invalidations=0;
 const env={$:id=>nodes[id],lines:()=>nodes['checksum-paths'].value.split('\n'),invalidateChecksum:()=>invalidations++,invoke:async name=>{assert.equal(name,'pick_folder');return selected;}};
 vm.createContext(env);vm.runInContext(source.slice(source.indexOf('function addChecksumPaths('),source.indexOf("if(initialOperation==='preview'")),env);
 await nodes['checksum-folder'].onclick();assert.equal(nodes['checksum-paths'].value,'C:/test/file\nC:/test/folder');
 await nodes['checksum-folder'].onclick();assert.equal(nodes['checksum-paths'].value,'C:/test/file\nC:/test/folder');
 selected='';await nodes['checksum-folder'].onclick();assert.equal(nodes['checksum-paths'].value,'C:/test/file\nC:/test/folder');
 assert.equal(invalidations,1);
});
