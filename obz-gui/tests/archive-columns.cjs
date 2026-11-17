const test=require('node:test'),assert=require('node:assert/strict');
const {normalize,move,packedKnown}=require('../frontend/archive-columns.js');
test('untrusted stored settings normalize without losing mandatory name or admitting unknown keys',()=>{const c=normalize([{id:'crc',visible:true,width:99999},{id:'crc',width:2},{id:'name',visible:false,width:'NaN'},{id:'bad',visible:true}]);assert.equal(c.length,12);assert.equal(c[0].width,800);assert.equal(c.find(x=>x.id==='name').visible,true);assert.ok(!c.find(x=>x.id==='bad'));assert.deepEqual(normalize(c),c);});
test('defaults and ordering roundtrip preserve hidden metadata columns',()=>{const c=normalize(null);assert.deepEqual(c.filter(x=>x.visible).map(x=>x.id),['name','packed_size','size','type','modified']);const m=move(c,'packed_size',-1);assert.equal(m[0].id,'packed_size');assert.deepEqual(normalize(JSON.parse(JSON.stringify(m))),m);assert.deepEqual(move(m,'packed_size',-1),m);});
test('omitted packed sizes are not known zeroes',()=>{assert.equal(packedKnown({size:9,packed_size:0}),false);assert.equal(packedKnown({size:0,size_known:false,packed_size:0}),false);assert.equal(packedKnown({size:0,packed_size:0}),true);assert.equal(packedKnown({size:9,packed_size:3}),true);});
test('existing saved columns retain visibility order and width while new date columns start hidden',()=>{const old=normalize(null).filter(c=>!['created','accessed'].includes(c.id));old.reverse();old.find(c=>c.id==='modified').width=240;const next=normalize(old);assert.deepEqual(next.slice(0,10),old);assert.deepEqual(next.slice(10).map(c=>[c.id,c.visible]),[['created',false],['accessed',false]]);next[10].visible=true;next[10].width=200;assert.deepEqual(normalize(JSON.parse(JSON.stringify(next))),next);});
test('detail date cells localize supplied values and show absent metadata honestly while icons stay compact',()=>{
 const fs=require('node:fs'),vm=require('node:vm'),path=require('node:path');
 const source=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
 const code=source.slice(source.indexOf('function fileRow('),source.indexOf('function releaseListingWindow('));
 const context={s:{flatView:false,selected:new Set(),listStyle:'details'},columns:normalize(null).map(c=>({...c,visible:['name','created','accessed'].includes(c.id)})),esc:v=>String(v??''),icon:()=>'',isImage:()=>false,displayModified:v=>v?'local '+v:''};
 const row=vm.runInNewContext(code+';fileRow',context),entry={name:'file.txt',leaf:'file.txt',created:'2025-01-02 03:04:05Z'};
 assert.match(row(entry,0),/data-column="created" title="local 2025-01-02 03:04:05Z"/);
 assert.match(row(entry,0),/data-column="accessed" title="未提供"/);
 context.s.listStyle='icons';assert.doesNotMatch(row(entry,0),/data-column="(?:created|accessed)"/);
});
