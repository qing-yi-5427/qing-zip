const test=require('node:test'),assert=require('node:assert/strict');
const {create}=require('../frontend/file-workspace.js');
const tick=()=>new Promise(setImmediate);
function setup(run,directory=false){
 const handlers={},calls=[],errors=[];const ws=create({esc:s=>String(s).replaceAll('<','&lt;'),icon:()=>'',render(){},error:e=>errors.push(e),run,invoke:async(_,v)=>{calls.push(v.path);return {path:v.path,parent:'C:/',entries:[]};}});
 for(const [index,p] of ws.state.panes.entries())Object.assign(p,{loaded:true,path:index?'D:/right':'C:/left'});
 Object.assign(ws.state.panes[0],{entries:[{name:'source<.txt',path:'C:/left/source<.txt',is_dir:directory,linked:false,size:4}],selected:new Set(['C:/left/source<.txt'])});
 ws.mount({querySelector:()=>({addEventListener:(kind,fn)=>handlers[kind]=fn})});
 const click=action=>handlers.click({preventDefault(){},target:{closest:s=>s==='[data-fm]'?{dataset:{fm:action,pane:'0'}}:null}});
 const edit=(field,value)=>handlers.input({target:{value,hasAttribute:k=>k==='data-fm-link-'+field,closest:()=>null,dataset:{}}});
 const submit=()=>handlers.submit({preventDefault(){},target:{closest:s=>s==='[data-fm-link]'?{}:null}});
 return {ws,handlers,calls,errors,click,edit,submit};
}
test('local link form filters types, escapes target, uses opposite pane and ignores transfer overwrite',async()=>{
 const requests=[];const t=setup(async body=>{requests.push(body);return {path:'D:/right/new'};});t.ws.state.overwrite='overwrite';t.click('link');
 assert.equal(t.ws.state.link.parent,'D:/right');assert.match(t.ws.page(),/共享同一文件内容/);assert.match(t.ws.page(),/source&lt;/);assert.doesNotMatch(t.ws.page(),/value="symbolic-directory"/);
 t.edit('name','new');t.submit();await tick();assert.deepEqual(requests,[{operation:'fs-create-link',path:'C:/left/source<.txt',dest:'D:/right',name:'new',link_kind:'hard',options:{overwrite:'error'}}]);assert.deepEqual(t.calls,['D:/right']);assert.equal(t.ws.state.link,null);assert.equal(t.ws.state.busy,false);
 const d=setup(async()=>{},true);d.click('link');assert.equal(d.ws.state.link.kind,'symbolic-directory');assert.doesNotMatch(d.ws.page(),/value="hard"|value="symbolic-file"/);
});
test('failed link retains all inputs and repeated submission has one native request',async()=>{
 let reject;let calls=0;const t=setup(()=>{calls++;return new Promise((_,r)=>reject=r);});t.click('link');t.edit('name','existing');t.submit();t.submit();t.click('link-close');
 assert.equal(calls,1);assert.equal(t.ws.state.busy,true);assert.equal(t.ws.state.link.name,'existing');assert.match(t.ws.page(),/正在创建/);
 reject(Error('exists (os error 183)'));await tick();assert.equal(t.ws.state.link.name,'existing');assert.equal(t.ws.state.link.parent,'D:/right');assert.match(t.ws.state.link.error,/同名项目已存在/);assert.equal(t.ws.state.busy,false);assert.equal(t.calls.length,0);
});
test('invalid names do not submit; stale completion cannot refresh a newer destination',async()=>{
 let done,calls=0;const t=setup(()=>{calls++;return new Promise(r=>done=r);});t.click('link');
 for(const name of ['', '..', 'a/b']){t.edit('name',name);t.submit();assert.match(t.ws.state.link.error,/单个/);}assert.equal(calls,0);
 t.edit('name','new');t.submit();t.ws.state.panes[1].path='E:/new';t.ws.state.panes[1].requestId++;done({path:'D:/right/new'});await tick();assert.equal(t.calls.length,0);assert.equal(t.ws.state.panes[1].path,'E:/new');
});
test('archive and linked selections never launch link creation',()=>{
 const t=setup(async()=>{});t.ws.state.panes[0].kind='archive';t.click('link');assert.equal(t.ws.state.link,null);t.ws.state.panes[0].kind='local';t.ws.state.panes[0].entries[0].linked=true;t.click('link');assert.equal(t.ws.state.link,null);
});

test('Windows path spelling refreshes the matching destination and displays one separator',async()=>{
 const t=setup(async()=>({path:'D:/right/new'}));t.ws.state.panes[1].path='D:\\right';t.click('link');t.edit('name','new');assert.ok(t.ws.page().includes('新链接：D:\\right\\new'));t.edit('parent','D:/right');t.submit();await tick();assert.deepEqual(t.calls,['D:\\right']);
});
