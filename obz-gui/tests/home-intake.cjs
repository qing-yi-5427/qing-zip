const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const logic = source.slice(source.indexOf('let homeIntake='), source.indexOf('const compressionExtensions='));

test('home starts without programmatic focus and restores its control after navigation', () => {
 const body = {};
 const document = {activeElement: body};
 const heading = {matches: () => false, hasAttribute: () => false, focus() { document.activeElement = this; }};
 const control = {
  matches: () => true,
  hasAttribute: name => name === 'data-action',
  getAttribute: () => 'home-smart',
  focus() { document.activeElement = this; },
 };
 const main = {
  contains: element => element === control || element === heading,
  querySelector: selector => selector === 'h1' ? heading : selector === '[data-action="home-smart"]' ? control : null,
  set innerHTML(_) { document.activeElement = body; },
 };
 const noop = () => {};
 const {render, navigate} = vm.runInNewContext(
  source.slice(source.indexOf('const pagePositions='), source.indexOf('function dialog(')) + '({render,navigate})',
  {
   s: {page: 'files', view: 'home'}, main, document, CSS: {escape: String},
   overlay: {hidden: true, dataset: {}}, $: () => ({}),
   archiveTabs: [], currentTabId: null, homeIntake: null, close: noop,
   settings: noop, associationPage: noop, batchPage: noop, profilesPage: noop,
   toolbox: noop, toolPage: noop, historyPage: noop, passwordBookPage: noop,
   home: noop, releaseListingWindow: noop, stopThumbnailObserver: noop,
   renderChrome: noop, updateColumnScroll: noop, renderInlineTask: noop,
  },
 );
 render();
 assert.equal(document.activeElement, body);
 assert.equal(heading.tabIndex, undefined);

 control.focus();
 main.scrollTop = 42;
 navigate('tools');
 assert.equal(document.activeElement, heading);
 assert.equal(heading.tabIndex, -1);
 navigate('files');
 assert.equal(document.activeElement, control);
 assert.equal(main.scrollTop, 42);
});

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
 assert.match(home,/data-action="compress"[\s\S]*?新建压缩包/);
 assert.match(home,/recent\.zip/);
 assert.doesNotMatch(home,/repair-tool|修复/);
 assert.equal(toolbox.match(/class="oa-toolgrid"/g).length,3);
 const cards=[...toolbox.matchAll(/class="oa-toolcard"[^>]+>[\s\S]*?<svg[^>]*>(.*?)<\/svg>[\s\S]*?<\/button>/g)];
 assert.equal(cards.length,13);
 assert.equal(new Set(cards.map(card=>card[1])).size,cards.length);
 assert.match(toolbox,/data-action="repair-tool"[\s\S]*?修复 ZIP 压缩包/);
 assert.match(toolbox,/data-launch-tool="file-split"/);
});

function fixture(responses, chosen = []) {
 const calls = [], fields = {}, timers = new Map();
 const context = {
  s: {page:'files',archive:''},
  overlay: {hidden:true,dataset:{page:''},querySelector:()=>null,querySelectorAll:()=>[]},
  $: id=>fields[id], esc: value=>String(value).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;'),
  filename: path.win32.basename, archiveKey:p=>p.toLowerCase(),
  QingProgress:require('../frontend/progress.js'), QingArchiveListing:{read:body=>body}, tabBusy:()=>false, schedulePendingQueue:()=>{}, archiveOpenMessage:String,
  setTimeout:fn=>{const id=Symbol();timers.set(id,fn);return id;},
  clearTimeout:id=>timers.delete(id),
  btn:(_label,action)=>action,
  opts:(options,selected)=>options.map(([value,label])=>`<option value="${value}" ${selected===value?'selected':''}>${label}</option>`).join(''),
  deepExtract:{acceptDrop:()=>false,batch:paths=>calls.push(['deep',Array.from(paths)])},
  pendingFileGroups:[],showCompressionFiles:paths=>calls.push(['compression',Array.from(paths)]),toast:()=>{},
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
  [unknown]:{route:'review',path:unknown,reason:'未识别为支持的压缩包。',compression_source:'file'},
  [nested]:{route:'deep',path:nested},
 });
 await f.context.routeHomeArchives([unknown,nested]);
 const dialog=f.calls.find(c=>c[0]==='dialog');
 assert.match(dialog[2],/photo\.jpg/);
 assert.match(dialog[2],/value="compress" selected>作为来源压缩/);
 assert.equal(f.context.overlay.dataset.page,'home-review');
 f.fields['oa-home-review-0']={value:'skip'};
 f.context.submitHomeReview();
 assert.deepEqual(f.calls.filter(c=>c[0]==='deep'),[['deep',[nested]]]);
});

