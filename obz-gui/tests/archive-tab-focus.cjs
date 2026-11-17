const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
function fixture(){
 const frames=[],calls=[],nodes=[];
 const document={activeElement:null,body:{isConnected:true}};
 function node(name,dataset={},tab=false){const n={name,dataset,isConnected:true,closest:selector=>tab&&selector==='[data-tab-container]'?n:null,focus(options){document.activeElement=n;calls.push(['focus',name,options]);},scrollIntoView(){calls.push(['scroll-tab',name]);}};nodes.push(n);return n;}
 const main=node('main');main.scrollTop=110;
 const tree={scrollTop:19},menu={hidden:true,contains:n=>n?.menu===true};
 let rows=[],buttons=new Map();
 const context={document,main,s:{page:'files',view:'archive',archive:'a.zip',cursor:'a-row',anchor:'a-row',search:'a-query',selected:new Set(['a-row'])},
  archiveTabs:[{id:'a',archive:'a.zip',cursor:'a-row',anchor:'a-row',search:'a-query',selected:new Set(['a-row']),scrollTop:110,treeScroll:19},{id:'b',archive:'b.zip',cursor:'b-row',anchor:'b-row',search:'b-query',selected:new Set(['b-row']),scrollTop:220,treeScroll:29}],
  currentTabId:'a',TAB_FIELDS:['archive','cursor','anchor','search','selected'],pref:{},overlay:{hidden:true,firstElementChild:null},menuReturnFocus:null,
  root:{querySelector(selector){if(selector==='.oa-tree-section')return tree;return buttons.get(selector.match(/data-archive-tab="([^"]+)"/)?.[1])||null;}},
  $:()=>menu,rowButtons:()=>rows,requestAnimationFrame:fn=>frames.push(fn),tabBusy:()=>false,toast(){},
  invoke:async name=>{calls.push(['invoke',name]);},resetPreviewState:()=>calls.push(['reset-preview']),
  close(){calls.push(['close']);context.overlay.hidden=true;main.focus({preventScroll:true});},
  esc:x=>x,filename:x=>x,btn:()=>'',dialog(){context.overlay.hidden=false;context.overlay.firstElementChild=node('own-confirmation');document.activeElement=node('confirm-button');}
 };
 function paintTabs(){for(const n of buttons.values())n.isConnected=false;buttons=new Map(context.archiveTabs.map(t=>[t.id,node(t.id+'-tab',{},true)]));calls.push(['paint-tabs']);}
 context.renderArchiveTabs=paintTabs;
 context.render=()=>{for(const n of rows)n.isConnected=false;if(!document.activeElement?.isConnected)document.activeElement=document.body;rows=context.s.archive?[node(context.s.cursor,{entry:context.s.cursor})]:[];paintTabs();calls.push(['render']);};
 vm.createContext(context);
 for(const name of ['saveCurrentTab','applyTab','emptyArchiveState','showNestedClose'])vm.runInContext(source.split('\n').find(line=>line.startsWith('function '+name+'(')),context);
 vm.runInContext(source.slice(source.indexOf('let archiveTabFocusGeneration='),source.indexOf('function discardArchiveTab(')),context);
 context.render();document.activeElement=rows[0];calls.length=0;
 return {context,document,main,tree,menu,calls,node,frames,flush(){while(frames.length)frames.shift()();},row:()=>rows[0],tab:id=>buttons.get(id)};
}
test('switch rebuilds file focus without changing selection, query, anchor or saved scroll',()=>{
 const f=fixture(),c=f.context;c.switchArchiveTab('b');f.flush();
 assert.equal(f.document.activeElement.name,'b-row');assert.deepEqual([...c.s.selected],['b-row']);assert.equal(c.s.anchor,'b-row');assert.equal(c.s.search,'b-query');assert.equal(f.main.scrollTop,220);assert.equal(f.tree.scrollTop,29);
 assert.ok(f.calls.some(x=>x[0]==='focus'&&x[2].preventScroll===true));
});
test('tab buttons and tab menu retain the tab focus surface after replacement',()=>{
 for(const menuOrigin of [false,true]){
  const f=fixture(),c=f.context;f.document.activeElement=menuOrigin?{menu:true,isConnected:true,closest:()=>null}:f.tab('a');c.menuReturnFocus=f.tab('a');
  c.switchArchiveTab('b');f.flush();assert.equal(f.document.activeElement,f.tab('b'));
 }
});
test('rapid A-B-A switches discard stale frames even when the final id matches',()=>{
 const f=fixture(),c=f.context;c.switchArchiveTab('b');c.switchArchiveTab('a');f.flush();
 assert.equal(f.document.activeElement.name,'a-row');assert.equal(f.calls.filter(x=>x[0]==='scroll-tab').length,1);assert.equal(f.main.scrollTop,110);
});
test('scheduled restoration does not take focus from a newer modal, page, tab or input',()=>{
 for(const change of ['modal','page','tab','input']){
  const f=fixture(),c=f.context;c.switchArchiveTab('b');const expected=change==='input'?f.node('new-input'):f.document.activeElement;f.document.activeElement=expected;
  if(change==='modal')c.overlay.hidden=false;if(change==='page')c.s.page='settings';if(change==='tab')c.currentTabId='a';
  f.flush();assert.equal(f.document.activeElement,expected,change);assert.equal(f.calls.some(x=>x[0]==='scroll-tab'),false,change);
 }
});
test('closing current tab restores the adjacent cursor; last tab falls back to main',async()=>{
 const f=fixture(),c=f.context;await c.removeArchiveTabs([c.archiveTabs[0]]);f.flush();assert.equal(c.currentTabId,'b');assert.equal(f.document.activeElement.name,'b-row');assert.equal(f.main.scrollTop,220);
 await c.removeArchiveTabs([c.archiveTabs[0]]);f.flush();assert.equal(c.currentTabId,null);assert.equal(f.document.activeElement,f.main);assert.equal(f.main.tabIndex,-1);
 assert.deepEqual(f.calls.filter(x=>x[0]==='invoke').map(x=>x[1]),['revoke_archive_verification','clear_preview_cache','revoke_archive_verification','clear_preview_cache']);
});
test('closing a background tab leaves search and a later modal connected and focused',async()=>{
 for(const modal of [false,true]){
  const f=fixture(),c=f.context,search=f.node('search');f.document.activeElement=search;
  let resume;c.invoke=name=>{f.calls.push(['invoke',name]);return new Promise(resolve=>{resume=resolve;});};
  const pending=c.removeArchiveTabs([c.archiveTabs[1]]);if(modal){c.overlay.hidden=false;c.overlay.firstElementChild=f.node('later-dialog');f.document.activeElement=c.overlay.firstElementChild;}
  const expected=f.document.activeElement;resume();await pending;f.flush();assert.equal(f.document.activeElement,expected);assert.equal(search.isConnected,true);assert.equal(c.s.search,'a-query');assert.equal(f.calls.some(x=>x[0]==='render'||x[0]==='close'),false);
 }
});
test('background dirty close dismisses its own confirmation but protects a replacement dialog',async()=>{
 for(const replace of [false,true]){
  const f=fixture(),c=f.context;f.document.activeElement=f.tab('a');const origin=c.archiveTabFocusOrigin();c.pendingNestedClose={tabs:[{dirty:true,archive:'b.zip'}],focusOrigin:origin};c.showNestedClose();
  let resume;c.invoke=()=>new Promise(resolve=>{resume=resolve;});const pending=c.removeArchiveTabs([c.archiveTabs[1]],origin);
  if(replace){c.overlay.firstElementChild=f.node('new-dialog');f.document.activeElement=c.overlay.firstElementChild;}
  const expected=f.document.activeElement;resume();await pending;f.flush();assert.equal(c.overlay.hidden,!replace);
  assert.equal(f.calls.filter(x=>x[0]==='close').length,replace?0:1);assert.equal(f.document.activeElement,replace?expected:f.tab('a'));
 }
});
test('async background close after a user switch does not restore the old focus origin',async()=>{
 const f=fixture(),c=f.context;let resume;c.invoke=()=>new Promise(resolve=>{resume=resolve;});const pending=c.removeArchiveTabs([c.archiveTabs[0]]);
 c.currentTabId='b';c.s.cursor='b-row';const search=f.node('new-tab-search');f.document.activeElement=search;resume();await pending;f.flush();assert.equal(f.document.activeElement,search);assert.equal(c.currentTabId,'b');
});
test('background Escape has no close side effects; modal and menu Escape preserve their owners',()=>{
 const c={document:{activeElement:null},overlay:{hidden:true},root:{addEventListener(type,handler){c.handler=handler;}},menu:{hidden:true},menuReturnFocus:{isConnected:true,focus(){c.focused=true;}},close(){c.closed++;},closed:0,focused:false,$:()=>c.menu};vm.createContext(c);
 vm.runInContext(source.split('\n').find(line=>line.startsWith('function handleDialogKeydown(')),c);
 const press=()=>c.handleDialogKeydown({key:'Escape',preventDefault(){c.prevented=true;}});
 press();assert.equal(c.closed,0);c.menu.hidden=false;press();assert.equal(c.closed,0);assert.equal(c.menu.hidden,true);assert.equal(c.focused,true);c.overlay.hidden=false;press();assert.equal(c.closed,1);
});
