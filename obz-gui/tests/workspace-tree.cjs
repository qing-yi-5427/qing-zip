const test=require('node:test'),assert=require('node:assert/strict');
const {create,localTreeRows}=require('../frontend/file-workspace.js');
const entry=(path,name=path)=>({path,name,is_dir:true,linked:false,size:0});
function harness(invoke=async()=>{throw Error('unexpected IO')}){
 const handlers={},focus=[],errors=[];let ws,nodes=[],html='';
 const section={addEventListener:(k,f)=>handlers[k]=f,querySelectorAll:selector=>selector==='[data-fm-tree-item]'?nodes:[],querySelector:()=>null};
 const host={querySelector:()=>section};
 const render=()=>{html=ws.page();nodes=ws.state.panes.flatMap((p,index)=>localTreeRows(p).map(r=>({dataset:{pane:String(index),path:r.path,fmTreeItem:'',fm:'tree'},closest:sel=>sel==='[data-fm-tree]'?{dataset:{fmTree:String(index)}}:sel.includes('data-fm')?nodes.find(n=>n.dataset.pane===String(index)&&n.dataset.path===r.path):null,focus(){focus.push([index,r.path]);global.document.activeElement=this},scrollIntoView(){}})));ws.mount(host);};
 ws=create({esc:String,icon:()=>'',render,invoke,error:e=>errors.push(e),archive:()=>({path:'outer.zip',entries:[]})});
 for(const p of ws.state.panes)p.loaded=true;
 const previous=global.document;global.document={activeElement:null,hasFocus:()=>true,querySelectorAll:()=>[],querySelector:()=>null,addEventListener(){}};
 const node=(path,index=0)=>nodes.find(n=>n.dataset.path===path&&n.dataset.pane===String(index));
 const key=(path,key,index=0)=>{global.document.activeElement=node(path,index);handlers.keydown({target:node(path,index),key,preventDefault(){},stopPropagation(){}})};
 return {ws,render,key,node,focus,errors,handlers,get html(){return html},close(){global.document=previous}};
}
test('local tree uses one roving item per pane, valid flat tree levels and no duplicate paths',()=>{
 const h=harness();try{const p=h.ws.state.panes[0];p.tree.set('',[entry('C:/')]);p.tree.set('C:/',[entry('C:/a'),{...entry('C:/link'),linked:true}]);p.treeExpanded.add('C:/');p.path='C:/a';h.render();const rows=localTreeRows(p);assert.deepEqual(rows.map(r=>[r.path,r.parent,r.level]),[['',null,1],['C:/','',2],['C:/a','C:/',3]]);assert.equal((h.html.match(/data-fm-tree-item tabindex="0"/g)||[]).length,2);assert.match(h.html,/aria-selected="true"/);assert.ok(!rows.some(r=>r.path.includes('link')));
 }finally{h.close()}
});
test('tree arrow navigation expands, collapses, returns to parents and Home End stay in tree',()=>{
 const h=harness();try{const p=h.ws.state.panes[0];p.tree.set('',[entry('C:/'),entry('D:/')]);p.tree.set('C:/',[entry('C:/a')]);h.render();h.key('','ArrowDown');assert.equal(p.treeCursor,'C:/');h.key('C:/','ArrowRight');assert.ok(p.treeExpanded.has('C:/'));h.key('C:/','ArrowRight');assert.equal(p.treeCursor,'C:/a');h.key('C:/a','ArrowLeft');assert.equal(p.treeCursor,'C:/');h.key('C:/','ArrowLeft');assert.ok(!p.treeExpanded.has('C:/'));h.key('C:/','End');assert.equal(p.treeCursor,'D:/');h.key('D:/','Home');assert.equal(p.treeCursor,'');assert.equal(p.path,'');assert.equal(p.selected.size,0);
 }finally{h.close()}
});
test('async expansion preserves a later keyboard focus and never navigates the file list',async()=>{
 let resolve;const h=harness(()=>new Promise(r=>resolve=r));try{const p=h.ws.state.panes[0];p.tree.set('',[entry('C:/'),entry('D:/')]);h.render();h.key('C:/','ArrowRight');assert.ok(p.treePending.has('C:/'));h.key('C:/','ArrowDown');assert.equal(p.treeCursor,'D:/');resolve({path:'C:/',entries:[entry('C:/child')]});await new Promise(setImmediate);assert.ok(p.treeExpanded.has('C:/'));assert.deepEqual(h.focus.at(-1),[0,'D:/']);assert.equal(p.path,'');assert.equal(h.errors.length,0);
 }finally{h.close()}
});
test('Left cancels pending expansion result without destructive commands',async()=>{
 let resolve,calls=[];const h=harness((op,args)=>{calls.push([op,args]);return new Promise(r=>resolve=r)});try{const p=h.ws.state.panes[0];p.tree.set('',[entry('C:/')]);h.render();h.key('C:/','ArrowRight');h.key('C:/','ArrowLeft');resolve({path:'C:/',entries:[entry('C:/child')]});await new Promise(setImmediate);assert.ok(!p.tree.has('C:/'));assert.ok(!p.treeExpanded.has('C:/'));assert.deepEqual(calls.map(c=>c[0]),['browse_local']);
 }finally{h.close()}
});
test('Enter selects location and async completion does not steal focus after Tab away',async()=>{
 let resolve;const h=harness(()=>new Promise(r=>resolve=r));try{const p=h.ws.state.panes[0];p.tree.set('',[entry('C:/')]);h.render();h.key('C:/','Enter');global.document.activeElement=null;const count=h.focus.length;resolve({path:'C:/',parent:'',entries:[entry('C:/a')]});await new Promise(setImmediate);assert.equal(p.path,'C:/');assert.equal(p.treeCursor,'C:/');assert.equal(h.focus.length,count);assert.match(h.html,/data-path="C:\/"[^>]+aria-selected="true"/);
 }finally{h.close()}
});
test('tree model supports deep cached paths without the old depth20 truncation',()=>{
 const h=harness();try{const p=h.ws.state.panes[0];let parent='';for(let i=0;i<30;i++){const path='C:/'+Array(i+1).fill('a').join('/');p.tree.set(parent,[entry(path)]);p.treeExpanded.add(parent);parent=path;}assert.equal(localTreeRows(p).length,31);
 }finally{h.close()}
});
