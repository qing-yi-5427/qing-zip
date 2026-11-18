const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const context={};
vm.createContext(context);
vm.runInContext(source.slice(source.indexOf('const folderViews ='),source.indexOf('function visibleEntries()'))+'\nthis.api={folderChildren,flatChildren};',context);
const project=(entries,folder='',flat=false)=>JSON.parse(JSON.stringify(context.api[flat?'flatChildren':'folderChildren'](entries,folder)));
const rich={size:12,size_known:true,packed_size:9,modified:'2025-01-02 03:04:05Z',created:'2024-01-02 03:04:05Z',accessed:'2026-01-02 03:04:05Z',crc:'12345678',method:'Deflate',attributes:'A',comment:'child comment',encrypted:true};
const neutral=name=>({name,leaf:name.slice(0,-1).split('/').pop(),is_dir:true,size:0,size_known:true});

test('implied folder metadata comes from neither the first nor a later child',()=>{
 const children=[{...rich,name:'a/first.txt'},{...rich,name:'a/second.txt',comment:'second'}];
 for(const entries of [children,[...children].reverse()])assert.deepEqual(project(entries),[neutral('a/')]);
});

test('explicit directory metadata wins before or after its descendants in both views',()=>{
 const directory={...rich,name:'a/',is_dir:true,comment:'directory',encrypted:false};
 const child={...rich,name:'a/child.txt'};
 for(const entries of [[directory,child],[child,directory]])for(const flat of [false,true]) {
  const row=project(entries,'',flat).find(e=>e.name==='a/');
  assert.deepEqual(row,{...directory,leaf:'a',size:0,size_known:true});
 }
});

test('nested implied folders agree with flat view without inheriting descendant directory metadata',()=>{
 const entries=[{...rich,name:'a/b/c.txt'},{...rich,name:'a/b/',is_dir:true,comment:'actual b'}];
 assert.deepEqual(project(entries),[neutral('a/')]);
 assert.deepEqual(project(entries)[0],project(entries,'',true).find(e=>e.name==='a/'));
 assert.deepEqual(project(entries,'a/')[0],project(entries,'a/',true).find(e=>e.name==='a/b/'));
 assert.equal(project(entries,'a/')[0].comment,'actual b');
 assert.deepEqual(project([{...rich,name:'a/b/c.txt'}],'a/'),[neutral('a/b/')]);
});

test('ordinary files retain their exact metadata in normal and flat views',()=>{
 const file={...rich,name:'a/unknown.txt',is_dir:false,size:0,size_known:false};
 for(const flat of [false,true])assert.deepEqual(project([file],'a/',flat),[{...file,leaf:'unknown.txt'}]);
});

test('folder projection does not mutate listing entries or depend on mutable inputs',()=>{
 const entries=[Object.freeze({...rich,name:'a/b.txt'}),Object.freeze({...rich,name:'a/',is_dir:true})];
 Object.freeze(entries);
 const before=JSON.stringify(entries);
 for(const flat of [false,true]) { const rows=project(entries,'',flat);rows[0].comment='changed'; }
 assert.equal(JSON.stringify(entries),before);
});

test('implied folder cells show absent metadata and unmarked encryption without a child lock',()=>{
 const columns=require('../frontend/archive-columns.js').normalize(null).map(c=>({...c,visible:true}));
 const rowContext={s:{flatView:false,selected:new Set(),listStyle:'details'},columns,esc:v=>String(v??''),icon:()=>'',isImage:()=>false,displayModified:v=>v||''};
 const render=vm.runInNewContext(source.slice(source.indexOf('function fileRow('),source.indexOf('function releaseListingWindow('))+';fileRow',rowContext);
 const row=project([{...rich,name:'a/file.txt'}])[0],html=render(row,0);
 for(const id of ['modified','created','accessed','crc','method','attributes','comment'])assert.match(html,new RegExp('data-column="'+id+'" title="未提供"'));
 assert.match(html,/data-column="encrypted" title="未标记"/);
 assert.doesNotMatch(html,/🔒|child comment|Deflate|12345678/);
 assert.equal(Object.hasOwn(row,'encrypted'),false);
});

test('tree navigation reuses one parent index per listing',()=>{
 let reads=0;
 const entries=[{get name(){reads++;return 'a/b/file.txt';},size:1},{name:'c/other.txt',size:2}];
 assert.deepEqual(project(entries).map(e=>e.name),['a/','c/']);
 const indexed=reads;
 assert.equal(project(entries,'a/')[0].name,'a/b/');
 assert.equal(project(entries,'c/')[0].name,'c/other.txt');
 assert.equal(project(entries,'a/b/')[0].name,'a/b/file.txt');
 assert.equal(reads,indexed);
 assert.deepEqual(project([{name:'new/file.txt'}]).map(e=>e.name),['new/']);
});
