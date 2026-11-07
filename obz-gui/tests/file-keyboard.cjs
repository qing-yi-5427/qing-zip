// Keyboard behavior regressions; native desktop acceptance is separate.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),test=require('node:test');
const source=fs.readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
const keyboard=source.split('// File and menu keyboard behavior. Keep focus separate from the range anchor.')[1].split('// End file and menu keyboard behavior.')[0];
const select=source.split('\n').find(line=>line.startsWith('function selectRow('));
function fixture(count=8,columns=1){
 const calls=[],document={activeElement:null},buttons=[];
 const state={selected:new Set(),cursor:null,anchor:null,listStyle:columns>1?'icons':'details'};
 for(let i=0;i<count;i++){
  const row={dataset:{row:'file'+i},scrollIntoView(){},getBoundingClientRect:()=>({left:(i%columns)*100,top:Math.floor(i/columns)*50,bottom:Math.floor(i/columns)*50+50,height:50})};
  const button={dataset:{entry:row.dataset.row},tabIndex:-1,closest:()=>row,focus(){document.activeElement=button;},getBoundingClientRect:row.getBoundingClientRect};
  buttons.push(button);
 }
 const menu={hidden:true,contains:node=>menu.items.includes(node),items:[],querySelectorAll:()=>menu.items,querySelector:()=>null};
 const context={s:state,root:{querySelectorAll:()=>buttons},main:{clientHeight:200},overlay:{hidden:true},document,
  window:{getSelection:()=>({removeAllRanges(){}})},visibleEntries:()=>buttons.map(b=>({name:b.dataset.entry})),refreshSelection(){},
  entry:async name=>calls.push(['open',name]),error:message=>calls.push(['error',message]),showMenu:(group,row)=>calls.push(['menu',group,row.dataset.row]),showMore:()=>calls.push(['more']),
  $:()=>menu,menuReturnFocus:null};
 vm.createContext(context);vm.runInContext(keyboard+'\n'+select,context);
 const press=(index,key,modifiers={})=>{let prevented=false,stopped=false;const e={target:buttons[index],key,preventDefault(){prevented=true;},stopPropagation(){stopped=true;},...modifiers};context.handleFileKeydown(e);return {prevented,stopped};};
 return {context,state,buttons,calls,menu,document,press};
}
test('direction, page and boundary movement work in details and icon grids',()=>{
 const f=fixture(),index=f.context.fileKeyIndex;
 assert.equal(index('ArrowDown',3,8),4);assert.equal(index('ArrowUp',0,8),0);
 assert.equal(index('End',0,8),7);assert.equal(index('Home',7,8),0);
 assert.equal(index('ArrowDown',2,8,3),5);assert.equal(index('ArrowUp',4,8,3),1);
 assert.equal(index('PageDown',1,20,3,4),13);assert.equal(index('PageUp',13,20,3,4),1);
 assert.equal(index('ArrowRight',7,8,3),7);assert.equal(index('End',0,0),-1);
});
test('Shift expands and contracts from a stable anchor; Ctrl movement keeps selection',()=>{
 const f=fixture();f.context.selectRow('file2',{});f.press(2,'ArrowDown',{shiftKey:true});f.press(3,'ArrowDown',{shiftKey:true});
 assert.deepEqual([...f.state.selected],['file2','file3','file4']);assert.equal(f.state.anchor,'file2');
 f.press(4,'ArrowUp',{shiftKey:true});assert.deepEqual([...f.state.selected],['file2','file3']);
 f.press(3,'ArrowDown',{ctrlKey:true});assert.deepEqual([...f.state.selected],['file2','file3']);assert.equal(f.state.cursor,'file4');
 assert.equal(f.buttons.filter(b=>b.tabIndex===0).length,1);assert.equal(f.document.activeElement,f.buttons[4]);
});
test('first Shift movement anchors the focused file, not a stale hidden entry',()=>{
 const f=fixture();f.state.anchor='hidden';f.press(3,'ArrowDown',{shiftKey:true});
 assert.equal(f.state.anchor,'file3');assert.deepEqual([...f.state.selected],['file3','file4']);
});
test('icon vertical movement follows the visible column count',()=>{
 const f=fixture(8,3);f.context.selectRow('file1',{});f.press(1,'ArrowDown');
 assert.equal(f.state.cursor,'file4');assert.deepEqual([...f.state.selected],['file4']);
 f.press(4,'ArrowRight');assert.equal(f.state.cursor,'file5');
});
test('space toggles without opening; Enter opens focused entry once',()=>{
 const f=fixture();f.context.selectRow('file1',{});f.press(1,' ');assert.equal(f.state.selected.size,0);
 f.press(1,' ');assert.deepEqual([...f.state.selected],['file1']);assert.equal(f.calls.length,0);
 assert.equal(f.press(1,'Enter').prevented,true);assert.deepEqual(f.calls,[['open','file1']]);
 f.press(1,'Enter',{ctrlKey:true});assert.equal(f.calls.length,1);
});
test('keyboard context menu keeps existing multiple selection and anchors to row',()=>{
 const f=fixture();f.state.selected=new Set(['file1','file2']);f.press(1,'F10',{shiftKey:true});
 assert.deepEqual([...f.state.selected],['file1','file2']);assert.deepEqual(f.calls,[['menu','edit','file1']]);
 f.press(4,'ContextMenu');assert.deepEqual([...f.state.selected],['file4']);assert.equal(f.state.cursor,'file4');
});
test('active task or modal does not trigger archive keyboard actions',()=>{
 const f=fixture();f.state.active='job';assert.equal(f.press(0,'Enter').prevented,false);
 f.state.active=null;f.context.overlay.hidden=false;f.press(0,' ');assert.equal(f.state.selected.size,0);assert.equal(f.calls.length,0);
});
test('all menu types support wrapping, Home/End, Escape and focus return',()=>{
 const f=fixture();const items=[0,1,2].map(i=>({disabled:false,dataset:{},isConnected:true,focus(){f.document.activeElement=this;}}));
 f.menu.items=items;f.menu.hidden=false;items[2].focus();
 const origin={isConnected:true,focus(){f.document.activeElement=this;}};f.context.menuReturnFocus=origin;
 const event=key=>({key,target:f.document.activeElement,preventDefault(){},stopPropagation(){},stopImmediatePropagation(){}});
 f.context.handleMenuKeydown(event('ArrowDown'));assert.equal(f.document.activeElement,items[0]);
 f.context.handleMenuKeydown(event('End'));assert.equal(f.document.activeElement,items[2]);
 f.context.handleMenuKeydown(event('Home'));assert.equal(f.document.activeElement,items[0]);
 f.context.handleMenuKeydown(event('Escape'));assert.equal(f.menu.hidden,true);assert.equal(f.document.activeElement,origin);
});
test('context submenu uses the supplied location; app submenu retains original trigger anchor',()=>{
 const f=fixture();const positions=[];f.context.menuGroups={edit:[]};f.context.rememberMenuFocus=()=>{};f.context.btn=()=>'';
 f.context.positionMenu=(box,anchor)=>positions.push(anchor.getBoundingClientRect());
 f.context.root.querySelector=()=>({getBoundingClientRect:()=>({left:20,bottom:50})});
 f.context.$=()=>({querySelector:()=>({focus(){}})});
 vm.runInContext(source.split('\n').find(line=>line.startsWith('function showMenu(')),f.context);
 f.context.showMenu('edit',{getBoundingClientRect:()=>({left:700,bottom:420})});assert.deepEqual(positions[0],{left:700,bottom:420});
 f.context.menuAnchorRect={left:20,bottom:50};f.context.showMenu('edit',{closest:()=>true,getBoundingClientRect:()=>({left:40,bottom:200})});
 assert.equal(positions[1].left,20);assert.equal(positions[1].bottom,50);
});

test('range selection and keyboard focus use the logical order outside the mounted window',()=>{
 const f=fixture(1000),all=f.buttons;
 let mounted=all.slice(0,10);
 f.context.root.querySelectorAll=()=>mounted;
 f.context.testWindow={reveal(index){mounted=all.slice(Math.max(0,index-3),index+4)}};
 vm.runInContext("detailWindow=testWindow",f.context);
 f.context.selectRow('file2',{});
 f.press(2,'End',{shiftKey:true});
 assert.equal(f.state.cursor,'file999');assert.equal(f.state.selected.size,998);assert.equal(f.state.anchor,'file2');
 f.press(999,'Home',{ctrlKey:true});
 assert.equal(f.state.cursor,'file0');assert.equal(f.state.selected.size,998);
 f.press(0,'ArrowDown',{shiftKey:true});
 assert.deepEqual([...f.state.selected],['file1','file2']);
});
