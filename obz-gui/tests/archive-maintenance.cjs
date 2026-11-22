const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const routing = source.slice(source.indexOf('let toolContext;'), source.indexOf('let destinationRequest'));

function createContext(){
 const s = {
  page:'files', archive:'C:/fixture/图片 & ZIP#1.zip', codepage:936, selected:new Set(),
  password:'archive-password'
 };
 const tab = {
  id:'current', archive:s.archive, password:s.password, folder:'photos/',
  entries:[
   {name:'cover.png'}, {name:'photos/first.png'}, {name:'photos/sub/second.png'},
   {name:'photos/nope.jpg', is_dir:true}, {name:'photos/not-image.txt'}
  ]
 };
 const calls = [];
 const context = vm.createContext({
  s, archiveTabs:[tab], currentTabId:tab.id, location:{href:'http://tauri.localhost/index.html'},
  URL, URLSearchParams, closingArchiveTabs:false, removingArchiveTabs:false, finishingArchiveTabs:false,
  tabBusy:()=>false, repairableZip:path=>/\.zip$/i.test(path), isImage:name=>/\.(png|jpg)$/i.test(name),
  navigate:page=>calls.push(['navigate',page]), toast:message=>calls.push(['toast',message]),
  visibleEntries:()=>tab.entries, entry:(name, options)=>calls.push(['preview',name,options.preview]),
  $:()=>null
 });
 vm.runInContext(routing,context);
 return {context, s, tab, calls, getTool:()=>vm.runInContext('toolContext',context)};
}

test('current-archive checksum and ZIP repair scope the path without serializing the password',async()=>{
 const {context,s,calls,getTool}=createContext();
 for(const [action, op] of [['archive-checksum','checksum'],['archive-repair','repair']]){
  await context.startArchiveTool(action);
  const tool=getTool(), params=new URL(tool.frameUrl).searchParams;
  assert.equal(params.get('operation'),op);
  assert.equal(params.get('path'),s.archive);
  assert.equal(params.get('scope'),'archive');
  assert.equal(params.get('codepage'),'936');
  assert.equal(tool.origin,'files');
  assert.ok(!tool.frameUrl.includes(s.password));
 }
 assert.deepEqual(calls,[['navigate','tool'],['navigate','tool']]);
 s.archive='C:/fixture/other.7z';
 await context.startArchiveTool('archive-repair');
 assert.equal(calls.length,2);
 context.tabBusy=()=>true;
 await context.startArchiveTool('archive-checksum');
 assert.equal(calls.length,2);
});

test('selected images preview directly and the archive picker orders its current folder first',async()=>{
 const {context,s,calls,getTool}=createContext();
 s.selected.add('photos/first.png');
 await context.startArchiveTool('archive-preview');
 assert.deepEqual(calls,[['preview','photos/first.png',true]]);
 s.selected.clear();
 await context.startArchiveTool('archive-preview');
 assert.deepEqual(calls[1],['navigate','tool']);
 let load;
 context.mountArchiveTool({
  addEventListener(name, callback){assert.equal(name,'load');load=callback;},
  contentWindow:{
   setArchiveToolPassword:password=>calls.push(['password',password]),
   setArchiveToolImages:images=>calls.push(['images',Array.from(images)])
  }
 },getTool());
 load();
 assert.deepEqual(calls.slice(2),[
  ['password',s.password],['images',['photos/first.png','cover.png','photos/sub/second.png']]
 ]);
});
