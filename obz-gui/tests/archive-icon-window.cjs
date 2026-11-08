const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

test('large icons retain the logical grid across scrolling, reflow and focus jumps', () => {
  let columns = 6, tileHeight = 160, cursor = 0;
  const nodes = [], watched = new Set(), document = {activeElement:null};
  const viewport = {
    clientHeight:480, scrollTop:0, getBoundingClientRect:()=>({top:0,bottom:480}),
    addEventListener(){}, removeEventListener(){}, querySelector:()=>null,
  };
  const body = {
    clientWidth:900, isConnected:true, ownerDocument:document, style:{removeProperty(){}},
    get firstElementChild(){return nodes[0] || null}, get lastElementChild(){return nodes.at(-1) || null},
    getBoundingClientRect:()=>({top:-viewport.scrollTop}), contains:node=>nodes.includes(node), replaceChildren(){nodes.length=0},
    insertBefore(node,next){node.remove();nodes.splice(next ? nodes.indexOf(next) : nodes.length,0,node);},
    insertAdjacentHTML(where,html){
      const node = {
        i:Number(html), style:{}, getBoundingClientRect:()=>({height:tileHeight}),
        get nextElementSibling(){return nodes[nodes.indexOf(node)+1] || null},
        remove(){const i=nodes.indexOf(node);if(i>=0)nodes.splice(i,1);},
      };
      nodes.push(node);
    },
  };
  const env={
    getComputedStyle:()=>({gridTemplateColumns:Array(columns).fill('140px').join(' '),rowGap:'10px',columnGap:'10px',paddingTop:'15px',paddingBottom:'15px'}),
    ResizeObserver:class{observe(){} disconnect(){}}, cancelAnimationFrame(){},
  };
  const code=fs.readFileSync(path.join(__dirname,'../frontend/archive-icon-window.js'),'utf8');
  const Grid=vm.runInNewContext(code+'\nArchiveIconWindow',env);
  const grid=new Grid(body,viewport,Array(50000),(_,i)=>String(i),()=>cursor,n=>watched.add(n),n=>watched.delete(n));
  assert.ok(nodes.length<40);
  viewport.scrollTop=17015;grid.paint();
  assert.ok(nodes.includes(grid.nodes.get(0)));
  assert.equal(grid.nodes.get(600).style.top,'17015px');
  columns=3;tileHeight=240;grid.paint();
  assert.equal(viewport.scrollTop,50015);
  assert.equal(grid.nodes.get(601).style.insetInlineStart,'150px');
  cursor=49999;grid.reveal(cursor);
  assert.ok(grid.nodes.has(cursor));assert.ok(nodes.length<40);
  assert.deepEqual(nodes.map(n=>n.i),nodes.map(n=>n.i).sort((a,b)=>a-b));
  assert.equal(watched.size,nodes.length);grid.stop();assert.equal(watched.size,0);
});
