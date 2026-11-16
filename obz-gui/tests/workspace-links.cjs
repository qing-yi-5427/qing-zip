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
test('both panes showing the destination refresh with automatic refresh disabled',async()=>{
 const t=setup(async()=>({path:'C:/left/new'}));t.ws.state.panes[0].autoRefresh=false;Object.assign(t.ws.state.panes[1],{path:'C:/left',autoRefresh:false});t.click('link');t.edit('name','new');t.submit();await tick();assert.deepEqual(t.calls,['C:/left','C:/left']);
});

test('directory junction submits its typed kind without symbolic privilege guidance',async()=>{
 const requests=[];const t=setup(async body=>{requests.push(body);return {path:'D:/right/junction'};},true);t.click('link');
 assert.match(t.ws.page(),/value="junction"/);t.handlers.change({target:{value:'junction',hasAttribute:key=>key==='data-fm-link-kind'}});
 assert.equal(t.ws.state.link.kind,'junction');assert.match(t.ws.page(),/无需符号链接创建权限/);t.edit('name','junction');t.submit();await tick();
 assert.deepEqual(requests,[{operation:'fs-create-link',path:'C:/left/source<.txt',dest:'D:/right',name:'junction',link_kind:'junction',options:{overwrite:'error'}}]);assert.deepEqual(t.calls,['D:/right']);
});

test('junction target input accepts dangling directory text',async()=>{
 const requests=[];const t=setup(async body=>{requests.push(body);return {path:'D:/right/new'};},true);t.click('link');t.handlers.change({target:{value:'junction',hasAttribute:key=>key==='data-fm-link-kind'}});
 t.handlers.input({target:{value:'C:/missing/child',hasAttribute:key=>key==='data-fm-link-target',closest:()=>null}});t.edit('name','new');t.submit();await tick();assert.equal(requests[0].path,'C:/missing/child');assert.equal(requests[0].link_kind,'junction');
});
test('WSL raw target has its own field, validates empty input and retains failure input',async()=>{
 const requests=[];const t=setup(async body=>{requests.push(body);throw Error('fixture failure');});t.click('link');t.handlers.change({target:{value:'wsl',hasAttribute:key=>key==='data-fm-link-kind'}});
 assert.match(t.ws.page(),/WSL 目标路径/);assert.equal(t.ws.state.link.wslTarget,'');t.submit();assert.equal(requests.length,0);assert.match(t.ws.state.link.error,/WSL/);
 t.handlers.input({target:{value:'../用户/target',hasAttribute:key=>key==='data-fm-link-wsl-target',closest:()=>null}});t.edit('name','wsl');t.submit();await tick();
 assert.deepEqual(requests,[{operation:'fs-create-link',wsl_target:'../用户/target',dest:'D:/right',name:'wsl',link_kind:'wsl',options:{overwrite:'error'}}]);assert.equal(t.ws.state.link.wslTarget,'../用户/target');assert.equal(t.ws.state.link.kind,'wsl');assert.match(t.ws.state.link.error,/fixture failure/);
});

