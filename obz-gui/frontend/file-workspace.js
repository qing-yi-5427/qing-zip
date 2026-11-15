(function(global){'use strict';
function request(source,target,move,overwrite){
 if(source.pending||target.pending)throw Error('请等待位置读取完成。');
 for(const p of [source,target])if(p.kind==='archive'&&p.codepage!=null&&(!Number.isInteger(p.codepage)||p.codepage<1||p.codepage>65535))throw Error('文件名编码必须是有效的编码页编号，留空则自动识别。');
 const selected=[...source.selected];if(!selected.length)throw Error('请先选择源项目。');
 if(source.kind==='local'&&target.kind==='local'){
  if(!target.path)throw Error('请选择目标文件夹。');
  return {operation:move?'fs-move':'fs-copy',inputs:selected,dest:target.path,options:{overwrite}};
 }
 if(move)throw Error('移动仅用于两个本地文件夹之间。');
 if(source.kind==='local'&&target.kind==='archive')return {operation:'archive-add',path:target.archive,dest:target.path,inputs:selected,options:{password:target.password||null,codepage:target.codepage||null,overwrite}};
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
function create(ui){
 const pane=()=>({kind:'local',path:'',parent:null,entries:[],selected:new Set(),cursor:null,anchor:null,loaded:false,pending:false,tree:new Map(),archive:'',all:[],offset:0,requestId:0,credentialsOpen:false});
 const state={panes:[pane(),pane()],active:0,overwrite:'error',busy:false,focus:null};
 const button=(label,action,index,disabled=false)=>`<button class="qa-button" data-fm="${action}" data-pane="${index}" ${disabled?'disabled':''}>${label}</button>`;
 function branch(p,path='',depth=0){if(depth>20)return '';const children=p.tree.get(path)||[];return children.filter(e=>e.is_dir&&!e.linked).map(e=>`<div role="treeitem" aria-level="${depth+1}"><button data-fm="tree" data-pane="${state.panes.indexOf(p)}" data-path="${ui.esc(e.path)}" title="${ui.esc(e.path)}" style="padding-left:${8+depth*12}px">${ui.icon('folder')}<span>${ui.esc(e.name)}</span></button>${p.tree.has(e.path)?branch(p,e.path,depth+1):''}</div>`).join('');}
 function panel(p,index){const local=p.kind==='local',rows=p.entries,visible=rows.slice(p.offset,p.offset+200),active=state.active===index;return `<section class="fm-pane ${active?'active':''}" data-fm-pane="${index}" aria-label="${index?'右':'左'}侧位置"><header><strong>${index?'右':'左'}侧位置 · ${local?'本地文件夹':'压缩包'}</strong><div>${button('本地','local',index)}${button('当前压缩包','archive',index)}</div></header><form data-fm-path="${index}"><input aria-label="${index?'右':'左'}侧路径" value="${ui.esc(local?p.path:(p.displayArchive||p.archive))}" placeholder="输入文件夹路径" ${local?'':'readonly'} ${p.pending?'disabled':''}>${button('转到','go',index,!local||p.pending)}</form>${local?'':`<div class="fm-archive-location" aria-label="${index?'右':'左'}侧压缩包内部位置"><strong>${ui.esc(p.archive.split(/[\\/]/).pop())}</strong><span> / ${ui.esc(p.path||'根目录')}</span></div><details class="fm-credentials" data-fm-credentials="${index}" ${p.credentialsOpen?'open':''}><summary>密码与文件名编码</summary><label>此压缩包密码<input type="password" data-fm-password="${index}" aria-label="${index?'右':'左'}侧压缩包密码" autocomplete="off" ${state.busy?'disabled':''}></label><label>文件名编码<select data-fm-codepage="${index}" aria-label="${index?'右':'左'}侧文件名编码" ${state.busy||p.pending?'disabled':''}>${[['','自动'],[65001,'UTF-8'],[936,'简体中文 (GBK)'],[950,'繁体中文 (Big5)'],[932,'日文 (Shift-JIS)'],[949,'韩文'],[1252,'西欧'],...(p.codepage&&![65001,936,950,932,949,1252].includes(p.codepage)?[[p.codepage,String(p.codepage)]]:[])].map(([value,label])=>`<option value="${value}" ${String(p.codepage||'')===String(value)?'selected':''}>${label}</option>`).join('')}</select></label></details>`}<div class="fm-pane-actions">${button('上级','up',index,p.pending||(!p.path&&!local))}${button('刷新','refresh',index,p.pending)}${button('选择文件夹','pick',index)}<span>${rows.length} 项 · 已选 ${p.selected.size} 项</span>${rows.length>200?button('上一页','prev',index,p.offset===0)+button('下一页','next',index,p.offset+200>=rows.length):''}</div><div class="fm-browser">${local?`<nav class="fm-tree" role="tree" aria-label="${index?'右':'左'}侧目录树">${button('此电脑','root',index)}${branch(p)}${p.path&&!/^[A-Za-z]:[\\/]?$/.test(p.path)?`<hr><small>当前文件夹</small><button data-fm="tree" data-pane="${index}" data-path="${ui.esc(p.path)}" title="${ui.esc(p.path)}">${ui.icon('folder')}<span>${ui.esc(p.path.split(/[\\/]/).filter(Boolean).pop())}</span></button>${branch(p,p.path,1)}`:''}</nav>`:''}<div class="fm-list" role="list" aria-label="${index?'右':'左'}侧文件列表">${p.pending?'<p>正在读取…</p>':visible.map((e,j)=>{const n=j+p.offset;return `<div class="fm-row ${p.selected.has(e.path)?'selected':''}" role="listitem"><input type="checkbox" data-fm-check="${index}" data-index="${n}" tabindex="-1" aria-label="选择 ${ui.esc(e.name)}" ${p.selected.has(e.path)?'checked':''} ${e.linked?'disabled':''}><button data-fm="entry" data-pane="${index}" data-index="${n}" tabindex="${e.path===(p.cursor||rows.find(v=>!v.linked)?.path)?0:-1}" ${e.linked?'disabled':''}>${ui.icon(e.is_dir?'folder':'file')}<span>${ui.esc(e.name)}</span></button><small>${e.linked?'链接 · 不跟随':e.is_dir?'文件夹':e.size_known===false?'未知':e.size.toLocaleString()+' 字节'}</small></div>`;}).join('')||'<p>此位置为空</p>'}</div></div></section>`;}
 function page(){return `<section class="fm-workspace"><div class="qa-heading"><div><h1>文件管理</h1><p>选择源项目，再将它们送到另一侧。</p></div><button class="qa-button" data-view="files">返回压缩包</button></div><div class="fm-transfer"><strong>从${state.active?'右':'左'}侧到${state.active?'左':'右'}侧</strong>${button(state.panes[state.active].kind==='archive'?(state.panes[1-state.active].kind==='archive'?'复制到压缩包目录':'解压到另一侧'):state.panes[1-state.active].kind==='archive'?'添加到当前压缩包目录':'复制到另一侧','copy',state.active,state.busy||state.panes.some(p=>p.pending))}${button('移动到另一侧','move',state.active,state.busy||state.panes.some(p=>p.pending||p.kind!=='local'))}<label>遇到同名文件 <select id="fm-conflict">${[['error','停止并提示'],['skip','跳过'],['rename','保留两份'],['overwrite','替换']].map(([v,l])=>`<option value="${v}" ${state.overwrite===v?'selected':''}>${l}</option>`).join('')}</select></label></div><div class="fm-panes">${state.panes.map(panel).join('')}</div></section>`;}
 function repaintFocus(index,n,checkbox=false){
  const p=state.panes[index],entry=p.entries[n];if(!entry||entry.linked)return;
  p.cursor=entry.path;p.offset=Math.floor(n/200)*200;state.focus={index,n,checkbox};ui.render();
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
 async function load(index,path,focus=false){const p=state.panes[index],token=++p.requestId;p.pending=true;ui.render();try{const data=await ui.invoke('browse_local',{path});if(token!==p.requestId)return;p.offset=0;p.kind='local';p.path=data.path;p.parent=data.parent;p.entries=data.entries;p.tree.set(data.path,data.entries);p.selected.clear();p.cursor=null;p.anchor=null;p.loaded=true;}finally{if(token===p.requestId){p.pending=false;if(focus){const first=p.entries.findIndex(e=>!e.linked);if(first>=0)repaintFocus(index,first);else ui.render();}else ui.render();}}}
 async function refresh(index){const p=state.panes[index];if(p.kind==='local')return load(index,p.path);const token=++p.requestId,archive=p.archive,folder=p.path;p.pending=true;ui.render();try{const value=await ui.run({operation:'list',path:archive,options:{password:p.password||null,codepage:p.codepage||null}},'刷新压缩包',{silent:true});if(token!==p.requestId)return;p.all=value.entries;p.entries=archiveRows(p.all,folder);p.offset=0;p.selected.clear();await ui.updated(archive,value.entries);}finally{if(token===p.requestId){p.pending=false;ui.render();}}}

 async function act(action,index,element){const p=state.panes[index];state.active=index;
  if(action==='root'||action==='local')return load(index,'');
  if(action==='go')return load(index,element.closest('section').querySelector('input[aria-label$="侧路径"]').value.trim());
  if(action==='tree')return load(index,element.dataset.path);
  if(action==='pick'){const path=await ui.invoke('pick_folder');if(path)return load(index,path);return;}
  if(action==='prev'||action==='next'){p.offset=Math.max(0,p.offset+(action==='next'?200:-200));const n=p.entries.findIndex((e,i)=>i>=p.offset&&!e.linked);if(n>=0)repaintFocus(index,n);else ui.render();return;}
  if(action==='archive'){p.requestId++;p.pending=false;p.offset=0;const a=ui.archive();if(!a.path)throw Error('请先在压缩包页面打开一个压缩包。');Object.assign(p,{kind:'archive',archive:a.path,displayArchive:a.displayPath||a.path,path:'',all:a.entries,password:a.password,codepage:a.codepage,loaded:true,entries:archiveRows(a.entries,'')});p.selected.clear();p.cursor=null;p.anchor=null;ui.render();return;}
  if(action==='refresh')return refresh(index);
  if(action==='up'){if(p.kind==='local')return load(index,p.parent||'');p.path=p.path.replace(/[^/]+\/$/,'');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();ui.render();return;}
  if(action==='entry'||action==='enter'){const n=Number(element.dataset.index),e=p.entries[n];if(!e||e.linked)return;if(action==='enter'&&e.is_dir){if(p.kind==='local')return load(index,e.path,true);p.path=e.path.replace(/\\/g,'/').replace(/\/?$/,'/');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();p.cursor=null;p.anchor=null;const first=p.entries.findIndex(e=>!e.linked);if(first>=0)repaintFocus(index,first);else ui.render();}else select(index,n,{toggle:true});return;}

  if(action==='copy'||action==='move'){
   const target=state.panes[1-index],body=request(p,target,action==='move',state.overwrite);state.busy=true;ui.render();
   let failure;try{const value=await ui.run(body,action==='move'?'移动文件':body.operation==='archive-add'?'添加到压缩包':body.operation==='archive-export'?'解压到另一侧':body.operation==='archive-copy'?'复制压缩包内容':'复制文件');if(p.kind==='archive')ui.credentials?.(p.archive,p.password||'',p.codepage||null);if(target.kind==='archive')ui.credentials?.(target.archive,target.password||'',target.codepage||null);if(value.retained_sources?.length)ui.error(`已完成复制；${value.retained_sources.length} 个源项目无法安全移动，仍保留在原位置。`);}catch(error){failure=error;}
   finally{const sameArchive=p.kind==='archive'&&target.kind==='archive'&&p.archive.replace(/\\/g,'/').toLowerCase()===target.archive.replace(/\\/g,'/').toLowerCase();for(const pane of [1-index,...(action==='move'||sameArchive?[index]:[])]){try{await refresh(pane);}catch(error){if(!failure)failure=error;}}state.busy=false;ui.render();}if(failure)throw failure;return;
  }
 }
 function mount(host){const section=host.querySelector('.fm-workspace');if(!section)return;
  section.querySelectorAll?.('[data-fm-password]').forEach(input=>{input.value=state.panes[Number(input.dataset.fmPassword)].password||'';});
  section.addEventListener('toggle',e=>{const index=e.target.dataset?.fmCredentials;if(index!==undefined)state.panes[Number(index)].credentialsOpen=e.target.open;},true);
  section.addEventListener('input',e=>{if(state.busy)return;const index=e.target.dataset.fmPassword;if(index!==undefined)state.panes[Number(index)].password=e.target.value;const codepage=e.target.dataset.fmCodepage;if(codepage!==undefined){const value=e.target.value;state.panes[Number(codepage)].codepage=value?Number(value):null;}});
  section.addEventListener('click',e=>{const b=e.target.closest('[data-fm]');if(!b)return;e.preventDefault();if(state.busy)return;if(b.dataset.fm==='entry'&&e.shiftKey){select(Number(b.dataset.pane),Number(b.dataset.index),{shift:true,ctrl:e.ctrlKey||e.metaKey});return;}act(b.dataset.fm,Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('dblclick',e=>{const b=e.target.closest('[data-fm="entry"]');if(!b||state.busy)return;e.preventDefault();act('enter',Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('keydown',keyboard);
  section.addEventListener('submit',e=>{e.preventDefault();const form=e.target.closest('[data-fm-path]');if(form)act('go',Number(form.dataset.fmPath),form.querySelector('button')).catch(ui.error);});
  section.addEventListener('change',e=>{const encoding=e.target.dataset.fmCodepage;if(encoding!==undefined){if(state.busy)return;state.panes[Number(encoding)].codepage=e.target.value?Number(e.target.value):null;refresh(Number(encoding)).catch(ui.error);return;}if(e.target.id==='fm-conflict'){state.overwrite=e.target.value;return;}const id=e.target.dataset.fmCheck;if(id===undefined)return;select(Number(id),Number(e.target.dataset.index),{checkbox:true,checked:e.target.checked});});
  if(state.focus){const {index,n,checkbox}=state.focus;state.focus=null;const target=section.querySelector?.(checkbox?`[data-fm-check="${index}"][data-index="${n}"]`:`[data-fm="entry"][data-pane="${index}"][data-index="${n}"]`);target?.focus?.({preventScroll:true});target?.scrollIntoView?.({block:'nearest',inline:'nearest'});}
  for(let index=0;index<2;index++){const p=state.panes[index];if(!p.loaded&&!p.pending)load(index,'').catch(ui.error);}
 }
 return {page,mount,state};
}
const api={create,request,archiveRows,displayArchivePath};if(typeof module!=='undefined'&&module.exports)module.exports=api;global.QingFileWorkspace=api;
})(typeof window!=='undefined'?window:globalThis);
