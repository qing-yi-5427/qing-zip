// Exercise the production window against a small DOM/geometry test double.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(path.join(__dirname,'../frontend/archive-row-window.js'),'utf8');
function viewFixture(count=50000){
  let cursor=0, nextFrame=0, observer;
  const sizes=new Map(),frames=new Map();
  const document={activeElement:null,createElement:()=>new Element()};
  class Element {
    constructor(){this.children=[];this.parent=null;this.style={};this.attributes={};this.ownerDocument=document;this.isConnected=true;}
    get firstElementChild(){return this.children[0]||null}
    get lastElementChild(){return this.children.at(-1)||null}
    get previousElementSibling(){if(!this.parent)return null;return this.parent.children[this.parent.children.indexOf(this)-1]||null}
    get nextElementSibling(){if(!this.parent)return null;return this.parent.children[this.parent.children.indexOf(this)+1]||null}
    setAttribute(k,v){this.attributes[k]=v}
    insertBefore(node,ref){node.remove();const i=ref===null?this.children.length:this.children.indexOf(ref);assert.ok(i>=0);this.children.splice(i,0,node);node.parent=this;}
    insertCell(){const cell=new Element();this.insertBefore(cell,null);return cell}
    remove(){if(this.parent){const a=this.parent.children;a.splice(a.indexOf(this),1);this.parent=null;}}
    replaceChildren(){for(const child of [...this.children])child.remove()}
    contains(node){return node===this||this.children.some(e=>e.contains(node))}
    focus(){document.activeElement=this}
    getBoundingClientRect(){return{height:sizes.get(this.index)||56}}
    insertAdjacentHTML(where,html){
      assert.equal(where,'beforeend');
      for(const match of html.matchAll(/<tr data-i="(\d+)" aria-rowindex="(\d+)">/g)){
        const row=new Element();row.index=Number(match[1]);row.setAttribute('aria-rowindex',match[2]);this.insertBefore(row,null);
      }
    }
  }
  const viewport={scrollTop:0,querySelector:()=>null,clientWidth:900,clientHeight:320,listeners:new Map(),getBoundingClientRect:()=>({top:0,bottom:320}),
    addEventListener(k,fn){this.listeners.set(k,fn)},removeEventListener(k){this.listeners.delete(k)}};
  const table=new Element(),header=new Element(),body=new Element();table.insertBefore(header,null);table.insertBefore(body,null);
  header.getBoundingClientRect=()=>({bottom:84-viewport.scrollTop});
  const env={getComputedStyle:()=>({getPropertyValue:()=>'56px'}),
    requestAnimationFrame:fn=>{frames.set(++nextFrame,fn);return nextFrame},cancelAnimationFrame:id=>frames.delete(id),
    ResizeObserver:class{constructor(fn){this.fn=fn;this.watched=new Set();observer=this}observe(x){this.watched.add(x)}unobserve(x){this.watched.delete(x)}disconnect(){this.watched.clear()}}};
  vm.createContext(env);vm.runInContext(source+'\nthis.Classes={ArchiveRowWindow,ArchiveRowHeights}',env);
  const list=Array.from({length:count},(_,i)=>i);
  const mounted=new env.Classes.ArchiveRowWindow(body,viewport,list,(e,i)=>`<tr data-i="${i}" aria-rowindex="${i+2}"></tr>`,()=>cursor);
  return {body,viewport,window:mounted,observer,frames,document,sizes,setCursor:i=>cursor=i,advance(){const pending=[...frames.values()];frames.clear();for(const f of pending)f();},Heights:env.Classes.ArchiveRowHeights};
}
test('50000 detail rows stay bounded through scrolling and logical End/Home focus',()=>{
  const f=viewFixture(),last=49999,initial=f.window.nodes.get(0);initial.focus();
  assert.ok(f.window.nodes.size<32);
  f.viewport.scrollTop=84+34000*56;f.viewport.listeners.get('scroll')();f.viewport.listeners.get('scroll')();
  assert.equal(f.frames.size,1);f.advance();
  assert.strictEqual(f.window.nodes.get(0),initial);assert.strictEqual(f.document.activeElement,initial);
  assert.ok(f.window.nodes.has(34003));assert.ok(f.window.nodes.size<32);
  assert.equal(f.window.nodes.get(34003).attributes['aria-rowindex'],'34005');
  const physical=f.body.children.map(n=>n.index).filter(i=>i!==undefined);assert.deepEqual(physical,[...physical].sort((a,b)=>a-b));
  const projected=f.body.children.reduce((h,n)=>h+(n.index===undefined?parseFloat(n.firstElementChild.style.height):n.getBoundingClientRect().height),0);
  assert.equal(projected,50000*56);
  f.setCursor(last);f.window.reveal(last);assert.ok(f.window.nodes.has(last));assert.equal(f.window.nodes.get(last).attributes['aria-rowindex'],'50001');
  f.setCursor(0);f.window.reveal(0);assert.ok(f.window.nodes.has(0));assert.ok(!f.window.nodes.has(last));
  f.viewport.listeners.get('scroll')();f.window.stop();assert.equal(f.frames.size,0);assert.equal(f.viewport.listeners.size,0);assert.equal(f.observer.watched.size,0);
});
test('keyboard navigation keeps the target row between the sticky controls',()=>{
  const f=viewFixture();
  f.viewport.querySelector=selector=>selector==='.qa-path'
    ? {getBoundingClientRect:()=>({bottom:45})}
    : {getBoundingClientRect:()=>({top:260})};
  f.setCursor(49999);f.window.reveal(49999);
  assert.equal(f.window.heights.before(50000)-f.window.visibleTop(),260);
  f.setCursor(0);f.window.reveal(0);
  assert.equal(f.window.heights.before(0)-f.window.visibleTop(),45);
});
test('wrapped rows change cumulative height and are remeasured after a width change',()=>{
  const f=viewFixture(1000);f.sizes.set(0,112);f.observer.fn();f.advance();
  assert.equal(f.window.heights.before(2),168);assert.equal(f.window.heights.at(111),0);assert.equal(f.window.heights.at(112),1);
  f.viewport.scrollTop=84+56*310+56;f.window.paint();assert.equal(f.window.heights.at(f.window.visibleTop()),310);
  f.viewport.clientWidth=550;f.sizes.set(0,168);f.observer.fn();f.advance();
  assert.equal(f.window.heights.at(f.window.visibleTop()),310);
  f.sizes.set(306,140);f.observer.fn();f.advance();assert.equal(f.window.heights.at(f.window.visibleTop()),310);
});
test('height lookup matches the cumulative model for sparse, fractional and shrinking rows',()=>{
  const f=viewFixture(300),model=new f.Heights(257,42),heights=Array(257).fill(42);
  for(const [i,h] of [[0,87],[21,76.5],[80,43],[234,124],[21,42],[256,61]]){model.measure(i,h);heights[i]=h}
  let offset=0;
  for(const [i,h] of heights.entries()){assert.equal(model.before(i),offset);assert.equal(model.at(offset),i);assert.equal(model.at(offset+h-.01),i);offset+=h;}
  assert.equal(model.before(heights.length),offset);assert.equal(model.at(offset+1000),256);
});

