const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const tabs=[
 {id:'outer',archive:'C:\\outer.zip'},
 {id:'a',archive:'C:\\private-a\\a.zip',parentArchive:'C:\\outer.zip'},
 {id:'deep',archive:'C:\\private-deep\\deep.zip',parentArchive:'C:\\private-a\\a.zip'},
 {id:'b',archive:'C:\\private-b\\b.zip',parentArchive:'C:\\outer.zip'},
 {id:'independent',archive:'C:\\independent.zip'}
];
function selection(mode,id){
 const context={archiveTabs:tabs};vm.createContext(context);
 for(const name of ['archiveKey','nestedCloseSelection'])vm.runInContext(source.split('\n').find(line=>line.startsWith('function '+name+'(')),context);
 return Array.from(context.nestedCloseSelection(mode,id),tab=>tab.id);
}
test('closing an enclosing tab includes all descendants and preserves unrelated tabs',()=>{
 assert.deepEqual(selection('current','outer'),['outer','a','b','deep']);
 assert.deepEqual(selection('current','a'),['a','deep']);
});
test('close others keeps the selected nested tab and its complete parent chain',()=>{
 assert.deepEqual(selection('others','deep'),['b','independent']);
 assert.deepEqual(selection('others','a'),['deep','b','independent']);
});
test('closing all or exiting includes every nested session for confirmation and cleanup',()=>{
 assert.deepEqual(selection('all','deep'),tabs.map(t=>t.id));
 assert.deepEqual(selection('exit','deep'),tabs.map(t=>t.id));
});