test('ordinary files and directories fill one compression form directly in selection order',async()=>{
 const photo='C:/photo.jpg',folder='C:/Sample Folder/';
 const f=fixture({
  [photo]:{route:'review',path:photo,reason:'Plain input reason',compression_source:'file'},
  [folder]:{route:'review',path:folder,reason:'Folder reason',compression_source:'directory'},
 });
 await f.context.routeHomeArchives([photo,folder]);
 assert.equal(f.calls.some(c=>c[0]==='dialog'),false);
 assert.deepEqual(f.calls.filter(c=>c[0]==='compression'),[['compression',[photo,folder]]]);
});

test('an uncertain item keeps the batch in review alongside a valid source',async()=>{
 const photo='C:/photo.jpg',broken='C:/bad.zip';
 const f=fixture({
  [photo]:{route:'review',path:photo,reason:'Plain input reason',compression_source:'file'},
  [broken]:{route:'review',path:broken,reason:'Invalid archive header'},
 });
 await f.context.routeHomeArchives([photo,broken]);
 const [, ,html]=f.calls.find(c=>c[0]==='dialog');
 assert.equal((html.match(/value="compress" selected/g)||[]).length,1);
 assert.match(html,/bad\.zip/);
 assert.equal(f.calls.some(c=>c[0]==='compression'),false);
});

test('a classification finishing while a different dialog is open queues the sources',async()=>{
 const folder='C:/Sample Folder';
 let resolveInspection;
 const f=fixture({[folder]:new Promise(resolve=>{resolveInspection=resolve;})});
 const intake=f.context.routeHomeArchives([folder]);
 f.context.overlay.hidden=false;
 f.context.overlay.dataset.page='other-dialog';
 resolveInspection({route:'review',path:folder,reason:'Folder reason',compression_source:'directory'});
 await intake;
 assert.equal(f.context.overlay.dataset.page,'other-dialog');
 assert.equal(f.calls.some(c=>c[0]==='compression'),false);
 assert.deepEqual(f.context.pendingFileGroups.map(paths=>Array.from(paths)),[[folder]]);
});

test('selected compression sources wait for the recognized deep batch dialog',async()=>{
 const photo='C:/photo.jpg',nested='C:/nested.zip';
 const f=fixture({
  [photo]:{route:'review',path:photo,reason:'未识别为支持的压缩包。',compression_source:'file'},
  [nested]:{route:'deep',path:nested},
 });
 await f.context.routeHomeArchives([photo,nested]);
 f.fields['oa-home-review-0']={value:'compress'};
 f.context.submitHomeReview();
 assert.deepEqual(f.calls.filter(c=>c[0]==='deep'),[['deep',[nested]]]);
 assert.equal(f.calls.some(c=>c[0]==='compression'),false);
 assert.deepEqual(f.context.pendingFileGroups.map(paths=>Array.from(paths)),[[photo]]);
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
 assert.doesNotMatch(html,/value="compress"/);
 assert.equal(html.match(/复制诊断/g).length,2);
});

test('diagnostic text keeps literal angle brackets and ampersands in the treatment row',async()=>{
 const file='C:/sample & set.zip',reason='无效的归档路径 < &';
 const f=fixture({[file]:{route:'review',path:file,reason}});
 await f.context.routeHomeArchives([file]);
 const [, ,html]=f.calls.find(c=>c[0]==='dialog');
 assert.match(html,/id="oa-home-review-detail-0">无效的归档路径 &lt; &amp;<\/span>/);
 assert.doesNotMatch(html,/无效的归档路径 < &/);
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

test('the file picker opens once and can be reused while a stopped inspection still unwinds',async()=>{
 const file='C:/slow.zip';
 let finishPicker,finishInspection;
 const f=fixture({[file]:new Promise(resolve=>{finishInspection=resolve;})},new Promise(resolve=>{finishPicker=resolve;}));
 const first=f.context.pickHomeArchives();
 await f.context.pickHomeArchives();
 assert.equal(f.calls.filter(c=>c[0]==='pick_home_files').length,1);
 finishPicker([file]);
 await new Promise(resolve=>setImmediate(resolve));
 f.context.stopHomeIntake();
 const next=f.context.pickHomeArchives();
 assert.equal(f.calls.filter(c=>c[0]==='pick_home_files').length,2);
 finishInspection({route:'deep',path:file});
 await Promise.all([first,next]);
});