const receipt={volume:'01234567',file_id:'fedcba9876543210',tag:'a000001d',reparse_sha256:'a'.repeat(64)};
const inspected={path:'C:/left/source<.txt',kind:'wsl',target:'../actual<',print_target:'cosmetic<',relative:false,can_edit:true,edit_reason:null,receipt};
function editorSetup(run){
 const t=setup(run);t.ws.state.panes[0].entries[0].linked=true;t.ws.state.panes[0].selected.clear();
 t.click=action=>t.handlers.click({preventDefault(){},target:{closest:s=>s==='[data-fm]'?{dataset:{fm:action,pane:'0',path:'C:/left/source<.txt',index:'0'}}:null}});
 t.editTarget=value=>t.handlers.input({target:{value,hasAttribute:key=>key==='data-fm-link-edit-target',closest:()=>null}});
 t.submitEdit=()=>t.handlers.submit({preventDefault(){},target:{closest:s=>s==='[data-fm-link-editor]'?{}:null}});return t;
}
test('linked row has dedicated inspection without enabling selection or ordinary open/copy',async()=>{
 const requests=[];const t=editorSetup(async body=>{requests.push(body);return inspected;});assert.match(t.ws.page(),/查看\/编辑链接/);assert.match(t.ws.page(),/type="checkbox"[^>]*disabled/);
 t.click('entry');assert.equal(t.ws.state.panes[0].selected.size,0);t.click('copy');await tick();assert.equal(requests.length,0);
 t.click('link-inspect');await tick();assert.deepEqual(requests,[{operation:'fs-inspect-link',path:'C:/left/source<.txt'}]);assert.equal(t.ws.state.linkEditor.target,'../actual<');assert.match(t.ws.page(),/value="\.\.\/actual&lt;"/);assert.match(t.ws.page(),/显示名称：cosmetic&lt;/);
 t.click('link-editor-close');t.ws.state.panes[0].kind='archive';t.click('link-inspect');await tick();assert.equal(requests.length,1);
});
test('closing pending inspection prevents a late response reopening the editor',async()=>{
 let done;const t=editorSetup(()=>new Promise(resolve=>done=resolve));t.click('link-inspect');assert.equal(t.ws.state.busy,true);t.click('link-editor-close');assert.equal(t.ws.state.linkEditor,null);done(inspected);await tick();assert.equal(t.ws.state.linkEditor,null);assert.equal(t.ws.state.busy,false);
});
test('failed edit preserves target and receipt and busy forbids duplicate edits or closing',async()=>{
 let reject;const requests=[];const t=editorSetup(body=>{requests.push(body);return body.operation==='fs-inspect-link'?inspected:new Promise((_,r)=>reject=r);});t.click('link-inspect');await tick();t.editTarget('../changed');t.submitEdit();t.submitEdit();t.click('link-editor-close');
 assert.equal(requests.length,2);assert.equal(t.ws.state.busy,true);assert.equal(t.ws.state.linkEditor.target,'../changed');reject(Error('fixture failure'));await tick();assert.equal(t.ws.state.linkEditor.target,'../changed');assert.deepEqual(t.ws.state.linkEditor.data.receipt,receipt);assert.equal(t.ws.state.linkEditor.error,'fixture failure');assert.equal(t.ws.state.busy,false);
});
test('edit refreshes all matching panes with auto refresh off and Windows spelling differences',async()=>{
 const requests=[];const t=editorSetup(async body=>{requests.push(body);return body.operation==='fs-inspect-link'?inspected:{removed:false};});t.ws.state.panes[0].autoRefresh=false;Object.assign(t.ws.state.panes[1],{path:'C:\\left',autoRefresh:false});t.click('link-inspect');await tick();t.editTarget('/new');t.submitEdit();await tick();
 assert.deepEqual(requests[1],{operation:'fs-edit-link',path:'C:/left/source<.txt',link_receipt:receipt,link_target:'/new',options:{overwrite:'error'}});assert.deepEqual(t.calls,['C:/left','C:\\left']);assert.equal(t.ws.state.linkEditor,null);
});
test('late edit completion never refreshes a pane that navigated away',async()=>{
 let done;const t=editorSetup(body=>body.operation==='fs-inspect-link'?inspected:new Promise(resolve=>done=resolve));t.click('link-inspect');await tick();t.editTarget('/new');t.submitEdit();t.ws.state.panes[0].path='E:/new';t.ws.state.panes[0].requestId++;done({removed:false});await tick();assert.deepEqual(t.calls,[]);assert.equal(t.ws.state.panes[0].path,'E:/new');
});
test('blank target cannot clear; clear requires its distinct confirmation and preserves original entry',async()=>{
 const requests=[];const t=editorSetup(async body=>{requests.push(body);return body.operation==='fs-inspect-link'?inspected:{removed:true};});t.click('link-inspect');await tick();t.editTarget('');t.submitEdit();assert.equal(requests.length,1);assert.match(t.ws.state.linkEditor.error,/目标不能为空/);
 t.click('link-editor-clear-confirm');assert.equal(requests.length,1);t.click('link-editor-clear');assert.equal(requests.length,1);assert.match(t.ws.page(),/项目本身和原有数据都会保留/);t.click('link-editor-clear-confirm');await tick();
 assert.deepEqual(requests[1],{operation:'fs-clear-link-data',path:'C:/left/source<.txt',link_receipt:receipt,options:{overwrite:'error'}});assert.deepEqual(t.calls,['C:/left']);assert.equal(t.ws.state.linkEditor,null);
});
test('readonly inspection reports reason and cannot submit edit or clear',async()=>{
 let calls=0;const t=editorSetup(async()=>{calls++;return {...inspected,can_edit:false,edit_reason:'多硬链接对象仅支持查看'};});t.click('link-inspect');await tick();assert.match(t.ws.page(),/多硬链接对象仅支持查看/);t.submitEdit();t.click('link-editor-clear');t.click('link-editor-clear-confirm');await tick();assert.equal(calls,1);
});

test('return keeps a target draft but reopening always obtains a fresh receipt',async()=>{
 const requests=[];const t=editorSetup(async body=>{requests.push(body);return {...inspected,receipt:{...receipt,reparse_sha256:String(requests.length).repeat(64)}};});t.click('link-inspect');await tick();t.editTarget('../draft');t.click('link-editor-close');assert.equal(t.ws.state.linkEditor,null);t.click('link-inspect');await tick();assert.equal(t.ws.state.linkEditor.target,'../draft');assert.equal(requests.length,2);assert.equal(t.ws.state.linkEditor.data.receipt.reparse_sha256,'2'.repeat(64));assert.match(t.ws.page(),/当前实际目标：\.\.\/actual&lt;/);
});

test('a replacement object at the same path never receives the previous object draft',async()=>{
 let calls=0;const t=editorSetup(async()=>{calls++;return calls===1?inspected:{...inspected,target:'/foreign-target',receipt:{...receipt,file_id:'1111111111111111'}};});t.click('link-inspect');await tick();t.editTarget('../old-object-draft');t.click('link-editor-close');t.click('link-inspect');await tick();assert.equal(t.ws.state.linkEditor.target,'/foreign-target');assert.equal(t.ws.state.linkEditor.data.receipt.file_id,'1111111111111111');assert.equal(t.ws.state.linkDrafts.size,0);assert.match(t.ws.page(),/未恢复旧草稿/);
});
