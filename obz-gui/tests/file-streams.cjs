const test=require('node:test'),assert=require('node:assert/strict');
const {create}=require('../frontend/file-workspace.js');
const esc=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
function harness(invoke){let click;const w=create({esc,icon:()=>'',render(){},error:e=>{throw e},invoke});for(const p of w.state.panes)Object.assign(p,{loaded:true,path:'C:/test'});Object.assign(w.state.panes[0],{entries:[{name:'file',path:'C:/test/file',is_dir:false,size:1}],selected:new Set(['C:/test/file'])});w.mount({querySelector:()=>({addEventListener:(type,fn)=>{if(type==='click')click=fn;}})});return{w,click:action=>click({preventDefault(){},target:{closest:()=>({dataset:{fm:action,pane:'0'}})}})};}
test('secondary local stream action lists escaped metadata, empty and error states',async()=>{
 const calls=[];let response={streams:[{name:'<img>',size:0,allocated_size:0}]};const h=harness(async(cmd,args)=>{calls.push({cmd,args});if(response instanceof Error)throw response;return response;});
 assert.match(h.w.page(),/附加数据流…/);h.click('streams');await new Promise(setImmediate);assert.deepEqual(calls,[{cmd:'local_streams',args:{path:'C:/test/file'}}]);assert.match(h.w.page(),/&lt;img>/);assert.ok(!h.w.page().includes('<img>'));assert.match(h.w.page(),/大小（字节）/);assert.match(h.w.page(),/qa-table fm-stream-table/);assert.match(h.w.page(),/fm-stream-actions/);assert.ok(!h.w.page().includes('不提供修改或删除'));
 h.click('streams-close');response={streams:[]};h.click('streams');await new Promise(setImmediate);assert.match(h.w.page(),/没有附加数据流/);
 h.click('streams-close');response=Error('<denied>');h.click('streams');await new Promise(setImmediate);assert.match(h.w.page(),/role="alert"/);assert.match(h.w.page(),/&lt;denied>/);assert.ok(!h.w.page().includes('没有附加数据流'));
});
test('late stream result cannot reopen closed view and archives/linked entries cannot invoke',async()=>{
 let resolve;const calls=[];const h=harness((cmd,args)=>{calls.push(cmd);return new Promise(r=>resolve=r)});h.click('streams');assert.match(h.w.page(),/正在读取/);h.click('streams-close');resolve({streams:[]});await new Promise(setImmediate);assert.equal(h.w.state.streams,null);
 h.w.state.panes[0].kind='archive';h.click('streams');assert.equal(calls.length,1);h.w.state.panes[0].kind='local';h.w.state.panes[0].entries[0].linked=true;h.click('streams');assert.equal(calls.length,1);
});

test('refresh retains original path and source pane after selection disappears',async()=>{
 const calls=[];let deleted=false;const h=harness(async(cmd,args)=>{calls.push(args.path);if(deleted)throw Error('目标已删除');return{streams:[]};});
 h.click('streams');await new Promise(setImmediate);h.w.state.panes[0].selected.clear();h.w.state.panes[0].entries=[];h.w.state.active=1;deleted=true;
 h.click('streams-refresh');await new Promise(setImmediate);assert.deepEqual(calls,['C:/test/file','C:/test/file']);assert.match(h.w.page(),/目标已删除/);assert.equal(h.w.state.streams.index,0);h.click('streams-close');assert.equal(h.w.state.active,0);
});

test('stream names remain complete accessible text with table column headers',()=>{
 const h=harness(async()=>({streams:[]})),name='备注-😀-'.repeat(40);h.w.state.streams={path:'C:/test/file',index:0,pending:false,error:'',entries:[{name,size:7,allocated_size:8}]};const html=h.w.page();assert.ok(html.includes(name));assert.equal((html.match(/scope="col"/g)||[]).length,3);
});
