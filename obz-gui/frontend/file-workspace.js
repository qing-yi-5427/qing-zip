(function(global){'use strict';
let preserveAtime;
function preservePreference(value){
 if(typeof value==='boolean'){
  preserveAtime=value;
  try{global.localStorage.setItem('qing-add-preserve-atime',JSON.stringify(value));}catch{}
 }else if(preserveAtime===undefined){
  try{preserveAtime=JSON.parse(global.localStorage.getItem('qing-add-preserve-atime'))===true;}catch{preserveAtime=false;}
 }
 return preserveAtime;
}
function archiveMoveNotice(value){
 const retained=(value.items||[]).filter(item=>item.status==='copied_retained'||item.status==='uncertain');
 return [value.summary,...retained.slice(0,5).map(item=>`${String(item.source||'').split(/[\\/]/).pop()}：${item.reason||'来源状态需要核查，请保留目标副本。'}`),...(retained.length>5?[`另有 ${retained.length-5} 项仍保留或需要核查。`]:[])].join('\n');
}
function request(source,target,move,overwrite,preserve=false){
 if(source.pending||target.pending)throw Error('请等待位置读取完成。');
 for(const p of [source,target])if(p.kind==='archive'&&p.codepage!=null&&(!Number.isInteger(p.codepage)||p.codepage<1||p.codepage>65535))throw Error('文件名编码必须是有效的编码页编号，留空则自动识别。');
 const selected=[...source.selected];if(!selected.length)throw Error('请先选择源项目。');
 if(source.kind==='local'&&target.kind==='local'){
  if(!target.path)throw Error('请选择目标文件夹。');
  return {operation:move?'fs-move':'fs-copy',inputs:selected,dest:target.path,options:{overwrite}};
 }
 if(source.kind==='local'&&target.kind==='archive')return {operation:move?'archive-move':'archive-add',path:target.archive,dest:target.path,inputs:selected,options:{password:target.password||null,codepage:target.codepage||null,overwrite,...(preserve===true?{preserve_input_atime:true}:{})}};
 if(move)throw Error('暂不支持移动压缩包内的源项目，请使用复制。');
 if(source.kind==='archive'){
  const source_archive={path:source.archive,folder:source.path,selected,password:source.password||null,codepage:source.codepage||null};
  if(target.kind==='local'){
   if(!target.path)throw Error('请选择解压目标文件夹。');
   return {operation:'archive-export',path:source.archive,dest:target.path,source_archive,options:{overwrite}};
  }
  return {operation:'archive-copy',path:target.archive,dest:target.path,source_archive,options:{overwrite,password:target.password||null,codepage:target.codepage||null}};
 }
 throw Error('不支持的复制位置');
}
function displayArchivePath(path,tabs,seen=new Set(),nestedContext=false){
 const key=value=>String(value||'').replace(/\\/g,'/').toLowerCase(),leaf=value=>String(value||'').split(/[\\/]/).pop();
 const id=key(path),tab=tabs.find(t=>key(t.archive)===id);
 if(!tab?.nestedId)return nestedContext?leaf(path):path;
 if(seen.has(id))return leaf(path);seen.add(id);
 const parent=tabs.find(t=>key(t.archive)===key(tab.parentArchive));
 const label=parent?displayArchivePath(parent.archive,tabs,seen,true):leaf(tab.parentArchive);
 return label+' / '+(tab.nestedMember||leaf(path));
}
function archiveRows(entries,folder){
 const rows=new Map();for(const entry of entries){const path=entry.name.replace(/\\/g,'/').replace(/\/$/,'');if(!path.startsWith(folder))continue;const rest=path.slice(folder.length);if(!rest)continue;const split=rest.indexOf('/'),name=split<0?rest:rest.slice(0,split),is_dir=split>=0||entry.is_dir;
  if(!rows.has(name))rows.set(name,{name,path:split>=0?folder+name+'/':entry.name,is_dir,linked:false,size:entry.size,size_known:is_dir||entry.size_known!==false});
 }
 return [...rows.values()].sort((a,b)=>Number(b.is_dir)-Number(a.is_dir)||a.name.localeCompare(b.name,'zh-CN'));
}
// Visible tree model is independent of DOM order and retains logical parents.
function localTreeRows(p){
 const rows=[],seen=new Set();
 function visit(path,name,parent,level,pos,size){
  const stack=[{path,name,parent,level,pos,size}];
  while(stack.length){
   const node=stack.pop();if(seen.has(node.path))continue;seen.add(node.path);
   const children=(p.tree.get(node.path)||[]).filter(e=>e.is_dir&&!e.linked),known=p.tree.has(node.path),expandable=!known||children.length>0,expanded=expandable&&p.treeExpanded.has(node.path);
   rows.push({...node,expandable,expanded});
   if(expanded)for(let i=children.length-1;i>=0;i--)stack.push({path:children[i].path,name:children[i].name,parent:node.path,level:node.level+1,pos:i+1,size:children.length});
  }
 }
 visit('','此电脑',null,1,1,1);
 // Directly entered paths may have no cached ancestor chain. Keep the existing
 // current-folder branch, but never duplicate a path already visible above.
 if(p.path&&!seen.has(p.path))visit(p.path,p.path.split(/[\\/]/).filter(Boolean).pop(),null,1,2,2);
 if(rows.some(r=>r.level===1&&r.path))rows[0].size=2;
 return rows;
}
function create(ui){
 const pane=()=>({kind:'local',path:'',parent:null,entries:[],selected:new Set(),cursor:null,anchor:null,loaded:false,pending:false,tree:new Map(),treeExpanded:new Set(['']),treeCursor:'',treePending:new Map(),treeSequence:0,archive:'',all:[],offset:0,requestId:0,credentialsOpen:false,creating:null,newName:''});
 const state={panes:[pane(),pane()],active:0,overwrite:'error',busy:false,focus:null};
 function render(){
  let restore=null;
  if(!state.focus&&typeof document!=='undefined'){
   const active=document.activeElement,tree=active?.closest?.('[data-fm-tree-item]');
   if(tree)state.focus={index:Number(tree.dataset.pane),tree:true,path:tree.dataset.path};
   else if(active?.closest?.('.fm-workspace')){
    // A lazy tree response also redraws the other pane. Preserve the specific
    // control the user moved to, including unfinished path text and selection.
    const markers=['id','data-view','aria-label','data-fm','data-pane','data-index','data-path','data-fm-check','data-fm-password','data-fm-codepage'];
    const attrs=markers.filter(k=>active.hasAttribute(k)).map(k=>[k,active.getAttribute(k)]);
    const form=active.closest('[data-fm-path]'),details=active.closest('[data-fm-credentials]');
    const tag=active.tagName,value=['INPUT','TEXTAREA'].includes(tag)?active.value:undefined;
    const start=active.selectionStart,end=active.selectionEnd,direction=active.selectionDirection;
    restore=()=>{
     const candidates=[...document.querySelectorAll('.fm-workspace '+tag.toLowerCase())];
     const target=candidates.find(el=>attrs.every(([k,v])=>el.getAttribute(k)===v)&&
      (!form||el.closest('[data-fm-path]')?.dataset.fmPath===form.dataset.fmPath)&&
      (!details||el.closest('[data-fm-credentials]')?.dataset.fmCredentials===details.dataset.fmCredentials));
     if(!target||target.disabled)return;
     if(value!==undefined)target.value=value;target.focus({preventScroll:true});
     if(typeof start==='number'&&typeof end==='number')try{target.setSelectionRange(start,end,direction);}catch{}
    };
   }
  }
  ui.render();restore?.();
 }
 const button=(label,action,index,disabled=false)=>`<button type="button" class="qa-button" data-fm="${action}" data-pane="${index}" ${disabled?'disabled':''}>${label}</button>`;
 function treePanel(p,index){
  const rows=localTreeRows(p);if(!rows.some(r=>r.path===p.treeCursor))p.treeCursor=rows.find(r=>r.path===p.path)?.path??'';
  return `<nav class="fm-tree" role="tree" aria-label="${index?'右':'左'}侧目录树" data-fm-tree="${index}">${rows.map(r=>`<button role="treeitem" data-fm="tree" data-pane="${index}" data-path="${ui.esc(r.path)}" data-fm-tree-item tabindex="${r.path===p.treeCursor?0:-1}" aria-level="${r.level}" aria-posinset="${r.pos}" aria-setsize="${r.size}" aria-selected="${p.path===r.path}" ${r.expandable?`aria-expanded="${r.expanded}"`:''} ${p.treePending.has(r.path)?'aria-busy="true"':''} title="${ui.esc(r.path||'此电脑')}" style="padding-left:${8+(r.level-1)*12}px"><span aria-hidden="true" data-fm-tree-toggle>${r.expandable?(r.expanded?'▾':'▸'):'·'}</span>${ui.icon('folder')}<span>${ui.esc(r.name)}</span></button>`).join('')}</nav>`;
 }
 function treeFocus(index,path){const p=state.panes[index];p.treeCursor=path;state.active=index;state.focus={index,tree:true,path};render();}
 function focusedTree(index){return typeof document!=='undefined'&&document.activeElement?.closest?.('[data-fm-tree]')?.dataset.fmTree===String(index);}
 async function expandTree(index,path){
  const p=state.panes[index];if(p.pending||state.busy||p.treePending.has(path))return;
  if(p.tree.has(path)){p.treeExpanded.add(path);treeFocus(index,path);return;}
  const token=++p.treeSequence,location=p.requestId;p.treePending.set(path,token);treeFocus(index,path);
  try{const data=await ui.invoke('browse_local',{path});
   if(p.kind!=='local'||p.requestId!==location||p.treePending.get(path)!==token)return;
   // A canonical path can differ in spelling from the row. Keep the row key:
   // its identity in this visible tree must not jump on async completion.
   p.tree.set(path,data.entries);p.treeExpanded.add(path);
  }finally{if(p.treePending.get(path)===token){p.treePending.delete(path);if(p.kind==='local'&&p.requestId===location){if(focusedTree(index))state.focus={index,tree:true,path:p.treeCursor};render();}}}
 }
 function treeKeyboard(event,control){
  const index=Number(control.dataset.pane),p=state.panes[index],path=control.dataset.path,key=event.key;
  if(event.altKey||event.ctrlKey||event.metaKey||event.shiftKey||!['ArrowUp','ArrowDown','Home','End','ArrowLeft','ArrowRight','Enter',' '].includes(key))return;
  event.preventDefault();event.stopPropagation();if(state.busy||p.pending)return;
  const rows=localTreeRows(p),at=rows.findIndex(r=>r.path===path),row=rows[at];if(!row)return;
  if(key==='Enter'||key===' '){act('tree',index,control).catch(ui.error);return;}
  if(key==='ArrowRight'){
   if(!row.expandable)return;
   if(!row.expanded){expandTree(index,path).catch(ui.error);return;}
   const child=rows[at+1];if(child?.parent===path)treeFocus(index,child.path);return;
  }
  if(key==='ArrowLeft'){
   if(p.treePending.has(path)){p.treePending.delete(path);treeFocus(index,path);return;}
   if(row.expanded){p.treeExpanded.delete(path);treeFocus(index,path);}
   else if(row.parent!==null)treeFocus(index,row.parent);return;
  }
  const next=key==='Home'?0:key==='End'?rows.length-1:Math.max(0,Math.min(rows.length-1,at+(key==='ArrowUp'?-1:1)));
  treeFocus(index,rows[next].path);
 }
 function panel(p,index){const local=p.kind==='local',rows=p.entries,visible=rows.slice(p.offset,p.offset+200),active=state.active===index;return `<section class="fm-pane ${active?'active':''}" data-fm-pane="${index}" aria-label="${index?'右':'左'}侧位置"><header><strong>${index?'右':'左'}侧位置 · ${local?'本地文件夹':'压缩包'}</strong><div>${button('本地','local',index)}${button('当前压缩包','archive',index)}</div></header><form data-fm-path="${index}"><input aria-label="${index?'右':'左'}侧路径" value="${ui.esc(local?p.path:(p.displayArchive||p.archive))}" placeholder="输入文件夹路径" ${local?'':'readonly'} ${p.pending?'disabled':''}>${button('转到','go',index,!local||p.pending)}</form>${local?'':`<div class="fm-archive-location" aria-label="${index?'右':'左'}侧压缩包内部位置"><strong>${ui.esc(p.archive.split(/[\\/]/).pop())}</strong><span> / ${ui.esc(p.path||'根目录')}</span></div><details class="fm-credentials" data-fm-credentials="${index}" ${p.credentialsOpen?'open':''}><summary>密码与文件名编码</summary><label>此压缩包密码<input type="password" data-fm-password="${index}" aria-label="${index?'右':'左'}侧压缩包密码" autocomplete="off" ${state.busy?'disabled':''}></label><label>文件名编码<select data-fm-codepage="${index}" aria-label="${index?'右':'左'}侧文件名编码" ${state.busy||p.pending?'disabled':''}>${[['','自动'],[65001,'UTF-8'],[936,'简体中文 (GBK)'],[950,'繁体中文 (Big5)'],[932,'日文 (Shift-JIS)'],[949,'韩文'],[1252,'西欧'],...(p.codepage&&![65001,936,950,932,949,1252].includes(p.codepage)?[[p.codepage,String(p.codepage)]]:[])].map(([value,label])=>`<option value="${value}" ${String(p.codepage||'')===String(value)?'selected':''}>${label}</option>`).join('')}</select></label></details>`}<div class="fm-pane-actions">${button('上级','up',index,p.pending||(!p.path&&!local))}${button('刷新','refresh',index,p.pending)}${button('选择文件夹','pick',index)}${local?button('新建文件夹','new-directory',index,p.pending||!p.path)+button('新建文件','new-file',index,p.pending||!p.path):''}<span>${rows.length} 项 · 已选 ${p.selected.size} 项</span>${rows.length>200?button('上一页','prev',index,p.offset===0)+button('下一页','next',index,p.offset+200>=rows.length):''}</div>${local&&p.creating?`<form data-fm-create="${index}"><input data-fm-new-name="${index}" aria-label="${index?'右':'左'}侧新${p.creating==='directory'?'文件夹':'文件'}名称" value="${ui.esc(p.newName)}" placeholder="${p.creating==='directory'?'文件夹名称':'文件名称'}" required maxlength="255" ${state.busy?'disabled':''}><button class="qa-button" type="submit" ${state.busy?'disabled':''}>创建</button>${button('取消','cancel-create',index,state.busy)}</form>`:''}<div class="fm-browser">${local?treePanel(p,index):''}<div class="fm-list" role="list" aria-label="${index?'右':'左'}侧文件列表">${p.pending?'<p>正在读取…</p>':visible.map((e,j)=>{const n=j+p.offset;return `<div class="fm-row ${p.selected.has(e.path)?'selected':''}" role="listitem"><input type="checkbox" data-fm-check="${index}" data-index="${n}" tabindex="-1" aria-label="选择 ${ui.esc(e.name)}" ${p.selected.has(e.path)?'checked':''} ${e.linked?'disabled':''}><button data-fm="entry" data-pane="${index}" data-index="${n}" tabindex="${e.path===(p.cursor||rows.find(v=>!v.linked)?.path)?0:-1}" ${e.linked?'disabled':''}>${ui.icon(e.is_dir?'folder':'file')}<span>${ui.esc(e.name)}</span></button><small>${e.linked?'链接 · 不跟随':e.is_dir?'文件夹':e.size_known===false?'未知':e.size.toLocaleString()+' 字节'}</small></div>`;}).join('')||'<p>此位置为空</p>'}</div></div></section>`;}
 function createMenu(){return `<div class="fm-create-menu" popover="auto" role="menu" aria-label="新建项目">${button('新建文件夹','new-directory',state.active)}${button('新建文件','new-file',state.active)}</div>`;}
 function page(){return `<section class="fm-workspace"><div class="qa-heading"><div><h1>文件管理</h1><p>选择源项目，再将它们送到另一侧。</p></div><button class="qa-button" data-view="files">返回压缩包</button></div><div class="fm-transfer"><strong>从${state.active?'右':'左'}侧到${state.active?'左':'右'}侧</strong>${button(state.panes[state.active].kind==='archive'?(state.panes[1-state.active].kind==='archive'?'复制到压缩包目录':'解压到另一侧'):state.panes[1-state.active].kind==='archive'?'添加到当前压缩包目录':'复制到另一侧','copy',state.active,state.busy||state.panes.some(p=>p.pending))}${button('移动到另一侧','move',state.active,state.busy||(state.panes.some(p=>p.pending)||state.panes[state.active].kind!=='local'))}<label>遇到同名文件 <select id="fm-conflict">${[['error','停止并提示'],['skip','跳过'],['rename','保留两份'],['overwrite','替换']].map(([v,l])=>`<option value="${v}" ${state.overwrite===v?'selected':''}>${l}</option>`).join('')}</select></label></div>${state.panes[state.active].kind==='local'&&state.panes[1-state.active].kind==='archive'?`<details class="qa-advanced"><summary>添加选项</summary><label class="oa-switch-row"><span>保留来源访问时间</span><input id="fm-preserve-atime" class="oa-switch" role="switch" type="checkbox" ${preservePreference()?'checked':''} ${state.busy?'disabled':''}></label></details>`:''}<div class="fm-panes">${state.panes.map(panel).join('')}</div>${createMenu()}</section>`;}
 function repaintFocus(index,n,checkbox=false){
  const p=state.panes[index],entry=p.entries[n];if(!entry||entry.linked)return;
  p.cursor=entry.path;p.offset=Math.floor(n/200)*200;state.focus={index,n,checkbox};render();
 }
 function select(index,n,{shift=false,ctrl=false,toggle=false,focusOnly=false,checkbox=false,checked}={}){
  const p=state.panes[index],entry=p.entries[n];if(!entry||entry.linked)return;
  state.active=index;
  if(!focusOnly){
   const anchor=p.entries.findIndex(e=>e.path===p.anchor);
   if(shift&&anchor>=0){if(!ctrl)p.selected.clear();for(let j=Math.min(anchor,n);j<=Math.max(anchor,n);j++)if(!p.entries[j].linked)p.selected.add(p.entries[j].path);}
   else {if(!ctrl&&!toggle&&checked===undefined)p.selected.clear();const on=checked===undefined?!(toggle&&p.selected.has(entry.path)):checked;on?p.selected.add(entry.path):p.selected.delete(entry.path);p.anchor=entry.path;}
  }
  repaintFocus(index,n,checkbox);
 }
 function keyboard(event){
  const tree=event.target.closest('[data-fm-tree-item]');if(tree?.dataset?.fmTreeItem!==undefined){treeKeyboard(event,tree);return;}
  const control=event.target.closest('[data-fm="entry"],[data-fm-check]');if(!control)return;
  const index=Number(control.dataset.pane??control.dataset.fmCheck),p=state.panes[index],n=Number(control.dataset.index),key=event.key,ctrl=event.ctrlKey||event.metaKey;
  if(state.busy||p.pending)return;
  if(!['Enter',' ','ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End','PageUp','PageDown'].includes(key)&&!(ctrl&&key.toLowerCase()==='a'))return;
  if(event.altKey)return;event.preventDefault();event.stopPropagation();
  if(key==='Enter'){act('enter',index,control).catch(ui.error);return;}
  if(ctrl&&key.toLowerCase()==='a'){state.active=index;p.selected=new Set(p.entries.filter(e=>!e.linked).map(e=>e.path));if(!p.anchor)p.anchor=p.entries[n]?.path;repaintFocus(index,n);return;}
  if(key===' '){select(index,n,{shift:event.shiftKey,ctrl,toggle:true});return;}
  const available=p.entries.map((e,i)=>e.linked?-1:i).filter(i=>i>=0),position=available.indexOf(n);if(position<0)return;
  const next=key==='Home'?0:key==='End'?available.length-1:Math.max(0,Math.min(available.length-1,position+(['ArrowUp','ArrowLeft','PageUp'].includes(key)?-1:1)*(key.startsWith('Page')?200:1)));
  if(event.shiftKey&&!p.anchor)p.anchor=p.entries[n]?.path;
  select(index,available[next],{shift:event.shiftKey,ctrl,focusOnly:ctrl&&!event.shiftKey});
 }
 async function load(index,path,focus=false){const p=state.panes[index],token=++p.requestId;p.treePending.clear();p.pending=true;if(focus==='tree'){p.treeCursor=path;state.focus={index,tree:true,path};}render();try{const data=await ui.invoke('browse_local',{path});if(token!==p.requestId)return;p.offset=0;p.creating=null;p.newName='';p.kind='local';p.path=data.path;p.parent=data.parent;p.entries=data.entries;p.tree.set(data.path,data.entries);p.treeExpanded.add(data.path);if(focus==='tree')p.treeCursor=data.path;p.selected.clear();p.cursor=null;p.anchor=null;p.loaded=true;}finally{if(token===p.requestId){p.pending=false;if(focus==='tree'){if(focusedTree(index))state.focus={index,tree:true,path:p.treeCursor};render();}else if(focus){const first=p.entries.findIndex(e=>!e.linked);if(first>=0)repaintFocus(index,first);else render();}else render();}}}
 async function refresh(index){const p=state.panes[index];if(p.kind==='local')return load(index,p.path);const token=++p.requestId,archive=p.archive,folder=p.path;p.pending=true;render();try{const value=await ui.run({operation:'list',path:archive,options:{password:p.password||null,codepage:p.codepage||null}},'刷新压缩包',{silent:true});if(token!==p.requestId)return;p.all=value.entries;p.entries=archiveRows(p.all,folder);p.offset=0;p.selected.clear();await ui.updated(archive,value.entries);}finally{if(token===p.requestId){p.pending=false;render();}}}

 async function createItem(index){
  const p=state.panes[index];if(state.busy||p.pending||p.kind!=='local'||!p.path||!p.creating)return;
  const path=p.path,requestId=p.requestId,name=p.newName,directory=p.creating==='directory';state.active=index;state.busy=true;render();
  try{const value=await ui.run({operation:directory?'fs-create-directory':'fs-create-file',path,name,options:{overwrite:'error'}},directory?'新建文件夹':'新建文件',{silent:true});
   if(p.kind!=='local'||p.path!==path||p.requestId!==requestId)return;
   p.creating=null;p.newName='';const refreshId=p.requestId+1;await load(index,path);if(p.kind!=='local'||p.path!==path||p.requestId!==refreshId)return;const n=p.entries.findIndex(e=>e.path===value.path);
   if(n>=0){p.selected=new Set([p.entries[n].path]);p.cursor=p.anchor=p.entries[n].path;p.offset=Math.floor(n/200)*200;state.focus={index,n};}
  }finally{state.busy=false;render();}
 }
 async function act(action,index,element){if(state.busy)return;const p=state.panes[index];state.active=index;
  if(action==='new-file'||action==='new-directory'){if(p.kind!=='local'||!p.path||p.pending)return;p.creating=action==='new-file'?'file':'directory';p.newName='';render();if(typeof document!=='undefined')document.querySelector(`[data-fm-new-name="${index}"]`)?.focus();return;}
  if(action==='cancel-create'){if(state.busy)return;p.creating=null;p.newName='';render();return;}
  if(action==='root'||action==='local')return load(index,'');
  if(action==='go')return load(index,element.closest('section').querySelector('input[aria-label$="侧路径"]').value.trim());
  if(action==='tree')return load(index,element.dataset.path,'tree');
  if(action==='pick'){const path=await ui.invoke('pick_folder');if(path)return load(index,path);return;}
  if(action==='prev'||action==='next'){p.offset=Math.max(0,p.offset+(action==='next'?200:-200));const n=p.entries.findIndex((e,i)=>i>=p.offset&&!e.linked);if(n>=0)repaintFocus(index,n);else render();return;}
  if(action==='archive'){p.treePending.clear();p.requestId++;p.pending=false;p.offset=0;const a=ui.archive();if(!a.path)throw Error('请先在压缩包页面打开一个压缩包。');Object.assign(p,{kind:'archive',archive:a.path,displayArchive:a.displayPath||a.path,path:'',all:a.entries,password:a.password,codepage:a.codepage,loaded:true,entries:archiveRows(a.entries,'')});p.selected.clear();p.cursor=null;p.anchor=null;render();return;}
  if(action==='refresh')return refresh(index);
  if(action==='up'){if(p.kind==='local')return load(index,p.parent||'');p.path=p.path.replace(/[^/]+\/$/,'');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();render();return;}
  if(action==='entry'||action==='enter'){const n=Number(element.dataset.index),e=p.entries[n];if(!e||e.linked)return;if(action==='enter'&&e.is_dir){if(p.kind==='local')return load(index,e.path,true);p.path=e.path.replace(/\\/g,'/').replace(/\/?$/,'/');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();p.cursor=null;p.anchor=null;const first=p.entries.findIndex(e=>!e.linked);if(first>=0)repaintFocus(index,first);else render();}else select(index,n,{toggle:true});return;}

  if(action==='copy'||action==='move'){
   const target=state.panes[1-index],body=request(p,target,action==='move',state.overwrite,preservePreference());state.busy=true;render();
   let failure;try{const value=await ui.run(body,action==='move'?'移动文件':body.operation==='archive-add'?'添加到压缩包':body.operation==='archive-export'?'解压到另一侧':body.operation==='archive-copy'?'复制压缩包内容':'复制文件');if(p.kind==='archive')ui.credentials?.(p.archive,p.password||'',p.codepage||null);if(target.kind==='archive')ui.credentials?.(target.archive,target.password||'',target.codepage||null);if(body.operation==='archive-move'&&value.partial)ui.error(archiveMoveNotice(value));else if(value.retained_sources?.length)ui.error(`已移动 ${value.moved||0} 项；${value.retained_sources.length} 个源项目仍保留在原位置，目标副本已保留。${value.retained_reasons?.[0]?`\n${value.retained_reasons[0]}`:""}`);else if(value.metadata_not_merged?.length)ui.error(`${value.metadata_not_merged.length} 个文件夹的附加数据未合并，来源完整保留。\n${value.metadata_not_merged[0]}`);}catch(error){failure=error;}
   finally{const sameArchive=p.kind==='archive'&&target.kind==='archive'&&p.archive.replace(/\\/g,'/').toLowerCase()===target.archive.replace(/\\/g,'/').toLowerCase();for(const pane of [1-index,...(action==='move'||sameArchive?[index]:[])]){try{await refresh(pane);}catch(error){if(!failure)failure=error;}}state.busy=false;render();}if(failure)throw failure;return;
  }
 }
 function mount(host){const section=host.querySelector('.fm-workspace');if(!section)return;
  section.querySelectorAll?.('[data-fm-password]').forEach(input=>{input.value=state.panes[Number(input.dataset.fmPassword)].password||'';});
  section.addEventListener('toggle',e=>{const index=e.target.dataset?.fmCredentials;if(index!==undefined)state.panes[Number(index)].credentialsOpen=e.target.open;},true);
  section.addEventListener('input',e=>{if(state.busy)return;const newIndex=e.target.dataset.fmNewName;if(newIndex!==undefined){state.panes[Number(newIndex)].newName=e.target.value;return;}const index=e.target.dataset.fmPassword;if(index!==undefined)state.panes[Number(index)].password=e.target.value;const codepage=e.target.dataset.fmCodepage;if(codepage!==undefined){const value=e.target.value;state.panes[Number(codepage)].codepage=value?Number(value):null;}});
  section.addEventListener('click',e=>{const b=e.target.closest('[data-fm]');if(!b)return;e.preventDefault();if(state.busy)return;if(e.target.closest('[data-fm-tree-toggle]')?.dataset?.fmTreeToggle!==undefined){const index=Number(b.dataset.pane),p=state.panes[index],path=b.dataset.path;if(p.pending)return;if(p.treeExpanded.has(path)){p.treeExpanded.delete(path);treeFocus(index,path);}else expandTree(index,path).catch(ui.error);return;}if(b.dataset.fm==='entry'&&e.shiftKey){select(Number(b.dataset.pane),Number(b.dataset.index),{shift:true,ctrl:e.ctrlKey||e.metaKey});return;}act(b.dataset.fm,Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('contextmenu',e=>{const owner=e.target.closest('[data-fm-pane]');if(!owner||e.target.closest('input,textarea,select')||state.busy)return;const index=Number(owner.dataset.fmPane),p=state.panes[index];if(p.kind!=='local'||!p.path||p.pending)return;e.preventDefault();state.active=index;const menu=section.querySelector('.fm-create-menu');if(!menu)return;menu.querySelectorAll('[data-pane]').forEach(b=>b.dataset.pane=String(index));menu.style.left=Math.max(8,Math.min(e.clientX,window.innerWidth-190))+'px';menu.style.top=Math.max(8,Math.min(e.clientY,window.innerHeight-100))+'px';menu.showPopover();});
  section.addEventListener('dblclick',e=>{const b=e.target.closest('[data-fm="entry"]');if(!b||state.busy)return;e.preventDefault();act('enter',Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('keydown',e=>{if(e.key==='Escape'&&e.target.closest('[data-fm-create]')){e.preventDefault();act('cancel-create',Number(e.target.closest('[data-fm-create]').dataset.fmCreate)).catch(ui.error);return;}keyboard(e);});
  section.addEventListener('focusin',e=>{const b=e.target.closest('[data-fm-tree-item]');if(b)state.panes[Number(b.dataset.pane)].treeCursor=b.dataset.path;});
  section.addEventListener('submit',e=>{e.preventDefault();if(state.busy)return;const creation=e.target.closest('[data-fm-create]');if(creation){createItem(Number(creation.dataset.fmCreate)).catch(ui.error);return;}const form=e.target.closest('[data-fm-path]');if(form)act('go',Number(form.dataset.fmPath),form.querySelector('button')).catch(ui.error);});
  section.addEventListener('change',e=>{if(state.busy)return;if(e.target.id==='fm-preserve-atime'){preservePreference(e.target.checked);return;}const encoding=e.target.dataset.fmCodepage;if(encoding!==undefined){if(state.busy)return;state.panes[Number(encoding)].codepage=e.target.value?Number(e.target.value):null;refresh(Number(encoding)).catch(ui.error);return;}if(e.target.id==='fm-conflict'){state.overwrite=e.target.value;return;}const id=e.target.dataset.fmCheck;if(id===undefined)return;select(Number(id),Number(e.target.dataset.index),{checkbox:true,checked:e.target.checked});});
  if(state.focus){const {index,n,checkbox,tree,path}=state.focus;state.focus=null;const target=tree?[...(section.querySelectorAll?.('[data-fm-tree-item]')||[])].find(b=>Number(b.dataset.pane)===index&&b.dataset.path===(localTreeRows(state.panes[index]).some(r=>r.path===path)?path:state.panes[index].treeCursor)):section.querySelector?.(checkbox?`[data-fm-check="${index}"][data-index="${n}"]`:`[data-fm="entry"][data-pane="${index}"][data-index="${n}"]`);target?.focus?.({preventScroll:true});target?.scrollIntoView?.({block:'nearest',inline:'nearest'});}
  for(let index=0;index<2;index++){const p=state.panes[index];if(!p.loaded&&!p.pending)load(index,'').catch(ui.error);}
 }
 return {page,mount,state};
}
const api={preservePreference,archiveMoveNotice,create,request,archiveRows,displayArchivePath,localTreeRows};if(typeof module!=='undefined'&&module.exports)module.exports=api;global.QingFileWorkspace=api;
})(typeof window!=='undefined'?window:globalThis);
