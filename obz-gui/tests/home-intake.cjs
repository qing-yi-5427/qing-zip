const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const logic = source.slice(source.indexOf('let homeIntake='), source.indexOf('const compressionExtensions='));

test('ZIP repair moves from home to a labeled toolbox group while recent files remain on home',()=>{
 const context={
  btn:(_label,action)=>action, esc:String, filename:path.win32.basename,
  s:{recent:['C:/fixture/recent.zip']}
 };
 vm.runInNewContext(
  source.slice(source.indexOf('const toolIcons='),source.indexOf('const btn='))+
  source.slice(source.indexOf('function home()'),source.indexOf('// Refreshing an archive'))+
  source.slice(source.indexOf('const toolCatalog='),source.indexOf('function toolPage()')),
  context
 );
 const home=context.home(),toolbox=context.toolbox();
 assert.match(home,/data-action="home-smart"/);
 assert.match(home,/recent\.zip/);
 assert.doesNotMatch(home,/repair-tool|修复/);
 assert.equal(toolbox.match(/class="oa-toolgrid"/g).length,3);
 const cards=[...toolbox.matchAll(/class="oa-toolcard"[^>]+>[\s\S]*?<svg[^>]*>(.*?)<\/svg>[\s\S]*?<\/button>/g)];
 assert.equal(cards.length,12);
 assert.equal(new Set(cards.map(card=>card[1])).size,12);
 assert.match(toolbox,/data-action="repair-tool"[\s\S]*?修复 ZIP 压缩包/);
 assert.match(toolbox,/data-launch-tool="file-split"/);
});

function fixture(responses, chosen = []) {
 const calls = [], fields = {}, timers = new Map();
 const context = {
  s: {page:'files',archive:''},
  overlay: {hidden:true,dataset:{page:''},querySelector:()=>null,querySelectorAll:()=>[]},
  $: id=>fields[id], esc: String, filename: path.win32.basename, archiveKey:p=>p.toLowerCase(),
  QingProgress:require('../frontend/progress.js'), QingArchiveListing:{read:body=>body}, tabBusy:()=>false, schedulePendingQueue:()=>{}, archiveOpenMessage:String,
  setTimeout:fn=>{const id=Symbol();timers.set(id,fn);return id;},
  clearTimeout:id=>timers.delete(id),
  btn:(_label,action)=>action,
  opts:(_options,selected)=>selected,
  deepExtract:{acceptDrop:()=>false,batch:paths=>calls.push(['deep',Array.from(paths)])},
  dialog(title,html){context.overlay.hidden=false;calls.push(['dialog',title,html]);},
  close(){context.overlay.hidden=true;context.overlay.dataset.page='';},
  error(reason){calls.push(['error',String(reason)]);},
  navigate(page){calls.push(['navigate',page]);},
  enqueueArchives:paths=>calls.push(['queue',Array.from(paths)]),
  openDialog:file=>calls.push(['password',file]),
  async openArchive(file,_password,{listing,show}){calls.push(['browse',file,listing.entries,show]);},
  async invoke(command,args){
   calls.push([command,args?.path]);
   if(command==='pick_home_files')return chosen;
   return responses[args.path];
  },
 };
 vm.runInNewContext(logic,context);
 return {context,calls,timers,fields};
}

test('native drop and picker use the same route across one mixed batch',async()=>{
 const normal='C:/normal.zip',nested='C:/nested.zip',disguised='C:/archive.jpg';
 const results={
  [normal]:{route:'browse',path:normal,entries:[{name:'a.txt'}]},
  [nested]:{route:'deep',path:nested},
  [disguised]:{route:'deep',path:disguised},
 };
 const click=fixture(results,[normal,nested,disguised]);
 await click.context.pickHomeArchives();
 const drop=fixture(results);
 drop.context.acceptDroppedPaths([normal,nested,disguised]);
 await new Promise(resolve=>setImmediate(resolve));
 assert.deepEqual(click.calls.slice(1),drop.calls);
 assert.deepEqual(drop.calls.filter(c=>c[0]==='browse'),[['browse',normal,results[normal].entries,false]]);
 assert.deepEqual(drop.calls.filter(c=>c[0]==='deep'),[['deep',[nested,disguised]]]);
 assert.equal(drop.calls.some(c=>c[0]==='dialog'),false);
});

test('unsupported item stays visible alongside a recognized nested archive',async()=>{
 const unknown='C:/photo.jpg',nested='C:/nested.zip';
 const f=fixture({
  [unknown]:{route:'review',path:unknown,reason:'未识别为支持的压缩包。'},
  [nested]:{route:'deep',path:nested},
 });
 await f.context.routeHomeArchives([unknown,nested]);
 const dialog=f.calls.find(c=>c[0]==='dialog');
 assert.match(dialog[2],/photo\.jpg/);
 assert.equal(f.context.overlay.dataset.page,'home-review');
 f.fields['oa-home-review-0']={value:'skip'};
 f.context.submitHomeReview();
 assert.deepEqual(f.calls.filter(c=>c[0]==='deep'),[['deep',[nested]]]);
});

test('a header-encrypted archive prompts to open and has a manual deep action',async()=>{
 const file='C:/encrypted.7z';
 const f=fixture({[file]:{route:'review',path:file,reason:'目录需要密码'}});
 await f.context.routeHomeArchives([file]);
 const dialog=f.calls.find(c=>c[0]==='dialog');
 assert.match(dialog[2],/尝试打开/);
 f.fields['oa-home-review-0']={value:'browse'};
 f.context.submitHomeReview();
 assert.deepEqual(f.calls.filter(c=>c[0]==='password'),[['password',file]]);
 assert.match(source,/openDialog\(browse\[0\],\{allowDeep:true\}\)/);
});

test('long mixed failures keep diagnostics below each treatment choice',async()=>{
 const corrupt='C:/corrupt.zip',missing='C:/missing.zip';
 const reason='Invalid archive header: '+('long engine detail '.repeat(140));
 const f=fixture({
  [corrupt]:{route:'review',path:corrupt,reason},
  [missing]:{route:'review',path:missing,reason:'File not found'},
 });
 await f.context.routeHomeArchives([corrupt,missing]);
 const [, ,html]=f.calls.find(c=>c[0]==='dialog');
 assert.ok(html.indexOf('oa-home-review-0')<html.indexOf(reason));
 assert.match(html,/无法识别压缩包/);
 assert.match(html,/找不到文件/);
 assert.match(html,/<details><summary>诊断详情/);
 assert.equal(html.match(/复制诊断/g).length,2);
});

test('stopping recognition prevents a late result from applying to a new intake',async()=>{
 const old='C:/slow.zip',fresh='C:/fresh.zip';
 let finishOld;
 const f=fixture({
  [old]:new Promise(resolve=>{finishOld=resolve;}),
  [fresh]:{route:'browse',path:fresh,entries:[]},
 });
 f.fields['oa-home-identify-status']={};
 const first=f.context.routeHomeArchives([old,'C:/unread.zip']);
 f.timers.values().next().value();
 assert.equal(f.context.overlay.dataset.page,'home-inspecting');
 f.context.stopHomeIntake();
 await f.context.routeHomeArchives([fresh]);
 finishOld({route:'deep',path:old});
 await first;
 assert.deepEqual(f.calls.filter(c=>c[0]==='browse').map(c=>c[1]),[fresh]);
 assert.equal(f.calls.some(c=>c[0]==='deep'||c[1]==='C:/unread.zip'),false);
});