test('the archive view uses the window only for large details and keeps icon thumbnails',()=>{
  const app=fs.readFileSync(path.join(__dirname,'../frontend/app.js'),'utf8');
  const excerpt=app.slice(app.indexOf('const windowThreshold'),app.indexOf('\n}',app.indexOf('function rows() {'))+2);
  const table={attributes:{},classList:{toggle(){}},setAttribute(k,v){this.attributes[k]=v}};
  const body={closest:()=>table,innerHTML:''};
  const state={selected:new Set(),listStyle:'details'},calls=[];
  const env={s:state,main:{},document:{activeElement:null},detailWindow:null,
    $:()=>body,positionsFor:l=>new Map(l.map((e,i)=>[e.name,i])),visibleEntries:()=>env.list,
    refreshSelection:()=>calls.push('selection'),stopThumbnailObserver:()=>calls.push('stop'),observeThumbnails:()=>calls.push('observe'),
    icon:()=>'',esc:s=>s,size:()=>'',entrySize:()=>'',displayModified:()=>'',isImage:()=>true,
    ArchiveRowWindow:class{constructor(b,m,l,row,cursor){Object.assign(this,{body:b,list:l,row,cursor});calls.push('mount')}paint(){calls.push('paint')}stop(){calls.push('release')}}};
  vm.createContext(env);vm.runInContext(excerpt,env);
  const entries=n=>Array.from({length:n},(_,i)=>({name:i+'.png',leaf:i+'.png',is_dir:false}));
  env.list=entries(256);env.rows();assert.equal((body.innerHTML.match(/<tr /g)||[]).length,256);
  assert.equal(table.attributes['aria-rowcount'],'257');assert.ok(!body.innerHTML.includes('data-thumbnail='));
  env.list=entries(257);env.rows();assert.equal(calls.filter(x=>x==='mount').length,1);
  assert.match(env.detailWindow.row(env.list[256],256),/aria-rowindex="258"/);
  env.rows();assert.ok(calls.includes('paint'));
  state.listStyle='icons';env.rows();assert.equal(calls.at(-1),'observe');assert.equal(env.detailWindow,null);
  assert.equal((body.innerHTML.match(/data-thumbnail=/g)||[]).length,257);
  state.listStyle='details';env.list=[];env.rows();assert.ok(body.innerHTML.includes('colspan="6"'));assert.equal(calls.at(-1),'stop');
});
