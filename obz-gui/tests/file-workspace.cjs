const test=require('node:test'),assert=require('node:assert/strict');
const {request,archiveRows,create}=require('../frontend/file-workspace.js');
const local=(path,selected=[])=>({kind:'local',path,selected:new Set(selected)});
const archive=(selected=[])=>({kind:'archive',archive:'C:\\sample.zip',path:'folder/',password:'secret',selected:new Set(selected)});
test('local transfer and archive handoffs retain explicit selections and conflict choices',()=>{
 assert.deepEqual(request(local('C:\\left',['C:\\left\\a']),local('D:\\right'),true,'rename'),{operation:'fs-move',inputs:['C:\\left\\a'],dest:'D:\\right',options:{overwrite:'rename'}});
 const add=request(local('C:\\left',['C:\\left\\a']),archive(),false,'error');assert.equal(add.operation,'archive-add');assert.equal(add.dest,'folder/');assert.equal(add.options.overwrite,'error');assert.equal(add.path,'C:\\sample.zip');assert.equal(add.options.password,'secret');
 const extract=request(archive(['folder/a']),local('D:\\right'),false,'skip');assert.equal(extract.operation,'archive-export');assert.deepEqual(extract.source_archive.selected,['folder/a']);assert.equal(extract.source_archive.folder,'folder/');assert.equal(extract.options.password,undefined);assert.equal(extract.options.overwrite,'skip');
});
test('empty targets, in-flight navigation and unsupported archive moves cannot start a job',()=>{
 assert.throws(()=>request(local('C:\\left',['x']),local(''),false,'error'));
 assert.throws(()=>request({...local('C:\\left',['x']),pending:true},local('D:\\right'),false,'error'));
 assert.throws(()=>request(archive(['x']),local('D:\\right'),true,'error'));
 assert.equal(request(archive(['folder/x']),archive(),false,'error').operation,'archive-copy');
});
test('archive directory browsing does not include adjacent folder prefixes and preserves raw member selections',()=>{
 const entries=[{name:'folder\\a.txt',size:1,is_dir:false},{name:'folder/deep/b',size:2,is_dir:false},{name:'folder2/no',size:3,is_dir:false}];
 const rows=archiveRows(entries,'folder/');assert.deepEqual(rows.map(v=>v.name),['deep','a.txt']);assert.equal(rows[1].path,'folder\\a.txt');assert.equal(rows[0].path,'folder/deep/');
});
test('a large local folder renders a bounded page while retaining selections outside that page',()=>{
 const ui={esc:String,icon:()=>'',invoke:()=>{},render:()=>{}};const workspace=create(ui);const p=workspace.state.panes[0];
 p.entries=Array.from({length:10000},(_,i)=>({name:'f'+i,path:'C:/'+i,size:i,is_dir:false}));p.loaded=true;p.selected.add('C:/9000');
 assert.equal((workspace.page().match(/class="fm-row /g)||[]).length,200);p.offset=200;assert.ok(workspace.page().includes('data-index="399"'));assert.deepEqual([...p.selected],['C:/9000']);
});
test('an old archive refresh cannot overwrite a newer local navigation',async()=>{
 let finish,click;const updates=[];const ui={esc:String,icon:()=>'',render:()=>{},error:e=>{throw e},updated:(...a)=>updates.push(a),run:()=>new Promise(r=>finish=r),invoke:async()=>({path:'D:/target',parent:'D:/',entries:[{name:'local.txt',path:'D:/target/local.txt',size:1,is_dir:false}]})};
 const workspace=create(ui);for(const p of workspace.state.panes)p.loaded=true;
 Object.assign(workspace.state.panes[0],{kind:'archive',archive:'C:/old.zip',path:''});
 const section={addEventListener:(type,fn)=>{if(type==='click')click=fn;}};
 workspace.mount({querySelector:()=>section});
 const trigger=action=>click({preventDefault(){},target:{closest:()=>({dataset:{fm:action,pane:'0'}})}});
 trigger('refresh');assert.equal(workspace.state.panes[0].pending,true);
 trigger('local');await new Promise(setImmediate);
 finish({entries:[{name:'stale.txt',size:1,is_dir:false}]});await new Promise(setImmediate);
 assert.equal(workspace.state.panes[0].kind,'local');assert.equal(workspace.state.panes[0].path,'D:/target');
 assert.deepEqual(workspace.state.panes[0].entries.map(e=>e.name),['local.txt']);assert.deepEqual(updates,[]);
});
test('a partially failed move refreshes both panes and preserves the operation error',async()=>{
 let click;const browsed=[],errors=[];const original=Error('one item moved; second item conflicts');
 const workspace=create({esc:String,icon:()=>'',render(){},error:e=>errors.push(e),run:async()=>{throw original},invoke:async(_,v)=>{browsed.push(v.path);return {path:v.path,parent:null,entries:[]}}});
 for(const [i,p] of workspace.state.panes.entries())Object.assign(p,{loaded:true,path:i?'D:/target':'C:/source'});
 workspace.state.panes[0].selected.add('C:/source/a.txt');
 workspace.mount({querySelector:()=>({addEventListener:(type,fn)=>{if(type==='click')click=fn;}})});
 click({preventDefault(){},target:{closest:()=>({dataset:{fm:'move',pane:'0'}})}});
 await new Promise(setImmediate);
 assert.deepEqual(browsed.sort(),['C:/source','D:/target']);assert.deepEqual(errors,[original]);assert.equal(workspace.state.busy,false);
});
test('unknown archive sizes stay unknown and folders use selection then explicit entry',async()=>{
 const handlers={},calls=[];const workspace=create({esc:String,icon:()=>'',render(){},error:e=>{throw e},invoke:async(_,v)=>{calls.push(v.path);return {path:v.path,parent:'C:/',entries:[]}}});
 for(const p of workspace.state.panes)p.loaded=true;
 Object.assign(workspace.state.panes[0],{path:'C:/',entries:[{name:'folder',path:'C:/folder',is_dir:true,size:0}]});
 workspace.mount({querySelector:()=>({addEventListener:(kind,fn)=>handlers[kind]=fn})});
 const event=()=>({preventDefault(){},stopPropagation(){},target:{closest:()=>({dataset:{fm:'entry',pane:'0',index:'0'}})}});
 handlers.click(event());assert.deepEqual([...workspace.state.panes[0].selected],['C:/folder']);assert.deepEqual(calls,[]);
 handlers.dblclick(event());await new Promise(setImmediate);assert.deepEqual(calls,['C:/folder']);
 Object.assign(workspace.state.panes[0],{entries:[{name:'next',path:'C:/folder/next',is_dir:true,size:0}]});
 handlers.keydown({...event(),key:'Enter'});await new Promise(setImmediate);assert.equal(calls[1],'C:/folder/next');
 workspace.state.panes[0].entries=archiveRows([{name:'unknown',size:0,size_known:false,is_dir:false}],'');
 assert.equal(workspace.state.panes[0].entries[0].size_known,false);assert.ok(workspace.page().includes('>未知</small>'));
});

test('workspace archive refresh invalidates previews and removes only missing selections',async()=>{
 const source=require('node:fs').readFileSync(require('node:path').join(__dirname,'../frontend/app.js'),'utf8');
 for(const newline of ['\n','\r\n']){
 const code=source.replace(/\r?\n/g,newline).match(/ updated:(async\(path,entries\)=>\{[^\r\n]+\})\r?\n/)[1];
 const tab={archive:'C:/a.zip',entries:[],selected:new Set(['folder/','folder\\nested/','old.txt']),anchor:'old.txt',cursor:'old.txt',focusFile:'old.txt'};
 const state={...tab,selected:new Set(tab.selected)};const calls=[];let finish;
 const updated=Function('archiveTabs','s','archiveKey','resetPreviewState','invoke','return '+code)([tab],state,p=>p.toLowerCase(),()=>calls.push('reset'),name=>{calls.push(name);return new Promise(r=>finish=r)});
 const entries=[{name:'folder/new.txt'},{name:'folder/nested/child.txt'}];const pending=updated('C:/a.zip',entries);
 assert.deepEqual(calls,['reset','clear_preview_cache']);assert.notEqual(tab.entries,entries);
 finish();await pending;
 for(const value of [tab,state]){assert.equal(value.entries,entries);assert.deepEqual([...value.selected],['folder/','folder\\nested/']);assert.equal(value.anchor,null);assert.equal(value.cursor,null);assert.equal(value.focusFile,null);}
 }
});


test('archive copy credentials are independent and visible password inputs remain session-only',()=>{
 const source={...archive(['folder/a']),password:'SOURCE-SECRET',codepage:936};
 const target={...archive(),archive:'C:/target.7z',path:'backup/',password:'TARGET-SECRET',codepage:65001};
 const copy=request(source,target,false,'rename');assert.equal(copy.operation,'archive-copy');assert.equal(copy.path,target.archive);assert.equal(copy.dest,'backup/');assert.equal(copy.source_archive.password,'SOURCE-SECRET');assert.equal(copy.options.password,'TARGET-SECRET');assert.equal(copy.source_archive.codepage,936);assert.equal(copy.options.codepage,65001);
 const handlers={},inputs=[{dataset:{fmPassword:'0'},value:''},{dataset:{fmPassword:'1'},value:''}];
 const workspace=create({esc:String,icon:()=>'',render(){}});
 Object.assign(workspace.state.panes[0],source,{loaded:true});Object.assign(workspace.state.panes[1],target,{loaded:true});
 assert.ok(workspace.page().includes('左侧压缩包密码'));assert.ok(workspace.page().includes('右侧压缩包密码'));assert.ok(!workspace.page().includes('SOURCE-SECRET'));assert.ok(!workspace.page().includes('TARGET-SECRET'));
 workspace.mount({querySelector:()=>({querySelectorAll:()=>inputs,addEventListener:(kind,fn)=>handlers[kind]=fn})});
 assert.equal(inputs[0].value,'SOURCE-SECRET');assert.equal(inputs[1].value,'TARGET-SECRET');
 inputs[0].value='FIXED-SOURCE';handlers.input({target:inputs[0]});assert.equal(workspace.state.panes[0].password,'FIXED-SOURCE');assert.equal(workspace.state.panes[1].password,'TARGET-SECRET');
 const add=request(local('C:/local',['C:/local/a']),workspace.state.panes[1],false,'overwrite');assert.equal(add.options.password,'TARGET-SECRET');
});


test('nested workspace locations display logical ancestry without private working directories',()=>{
 const {displayArchivePath}=require('../frontend/file-workspace.js');
 const tabs=[{archive:'C:/archives/outer.zip'},{archive:'C:/Temp/qing-zip-member-a/inner.zip',nestedId:1,parentArchive:'C:/archives/outer.zip',nestedMember:'docs/inner.zip'},{archive:'C:/Temp/qing-zip-member-b/deep.7z',nestedId:2,parentArchive:'C:/Temp/qing-zip-member-a/inner.zip',nestedMember:'deep.7z'}];
 assert.equal(displayArchivePath(tabs[0].archive,tabs),'C:/archives/outer.zip');
 assert.equal(displayArchivePath(tabs[2].archive,tabs),'outer.zip / docs/inner.zip / deep.7z');
 assert.ok(!displayArchivePath(tabs[2].archive,tabs.slice(1)).includes('qing-zip-member'));
});

test('partial copied moves show committed counts and the reason sources remain',async()=>{
 let click;const errors=[];
 const workspace=create({esc:String,icon:()=>'',render(){},error:e=>errors.push(String(e)),run:async()=>({moved:2,copied:1,retained_sources:['C:/source/a'],retained_reasons:['当前磁盘不支持文件事务，保留来源']}),invoke:async(_,v)=>({path:v.path,parent:null,entries:[]})});
 for(const [i,p] of workspace.state.panes.entries())Object.assign(p,{loaded:true,path:i?'D:/target':'C:/source'});
 workspace.state.panes[0].selected.add('C:/source/a');
 workspace.mount({querySelector:()=>({addEventListener:(type,fn)=>{if(type==='click')click=fn;}})});
 click({preventDefault(){},target:{closest:()=>({dataset:{fm:'move',pane:'0'}})}});
 await new Promise(setImmediate);
 assert.equal(errors.length,1);assert.match(errors[0],/已移动 2 项/);assert.match(errors[0],/1 个源项目仍保留/);assert.match(errors[0],/不支持文件事务/);
});
