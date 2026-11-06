const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const block=source.slice(source.indexOf('function folderTree('),source.indexOf('// End folder tree model.'));
const context={};vm.createContext(context);vm.runInContext(block,context);
const tree=(entries,collapsed=[])=>JSON.parse(JSON.stringify(context.folderTree(entries,new Set(collapsed))));
test('implicit, explicit and Windows-separated folders are deduplicated',()=>{
 assert.deepEqual(tree([{name:'a/b/c.txt'},{name:'a/',is_dir:true},{name:'a/b/d.txt'},{name:'empty/',is_dir:true},{name:'windows\\child.txt'}]),[
  {path:'',depth:0,hasChildren:true},{path:'a/',depth:1,hasChildren:true},{path:'a/b/',depth:2,hasChildren:false},
  {path:'empty/',depth:1,hasChildren:false},{path:'windows/',depth:1,hasChildren:false}]);
});
test('collapsed branches hide only their descendants and still show expand control',()=>{
 const input=[{name:'a/b/file'},{name:'a/c/file'},{name:'ab/d/file'},{name:'z/file'}];
 assert.deepEqual(tree(input,['a/','ab/d/']).map(n=>n.path),['','a/','ab/','ab/d/','z/']);
 assert.equal(tree(input,['a/'])[1].hasChildren,true);
 assert.deepEqual(tree(input,['']).map(n=>n.path),['']);
 assert.deepEqual(tree(input,['not-present/']).map(n=>n.path),['','a/','a/b/','a/c/','ab/','ab/d/','z/']);
});
test('empty archives and files directly at root have no false child folders',()=>{
 assert.deepEqual(tree([]),[{path:'',depth:0,hasChildren:false}]);
 assert.deepEqual(tree([{name:'readme.txt'}]),[{path:'',depth:0,hasChildren:false}]);
});
test('large directory models retain every path without nested full-folder scans',()=>{
 const input=Array.from({length:20000},(_,i)=>({name:'folder'+String(i).padStart(5,'0')+'/nested/item.txt'}));
 const result=tree(input);assert.equal(result.length,40001);assert.equal(result[0].hasChildren,true);
 assert.equal(result.at(-1).path,'folder19999/nested/');assert.equal(result.at(-1).depth,2);
 assert.equal(tree(input,['folder00000/']).length,40000);
});
