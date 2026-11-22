(function(global){'use strict';
const {preservationNotice,preservePreference}=typeof module!=='undefined'&&module.exports?require('./input-atime.js'):global.QingInputAtime;
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
// Compare the two ordinary local directory spellings without turning C:/
// into drive-relative C:. Backend path validation remains authoritative.
function localDirectoryKey(path){const value=String(path||'').replace(/\\/g,'/').toLowerCase();return /^[a-z]:\/+$/.test(value)?value.slice(0,2)+'/':value.replace(/\/+$/,'');}
function create(ui){
 const pane=()=>({kind:'local',path:'',parent:null,entries:[],total:0,next:null,selected:new Set(),cursor:null,anchor:null,loaded:false,pending:false,tree:new Map(),treePages:new Map(),selectionNotice:'',treeExpanded:new Set(['']),treeCursor:'',treePending:new Map(),treeSequence:0,archive:'',all:[],offset:0,requestId:0,credentialsOpen:false,creating:null,newName:'',createError:'',history:[],historyIndex:-1,revision:'',error:'',refreshError:'',probePath:null,probeGeneration:0,probeFailed:false,probing:false,dirty:false,refreshing:false,timer:null,autoRefresh:true});
 const state={panes:[pane(),pane()],active:0,overwrite:'error',busy:false,focus:null,visible:false,streams:null,streamRequest:0,link:null,linkEditor:null,linkDrafts:new Map()};
 function render(){
  let restore=null;
  if(!state.focus&&typeof document!=='undefined'){
   const active=document.activeElement,tree=active?.closest?.('[data-fm-tree-item]');
   if(tree)state.focus={index:Number(tree.dataset.pane),tree:true,path:tree.dataset.path};
   else if(active?.closest?.('.fm-workspace')){
    // A lazy tree response also redraws the other pane. Preserve the specific
    // control the user moved to, including unfinished path text and selection.
    const markers=['id','data-view','aria-label','data-fm','data-pane','data-index','data-path','data-fm-check','data-fm-password','data-fm-codepage','data-entry-path','type','data-fm-auto','data-fm-history','data-fm-new-name','data-fm-stream-name','data-fm-stream-dest','data-fm-stream-check'];
    const attrs=markers.filter(k=>active.hasAttribute(k)&&!(k==='data-index'&&active.hasAttribute('data-entry-path'))).map(k=>[k,active.getAttribute(k)]);
    const form=active.closest('[data-fm-path]'),details=active.closest('[data-fm-credentials]');
    const tag=active.tagName,value=(tag==='TEXTAREA'||tag==='INPUT'&&!['checkbox','radio','button','submit'].includes(active.type))?active.value:undefined;
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
  const scroll=typeof document==='undefined'?[]:[...document.querySelectorAll('.fm-pane .fm-list,.fm-pane .fm-tree')].map(el=>({pane:el.closest('[data-fm-pane]').dataset.fmPane,kind:el.classList.contains('fm-tree')?'fm-tree':'fm-list',top:el.scrollTop,left:el.scrollLeft}));
  ui.render();for(const item of scroll){const el=document.querySelector(`[data-fm-pane="${item.pane}"] .${item.kind}`);if(el){el.scrollTop=item.top;el.scrollLeft=item.left;}}restore?.();
 }
 const button=(label,action,index,disabled=false)=>`<button type="button" class="qa-button" data-fm="${action}" data-pane="${index}" ${disabled?'disabled':''}>${label}</button>`;
 function treePanel(p,index){
  const rows=localTreeRows(p);if(!rows.some(r=>r.path===p.treeCursor))p.treeCursor=rows.find(r=>r.path===p.path)?.path??'';
  const more=rows.filter(r=>r.expanded&&p.treePages.get(r.path)?.next).map(r=>`<button type="button" data-fm="tree-more" data-pane="${index}" data-path="${ui.esc(r.path)}" aria-label="加载更多子目录：${ui.esc(r.name)}" title="${ui.esc(r.path)}" ${p.treePending.has(r.path)?'disabled':''}><span>继续加载 · ${ui.esc(r.name)}</span></button>`).join('');
  return `<div class="fm-tree"><nav role="tree" aria-label="${index?'右':'左'}侧目录树" data-fm-tree="${index}">${rows.map(r=>`<button role="treeitem" data-fm="tree" data-pane="${index}" data-path="${ui.esc(r.path)}" data-fm-tree-item tabindex="${r.path===p.treeCursor?0:-1}" aria-level="${r.level}" aria-posinset="${r.pos}" aria-setsize="${r.size}" aria-selected="${p.path===r.path}" ${r.expandable?`aria-expanded="${r.expanded}"`:''} ${p.treePending.has(r.path)?'aria-busy="true"':''} title="${ui.esc(r.path||'此电脑')}" style="padding-left:${8+(r.level-1)*12}px"><span aria-hidden="true" data-fm-tree-toggle>${r.expandable?(r.expanded?'▾':'▸'):'·'}</span>${ui.icon('folder')}<span>${ui.esc(r.name)}</span></button>`).join('')}</nav>${more?`<div role="group" aria-label="目录树分页">${more}</div>`:''}</div>`;
 }
 function treeFocus(index,path){const p=state.panes[index];p.treeCursor=path;state.active=index;state.focus={index,tree:true,path};render();}
 function focusedTree(index){return typeof document!=='undefined'&&document.activeElement?.closest?.('[data-fm-tree]')?.dataset.fmTree===String(index);}
 function recordTreePage(p,path,data,append=false){
  const before=p.treePages.get(path),directories=data.entries.filter(e=>e.is_dir&&!e.linked);
  if(!append&&before?.revision&&before.revision===data.revision&&(p.tree.get(path)?.length||0)>directories.length)return;
  p.tree.set(path,append?[...(p.tree.get(path)||[]),...directories]:directories);
  p.treePages.set(path,{next:data.next?.directory?data.next:null,revision:data.revision});
 }
 async function expandTree(index,path,more=false){
  const p=state.panes[index];if(p.pending||state.busy||p.treePending.has(path))return;
  if(!more&&p.tree.has(path)){p.treeExpanded.add(path);treeFocus(index,path);return;}
  const cursor=more?p.treePages.get(path)?.next:null;if(more&&!cursor)return;
  const token=++p.treeSequence,location=p.requestId;p.treePending.set(path,token);treeFocus(index,path);
  try{const data=await ui.invoke('browse_local',cursor?{path,cursor}:{path});
   if(p.kind!=='local'||p.requestId!==location||p.treePending.get(path)!==token)return;
   // A canonical path can differ in spelling from the row. Keep the row key:
   // its identity in this visible tree must not jump on async completion.
   recordTreePage(p,path,data,more);p.treeExpanded.add(path);
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
 function paneMenu(p,index){
  const local=p.kind==='local';
  const location=local?`<div class="fm-location-options">
   <label>历史位置 <select data-fm-history="${index}" aria-label="${index?'右':'左'}侧历史位置" ${p.pending?'disabled':''}>${p.history.map((path,i)=>`<option value="${i}" ${i===p.historyIndex?'selected':''}>${ui.esc(path||'此电脑')}</option>`).join('')}</select></label>
   <label><input type="checkbox" data-fm-auto="${index}" ${p.autoRefresh?'checked':''}> 自动刷新</label>
  </div>`:'';
  return `<button type="button" class="qa-button" id="fm-more-${index}" popovertarget="fm-more-actions-${index}">更多</button>
   <div class="fm-create-menu fm-more-actions" id="fm-more-actions-${index}" popover="auto" role="group" aria-label="${index?'右':'左'}侧更多操作">
   ${local?button('前进','forward',index,p.pending||p.historyIndex>=p.history.length-1):''}
   ${button('刷新','refresh',index,p.pending)}${button('选择文件夹','pick',index)}
   ${local?button('新建文件夹','new-directory',index,p.pending||!p.path)+button('新建文件','new-file',index,p.pending||!p.path)+button('附加数据流…','streams',index,p.pending||p.selected.size!==1)+button('创建链接…','link',index,p.pending||p.selected.size!==1):''}
   ${location}</div>`;
 }
 function panel(p,index){const local=p.kind==='local',rows=p.entries,total=local?(p.total||rows.length):rows.length,visible=rows.slice(p.offset,p.offset+200),active=state.active===index;return `<section class="fm-pane ${active?'active':''}" data-fm-pane="${index}" aria-label="${index?'右':'左'}侧位置"><header><strong>${index?'右':'左'}侧位置 · ${local?'本地文件夹':'压缩包'}</strong><div>${button('本地','local',index)}${button('当前压缩包','archive',index)}</div></header><form data-fm-path="${index}"><input aria-label="${index?'右':'左'}侧路径" value="${ui.esc(local?p.path:(p.displayArchive||p.archive))}" placeholder="输入文件夹路径" ${local?'':'readonly'} ${p.pending?'disabled':''}>${button('转到','go',index,!local||p.pending)}</form>${local?'':`<div class="fm-archive-location" aria-label="${index?'右':'左'}侧压缩包内部位置"><strong>${ui.esc(p.archive.split(/[\\/]/).pop())}</strong><span> / ${ui.esc(p.path||'根目录')}</span></div><details class="fm-credentials" data-fm-credentials="${index}" ${p.credentialsOpen?'open':''}><summary>密码与文件名编码</summary><label>此压缩包密码<input type="password" data-fm-password="${index}" aria-label="${index?'右':'左'}侧压缩包密码" autocomplete="off" ${state.busy?'disabled':''}></label><label>文件名编码<select data-fm-codepage="${index}" aria-label="${index?'右':'左'}侧文件名编码" ${state.busy||p.pending?'disabled':''}>${[['','自动'],[65001,'UTF-8'],[936,'简体中文 (GBK)'],[950,'繁体中文 (Big5)'],[932,'日文 (Shift-JIS)'],[949,'韩文'],[1252,'西欧'],...(p.codepage&&![65001,936,950,932,949,1252].includes(p.codepage)?[[p.codepage,String(p.codepage)]]:[])].map(([value,label])=>`<option value="${value}" ${String(p.codepage||'')===String(value)?'selected':''}>${label}</option>`).join('')}</select></label></details>`}<div class="fm-pane-actions">${local?button('后退','back',index,p.pending||p.historyIndex<=0):''}${button('上级','up',index,p.pending||(!p.path&&!local))}${paneMenu(p,index)}</div><div class="fm-pane-status"><span>${p.kind==='local'&&p.next?`已加载 ${rows.length} / ${total} 项`:`${total} 项`} · 已选 ${p.selected.size} 项</span>${total>200||local&&p.next?button('上一页','prev',index,p.pending||p.offset===0)+button(local&&p.next&&p.offset+200>rows.length?'继续加载':'下一页','next',index,p.pending||(!p.next&&p.offset+200>=rows.length)):''}</div>${local&&p.creating?`<form data-fm-create="${index}"><input data-fm-new-name="${index}" aria-label="${index?'右':'左'}侧新${p.creating==='directory'?'文件夹':'文件'}名称" value="${ui.esc(p.newName)}" placeholder="${p.creating==='directory'?'文件夹名称':'文件名称'}" required maxlength="255" ${state.busy?'disabled':''}><button class="qa-button" type="submit" ${state.busy?'disabled':''}>创建</button>${button('取消','cancel-create',index,state.busy)}</form>${p.createError?`<p class="qa-error fm-create-error" role="alert">${ui.esc(p.createError)}</p>`:''}`:''}${local&&p.refreshError?`<p class="fm-refresh-status" role="status">${ui.esc(p.refreshError)}</p>`:''}${p.error?`<p class="qa-error" role="alert">${ui.esc(p.error)}</p>`:''}${local&&p.selectionNotice?`<p class="fm-refresh-status" role="status">${ui.esc(p.selectionNotice)}</p>`:''}<div class="fm-browser">${local?treePanel(p,index):''}<div class="fm-list" role="list" aria-label="${index?'右':'左'}侧文件列表">${p.pending?'<p>正在读取…</p>':visible.map((e,j)=>{const n=j+p.offset;return `<div class="fm-row ${p.selected.has(e.path)?'selected':''}" role="listitem"><input type="checkbox" data-fm-check="${index}" data-index="${n}" tabindex="-1" aria-label="选择 ${ui.esc(e.name)}" ${p.selected.has(e.path)?'checked':''} ${e.linked?'disabled':''}><button data-fm="entry" data-pane="${index}" data-index="${n}" data-entry-path="${ui.esc(e.path)}" tabindex="${e.path===(p.cursor||rows.find(v=>!v.linked)?.path)?0:-1}" ${e.linked?'disabled':''}>${ui.icon(e.is_dir?'folder':'file')}<span>${ui.esc(e.name)}</span></button><small>${e.linked?'链接 · 不跟随':e.is_dir?'文件夹':e.size_known===false?'未知':e.size.toLocaleString()+' 字节'}</small>${local&&e.linked?`<button class="qa-button fm-link-inspect" data-fm="link-inspect" data-pane="${index}" data-path="${ui.esc(e.path)}" ${state.busy?'disabled':''}>查看/编辑链接</button>`:''}</div>`;}).join('')||'<p>此位置为空</p>'}</div></div></section>`;}
 function linkForm(){
  const v=state.link;if(!v)return '';
  const types=v.directory?[['symbolic-directory','目录符号链接'],['junction','目录联接点'],['wsl','WSL 链接']]:[['hard','硬链接'],['symbolic-file','文件符号链接'],['wsl','WSL 链接']];
  return `<form class="qa-advanced fm-link-form" data-fm-link><h2>创建链接</h2><p>所选项目：${ui.esc(v.target)}</p>${v.kind==='junction'?`<label class="qa-field">目标目录路径<input data-fm-link-target aria-label="目录联接点目标" value="${ui.esc(v.junctionTarget)}" required ${state.busy?'disabled':''}></label>`:v.kind==='wsl'?`<label class="qa-field">WSL 目标路径<input data-fm-link-wsl-target aria-label="WSL 目标路径" value="${ui.esc(v.wslTarget)}" placeholder="/home/user/file 或 ../relative" required ${state.busy?'disabled':''}></label>`:''}<label class="qa-field">链接类型<select data-fm-link-kind ${state.busy?'disabled':''}>${types.map(([key,label])=>`<option value="${key}" ${key===v.kind?'selected':''}>${label}</option>`).join('')}</select></label><label class="qa-field">创建位置<input data-fm-link-parent aria-label="链接创建文件夹" value="${ui.esc(v.parent)}" required ${state.busy?'disabled':''}></label>${button('选择文件夹…','link-pick',v.index,state.busy)}<label class="qa-field">新链接名称<input data-fm-link-name aria-label="新链接名称" value="${ui.esc(v.name)}" required maxlength="255" ${state.busy?'disabled':''}></label><p data-fm-link-path>新链接：${ui.esc(v.parent.replace(/[\\/]$/,'')+'\\'+v.name)}</p><p>${v.kind==='hard'?'硬链接共享同一文件内容；修改任一名称下的内容会影响另一处。':v.kind==='junction'?'目录联接点可指向尚不存在的目录；无需符号链接创建权限。':v.kind==='wsl'?'WSL 链接保存 Linux 目标路径，供 WSL 使用；无需符号链接创建权限。':'符号链接指向现有目标；创建需要当前账户已有的权限。'}</p><p>仅创建新项目；同名项目会保留并提示。</p>${v.error?`<p class="qa-error fm-link-error" role="alert">${ui.esc(v.error)}</p>`:''}<div class="qa-actions"><button class="qa-button" type="submit" ${state.busy?'disabled':''}>${state.busy?'正在创建…':'创建链接'}</button>${button('取消','link-close',v.index,state.busy)}</div></form>`;
 }
 async function createLink(){
  const v=state.link;if(!v||state.busy)return;
  if(!v.name||/[\\/]/.test(v.name)||v.name==='.'||v.name==='..'){v.error='请输入单个新链接名称。';render();return;}
  if(!v.parent.trim()){v.error='请选择创建文件夹。';render();return;}
  if(v.kind==='wsl'&&!v.wslTarget){v.error='请输入 WSL 目标路径。';render();return;}
  if(v.kind==='junction'&&!v.junctionTarget){v.error='请输入目标目录路径。';render();return;}
  const matches=p=>p.kind==='local'&&p.path.replace(/\\/g,'/').toLowerCase()===v.parent.replace(/\\/g,'/').toLowerCase();
  const destinations=state.panes.flatMap((p,index)=>matches(p)?[{index,requestId:p.requestId}]:[]);
  state.busy=true;v.error='';render();
  try{const result=await ui.run({operation:'fs-create-link',...(v.kind==='wsl'?{wsl_target:v.wslTarget}:{path:v.kind==='junction'?v.junctionTarget:v.target}),dest:v.parent,name:v.name,link_kind:v.kind,options:{overwrite:'error'}},'创建链接',{silent:true});
   if(state.link!==v)return;state.link=null;
   for(const destination of destinations){const p=state.panes[destination.index];if(matches(p)&&p.requestId===destination.requestId)await refreshLocal(destination.index);}
   ui.toast?.('已创建链接：'+result.path);
  }catch(error){if(state.link===v){const message=String(error).replace(/^Error:\s*/,'');v.error=/os error (?:80|183)\b/.test(message)?'同名项目已存在，请换一个名称。':message;}else ui.error(error);}
  finally{state.busy=false;render();}
 }
 function linkEditorForm(){
  const v=state.linkEditor;if(!v)return '';const data=v.data,editable=data?.can_edit===true;
  const label={'junction':'目录联接点','wsl':'WSL 链接','symbolic-file':'文件符号链接','symbolic-directory':'目录符号链接'}[data?.kind]||'';
  return `<form class="qa-advanced fm-link-form" data-fm-link-editor>${v.pending?'<p role="status">正在读取链接…</p>':data?`<p>${label}${data.relative?' · 相对路径':''}</p><p>当前实际目标：${ui.esc(data.target)}</p><label class="qa-field">目标路径<input data-fm-link-edit-target aria-label="已有链接实际目标" value="${ui.esc(v.target)}" ${state.busy||!editable?'disabled':''}></label>${data.print_target&&data.print_target!==data.target?`<p>显示名称：${ui.esc(data.print_target)}</p>`:''}<p>${data.kind==='wsl'?'WSL 目标按 Linux 路径文字原样保存。':data.relative?'保持相对路径；不能输入盘符或从根目录开始的路径。':'目标使用普通本地绝对路径，可指向尚不存在的位置。'}</p>${data.edit_reason?`<p>${ui.esc(data.edit_reason)}</p>`:''}${v.confirmClear?`<p>清除链接数据后，原项目会恢复为普通文件或文件夹，项目本身和原有数据都会保留。</p><div class="qa-actions">${button('确认清除链接数据','link-editor-clear-confirm',v.index,state.busy||!editable)}${button('取消清除','link-editor-clear-cancel',v.index,state.busy)}</div>`:`<div class="qa-actions"><button class="qa-button" type="submit" ${state.busy||!editable?'disabled':''}>${state.busy?'正在保存…':'保存目标'}</button>${button('清除链接数据…','link-editor-clear',v.index,state.busy||!editable)}</div>`}`:''}${v.draftNotice?`<p role="status">${ui.esc(v.draftNotice)}</p>`:''}${v.error?`<p class="qa-error fm-link-edit-error" role="alert">${ui.esc(v.error)}</p>`:''}<div class="qa-actions">${button('重新读取','link-editor-refresh',v.index,state.busy)}${button('关闭','link-editor-close',v.index,state.busy&&!v.pending)}</div></form>`;
 }
 async function readLinkEditor(v){
  if(state.busy)return;v.pending=true;v.error='';v.data=null;v.confirmClear=false;state.busy=true;render();
  try{const data=await ui.run({operation:'fs-inspect-link',path:v.path},'读取链接',{silent:true});if(state.linkEditor!==v)return;v.data=data;const draft=v.resumeDraft,same=draft&&['volume','file_id','tag'].every(key=>draft[key]===data.receipt?.[key]);v.target=same?draft.target:data.target;if(draft&&!same){state.linkDrafts.delete(v.path);v.draftNotice='链接对象已变更，未恢复旧草稿。';}delete v.resumeDraft;}
  catch(error){if(state.linkEditor===v)v.error=String(error).replace(/^Error:\s*/,'');}
  finally{v.pending=false;state.busy=false;render();}
 }
 async function saveLinkEditor(remove=false){
  const v=state.linkEditor;if(!v||state.busy||v.data?.can_edit!==true||!v.data.receipt)return;
  if(remove&&!v.confirmClear)return;if(!remove&&!v.target){v.error='目标不能为空；如需清除，请使用“清除链接数据”。';render();return;}
  const matches=p=>p.kind==='local'&&p.path.replace(/\\/g,'/').toLowerCase()===v.parent.replace(/\\/g,'/').toLowerCase();
  const destinations=state.panes.flatMap((p,index)=>matches(p)?[{index,requestId:p.requestId}]:[]);
  state.busy=true;v.error='';render();
  try{await ui.run({operation:remove?'fs-clear-link-data':'fs-edit-link',path:v.path,link_receipt:v.data.receipt,...(remove?{}:{link_target:v.target}),options:{overwrite:'error'}},remove?'清除链接数据':'修改链接目标',{silent:true});
   if(state.linkEditor!==v)return;state.linkDrafts.delete(v.path);state.linkEditor=null;
   for(const destination of destinations){const p=state.panes[destination.index];if(matches(p)&&p.requestId===destination.requestId)await refreshLocal(destination.index);}
   ui.toast?.(remove?'已清除链接数据，原项目已保留。':'已保存链接目标。');
  }catch(error){if(state.linkEditor===v)v.error=String(error).replace(/^Error:\s*/,'');else ui.error(error);}
  finally{state.busy=false;render();}
 }
 function streamsPage(){
  const v=state.streams,selected=v.entries.filter(e=>v.selected?.has(e.name)),disabled=state.busy?'disabled':'';
  const form=selected.length?`<form class="qa-advanced fm-stream-export" data-fm-stream-export id="fm-stream-export"><h2>导出所选数据流</h2><p>导出为普通文件，来源保留，同名不覆盖。取消或失败时，已完成文件保留。</p><div class="fm-stream-destination"><label for="fm-stream-dest">输出文件夹</label><div class="fm-stream-destination-controls"><input id="fm-stream-dest" data-fm-stream-dest value="${ui.esc(v.dest||'')}" ${disabled} required>${button('选择文件夹','streams-pick',v.index,state.busy)}</div></div><div class="fm-stream-mappings"><p>输出名称，可逐项修改</p>${selected.map(e=>`<label class="fm-stream-mapping"><span class="fm-stream-source">${ui.esc(e.name)}</span><span aria-hidden="true">→</span><input data-fm-stream-name="${v.entries.indexOf(e)}" aria-label="输出文件名：${ui.esc(e.name)}" value="${ui.esc(v.outputNames.get(e.name)||'')}" ${disabled} required></label>`).join('')}</div><button class="qa-button" type="submit" ${disabled}>${state.busy?'正在导出…':`导出 ${selected.length} 项`}</button>${v.exportError?`<p class="qa-error fm-stream-export-error" role="alert">${ui.esc(v.exportError)}</p>`:''}</form>`:'';
  return `<section class="fm-workspace fm-streams"><div class="qa-heading"><div><h1>附加数据流</h1><p>${ui.esc(v.path)}</p></div><div class="fm-stream-actions">${button('刷新','streams-refresh',v.index,v.pending||state.busy)}${button('返回文件管理','streams-close',v.index,state.busy)}</div></div>${v.pending?'<p role="status">正在读取数据流元数据…</p>':v.error?`<p class="qa-error" role="alert">${ui.esc(v.error)}</p>`:v.entries.length?`<div class="fm-stream-selection">${button(selected.length===v.entries.length?'取消全选':'全选','streams-select-all',v.index,state.busy||v.entries.length>4096)}${button('导出所选'+(selected.length?'（'+selected.length+'）':''),'streams-configure',v.index,state.busy||!selected.length)}</div><table class="qa-table fm-stream-table"><colgroup><col><col><col></colgroup><thead><tr><th scope="col">名称</th><th scope="col">大小（字节）</th><th scope="col">已分配（字节）</th></tr></thead><tbody>${v.entries.map((e,n)=>`<tr><td><label><input type="checkbox" data-fm-stream-check="${n}" aria-label="选择数据流 ${ui.esc(e.name)}" ${v.selected?.has(e.name)?'checked':''} ${disabled}> ${ui.esc(e.name)}</label></td><td>${ui.esc(String(e.size))}</td><td>${ui.esc(String(e.allocated_size))}</td></tr>`).join('')}</tbody></table>${form}${v.notice?`<p role="status">${ui.esc(v.notice)}</p>`:''}`:'<p role="status">没有附加数据流。</p>'}</section>`;
 }
 async function exportStreams(){
  const v=state.streams;if(!v||v.pending||state.busy)return;const selected=v.entries.filter(e=>v.selected?.has(e.name));
  if(!selected.length||selected.length>4096){v.exportError='请选择 1–4096 个附加数据流。';render();return;}
  const stream_exports=selected.map(e=>({name:e.name,size:e.size,output_name:v.outputNames.get(e.name)}));
  if(!v.dest||stream_exports.some(e=>!e.output_name||/[\\/]/.test(e.output_name)||['.','..'].includes(e.output_name))){v.exportError='请输入输出文件夹和有效的单个文件名。';render();return;}
  const names=new Set();for(const item of stream_exports){const key=item.output_name.toUpperCase();if(names.has(key)){v.exportError='多个数据流映射到同一输出文件名，请逐项修改后重试：'+item.output_name;render();return;}names.add(key);}
  const dest=v.dest,matches=p=>p.kind==='local'&&localDirectoryKey(p.path)===localDirectoryKey(dest);
  const destinations=state.panes.flatMap((p,index)=>matches(p)?[{index,requestId:p.requestId}]:[]);
  state.busy=true;v.exportError='';v.notice='';render();
  try{const result=await ui.run({operation:'fs-export-streams',path:v.path,stream_revision:v.revision,stream_exports,dest,options:{overwrite:'error'}},'导出附加数据流');if(state.streams===v)v.notice=result.summary;}
  catch(error){if(state.streams===v)v.exportError=String(error).replace(/^Error:\s*/,'');else ui.error(error);}
  finally{for(const d of destinations){const p=state.panes[d.index];if(matches(p)&&p.requestId===d.requestId)try{await refreshLocal(d.index);}catch(error){ui.error(error);}}state.busy=false;render();}
 }
 function createMenu(){return `<div class="fm-create-menu" popover="auto" role="menu" aria-label="新建项目">${button('新建文件夹','new-directory',state.active)}${button('新建文件','new-file',state.active)}</div>`;}
 function page(){if(state.linkEditor){const v=state.linkEditor;return `<section class="fm-workspace fm-link-editor-page"><div class="qa-heading"><div><h1>查看/编辑链接</h1><p>${ui.esc(v.path)}</p></div>${button('返回文件管理','link-editor-close',v.index,state.busy&&!v.pending)}</div>${linkEditorForm()}</section>`;}if(state.streams)return streamsPage();return `<section class="fm-workspace fm-browser-workspace"><div class="qa-heading"><h1>文件管理</h1><button class="qa-button" data-view="files">返回压缩包</button></div><div class="fm-transfer"><strong>从${state.active?'右':'左'}侧到${state.active?'左':'右'}侧</strong>${button(state.panes[state.active].kind==='archive'?(state.panes[1-state.active].kind==='archive'?'复制到压缩包目录':'解压到另一侧'):state.panes[1-state.active].kind==='archive'?'添加到当前压缩包目录':'复制到另一侧','copy',state.active,state.busy||state.panes.some(p=>p.pending))}${button('移动到另一侧','move',state.active,state.busy||(state.panes.some(p=>p.pending)||state.panes[state.active].kind!=='local'))}<label>遇到同名文件 <select id="fm-conflict">${[['error','停止并提示'],['skip','跳过'],['rename','保留两份'],['overwrite','替换']].map(([v,l])=>`<option value="${v}" ${state.overwrite===v?'selected':''}>${l}</option>`).join('')}</select></label></div>${state.panes[state.active].kind==='local'&&state.panes[1-state.active].kind==='archive'?`<details class="qa-advanced"><summary>添加选项</summary><label class="oa-switch-row"><span>保留来源访问时间</span><input id="fm-preserve-atime" class="oa-switch" role="switch" type="checkbox" ${preservePreference()?'checked':''} ${state.busy?'disabled':''}></label></details>`:''}${linkForm()}<div class="fm-panes">${state.panes.map(panel).join('')}</div>${createMenu()}</section>`;}
 function repaintFocus(index,n,checkbox=false){
  const p=state.panes[index],entry=p.entries[n];if(!entry||entry.linked)return;
  const oldOffset=p.offset;p.cursor=entry.path;p.offset=Math.floor(n/200)*200;state.focus={index,n,checkbox};render();if(p.offset!==oldOffset){p.probeGeneration++;clearTimeout(p.timer);p.timer=null;queueRefresh(index,0);}
 }
 function select(index,n,{shift=false,ctrl=false,toggle=false,focusOnly=false,checkbox=false,checked}={}){
  const p=state.panes[index],entry=p.entries[n];if(!entry||entry.linked)return;
  state.active=index;
  if(!focusOnly){
   p.selectionNotice='';
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
 function remember(p,path,index){
  if(Number.isInteger(index)){p.historyIndex=index;return;}
  if(p.history[p.historyIndex]===path)return;
  p.history=p.history.slice(0,p.historyIndex+1);p.history.push(path);if(p.history.length>50)p.history.shift();p.historyIndex=p.history.length-1;
 }
 function foreground(){return typeof document==='undefined'||(!document.hidden&&document.hasFocus());}
 function queueRefresh(index,delay=2000){
  const p=state.panes[index];if(delay===0&&p.timer){clearTimeout(p.timer);p.timer=null;}if(!ui.probe||!state.visible||!foreground()||!p.autoRefresh||!p.probePath||p.probeFailed||p.timer||p.pending||p.refreshing||p.probing||state.busy||p.creating)return;
  p.timer=setTimeout(()=>{p.timer=null;probe(index).catch(ui.error);},delay);
 }
 function resetProbe(index,path,force=false){
  if(!ui.probe)return;const p=state.panes[index],key=state.visible&&p.autoRefresh?path:null;if(!force&&p.probePath===key)return;
  p.probePath=key;p.probeGeneration++;p.probeFailed=false;p.refreshError='';clearTimeout(p.timer);p.timer=null;if(key)queueRefresh(index,0);
 }
 async function probe(index){
  const p=state.panes[index];if(!state.visible||!foreground()||p.kind!=='local'||!p.path||p.pending||p.probing||p.refreshing||state.busy||p.creating){queueRefresh(index);return;}
  const path=p.path,generation=p.probeGeneration,offset=p.offset,names=p.entries.slice(offset,offset+200).map(e=>e.name);p.probing=true;
  try{const stamp=await ui.probe(path,names);if(generation!==p.probeGeneration||path!==p.path||offset!==p.offset||!state.visible)return;
   if(stamp.revision!==p.revision||stamp.entries.some((entry,i)=>entry.name!==names[i]||entry.revision!==p.entries[offset+i]?.revision)){await refreshLocal(index,true);}
  }catch(error){if(generation===p.probeGeneration&&path===p.path){p.probeFailed=true;p.refreshError='自动刷新不可用，请手动刷新';await refreshLocal(index,true);render();}}
  finally{p.probing=false;queueRefresh(index,generation===p.probeGeneration?2000:0);}
 }
 function activate(){
  if(state.visible)return;state.visible=true;if(!ui.probe)return;
  for(let i=0;i<2;i++){const p=state.panes[i];if(p.loaded&&p.kind==='local'&&p.path)resetProbe(i,p.path);}
 }
 function suspend(){
  if(!state.visible)return;state.visible=false;if(!ui.probe)return;
  for(let i=0;i<2;i++){const p=state.panes[i];clearTimeout(p.timer);p.timer=null;p.dirty=false;resetProbe(i,null);}
 }
 if(typeof document!=='undefined')document.addEventListener('visibilitychange',()=>{if(!foreground())suspend();else if(ui.isActive?.())activate();});
 if(typeof window!=='undefined'){window.addEventListener('blur',suspend);window.addEventListener('focus',()=>{if(ui.isActive?.()&&foreground())activate();});}
 async function load(index,path,focus=false,historyIndex=null){
  const p=state.panes[index],token=++p.requestId;p.treePending.clear();p.pending=true;p.error='';p.selectionNotice='';resetProbe(index,null);if(focus==='tree'){p.treeCursor=path;state.focus={index,tree:true,path};}render();
  try{const data=await ui.invoke('browse_local',{path});if(token!==p.requestId)return;
   p.offset=0;p.creating=null;p.newName='';p.createError='';p.kind='local';p.path=data.path;p.parent=data.parent;p.entries=data.entries;p.total=data.total??data.entries.length;p.next=data.next??null;p.revision=data.revision||'';recordTreePage(p,data.path,data);p.treeExpanded.add(data.path);if(focus==='tree')p.treeCursor=data.path;p.selected.clear();p.cursor=null;p.anchor=null;p.loaded=true;p.dirty=false;remember(p,data.path,historyIndex);resetProbe(index,data.path);
  }catch(error){if(token===p.requestId){p.error=String(error).replace(/^Error:\s*/,'');p.loaded=true;}throw error;
  }finally{if(token===p.requestId){p.pending=false;if(focus==='tree'){if(focusedTree(index))state.focus={index,tree:true,path:p.treeCursor};render();}else if(focus){const first=p.entries.findIndex(e=>!e.linked);if(first>=0)repaintFocus(index,first);else render();}else render();}}
 }
 async function refreshLocal(index,automatic=false){
  const p=state.panes[index];if(p.kind!=='local'||p.pending||p.refreshing||(automatic&&(state.busy||p.creating))){if(automatic)p.dirty=true;return;}
  const path=p.path,token=p.requestId,loaded=p.entries.length;p.refreshing=true;p.dirty=false;
  const current=()=>p.kind==='local'&&p.path===path&&p.requestId===token;
  try{
   const data=await ui.invoke('browse_local',{path});if(!current())return;
   while(data.next&&data.entries.length<loaded){
    const page=await ui.invoke('browse_local',{path,cursor:data.next});if(!current())return;
    data.entries.push(...page.entries);data.next=page.next;data.total=page.total;
   }
   const changed=JSON.stringify(data.entries)!==JSON.stringify(p.entries)||(data.total??data.entries.length)!==p.total||!!p.error;
   p.error='';p.entries=data.entries;p.total=data.total??data.entries.length;p.next=data.next??null;p.revision=data.revision||'';p.parent=data.parent;
   recordTreePage(p,path,data);
   const present=new Set(data.entries.filter(e=>!e.linked).map(e=>e.path)),previous=p.selected.size;
   p.selected=new Set([...p.selected].filter(v=>present.has(v)));
   if(previous>p.selected.size)p.selectionNotice=`刷新后有 ${previous-p.selected.size} 个项目已不在已载入的可选列表中，选择已取消。`;
   if(!present.has(p.cursor))p.cursor=null;if(!present.has(p.anchor))p.anchor=null;
   const cursorIndex=p.entries.findIndex(e=>e.path===p.cursor);
   p.offset=cursorIndex>=0?Math.floor(cursorIndex/200)*200:Math.min(p.offset,Math.max(0,Math.floor((data.entries.length-1)/200)*200));
   if(!automatic)resetProbe(index,path,true);if(changed||previous!==p.selected.size)render();
  }catch(error){if(current()){
   p.error='无法读取此文件夹：'+String(error).replace(/^Error:\s*/,'');
   if(p.selected.size)p.selectionNotice='刷新失败，无法确认先前选中的项目，选择已取消。';
   p.entries=[];p.total=0;p.next=null;p.selected.clear();p.cursor=p.anchor=null;p.tree.delete(path);p.treePages.delete(path);resetProbe(index,null);render();
  }}
  finally{p.refreshing=false;queueRefresh(index);}
 }

 async function refresh(index){const p=state.panes[index];if(p.kind==='local')return refreshLocal(index);const token=++p.requestId,archive=p.archive,folder=p.path;p.pending=true;render();try{const value=await ui.run({operation:'list',path:archive,options:{password:p.password||null,codepage:p.codepage||null}},'刷新压缩包',{silent:true});if(token!==p.requestId)return;p.all=value.entries;p.entries=archiveRows(p.all,folder);p.offset=0;p.selected.clear();await ui.updated(archive,value.entries);}finally{if(token===p.requestId){p.pending=false;render();}}}

 async function createItem(index){
  const p=state.panes[index];if(state.busy||p.pending||p.kind!=='local'||!p.path||!p.creating)return;
  const path=p.path,requestId=p.requestId,name=p.newName,directory=p.creating==='directory';state.active=index;state.busy=true;render();
  try{const value=await ui.run({operation:directory?'fs-create-directory':'fs-create-file',path,name,options:{overwrite:'error'}},directory?'新建文件夹':'新建文件',{silent:true});
   if(p.kind!=='local'||p.path!==path||p.requestId!==requestId)return;
   p.creating=null;p.newName='';p.createError='';const refreshId=p.requestId+1;await load(index,path);if(p.kind!=='local'||p.path!==path||p.requestId!==refreshId)return;const n=p.entries.findIndex(e=>e.path===value.path);
   if(n>=0){p.selected=new Set([p.entries[n].path]);p.cursor=p.anchor=p.entries[n].path;p.offset=Math.floor(n/200)*200;state.focus={index,n};}
  }catch(error){
   if(p.kind==='local'&&p.path===path&&p.requestId===requestId){const message=String(error).replace(/^Error:\s*/,'');p.createError=/os error (?:80|183)\b/.test(message)?'同名项目已存在，请换一个名称。':message.replace(/^(?:commit private output|create private output|prepare private output):\s*/,'');}
   else ui.error(error);
  }finally{state.busy=false;render();}
 }
 async function act(action,index,element){if(action==='link-editor-close'&&(!state.busy||state.linkEditor?.pending)){const v=state.linkEditor;if(v?.data?.can_edit&&v.target!==v.data.target){const receipt=v.data.receipt;state.linkDrafts.delete(v.path);state.linkDrafts.set(v.path,{target:v.target,volume:receipt.volume,file_id:receipt.file_id,tag:receipt.tag});while(state.linkDrafts.size>16)state.linkDrafts.delete(state.linkDrafts.keys().next().value);}state.linkEditor=null;render();return;}if(state.busy)return;const p=state.panes[index];state.active=index;
  if(action==='link-inspect'){
   if(p.kind!=='local'||p.pending)return;const entry=p.entries.find(e=>e.path===element.dataset.path&&e.linked);if(!entry)return;
   state.link=null;const v={index,path:entry.path,parent:p.path,resumeDraft:state.linkDrafts.get(entry.path),data:null,target:'',pending:false,error:'',confirmClear:false};state.linkEditor=v;return readLinkEditor(v);
  }
  if(action==='link-editor-refresh'){if(state.linkEditor)return readLinkEditor(state.linkEditor);return;}
  if(action==='link-editor-clear'||action==='link-editor-clear-cancel'){const v=state.linkEditor;if(v?.data?.can_edit){v.confirmClear=action==='link-editor-clear';render();}return;}
  if(action==='link-editor-clear-confirm')return saveLinkEditor(true);
  if(action==='link-close'){state.link=null;render();return;}
  if(action==='link-pick'){const v=state.link;if(!v)return;const path=await ui.invoke('pick_folder');if(state.link===v&&path){v.parent=path;v.error='';render();}return;}
  if(action==='link'){
   if(p.kind!=='local'||p.pending||p.selected.size!==1)return;const entry=p.entries.find(e=>p.selected.has(e.path));if(!entry||entry.linked)return;
   state.linkEditor=null;const other=state.panes[1-index];state.link={index,target:entry.path,junctionTarget:entry.path,wslTarget:'',directory:entry.is_dir,parent:other.kind==='local'&&other.path?other.path:p.path,name:entry.name+'.link',kind:entry.is_dir?'symbolic-directory':'hard',error:''};render();
   if(typeof document!=='undefined')document.querySelector('[data-fm-link-name]')?.focus();return;
  }
  if(action==='streams-configure'){if(state.streams?.selected?.size&&typeof document!=='undefined'){const form=document.querySelector('[data-fm-stream-export]');form?.scrollIntoView({block:'start'});form?.querySelector('[data-fm-stream-dest]')?.focus({preventScroll:true});}return;}
  if(action==='streams-pick'){const v=state.streams;if(!v)return;const dest=await ui.invoke('pick_folder');if(state.streams===v&&dest){v.dest=dest;v.exportError='';render();}return;}
  if(action==='streams-select-all'){const v=state.streams;if(!v||v.pending||v.entries.length>4096)return;v.selected=v.selected.size===v.entries.length?new Set():new Set(v.entries.map(e=>e.name));v.exportError='';render();return;}
  if(action==='streams-close'){state.active=state.streams?.index??index;state.streamRequest++;state.streams=null;render();return;}
  if(action==='streams'||action==='streams-refresh'){
   let path,source=index;
   if(action==='streams-refresh'){
    if(!state.streams||state.streams.pending)return;
    path=state.streams.path;source=state.streams.index;
   }else{
    if(p.kind!=='local'||p.pending||p.selected.size!==1)return;
    path=[...p.selected][0];const entry=p.entries.find(e=>e.path===path);if(!entry||entry.linked)return;
   }
   const token=++state.streamRequest;state.streams={path,index:source,pending:true,error:'',entries:[]};render();
   try{const value=await ui.invoke('local_streams',{path});if(token!==state.streamRequest)return;state.streams={path,index:source,pending:false,error:'',entries:value.streams,revision:value.revision,selected:new Set(),dest:state.panes[1-source].kind==='local'&&state.panes[1-source].path?state.panes[1-source].path:state.panes[source].path,outputNames:new Map(value.streams.map(e=>[e.name,e.export_name||e.name])),exportError:'',notice:''};}
   catch(e){if(token!==state.streamRequest)return;state.streams={path,index:source,pending:false,error:String(e),entries:[]};}
   render();return;
  }

  if(action==='new-file'||action==='new-directory'){if(p.kind!=='local'||!p.path||p.pending)return;p.creating=action==='new-file'?'file':'directory';p.newName='';p.createError='';render();if(typeof document!=='undefined')document.querySelector(`[data-fm-new-name="${index}"]`)?.focus();return;}
  if(action==='cancel-create'){if(state.busy)return;p.creating=null;p.newName='';p.createError='';render();return;}
  if(action==='back'||action==='forward'||action==='history'){const at=action==='history'?Number(element.value):p.historyIndex+(action==='back'?-1:1);if(p.kind==='local'&&at>=0&&at<p.history.length)return load(index,p.history[at],false,at);return;}
  if(action==='root'||action==='local')return load(index,'');
  if(action==='go')return load(index,element.closest('section').querySelector('input[aria-label$="侧路径"]').value.trim());
  if(action==='tree')return load(index,element.dataset.path,'tree');
  if(action==='tree-more')return expandTree(index,element.dataset.path,true);
  if(action==='pick'){const path=await ui.invoke('pick_folder');if(path)return load(index,path);return;}
  if((action==='prev'||action==='next')&&p.refreshing)return;
  if(action==='next'&&p.kind==='local'&&p.next&&p.offset+200>=p.entries.length){
   const path=p.path,token=p.requestId,cursor=p.next,partial=p.offset+200>p.entries.length;p.pending=true;p.error='';render();
   try{
    const data=await ui.invoke('browse_local',{path,cursor});
    if(p.kind!=='local'||p.path!==path||p.requestId!==token)return;
    p.entries.push(...data.entries);p.total=data.total;p.next=data.next??null;recordTreePage(p,path,{...data,entries:p.entries});if(partial)return;
   }catch(error){
    if(p.requestId===token)p.error=String(error).replace(/^Error:\s*/,'');
    return;
   }finally{if(p.requestId===token){p.pending=false;render();queueRefresh(index);}}
  }
  if(action==='prev'||action==='next'){p.probeGeneration++;clearTimeout(p.timer);p.timer=null;p.offset=Math.min(Math.max(0,Math.floor((p.entries.length-1)/200)*200),Math.max(0,p.offset+(action==='next'?200:-200)));const n=p.entries.findIndex((e,i)=>i>=p.offset&&!e.linked);if(n>=0)repaintFocus(index,n);else render();clearTimeout(p.timer);p.timer=null;queueRefresh(index,0);return;}
  if(action==='archive'){resetProbe(index,null);p.treePending.clear();p.requestId++;p.pending=false;p.offset=0;const a=ui.archive();if(!a.path)throw Error('请先在压缩包页面打开一个压缩包。');Object.assign(p,{kind:'archive',archive:a.path,displayArchive:a.displayPath||a.path,path:'',all:a.entries,password:a.password,codepage:a.codepage,loaded:true,entries:archiveRows(a.entries,'')});p.selected.clear();p.cursor=null;p.anchor=null;render();return;}
  if(action==='refresh')return refresh(index);
  if(action==='up'){if(p.kind==='local')return load(index,p.parent||'');p.path=p.path.replace(/[^/]+\/$/,'');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();render();return;}
  if(action==='entry'||action==='enter'){const n=Number(element.dataset.index),e=p.entries[n];if(!e||e.linked)return;if(action==='enter'&&e.is_dir){if(p.kind==='local')return load(index,e.path,true);p.path=e.path.replace(/\\/g,'/').replace(/\/?$/,'/');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();p.cursor=null;p.anchor=null;const first=p.entries.findIndex(e=>!e.linked);if(first>=0)repaintFocus(index,first);else render();}else select(index,n,{toggle:true});return;}

  if(action==='copy'||action==='move'){
   const target=state.panes[1-index],body=request(p,target,action==='move',state.overwrite,preservePreference());state.busy=true;render();
   let failure,notice='';try{const value=await ui.run(body,action==='move'?'移动文件':body.operation==='archive-add'?'添加到压缩包':body.operation==='archive-export'?'解压到另一侧':body.operation==='archive-copy'?'复制压缩包内容':'复制文件');notice=preservationNotice(value);if(p.kind==='archive')ui.credentials?.(p.archive,p.password||'',p.codepage||null);if(target.kind==='archive')ui.credentials?.(target.archive,target.password||'',target.codepage||null);if(body.operation==='archive-move'&&value.partial)ui.error(archiveMoveNotice(value));else if(value.retained_sources?.length)ui.error(`已移动 ${value.moved||0} 项；${value.retained_sources.length} 个源项目仍保留在原位置，目标副本已保留。${value.retained_reasons?.[0]?`\n${value.retained_reasons[0]}`:""}`);else if(value.metadata_not_merged?.length)ui.error(`${value.metadata_not_merged.length} 个文件夹的附加数据未合并，来源完整保留。\n${value.metadata_not_merged[0]}`);}catch(error){failure=error;}
   finally{const sameArchive=p.kind==='archive'&&target.kind==='archive'&&p.archive.replace(/\\/g,'/').toLowerCase()===target.archive.replace(/\\/g,'/').toLowerCase();for(const pane of [1-index,...(action==='move'||sameArchive?[index]:[])]){try{await refresh(pane);}catch(error){if(!failure)failure=error;}}state.busy=false;render();}if(failure)throw failure;if(notice)ui.notice?.(notice);return;
  }
 }
 function mount(host){const section=host.querySelector('.fm-workspace');if(!section)return;if(foreground())activate();
  section.querySelectorAll?.('[data-fm-password]').forEach(input=>{input.value=state.panes[Number(input.dataset.fmPassword)].password||'';});
  section.addEventListener('beforetoggle',e=>{
   const menu=e.target;
   if(e.newState!=='open'||!menu.classList?.contains('fm-more-actions'))return;
   const trigger=section.querySelector('[popovertarget="'+menu.id+'"]');
   const bounds=trigger.getBoundingClientRect();
   menu.style.left=Math.max(8,Math.min(bounds.left,innerWidth-228))+'px';
   menu.style.top=Math.min(bounds.bottom+4,Math.max(8,innerHeight-390))+'px';
  },true);
  section.addEventListener('toggle',e=>{const index=e.target.dataset?.fmCredentials;if(index!==undefined)state.panes[Number(index)].credentialsOpen=e.target.open;},true);
  section.addEventListener('input',e=>{if(state.busy)return;if(state.streams&&(e.target.hasAttribute?.('data-fm-stream-dest')||e.target.hasAttribute?.('data-fm-stream-name'))){const v=state.streams;if(e.target.hasAttribute('data-fm-stream-dest'))v.dest=e.target.value;else{const entry=v.entries[Number(e.target.dataset.fmStreamName)];if(entry)v.outputNames.set(entry.name,e.target.value);}v.exportError='';e.target.closest?.('[data-fm-stream-export]')?.querySelector?.('.fm-stream-export-error')?.remove();return;}if(state.linkEditor&&e.target.hasAttribute?.('data-fm-link-edit-target')){state.linkEditor.target=e.target.value;state.linkEditor.error='';e.target.closest?.('[data-fm-link-editor]')?.querySelector?.('.fm-link-edit-error')?.remove();return;}if(state.link&&(e.target.hasAttribute?.('data-fm-link-target')||e.target.hasAttribute?.('data-fm-link-wsl-target'))){state.link[e.target.hasAttribute('data-fm-link-target')?'junctionTarget':'wslTarget']=e.target.value;state.link.error='';e.target.closest?.('[data-fm-link]')?.querySelector?.('.fm-link-error')?.remove();return;}if(state.link&&(e.target.hasAttribute?.('data-fm-link-name')||e.target.hasAttribute?.('data-fm-link-parent'))){const v=state.link;v[e.target.hasAttribute('data-fm-link-name')?'name':'parent']=e.target.value;v.error='';const form=e.target.closest('[data-fm-link]');const path=form?.querySelector('[data-fm-link-path]');if(path)path.textContent='新链接：'+v.parent.replace(/[\\/]$/,'')+'\\'+v.name;form?.querySelector('.fm-link-error')?.remove();return;}const newIndex=e.target.dataset.fmNewName;if(newIndex!==undefined){state.panes[Number(newIndex)].newName=e.target.value;state.panes[Number(newIndex)].createError='';e.target.closest?.('[data-fm-pane]')?.querySelector?.('.fm-create-error')?.remove();return;}const index=e.target.dataset.fmPassword;if(index!==undefined)state.panes[Number(index)].password=e.target.value;const codepage=e.target.dataset.fmCodepage;if(codepage!==undefined){const value=e.target.value;state.panes[Number(codepage)].codepage=value?Number(value):null;}});
  section.addEventListener('click',e=>{const b=e.target.closest('[data-fm]');if(!b)return;e.preventDefault();if(state.busy&&!(b.dataset.fm==='link-editor-close'&&state.linkEditor?.pending))return;if(e.target.closest('[data-fm-tree-toggle]')?.dataset?.fmTreeToggle!==undefined){const index=Number(b.dataset.pane),p=state.panes[index],path=b.dataset.path;if(p.pending)return;if(p.treeExpanded.has(path)){p.treeExpanded.delete(path);treeFocus(index,path);}else expandTree(index,path).catch(ui.error);return;}e.target.closest('.fm-more-actions:popover-open')?.hidePopover?.();if(b.dataset.fm==='entry'&&e.shiftKey){select(Number(b.dataset.pane),Number(b.dataset.index),{shift:true,ctrl:e.ctrlKey||e.metaKey});return;}act(b.dataset.fm,Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('contextmenu',e=>{const owner=e.target.closest('[data-fm-pane]');if(!owner||e.target.closest('input,textarea,select')||state.busy)return;const index=Number(owner.dataset.fmPane),p=state.panes[index];if(p.kind!=='local'||!p.path||p.pending)return;e.preventDefault();state.active=index;render();const menu=host.querySelector('.fm-create-menu:not(.fm-more-actions)');if(!menu)return;menu.querySelectorAll('[data-pane]').forEach(b=>b.dataset.pane=String(index));menu.style.left=Math.max(8,Math.min(e.clientX,window.innerWidth-190))+'px';menu.style.top=Math.max(8,Math.min(e.clientY,window.innerHeight-100))+'px';menu.showPopover();});
  section.addEventListener('dblclick',e=>{const b=e.target.closest('[data-fm="entry"]');if(!b||state.busy)return;e.preventDefault();act('enter',Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('keydown',e=>{if(e.key==='Escape'&&e.target.closest('[data-fm-link-editor]')){e.preventDefault();if(!state.busy||state.linkEditor?.pending)act('link-editor-close',state.linkEditor?.index??state.active).catch(ui.error);return;}if(e.key==='Escape'&&e.target.closest('[data-fm-link]')){e.preventDefault();if(!state.busy){state.link=null;render();}return;}if(e.key==='Escape'&&e.target.closest('[data-fm-create]')){e.preventDefault();act('cancel-create',Number(e.target.closest('[data-fm-create]').dataset.fmCreate)).catch(ui.error);return;}keyboard(e);});
  section.addEventListener('focusin',e=>{const b=e.target.closest('[data-fm-tree-item]');if(b)state.panes[Number(b.dataset.pane)].treeCursor=b.dataset.path;});
  section.addEventListener('submit',e=>{e.preventDefault();if(state.busy)return;if(e.target.closest('[data-fm-stream-export]')){exportStreams().catch(ui.error);return;}if(e.target.closest('[data-fm-link-editor]')){saveLinkEditor().catch(ui.error);return;}if(e.target.closest('[data-fm-link]')){createLink().catch(ui.error);return;}const creation=e.target.closest('[data-fm-create]');if(creation){createItem(Number(creation.dataset.fmCreate)).catch(ui.error);return;}const form=e.target.closest('[data-fm-path]');if(form)act('go',Number(form.dataset.fmPath),form.querySelector('button')).catch(ui.error);});
  section.addEventListener('change',e=>{if(state.busy)return;if(e.target.dataset?.fmStreamCheck!==undefined&&state.streams){const v=state.streams,entry=v.entries[Number(e.target.dataset.fmStreamCheck)];if(!entry)return;if(e.target.checked){if(v.selected.size>=4096){e.target.checked=false;v.exportError='每次最多导出 4096 项。';render();return;}v.selected.add(entry.name);}else v.selected.delete(entry.name);v.exportError='';render();return;}if(e.target.hasAttribute?.('data-fm-link-kind')&&state.link){state.link.kind=e.target.value;state.link.error='';render();return;}if(e.target.dataset.fmHistory!==undefined){e.target.closest('.fm-more-actions:popover-open')?.hidePopover?.();act('history',Number(e.target.dataset.fmHistory),e.target).catch(ui.error);return;}if(e.target.dataset.fmAuto!==undefined){e.target.closest('.fm-more-actions:popover-open')?.hidePopover?.();const i=Number(e.target.dataset.fmAuto),p=state.panes[i];p.autoRefresh=e.target.checked;resetProbe(i,p.path,true);render();return;}if(e.target.id==='fm-preserve-atime'){preservePreference(e.target.checked);return;}const encoding=e.target.dataset.fmCodepage;if(encoding!==undefined){if(state.busy)return;state.panes[Number(encoding)].codepage=e.target.value?Number(e.target.value):null;refresh(Number(encoding)).catch(ui.error);return;}if(e.target.id==='fm-conflict'){state.overwrite=e.target.value;return;}const id=e.target.dataset.fmCheck;if(id===undefined)return;select(Number(id),Number(e.target.dataset.index),{checkbox:true,checked:e.target.checked});});
  if(state.focus){const {index,n,checkbox,tree,path}=state.focus;state.focus=null;const target=tree?[...(section.querySelectorAll?.('[data-fm-tree-item]')||[])].find(b=>Number(b.dataset.pane)===index&&b.dataset.path===(localTreeRows(state.panes[index]).some(r=>r.path===path)?path:state.panes[index].treeCursor)):section.querySelector?.(checkbox?`[data-fm-check="${index}"][data-index="${n}"]`:`[data-fm="entry"][data-pane="${index}"][data-index="${n}"]`);target?.focus?.({preventScroll:true});target?.scrollIntoView?.({block:'nearest',inline:'nearest'});}
  for(let index=0;index<2;index++){const p=state.panes[index];if(!p.loaded&&!p.pending)load(index,'').catch(ui.error);else queueRefresh(index);}
 }
 return {page,mount,state,suspend};
}
const api={localDirectoryKey,preservationNotice,preservePreference,archiveMoveNotice,create,request,archiveRows,displayArchivePath,localTreeRows};if(typeof module!=='undefined'&&module.exports)module.exports=api;global.QingFileWorkspace=api;
})(typeof window!=='undefined'?window:globalThis);
