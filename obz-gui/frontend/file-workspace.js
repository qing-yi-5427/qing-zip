(function(global){'use strict';
function request(source,target,move,overwrite){
 if(source.pending||target.pending)throw Error('请等待位置读取完成。');
 const selected=[...source.selected];if(!selected.length)throw Error('请先选择源项目。');
 if(source.kind==='local'&&target.kind==='local'){
  if(!target.path)throw Error('请选择目标文件夹。');
  return {operation:move?'fs-move':'fs-copy',inputs:selected,dest:target.path,options:{overwrite}};
 }
 if(move)throw Error('移动仅用于两个本地文件夹之间。');
 if(source.kind==='local'&&target.kind==='archive')return {operation:'add',path:target.archive,inputs:selected,options:{password:target.password||null,codepage:target.codepage||null,overwrite}};
 if(source.kind==='archive'&&target.kind==='local'){
  if(!target.path)throw Error('请选择解压目标文件夹。');
  return {operation:'extract',path:source.archive,dest:target.path,smart:false,options:{selected,overwrite,password:source.password||null,codepage:source.codepage||null}};
 }
 throw Error('两个压缩包之间请先解压到本地，再添加到目标压缩包。');
}
function archiveRows(entries,folder){
 const rows=new Map();for(const entry of entries){const path=entry.name.replace(/\\/g,'/').replace(/\/$/,'');if(!path.startsWith(folder))continue;const rest=path.slice(folder.length);if(!rest)continue;const split=rest.indexOf('/'),name=split<0?rest:rest.slice(0,split),is_dir=split>=0||entry.is_dir;
  if(!rows.has(name))rows.set(name,{name,path:split>=0?folder+name+'/':entry.name,is_dir,linked:false,size:entry.size,size_known:is_dir||entry.size_known!==false});
 }
 return [...rows.values()].sort((a,b)=>Number(b.is_dir)-Number(a.is_dir)||a.name.localeCompare(b.name,'zh-CN'));
}
function create(ui){
 const pane=()=>({kind:'local',path:'',parent:null,entries:[],selected:new Set(),loaded:false,pending:false,tree:new Map(),archive:'',all:[],offset:0,requestId:0});
 const state={panes:[pane(),pane()],active:0,overwrite:'error',busy:false};
 const button=(label,action,index,disabled=false)=>`<button class="qa-button" data-fm="${action}" data-pane="${index}" ${disabled?'disabled':''}>${label}</button>`;
 function branch(p,path='',depth=0){if(depth>20)return '';const children=p.tree.get(path)||[];return children.filter(e=>e.is_dir&&!e.linked).map(e=>`<div role="treeitem" aria-level="${depth+1}"><button data-fm="tree" data-pane="${state.panes.indexOf(p)}" data-path="${ui.esc(e.path)}" title="${ui.esc(e.path)}" style="padding-left:${8+depth*12}px">${ui.icon('folder')}<span>${ui.esc(e.name)}</span></button>${p.tree.has(e.path)?branch(p,e.path,depth+1):''}</div>`).join('');}
 function panel(p,index){const local=p.kind==='local',rows=p.entries,visible=rows.slice(p.offset,p.offset+200),active=state.active===index;return `<section class="fm-pane ${active?'active':''}" data-fm-pane="${index}" aria-label="${index?'右':'左'}侧位置"><header><strong>${index?'右':'左'}侧位置 · ${local?'本地文件夹':'压缩包'}</strong><div>${button('本地','local',index)}${button('当前压缩包','archive',index)}</div></header><form data-fm-path="${index}"><input aria-label="${index?'右':'左'}侧路径" value="${ui.esc(local?p.path:p.archive)}" placeholder="输入文件夹路径" ${local?'':'readonly'}>${button('转到','go',index,!local)}</form><div class="fm-pane-actions">${button('上级','up',index,p.pending||(!p.path&&!local))}${button('刷新','refresh',index,p.pending)}${button('选择文件夹','pick',index)}<span>${rows.length} 项 · 已选 ${p.selected.size} 项</span>${rows.length>200?button('上一页','prev',index,p.offset===0)+button('下一页','next',index,p.offset+200>=rows.length):''}</div><div class="fm-browser">${local?`<nav class="fm-tree" role="tree" aria-label="${index?'右':'左'}侧目录树">${button('此电脑','root',index)}${branch(p)}${p.path&&!/^[A-Za-z]:[\\/]?$/.test(p.path)?`<hr><small>当前文件夹</small><button data-fm="tree" data-pane="${index}" data-path="${ui.esc(p.path)}" title="${ui.esc(p.path)}">${ui.icon('folder')}<span>${ui.esc(p.path.split(/[\\/]/).filter(Boolean).pop())}</span></button>${branch(p,p.path,1)}`:''}</nav>`:''}<div class="fm-list" role="list" aria-label="${index?'右':'左'}侧文件列表">${p.pending?'<p>正在读取…</p>':visible.map((e,j)=>{const n=j+p.offset;return `<div class="fm-row ${p.selected.has(e.path)?'selected':''}" role="listitem"><input type="checkbox" data-fm-check="${index}" data-index="${n}" aria-label="选择 ${ui.esc(e.name)}" ${p.selected.has(e.path)?'checked':''} ${e.linked?'disabled':''}><button data-fm="entry" data-pane="${index}" data-index="${n}" ${e.linked?'disabled':''}>${ui.icon(e.is_dir?'folder':'file')}<span>${ui.esc(e.name)}</span></button><small>${e.linked?'链接 · 不跟随':e.is_dir?'文件夹':e.size_known===false?'未知':e.size.toLocaleString()+' 字节'}</small></div>`;}).join('')||'<p>此位置为空</p>'}</div></div></section>`;}
 function page(){return `<section class="fm-workspace"><div class="qa-heading"><div><h1>文件管理</h1><p>选择源项目，再将它们送到另一侧。</p></div><button class="qa-button" data-view="files">返回压缩包</button></div><div class="fm-transfer"><strong>从${state.active?'右':'左'}侧到${state.active?'左':'右'}侧</strong>${button(state.panes[state.active].kind==='archive'?'解压到另一侧':state.panes[1-state.active].kind==='archive'?'添加到压缩包根目录':'复制到另一侧','copy',state.active,state.busy||state.panes.some(p=>p.pending))}${button('移动到另一侧','move',state.active,state.busy||state.panes.some(p=>p.pending||p.kind!=='local'))}<label>遇到同名文件 <select id="fm-conflict">${[['error','停止并提示'],['skip','跳过'],['rename','保留两份'],['overwrite','替换']].map(([v,l])=>`<option value="${v}" ${state.overwrite===v?'selected':''}>${l}</option>`).join('')}</select></label></div><div class="fm-panes">${state.panes.map(panel).join('')}</div></section>`;}
 async function load(index,path){const p=state.panes[index],token=++p.requestId;p.pending=true;ui.render();try{const data=await ui.invoke('browse_local',{path});if(token!==p.requestId)return;p.offset=0;p.kind='local';p.path=data.path;p.parent=data.parent;p.entries=data.entries;p.tree.set(data.path,data.entries);p.selected.clear();p.loaded=true;}finally{if(token===p.requestId){p.pending=false;ui.render();}}}
 async function refresh(index){const p=state.panes[index];if(p.kind==='local')return load(index,p.path);const token=++p.requestId,archive=p.archive,folder=p.path;p.pending=true;ui.render();try{const value=await ui.run({operation:'list',path:archive,options:{password:p.password||null,codepage:p.codepage||null}},'刷新压缩包',{silent:true});if(token!==p.requestId)return;p.all=value.entries;p.entries=archiveRows(p.all,folder);p.offset=0;p.selected.clear();ui.updated(archive,value.entries);}finally{if(token===p.requestId){p.pending=false;ui.render();}}}

 async function act(action,index,element){const p=state.panes[index];state.active=index;
  if(action==='root'||action==='local')return load(index,'');
  if(action==='go')return load(index,element.closest('section').querySelector('input[aria-label$="侧路径"]').value.trim());
  if(action==='tree')return load(index,element.dataset.path);
  if(action==='pick'){const path=await ui.invoke('pick_folder');if(path)return load(index,path);return;}
  if(action==='prev'||action==='next'){p.offset=Math.max(0,p.offset+(action==='next'?200:-200));ui.render();return;}
  if(action==='archive'){p.requestId++;p.pending=false;p.offset=0;const a=ui.archive();if(!a.path)throw Error('请先在压缩包页面打开一个压缩包。');Object.assign(p,{kind:'archive',archive:a.path,path:'',all:a.entries,password:a.password,codepage:a.codepage,loaded:true,entries:archiveRows(a.entries,'')});p.selected.clear();ui.render();return;}
  if(action==='refresh')return refresh(index);
  if(action==='up'){if(p.kind==='local')return load(index,p.parent||'');p.path=p.path.replace(/[^/]+\/$/,'');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();ui.render();return;}
  if(action==='entry'||action==='enter'){const e=p.entries[Number(element.dataset.index)];if(!e||e.linked)return;if(action==='enter'&&e.is_dir){if(p.kind==='local')return load(index,e.path);p.path=e.path.replace(/\\/g,'/').replace(/\/?$/,'/');p.entries=archiveRows(p.all,p.path);p.offset=0;p.selected.clear();}else{p.selected.has(e.path)?p.selected.delete(e.path):p.selected.add(e.path);}ui.render();return;}

  if(action==='copy'||action==='move'){
   const target=state.panes[1-index],body=request(p,target,action==='move',state.overwrite);state.busy=true;ui.render();
   let failure;try{const value=await ui.run(body,action==='move'?'移动文件':body.operation==='add'?'添加到压缩包':body.operation==='extract'?'解压到另一侧':'复制文件');if(value.retained_sources?.length)ui.error(`已完成复制；${value.retained_sources.length} 个源项目无法安全移动，仍保留在原位置。`);}catch(error){failure=error;}
   finally{const refreshed=await Promise.allSettled([refresh(1-index),...(action==='move'?[refresh(index)]:[])]);state.busy=false;ui.render();if(!failure)failure=refreshed.find(r=>r.status==='rejected')?.reason;}if(failure)throw failure;return;
  }
 }
 function mount(host){const section=host.querySelector('.fm-workspace');if(!section)return;
  section.addEventListener('click',e=>{const b=e.target.closest('[data-fm]');if(!b)return;e.preventDefault();if(state.busy)return;act(b.dataset.fm,Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('dblclick',e=>{const b=e.target.closest('[data-fm="entry"]');if(!b||state.busy)return;e.preventDefault();act('enter',Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('keydown',e=>{if(e.key!=='Enter')return;const b=e.target.closest('[data-fm="entry"]');if(!b||state.busy)return;e.preventDefault();e.stopPropagation();act('enter',Number(b.dataset.pane),b).catch(ui.error);});
  section.addEventListener('submit',e=>{e.preventDefault();const form=e.target.closest('[data-fm-path]');if(form)act('go',Number(form.dataset.fmPath),form.querySelector('button')).catch(ui.error);});
  section.addEventListener('change',e=>{if(e.target.id==='fm-conflict'){state.overwrite=e.target.value;return;}const id=e.target.dataset.fmCheck;if(id===undefined)return;const p=state.panes[Number(id)],entry=p.entries[Number(e.target.dataset.index)];state.active=Number(id);e.target.checked?p.selected.add(entry.path):p.selected.delete(entry.path);ui.render();});
  for(let index=0;index<2;index++){const p=state.panes[index];if(!p.loaded&&!p.pending)load(index,'').catch(ui.error);}
 }
 return {page,mount,state};
}
const api={create,request,archiveRows};if(typeof module!=='undefined'&&module.exports)module.exports=api;global.QingFileWorkspace=api;
})(typeof window!=='undefined'?window:globalThis);
