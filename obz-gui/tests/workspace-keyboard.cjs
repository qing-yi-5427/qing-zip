const test=require('node:test'),assert=require('node:assert/strict');
const {create}=require('../frontend/file-workspace.js');
test('keyboard ranges, cross-page selection, focus restoration and linked entries',()=>{
 const handlers={},focused=[];let ws;const section={addEventListener:(k,f)=>handlers[k]=f,querySelector:s=>({focus:()=>focused.push(s),scrollIntoView(){}})};
 ws=create({esc:String,icon:()=>'',render:()=>ws.mount({querySelector:()=>section}),error:e=>{throw e}});
 for(const p of ws.state.panes)p.loaded=true;
 const p=ws.state.panes[0];p.entries=Array.from({length:405},(_,i)=>({path:'C:/'+i,name:''+i,size:1,linked:i===2}));ws.mount({querySelector:()=>section});
 const key=(n,k,mods={})=>handlers.keydown({key:k,preventDefault(){},stopPropagation(){},target:{closest:()=>({dataset:{pane:'0',index:''+n}})},...mods});
 key(0,'ArrowDown');assert.deepEqual([...p.selected],['C:/1']);assert.equal(p.cursor,'C:/1');
 key(1,'ArrowDown',{shiftKey:true});assert.deepEqual([...p.selected],['C:/1','C:/3']);
 key(3,'ArrowDown',{ctrlKey:true});assert.equal(p.cursor,'C:/4');assert.equal(p.selected.size,2);
 key(4,'End',{shiftKey:true});assert.equal(p.offset,400);assert.equal(p.selected.size,403);assert.ok(focused.at(-1).includes('404'));
 key(404,'a',{ctrlKey:true});assert.equal(p.selected.size,404);assert.ok(!p.selected.has('C:/2'));
 key(404,'Home');assert.deepEqual([...p.selected],['C:/0']);assert.equal(p.offset,0);
});
