(()=>{'use strict';
const root=document.getElementById('qing-archive'),main=document.getElementById('qa-main'),overlay=document.getElementById('qa-overlay');
const invoke=window.__TAURI__.core.invoke,$=id=>document.getElementById(id),esc=v=>String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const icon=n=>{const paths={archive:'<rect x="4" y="3" width="16" height="18" rx="3"/><path d="M11 3v3h2v3h-2v3h2v3h-2v3h2"/>','folder-open':'<path d="M3 7V4h6l3 3h9v13H3zM3 10h18"/>','folder-output':'<path d="M3 7V4h6l3 3h9v13H3zM12 10v7m-3-3 3 3 3-3"/>',folder:'<path d="M3 7V4h6l3 3h9v13H3z"/>',plus:'<path d="M12 4v16M4 12h16"/>',x:'<path d="m6 6 12 12M18 6 6 18"/>',check:'<path d="m3 6 2 2 3-4M11 6h10M3 13l2 2 3-4M11 13h10M4 20h17"/>',shield:'<path d="m12 3 8 3v5c0 5-4 8-8 10-4-2-8-5-8-10V6zM8 12l3 3 5-6"/>',file:'<path d="M5 3h9l5 5v13H5zM14 3v6h5M8 13h8M8 17h6"/>',globe:'<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18M5 7h14M5 17h14"/>',home:'<path d="m3 11 9-8 9 8M5 10v11h14V10M10 21v-7h4v7"/>','arrow-up':'<path d="m5 11 7-7 7 7M12 4v16"/>',sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1 1M18 18l1 1M5 19l1-1M18 6l1-1"/>',moon:'<path d="M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11z"/>',grid:'<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',settings:'<path d="M3 6h18M3 12h18M3 18h18"/><circle cx="8" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>',more:'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',chevron:'<path d="m9 5 7 7-7 7"/>',search:'<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>'};return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[n]||paths.file}</svg>`;};

const btn=(label,action,cls='',ico='')=>`<button type="button" class="qa-button ${cls}" data-action="${action}" ${label?'':'aria-label="上级目录"'}>${ico?icon(ico):''}${label}</button>`;
const opts=(values,value)=>values.map(([v,l])=>`<option value="${v}" ${String(v)===String(value)?'selected':''}>${l}</option>`).join('');
const formats=[['zip','ZIP'],['7z','7z'],['zipx','ZIPX · XZ'],['tar','TAR'],['tgz','TAR.GZ'],['tbz2','TAR.BZ2'],['txz','TAR.XZ'],['gzip','GZip'],['bzip2','BZip2'],['xz','XZ'],['zstd','Zstandard（单文件）'],['brotli','Brotli（单文件）'],['lz4','LZ4（单文件）'],['lz5','LZ5（单文件）'],['lizard','Lizard（单文件）'],['wim','WIM'],['sfx','EXE 自解压'],['iso','ISO'],['lzh','LZH']];
const load=(key,fallback)=>{try{return JSON.parse(localStorage.getItem(key))??fallback}catch{return fallback}};
let appVersion='',folderMenuEnabled=false,fileMenuEnabled=false;const pendingFolders=[],pendingFileGroups=[];
const pref=load('qing-preferences',{}),s={page:'files',view:'home',collapsed:new Set(),theme:QingAppearance.preferredMode(pref.theme),material:pref.material||'acrylic',density:pref.density||'relaxed',overwrite:pref.overwrite||'error',sameFolder:pref.sameFolder!==false,recycleAfter:!!pref.recycleAfter,openAfter:!!pref.openAfter,closeAfter:!!pref.closeAfter,archive:'',entries:[],folder:'',flatView:false,search:'',sort:'name',reverse:false,selected:new Set(),jobs:[],active:null,password:'',advancedTab:'archive-tools',recent:load('qing-recent',[]),profiles:load('qing-profiles',[]),batchStop:false};
const deepExtract=window.QingDeepExtract({root,s,pref,esc,btn,dialog,$,run,invoke,appearance,toast,archiveParent,archiveStem,estimatedArchiveFolder,
 extractionSummary:options=>extractionSummary(options),sourceDisposition:(recycle,options)=>sourceDisposition(recycle,options),
 onRecycled:paths=>forgetRecycledArchives(paths),onBrowse:paths=>{close();enqueueArchives(paths);}});
let columns=ArchiveColumns.normalize(load('qing-columns',null)),columnDraft=null;
let toastTimer,returnFocus,menuReturnFocus,menuAnchorRect;
s.listStyle=pref.listStyle==='icons'?'icons':'details';s.iconSize=Math.max(80,Math.min(256,Number(pref.iconSize)||144));
const archiveTabs=[];let currentTabId=null,nextTabId=1,openingArchive=false,openQueueRunning=false,drainingNativeQueue=false;const pendingArchivePaths=[];const pendingShellExtractions=[];let shellQueueRunning=false,shellQueuePaused=false,shellQueueInterrupted=false,shellActiveRequest=null,shellPasswordRequest=null;
const filename=p=>p.split(/[\\/]/).pop(),size=n=>n<1024?`${n} B`:n<1048576?`${(n/1024).toFixed(1)} KB`:`${(n/1048576).toFixed(1)} MB`;
const entrySize=e=>e.size_known===false?'未知':size(e.size);
function displayModified(value){const text=String(value||'');if(!/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d{1,7})?Z$/.test(text))return text;const date=new Date(text.replace(' ','T').replace(/(\.\d{3})\d+(Z)$/,'$1$2'));if(!Number.isFinite(date.getTime()))return text;const pad=n=>String(n).padStart(2,'0');return String(date.getFullYear()).padStart(4,'0')+'-'+pad(date.getMonth()+1)+'-'+pad(date.getDate())+' '+pad(date.getHours())+':'+pad(date.getMinutes())+':'+pad(date.getSeconds());}
function entryType(entry) {
  const dot = entry.leaf.lastIndexOf('.');
  return dot < 0 ? '' : entry.leaf.slice(dot + 1).toUpperCase();
}
function entryTime(entry, key) {
  return entry[key+'Time'] ??= Date.parse((entry[key] || '').replace(' ', 'T'));
}
const flatNameOrder = new Intl.Collator(undefined, {numeric:true, sensitivity:'base'});
function compareEntries(a, b) {
  const directoriesFirst = Number(!!b.is_dir) - Number(!!a.is_dir);
  if (directoriesFirst) return directoriesFirst;
  let order = 0;
  if (s.sort === 'size' || s.sort === 'packed_size') {
    if (s.sort === 'packed_size') { const unknownLast=Number(!ArchiveColumns.packedKnown(a))-Number(!ArchiveColumns.packedKnown(b)); if(unknownLast)return unknownLast; }
    if (s.sort === 'size' && a.size_known !== b.size_known) {
      const unknownLast = Number(a.size_known === false) - Number(b.size_known === false);
      if (unknownLast) return unknownLast;
    }
    order = (a[s.sort] || 0) - (b[s.sort] || 0);
  } else if (s.sort === 'type') {
    if (!a.is_dir) order = entryType(a).localeCompare(entryType(b));
  } else if (['modified','created','accessed'].includes(s.sort)) {
    const left = entryTime(a,s.sort), right = entryTime(b,s.sort);
    const unknownLast = Number(!Number.isFinite(left)) - Number(!Number.isFinite(right));
    if (unknownLast) return unknownLast;
    order = left - right;
  } else if (s.sort === 'encrypted') { order=Number(a.encrypted)-Number(b.encrypted);
  } else if (['crc','method','attributes','comment'].includes(s.sort)) { const left=a[s.sort]||'',right=b[s.sort]||''; if(!left!==!right)return Number(!left)-Number(!right); order=left.localeCompare(right);
  }
  if (!order) order = s.flatView
    ? flatNameOrder.compare(a.leaf,b.leaf) || flatNameOrder.compare(a.name,b.name) || a.name.localeCompare(b.name)
    : a.leaf.localeCompare(b.leaf);
  return s.reverse ? -order : order;
}
function sortableColumn(key, label) {
  const direction = s.sort !== key ? 'none' : s.reverse ? 'descending' : 'ascending';
  return `<th data-column="${key}" style="width:${columns.find(c=>c.id===key)?.width||100}px" aria-sort="${direction}"><button data-sort="${key}">${label}</button></th>`;
}
function showColumns(){columnDraft=ArchiveColumns.normalize(columns);dialog('列表列','<div id="oa-column-list"></div>',btn('恢复默认','columns-reset')+btn('取消','close')+btn('应用','columns-apply','primary'));paintColumns();}
function paintColumns(){const box=$('oa-column-list');box.innerHTML=columnDraft.map((c,i)=>`<div class="oa-column-setting"><label><input type="checkbox" data-column-visible="${c.id}" ${c.visible?'checked':''} ${c.id==='name'?'disabled':''}>${c.label}</label><label>宽度 <input type="number" min="64" max="800" step="8" aria-label="${c.label}宽度" data-column-width="${c.id}" value="${c.width}"></label><button type="button" class="qa-button quiet" data-column-move="${c.id}" data-delta="-1" aria-label="${c.label}前移" ${i===0?'disabled':''}>↑</button><button type="button" class="qa-button quiet" data-column-move="${c.id}" data-delta="1" aria-label="${c.label}后移" ${i===columnDraft.length-1?'disabled':''}>↓</button></div>`).join('');}
function toast(message){$('qa-toast').textContent=message;$('qa-toast').hidden=false;clearTimeout(toastTimer);toastTimer=setTimeout(()=>$('qa-toast').hidden=true,4500)}
function syncToolTheme(frame){
  const doc=frame?.contentDocument;
  if(doc)doc.documentElement.style.colorScheme=document.documentElement.style.colorScheme;
}
root.addEventListener('load',event=>{
  if(event.target.id==='qa-tools')syncToolTheme(event.target);
},true);
const updateNativeAppearance=QingAppearance.bind(
  ()=>({theme:s.theme,material:s.material}),
  (theme,material)=>{
    document.documentElement.style.colorScheme=theme;
    root.dataset.material=material;
    syncToolTheme($('qa-tools'));
  },
  toast,
);
function appearance(){
  root.style.setProperty('--qa-row',s.density==='compact'?'35px':'43px');
  root.querySelectorAll('[data-theme]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.theme===s.theme)));
  if($('qa-theme'))$('qa-theme').value=s.theme;
  localStorage.setItem('qing-preferences',JSON.stringify({theme:s.theme,material:s.material,density:s.density,overwrite:s.overwrite,sameFolder:s.sameFolder,recycleAfter:s.recycleAfter,openAfter:s.openAfter,closeAfter:s.closeAfter,deepExtract:s.deepExtract,deepRecycleAfter:s.deepRecycleAfter,deepLimits:s.deepLimits,listStyle:s.listStyle,iconSize:s.iconSize}));
  updateNativeAppearance();
}
function home(){return `<section class="oa-home"><h1>打开或解压压缩包</h1><div class="oa-home-targets">
 <button type="button" id="oa-home-smart-drop" class="oa-home-smart-drop" data-action="home-smart">${icon('folder-open')}<span><strong>拖入文件或点击选择</strong><small data-drop-label>自动识别压缩包和多层压缩包</small></span></button>
 </div></section>${s.recent.length?`<section class="oa-recent"><div class="oa-section-head"><h3>最近打开</h3>${btn('全部记录','recent-menu','quiet')}</div>${s.recent.slice(0,4).map((path,i)=>`<button class="qa-recent-row" data-recent="${i}"><span class="oa-file-symbol">${icon('archive')}</span><span class="qa-file-copy"><strong>${esc(filename(path))}</strong><small>${esc(path)}</small></span>${icon('chevron')}</button>`).join('')}</section>`:''}`;}

// Refreshing an archive replaces its entry array. Keep just the current
// folder's projection per live listing, so selection changes can reuse it.
const folderViews = new WeakMap();
const folderIndexes = new WeakMap();
function folderEntry(name, metadata) {
  return {...metadata, name, leaf:name.slice(0,-1).split('/').pop(),
    is_dir:true, size:0, size_known:true};
}
// Archive formats may omit parent directory records. Include each implied parent once.
function flatChildren(entries, folder) {
  const items = new Map();
  for (const entry of entries) {
    const path = entry.name.replaceAll('\\','/').replace(/^\.\//,'').replace(/\/+$/,'');
    if (!path.startsWith(folder) || path.length <= folder.length) continue;
    const leaf = path.slice(path.lastIndexOf('/')+1);
    if (entry.is_dir) items.set(path+'/', folderEntry(path+'/', entry));
    else items.set(path, {...entry, name:path, leaf, size_known:entry.size_known !== false});
    let end = path.lastIndexOf('/');
    while (end > 0 && end >= folder.length) {
      const parent = path.slice(0,end+1);
      if (items.has(parent)) break;
      items.set(parent, folderEntry(parent));
      end = path.lastIndexOf('/',end-1);
    }
  }
  return [...items.values()];
}
function indexFolderChildren(entries) {
  const folders = new Map();
  for (const entry of entries) {
    const path = entry.name.replaceAll('\\','/').replace(/^\.\//,'').split('/').filter(Boolean).join('/');
    if (!path) continue;
    let end = path.length;
    let directory = entry.is_dir;
    let explicit = true;
    while (end > 0) {
      const separator = path.lastIndexOf('/', end - 1);
      const parent = path.slice(0, separator + 1);
      const leaf = path.slice(separator + 1, end);
      const name = path.slice(0, end) + (directory ? '/' : '');
      let children = folders.get(parent);
      if (!children) folders.set(parent, children = new Map());
      if (children.has(name) && !(directory && explicit)) break;
      children.set(name, directory ? folderEntry(name, explicit ? entry : undefined)
        : {...entry, name, leaf, is_dir:false, size_known:entry.size_known !== false});
      end = separator;
      directory = true;
      explicit = false;
    }
  }
  return folders;
}
function folderChildren(entries, folder) {
  let index = folderIndexes.get(entries);
  if (!index) {
    index = indexFolderChildren(entries);
    folderIndexes.set(entries, index);
  }
  return [...(index.get(folder)?.values() || [])];
}
function searchEntries(){
 const folder=s.searchScope==='archive'?'':s.folder;
 let indexed=archiveSearchViews.get(s.entries);
 if(!indexed||indexed.folder!==folder){
  indexed={folder,entries:flatChildren(s.entries,folder)};
  archiveSearchViews.set(s.entries,indexed);
 }
 return indexed.entries;
}
const archiveSearchViews=new WeakMap();
function visibleEntries() {
  let view = folderViews.get(s.entries);
  if (!view || view.folder !== s.folder || view.flatView !== !!s.flatView) {
    view = {folder:s.folder, flatView:!!s.flatView, children:s.flatView ? flatChildren(s.entries,s.folder) : folderChildren(s.entries,s.folder)};
    folderViews.set(s.entries,view);
  }
  const search = s.search.toLowerCase();
  if (!view.rows || view.search !== search || view.searchScope!==s.searchScope || view.sort !== s.sort || view.reverse !== s.reverse) {
    const source=search?searchEntries():view.children;
    view.rows = source.filter(entry => !search || (s.searchScope==='archive'?entry.name:entry.name.slice(s.folder.length)).toLowerCase().includes(search)).sort(compareEntries);
    Object.assign(view, {search,searchScope:s.searchScope,sort:s.sort,reverse:s.reverse});
  }
  return view.rows;
}
function applyArchiveFilter(){
 const visible=positionsFor(visibleEntries());
 s.selected=new Set([...s.selected].filter(name=>visible.has(name)));
 if(!visible.has(s.anchor))s.anchor=null;
 rows();
}
function updateArchiveSearch(input){
 if(s.search===input.value)return;
 s.search=input.value;
 applyArchiveFilter();
}
function breadcrumbs(){
 let path='';
 return [`<button data-breadcrumb="" title="${esc(s.archive)}">${esc(filename(s.archive))}</button>`,
  ...s.folder.split('/').filter(Boolean).map(part=>{
   path+=part+'/';
   return `<button data-breadcrumb="${esc(path)}" title="${esc(path)}">${esc(part)}</button>`;
  })].join('<span aria-hidden="true">/</span>');
}
function gotoArchiveFolder(path){
 s.folder=path;s.search='';s.selected.clear();s.anchor=null;s.cursor=null;
 s.page='files';s.view='archive';render();main.scrollTop=0;
}
function archive(){if(!s.archive)return home();return `${nestedBanner()}<div class="qa-path">${btn('','up','quiet','arrow-up')}<nav class="oa-breadcrumb" aria-label="包内路径">${breadcrumbs()}</nav><label class="oa-search">${icon('search')}<input id="qa-search" aria-label="查找文件" placeholder="查找文件  Ctrl+F" value="${esc(s.search)}"></label><select id="oa-search-scope" aria-label="查找范围">${opts([['folder','当前目录'],['archive','全包']],s.searchScope||'folder')}</select></div>${viewControls()}<div class="oa-table-wrap" style="--oa-icon-size:${s.iconSize}px"><table style="${s.listStyle==='icons'?'':'min-width:'+(36+columns.filter(c=>c.visible).reduce((n,c)=>n+c.width,0))+'px'}" role="grid" aria-label="压缩包文件" aria-multiselectable="true" class="qa-table ${s.listStyle==='icons'?'oa-icons':''}"><thead><tr aria-rowindex="1"><th><input id="qa-all" type="checkbox" aria-label="全选当前列表"></th>${columns.filter(c=>c.visible).map(c=>sortableColumn(c.id,c.label)).join('')}</tr></thead><tbody id="qa-rows"></tbody></table><div id="qa-selection" class="qa-selection" role="status" aria-live="polite"></div></div><div id="oa-selection-actions" class="oa-selection-actions" hidden><span id="oa-selected-count"></span><div class="qa-actions">${btn('解压选中','extract-selected','quiet','folder-output')}${btn('重命名','rename','quiet')}${btn('删除','delete','quiet','x')}</div></div>`;}

const windowThreshold = 256;
function fileRow(e,index){const label=s.search?e.name.replace(/\/$/,''):s.flatView?e.name.slice(s.folder.length).replace(/\/$/,''):e.leaf;
const nameCell=`<td data-column="name"><button class="qa-filebtn" tabindex="${e.name===s.cursor?0:-1}" data-entry="${esc(e.name)}" title="${esc(label)}"><span class="oa-file-symbol ${e.is_dir?'folder':''}" ${!e.is_dir&&isImage(e.name)&&s.listStyle==='icons'?`data-thumbnail="${esc(e.name)}" aria-label="图片缩略图"`:''}>${icon(e.is_dir?'folder':'file')}</span><span>${esc(label)}</span></button></td>`;
const cell=id=>{if(id==='name')return nameCell;let value='';if(id==='size')value=e.is_dir?'':entrySize(e);else if(id==='packed_size')value=e.is_dir?'':ArchiveColumns.packedKnown(e)?size(e.packed_size||0):'未提供';else if(id==='type')value=e.is_dir?'文件夹':entryType(e)?entryType(e)+' 文件':'文件';else if(['modified','created','accessed'].includes(id))value=displayModified(e[id])||'未提供';else if(id==='encrypted')value=e.encrypted?'是':'未标记';else value=e[id]||'未提供';if(id==='type'&&e.encrypted)value+=' 🔒';return `<td data-column="${id}" title="${esc(value)}">${esc(value)}</td>`;};
return `<tr aria-rowindex="${index+2}" data-row="${esc(e.name)}" aria-selected="${s.selected.has(e.name)}" class="${s.selected.has(e.name)?'selected':''}"><td><input type="checkbox" tabindex="-1" data-select="${esc(e.name)}" aria-label="选择 ${esc(label)}" ${s.selected.has(e.name)?'checked':''}></td>${s.listStyle==='icons'?nameCell:columns.filter(c=>c.visible).map(c=>cell(c.id)).join('')}</tr>`;}

function releaseListingWindow() { listingWindow?.stop(); listingWindow = null; }
function rows() {
  const body = $('qa-rows'), table = body.closest('table'), list = visibleEntries();
  const restore = document.activeElement?.closest('[data-row]')?.dataset.row;
  const positions = positionsFor(list);
  if (!positions.has(s.cursor)) s.cursor = list[0]?.name;
  const windowed = list.length > windowThreshold;
  table.classList.toggle('oa-windowed', windowed);
  table.setAttribute('aria-rowcount', String(list.length + 1));
  if (listingWindow && (!windowed || listingWindow.body !== body || listingWindow.list !== list || !!listingWindow.iconGrid !== (s.listStyle === 'icons'))) releaseListingWindow();
  if (windowed && s.listStyle === 'icons') {
    if (!listingWindow) {
      observeThumbnails(false);
      listingWindow = new ArchiveIconWindow(body, main, list, fileRow, () => positions.get(s.cursor) ?? -1, watchTile, unwatchTile);
    } else listingWindow.paint();
  } else if (windowed) {
    if (!listingWindow) listingWindow = new ArchiveRowWindow(body, main, list, fileRow, () => positions.get(s.cursor) ?? -1, columns.filter(c=>c.visible).length+1, JSON.stringify(columns));
    else listingWindow.paint();
  } else {
    body.innerHTML = list.map(fileRow).join('') || `<tr><td colspan="${columns.filter(c=>c.visible).length+1}" class="qa-empty">${s.search ? '没有匹配的文件' : s.entries.length ? '此文件夹为空' : '此压缩包为空'}</td></tr>`;
  }
  refreshSelection();
  if (s.listStyle !== 'icons') stopThumbnailObserver(); else if (!windowed) observeThumbnails();
  if (restore && positions.has(restore)) focusEntry(restore);
}

const ordinaryRetries=new Map();
const phaseLabels={preflight:'检查输入',listing:'读取目录',validating:'检查文件和目标',
 decoding:'验证并解码',extracting:'解压文件','staging-files':'写入已验证文件','publishing-files':'保存已验证文件',
 'verifying-files':'验证归档文件','deep-inspect':'识别内层压缩包','deep-publish':'保存深度解压结果',recycling:'正在移入回收站',
 compressing:'正在压缩',testing:'正在检查完整性',complete:'正在确认结果'};
function jobStatus(j){
 if(j.cancelRequested&&j.status==='running')return '正在取消';
 if(j.deep?.cancelled||j.status==='cancelled')return '已取消';
 if(j.deep&&!j.deep.finished&&!j.deep.session_closed)return '待继续';
 if(j.partial)return '部分完成';
 return {running:'处理中',complete:'已完成',failed:'失败'}[j.status];
}
function updateJobProgress(j,v){
 const {phase,percent,message,context,work}=v.progress;
 const measured=j.background&&['recycling','complete'].includes(phase)?null:percent;
 Object.assign(j,{status:v.status,phase,percent:measured,message,context,work,error:v.error});
 const key=JSON.stringify([phase,context,work?.items_total,work?.bytes_total]);
 const now=performance.now(), bytes=work?.bytes_done;
 if(!j.rate||j.rate.key!==key||bytes<j.rate.bytes)j.rate={key,bytes,time:now};
 else if(bytes>j.rate.bytes&&now-j.rate.time>=750){
  j.rate={key,bytes,time:now,bps:(bytes-j.rate.bytes)*1000/(now-j.rate.time)};
 }
}
function jobMeasurements(j){
 const work=j.work;
 if(!work)return '';
 const parts=[`${work.items_done.toLocaleString()} / ${work.items_total.toLocaleString()} 个文件`];
 if(work.bytes_total!=null)parts.push(`${size(work.bytes_done)} / ${size(work.bytes_total)}`);
 const bps=j.rate?.bps,remaining=work.bytes_total-work.bytes_done;
 if(bps>0&&remaining>0&&performance.now()-j.rate.time<2500){
  const seconds=Math.ceil(remaining/bps);
  parts.push(size(bps)+'/秒',seconds>=60?`本阶段约 ${Math.ceil(seconds/60)} 分钟`:`本阶段约 ${seconds} 秒`);
 }
 return parts.join(' · ');
}
function elapsedTime(j){
 const seconds=Math.max(0,Math.floor(((j.finishedAt??performance.now())-j.startedAt)/1000));
 const minutes=Math.floor(seconds/60);
 return minutes?`已用 ${minutes} 分 ${seconds%60} 秒`:`已用 ${seconds} 秒`;
}
function compressionStageLabel(job){
 return `${job.stage==='verify'?'正在检查完整性':'正在创建压缩文件'} · ${job.current+1}/${job.archives.length}`;
}
const compressionResultPageSize=12;
function compressionResults(job,inline){
 const states={pending:'未处理',creating:'正在创建',created:'已创建',unverified:'已创建，未完成检查',
  verifying:'正在检查完整性',verified:'检查通过',createFailed:'创建失败',verifyFailed:'已创建，完整性检查失败',createCancelled:'创建已取消'};
 const running=job.status==='running',page=job.resultPage||0,start=page*compressionResultPageSize;
 const visible=running?[[job.archives[job.current],job.current]]:job.archives.slice(start,start+compressionResultPageSize).map((archive,index)=>[archive,start+index]);
 const nav=(label,target,disabled)=>`<button class="qa-button quiet" data-compression-page="${job.id}" data-result-page="${target}" ${disabled?'disabled':''}>${label}</button>`;
 return `<ul class="oa-compression-results">${visible.map(([archive,index])=>`<li>
  ${inline?`<div class="oa-compression-result-row">
   <div class="oa-compression-result-file">
    <strong title="${esc(archive.outputs.join('\n')||archive.path)}">${esc(filename(archive.outputs[0]||archive.path))}</strong>
    ${archive.outputs.length>1?`<small>${archive.outputs.length} 卷</small>`:''}
    <span class="qa-sub">${states[archive.state]}</span>
   </div>
   ${!running&&archive.outputs.length?compressionOutputActions(job,archive,index,true):''}
  </div>`:`<div class="qa-sub"><strong title="${esc(archive.path)}">${esc(filename(archive.path))}</strong> · ${states[archive.state]}</div>`}
  ${archive.error?`<div class="qa-jobresult">${esc(taskError(archive.error))}</div>`:''}
  ${archive.warning?`<p class="qa-sub">${esc(archive.warning)}</p>`:''}
  ${!inline&&!running&&archive.outputs.length?compressionOutputActions(job,archive,index):''}
 </li>`).join('')}</ul>
 ${!running&&job.archives.length>compressionResultPageSize?`<div class="qa-actions">${nav('上一页',page-1,page===0)}<span class="qa-sub">${start+1}–${Math.min(start+compressionResultPageSize,job.archives.length)} / ${job.archives.length}</span>${nav('下一页',page+1,start+compressionResultPageSize>=job.archives.length)}</div>`:''}
 ${compressionDrafts.has(job)&&!s.active?`<button class="qa-button primary" data-compression-draft="${job.id}">修改设置并重试剩余来源</button>`:''}`;
}
function compressionOutputActions(job,archive,index,inline=false){
 const button=(label,action,disabled=false)=>`<button class="qa-button ${action==='reveal'?'quiet':''}" data-compression-output="${job.id}" data-archive-index="${index}" data-output-action="${action}" ${disabled?'disabled':''}>${label}</button>`;
 return `<div class="oa-compression-output"${inline?'':` title="${esc(archive.outputs.join('\n'))}"`}>
  ${inline?'':`<span class="qa-file-copy">${esc(filename(archive.outputs[0]))}${archive.outputs.length>1?` · ${archive.outputs.length} 卷`:''}</span>`}
  ${button('打开压缩包','open',!!s.active)}${button('打开所在文件夹','reveal')}
  ${['unverified','verifyFailed'].includes(archive.state)?button('重新检查完整性','recheck',!!s.active):''}
 </div>`;
}
function jobCard(j,inline=false){
 const compact=inline===true&&!!j.archives;
 const running=j.status==='running',message=j.error?taskError(j.error):j.summary||j.message||(j.work?'':'正在处理…');
 const phase=j.archives?compressionStageLabel(j):phaseLabels[j.phase]||'正在处理';
 const display=running?({'Verifying all archive entries':'正在验证全部归档条目',
  'Decoding archive into temporary storage':'正在解码到临时位置',
  'Reading archive directory':'正在读取压缩包目录',
  'Verifying archive and staging selected files':'正在验证压缩包并暂存选中内容',
  'Verifying complete archive in bounded memory':'正在验证压缩包内容',
  'Verifying complete solid archive in bounded memory':'正在验证固实压缩包',
  'Creating archive':'','Checking archive integrity':'','Archive created':'正在确认已创建的文件','Archive integrity verified':'正在确认检查结果',
  'Extraction complete':'正在确认解压结果'})[message]??message:message;
 return `<div class="qa-jobcard${compact?' oa-compression-inline':''}">
  ${compact?'':`<div class="qa-jobhead"><strong>${esc(j.label)}</strong><span class="qa-badge">${esc(jobStatus(j))}</span></div>`}
  ${j.path?`<p class="qa-sub">${esc(j.path)}</p>`:''}
  ${running?`<p class="qa-sub">${compact?elapsedTime(j):esc(phase)+(j.startedAt==null?'':' · '+elapsedTime(j))}</p><progress aria-label="${esc(phase)}" max="100" ${j.percent==null?'':`value="${j.percent}"`}></progress>`:''}
  ${running&&j.archives?.length>1?`<p class="qa-sub">已处理 ${j.current} 个 · 后续待处理 ${j.archives.length-j.current-1} 个</p>`:''}
  ${running&&j.context?`<div class="qa-jobresult">${esc(j.context)}</div>`:''}
  ${running&&j.work?`<p class="qa-sub">${esc(jobMeasurements(j))}</p>`:''}
  ${display?`<div class="qa-jobresult">${j.work?.items_done>0&&running?'最近完成 · ':''}${esc(display)}</div>`:''}
  ${j.archives?compressionResults(j,compact):''}
  ${j.timeWarning?`<details class="qa-sub"><summary>${esc(j.timeWarning)}</summary>${j.timeReasons.map(reason=>`<p>${esc(reason)}</p>`).join('')}</details>`:''}
  ${running?(j.phase==='recycling'?'<span class="qa-sub">请等待回收完成</span>':j.cancelRequested?'<span class="qa-sub">正在取消…</span>':btn('取消任务','canceljob','quiet')):j.outputPath&&!j.deep?`<button class="qa-button" data-job-output="${j.id}" data-action="reveal-output">打开输出位置</button>`:''}
  ${ordinaryRetries.has(j.id)?btn(j.retryLabel,'extract-retry-'+j.id,'primary'):''}
  ${deepExtract.jobActions(j)}
 </div>`;
}
function jobs(){return shellQueueStatus()+(s.jobs.length?jobCard(s.jobs.at(-1)):'<div class="qa-empty">还没有任务。</div>');}
function renderInlineTask(){
 const host=$('oa-task-panel'),job=s.jobs.findLast(j=>j.id===s.inlineTask);
 if(!host)return;
 host.hidden=!job;
 if(!job)return;
 const label=job.path?job.label+' · '+filename(job.path):job.label;
 const text=job.status==='running'&&!job.cancelRequested?(job.archives?compressionStageLabel(job):phaseLabels[job.phase]||'正在准备'):jobStatus(job);
 const headline=host.querySelector('[data-action=task-toggle]');
 const summary=label+' · '+text+(job.status==='running'&&job.percent!=null&&!job.cancelRequested?` · ${job.percent}%`:'');
 const output=$('oa-task-output');
 output.hidden=job.status==='running'||!job.outputPath;
 output.dataset.jobOutput=String(job.id);
 output.textContent=job.deep?.outputs?.length>1?'打开首个结果':'打开输出位置';
 if(headline.textContent!==summary)headline.textContent=summary;
 if((!s.active||s.active===job.id||job===s.compressionJob)&&$('qa-status-text').textContent!==text)$('qa-status-text').textContent=text;
 headline.setAttribute('aria-expanded',String(s.inlineExpanded));
 $('oa-task-content').hidden=!s.inlineExpanded;
 if(!s.inlineExpanded)return;
 const content=$('oa-task-content'),html=shellQueueStatus()+jobCard(job,true);
 if(content.renderedHtml===html)return;
 const active=document.activeElement;
 const focus=content.contains(active)?Object.entries(active.dataset).map(([key,value])=>[key,value]):null;
 content.innerHTML=html;
 content.renderedHtml=html;
 if(focus){
  const next=[...content.querySelectorAll('button')].find(button=>focus.every(([key,value])=>button.dataset[key]===value));
  (next||content.querySelector('[data-job-output],[data-deep-action=manage]')||headline).focus({preventScroll:true});
 }
}
const tabs=[['batch','批量处理'],['archive-tools','归档工具'],['security','安全与校验'],['automation','压缩预设']];
function advanced(){let body='';if(s.advancedTab==='batch')body=`<p class="qa-sub">逐个处理压缩包，每个压缩包解压到独立文件夹。失败后继续其余任务。</p><label class="qa-field">压缩包路径（每行一个）<textarea id="qa-batch-inputs"></textarea></label>${btn('选择压缩包','batch-pick')}<label class="qa-field">操作<select id="qa-batch-op"><option value="test">检查完整性</option><option value="extract">批量解压</option></select></label><label class="qa-field">解压目标文件夹<div class="qa-inlinefield"><input id="qa-batch-dest">${btn('浏览…','batch-dest')}</div></label><div class="qa-actions">${btn('开始批量处理','batch-start','primary')}${btn('停止队列','batch-stop')}</div>`;else if(s.advancedTab==='automation')body=`<p class="qa-sub">保存常用压缩参数，创建时一键载入。不会保存密码。</p><label class="qa-field">预设名称<input id="qa-profile-name" maxlength="80"></label><div class="qa-fieldrow"><label class="qa-field">格式<select id="qa-profile-format">${opts(formats,'zip')}</select></label><label class="qa-field">压缩级别<select id="qa-profile-level">${opts([[0,'仅存储'],[1,'快速'],[5,'标准'],[9,'最大']],5)}</select></label></div>${btn('保存预设','profile-save','primary')}<div class="qa-recent">${s.profiles.map((p,i)=>`<div class="qa-recent-row"><span class="qa-file-copy">${esc(p.name)} · ${esc(p.format)} · ${p.level}</span><button class="qa-button" data-profile="${i}">使用</button><button class="qa-button" data-profile-delete="${i}">删除</button></div>`).join('')}</div>`;else{const list=s.advancedTab==='security'?[['test','完整性检查'],['checksum','文件校验值'],['recover','密码恢复'],['vault-list','密码库']]:[['list','文件列表'],['repair','修复 ZIP'],['preview','图片预览'],['create','多格式创建'],['add','添加'],['delete','删除'],['rename','重命名'],['capabilities','支持能力']];body=`<div class="qa-tool-buttons">${list.map(([op,label])=>`<button class="qa-button" data-tool="${op}">${label}</button>`).join('')}</div><iframe id="qa-tools" title="归档工具操作" class="qa-tool-frame" src="tools.html?operation=${s.advancedTab==='security'?'test':'list'}&path=${encodeURIComponent(s.archive)}&codepage=${s.codepage||''}"></iframe>`;}return `<div class="oa-page-back">${btn('工具箱','tools-home','quiet','arrow-up')}</div><div class="qa-heading"><h1>${s.advancedTab==='batch'?'批量处理':s.advancedTab==='automation'?'压缩预设':'归档工具'}</h1></div><div class="qa-advanced-tabs" role="tablist">${tabs.map(([id,label])=>`<button role="tab" data-advanced="${id}" aria-selected="${id===s.advancedTab}">${label}</button>`).join('')}</div><section>${body}</section>`;}
function settings(){return `<div class="qa-heading"><h1>设置</h1></div><section class="qa-setting-group oa-app-info"><img src="app-icon.png" alt="" width="44" height="44"><div><strong>qing-zip</strong><p id="oa-version">版本 ${esc(appVersion||'…')}</p></div></section><section class="qa-setting-group"><h3>外观</h3><div class="qa-setting-row"><div>应用主题</div><select id="qa-theme" aria-label="应用主题">${opts([['system','跟随系统'],['light','浅色'],['dark','深色']],s.theme)}</select></div><div class="qa-setting-row"><div>窗口材质<p>Mica 适用于 Windows 11。高对比度模式使用纯色。</p></div><select id="qa-material" aria-label="窗口材质">${opts([['acrylic','Acrylic · 通透'],['mica','Mica · 柔和'],['solid','纯色']],s.material)}</select></div><div class="qa-setting-row"><div>列表密度</div><select id="qa-density" aria-label="列表密度">${opts([['relaxed','舒适'],['compact','紧凑']],s.density)}</select></div></section><section class="qa-setting-group"><h3>解压习惯</h3><div class="qa-setting-row"><div>默认重名处理</div><select id="qa-defaultconflict" aria-label="默认重名处理">${conflicts(s.overwrite)}</select></div></section><section class="qa-setting-group"><h3>资源管理器</h3><label class="oa-switch-row"><span>文件夹右键显示“用 qing-zip 压缩”</span><input id="oa-folder-menu" class="oa-switch" role="switch" type="checkbox" ${folderMenuEnabled?'checked':''}></label><label class="oa-switch-row"><span>文件右键显示“用 qing-zip 压缩”</span><input id="oa-file-menu" class="oa-switch" role="switch" type="checkbox" ${fileMenuEnabled?'checked':''}></label></section><div class="qa-actions">${btn('文件关联设置','associations')}${btn('资源管理器右键设置','shell-settings')}${btn('清空最近记录','clear-recent')}</div>`;}
const conflicts=v=>opts([['error','停止并报告冲突'],['rename','自动重命名'],['skip','跳过已有文件'],['overwrite','替换已有文件']],v);
const fileWorkspace=QingFileWorkspace.create({invoke,run,esc,icon,error,toast,
 probe:(path,names)=>invoke('probe_local_directory',{path,names}),isActive:()=>s.page==='filemanager',notice:showPreservationNotice,
 render:()=>{if(s.page==='filemanager')render();},
 archive:()=>({path:s.archive,displayPath:QingFileWorkspace.displayArchivePath(s.archive,archiveTabs),entries:s.entries,password:s.password,codepage:s.codepage}),
 credentials:(path,password,codepage)=>{for(const tab of archiveTabs)if(archiveKey(tab.archive)===archiveKey(path)){tab.password=password;tab.codepage=codepage;}if(archiveKey(s.archive)===archiveKey(path)){s.password=password;s.codepage=codepage;}},
 updated:async(path,entries)=>{resetPreviewState();await invoke('clear_preview_cache').catch(()=>{});const normalize=name=>name.replace(/\\/g,'/').replace(/\/+$/,'');const names=new Set();for(const entry of entries){let name=normalize(entry.name);names.add(name);while(name.includes('/')){name=name.slice(0,name.lastIndexOf('/'));names.add(name);}}const present=name=>names.has(normalize(name));const update=tab=>{tab.entries=entries;tab.selected=new Set([...tab.selected].filter(present));if(tab.anchor&&!present(tab.anchor))tab.anchor=null;if(tab.cursor&&!present(tab.cursor))tab.cursor=null;if(tab.focusFile&&!present(tab.focusFile))tab.focusFile=null;};for(const tab of archiveTabs)if(archiveKey(tab.archive)===archiveKey(path))update(tab);if(archiveKey(s.archive)===archiveKey(path))update(s);}
});
const pagePositions=new Map();
let renderedPage='';
function rememberPage(focus=document.activeElement){
 if(!renderedPage||renderedPage.startsWith('archive:'))return;
 const previous=pagePositions.get(renderedPage);
 const key=['id','data-launch-tool','data-view','data-action'].find(name=>focus!==main&&main.contains(focus)&&focus.hasAttribute(name));
 pagePositions.set(renderedPage,{top:main.scrollTop,left:main.scrollLeft,
  focus:key?'['+key+'="'+CSS.escape(focus.getAttribute(key))+'"]':previous?.focus});
}
function applyPagePosition(page,transition,position){
 main.scrollTop=position?.top||0;
 main.scrollLeft=position?.left||0;
 if(!transition||!overlay.hidden||page.startsWith('archive:'))return;
 const target=(position?.focus&&main.querySelector(position.focus))||main.querySelector('h1')||main;
 if(!target.matches('button,input,select,textarea,a[href]'))target.tabIndex=-1;
 target.focus({preventScroll:true});
}
function render(){if(overlay.dataset.page==='close-confirm')return;releaseListingWindow();stopThumbnailObserver();rememberPage();
 const page=s.page==='files'?(s.archive?'archive:'+currentTabId:'home'):s.page==='tool'?'tool:'+s.toolOp:s.page==='advanced'?'advanced:'+s.advancedTab:s.page;
 const transition=page!==renderedPage;
 const currentTab=archiveTabs.find(tab=>tab.id===currentTabId);
 const position=page.startsWith('archive:')?(transition?{top:currentTab?.scrollTop,left:currentTab?.scrollLeft}:{top:main.scrollTop,left:main.scrollLeft}):pagePositions.get(page);
 renderedPage=page;
 const pages={settings,associations:associationPage,advanced,tools:toolbox,tool:toolPage,history:historyPage,filemanager:()=>fileWorkspace.page()};main.innerHTML=s.page==='files'?(s.archive?archive():home()):`<div class="oa-page">${(pages[s.page]||toolbox)()}</div>`;renderChrome();if(s.page==='filemanager')fileWorkspace.mount(main);else fileWorkspace.suspend();if(s.page==='files'&&s.archive)rows();applyPagePosition(page,transition,position);updateColumnScroll();$('qa-status-text').textContent=s.active?'正在处理文件…':'就绪';renderInlineTask();$('oa-total').textContent=s.archive?`${s.entries.filter(e=>!e.is_dir).length} 个文件 · ${s.entries.filter(e=>e.is_dir).length} 个文件夹`:'';if(s.view==='jobs'){const html=jobs();if(!overlay.hidden&&overlay.dataset.page==='jobs'&&$('oa-dialog-content'))refreshJobContent(html);else{dialog('任务进度',`<div id="oa-dialog-content">${html}</div>`,btn('关闭','close'));overlay.dataset.page='jobs';}overlay.querySelectorAll('.qa-dialog-footer [data-action=close]').forEach(b=>b.disabled=!!s.active);}}

function navigate(v){if(homeIntake)stopHomeIntake();if($('qa-tools')?.contentWindow?.obzBusy){toast('请等待或取消当前工具任务。');return;}if(s.active&&v!=='jobs'&&!(s.backgroundExtraction&&['archive','files','history'].includes(v))){toast('请先等待或取消当前任务。');return;}if(renderedPage==='archive:'+currentTabId)saveCurrentTab();rememberPage();close(false);if(v==='archive'||v==='files'||v==='home'){s.page='files';s.view=s.archive?'archive':'home';if(v==='home'){closeArchiveTabs('all');return;}}else{s.page=v;s.view=v;}render();}

function dialog(title,body,footer,layout=''){overlay.dataset.page='';returnFocus=document.activeElement;overlay.hidden=false;overlay.innerHTML=`<section class="qa-dialog ${layout}" role="dialog" aria-modal="true" aria-labelledby="qa-dialog-title"><div class="qa-dialog-header"><h2 id="qa-dialog-title">${esc(title)}</h2>${btn('关闭','close','quiet')}</div>${layout?`<div class="oa-dialog-body">${body}</div>`:body}<div id="qa-formerror" class="qa-error" role="alert"></div><div class="qa-dialog-footer">${footer}</div></section>`;root.querySelector('.qa-shell').inert=true;if($('oa-task-panel'))$('oa-task-panel').inert=true;$('oa-menu').inert=true;$('oa-toolbar').inert=true;$('oa-archive-tabs').inert=true;overlay.querySelector('input,textarea,button')?.focus();}
function close(restoreFocus=true){if(overlay.dataset.page==='home-inspecting'&&homeIntake){stopHomeIntake();return;}if($('qa-edit-inputs')?.closest('.qa-dialog').getAttribute('aria-busy')==='true'&&overlay.dataset.page!=='close-confirm')return;if($('qa-property-status')&&Number.isInteger(s.active))invoke('cancel_operation',{id:s.active}).catch(()=>{});if(pendingNestedOpen&&!openingArchive&&!s.active){const orphan=pendingNestedOpen.session;pendingNestedOpen=null;if(orphan)invoke('close_nested',{ids:[orphan.id],discard:false}).catch(error);}$('oa-menu').hidden=true;if($('qa-tools')?.contentWindow?.obzBusy){toast('请先等待或取消当前工具任务。');return;}if(s.active&&s.view==='jobs'&&!s.backgroundExtraction){if(overlay.dataset.page==='close-confirm')action('keep-running').catch(error);return;}if(overlay.dataset.page==='home-review')homeReview=null;overlay.hidden=true;overlay.replaceChildren();overlay.dataset.page='';root.querySelector('.qa-shell').inert=false;if($('oa-task-panel'))$('oa-task-panel').inert=false;$('oa-menu').inert=false;$('oa-toolbar').inert=false;$('oa-archive-tabs').inert=false;if(s.view==='jobs')s.view=s.page==='files'?(s.archive?'archive':'home'):s.page;if(restoreFocus){if(returnFocus?.isConnected&&returnFocus.getClientRects().length)returnFocus.focus();else{main.tabIndex=-1;main.focus();}}}

function taskError(e){const text=String(e);if(/^(?:Error:\s*)?(?:operation cancelled|任务已取消)[.!。]?$/i.test(text))return '任务已取消';if(/^(?:Error:\s*)?写回失败，内层工作副本已保留，可重试或另存为：operation cancelled[.!。]?$/i.test(text))return '任务已取消。内层工作副本已保留，可重试或另存为。';return text;}
function showPreservationNotice(message){dialog('操作提示',`<p>${esc(message)}</p>`,btn('关闭','close'));}
function error(e){const message=taskError(e);if(message==='任务已取消'||message==='任务已取消。内层工作副本已保留，可重试或另存为。'){toast(message);return;}if(!overlay.hidden&&$('qa-formerror'))$('qa-formerror').textContent=message;else toast(message);}
async function copySelectedFiles(){
 const names=[...s.selected];
 if(!names.length||names.length>512)throw Error('请选择 1 至 512 个文件或文件夹。');
 const request={operation:'extract',path:s.archive,options:{selected:names,overwrite:'error',password:s.password||null,codepage:s.codepage||null}};
 const dest=await invoke('clipboard_directory');
 try{
  await run({...request,dest},'复制文件',{temporaryOutput:true});
  const paths=names.map(name=>dest+'\\'+name.replaceAll('/','\\'));
  await invoke('publish_clipboard_export',{dest,paths});
 }finally{
  await invoke('discard_clipboard_export',{dest});
 }
 toast('文件已复制到剪贴板，可在资源管理器中粘贴。');
}
async function run(request,label,{silent=false,recycleAfter=false,startJob=null,temporaryOutput=false}={}){if(s.active)throw new Error('已有任务正在运行，请先等待或取消。');const background=!silent&&!temporaryOutput&&['extract','deep-extract','deep-resume','deep-discard'].includes(request.operation);
 const startedAt=performance.now();
 s.backgroundExtraction=background;s.active='starting';if(background)updateToolbar();let id;try{id=await (startJob?startJob(request):invoke('start_operation',{request,recycleAfter}));}catch(e){s.active=null;s.backgroundExtraction=false;if(background)updateToolbar();throw e;}const j={id,label,path:request.path||'',status:'running',percent:null,startedAt,background};s.jobs.push(j);s.active=id;let autoExpand;
 if(background){
  close();s.inlineTask=id;s.inlineExpanded=false;s.inlineToggled=false;renderInlineTask();
  autoExpand=setTimeout(()=>{
   if(s.active===id&&!s.inlineToggled){s.inlineExpanded=true;renderInlineTask();}
  },300);
 }else if(!silent){close();s.view='jobs';render();}try{for(;;){const v=await invoke('wait_operation',{id});updateJobProgress(j,v);if(v.status!=='running'){if(v.status==='complete'||v.result?.deep_session){if(v.result?.deep_session)j.deep=v.result;if(['add','archive-add','archive-move','archive-copy','delete','rename','comment-set','nested-save'].includes(request.operation))await syncNestedSessions();if(temporaryOutput){j.summary='文件已准备好';return v.result;}j.outputPath=j.deep?v.result?.outputs?.[0]:v.result?.destination||v.result?.outputs?.[0];const protection=v.result?.input_atime;if(protection&&(protection.unavailable_handles||protection.unconfirmed_sources||protection.backend_best_effort)){j.timeWarning=QingFileWorkspace.preservationNotice(v.result);j.timeReasons=[...new Set((protection.warnings||[]).map(w=>w.source+'：'+({alternate_stream_proof:'附加数据检查无法完整保留访问时间',encrypted_source_proof:'加密文件检查无法确认访问时间保护',enhanced_open:'无法取得所需的文件权限',enhanced_reopen:'无法取得所需的文件权限',suppress_updates:'系统未能启用访问时间保护'}[w.phase]||'无法确认访问时间保护')))];if(protection.backend_best_effort)j.timeReasons.push('未能逐项确认外部引擎的访问时间保护');}j.partial=!!(v.result?.partial||v.result?.retained_sources?.length||v.result?.metadata_not_merged?.length);j.summary=v.result?.summary|| (v.result?.retained_sources?.length?`已移动 ${v.result.moved||0} 项；${v.result.retained_sources.length} 个来源仍保留，目标副本已保留。${v.result.retained_reasons?.[0]?`\n${v.result.retained_reasons[0]}`:''}`:v.result?.metadata_not_merged?.length?`已复制 ${v.result.copied||0} 个文件；${v.result.metadata_not_merged.length} 个文件夹的附加数据未合并，来源完整保留。\n${v.result.metadata_not_merged[0]}`:v.result?.files!==undefined?`已提取 ${v.result.files} 个文件至 ${v.result.destination}`:v.result?.outputs?'已创建：'+v.result.outputs.join('、'):v.result?.valid?'完整性检查通过':'操作完成');return v.result;}throw new Error(v.error||'任务已取消');}if(background)renderInlineTask();else if(s.view==='jobs')renderProgress();
 }}finally{
  if(autoExpand)clearTimeout(autoExpand);
  j.finishedAt=performance.now();
  await invoke('forget_operation',{id}).catch(()=>{});
  s.active=null;s.backgroundExtraction=false;
  if(background){
   if(!s.inlineToggled&&(j.status==='failed'||j.partial||j.deep?.cancelled))s.inlineExpanded=true;
   renderInlineTask();updateToolbar();if(s.page==='history')render();
  }
  else if(s.view==='jobs')renderProgress();
  schedulePendingQueue();
 }}
let nextCompressionJob=1;
const compressionDrafts=new WeakMap();

async function runCompressionStage(job, request){
 s.active='starting';
 const id=await invoke('start_operation',{request,recycleAfter:false});
 s.active=id;
 try{
  if(job.cancelRequested)await invoke('cancel_operation',{id});
  for(;;){
   const snapshot=await invoke('wait_operation',{id});
   if(snapshot.status!=='running')return snapshot;
   updateJobProgress(job,snapshot);
   renderInlineTask();
  }
 }finally{
  await invoke('forget_operation',{id}).catch(()=>{});
  s.active='starting';
 }
}

async function runCompression(requests, verify, checkOptions={}, recheck=null){
 if(tabBusy()||closingArchiveTabs||finishingArchiveTabs||removingArchiveTabs)throw Error('请先等待或取消当前任务。');
 const draft=recheck?null:overlay.querySelector('.qa-dialog');
 const rechecked=recheck?.job.archives[recheck.index];
 const job={id:`compression-${nextCompressionJob++}`,label:recheck?'检查压缩文件':'新建压缩文件',status:'running',startedAt:performance.now(),background:true,
  archives:requests.map(request=>({path:request.path,state:rechecked?'unverified':'pending',outputs:rechecked?.outputs||[],
   encrypted:rechecked?.encrypted||!!request.options?.password})),current:0,stage:recheck?'verify':'create',verify,recheck:!!recheck};
 s.compressionJob=job;
 s.active='starting';
 s.backgroundExtraction=true;
 s.jobs.push(job);
 close();
 s.inlineTask=job.id;
 s.inlineExpanded=true;
 s.inlineToggled=false;
 renderInlineTask();
 updateToolbar();
 const runStage=async (stage, state, request)=>{
  Object.assign(job,{stage,phase:null,percent:null,message:'',context:null,work:null,rate:null});
  const archive=job.archives[job.current];
  archive.state=state;
  renderInlineTask();
  let snapshot;
  try{snapshot=await runCompressionStage(job,request);}
  catch(e){snapshot={status:'failed',error:taskError(e)};}
  if(snapshot.status==='complete')return snapshot.result;
  const cancelled=snapshot.status==='cancelled';
  if(cancelled)job.cancelRequested=true;
  archive.state=stage==='verify'?(cancelled?'unverified':'verifyFailed'):(cancelled?'createCancelled':'createFailed');
  if(!cancelled)archive.error=snapshot.error||'引擎未返回结果。';
  return null;
 };
 try{
  for(const [index,request] of requests.entries()){
   if(job.cancelRequested)break;
   job.current=index;
   const archive=job.archives[index];
   if(!recheck){
    const result=await runStage('create','creating',request);
    if(!result)break;
    archive.outputs=result.outputs||[];
    if(!archive.outputs.length){
     archive.state='createFailed';
     archive.error='引擎未返回已保存的压缩文件。';
     break;
    }
    archive.state=verify?'unverified':'created';
    archive.warning=QingFileWorkspace.preservationNotice(result);
   }
   if(verify&&!job.cancelRequested){
    const checked=await runStage('verify','verifying',{operation:'test',path:archive.outputs[0],options:checkOptions});
    if(!checked)break;
    if(!checked.valid){
     archive.state='verifyFailed';
     archive.error='引擎未确认完整性。';
     break;
    }
    archive.state='verified';
   }
  }
 }finally{
  finishCompressionSummary(job);
  job.finishedAt=performance.now();
  if(rechecked&&job.status!=='cancelled'){
   const attempt=job.archives[0];
   rechecked.state=attempt.state;
   rechecked.error=attempt.error;
   finishCompressionSummary(recheck.job);
  }
  if(draft){
   const remaining=requests.filter((_,index)=>['pending','createFailed','createCancelled'].includes(job.archives[index].state));
   if(remaining.length){
    const hadPassword=remaining.some(request=>!!request.options?.password);
    draft.querySelectorAll('#qa-new-password,#oa-password-again').forEach(input=>input.value='');
    compressionDrafts.set(job,{draft,hadPassword,inputs:remaining.flatMap(request=>request.inputs)});
   }
  }
  s.compressionJob=null;
  s.active=null;
  s.backgroundExtraction=false;
  if(job.status!=='complete'&&!s.inlineToggled)s.inlineExpanded=true;
  renderInlineTask();
  updateToolbar();
  if(s.page==='history')render();
  schedulePendingQueue();
 }
}

function finishCompressionSummary(job){
 const total=job.archives.length;
 const created=job.archives.filter(archive=>archive.outputs.length).length;
 const verified=job.archives.filter(archive=>archive.state==='verified').length;
 const failed=job.archives.filter(archive=>['createFailed','verifyFailed'].includes(archive.state)).length;
 const pending=job.archives.filter(archive=>archive.state==='pending').length;
 const unverified=job.archives.filter(archive=>archive.state==='unverified').length;
 job.status=failed?'failed':pending||created<total||unverified?'cancelled':'complete';
 job.summary=[job.recheck?'已保留压缩文件':`已创建 ${created}/${total} 个压缩包`,job.verify?`检查通过 ${verified} 个`:null,
  failed?`失败 ${failed} 个`:null,pending?`未处理 ${pending} 个`:null,
  unverified?`已保留 ${unverified} 个未完成检查的压缩包`:null].filter(Boolean).join(' · ');
}

function restoreCompressionDraft(job){
 const saved=compressionDrafts.get(job);
 if(!saved||s.active)return;
 const {draft,hadPassword,inputs}=saved;
 compressionDrafts.delete(job);
 dialog('新建压缩文件','','');
 overlay.replaceChildren(draft);
 draft.dataset.retryPasswordRequired=String(hadPassword);
 $('qa-inputs').value=inputs.join('\n');
 refreshSources();
 syncCompressionPassword();
 const issue=job.archives.find(archive=>archive.state==='createFailed')?.error;
 draft.querySelector('#qa-formerror').textContent=[issue,hadPassword?'请重新输入密码。':null].filter(Boolean).join(' ');
 draft.querySelector(hadPassword?'#qa-new-password':'#qa-save')?.focus();
 renderInlineTask();
}

async function recheckCompressionOutput(job,index){
 const archive=job?.archives[index];
 if(!archive||s.active||!['unverified','verifyFailed'].includes(archive.state))return;
 const verify=password=>runCompression([{path:archive.path,options:{}}],true,password?{password}:{},{job,index});
 if(!archive.encrypted){await verify('');return;}
 dialog('重新检查完整性',`<p class="qa-sub">${esc(archive.outputs[0])}</p><label class="qa-field">压缩包密码<input id="oa-compression-recheck-password" type="password" autocomplete="off"></label>`,btn('取消','close')+btn('开始检查','compression-recheck-submit','primary'));
 const submit=overlay.querySelector('[data-action=compression-recheck-submit]');
 submit.addEventListener('click',event=>{
  event.stopPropagation();
  const password=$('oa-compression-recheck-password').value;
  if(!password){error('请输入压缩包密码。');return;}
  verify(password).catch(error);
 });
 $('oa-compression-recheck-password').focus();
}

async function useCompressionResult(button){
 const job=s.jobs.find(job=>job.id===button.dataset.compressionOutput),index=Number(button.dataset.archiveIndex);
 const path=job?.archives[index]?.outputs[0];
 if(!path)return;
 if(button.dataset.outputAction==='reveal'){await invoke('reveal_path',{path});return;}
 if(s.active)return;
 if(button.dataset.outputAction==='recheck'){await recheckCompressionOutput(job,index);return;}
 try{await openArchive(path);}catch(e){openDialog(path);error(e);}
}

function forgetRecycledArchives(paths){
 const removed=new Set(paths.map(archiveKey));
 s.recent=s.recent.filter(path=>!removed.has(archiveKey(path)));
 localStorage.setItem('qing-recent',JSON.stringify(s.recent));
 for(const tab of [...archiveTabs])if(removed.has(archiveKey(tab.archive)))discardArchiveTab(tab.archive,{preserveJob:true});
 render();
}
async function finishExtraction(source,dest,result,{recycle=false,openAfter=false,closeAfter=false}={}){if(recycle){const last=s.jobs.at(-1);if(result.recycled){s.recent=s.recent.filter(p=>p!==source);localStorage.setItem('qing-recent',JSON.stringify(s.recent));discardArchiveTab(source,{preserveJob:true});if(last)last.summary+='；原压缩包已移入回收站。';render();}else{const message='解压已完成，但原压缩包未移入回收站：'+(result.recycle_error||'原压缩包已保留。');if(last){last.summary=message;last.partial=true;}s.inlineExpanded=true;renderProgress();toast(message);}}if(openAfter)await invoke('reveal_path',{path:result.destination||dest});if(closeAfter&&(!recycle||result.recycled))await invoke('window_action',{action:'close'});return !recycle||!!result.recycled;}
async function openArchive(path,password='',{refresh=false,nested=null,folder='',listing=null,show=true,isCurrent=()=>true}={}){if(openingArchive||s.active||$('qa-tools')?.contentWindow?.obzBusy)throw Error('请先等待或取消当前任务。');openingArchive=true;try{path=listing?.path??await invoke('canonical_archive_path',{path});if(typeof path!=='string'||!path)throw Error('无法解析压缩文件路径。');const existing=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(path));if(existing&&!refresh){openingArchive=false;if(show)switchArchiveTab(existing.id);rememberRecent(path);return;}const codepage=refresh?s.codepage||null:null,r=listing??await run({operation:'list',path,options:{password:password||null,codepage}},'打开压缩包',{silent:!!$('qa-open-path')||!!$('oa-nested-password')});saveCurrentTab();resetPreviewState();await invoke('clear_preview_cache').catch(()=>{});if(!isCurrent())return;const defaults=load('qing-preferences',{}),tab=existing||{id:String(nextTabId++)};if(nested)Object.assign(tab,{nestedId:nested.id,parentArchive:nested.parent,nestedMember:nested.member,dirty:nested.dirty,conflict:nested.conflict});Object.assign(tab,{archive:path,password,entries:r.entries,codepage,folder:folder&&(folderChildren(r.entries,folder).length||r.entries.some(entry=>entry.is_dir&&entry.name===folder))?folder:'',flatView:refresh&&!!s.flatView,search:'',sort:refresh?s.sort:'name',reverse:refresh?s.reverse:false,selected:new Set(),collapsed:new Set(),anchor:null,cursor:null,focusFile:null,listStyle:refresh?s.listStyle:(defaults.listStyle==='icons'?'icons':'details'),iconSize:refresh?s.iconSize:Math.max(80,Math.min(256,Number(defaults.iconSize)||144)),scrollTop:0,treeScroll:0});if(!existing)archiveTabs.push(tab);currentTabId=tab.id;applyTab(tab);rememberRecent(path);if(show)navigate('archive');}catch(e){throw new Error(archiveOpenMessage(e,!!password));}finally{openingArchive=false;}}


function archiveOpenMessage(error,passwordProvided=false){const message=String(error);if(/wrong password|incorrect password|password is incorrect/i.test(message))return '密码不正确，请重新输入。';if(/enter password|password (?:is )?required|encrypted archive.*password/i.test(message))return passwordProvided?'无法使用此密码读取压缩包，请检查密码后重试。':'此压缩包的文件目录已加密，请输入密码后打开。';return message.replace(/^Error: /,'');}
function openDialog(path='',{allowDeep=false}={}){s.view=s.archive?'archive':'home';dialog('打开压缩包',`<label class="qa-field">文件路径<div class="qa-inlinefield"><input id="qa-open-path" value="${esc(path)}">${btn('浏览…','browse-open')}</div></label><label class="qa-field">密码（需要时填写）<input id="qa-open-password" type="password" autocomplete="off"></label>`,btn('取消','close')+(allowDeep?btn('改为深度解压','open-submit-deep'):'')+btn('打开','open-submit','primary'));}
function appendArchiveInputs(input, paths) {
  if (!input?.isConnected || !Array.isArray(paths)) return;
  const known = new Set(lines(input.id));
  const added = [];
  for (const path of paths) {
    if (typeof path !== 'string' || !path.trim() || /[\r\n\0]/.test(path) || known.has(path)) continue;
    added.push(path);
    known.add(path);
  }
  if (!added.length) return;
  const separator = input.value && !input.value.endsWith('\n') ? '\n' : '';
  input.value += separator + added.join('\n');
  if (input.id === 'qa-inputs' || input.id === 'qa-edit-inputs') refreshSources();
}
let homeIntake=null,homeReview=null;
function homeIntakeAvailable(){
  return overlay.hidden && s.page==='files' && !s.archive;
}
function stopHomeIntake(){
  finishHomeIntake(homeIntake);
}
function finishHomeIntake(intake){
  homeIntake=null;
  clearTimeout(intake.progress);
  if(overlay.dataset.page==='home-inspecting')close();
  if(intake.browsed)navigate('archive');
  schedulePendingQueue();
}
async function pickHomeArchives(){
  if(!homeIntakeAvailable()||tabBusy())return;
  const paths=await invoke('pick_home_files');
  if(paths.length)await routeHomeArchives(paths);
}
async function routeHomeArchives(paths){
  if(!paths.length)return;
  if(homeIntake){
    for(const path of paths)homeIntake.paths.set(archiveKey(path),path);
    return;
  }
  if(!homeIntakeAvailable()||tabBusy()){enqueueArchives(paths);return;}
  const intake={paths:new Map(paths.map(path=>[archiveKey(path),path])),browsed:0,path:''};
  homeIntake=intake;
  const isCurrent=()=>homeIntake===intake;
  const showStatus=()=>{
    const status=$('oa-home-identify-status');
    if(status)status.textContent='正在识别 '+filename(intake.path)+'…';
  };
  intake.progress=setTimeout(()=>{
    if(!isCurrent()||!overlay.hidden)return;
    dialog('识别压缩包','<p id="oa-home-identify-status" class="qa-sub" role="status"></p>',btn('停止识别','close'));
    overlay.dataset.page='home-inspecting';
    showStatus();
  },220);
  const deep=[], review=[];
  try{
    for(const path of intake.paths.values()){
      intake.path=path;
      showStatus();
      try{
        const result=await invoke('inspect_home_archive',{path});
        if(!isCurrent())return;
        if(result.route==='browse'){
          await openArchive(result.path,'',{listing:result,show:false,isCurrent});
          if(!isCurrent())return;
          intake.browsed++;
        }else if(result.route==='deep')deep.push(result.path);
        else review.push(result);
      }catch(reason){
        if(!isCurrent())return;
        review.push({path,reason:String(reason)});
      }
    }
  }finally{
    if(isCurrent())finishHomeIntake(intake);
  }
  if(review.length){showHomeReview(review,deep);return;}
  if(deep.length)deepExtract.batch(deep,{allowBrowse:true});
}
function homeReviewReason(reason){
  const message=archiveOpenMessage(reason);
  if(/密码|password required/i.test(message))return {text:'需要密码，选择“尝试打开”后输入。',choice:'browse'};
  if(/access (?:is )?denied|permission denied|拒绝访问|权限/i.test(message))return {text:'无法读取文件，请检查访问权限。',choice:'skip'};
  if(/not found|cannot find|no such file|找不到|不存在/i.test(message))return {text:'找不到文件，请确认路径后重新选择。',choice:'skip'};
  return {text:'无法识别压缩包，请检查格式和文件是否完整。',choice:'skip'};
}
function showHomeReview(review,deep){
  homeReview={review,deep};
  dialog('选择如何处理',`${deep.length?`<p class="qa-sub">${deep.length} 个文件已识别，继续后将加入深度解压。</p>`:''}
    <div class="oa-home-review-list">${review.map((item,i)=>{
      const {text,choice}=homeReviewReason(item.reason);
      return `<section class="oa-home-review-item">
        <label class="qa-field"><strong title="${esc(item.path)}">${esc(filename(item.path))}</strong>
        <span class="qa-sub">${text}</span>
        <select id="oa-home-review-${i}">${opts([['skip','跳过'],['browse','尝试打开'],['deep','加入深度解压']],choice)}</select></label>
        <details><summary>诊断详情</summary>
        <textarea rows="4" readonly aria-label="诊断详情">${esc(item.path+'\n'+item.reason)}</textarea>
        <button type="button" class="qa-button" data-review-copy="${i}">复制诊断</button></details>
      </section>`;
    }).join('')}</div>`,
    btn('取消','close')+btn('继续','home-review-submit','primary'),'oa-extraction-dialog');
  overlay.dataset.page='home-review';
  $('oa-home-review-0')?.focus();
  overlay.querySelectorAll('[data-review-copy]').forEach(button=>button.addEventListener('click',async ()=>{
    const diagnostic=button.closest('details').querySelector('textarea');
    try{
      await navigator.clipboard.writeText(diagnostic.value);
      toast('已复制诊断');
    }catch{
      diagnostic.focus();
      diagnostic.select();
      toast('请按 Ctrl+C 复制诊断');
    }
  }));
}
function submitHomeReview(){
  if(overlay.dataset.page!=='home-review'||!homeReview)return;
  const {review,deep}=homeReview,browse=[];
  for(const [i,item] of review.entries()){
    const choice=$('oa-home-review-'+i).value;
    if(choice==='deep')deep.push(item.path);
    if(choice==='browse')browse.push(item.path);
  }
  close();
  if(browse.length===1&&review.length===1&&!deep.length&&homeReviewReason(review[0].reason).choice==='browse'){
    openDialog(browse[0],{allowDeep:true});
    $('qa-open-password')?.focus();
    return;
  }
  if(deep.length)deepExtract.batch(deep,{allowBrowse:true});
  enqueueArchives(browse);
}
function acceptDroppedPaths(paths) {
  if(deepExtract.acceptDrop(paths))return;
  const input = !overlay.hidden && overlay.querySelector('#qa-inputs, #qa-edit-inputs');
  if (input) {
    if (s.active) toast('请等待当前任务完成后再添加文件。');
    else appendArchiveInputs(input, paths);
    return;
  }
  if(homeIntakeAvailable()||homeIntake){
    routeHomeArchives(paths).catch(error);
    return;
  }
  enqueueArchives(paths);
}
const compressionExtensions=Object.freeze({
 zip:'.zip','7z':'.7z',zipx:'.zipx',tar:'.tar',tgz:'.tar.gz',tbz2:'.tar.bz2',txz:'.tar.xz',
 gzip:'.gz',bzip2:'.bz2',xz:'.xz',zstd:'.zst',brotli:'.br',lz4:'.lz4',lz5:'.lz5',
 lizard:'.liz',wim:'.wim',sfx:'.exe',iso:'.iso',lzh:'.lzh'
});
const compressionSuffixes=Object.entries(compressionExtensions).sort((a,b)=>b[1].length-a[1].length);
const compressionCanEncrypt=format=>['zip','7z','zipx','sfx'].includes(format);
const compressionParent=path=>path.slice(0,Math.max(path.lastIndexOf('/'),path.lastIndexOf('\\'))+1);
const compressionLeaf=path=>filename(path.replace(/[\\/]+$/,''));
function compressionSuffix(path){
 const leaf=filename(path).toLowerCase();
 return compressionSuffixes.find(([,ext])=>leaf.length>ext.length&&leaf.endsWith(ext));
}
function applyCompressionExtension(path,format){
 const current=compressionSuffix(path);
 return current?.[0]===format?path:(current?path.slice(0,-current[1].length):path)+compressionExtensions[format];
}
function suggestedCompressionPath(inputs,format,separate){
 if(!inputs.length)return '';
 const first=inputs[0].replace(/[\\/]+$/,''),parent=compressionParent(first);
 if(separate)return parent||'.';
 const parentLeaf=compressionLeaf(parent);
 const base=inputs.length===1?compressionLeaf(first):(parentLeaf&&!parentLeaf.includes(':')?parentLeaf:'archive');
 return parent+base+compressionExtensions[format];
}
function sourceFields(inputId,actionPrefix){
 return '<div id="oa-sources" class="qa-source-list"></div>'
  +'<div class="qa-actions oa-source-actions"><span id="oa-source-count" class="qa-sub"></span>'
  +btn('添加文件…',actionPrefix+'-files')+btn('添加文件夹…',actionPrefix+'-folder')
  +'<button type="button" class="qa-button quiet" data-action="sources-remove" disabled>移除所选</button></div>'
  +'<details class="oa-source-paths"><summary>输入或粘贴路径</summary><textarea id="'+inputId+'" aria-label="来源路径（每行一个）"></textarea></details>';
}
function sourceParentLabels(paths){
 const rows=paths.map(path=>{
  const clean=path.replace(/[\\/]+$/,''),parent=compressionParent(clean);
  const segments=parent.split(/[\\/]+/).filter(Boolean);
  return {leaf:filename(clean).toLowerCase(),segments,parent};
 });
 const counts=new Map(),keys=(row,n)=>JSON.stringify([row.leaf,row.segments.slice(-n).map(value=>value.toLowerCase())]);
 for(const row of rows)for(let n=1;n<=row.segments.length;n++){
  const key=keys(row,n);
  counts.set(key,(counts.get(key)||0)+1);
 }
 return rows.map(row=>{
  for(let n=1;n<=row.segments.length;n++){
   if(counts.get(keys(row,n))===1)return row.segments.slice(-n).join('\\');
  }
  return row.parent||'.';
 });
}
function refreshSources(){
 const input=$('qa-inputs')||$('qa-edit-inputs'), inputs=lines(input.id);
 if(input.id==='qa-edit-inputs'){
  $('qa-sync-confirm').checked=false;
  refreshAddConflicts(input,inputs);
 }
 const parents=sourceParentLabels(inputs);
 $('oa-sources').innerHTML=inputs.map((path,i)=>'<label class="qa-source-row" title="'+esc(path)+'"><input type="checkbox" aria-label="选择 '+esc(path)+'" data-source-index="'+i+'"><span>'+esc(compressionLeaf(path))+'</span><small aria-hidden="true">'+esc(parents[i])+'</small></label>').join('')||'<p class="qa-sub">添加或拖入文件或文件夹。</p>';
 $('oa-source-count').textContent=inputs.length+' 个来源';
 syncSourceSelection();
 if(input.id==='qa-inputs'){
  const save=$('qa-save');
  if(save.dataset.explicit!=='true')save.value=suggestedCompressionPath(inputs,$('qa-format').value,$('oa-separate').checked);
  clearCompressionPlan();
 }
}
function syncSourceSelection(){
 overlay.querySelector('[data-action=sources-remove]').disabled=!overlay.querySelector('[data-source-index]:checked');
}
function syncCompressionFilename(){
 const save=$('qa-save'),hint=$('oa-compression-filename');
 const path=save.value.trim(),name=path&&!$('oa-separate').checked&&!/[\\/]$/.test(path)
  ?filename(applyCompressionExtension(path,$('qa-format').value)):'';
 const suffix=compressionSuffix(name)?.[1]||'';
 hint.hidden=!name;
 hint.title=name;
 hint.firstElementChild.textContent=suffix?name.slice(0,-suffix.length):name;
 hint.lastElementChild.textContent=suffix?name.slice(-suffix.length):'';
}
function clearCompressionPlan(){
 const preview=$('oa-compression-preview');
 if(!preview)return;
 syncCompressionFilename();
 delete preview.dataset.confirmed;
 preview.hidden=true;
 const action=overlay.querySelector('[data-action=startcompress]');
 action.textContent=$('oa-separate').checked?'预览并创建':'创建压缩文件';
}
function changeCompressionSave(save){
 save.dataset.explicit=String(!!save.value.trim());
 if(!$('oa-separate').checked){
  const suffix=compressionSuffix(save.value.trim());
  if(suffix&&suffix[0]!==$('qa-format').value){
   $('qa-format').value=suffix[0];
   methodOptions();
  }
 }
 clearCompressionPlan();
}
function changeCompressionMode(){
 const separate=$('oa-separate').checked,save=$('qa-save');
 if(separate){
  save.dataset.singlePath=save.value;
  save.dataset.singleExplicit=save.dataset.explicit||'false';
  if(save.dataset.explicit==='true')save.value=compressionParent(save.value)||'.';
 }else{
  const path=save.dataset.singlePath||'';
  if(save.dataset.singleExplicit==='true'){
   const parent=save.value.trim().replace(/[\\/]+$/,'');
   save.value=applyCompressionExtension((parent?parent+'\\':'')+filename(path),$('qa-format').value);
  }else if(save.dataset.explicit==='true'){
   const parent=save.value.trim().replace(/[\\/]+$/,'');
   const suggested=suggestedCompressionPath(lines('qa-inputs'),$('qa-format').value,false);
   save.value=parent+'\\'+(filename(suggested)||'archive'+compressionExtensions[$('qa-format').value]);
  }
 }
 $('oa-compression-save-label').textContent=separate?'保存到文件夹':'保存为';
 save.placeholder=separate?'选择输出文件夹':'输入或选择压缩包路径';
 refreshSources();
}
function changeCompressionFormat(){
 const save=$('qa-save');
 if(!$('oa-separate').checked&&save.dataset.explicit==='true')save.value=applyCompressionExtension(save.value.trim(),$('qa-format').value);
 methodOptions();
 refreshSources();
}
function compressionPasswordPresent(){return !!($('qa-new-password').value||$('oa-password-again').value);}
function syncCompressionPassword(){
 const canEncrypt=compressionCanEncrypt($('qa-format').value),panel=$('oa-password-panel'),retry=panel.closest('.qa-dialog').dataset.retryPasswordRequired==='true',pending=compressionPasswordPresent()||retry;
 $('oa-password-unsupported').hidden=canEncrypt||!pending;
 $('oa-password-button').disabled=!canEncrypt&&!pending;
 $('oa-password-button').textContent=pending?'密码已填写':'设置密码';
 $('oa-encryption').disabled=!canEncrypt;
 if(!canEncrypt)panel.hidden=true;
 else if(retry)panel.hidden=false;
 $('oa-password-button').setAttribute('aria-expanded',String(!panel.hidden));
}
function validateCompressionPassword(format){
 if(!compressionPasswordPresent()){
  if($('qa-new-password').closest('.qa-dialog').dataset.retryPasswordRequired==='true'){
   $('oa-password-panel').hidden=false;
   $('oa-password-button').setAttribute('aria-expanded','true');
   $('qa-new-password').focus();
   throw Error('请输入原压缩任务使用的密码。');
  }
  return '';
 }
 if(!compressionCanEncrypt(format)){
  $('oa-password-unsupported').scrollIntoView({block:'nearest'});
  throw Error('当前格式无法使用已填写的密码。请选择支持加密的格式，或确认不加密。');
 }
 if($('qa-new-password').value!==$('oa-password-again').value){
  $('oa-password-panel').hidden=false;
  $('oa-password-button').setAttribute('aria-expanded','true');
  $('oa-password-again').focus();
  throw Error('两次输入的密码不一致。');
 }
 return $('qa-new-password').value;
}
function methodOptions(){
 const format=$('qa-format').value;
 const methods=format==='zip'?['','Deflate','Deflate64','ZSTD','Store','BZip2','LZMA','PPMd']:format==='7z'?['','LZMA2','LZMA','ZSTD','Brotli','LZ4','LZ5','Lizard','Kanzi','PPMd','BZip2','Copy']:format==='sfx'?['','LZMA2','LZMA','PPMd','BZip2','Copy']:format==='zipx'?['','XZ','Store']:[''];
 const method=$('oa-method'),encryption=$('oa-encryption');
 method.innerHTML=opts(methods.map(v=>[v,v||'自动']),methods.includes(method.value)?method.value:'');
 const algorithms=format==='zip'?['AES256','AES192','AES128','ZipCrypto']:format==='zipx'?['AES256','AES192','AES128']:['AES256'];
 encryption.innerHTML=opts(algorithms.map(v=>[v,v]),algorithms.includes(encryption.value)?encryption.value:'AES256');
 syncCompressionPassword();
 syncCompressionControls();
}
async function confirmCompressionOutputs(format,inputs,volumeBytes){
 const preview=$('oa-compression-preview'),save=$('qa-save'),separate=$('oa-separate').checked;
 if(!inputs.length||!save.value.trim())throw Error('请填写来源和保存路径。');
 const path=save.value.trim();
 if(!separate&&/[\\/]$/.test(path))throw Error('请输入完整的压缩包文件名。');
 const parent=path.replace(/[\\/]+$/,'');
 const paths=separate?inputs.map(input=>parent+'\\'+compressionLeaf(input)+compressionExtensions[format]):[applyCompressionExtension(path,format)];
 if(!separate)save.value=paths[0];
 const button=overlay.querySelector('[data-action=startcompress]');
 if(button.disabled)return null;
 const body=preview.closest('.oa-dialog-body');
 button.disabled=true;
 body.inert=true;
 try{
  const outputs=await invoke('plan_compression_outputs',{paths,format,volumeBytes:volumeBytes||null});
  if(!preview.isConnected||save.value.trim()!==(separate?path:paths[0])||separate!==$('oa-separate').checked||format!==$('qa-format').value||JSON.stringify(inputs)!==JSON.stringify(lines('qa-inputs')))return null;
  const conflict=outputs.some((output,i)=>output!==paths[i]);
  const key=JSON.stringify({paths,outputs,volumeBytes});
  if((separate||conflict)&&preview.dataset.confirmed!==key){
   preview.innerHTML='<p>'+ (conflict?'存在同名目标，将使用以下名称。':'将创建以下压缩包。')+'</p><ul>'+outputs.map(output=>'<li title="'+esc(output)+'">'+esc(output)+'</li>').join('')+'</ul>';
   preview.dataset.confirmed=key;
   preview.hidden=false;
   button.textContent='确认创建';
   preview.scrollIntoView({block:'nearest'});
   return null;
  }
  return outputs;
 }finally{button.disabled=false;body.inert=false;}
}
const compressionLevels=window.qingCompression.levels;
function syncCompressionControls(){const format=$('qa-format')?.value,seven=['7z','sfx'].includes(format),method=$('oa-method')?.value||'',raw=['zstd','brotli','lz4','lz5','lizard'].includes(format);if(!$('oa-dictionary'))return;$('oa-dictionary').disabled=!seven||!['LZMA2','LZMA'].includes(method||'LZMA2');$('oa-solid').disabled=!seven;$('oa-header-encryption').disabled=!seven;$('qa-volume').disabled=raw;const levels=compressionLevels(format,method),current=Number($('qa-level').value),preferred=levels.some(([value])=>value===current)?current:(method||format).toLowerCase()==='lizard'?10:5;$('qa-level').innerHTML=opts(levels,preferred);const range=window.qingCompression.wordRange(format,method),store=preferred===0||['copy','store'].includes(method.toLowerCase());$('oa-word-size').disabled=!range||store;$('oa-word-label').textContent=method.toLowerCase()==='ppmd'?'模型阶数':'字长';if(range){$('oa-word-size').min=range[0];$('oa-word-size').max=range[1];$('oa-word-size').placeholder=`自动（${range[0]}–${range[1]}）`;if($('oa-word-size').value&&(Number($('oa-word-size').value)<range[0]||Number($('oa-word-size').value)>range[1]))$('oa-word-size').value='';}$('oa-solid-block').disabled=!seven||store||$('oa-solid').value==='false';for(const id of ['oa-store-mtime','oa-store-ctime','oa-store-atime','oa-archive-time'])$(id).disabled=!window.qingCompression.timeSupported(format);$('oa-preserve-atime').disabled=!['zip','7z','sfx'].includes(format);}
const timePreferenceIds=['oa-store-mtime','oa-store-ctime','oa-store-atime','oa-archive-time','oa-preserve-atime'];
function restoreTimePreferences(){const value=load('qing-compression-times',{}),saved=value&&typeof value==='object'&&!Array.isArray(value)?value:{};for(const id of timePreferenceIds){const value=saved[id];if(!$(id))continue;if($(id).type==='checkbox')$(id).checked=value===true;else if([...$(id).options].some(option=>option.value===value))$(id).value=value;}}
function saveTimePreferences(){const saved={};for(const id of timePreferenceIds)if($(id))saved[id]=$(id).type==='checkbox'?$(id).checked:$(id).value;try{localStorage.setItem('qing-compression-times',JSON.stringify(saved));}catch{} }
function compress(profile){dialog('新建压缩文件',`${sourceFields('qa-inputs','compress')}<label class="qa-check oa-mode"><input id="oa-separate" type="checkbox">每个来源创建独立压缩包</label><label class="qa-field"><span class="oa-compression-save-heading"><span id="oa-compression-save-label">保存为</span><span id="oa-compression-filename" hidden><span></span><span></span></span></span><div class="qa-inlinefield"><input id="qa-save" aria-labelledby="oa-compression-save-label" aria-describedby="oa-compression-filename" placeholder="输入或选择压缩包路径">${btn('浏览…','compress-save')}</div></label><div class="qa-fieldrow"><label class="qa-field">格式<select id="qa-format">${opts(formats,profile?.format||'zip')}</select></label><label class="qa-field">压缩级别<select id="qa-level">${opts([[0,'仅存储'],[1,'快速'],[5,'标准（推荐）'],[9,'最大压缩']],profile?.level??5)}</select></label></div><div class="qa-actions"><button class="qa-button quiet" id="oa-password-button" data-action="password-settings" aria-expanded="false" aria-controls="oa-password-panel">设置密码</button><button class="qa-button quiet" data-action="compression-advanced" aria-expanded="false" aria-controls="oa-compression-advanced">分卷与高级设置</button></div><div id="oa-password-unsupported" class="oa-form-notice" role="status" hidden><span>当前格式不支持加密，已保留填写的密码。</span>${btn('确认不加密','compression-clear-password','quiet')}</div><div id="oa-password-panel" hidden><div class="qa-fieldrow"><label class="qa-field">密码<input id="qa-new-password" type="password" autocomplete="off"></label><label class="qa-field">再次输入密码<input id="oa-password-again" type="password" autocomplete="off"></label></div><label class="qa-check"><input id="oa-show-password" type="checkbox">显示密码</label></div><div id="oa-compression-advanced" hidden><div class="qa-fieldrow"><label class="qa-field">分卷大小（MiB）<input id="qa-volume" type="number" min="1" placeholder="不分卷"></label><label class="qa-field">CPU 线程数<input id="qa-threads" type="number" min="1" max="256" placeholder="自动"></label><label class="qa-field">压缩方法<select id="oa-method"></select></label><label class="qa-field">加密算法<select id="oa-encryption"></select></label><label class="qa-field">字典大小<select id="oa-dictionary">${opts([['','自动'],['1','1 MiB'],['4','4 MiB'],['16','16 MiB'],['32','32 MiB'],['64','64 MiB'],['128','128 MiB'],['256','256 MiB']], '')}</select></label><label class="qa-field"><span id="oa-word-label">字长</span><input id="oa-word-size" type="number" step="1" placeholder="自动"></label><label class="qa-field">固实块上限<select id="oa-solid-block">${opts([['','自动'],['1','1 MiB'],['16','16 MiB'],['64','64 MiB'],['256','256 MiB'],['1024','1 GiB'],['4096','4 GiB']], '')}</select></label><label class="qa-field">固实压缩<select id="oa-solid">${opts([['','自动'],['true','开启'],['false','关闭']], '')}</select></label></div><label class="oa-switch-row"><span>加密文件名</span><input id="oa-header-encryption" class="oa-switch" role="switch" type="checkbox" checked></label><details><summary>时间信息</summary><div class="qa-fieldrow"><label class="qa-field">修改时间<select id="oa-store-mtime">${opts([['','默认'],['true','保存'],['false','不保存']], '')}</select></label><label class="qa-field">创建时间<select id="oa-store-ctime">${opts([['','默认'],['true','保存'],['false','不保存']], '')}</select></label><label class="qa-field">访问时间<select id="oa-store-atime">${opts([['','默认'],['true','保存'],['false','不保存']], '')}</select></label><label class="qa-field">压缩包时间<select id="oa-archive-time">${opts([['default','默认'],['latest','内容最新修改时间']], 'default')}</select></label></div><label class="oa-switch-row"><span>保留来源访问时间</span><input id="oa-preserve-atime" class="oa-switch" role="switch" type="checkbox"></label></details></div><label class="qa-check oa-spaced"><input id="oa-test-after" type="checkbox" checked>创建完成后检查完整性</label><section id="oa-compression-preview" class="oa-compression-preview oa-form-notice" role="status" hidden></section>`,btn('取消','close')+btn('创建压缩文件','startcompress','primary','archive'),'oa-compression-dialog');refreshSources();methodOptions();restoreTimePreferences();overlay.querySelector('[data-action=compress-files]').focus({preventScroll:true});}
function extractSelection(){
 const scope=overlay.querySelector('[name=extract-scope]:checked')?.value;
 return scope==='selected'?[...s.selected]:scope==='folder'?[s.folder]:[];
}
function archiveParent(path){
 const i=Math.max(path.lastIndexOf('\\'),path.lastIndexOf('/'));
 return i<0?'':path.slice(0,i===2&&path[1]===':'?3:i);
}
function archiveStem(path){
 const name=filename(path).replace(/\.\d{3,}$/,'');
 return /\.tar\.(gz|bz2|xz|zst)$/i.test(name)?name.replace(/\.tar\.(gz|bz2|xz|zst)$/i,''):name.replace(/(?<=.)\.[^.]+$/,'').replace(/\.part\d+$/i,'');
}
function estimatedArchiveFolder(source,entries,selected,smart,deep){
 const stem=archiveStem(source),lowerAscii=name=>name.replace(/[A-Z]/g,c=>c.toLowerCase());
 if(!smart)return deep?stem:'';
 const scope=new Set(selected.map(n=>n.replaceAll('\\','/').replace(/\/$/,'')));
 const selectedPath=name=>{
  if(!scope.size||scope.has(name))return true;
  for(let i=name.lastIndexOf('/');i>0;i=name.lastIndexOf('/',i-1)){
   if(scope.has(name.slice(0,i)))return true;
  }
  return false;
 };
 const expected=lowerAscii(stem);
 let wrapped='';
 for(const entry of entries){
  const name=entry.name.replaceAll('\\','/').replace(/^\.\//,'').replace(/\/$/,'');
  if(!selectedPath(name))continue;
  const separator=name.indexOf('/'),top=separator<0?name:name.slice(0,separator);
  if(lowerAscii(top)!==expected||(!entry.is_dir&&separator<0))return stem;
  wrapped=top;
 }
 return deep?wrapped||stem:wrapped?'':stem;
}
function extractionSummary({pathsId,statusId,recycleId,recycle=false,batch=false,numbered=false}){
 const field=batch?'textarea':'input',attrs=batch?'rows="2"':'type="text"';
 return `<div class="oa-extract-summary">
  <label class="oa-extract-paths"><span>${numbered?'预计输出（同名目录会自动编号）':'预计输出'}</span>
  <${field} id="${pathsId}" ${attrs} readonly${batch?'></textarea>':'>'}</label>
  <div class="oa-extract-disposition">
   <span id="${statusId}" class="oa-source-disposition" role="status"></span>
   <label class="oa-disposition-control">成功后回收
    <input id="${recycleId}" class="oa-switch" role="switch" type="checkbox" aria-label="全部成功后将原包移入回收站" ${recycle?'checked':''}>
   </label>
  </div>
 </div>`;
}
function sourceDisposition(recycle,{blocked=false,deep=false}={}){
 if(blocked)return '原包：当前选项下保留。';
 if(!recycle)return '原包：保留。';
 return deep?'原包：全部成功后将符合条件的原包移入回收站。':'原包：全部成功后移入回收站。';
}
function updateExtractPreview(reuseLayout=false){
 const dest=$('qa-destination')?.value.trim(),output=$('oa-expected-output');
 if(!output)return;
 const smart=$('qa-smart').checked,deep=$('oa-deep-extract').checked;
 if(!reuseLayout||output.dataset.folder===undefined)output.dataset.folder=estimatedArchiveFolder(s.archive,s.entries,extractSelection(),smart,deep);
 const folder=output.dataset.folder;
 output.value=!dest?'':folder?dest.replace(/[\\/]+$/,'')+'\\'+folder:dest;
 output.placeholder='请选择目标文件夹';
 output.title=output.value;
 output.closest('label').firstElementChild.textContent=deep?'预计输出（同名目录会自动编号）':'预计输出';
}
function extract({recycleOnce=false,forceDeep=false,forceStandard=false}={}){
 if(!s.archive)return openDialog();
 const defaultPath=s.lastDest||archiveParent(s.archive),deep=forceDeep||(!forceStandard&&s.deepExtract);
 const scope=s.selected.size?'selected':s.folder?'folder':'all';
 const radio=(value,label,available=true)=>`<label class="qa-check"><input type="radio" name="extract-scope" value="${value}" ${(recycleOnce?'all':scope)===value?'checked':''} ${available?'':'disabled'}>${label}</label>`;
 dialog(recycleOnce?'解压并移入回收站':forceDeep?'深度解压':'解压到…',`
  <p class="qa-sub">${esc(filename(s.archive))}</p>
  <div class="qa-actions" role="group" aria-label="解压范围">${radio('all','整个压缩包')}${radio('folder','当前文件夹',!!s.folder)}${radio('selected',`选中项目（${s.selected.size}）`,!!s.selected.size)}</div>
  <label class="qa-field">保存到<div class="qa-inlinefield"><input id="qa-destination" value="${esc(defaultPath)}">${btn('浏览…','extract-dest','','folder-open')}</div></label>
  <label class="qa-field">密码（可选）<input id="qa-extract-password" type="password" autocomplete="off" value="${esc(s.password)}"></label>
  <details class="oa-extract-advanced"><summary>更多解压选项</summary>
   <label class="oa-switch-row"><span>智能解压</span><input id="qa-smart" class="oa-switch" role="switch" type="checkbox" ${s.sameFolder?'checked':''}></label>
   ${deepExtract.switchMarkup(deep)}
   <label class="qa-field">遇到同名文件<select id="qa-conflict">${conflicts(recycleOnce&&s.overwrite==='skip'?'error':s.overwrite)}</select></label>
   <label class="oa-switch-row"><span>完成后打开目标文件夹</span><input id="oa-open-after" class="oa-switch" role="switch" type="checkbox" ${s.openAfter?'checked':''}></label>
   <label class="oa-switch-row"><span>完成后关闭窗口</span><input id="oa-close-after" class="oa-switch" role="switch" type="checkbox" ${s.closeAfter?'checked':''}></label>
   <details class="oa-destination-details"><summary>常用位置与子文件夹</summary><div class="oa-extract-grid"><div class="oa-places"><button data-place="desktop">桌面</button><button data-place="documents">文档</button><button data-place="pictures">图片</button><button data-place="downloads">下载</button>${btn('上级目录','destination-up')}</div><div id="oa-destination-folders" class="oa-folders"></div></div></details>
   ${deepExtract.limitsMarkup()}
  </details>
  ${recycleOnce?'<p class="qa-sub" id="oa-recycle-once-note">仅应用于此次解压。</p>':'<label class="qa-check"><input id="oa-remember-extract" type="checkbox" checked>记住解压选项</label>'}
 `,extractionSummary({pathsId:'oa-expected-output',statusId:'oa-extract-source-status',recycleId:'oa-recycle-after',recycle:recycleOnce||(deep?s.deepRecycleAfter:s.recycleAfter)})+
 btn('取消','close')+btn('开始解压','startextract','primary','folder-output'),'oa-extraction-dialog');
 $('oa-recycle-after').dataset.wanted=String($('oa-recycle-after').checked);
 deepExtract.sync();syncRecycleOption();
 const input=$('qa-destination');
 invoke('known_folders').then(v=>{s.known=v;if(input===$('qa-destination')&&input.value===defaultPath)return setDestination(defaultPath);}).catch(error);
}
async function submitExtraction(){
 const dest=$('qa-destination').value.trim();
 if(!dest)throw Error('请选择目标文件夹。');
 const source=s.archive,selected=extractSelection(),sameFolder=$('qa-smart').checked;
 const overwrite=$('qa-conflict').value,recycle=$('oa-recycle-after').checked,deep=$('oa-deep-extract').checked;
 const openAfter=$('oa-open-after').checked,closeAfter=$('oa-close-after').checked;
 if(recycle&&(selected.length||overwrite==='skip'))throw Error('仅全部解压且不跳过已有文件时，才能自动移入回收站。');
 const limits=deep?deepExtract.settings():null,password=$('qa-extract-password').value||null;
 if($('oa-remember-extract')?.checked){
  Object.assign(s,{lastDest:dest,sameFolder,overwrite,deepExtract:deep,openAfter,closeAfter});
  if(!$('oa-recycle-after').disabled)s[deep?'deepRecycleAfter':'recycleAfter']=recycle;
  if(limits)s.deepLimits=limits;
  appearance();
 }
 await runExtraction({source,dest,selected,sameFolder,overwrite,recycle,deep,openAfter,closeAfter,limits,password,codepage:s.codepage||null});
}
async function runExtraction(config){
 const {source,dest,selected,sameFolder,overwrite,recycle,deep,openAfter,closeAfter,limits,password,codepage}=config;
 const request={operation:deep?'deep-extract':'extract',path:source,dest,smart:sameFolder,
   ...(deep?{deep:{...limits,dedicated_output:true}}:{}),
   options:{selected,overwrite,password,codepage}};
 const before=s.jobs.length;
 let result;
 try{
  result=await run(request,deep?'深度解压':selected.length?'解压选中':sameFolder?'智能解压':'解压到此处',{recycleAfter:recycle});
 }catch(error){
  const job=s.jobs.length>before?s.jobs.at(-1):null;
  if(!deep&&job?.status==='failed'){
   const {password:_,...retry}=config;
   ordinaryRetries.set(job.id,retry);
   job.retryLabel=/password|密码|encrypted archive/i.test(String(error))?'输入密码重试':/exists|conflict|同名|已存在/i.test(String(error))?'处理冲突并重试':'调整选项并重试';
   s.inlineExpanded=true;renderInlineTask();if(s.page==='history')render();
  }
  throw error;
 }
 if(deep)await deepExtract.finish(result,{openAfter,closeAfter});
 else await finishExtraction(source,dest,result,{recycle,openAfter,closeAfter});
}
function openExtractionRetry(id){
 const request=ordinaryRetries.get(id);
 if(!request)return;
 dialog('重试解压',`<p class="qa-sub">${esc(request.source)}</p>
  <p class="qa-sub">${request.selected.length?`原选定范围（${request.selected.length} 项）`:'整个压缩包'}</p>
  <input id="oa-retry-id" type="hidden" value="${id}">
  <label class="qa-field">保存到<div class="qa-inlinefield"><input id="oa-retry-dest" value="${esc(request.dest)}">${btn('浏览…','extract-retry-dest')}</div></label>
  <label class="qa-field">密码（需要时填写）<input id="oa-retry-password" type="password" autocomplete="off"></label>
  <label class="qa-field">遇到同名文件<select id="oa-retry-conflict">${conflicts(request.overwrite)}</select></label>
  ${request.recycle?'<p class="qa-sub">全部成功后将原包移入回收站。</p>':''}
 `,btn('取消','close')+btn(request.recycle?'重新解压并回收':'重新解压','extract-retry-submit','primary'),'oa-extraction-dialog');
 if(s.jobs.find(j=>j.id===id)?.retryLabel==='输入密码重试')$('oa-retry-password').focus();
 else $('oa-retry-conflict').focus();
}
async function submitExtractionRetry(){
 const id=Number($('oa-retry-id').value),request=ordinaryRetries.get(id);
 if(!request)return;
 const dest=$('oa-retry-dest').value.trim(),password=$('oa-retry-password').value||null,overwrite=$('oa-retry-conflict').value;
 if(!dest)throw Error('请选择目标文件夹。');
 if(request.recycle&&overwrite==='skip')throw Error('自动回收需要完整解压，请选择另一种同名文件处理方式。');
 await runExtraction({...request,dest,password,overwrite});
 ordinaryRetries.delete(id);
 if(s.page==='history')render();
}
async function quickExtract(){
 if(!s.archive)return;
 const source=s.archive,dest=archiveParent(source),selected=[...s.selected];
 if(!dest||(!selected.length&&(s.search||s.folder||(s.deepExtract?s.deepRecycleAfter:s.recycleAfter)))){extract();return;}
 await runExtraction({source,dest,selected,sameFolder:s.sameFolder,overwrite:s.overwrite,recycle:false,deep:s.deepExtract,
  openAfter:s.openAfter,closeAfter:s.closeAfter,limits:s.deepLimits,password:s.password||null,codepage:s.codepage||null});
}
async function entry(name,{preview=false}={}){const e=visibleEntries().find(e=>e.name===name);if(!e)throw Error('此条目不在当前列表中，请重新选择。');if(e.is_dir){s.folder=name;s.selected.clear();s.search='';render();focusEntry(visibleEntries()[0]?.name);return;}if(s.active)throw Error('请先等待或取消当前任务。');if(isNestedArchive(name)&&!preview){await openNestedEntry(name);return;}if(isImage(name)&&!preview){if(openingImage)return;openingImage=true;const generation=previewGeneration;toast('正在准备原始图片…');try{await invoke('open_archive_image',previewArgs(name));if(generation===previewGeneration)toast('已用系统默认看图程序打开原始图片。');}finally{openingImage=false;}return;}dialog(e.leaf,`<p class="qa-sub">${esc(e.name)} · ${entrySize(e)}</p><div id="qa-file-preview" role="status">${isText(name)?'正在读取文本…':''}</div>`,btn('返回','close')+btn('解压此文件','extractfile','primary'));s.focusFile=e.name;const target=$('qa-file-preview'),generation=previewGeneration;if(isText(name)){overlay.querySelector('.qa-dialog').classList.add('oa-text-dialog');target.innerHTML=`<div class="oa-text-toolbar"><label>文本编码<select id="oa-text-encoding">${opts([['auto','自动检测'],['utf-8','UTF-8'],['gbk','简体中文 GBK'],['utf-16le','UTF-16 LE'],['utf-16be','UTF-16 BE'],['windows-1252','西欧 Windows-1252']],'auto')}</select></label><span id="oa-text-meta" class="oa-preview-meta" role="status"></span></div><pre id="oa-text-preview" class="oa-text-preview" tabindex="0"></pre>`;target.oaTextContext={name,generation,args:previewArgs(name)};await loadTextPreview(target,'auto');}else if(isImage(name)||/\.avif$/i.test(name)){try{const r=await run({operation:'preview',path:s.archive,name,options:archiveOptions()},'图片预览',{silent:true});if(target===$('qa-file-preview')&&generation===previewGeneration){const img=document.createElement('img');img.src=r.data_url;img.alt=e.leaf;img.className='qa-preview';target.append(img);}}catch(e){if(target===$('qa-file-preview'))error(e);}}else{target.textContent='此文件暂不支持内嵌预览，可解压后打开。';}}
const archiveOptions=()=>({password:s.password||null,codepage:s.codepage||null});
const lines=id=>$(id).value.split(/\r?\n/).map(p=>p.trim()).filter(Boolean);
function showAddForm(paths=[]){
 const target=s.folder||'',archive=s.archive,existing=s.entries.some(entry=>entry.encrypted)||!!s.password;
 const zip=/\.zipx?$/i.test(archive),seven=/\.7z$/i.test(archive),canEncrypt=existing||zip||seven;
 const defaultMode=existing&&s.password?'current':existing&&canEncrypt?'enter':'none';
 const levels=/\.zipx$/i.test(archive)?['','AES256','AES192','AES128']:['','AES256','AES192','AES128','ZipCrypto'];
 dialog('添加文件',`
  <div class="oa-add-target" title="${esc(archive)}"><span>压缩包</span><strong>${esc(filename(archive))}</strong><span>包内位置</span><strong>${esc(target?'/'+target.replace(/\/$/,''):'/（根目录）')}</strong></div>
  ${sourceFields('qa-edit-inputs','edit')}
  <p id="oa-add-conflicts" class="oa-form-notice" role="status" hidden></p>
  <label class="qa-field">更新方式<select id="qa-update-mode"><option value="add">添加并替换</option><option value="update">更新较新文件并添加</option><option value="fresh">仅更新已有文件</option><option value="sync">同步所选范围</option></select></label>
  <p id="oa-add-mode-help" class="qa-sub" role="status"></p>
  <section id="qa-sync-warning" class="oa-form-notice" hidden><label class="qa-check"><input type="checkbox" id="qa-sync-confirm">允许删除所选范围内本地已缺失的条目</label></section>
  <details id="oa-add-details" ${existing?'open':''}><summary>密码与其他设置</summary>
   ${canEncrypt?`<label class="qa-field">新写入文件的密码<select id="oa-add-password-mode">${opts([
    ...(!existing?[['none','不设置密码']]:[]),
    ...(s.password?[['current','使用打开压缩包时的密码']]:[]),
    ['enter',existing?'输入压缩包密码':'设置密码']
   ],defaultMode)}</select></label>
   <div id="oa-add-password-fields" hidden><div class="qa-fieldrow"><label class="qa-field">密码<input id="oa-add-password" type="password" autocomplete="new-password"></label>${existing?'':`<label class="qa-field">再次输入密码<input id="oa-add-password-confirm" type="password" autocomplete="new-password"></label>`}</div></div>
   ${zip?`<label class="qa-field" id="oa-add-algorithm">加密算法<select id="oa-add-encryption">${opts(levels.map(value=>[value,value||'自动（沿用原包 / AES256）']),'')}</select></label>`:''}
   <p id="oa-add-encryption-help" class="qa-sub" hidden>${existing?'原有加密条目需要正确的压缩包密码。新写入文件会使用该密码。':'此密码用于新写入的文件（包括替换的文件）。'}</p>`:''}
   <label class="oa-switch-row"><span>保留来源访问时间</span><input id="oa-add-preserve-atime" class="oa-switch" role="switch" type="checkbox" ${window.QingFileWorkspace.preservePreference()?'checked':''}></label>
  </details><p id="oa-add-status" class="qa-sub" role="status" hidden></p>`,btn('取消','close')+'<button class="qa-button" data-action="add-cancel-task" hidden>取消任务</button>'+btn('添加','add-submit','primary'),'oa-compression-dialog oa-add-dialog');
 const input=$('qa-edit-inputs');
 input.addContext={archive,target,existing,existingNames:new Set(folderChildren(s.entries,target).map(entry=>entry.leaf.toLowerCase()))};
 input.value=paths.join('\n');
 refreshSources();
 syncAddMode();
 syncAddPassword();
 overlay.querySelector('[data-action=edit-files]').focus({preventScroll:true});
}
function refreshAddConflicts(input,inputs){
 const existing=input.addContext.existingNames, names=new Map(), matches=new Set(), duplicate=new Set();
 for(const source of inputs){
  const name=compressionLeaf(source),key=name.toLowerCase();
  if(existing.has(key))matches.add(name);
  if(names.has(key))duplicate.add(name);
  names.set(key,name);
 }
 const preview=$('oa-add-conflicts'),format=values=>[...values].slice(0,4).join('、')+(values.size>4?` 等 ${values.size} 项`:'');
 const messages=[];
 if(duplicate.size)messages.push('来源中有同名项目，请保留其中一项：'+format(duplicate)+'。');
 if(matches.size)messages.push('包内已有同名项目，将按所选更新方式处理：'+format(matches)+'。');
 preview.textContent=messages.join(' ');
 preview.hidden=messages.length===0;
 input.addContext.duplicate=duplicate.size>0;
}
const addModes={
 add:['添加','替换包内同名文件。'],
 update:['更新','替换较新的文件，或修改时间相同但大小不同的文件，并添加新文件。'],
 fresh:['更新','只更新已有文件，按修改时间和大小判断。'],
 sync:['同步','按修改时间和大小更新，并删除所选范围内本地已缺失的条目。']
};
function syncAddMode(){
 const mode=$('qa-update-mode').value,[label,help]=addModes[mode];
 $('oa-add-mode-help').textContent=help;
 $('qa-sync-confirm').checked=false;
 $('qa-sync-warning').hidden=mode!=='sync';
 overlay.querySelector('[data-action=add-submit]').textContent=label;
}
function syncAddPassword(){
 const mode=$('oa-add-password-mode')?.value;
 if(!mode)return;
 $('oa-add-password-fields').hidden=mode!=='enter';
 $('oa-add-encryption-help').hidden=mode==='none';
 if($('oa-add-algorithm'))$('oa-add-algorithm').hidden=mode==='none';
}
async function submitAddForm(){
 const input=$('qa-edit-inputs'),inputs=lines(input.id),{archive,target,existing,duplicate}=input.addContext;
 const dialogNode=input.closest('.qa-dialog');
 if(dialogNode.getAttribute('aria-busy')==='true')return;
 if(!inputs.length)throw Error('请添加文件或文件夹。');
 if(duplicate)throw Error('请移除来源中的同名项目，避免写入相同的包内位置。');
 const update_mode=$('qa-update-mode').value;
 if(update_mode==='sync'&&!$('qa-sync-confirm').checked)throw Error('请先确认同步删除范围。');
 const mode=$('oa-add-password-mode')?.value||'',options={...archiveOptions(),update_mode};
 if(mode==='none')options.password=null;
 if(mode==='enter'){
  const password=$('oa-add-password').value;
  if(!password){$('oa-add-password').focus();throw Error('请输入密码。');}
  if(!existing&&password!==$('oa-add-password-confirm').value){$('oa-add-password-confirm').focus();throw Error('两次输入的密码不一致。');}
  options.password=password;
 }
 if(options.password&&$('oa-add-encryption')?.value)options.encryption=$('oa-add-encryption').value;
 if($('oa-add-preserve-atime').checked)options.preserve_input_atime=true;
 const message=$('qa-formerror'),status=$('oa-add-status'),body=input.closest('.oa-dialog-body');
 const controls=[...dialogNode.querySelectorAll('[data-action=close],[data-action=add-submit]')];
 const cancel=dialogNode.querySelector('[data-action=add-cancel-task]');
 message.textContent='';
 status.hidden=false;
 status.textContent='正在更新压缩包…';
 dialogNode.setAttribute('aria-busy','true');
 body.querySelectorAll(':scope > :not(#oa-add-status)').forEach(node=>node.inert=true);
 controls.forEach(control=>control.disabled=true);
 cancel.hidden=false;
 cancel.disabled=false;
 status.scrollIntoView({block:'nearest'});
 cancel.focus({preventScroll:true});
 let result, passwordError=false;
 try{
  result=await run({operation:'add',path:archive,dest:target,inputs,options},update_mode==='sync'?'同步文件':'添加/更新文件',{silent:true});
 }catch(e){
  passwordError=!!$('oa-add-password-mode')&&/password|密码/i.test(String(e));
  message.textContent=taskError(e);
 }finally{
  if(dialogNode.isConnected){
   dialogNode.removeAttribute('aria-busy');
   body.querySelectorAll(':scope > :not(#oa-add-status)').forEach(node=>node.inert=false);
   controls.forEach(control=>control.disabled=false);
   status.hidden=true;
   cancel.hidden=true;
  }
 }
 if(!result){
  if(passwordError&&dialogNode.isConnected){
   $('oa-add-details').open=true;
   $('oa-add-password-mode').value='enter';
   $('oa-add-password').value='';
   if($('oa-add-password-confirm'))$('oa-add-password-confirm').value='';
   syncAddPassword();
   $('oa-add-password').focus();
  }
  return;
 }
 close();
 try{await openArchive(archive,options.password||'',{refresh:true,folder:target});}
 catch(e){throw Error('压缩包已更新，重新读取失败：'+taskError(e));}
 const notice=QingFileWorkspace.preservationNotice(result);
 if(notice)showPreservationNotice(notice);
}
async function action(a){switch(a){
case'close':close();break;case'keep-running':$('oa-exit-confirm')?.remove();overlay.querySelectorAll('.qa-dialog').forEach(e=>e.inert=false);overlay.dataset.page=s.returnPage||'';if(s.view==='jobs')renderProgress();else if(!overlay.querySelector('.qa-dialog')){overlay.hidden=true;root.querySelector('.qa-shell').inert=false;if($('oa-task-panel'))$('oa-task-panel').inert=false;}break;case'sources-remove':{const input=$('qa-inputs')||$('qa-edit-inputs'),drop=new Set([...overlay.querySelectorAll('[data-source-index]:checked')].map(e=>Number(e.dataset.sourceIndex)));input.value=lines(input.id).filter((_,i)=>!drop.has(i)).join('\n');refreshSources();break;}
case'password-settings':{const panel=$('oa-password-panel');if(!compressionCanEncrypt($('qa-format').value)){$('oa-password-unsupported').scrollIntoView({block:'nearest'});break;}panel.hidden=!panel.hidden;$('oa-password-button').setAttribute('aria-expanded',String(!panel.hidden));if(!panel.hidden)$('qa-new-password').focus();break;}
case'compression-clear-password':{delete $('qa-new-password').closest('.qa-dialog').dataset.retryPasswordRequired;$('qa-new-password').value='';$('oa-password-again').value='';syncCompressionPassword();$('qa-formerror').textContent='';break;}
case'compression-advanced':{
 const panel=$('oa-compression-advanced'),toggle=overlay.querySelector('[data-action=compression-advanced]');
 panel.hidden=!panel.hidden;
 toggle.setAttribute('aria-expanded',String(!panel.hidden));
 if(!panel.hidden)toggle.scrollIntoView({block:'start'});
 break;
}
case'destination-up':{const path=$('qa-destination').value.replace(/[\\/]+$/,'').replace(/[\\/][^\\/]+$/,'');await setDestination(path);break;}case'cancel-exit':{s.batchStop=true;const box=$('oa-exit-confirm');box?.querySelectorAll('button').forEach(b=>b.disabled=true);try{if(s.compressionJob){s.compressionJob.cancelRequested=true;if(Number.isInteger(s.active))await invoke('cancel_operation',{id:s.active});while(s.compressionJob)await new Promise(resolve=>setTimeout(resolve,75));}if(await invoke('cancel_and_close')===false){await action('keep-running');const finish=()=>{if(tabBusy()){setTimeout(finish,100);return;}closeArchiveTabs('exit');};finish();}}catch(e){box?.querySelectorAll('button').forEach(b=>b.disabled=false);throw e;}break;}case'home-smart':await pickHomeArchives();break;case'home-review-submit':submitHomeReview();break;case'open-submit-deep':{const path=$('qa-open-path').value.trim(),password=$('qa-open-password').value;if(path)deepExtract.batch([path],{password,allowBrowse:true});break;}case'open':{if(homeIntakeAvailable()){await pickHomeArchives();break;}const paths=await invoke('pick_files');enqueueArchives(paths);break;}case'open-path':openDialog();break;case'browse-open':{const p=await invoke('pick_files');if(p[0])$('qa-open-path').value=p[0];break;}case'open-submit':{const pathInput=$('qa-open-path'),passwordInput=$('qa-open-password'),dialogNode=pathInput?.closest('.qa-dialog');if(!dialogNode||pathInput.disabled)break;const path=pathInput.value.trim(),password=passwordInput.value;if(!path)throw Error('请选择压缩包。');const controls=[pathInput,passwordInput,...dialogNode.querySelectorAll('[data-action=open-submit],[data-action=browse-open]')],message=dialogNode.querySelector('#qa-formerror');message.textContent='';let status=dialogNode.querySelector('#qa-open-status');if(!status){status=document.createElement('p');status.id='qa-open-status';status.className='qa-sub';status.setAttribute('role','status');message.before(status);}status.textContent='正在打开压缩文件…';dialogNode.setAttribute('aria-busy','true');controls.forEach(control=>control.disabled=true);try{await openArchive(path,password);}catch(e){if(dialogNode.isConnected&&overlay.contains(dialogNode))message.textContent=String(e);else toast(String(e));}finally{if(dialogNode.isConnected&&overlay.contains(dialogNode)){controls.forEach(control=>control.disabled=false);dialogNode.removeAttribute('aria-busy');status.textContent='';}}break;}
case'compress':compress();break;case'compress-files':{const input=$('qa-inputs');appendArchiveInputs(input,await invoke('pick_files_for_zip'));break;}case'compress-folder':{const input=$('qa-inputs');appendArchiveInputs(input,[await invoke('pick_folder')]);break;}case'compress-save':{const save=$('qa-save'),separate=$('oa-separate').checked,path=save.value.trim();const p=separate?await invoke('pick_folder'):await invoke('pick_archive_save',{format:$('qa-format').value,suggestion:path});if(p&&save.isConnected&&separate===$('oa-separate').checked){save.value=p;changeCompressionSave(save);}break;}
case'startcompress':{const inputs=lines('qa-inputs'),format=$('qa-format').value,separate=$('oa-separate').checked,verify=$('oa-test-after').checked;
const password=validateCompressionPassword(format),options={level:Number($('qa-level').value)};if(password){options.password=password;options.encryption=$('oa-encryption').value;}
if($('oa-method').value)options.method=$('oa-method').value;if(!$('qa-volume').disabled&&$('qa-volume').value)options.volume_bytes=Number($('qa-volume').value)*1048576;if($('qa-threads').value)options.threads=Number($('qa-threads').value);if(!$('oa-dictionary').disabled&&$('oa-dictionary').value)options.dictionary_bytes=Number($('oa-dictionary').value)*1048576;if(!$('oa-solid').disabled&&$('oa-solid').value)options.solid=$('oa-solid').value==='true';if(!$('oa-word-size').disabled&&$('oa-word-size').value)options.word_size=Number($('oa-word-size').value);if(!$('oa-solid-block').disabled&&$('oa-solid-block').value)options.solid_block_bytes=Number($('oa-solid-block').value)*1048576;if(options.password&&!$('oa-header-encryption').disabled)options.header_encryption=$('oa-header-encryption').checked;for(const key of ['mtime','ctime','atime']){const field=$('oa-store-'+key);if(!field.disabled&&field.value!=='')options['store_'+key]=field.value==='true';}if(!$('oa-archive-time').disabled&&$('oa-archive-time').value==='latest')options.archive_timestamp='latest';if(!$('oa-preserve-atime').disabled&&$('oa-preserve-atime').checked)options.preserve_input_atime=true;const checkOptions={...options};for(const key of ['store_mtime','store_ctime','store_atime','archive_timestamp','preserve_input_atime'])delete checkOptions[key];
const outputs=await confirmCompressionOutputs(format,inputs,options.volume_bytes);if(!outputs)return;
const requests=outputs.map((path,i)=>({operation:'create',path,inputs:separate?[inputs[i]]:inputs,format,options}));
await runCompression(requests,verify,checkOptions);break;}
case'deep-batch':deepExtract.batch();break;case'extract':extract();break;case'extract-selected':if(s.selected.size)extract();else toast('请在当前列表中选择要解压的项目。');break;case'extractfile':s.selected=new Set([s.focusFile]);extract();break;case'extract-dest':{const p=await invoke('pick_folder');if(p)await setDestination(p);break;}case'startextract':await submitExtraction();break;
case'quick-extract':await quickExtract();break;
case'extract-deep':extract({forceDeep:true});break;
case'extract-standard':extract({forceStandard:true});break;
case'extract-menu':showExtractMenu();break;
case'extract-retry-submit':await submitExtractionRetry();break;
case'extract-retry-dest':{const path=await invoke('pick_folder');if(path&&$('oa-retry-dest'))$('oa-retry-dest').value=path;break;}
case'root':gotoArchiveFolder('');focusEntry(visibleEntries()[0]?.name);break;case'up':{const previous=s.folder;gotoArchiveFolder(s.folder.replace(/[^/]+\/$/,''));s.cursor=previous;focusEntry(s.cursor);}break;case'test':await run({operation:'test',path:s.archive,options:archiveOptions()},'完整性检查');break;
case'shell-continue':await retryShellExtraction();break;case'shell-password':showShellPassword();break;case'shell-password-submit':await retryShellExtraction(true);break;case'shell-clear':clearShellQueue();break;case'task-toggle':s.inlineToggled=true;s.inlineExpanded=!s.inlineExpanded;renderInlineTask();break;
case'canceljob':{const job=s.compressionJob||s.jobs.find(j=>j.id===s.active);if(job){job.cancelRequested=true;renderInlineTask();}}s.batchStop=true;if(shellQueueRunning){shellQueuePaused=true;shellQueueInterrupted=true;if(shellActiveRequest)shellActiveRequest.cancelRequested=true;}if(Number.isInteger(s.active))await invoke('cancel_operation',{id:s.active});break;
case'clear-recent':updateRecent([]);break;case'remove-recent':updateRecent(s.recent.filter(p=>p!==recentContextPath));break;
case'add':showAddForm();break;
case'edit-files':{const input=$('qa-edit-inputs');appendArchiveInputs(input,await invoke('pick_files_for_zip'));break;}
case'edit-folder':{const input=$('qa-edit-inputs');appendArchiveInputs(input,[await invoke('pick_folder')]);break;}
case'add-submit':await submitAddForm();break;
case'add-cancel-task':{if(!Number.isInteger(s.active))break;const cancel=overlay.querySelector('[data-action=add-cancel-task]');cancel.disabled=true;try{await invoke('cancel_operation',{id:s.active});}catch(e){cancel.disabled=false;throw e;}break;}
case'delete':if(!s.selected.size)throw Error('请先选中要删除的条目。');dialog('删除选中条目',`<p>将从压缩包删除 ${s.selected.size} 项。原始来源文件不受影响。</p>`,btn('取消','close')+btn('确认删除','delete-submit','danger'));break;
case'delete-submit':await run({operation:'delete',path:s.archive,inputs:[...s.selected],options:archiveOptions()},'删除条目');await openArchive(s.archive,s.password,{refresh:true});break;
case'rename':if(s.selected.size!==1)throw Error('请只选中一个要重命名的条目。');dialog('重命名',`<label class="qa-field">包内完整新名称<input id="qa-rename" value="${esc([...s.selected][0])}"></label>`,btn('取消','close')+btn('保存名称','rename-submit','primary'));break;
case'rename-submit':{const name=$('qa-rename').value;if(!name)throw Error('名称不能为空。');await run({operation:'rename',path:s.archive,inputs:[[...s.selected][0],name],options:archiveOptions()},'重命名');await openArchive(s.archive,s.password,{refresh:true});break;}
case'shell-settings':showTool('shell-enable','资源管理器右键入口');break;
case'batch-pick':{const p=await invoke('pick_files');$('qa-batch-inputs').value=p.join('\n');break;}case'batch-dest':{const p=await invoke('pick_folder');if(p)$('qa-batch-dest').value=p;break;}case'batch-stop':s.batchStop=true;if(s.active)await invoke('cancel_operation',{id:s.active});toast('队列已停止');break;
case'batch-start':{const paths=lines('qa-batch-inputs'),operation=$('qa-batch-op').value,dest=$('qa-batch-dest').value.trim();if(!paths.length||(operation==='extract'&&!dest))throw Error('请填写压缩包列表和解压目标。');if(s.active)throw Error('已有任务正在运行。');s.batchStop=false;close();s.view='jobs';render();for(const [i,path] of paths.entries()){if(s.batchStop)break;try{await run({operation,path,dest:operation==='extract'?dest+'\\'+(i+1)+'-'+filename(path).replace(/\.[^.]+$/,''):'',options:{overwrite:s.overwrite}},`批量 ${i+1}/${paths.length} · ${operation==='test'?'检查':'解压'}`);}catch(e){toast(String(e));}}break;}
case'profile-save':{const name=$('qa-profile-name').value.trim();if(!name)throw Error('请输入预设名称。');s.profiles=s.profiles.filter(p=>p.name!==name);s.profiles.push({name,format:$('qa-profile-format').value,level:Number($('qa-profile-level').value)});localStorage.setItem('qing-profiles',JSON.stringify(s.profiles));render();break;}
}}

const menuGroups={
 file:[['打开压缩文件','open','Ctrl+O'],['输入文件路径…','open-path',''],['最近的文件','recent-menu',''],['新建压缩文件','compress','Ctrl+N'],null,['关闭当前标签','close-archive','Ctrl+W'],['关闭所有标签','close-all-tabs',''],['保存至外层压缩包','save-nested','Ctrl+S'],['另存为…','save-as',''],['删除压缩文件','recycle-archive','Ctrl+Del'],['打开压缩文件位置','reveal',''],null,['压缩包属性','archive-properties',''],['测试压缩文件','test','Ctrl+T'],['解压到…','extract','Ctrl+E'],['解压并移入回收站…','extract-recycle',''],null,['退出','exit','Alt+F4']],
 edit:[['作为压缩包打开','open-as-archive',''],['外部编辑…','external-edit',''],['外部编辑会话…','external-sessions',''],['保留的编辑副本…','external-copies',''],null,['重命名文件','rename','F2'],['添加文件','add','Ins'],['删除文件','delete','Del'],['压缩文件注释','comment','Ctrl+Shift+M'],null,['全选','select-all','Ctrl+A'],['反向选择','invert',''],['按名称选中…','select-mask','小键盘 +'],['按名称取消选中…','unselect-mask','小键盘 −'],['选中同类型','select-type','Alt+小键盘 +'],['取消选中同类型','unselect-type','Alt+小键盘 −'],['复制完整路径','copy-path',''],['将文件复制到剪贴板','copy-files','Ctrl+C'],['从剪贴板粘贴文件','paste-files','Ctrl+V'],null,['属性','properties','Alt+Enter']],
 find:[['查找文件','find','Ctrl+F']],
 options:[['设置','settings','F5'],['密码管理器…','vault-tool',''],['压缩预设…','profiles','']],
 view:[['启动画面','close-archive','F6'],['树形目录','toggle-tree','F7'],['平铺视图','toggle-flat',''],['文件预览','preview-selected',''],['刷新当前标签','refresh-archive','Ctrl+R'],['状态栏','toggle-status',''],['文件名代码页…','codepage',''],null,['工具栏：大','toolbar-large',''],['工具栏：小','toolbar-small',''],['工具栏：隐藏','toolbar-hidden',''],null,['列表列…','columns',''],['详细信息','details',''],['图标','icons',''],['颜色设置…','settings','']],
 tools:[['修复压缩包…','repair-tool',''],['密码恢复…','recover-tool',''],['文件校验值…','checksum-tool',''],['批量处理…','batch',''],['资源管理器右键入口…','shell-settings','']],
 help:[['关于 qing-zip','about','F1'],['使用说明','help','']]
};
function updateToolbar(){root.querySelectorAll('#oa-toolbar [data-action],#oa-primary-action [data-action]').forEach(b=>{const a=b.dataset.action;b.disabled=(!s.archive&&!['open','compress','more-menu'].includes(a))||(!!s.active&&!['toggle-tree'].includes(a));if(a==='quick-extract'&&b.lastChild)b.lastChild.textContent=quickExtractLabel();if(a==='toggle-tree')b.setAttribute('aria-expanded',String(root.dataset.tree!=='hidden'));});if($('oa-selection-actions')){$('oa-selection-actions').hidden=!s.selected.size;$('oa-selected-count').textContent=`已选 ${s.selected.size} 个项目`;$('oa-selection-actions').querySelectorAll('[data-action]').forEach(b=>b.disabled=!!s.active||(b.dataset.action==='rename'&&s.selected.size!==1));}root.querySelectorAll('[data-view]').forEach(b=>{b.setAttribute('aria-current',String(b.dataset.view===(['tool','advanced'].includes(s.page)?'tools':s.page)));});}
// Sorted descendants are contiguous, so tree expansion does not rescan every folder.
const archiveFolderTrees = new WeakMap();
function folderTree(entries, collapsed) {
  let tree = archiveFolderTrees.get(entries);
  if (!tree) {
    const folders = new Set(['']);
    for (const entry of entries) {
      const parts = entry.name.replaceAll('\\','/').split('/').filter(Boolean);
      if (!entry.is_dir) parts.pop();
      let path = '';
      for (const part of parts) { path += part + '/'; folders.add(path); }
    }
    const names = [...folders].sort();
    tree = names.map((path, i) => ({
      path, depth:path.split('/').filter(Boolean).length,
      hasChildren:i + 1 < names.length && names[i + 1].startsWith(path)
    }));
    const siblings=new Map();for(const row of tree){const parent=row.path?row.path.replace(/[^/]+\/$/,''):null;const group=siblings.get(parent)||[];group.push(row);siblings.set(parent,group);}for(const group of siblings.values())group.forEach((row,i)=>{row.position=i+1;row.siblings=group.length;});
    archiveFolderTrees.set(entries, tree);
  }
  if (!collapsed.size) return tree;
  let hiddenPrefix = null;
  return tree.filter(({path}) => {
    if (hiddenPrefix !== null && path.startsWith(hiddenPrefix)) return false;
    hiddenPrefix = collapsed.has(path) ? path : null;
    return true;
  });
}
// End folder tree model.
function folderRow({path:p,depth,hasChildren,position,siblings}) {
  return `<div class="oa-tree-row" role="none" style="--depth:${depth}">${hasChildren?`<button class="oa-tree-toggle" aria-hidden="true" tabindex="-1" data-tree-toggle="${esc(p)}" aria-label="${s.collapsed.has(p)?'展开':'折叠'} ${esc(p||'根目录')}" aria-expanded="${!s.collapsed.has(p)}">${icon('chevron')}</button>`:'<span class="oa-tree-toggle"></span>'}<button data-tree="${esc(p)}" role="treeitem" aria-level="${depth+1}" aria-posinset="${position}" aria-setsize="${siblings}" ${hasChildren?`aria-expanded="${!s.collapsed.has(p)}"`:""} aria-current="${p===s.folder&&s.page==='files'}">${icon(p?'folder':'archive')}<span>${esc(p?p.split('/').filter(Boolean).pop():s.archive?filename(s.archive):'未打开压缩文件')}</span></button></div>`;
}

let renderedTree;
function renderFolderTree() {
  const host = $('oa-tree');
  host.onkeydown=handleTreeKeydown;
  host.onpointerdown=event=>{if(event.target.closest('[data-tree-toggle]'))event.preventDefault();};
  const current = s.page === 'files' ? s.folder : null;
  if (renderedTree && renderedTree.entries === s.entries && renderedTree.archive === s.archive
      && renderedTree.collapsed.size === s.collapsed.size
      && [...s.collapsed].every(path => renderedTree.collapsed.has(path))) {
    if (current !== renderedTree.current) {
      host.querySelector('[data-tree][aria-current="true"]')?.setAttribute('aria-current', 'false');
      const button = current === null ? null : host.querySelector('[data-tree="' + CSS.escape(current) + '"]');
      button?.setAttribute('aria-current', 'true');
      renderedTree.current = current;
    }
    return;
  }
  const viewport = host.closest('.oa-tree-section');
  const sameListing = renderedTree?.entries === s.entries;
  const scrollTop = sameListing ? viewport.scrollTop : 0;
  const focused = sameListing && host.contains(document.activeElement) ? document.activeElement.dataset : null;
  renderedTree?.window?.stop();
  const rows = folderTree(s.entries, s.collapsed);
  const treeWindow = rows.length > 256 ? new ArchiveTreeWindow(host, rows, folderRow, scrollTop) : null;
  if (!treeWindow) {
    host.innerHTML = rows.map(folderRow).join('');
    viewport.scrollTop = scrollTop;
  }
  if (focused?.treeToggle !== undefined || focused?.tree !== undefined) {
    const toggle = focused.treeToggle !== undefined;
    host.querySelector('[' + (toggle ? 'data-tree-toggle' : 'data-tree') + '="' + CSS.escape(toggle ? focused.treeToggle : focused.tree) + '"]')?.focus({preventScroll:true});
  }
  renderedTree = {entries:s.entries, archive:s.archive, collapsed:new Set(s.collapsed), current, rows, window:treeWindow};
}
function focusTreePath(path){const tree=renderedTree;if(!tree)return;const index=tree.rows.findIndex(row=>row.path===path);if(index<0)return;if(tree.window)tree.window.focusIndex(index);else{const node=$('oa-tree').querySelector('[data-tree="'+CSS.escape(path)+'"]');node?.focus({preventScroll:true});node?.scrollIntoView({block:'nearest'});}}
function handleTreeKeydown(event){
 if(isCompositionKey(event))return;
 if(event.defaultPrevented||event.altKey||event.ctrlKey||event.metaKey)return;
 const node=event.target.closest('[data-tree],[data-tree-toggle]');if(!node||!renderedTree)return;
 const path=node.dataset.tree??node.dataset.treeToggle,rows=renderedTree.rows,index=rows.findIndex(row=>row.path===path);if(index<0)return;
 const row=rows[index];let next=index;
 switch(event.key){
 case 'ArrowUp':next=Math.max(0,index-1);break;
 case 'ArrowDown':next=Math.min(rows.length-1,index+1);break;
 case 'Home':next=0;break;
 case 'End':next=rows.length-1;break;
 case 'ArrowLeft':if(row.hasChildren&&!s.collapsed.has(path)){s.collapsed.add(path);renderChrome();}else if(path)next=rows.findIndex(r=>r.path===path.replace(/[^/]+\/$/,''));break;
 case 'ArrowRight':if(row.hasChildren){if(s.collapsed.has(path)){s.collapsed.delete(path);renderChrome();}else next=index+1;}break;
 case 'Enter':event.preventDefault();event.stopPropagation();$('oa-tree').querySelector('[data-tree="'+CSS.escape(path)+'"]')?.click();focusTreePath(path);return;
 default:return;
 }
 event.preventDefault();event.stopPropagation();focusTreePath(rows[next]?.path??path);
}
function quickExtractLabel(){
 if(s.selected.size)return '解压选中';
 if(s.folder)return '解压当前目录…';
 if(s.search)return '解压到…';
 const recycle=s.deepExtract?s.deepRecycleAfter:s.recycleAfter;
 if(recycle)return s.deepExtract?'深度解压并回收…':'解压并回收…';
 return s.deepExtract?'深度解压':s.sameFolder?'智能解压':'解压到此处';
}
function showExtractMenu(){
 rememberMenuFocus();
 const menu=$('oa-menu');
 menu.innerHTML=`<button role="menuitem" data-action="extract-standard">解压到…</button>
 <button role="menuitem" data-action="extract-deep">深度解压…</button>
 <button role="menuitem" data-action="extract-recycle">解压并移入回收站…</button>`;
 positionMenu(menu,root.querySelector('[data-action=extract-menu]'));
 menu.querySelector('button')?.focus();
}
function renderChrome(){
 renderArchiveTabs();
 root.dataset.page=s.page;
 const browsing=s.page==='files'&&!!s.archive;
 $('oa-heading').hidden=true;
 root.querySelector('.oa-tree-section').hidden=!browsing;
 $('oa-toolbar').innerHTML=btn('打开','open','','folder-open')+btn('新建','compress','','plus')+(browsing?`<span class="oa-divider"></span>${btn('添加','add','','plus')}<span class="oa-spacer"></span><div class="oa-extract-split">${btn(quickExtractLabel(),'quick-extract','primary','folder-output')}<button class="qa-button primary" data-action="extract-menu" aria-haspopup="menu" aria-label="更多解压方式" title="更多解压方式">⌄</button></div>${btn('目录树','toggle-tree','quiet','folder')}`:'');
 renderFolderTree();
 updateToolbar();
}
function showMenu(group,anchor){rememberMenuFocus();const anchorRect=anchor?.closest?.('#oa-menu')?menuAnchorRect:(anchor?.getBoundingClientRect?.()||root.querySelector('[data-action=app-menu]').getBoundingClientRect());const items=menuGroups[group]||[];const box=$('oa-menu');box.innerHTML=`<div class="oa-submenu-head">${btn('返回','more-menu','quiet')}<strong>${({file:'文件',edit:'编辑',find:'查找',options:'选项',view:'视图',tools:'工具',help:'帮助'})[group]}</strong></div>`+items.map(it=>it?`<button ${['open-as-archive','external-edit'].includes(it[1])&&!selectedNestedMember()?'role="menuitem" disabled':['select-type','unselect-type'].includes(it[1])&&(!s.archive||!s.cursor)?'role="menuitem" disabled':it[1]==='toggle-flat'?'role="menuitemcheckbox" aria-checked="'+!!s.flatView+'"'+(!s.archive?' disabled':''):'role="menuitem"'} data-action="${it[1]}"><span>${it[0]}</span><small>${it[1]==='toggle-flat'&&s.flatView?'✓':it[2]}</small></button>`:'<hr>').join('');positionMenu(box,{getBoundingClientRect:()=>anchorRect});box.querySelector('button')?.focus();}
// File and menu keyboard behavior. Keep focus separate from the range anchor.
let listingWindow = null;
const entryPositions = new WeakMap();
function positionsFor(list) {
  let positions = entryPositions.get(list);
  if (!positions) { positions = new Map(); for(let i=0;i<list.length;i++)positions.set(list[i].name,i); entryPositions.set(list, positions); }
  return positions;
}
function rowButtons(){return [...root.querySelectorAll('[data-row] [data-entry]')];}
function focusEntry(name){
 const list=visibleEntries(),index=positionsFor(list).get(name);
 if(index === undefined)return;
 s.cursor=name;
 if(listingWindow)listingWindow.reveal(index);
 const buttons=rowButtons(),target=buttons.find(b=>b.dataset.entry===name);
 if(!target)return;
 for(const button of buttons)button.tabIndex=button===target?0:-1;
 target.focus({preventScroll:true});target.closest('[data-row]').scrollIntoView({block:'nearest',inline:'nearest'});
}
function fileKeyIndex(key,index,count,columns=1,pageRows=1){
 if(!count)return -1;
 const step=key==='ArrowUp'?-columns:key==='ArrowDown'?columns:key==='ArrowLeft'?-1:key==='ArrowRight'?1:key==='PageUp'?-columns*pageRows:key==='PageDown'?columns*pageRows:0;
 return key==='Home'?0:key==='End'?count-1:Math.max(0,Math.min(count-1,index+step));
}
// Match leaf names with the archive panel's two wildcards. Remember only the
// last star so repeated stars cannot cause recursive or exponential backtracking.
function nameMask(pattern){
 const mask=pattern.replace(/\*+/g,'*');
 if(mask==='*')return ()=>true;
 return name=>{
  let p=0,n=0,star=-1,retry=0;
  while(n<name.length){
   if(mask[p]==='*'){if(p===mask.length-1)return true;star=p++;retry=n;continue;}
   if(mask[p]==='?'||(p<mask.length&&mask[p].toUpperCase()===name[n].toUpperCase())){p++;n++;continue;}
   if(star===-1)return false;
   p=star+1;n=++retry;
  }
  while(mask[p]==='*')p++;
  return p===mask.length;
 };
}
function showSelectionMask(select){
 const title=select?'按名称选中':'按名称取消选中';
 dialog(title,'<label class="qa-field">名称模式<input id="oa-name-mask" value="*" aria-describedby="oa-mask-help"></label><p class="qa-sub" id="oa-mask-help">* 匹配任意多个字符，? 匹配一个字符。</p>',
  btn('取消','close')+'<button class="qa-button primary" id="oa-apply-mask">'+(select?'选中':'取消匹配项的选择')+'</button>');
 const input=$('oa-name-mask'),apply=$('oa-apply-mask');
 apply.addEventListener('click',()=>{
  const matches=nameMask(input.value);
  for(const item of visibleEntries()){
   if(matches(item.leaf)){if(select)s.selected.add(item.name);else s.selected.delete(item.name);}
  }
  close();refreshSelection();focusEntry(s.cursor);
 });
 input.addEventListener('keydown',event=>{
  if(event.key==='Enter'&&!isCompositionKey(event)){event.preventDefault();event.stopPropagation();apply.click();}
 });
 input.focus();input.select();
}
function selectFocusedType(select, name=s.cursor){
 const entries=visibleEntries(), focused=entries[positionsFor(entries).get(name)];
 if(!focused)return;
 const extension=entry=>{
  const dot=entry.leaf.lastIndexOf('.');
  return dot<0?'':entry.leaf.slice(dot).toLowerCase();
 };
 const wanted=focused.is_dir?'':extension(focused);
 for(const item of entries){
  if(item.is_dir!==focused.is_dir||(!item.is_dir&&extension(item)!==wanted))continue;
  if(select)s.selected.add(item.name);else s.selected.delete(item.name);
 }
 refreshSelection();
}
function handleFileKeydown(event){
 if(isCompositionKey(event)||!overlay.hidden||(s.active&&!s.backgroundExtraction))return;
 const row=event.target.closest('[data-row]');if(!row)return;
 const name=row.dataset.row,ctrl=event.ctrlKey||event.metaKey,key=event.key;
 if(event.altKey){
  if(!ctrl&&!event.shiftKey&&['NumpadAdd','NumpadSubtract'].includes(event.code)){
   event.preventDefault();event.stopPropagation();selectFocusedType(event.code==='NumpadAdd',name);
  }
  return;
 }
 if(!ctrl&&!event.shiftKey&&['NumpadAdd','NumpadSubtract'].includes(event.code)){
  event.preventDefault();event.stopPropagation();showSelectionMask(event.code==='NumpadAdd');return;
 }
 if(key==='Backspace'&&!ctrl&&!event.shiftKey){
  event.preventDefault();event.stopPropagation();
  if(s.folder)action('up').catch(error);
  return;
 }
 if(key==='ContextMenu'||(event.shiftKey&&key==='F10')){
  event.preventDefault();event.stopPropagation();focusEntry(name);
  if(!s.selected.has(name))selectRow(name,{});
  showMenu('edit',row);return;
 }
 if(key==='Enter'&&!ctrl){event.preventDefault();event.stopPropagation();entry(name).catch(error);return;}
 if(key===' '){
  event.preventDefault();event.stopPropagation();
  selectRow(name,event.shiftKey?{shiftKey:true,ctrlKey:ctrl}:{ctrlKey:true});return;
 }
 if(!['ArrowUp','ArrowDown','ArrowLeft','ArrowRight','Home','End','PageUp','PageDown'].includes(key))return;
 event.preventDefault();event.stopPropagation();
 const list=visibleEntries(),positions=positionsFor(list),index=positions.get(name);
 if(index===undefined)return;
 let columns=1;
 if(listingWindow?.columns)columns=listingWindow.columns;
 else if(s.listStyle==='icons'){const buttons=rowButtons(),top=buttons[0].getBoundingClientRect().top;columns=Math.max(1,buttons.filter(b=>Math.abs(b.getBoundingClientRect().top-top)<2).length);}
 const height=row.getBoundingClientRect().height||48,pageRows=Math.max(1,Math.floor(main.clientHeight/height)-1);
 const target=list[fileKeyIndex(key,index,list.length,columns,pageRows)].name;
 if(event.shiftKey&&!positions.has(s.anchor))s.anchor=name;
 if(ctrl&&!event.shiftKey)focusEntry(target);else selectRow(target,{shiftKey:event.shiftKey,ctrlKey:ctrl});
}
function handleMenuKeydown(event){
 if(isCompositionKey(event))return;
 const box=$('oa-menu');if(box.hidden||!box.contains(event.target))return;
 const items=[...box.querySelectorAll('button')].filter(b=>!b.disabled),index=items.indexOf(document.activeElement);
 if(event.key==='Escape'||event.key==='Tab'){
  if(event.key==='Escape')event.preventDefault();event.stopImmediatePropagation();event.stopPropagation();
  box.hidden=true;if(menuReturnFocus?.isConnected)menuReturnFocus.focus();return;
 }
 if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){
  event.preventDefault();event.stopImmediatePropagation();event.stopPropagation();
  const next=event.key==='Home'?0:event.key==='End'?items.length-1:index<0?(event.key==='ArrowUp'?items.length-1:0):(index+(event.key==='ArrowDown'?1:items.length-1))%items.length;
  items[next]?.focus();return;
 }
 if(event.key==='ArrowRight'&&document.activeElement?.dataset.menu){event.preventDefault();event.stopPropagation();document.activeElement.click();}
 if(event.key==='ArrowLeft'&&box.querySelector('.oa-submenu-head')){event.preventDefault();event.stopPropagation();showMore();}
}
// End file and menu keyboard behavior.
function selectRow(name,event){window.getSelection()?.removeAllRanges();const extend=event.ctrlKey||event.metaKey,list=visibleEntries(),positions=positionsFor(list),a=positions.get(s.anchor),b=positions.get(name);if(event.shiftKey&&a!==undefined&&b!==undefined){if(!extend)s.selected=new Set();for(let i=Math.min(a,b);i<=Math.max(a,b);i++)s.selected.add(list[i].name);}else if(extend){s.selected.has(name)?s.selected.delete(name):s.selected.add(name);s.anchor=name;}else{s.selected=new Set([name]);s.anchor=name;}refreshSelection();focusEntry(name);}
function refreshSelection(){for(const tr of root.querySelectorAll('[data-row]')){const selected=s.selected.has(tr.dataset.row);tr.classList.toggle('selected',selected);tr.setAttribute('aria-selected',String(selected));const box=tr.querySelector('[data-select]');if(box)box.checked=selected;}const list=visibleEntries();if($('qa-selection'))$('qa-selection').textContent=`${list.length} 个项目 · 已选 ${s.selected.size} 项`;if($('qa-all')){const positions=positionsFor(list);let count=0;for(const name of s.selected)if(positions.has(name))count++;$('qa-all').checked=list.length>0&&count===list.length;$('qa-all').indeterminate=count>0&&!$('qa-all').checked;}updateToolbar();}
function showTool(op,title){if($('qa-tools')?.contentWindow?.obzBusy){toast('请先等待或取消当前工具任务。');return;}s.toolOp=op;s.toolTitle=title;navigate('tool');}
let destinationRequest=0;
async function setDestination(path){if(!path)return;const input=$('qa-destination'),list=$('oa-destination-folders');if(!input||!list)return;const request=++destinationRequest;input.value=path;updateExtractPreview(true);const dirs=await invoke('list_directory',{path});if(request!==destinationRequest||input!==$('qa-destination')||list!==$('oa-destination-folders')||input.value!==path)return;list.innerHTML=dirs.map(e=>`<button data-directory="${esc(e.path)}">${icon('folder')} ${esc(e.name)}</button>`).join('')||'<p class="qa-sub">没有子文件夹</p>';}

const baseAction=action;
const backgroundActions=new Set(['canceljob','task-toggle','close','keep-running','cancel-exit','find','copy-path','root','up',
 'toggle-tree','toggle-flat','details','icons','reverse-sort','select-all','invert','select-mask','unselect-mask','select-type','unselect-type',
 'menu','app-menu','more-menu','shell-clear','files-home','reveal','reveal-output','about','help']);
action=async a=>{if(homeIntake&&a!=='close'&&a!=='exit'){return;}if(s.backgroundExtraction&&!backgroundActions.has(a)){toast('请先等待或取消当前任务。');return;}if($('qa-tools')?.contentWindow?.obzBusy&&!['keep-running','cancel-exit','close'].includes(a)){toast('请先等待或取消当前工具任务。');return;}
 if(/^extract-retry-\d+$/.test(a)){openExtractionRetry(Number(a.slice('extract-retry-'.length)));return;}
 switch(a){case'columns-scroll':main.scrollLeft=main.scrollLeft>0?0:main.scrollWidth;updateColumnScroll();return;case'columns':showColumns();return;case'columns-reset':columnDraft=ArchiveColumns.normalize(null);paintColumns();return;case'columns-apply':{columns=ArchiveColumns.normalize(columnDraft);localStorage.setItem('qing-columns',JSON.stringify(columns));for(const tab of archiveTabs){if(!columns.some(c=>c.visible&&c.id===tab.sort)){tab.sort='name';tab.reverse=false;}}if(!columns.some(c=>c.visible&&c.id===s.sort)){s.sort='name';s.reverse=false;}close();render();focusEntry(s.cursor);}return;case'associations':navigate('associations');if(s.page==='associations')await loadAssociations();return;case'associations-refresh':await loadAssociations();return;case'associations-save':await saveAssociations();return;case'default-apps':await invoke('open_default_apps');toast('请在 Windows 默认应用中选择 qing-zip，完成后返回刷新状态。');return;case'more-menu':case'app-menu':showMore();return;case'extract-recycle':extract({recycleOnce:true});return;case'all-tools':s.advancedTab='archive-tools';navigate('advanced');return;case'tools-home':navigate('tools');return;case'files-home':navigate('files');return;case'comment':{if(!s.archive)throw Error('请先打开 ZIP 压缩文件。');const r=await run({operation:'comment-get',path:s.archive},'读取注释',{silent:true});dialog('压缩文件注释',`<label class="qa-field">ZIP 注释<textarea id="oa-comment">${esc(r.comment)}</textarea></label>`,btn('取消','close')+btn('保存','comment-save','primary'));}return;case'comment-save':{const name=$('oa-comment').value;await run({operation:'comment-set',path:s.archive,name},'保存注释');}return;case'settings':navigate('settings');return;case'profiles':s.advancedTab='automation';navigate('advanced');return;case'batch':s.advancedTab='batch';navigate('advanced');return;case'close-archive':closeArchiveTabs('current');return;case'close-all-tabs':closeArchiveTabs('all');return;case'refresh-archive':if(s.archive)await openArchive(s.archive,s.password,{refresh:true});return;case'properties':await showProperties(false);return;case'archive-properties':await showProperties(true);return;case'save-nested':await saveNested();return;case'nested-open-submit':await retryNestedOpen();return;case'nested-close-save':await finishNestedClose(true);return;case'nested-close-discard':await finishNestedClose(false);return;case'exit':await invoke('window_action',{action:'close'});return;case'find':$('qa-search')?.focus();return;case'select-all':visibleEntries().forEach(e=>s.selected.add(e.name));if($('qa-rows'))refreshSelection();return;case'select-mask':case'unselect-mask':if(s.archive)showSelectionMask(a==='select-mask');return;case'select-type':case'unselect-type':selectFocusedType(a==='select-type');return;case'invert':visibleEntries().forEach(e=>s.selected.has(e.name)?s.selected.delete(e.name):s.selected.add(e.name));if($('qa-rows'))refreshSelection();return;case'toggle-flat':if(!s.archive||(tabBusy()&&!s.backgroundExtraction))return;s.flatView=!s.flatView;{const available=positionsFor(visibleEntries());s.selected=new Set([...s.selected].filter(name=>available.has(name)));if(!available.has(s.anchor))s.anchor=null;if(!available.has(s.cursor))s.cursor=null;}s.page='files';s.view='archive';render();main.scrollTop=0;return;case'toggle-tree':root.dataset.tree=root.dataset.tree==='hidden'?'shown':'hidden';updateToolbar();return;case'toggle-status':root.dataset.status=root.dataset.status==='hidden'?'shown':'hidden';return;case'toolbar-large':case'toolbar-small':case'toolbar-hidden':root.dataset.toolbar=a.slice(8);return;case'details':case'icons':s.listStyle=a;saveViewPreference();render();focusEntry(s.cursor);return;case'reverse-sort':s.reverse=!s.reverse;rows();syncViewControls();return;case'external-copies':await showRetainedEdits();return;case'external-retained-open':await invoke('reveal_path',{path:retainedEditRecords[Number($('oa-retained-edit').value)].path});return;case'external-retained-save':{const path=retainedEditRecords[Number($('oa-retained-edit').value)].path,dest=await invoke('pick_copy_save',{path});if(dest){await invoke('copy_archive',{path,dest});toast('已另存为 '+dest);}}return;case'external-edit':await beginExternalEdit();return;case'external-editor-default':$('oa-editor-path').value='';return;case'external-editor-pick':{const path=await invoke('pick_editor');if(path)$('oa-editor-path').value=path;}return;case'external-launch':await launchExternalEdit();return;case'external-sessions':await showExternalEdits();return;case'external-copy':if(externalCurrent){const dest=await invoke('pick_copy_save',{path:externalCurrent.path});if(dest){await invoke('copy_archive',{path:externalCurrent.path,dest});toast('已另存为 '+dest);}}return;case'external-save-confirm':confirmExternalFinish(true);return;case'external-discard-confirm':confirmExternalFinish(false);return;case'external-save':await finishExternalEdit(true);return;case'external-discard':await finishExternalEdit(false);return;case'open-as-archive':{const member=selectedNestedMember();if(!member)throw Error('请选中一个文件。');await openNestedEntry(member);}return;case'preview-selected':if(s.selected.size!==1)throw Error('请选中一个文件或文件夹。');await entry([...s.selected][0],{preview:true});return;case'codepage':dialog('文件名代码页',`<label class="qa-field">编码<select id="oa-codepage">${opts([['','自动检测'],['65001','UTF-8'],['936','简体中文 GBK'],['950','繁体中文 Big5'],['949','韩文'],['932','日文 Shift-JIS'],['437','OEM 437']],s.codepage||'')}</select></label>`,btn('取消','close')+btn('应用','codepage-apply','primary'));return;case'codepage-apply':s.codepage=$('oa-codepage').value?Number($('oa-codepage').value):null;await openArchive(s.archive,s.password,{refresh:true});return;case'recent-menu':showRecentDialog();return;case'save-as':if(!s.archive)throw Error('请先打开压缩文件。');{if(/\.\d{3,}$/.test(s.archive))throw Error('分卷请先解压后重新压缩，以免遗漏其他卷。');const dest=await invoke('pick_copy_save',{path:s.archive});if(dest){await invoke('copy_archive',{path:s.archive,dest});toast('已另存为 '+dest);}}return;case'reveal-output':{const path=s.jobs.at(-1)?.outputPath;if(path)await invoke('reveal_path',{path});}return;case'reveal':if(s.archive)await invoke('reveal_path',{path:s.archive});return;case'recycle-archive':if(s.archive)dialog('删除压缩文件',`<p>将 ${esc(filename(s.archive))} 移到回收站？</p>`,btn('取消','close')+btn('移到回收站','recycle-confirm','danger'));return;case'recycle-confirm':if(s.nestedId||archiveTabs.some(t=>archiveKey(t.parentArchive||'')===archiveKey(s.archive)))throw Error('请先保存并关闭内层标签，再删除外层压缩包。');await invoke('recycle_paths',{paths:[s.archive]});close();await action('close-archive');return;case'copy-path':await navigator.clipboard.writeText([...s.selected].map(n=>s.archive+' / '+n).join('\n')||s.archive);toast('已复制路径');return;case'paste-files':{const paths=await invoke('clipboard_paths',{paths:null});if(!paths.length)throw Error('剪贴板中没有文件。');showAddForm(paths);}return;case'copy-files':await copySelectedFiles();return;case'vault-tool':showTool('vault-list','密码管理器');return;case'repair-tool':showTool('repair','修复 ZIP 压缩包');return;case'recover-tool':showTool('recover','密码恢复');return;case'checksum-tool':showTool('checksum','文件校验值');return;case'about':dialog('关于 qing-zip',`<p>qing-zip ${esc(appVersion)}</p><p class="qa-sub">本地压缩与解压工具。第三方组件许可随程序附带。</p>`,btn('关闭','close'));return;case'help':dialog('使用说明','<p>打开压缩包后，单击选择条目，双击进入文件夹或查看文件。Ctrl / Shift 可多选。</p><p>方向键移动，Shift+方向键连续选择，空格勾选，回车打开，Shift+F10 打开右键菜单。</p><p>Ctrl+N 新建，Ctrl+E 解压，Ctrl+T 测试，F2 重命名，Del 删除包内条目。</p><p>Ctrl+Tab 切换标签，Ctrl+Shift+Tab 反向切换，Ctrl+W 关闭当前标签，Ctrl+R 刷新当前标签。标签右键可关闭其他或全部。</p><p>文本双击只读预览；图片双击用系统默认看图程序打开原图。大图标模式可用 Ctrl+滚轮调整尺寸。</p><p>工具菜单提供修复、密码恢复、校验值和批量处理。</p>',btn('关闭','close'));return;}return baseAction(a);};
root.addEventListener('dblclick',e=>{if(e.target.closest('input'))return;const row=e.target.closest('[data-row]');if(row)entry(row.dataset.row).catch(error);});
let lastSelectShift=false,dragCandidate=null,dragBusy=false,suppressRowClick=false;
root.addEventListener('mousedown',e=>{if(e.target.closest('[data-row]')&&!e.target.closest('input'))e.preventDefault();});
root.addEventListener('dragstart',e=>{if(e.target.closest('[data-row]'))e.preventDefault();});
root.addEventListener('click',e=>{if(suppressRowClick&&e.target.closest('[data-row]')){e.preventDefault();e.stopImmediatePropagation();suppressRowClick=false;}},true);
let dragGesture=null;
root.addEventListener('pointerdown',e=>{
  lastSelectShift=e.shiftKey;
  if(e.button!==0||s.active||dragBusy||!overlay.hidden)return;
  const row=e.target.closest('[data-row]');
  if(row&&!e.target.closest('input')&&!e.shiftKey&&!e.ctrlKey&&!e.metaKey)
    dragCandidate={name:row.dataset.row,x:e.clientX,y:e.clientY,pointerId:e.pointerId};
});
function cancelPreparingDrag(pointerId) {
  if(dragCandidate&&(!Number.isInteger(pointerId)||dragCandidate.pointerId===pointerId))dragCandidate=null;
  const gesture=dragGesture;
  if(!gesture||gesture.cancelled||(Number.isInteger(pointerId)&&gesture.pointerId!==pointerId))return;
  gesture.cancelled=true;
  invoke('cancel_drag_preparation',{token:gesture.token}).catch(()=>{});
}
window.addEventListener('pointerup',e=>cancelPreparingDrag(e.pointerId),true);
window.addEventListener('pointercancel',e=>cancelPreparingDrag(e.pointerId),true);
// Explorer may take focus or mouse capture while the user is still holding
// the button over a target. The native monitor continues across that boundary.
root.addEventListener('lostpointercapture',()=>{dragCandidate=null;});
window.addEventListener('blur',()=>{dragCandidate=null;});
window.addEventListener('keydown',e=>{
  if(!isCompositionKey(e)&&e.key==='Escape'&&dragGesture){cancelPreparingDrag();e.preventDefault();e.stopImmediatePropagation();}
},true);
root.addEventListener('pointermove',e=>{
  const c=dragCandidate;
  if(!c||c.pointerId!==e.pointerId||!(e.buttons&1)||Math.hypot(e.clientX-c.x,e.clientY-c.y)<6)return;
  dragCandidate=null;suppressRowClick=true;
  if(!s.selected.has(c.name))selectRow(c.name,{});
  dragSelection(c.pointerId).catch(error).finally(()=>{dragBusy=false;setTimeout(()=>suppressRowClick=false,300);});
});
async function dragSelection(pointerId) {
  if(dragBusy||s.active)return;
  dragBusy=true;
  const names=[...s.selected],path=s.archive,password=s.password||null,codepage=s.codepage||null;
  if(names.length>512)throw Error('一次最多拖出 512 个文件或文件夹。');
  const gesture={token:crypto.randomUUID(),pointerId,cancelled:false};
  dragGesture=gesture;
  let jobId;
  const ownJob=()=>s.jobs.find(item=>item.id===jobId);
  const status=(state,summary)=>{
    const job=ownJob();
    if(job){job.status=state;job.summary=summary;if(s.view==='jobs')renderProgress();}
  };
  const releaseCapture=()=>{
    try{if(root.hasPointerCapture(pointerId))root.releasePointerCapture(pointerId);}catch{}
  };
  try {
    // Capture covers release outside the list. The native monitor also covers
    // leaving WebView2 or losing focus while the start request is in flight.
    try{root.setPointerCapture(pointerId);}catch{}
    toast('正在准备拖出文件，松开鼠标或按 Esc 取消。');
    const startJob=async request=>{
      const id=await invoke('start_drag_preparation',{token:gesture.token,request});
      jobId=id;
      // A quick release may have arrived before native registration. Replay
      // only this gesture's cancellation, even if another job is now active.
      if(gesture.cancelled)await invoke('cancel_drag_preparation',{token:gesture.token}).catch(()=>{});
      return id;
    };
    await run({operation:'extract',path,options:{selected:names,overwrite:'error',password,codepage}},
      '准备拖出文件',{silent:true,startJob,temporaryOutput:true});
    releaseCapture();
    const copied=!gesture.cancelled&&await invoke('finish_drag',{token:gesture.token});
    const summary=copied?'文件已交给目标文件夹复制。':'已取消拖放。';
    status(copied?'complete':'cancelled',summary);
    toast(summary);
  } catch(e) {
    if(!gesture.cancelled&&ownJob()?.status!=='cancelled'&&!/operation cancelled|已取消拖放/i.test(String(e))){status('failed',String(e));throw e;}
    status('cancelled','已取消拖放。');
    toast('已取消拖放。');
  } finally {
    releaseCapture();
    if(dragGesture===gesture)dragGesture=null;
    await invoke('discard_drag',{token:gesture.token}).catch(()=>{});
  }
}
const scrollTimers=new WeakMap();root.addEventListener('scroll',e=>{const el=e.target;if(!(el instanceof HTMLElement))return;el.classList.add('oa-scrolling');clearTimeout(scrollTimers.get(el));scrollTimers.set(el,setTimeout(()=>el.classList.remove('oa-scrolling'),900));},true);
root.addEventListener('click',e=>{if(e.target.closest('#oa-menu')||e.target.closest('[data-menu]')||e.target.closest('[data-action=more-menu],[data-action=extract-menu]'))return;$('oa-menu').hidden=true;const row=e.target.closest('[data-row]');if(row&&!e.target.closest('button,input'))selectRow(row.dataset.row,e);});
root.addEventListener('change',e=>{const t=e.target,c=columnDraft?.find(c=>c.id===(t.dataset.columnVisible||t.dataset.columnWidth));if(!c)return;if(t.dataset.columnVisible)c.visible=t.checked;if(t.dataset.columnWidth){c.width=Math.max(64,Math.min(800,Number(t.value)||c.width));t.value=c.width;}});
root.addEventListener('click',e=>{const b=e.target.closest('[data-column-move]');if(!b)return;columnDraft=ArchiveColumns.move(columnDraft,b.dataset.columnMove,Number(b.dataset.delta));paintColumns();$('oa-column-list').querySelector(`[data-column-move="${b.dataset.columnMove}"][data-delta="${b.dataset.delta}"]`)?.focus();});
root.addEventListener('contextmenu',e=>{const recent=e.target.closest('[data-recent]');if(recent){e.preventDefault();showRecentContext(recent,e);return;}const tab=e.target.closest('[data-tab-container]');if(tab){e.preventDefault();tab.querySelector('[data-archive-tab]')?.focus();showArchiveTabMenu(tab.dataset.tabContainer,e);return;}const row=e.target.closest('[data-row]');if(row){e.preventDefault();focusEntry(row.dataset.row);if(!s.selected.has(row.dataset.row))selectRow(row.dataset.row,{});showMenu('edit',{getBoundingClientRect:()=>({left:e.clientX,bottom:e.clientY})});}});
document.addEventListener('keydown',e=>{if(e.defaultPrevented||isCompositionKey(e)||!overlay.hidden)return;const key=e.key.toLowerCase(),ctrl=e.ctrlKey||e.metaKey;if(ctrl&&key==='tab'){e.preventDefault();cycleArchiveTab(e.shiftKey?-1:1);return;}if(ctrl&&key==='w'){e.preventDefault();closeArchiveTabs('current');return;}if(['INPUT','TEXTAREA','SELECT'].includes(e.target.tagName))return;const map=ctrl?{o:'open',n:'compress',e:'extract',t:'test',w:'close-archive',r:'refresh-archive',f:'find',a:'select-all',c:s.backgroundExtraction?'copy-path':'copy-files',v:'paste-files',s:'save-nested'}:{f1:'about',f2:'rename',f4:'close-archive',f5:'settings',f6:'close-archive',f7:'toggle-tree',delete:'delete',insert:'add',backspace:'up'};const a=e.altKey&&!ctrl&&key==='enter'?'properties':ctrl&&e.shiftKey&&key==='m'?'comment':ctrl&&key==='delete'?'recycle-archive':map[key];if(a&&s.page!=='files'&&!['open','compress','exit','close-archive','settings','about','toggle-tree'].includes(a))return;if(a){e.preventDefault();action(a).catch(error);}});
root.addEventListener('click',async e=>{const b=e.target.closest('button');if(!b)return;try{if(b.dataset.archiveTab!==undefined){switchArchiveTab(b.dataset.archiveTab);return;}if(b.dataset.tabClose!==undefined){closeArchiveTabs('one',b.dataset.tabClose);return;}if(b.dataset.tabCloseOthers!==undefined){closeArchiveTabs('others',b.dataset.tabCloseOthers);return;}if(b.dataset.menu){showMenu(b.dataset.menu,b);return;}if(b.dataset.treeToggle!==undefined){const p=b.dataset.treeToggle;s.collapsed.has(p)?s.collapsed.delete(p):s.collapsed.add(p);renderChrome();focusTreePath(p);return;}if(b.dataset.breadcrumb!==undefined){gotoArchiveFolder(b.dataset.breadcrumb);root.querySelector('.oa-breadcrumb button:last-child')?.focus();return;}if(b.dataset.tree!==undefined){if((s.active&&!s.backgroundExtraction)||$('qa-tools')?.contentWindow?.obzBusy)throw Error('请先等待或取消当前任务。');s.folder=b.dataset.tree;s.search='';s.selected.clear();navigate('files');focusTreePath(b.dataset.tree);return;}if(b.dataset.compressionPage){const job=s.jobs.find(job=>job.id===b.dataset.compressionPage);if(job){job.resultPage=Number(b.dataset.resultPage);if(s.page==='history')render();else renderInlineTask();}return;}if(b.dataset.compressionDraft){const job=s.jobs.find(job=>job.id===b.dataset.compressionDraft);if(job)restoreCompressionDraft(job);return;}if(b.dataset.compressionOutput){await useCompressionResult(b);return;}if(b.dataset.jobOutput){const j=s.jobs.find(j=>String(j.id)===b.dataset.jobOutput),path=j?.outputPath;if(path)await invoke('reveal_path',{path});return;}if(b.dataset.launchTool){if(s.active)throw Error('请先等待或取消当前任务。');showTool(b.dataset.launchTool,b.dataset.toolTitle);return;}if(b.dataset.place){await setDestination(s.known?.[b.dataset.place]||'');return;}if(b.dataset.directory){await setDestination(b.dataset.directory);return;}if(b.dataset.window){await invoke('window_action',{action:b.dataset.window});return;}if(b.dataset.view){navigate(b.dataset.view);return;}if(b.dataset.theme){s.theme=b.dataset.theme;appearance();return;}if(b.dataset.recent!==undefined){const path=s.recent[Number(b.dataset.recent)];try{await openArchive(path);}catch(e){openDialog(path);error(e);}return;}if(b.dataset.entry){selectRow(b.dataset.entry,e);return;}if(b.dataset.sort){s.reverse=s.sort===b.dataset.sort?!s.reverse:false;s.sort=b.dataset.sort;rows();syncViewControls();return;}if(b.dataset.advanced){if($('qa-tools')?.contentWindow?.obzBusy)throw Error('请等待当前工具任务完成。');s.advancedTab=b.dataset.advanced;render();return;}if(b.dataset.tool){if($('qa-tools')?.contentWindow?.obzBusy)throw Error('请等待当前工具任务完成。');$('qa-tools').src=`tools.html?operation=${b.dataset.tool}&path=${encodeURIComponent(s.archive)}&codepage=${s.codepage||''}`;return;}if(b.dataset.profile!==undefined){compress(s.profiles[Number(b.dataset.profile)]);return;}if(b.dataset.profileDelete!==undefined){s.profiles.splice(Number(b.dataset.profileDelete),1);localStorage.setItem('qing-profiles',JSON.stringify(s.profiles));render();return;}if(b.dataset.action){$('oa-menu').hidden=true;await action(b.dataset.action)};}catch(e){error(e);}});
root.addEventListener('input',e=>{if(e.target.id==='oa-icon-size')setIconSize(Number(e.target.value));if(['qa-inputs','qa-edit-inputs'].includes(e.target.id))refreshSources();if(e.target.id==='qa-save'){e.target.dataset.explicit=String(!!e.target.value.trim());clearCompressionPlan();}if(['qa-new-password','oa-password-again'].includes(e.target.id))syncCompressionPassword();if(e.target.id==='qa-volume')clearCompressionPlan();if(e.target.id==='qa-destination')updateExtractPreview(true);if(e.target.id==='qa-search'&&!e.isComposing)updateArchiveSearch(e.target);});
root.addEventListener('change',e=>{const t=e.target;if(t.id==='oa-add-preserve-atime'){window.QingFileWorkspace.preservePreference(t.checked);return;}if(timePreferenceIds.includes(t.id)){saveTimePreferences();return;}if(t.id==='oa-file-menu'){t.disabled=true;invoke('set_file_compression_menu',{enabled:t.checked}).then(v=>{fileMenuEnabled=v;t.checked=v;}).catch(e=>{t.checked=fileMenuEnabled;error(e);}).finally(()=>t.disabled=false);return;}if(t.id==='oa-folder-menu'){t.disabled=true;invoke('set_folder_menu',{enabled:t.checked}).then(v=>{folderMenuEnabled=v;t.checked=v;}).catch(e=>{t.checked=folderMenuEnabled;error(e);}).finally(()=>t.disabled=false);return;}if(t.id==='oa-deep-extract'){const box=$('oa-recycle-after');box.dataset.wanted=String($('oa-remember-extract')?(t.checked?s.deepRecycleAfter:s.recycleAfter):box.dataset.wanted==='true');deepExtract.sync();syncRecycleOption();}if(t.id==='oa-recycle-after'){t.dataset.wanted=String(t.checked);syncRecycleOption();}if(t.id==='qa-smart')updateExtractPreview();if(t.id==='oa-text-encoding'){loadTextPreview($('qa-file-preview'),t.value).catch(error);}if(t.id==='oa-search-scope'){s.searchScope=t.value;applyArchiveFilter();}if(t.id==='oa-sort'){s.sort=t.value;rows();syncViewControls();}if(t.dataset.association!==undefined){t.checked?associationSelection.add(t.dataset.association):associationSelection.delete(t.dataset.association);}if(t.name==='extract-scope'||t.id==='qa-conflict')syncRecycleOption();if(t.id==='qa-format')changeCompressionFormat();if(t.id==='qa-save')changeCompressionSave(t);if(t.id==='oa-separate')changeCompressionMode();if(t.dataset.sourceIndex!==undefined)syncSourceSelection();if(['oa-method','oa-solid','qa-level'].includes(t.id))syncCompressionControls();if(t.id==='oa-show-password'){for(const id of ['qa-new-password','oa-password-again'])$(id).type=t.checked?'text':'password';}if(t.dataset.select){if(lastSelectShift)selectRow(t.dataset.select,{shiftKey:true});else{t.checked?s.selected.add(t.dataset.select):s.selected.delete(t.dataset.select);s.anchor=t.dataset.select;refreshSelection();}lastSelectShift=false;}if(t.id==='qa-all'){visibleEntries().forEach(v=>t.checked?s.selected.add(v.name):s.selected.delete(v.name));refreshSelection();}const k={'qa-theme':'theme','qa-material':'material','qa-density':'density','qa-defaultconflict':'overwrite'}[t.id];if(k){s[k]=t.value;appearance();}});
root.addEventListener('compositionend',event=>{if(event.target.id==='qa-search')updateArchiveSearch(event.target);});
function isCompositionKey(event){return event.isComposing||event.keyCode===229;}
function handleDialogKeydown(e){if(isCompositionKey(e))return;if(e.key==='Escape'){const menu=$('oa-menu');if(!menu.hidden){e.preventDefault();menu.hidden=true;if(menuReturnFocus?.isConnected)menuReturnFocus.focus();}else if(!overlay.hidden)close();}if(e.key==='Tab'&&!overlay.hidden){const list=[...overlay.querySelectorAll('input,textarea,select,button,summary,[tabindex]')].filter(v=>!v.disabled&&v.tabIndex>=0&&v.getClientRects().length);if(e.shiftKey&&document.activeElement===list[0]){e.preventDefault();list.at(-1)?.focus();}else if(!e.shiftKey&&document.activeElement===list.at(-1)){e.preventDefault();list[0]?.focus();}}}
root.addEventListener('keydown',handleDialogKeydown);
async function syncWindowControls(){try{const maximized=await invoke('window_maximized'),button=root.querySelector('[data-window=maximize]');button.setAttribute('aria-label',maximized?'还原窗口':'最大化窗口');button.title=maximized?'还原':'最大化';button.innerHTML=`<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" aria-hidden="true">${maximized?'<path d="M6 3.5h6.5V10M10 6V3.5"/><rect x="3.5" y="6" width="6.5" height="6.5" rx=".5"/>':'<rect x="3.5" y="3.5" width="9" height="9" rx=".5"/>'}</svg>`;}catch{}}
const maximizeButton=root.querySelector('[data-window=maximize]');
let snapBounds='';
function syncSnapButton(){
  const {left,top,right,bottom}=maximizeButton.getBoundingClientRect();
  const bounds={left,top,right,bottom,viewportWidth:window.innerWidth};
  const key=JSON.stringify(bounds);
  if(key===snapBounds)return;
  snapBounds=key;
  invoke('set_snap_button_bounds',{bounds}).catch(err=>{
    if(snapBounds===key)snapBounds='';
    console.warn(err);
  });
}
const snapObserver=new ResizeObserver(syncSnapButton);
snapObserver.observe(maximizeButton);
window.addEventListener('resize',()=>{syncSnapButton();syncWindowControls();});
window.__TAURI__.event.listen('snap-button-state',event=>{
  maximizeButton.dataset.nativeState=event.payload;
}).catch(console.warn);
syncWindowControls();
root.querySelector('.qa-titlebar').addEventListener('pointerdown',e=>{if(e.button===0&&!e.target.closest('button'))invoke('window_action',{action:'drag'}).catch(error);});
root.querySelector('.qa-titlebar').addEventListener('dblclick',e=>{if(!e.target.closest('button'))invoke('window_action',{action:'maximize'}).catch(error);});
window.__TAURI__.event.listen('close-requested',()=>{if(!tabBusy()){closeArchiveTabs('exit');return;}if($('oa-exit-confirm'))return;s.returnPage=overlay.dataset.page;overlay.dataset.page='close-confirm';overlay.hidden=false;root.querySelector('.qa-shell').inert=true;if($('oa-task-panel'))$('oa-task-panel').inert=true;overlay.querySelectorAll('.qa-dialog').forEach(e=>e.inert=true);const box=document.createElement('section');box.id='oa-exit-confirm';box.className='qa-dialog';box.style.position='absolute';box.style.zIndex='40';box.innerHTML='<h2>任务正在进行</h2><p>取消当前任务并关闭？已完成的输出会保留。</p><div class="qa-dialog-footer">'+btn('继续任务','keep-running')+btn('取消任务并退出','cancel-exit','danger')+'</div>';overlay.append(box);box.querySelector('button').focus();}).catch(error);

function canRecycleArchive(path,overwrite,partial=false){return !partial&&overwrite!=='skip'&&!/\.(?:\d{3,}|z\d{2,}|r\d{2,})$|\.part\d+\.rar$/i.test(path)&&/\.(zip|zipx|7z|tar|gz|gzip|bz2|bzip2|xz|tgz|tbz|tbz2|txz)$/i.test(path);}
function syncRecycleOption(){
 const box=$('oa-recycle-after');
 if(!box)return;
 const partial=extractSelection().length>0,deep=$('oa-deep-extract').checked,overwrite=$('qa-conflict').value;
 const nested=!!s.nestedId||archiveTabs.some(tab=>archiveKey(tab.parentArchive||'')===archiveKey(s.archive));
 box.disabled=nested||(deep?(partial||overwrite==='skip'):!canRecycleArchive(s.archive,overwrite,partial));
 box.checked=!box.disabled&&box.dataset.wanted==='true';
 const note=$('oa-recycle-once-note');
 if(note)note.textContent=box.disabled?'当前范围或格式无法回收，原压缩包会保留。':'仅应用于此次解压。';
 box.closest('label').title=nested?'请先保存并关闭内层标签':box.disabled?'仅支持完整解压且不跳过已有文件的受支持格式':'';
 const submit=overlay.querySelector('[data-action=startextract]');
 submit.lastChild.textContent=box.checked?(deep?'深度解压并回收':'解压并回收'):deep?'开始深度解压':'开始解压';
 const disposition=$('oa-extract-source-status');
 if(disposition){
  disposition.textContent=sourceDisposition(box.checked,{blocked:box.disabled,deep});
  disposition.dataset.recycle=String(box.checked);
 }
 updateExtractPreview();
}
function refreshJobContent(html){const content=$('oa-dialog-content');if(!content)return false;const focused=document.activeElement,action=content.contains(focused)?focused.dataset.action:null;content.innerHTML=html;if(action){const target=Array.from(content.querySelectorAll('[data-action]')).find(b=>b.dataset.action===action&&!b.disabled)||content.querySelector('[data-action="reveal-output"]')||overlay.querySelector('.qa-dialog-footer [data-action="close"]');if(target){if(target.dataset.action==='close')target.disabled=!!s.active;if(!target.disabled)target.focus({preventScroll:true});}}return true;}
function renderProgress(){renderInlineTask();if(overlay.dataset.page==='close-confirm')return;if(s.view!=='jobs')return;if(!refreshJobContent(jobs()))render();overlay.querySelectorAll('.qa-dialog-footer [data-action=close]').forEach(b=>b.disabled=!!s.active);$('qa-status-text').textContent=s.active?'正在处理文件…':'就绪';updateToolbar();}
const toolCatalog=[['深度解压',[['deep-batch','批量深度解压','自动识别并展开多层压缩包','folder-output']]],['归档维护',[['test','完整性检查','检查文件是否完整','shield'],['repair','修复 ZIP 压缩包','恢复受损 ZIP 中可读取的内容','archive'],['checksum','文件校验值','计算 SHA、BLAKE、CRC 与更多校验值','check'],['preview','图片预览','查看压缩包内的图片','file']]],['效率与安全',[['file-split','分割文件','将大文件分成多个独立小文件','file'],['file-combine','合并文件','按卷序还原分割的文件','file'],['benchmark','引擎基准测试','测量 LZMA 压缩和解压能力','grid'],['batch','批量处理','为多个压缩文件执行同一操作','grid'],['profiles','压缩预设','保存并复用常用压缩参数','settings'],['vault-list','密码管理器','管理本机保存的归档密码','shield'],['recover','密码恢复','按候选密码或字符集有限尝试，不保证成功','archive']]],['更多',[['associations','文件关联设置','选择文件格式，并前往 Windows 设置默认应用','settings'],['all','全部归档操作','列表、创建、解压、更新、注释等','grid'],['shell-enable','资源管理器右键入口','添加或移除文件右键操作','settings'],['capabilities','支持能力','查看当前引擎与格式支持情况','file']]]];
function toolbox(){return `<div class="qa-heading"><h1>工具箱</h1></div><button class="oa-toolrow" data-view="filemanager">${icon('folder-open')}<span><strong>文件管理</strong><small>在两个位置之间复制、移动、添加或解压</small></span>${icon('chevron')}</button>${toolCatalog.map(([group,items])=>`<section class="oa-toolgroup"><h3>${group}</h3>${items.map(([op,title,desc,ico])=>`<button class="oa-toolrow" ${op==='deep-batch'||op==='batch'||op==='profiles'||op==='associations'?`data-action="${op}"`:op==='all'?'data-action="all-tools"':`data-launch-tool="${op}" data-tool-title="${title}"`}>${icon(ico)}<span><strong>${title}</strong><small>${desc}</small></span>${icon('chevron')}</button>`).join('')}</section>`).join('')}`;}
function toolPage(){return `<div class="oa-page-back">${btn('工具箱','tools-home','quiet','arrow-up')}</div><div class="qa-heading"><h1>${esc(s.toolTitle||'归档工具')}</h1></div><iframe id="qa-tools" title="${esc(s.toolTitle||'归档工具')}" class="qa-tool-frame" src="tools.html?operation=${encodeURIComponent(s.toolOp||'list')}&path=${encodeURIComponent(s.archive)}&codepage=${s.codepage||''}"></iframe>`;}
let associationEntries=null,associationSelection=new Set(),associationBusy=false,associationError='';
function associationPage(){return `<div class="qa-heading"><div><h1>文件关联</h1><p>让 qing-zip 出现在这些文件的打开方式中。</p></div>${btn('返回设置','settings','quiet')}</div><section class="qa-setting-group"><p class="qa-sub">勾选并保存会注册可用的打开方式；当前默认应用由 Windows 管理，需要你在系统设置中确认。取消勾选仅移除本程序的格式注册，不替你更改其他应用或系统默认选择。</p><div class="qa-actions">${btn('在 Windows 中设为默认','default-apps')}${btn('刷新状态','associations-refresh','quiet')}</div></section><div id="oa-association-status" role="status">${esc(associationError|| (associationBusy?'正在读取或保存文件关联…':''))}</div>${associationEntries?`<div class="oa-association-table"><table><thead><tr><th>注册打开方式</th><th>本程序已注册</th><th>当前默认应用</th></tr></thead><tbody>${associationEntries.map(e=>`<tr><td><label class="qa-check"><input type="checkbox" data-association="${esc(e.extension)}" ${associationSelection.has(e.extension)?'checked':''} ${associationBusy?'disabled':''}>${esc(e.extension.toUpperCase())}</label></td><td>${e.registered?'已注册':'未注册'}</td><td title="${esc(e.current_progid||'尚未设置默认应用')}">${e.is_default?'qing-zip':'其他应用或未设置'}</td></tr>`).join('')}</tbody></table></div><div class="qa-actions oa-spaced"><button class="qa-button primary" data-action="associations-save" ${associationBusy?'disabled':''}>保存所选格式</button><span class="qa-sub">保存后仍可在 Windows 中确认默认应用。</span></div>`:!associationBusy?'<p class="qa-sub">无法读取文件关联，请刷新重试。</p>':''}`;}
async function loadAssociations(){if(associationBusy)return;associationBusy=true;associationError='';if(s.page==='associations')render();try{associationEntries=await invoke('association_status');associationSelection=new Set(associationEntries.filter(e=>e.registered).map(e=>e.extension));}catch(e){associationError=String(e);throw e;}finally{associationBusy=false;if(s.page==='associations')render();}}
async function saveAssociations(){if(associationBusy||!associationEntries)return;associationBusy=true;associationError='';const extensions=[...associationSelection];if(s.page==='associations')render();try{associationEntries=await invoke('set_file_associations',{extensions});associationSelection=new Set(associationEntries.filter(e=>e.registered).map(e=>e.extension));toast('已保存格式注册。需要更改默认打开方式时，请在 Windows 设置中确认。');}catch(e){associationError=String(e);throw e;}finally{associationBusy=false;if(s.page==='associations')render();}}
function historyPage(){return `<div class="qa-heading"><div><h1>任务</h1><p>本次会话的压缩、解压与检查记录。</p></div></div>${s.jobs.length?s.jobs.slice().reverse().map(jobCard).join(''):'<div class="qa-empty">还没有任务记录。</div>'}`;}
function rememberMenuFocus(){const box=$('oa-menu');if(!box.contains(document.activeElement))menuReturnFocus=document.activeElement;}
function positionMenu(box,anchor){const r=anchor?.getBoundingClientRect?.()||$('oa-toolbar').getBoundingClientRect();menuAnchorRect={left:r.left,bottom:r.bottom};box.hidden=false;box.scrollTop=0;const bounds=box.getBoundingClientRect();box.style.left=Math.max(8,Math.min(r.left,innerWidth-bounds.width-12))+'px';box.style.top=Math.max(8,Math.min(r.bottom+5,innerHeight-bounds.height-12))+'px';}
function showMore(){rememberMenuFocus();const box=$('oa-menu');box.innerHTML='<div class="oa-menu-title">菜单</div><nav id="oa-menubar" aria-label="所有菜单">'+[['file','文件'],['edit','编辑'],['find','查找'],['options','选项'],['view','视图'],['tools','工具'],['help','帮助']].map(([k,l])=>`<button role="menuitem" data-menu="${k}" aria-haspopup="menu"><span>${l}</span>${icon('chevron')}</button>`).join('')+'</nav>';positionMenu(box,root.querySelector('[data-action=app-menu]'));box.querySelector('button')?.focus();}


// Thumbnail requests are visible-only and share a bounded worker pool. Responses
// from an older archive generation never paint the current document.
const IMAGE_EXT=/\.(png|jpe?g|gif|webp|bmp|tiff?|ico|avif)$/i;
const TEXT_EXT=/\.(txt|md|markdown|log|csv|tsv|json|jsonl|xml|yaml|yml|toml|ini|cfg|conf|config|properties|html?|css|js|mjs|cjs|ts|tsx|jsx|rs|py|java|c|h|cpp|hpp|cs|sql|sh|bat|ps1|svg|srt|ass|vtt|tex|vue|go|kt|dart|rb|php)$/i;
const isImage=name=>IMAGE_EXT.test(name),isText=name=>TEXT_EXT.test(name)||/^(readme|license|notice|changelog|authors|makefile|dockerfile|\.gitignore|\.editorconfig|\.env)$/i.test(filename(name));
const previewArgs=name=>({path:s.archive,name,password:s.password||null,codepage:s.codepage||null});
let previewGeneration=0,openingImage=false,thumbnailObserver=null,thumbnailActive=0,thumbnailBytes=0,thumbnailPumpPending=false;
const thumbnailCache=new Map(),thumbnailQueue=new Map(),thumbnailInflight=new Set();
let thumbnailPolicyToken=null,thumbnailPolicyRevision=0,thumbnailPolicyBatch=Promise.resolve();
async function loadTextPreview(target,textEncoding){const context=target.oaTextContext;if(!context)return;const request=(target.oaTextRequest||0)+1;target.oaTextRequest=request;const meta=target.querySelector('#oa-text-meta'),text=target.querySelector('#oa-text-preview');meta.textContent='正在读取文本…';try{const r=await invoke('preview_entry',{...context.args,kind:'text',textEncoding});if(target!==$('qa-file-preview')||context.generation!==previewGeneration||target.oaTextRequest!==request)return;meta.textContent=`${r.encoding||'文本'} · ${r.truncated?'仅显示部分内容':'只读预览'}`;text.textContent=r.text||'';$('qa-formerror').textContent='';}catch(e){if(target===$('qa-file-preview')&&target.oaTextRequest===request){meta.textContent='无法预览';text.textContent='';error(e);}}}
function saveViewPreference(){const saved=load('qing-preferences',{});localStorage.setItem('qing-preferences',JSON.stringify({...saved,listStyle:s.listStyle,iconSize:s.iconSize}));}
function updateColumnScroll(){
 const control=$('oa-columns-scroll');
 if(!control)return;
 control.hidden=main.scrollWidth<=main.clientWidth+1;
 control.textContent=main.scrollLeft>0?'← 返回名称':'查看右侧列 →';
}
main.addEventListener('scroll',updateColumnScroll,{passive:true});
new ResizeObserver(updateColumnScroll).observe(main);
function viewControls(){return `<div class="oa-view-controls"><div class="oa-view-switch" role="group" aria-label="文件显示方式"><button data-action="details" aria-pressed="${s.listStyle!=='icons'}" title="详细信息">${icon('file')}<span>详情</span></button><button data-action="icons" aria-pressed="${s.listStyle==='icons'}" title="大图标和图片缩略图">${icon('grid')}<span>大图标</span></button></div><label class="oa-icon-control" ${s.listStyle==='icons'?'':'hidden'}><span>图标</span><input id="oa-icon-size" type="range" min="80" max="256" step="16" value="${s.iconSize}" aria-label="图标大小"><span id="oa-icon-size-label">${s.iconSize}</span></label><span class="oa-zoom-hint" ${s.listStyle==='icons'?'':'hidden'}>Ctrl + 滚轮调整</span><label class="oa-sort-control"><span>排序</span><select id="oa-sort" aria-label="排序依据">${opts(columns.filter(c=>c.visible).map(c=>[c.id,c.label]),s.sort)}</select><button type="button" data-action="reverse-sort" aria-label="${s.reverse?'改为升序':'改为降序'}" title="${s.reverse?'降序':'升序'}">${s.reverse?'↓':'↑'}</button></label>${s.listStyle==='icons'?'':`<div class="oa-column-controls">${btn('列表列','columns','quiet')}<button id="oa-columns-scroll" type="button" class="qa-button quiet" data-action="columns-scroll" hidden>查看右侧列 →</button></div>`}</div>`;}
function syncViewControls(){for(const button of root.querySelectorAll('th [data-sort]'))button.parentElement.setAttribute('aria-sort',button.dataset.sort!==s.sort?'none':s.reverse?'descending':'ascending');if($('oa-sort'))$('oa-sort').value=s.sort;const b=root.querySelector('.oa-view-controls [data-action=reverse-sort]');if(b){b.textContent=s.reverse?'↓':'↑';b.setAttribute('aria-label',s.reverse?'改为升序':'改为降序');b.title=s.reverse?'降序':'升序';}}
function setIconSize(value) {
  const grid = listingWindow?.iconGrid ? listingWindow : null;
  const focused = document.activeElement?.closest('[data-row]');
  const rect = focused?.getBoundingClientRect(), bounds = grid?.viewport.getBoundingClientRect();
  const keepFocusVisible = bounds && rect && rect.bottom > bounds.top && rect.top < bounds.bottom;
  s.iconSize = Math.max(80, Math.min(256, Math.round(value / 16) * 16));
  root.querySelector('.oa-table-wrap')?.style.setProperty('--oa-icon-size', s.iconSize + 'px');
  if ($('oa-icon-size')) $('oa-icon-size').value = s.iconSize;
  if ($('oa-icon-size-label')) $('oa-icon-size-label').textContent = s.iconSize;
  if (grid) {
    if (keepFocusVisible) grid.reveal(grid.cursorIndex()); else grid.paint();
  }
  saveViewPreference();
}
function stopThumbnailObserver(){thumbnailObserver?.disconnect();thumbnailObserver=null;thumbnailQueue.clear();}
function resetPreviewState(){previewGeneration++;stopThumbnailObserver();thumbnailCache.clear();thumbnailBytes=0;}
function thumbnailRecord(name){const record=thumbnailCache.get(name);if(!thumbnailPolicyToken||record?.format_policy!==thumbnailPolicyToken)return; if(record){thumbnailCache.delete(name);thumbnailCache.set(name,record);}return record;}
function rememberThumbnail(name,record){const old=thumbnailCache.get(name);if(old)thumbnailBytes-=old.cost;record.cost=(record.data_url?.length||0)*2+512;thumbnailCache.delete(name);thumbnailCache.set(name,record);thumbnailBytes+=record.cost;while(thumbnailCache.size>96||thumbnailBytes>12*1024*1024){const first=thumbnailCache.keys().next().value;thumbnailBytes-=thumbnailCache.get(first).cost;thumbnailCache.delete(first);}}
function thumbnailPlaceholder(el){el.replaceChildren();el.innerHTML=icon('file');el.classList.remove('oa-thumbnail-ready');el.removeAttribute('aria-busy');}
function paintThumbnail(el,record){if(!el.isConnected||el.dataset.thumbnailVisible!=='true')return;el.removeAttribute('aria-busy');el.title=record.error||'双击用系统默认看图程序打开原图';if(record.error){thumbnailPlaceholder(el);el.classList.add('oa-thumbnail-error');el.setAttribute('aria-label','缩略图不可用：'+record.error);return;}el.classList.remove('oa-thumbnail-error');el.classList.add('oa-thumbnail-ready');const img=document.createElement('img');img.alt=filename(el.dataset.thumbnail);img.decoding='async';img.draggable=false;img.src=record.data_url;el.replaceChildren(img);}
function enqueueThumbnail(el){
  const name=el.dataset.thumbnail, record=thumbnailRecord(name);
  if(record){if(!el.classList.contains('oa-thumbnail-ready'))paintThumbnail(el,record);return;}
  el.setAttribute('aria-busy','true');
  const key=previewGeneration+':'+thumbnailPolicyRevision+':'+name;
  if(!thumbnailInflight.has(key) && thumbnailQueue.size<128){
    const batch=s.entries.length>4096 && /\.zip$/i.test(s.archive) && !s.password && !s.codepage;
    thumbnailQueue.set(name,{name,batch,generation:previewGeneration,policyRevision:thumbnailPolicyRevision,args:previewArgs(name)});
  }
  if(!thumbnailPumpPending){
    thumbnailPumpPending=true;
    queueMicrotask(()=>{thumbnailPumpPending=false;pumpThumbnails();});
  }
}
function paintMatchingThumbnails(name,record){root.querySelectorAll('[data-thumbnail]').forEach(el=>{if(el.dataset.thumbnail===name)paintThumbnail(el,record);});}
function pumpThumbnails() {
  while (thumbnailActive < 2 && thumbnailQueue.size) {
    const tasks = [];
    const batch = thumbnailQueue.values().next().value.batch;
    if (batch && thumbnailActive) break;
    for (const [name, task] of thumbnailQueue) {
      if (task.batch !== batch) break;
      thumbnailQueue.delete(name);
      if (task.generation !== previewGeneration || task.policyRevision !== thumbnailPolicyRevision) continue;
      const key = task.generation + ':' + task.policyRevision + ':' + name;
      thumbnailInflight.add(key);
      tasks.push({...task, name, key});
      if (tasks.length === (batch ? 8 : 1)) break;
    }
    if (!tasks.length) continue;
    const {args, generation, policyRevision} = tasks[0];
    const current = () => generation === previewGeneration && policyRevision === thumbnailPolicyRevision;
    thumbnailActive++;
    const request = batch
      ? invoke('preview_thumbnails', {path:args.path, password:args.password, codepage:args.codepage, names:tasks.map(t=>t.name)})
      : invoke('preview_entry', {...args, name:tasks[0].name, kind:'thumbnail'}).then(result=>[result]);
    request.then(results => {
      if (!current()) return;
      if (!Array.isArray(results) || results.length !== tasks.length) throw Error('缩略图响应不完整');
      tasks.forEach(({name}, i) => {
        const r = results[i];
        try {
          if (r?.name !== name) throw Error('缩略图响应与文件不匹配');
          if (r.error) throw Error(r.error);
          if (!thumbnailPolicyToken || r.format_policy !== thumbnailPolicyToken)
            throw Error('格式策略已更新，请重新打开预览');
          if (typeof r.data_url !== 'string' || r.data_url.length > 2*1024*1024 ||
              !/^data:image\/(?:png|jpeg|webp);base64,[A-Za-z0-9+/=]+$/.test(r.data_url))
            throw Error('缩略图数据无效或超过预览限制');
          const record = {data_url:r.data_url, format_policy:r.format_policy};
          rememberThumbnail(name, record);
          paintMatchingThumbnails(name, record);
        } catch (error) {
          paintMatchingThumbnails(name, {error:String(error).slice(0,240)});
        }
      });
    }).catch(error => {
      if (current()) tasks.forEach(({name})=>paintMatchingThumbnails(name, {error:String(error).slice(0,240)}));
    }).finally(() => {
      thumbnailActive--;
      tasks.forEach(({key})=>thumbnailInflight.delete(key));
      pumpThumbnails();
    });
  }
}
function watchTile(row) {
  const thumbnail = row.querySelector('[data-thumbnail]');
  if (thumbnail) thumbnailObserver?.observe(thumbnail);
}
function unwatchTile(row) {
  const thumbnail = row.querySelector('[data-thumbnail]');
  if (thumbnail) {
    thumbnailObserver?.unobserve(thumbnail);
    thumbnailQueue.delete(thumbnail.dataset.thumbnail);
  }
}
function observeThumbnails(observeExisting=true){
  stopThumbnailObserver();
  if(s.page!=='files'||s.listStyle!=='icons')return;
  const observer=new IntersectionObserver(entries=>{
    for(const {target,isIntersecting} of entries){
      if(!target.isConnected)continue;
      target.dataset.thumbnailVisible=String(isIntersecting);
      if(!isIntersecting){thumbnailQueue.delete(target.dataset.thumbnail);thumbnailPlaceholder(target);}
    }
    if(!entries.some(entry=>entry.isIntersecting))return;
    thumbnailPolicyBatch=thumbnailPolicyBatch.then(async()=>{
      if(observer!==thumbnailObserver)return;
      const token=await invoke('preview_policy_token').catch(()=>null);
      if(observer!==thumbnailObserver)return;
      if(token!==thumbnailPolicyToken){
        thumbnailPolicyRevision++;thumbnailQueue.clear();thumbnailCache.clear();thumbnailBytes=0;
        thumbnailPolicyToken=token;
        root.querySelectorAll('[data-thumbnail]').forEach(thumbnailPlaceholder);
      }
      root.querySelectorAll('[data-thumbnail][data-thumbnail-visible="true"]').forEach(enqueueThumbnail);
    }).catch(()=>{});
  },{root:main,rootMargin:'100px 0px',threshold:0});
  thumbnailObserver=observer;
  if(observeExisting)root.querySelectorAll('[data-thumbnail]').forEach(el=>observer.observe(el));
}
window.addEventListener('resize',()=>{requestAnimationFrame(()=>{const strip=root.querySelector('.oa-tab-list'),active=strip?.querySelector('[aria-selected="true"]')?.closest('.oa-archive-tab');if(strip&&active){const r=strip.getBoundingClientRect(),a=active.getBoundingClientRect();if(a.left<r.left)strip.scrollLeft+=a.left-r.left;else if(a.right>r.right)strip.scrollLeft+=a.right-r.right;}$('oa-menu').hidden=true;});});
root.addEventListener('wheel',e=>{if(!e.ctrlKey||s.page!=='files'||!s.archive||!overlay.hidden||!e.target.closest('#qa-main'))return;e.preventDefault();e.stopPropagation();if(!e.deltaY)return;if(s.listStyle!=='icons'){s.listStyle='icons';render();}setIconSize(s.iconSize+(e.deltaY<0?16:-16));},{passive:false,capture:true});


const TAB_FIELDS=['nestedId','parentArchive','nestedMember','dirty','conflict','archive','entries','password','codepage','folder','flatView','search','searchScope','sort','reverse','selected','collapsed','anchor','cursor','focusFile','listStyle','iconSize','lastDest'];
function archiveKey(path){return String(path).replaceAll('/','\\').replace(/^\\\\\?\\/,'').replace(/^[a-z]:/, drive=>drive.toUpperCase());}
let recentContextPath='',recentContextRow=null;
function showRecentDialog(){dialog('最近的文件',s.recent.map((p,i)=>`<button class="qa-recent-row" data-recent="${i}">${esc(p)}</button>`).join('')||'<p>没有最近记录</p>',btn('清空记录','clear-recent')+btn('关闭','close'));overlay.dataset.page='recent';}
function updateRecent(paths){const inDialog=overlay.dataset.page==='recent';s.recent=paths;localStorage.setItem('qing-recent',JSON.stringify(s.recent));$('oa-menu').hidden=true;render();if(inDialog)showRecentDialog();}
function showRecentContext(row,event){rememberMenuFocus();const path=s.recent[Number(row.dataset.recent)];if(!path)return;recentContextPath=path;recentContextRow=row;const box=$('oa-menu');box.inert=false;box.innerHTML='<button role="menuitem" data-action="remove-recent">删除记录</button><button role="menuitem" data-action="clear-recent">清空记录</button>';positionMenu(box,{getBoundingClientRect:()=>({left:event.clientX,bottom:event.clientY})});box.querySelector('button').focus();}
root.addEventListener('keydown',handleMenuKeydown,true);
root.addEventListener('keydown',handleFileKeydown);
function rememberRecent(path){if(archiveTabs.some(t=>t.nestedId&&archiveKey(t.archive)===archiveKey(path)))return;s.recent=[path,...s.recent.filter(p=>archiveKey(p)!==archiveKey(path))].slice(0,12);localStorage.setItem('qing-recent',JSON.stringify(s.recent));}
function tabBusy(){return !!s.active||!!homeIntake||openingArchive||openingImage||!!$('qa-tools')?.contentWindow?.obzBusy;}
function saveCurrentTab(){const tab=archiveTabs.find(t=>t.id===currentTabId);if(!tab)return;for(const key of TAB_FIELDS)tab[key]=s[key] instanceof Set?new Set(s[key]):s[key];if(renderedPage==='archive:'+tab.id){tab.scrollTop=main.scrollTop;tab.scrollLeft=main.scrollLeft;tab.treeScroll=root.querySelector('.oa-tree-section').scrollTop;}}
function applyTab(tab){for(const key of TAB_FIELDS)s[key]=tab[key] instanceof Set?new Set(tab[key]):tab[key];if(!s.selected)s.selected=new Set();if(!s.collapsed)s.collapsed=new Set();}
function emptyArchiveState(){Object.assign(s,{nestedId:null,parentArchive:null,nestedMember:null,dirty:false,conflict:false,archive:'',entries:[],password:'',codepage:null,folder:'',flatView:false,search:'',sort:'name',reverse:false,selected:new Set(),collapsed:new Set(),anchor:null,cursor:null,focusFile:null,lastDest:undefined,listStyle:pref.listStyle==='icons'?'icons':'details',iconSize:Math.max(80,Math.min(256,Number(pref.iconSize)||144))});currentTabId=null;}
$('oa-archive-tabs').addEventListener('wheel',e=>{const list=$('oa-archive-tabs').querySelector('.oa-tab-list');if(list&&list.scrollWidth>list.clientWidth&&!e.ctrlKey){list.scrollLeft+=Math.abs(e.deltaX)>Math.abs(e.deltaY)?e.deltaX:e.deltaY;e.preventDefault();}},{passive:false});
$('oa-archive-tabs').addEventListener('keydown',handleArchiveTabKeydown);
function handleArchiveTabKeydown(event){
 if(isCompositionKey(event)||event.altKey||event.ctrlKey||event.metaKey)return;
 const id=event.target.closest('[data-archive-tab]')?.dataset.archiveTab;
 if(!id)return;
 const index=archiveTabs.findIndex(tab=>tab.id===id),count=archiveTabs.length;
 if(index<0)return;
 let next;
 switch(event.key){
  case 'ArrowLeft':next=(index+count-1)%count;break;
  case 'ArrowRight':next=(index+1)%count;break;
  case 'Home':next=0;break;
  case 'End':next=count-1;break;
  case 'Delete':event.preventDefault();event.stopPropagation();closeArchiveTabs('one',id);return;
  default:return;
 }
 event.preventDefault();
 event.stopPropagation();
 if(index!==next)switchArchiveTab(archiveTabs[next].id);
}
function renderArchiveTabs(){
 const host=$('oa-archive-tabs');
 if(!host)return;
 const panelActive=s.page==='files'&&!!currentTabId;
 host.hidden=!archiveTabs.length&&!pendingArchivePaths.length;
 root.dataset.tabs=String(!host.hidden);
 if(panelActive){
  main.setAttribute('role','tabpanel');
  main.setAttribute('aria-labelledby','oa-archive-tab-'+currentTabId);
 }else{
  main.removeAttribute('role');
  main.removeAttribute('aria-labelledby');
 }
 host.innerHTML=`<div class="oa-tab-list" role="tablist" aria-label="打开的压缩文件">${archiveTabs.map(tab=>{
  const current=tab.id===currentTabId,name=esc(filename(tab.archive));
  return `<div class="oa-archive-tab ${current?'active':''}" data-tab-container="${tab.id}">
   <button type="button" role="tab" id="oa-archive-tab-${tab.id}" tabindex="${current?0:-1}" data-archive-tab="${tab.id}" aria-selected="${panelActive&&current}" aria-controls="qa-main" title="${esc(tab.archive)}">${icon('archive')}<span>${tab.dirty?'● ':''}${name}${tab.nestedId?' ↳':''}</span></button>
   <button type="button" class="oa-tab-close" tabindex="-1" data-tab-close="${tab.id}" aria-label="关闭 ${name}" title="关闭标签">${icon('x')}</button></div>`;
 }).join('')}</div>${pendingArchivePaths.length?`<span class="oa-pending-tabs" title="当前操作结束并关闭对话框后继续打开">待打开 ${pendingArchivePaths.length}</span>`:''}`;
 requestAnimationFrame(()=>host.querySelector('[data-archive-tab][aria-selected="true"]')?.closest('.oa-archive-tab')?.scrollIntoView({block:'nearest',inline:'nearest'}));
}
let archiveTabFocusGeneration=0;
function archiveTabFocusOrigin(){const element=document.activeElement;return {element,id:currentTabId,page:s.page,dialog:overlay.hidden?null:overlay.firstElementChild,tabs:!!(element?.closest('[data-tab-container]')||($('oa-menu').contains(element)&&menuReturnFocus?.closest('[data-tab-container]')))};}
function restoreArchiveTabFocus(tab,origin,restoreScroll=true){
 const generation=++archiveTabFocusGeneration,id=currentTabId,element=document.activeElement;
 requestAnimationFrame(()=>{
  if(generation!==archiveTabFocusGeneration||currentTabId!==id||s.page!=='files'||!overlay.hidden||document.activeElement!==element)return;
  if(restoreScroll){main.scrollTop=tab?.scrollTop||0;main.scrollLeft=tab?.scrollLeft||0;root.querySelector('.oa-tree-section').scrollTop=tab?.treeScroll||0;}
  const button=root.querySelector(`[data-archive-tab="${id}"]`),row=rowButtons().find(b=>b.dataset.entry===s.cursor);
  const target=origin.tabs&&button?button:row||main;if(target===main)main.tabIndex=-1;target.focus({preventScroll:true});
  button?.scrollIntoView({block:'nearest',inline:'nearest'});
 });
}
function switchArchiveTab(id){const origin=archiveTabFocusOrigin();$('oa-menu').hidden=true;const tab=archiveTabs.find(t=>t.id===id);if(!tab)return;if(tabBusy()&&!s.backgroundExtraction){toast('任务正在进行，请先等待或取消后再切换标签。');return;}saveCurrentTab();resetPreviewState();currentTabId=id;applyTab(tab);close();s.page='files';s.view='archive';render();restoreArchiveTabFocus(tab,origin);}
function wipeTab(tab){tab.password='';tab.entries=[];tab.selected?.clear();tab.collapsed?.clear();}
let removingArchiveTabs=false,closingArchiveTabs=false,finishingArchiveTabs=false;
async function removeArchiveTabs(remove,origin=archiveTabFocusOrigin()){
 if(removingArchiveTabs)return false;$('oa-menu').hidden=true;if(tabBusy()){toast('任务正在进行，请先等待或取消后再关闭标签。');return;}saveCurrentTab();if(!remove.length)return true;removingArchiveTabs=true;
 try{
  await invoke('revoke_archive_verification');if(tabBusy())return false;remove=remove.filter(tab=>archiveTabs.includes(tab));
  const originalIndex=archiveTabs.findIndex(t=>t.id===currentTabId),removedCurrent=remove.some(t=>t.id===currentTabId);
  const ownsDialog=!overlay.hidden&&origin.dialog===overlay.firstElementChild;
  const canFocus=origin.id===currentTabId&&origin.page===s.page&&(ownsDialog||document.activeElement===origin.element||(!origin.element?.isConnected&&[document.body,main].includes(document.activeElement)));
  for(const tab of remove){const index=archiveTabs.indexOf(tab);if(index<0)continue;archiveTabs.splice(index,1);wipeTab(tab);}
  if(removedCurrent)resetPreviewState();invoke('clear_preview_cache').catch(()=>{});
  if(!removedCurrent){if(ownsDialog)close();renderArchiveTabs();if(canFocus&&origin.tabs)restoreArchiveTabFocus(archiveTabs.find(t=>t.id===currentTabId),origin,false);return true;}
  if(overlay.hidden||ownsDialog)close();
  const next=archiveTabs[Math.min(Math.max(originalIndex,0),archiveTabs.length-1)];if(next){currentTabId=next.id;applyTab(next);}else emptyArchiveState();
  if(s.page==='files'){s.view=s.archive?'archive':'home';render();if(canFocus)restoreArchiveTabFocus(next,origin);}else renderArchiveTabs();return true;
 }finally{removingArchiveTabs=false;}
}
function discardArchiveTab(path,{preserveJob=false}={}){const target=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(path));if(!target)return;const index=archiveTabs.indexOf(target),wasCurrent=target.id===currentTabId;archiveTabs.splice(index,1);wipeTab(target);resetPreviewState();invoke('clear_preview_cache').catch(()=>{});if(wasCurrent){const next=archiveTabs[Math.min(index,archiveTabs.length-1)];if(next){currentTabId=next.id;applyTab(next);}else emptyArchiveState();}if(!preserveJob){s.page='files';s.view=s.archive?'archive':'home';}renderArchiveTabs();}
function cycleArchiveTab(direction){if(archiveTabs.length<2)return;const index=archiveTabs.findIndex(t=>t.id===currentTabId),next=(index+direction+archiveTabs.length)%archiveTabs.length;switchArchiveTab(archiveTabs[next].id);}
function showArchiveTabMenu(id,event){rememberMenuFocus();const box=$('oa-menu');box.innerHTML=`<div class="oa-menu-title">标签管理</div><button type="button" data-tab-close="${id}">关闭此标签<small>Ctrl+W</small></button><button type="button" data-tab-close-others="${id}">关闭其他标签</button><button type="button" data-action="close-all-tabs">关闭所有标签</button>`;positionMenu(box,{getBoundingClientRect:()=>({left:event.clientX,bottom:event.clientY})});box.querySelector('button')?.focus();}
function enqueueArchives(paths){if(!Array.isArray(paths))return;for(const path of paths){if(typeof path!=='string'||!path.trim())continue;if(!pendingArchivePaths.some(p=>archiveKey(p)===archiveKey(path)))pendingArchivePaths.push(path);}renderArchiveTabs();if(pendingArchivePaths.length&&(tabBusy()||!overlay.hidden))toast(`已加入 ${pendingArchivePaths.length} 个待打开文件，当前操作结束后继续。`);pumpArchiveQueue().catch(error).finally(schedulePendingQueue);}
function shellQueueStatus(){if(!pendingShellExtractions.length)return '';const first=pendingShellExtractions[0],failed=first.state==='failed'||first.state==='cancelled';return `<div class="qa-jobcard"><strong>${shellQueuePaused?'解压队列已暂停':'解压队列'} · ${pendingShellExtractions.length} 个压缩包</strong>${failed?`<p class="qa-sub">${esc(filename(first.path))}</p><div class="qa-error" role="alert">${esc(first.error||'任务未完成')}</div>`:''}<div class="qa-actions">${shellQueuePaused?btn(failed?'重试':'继续队列','shell-continue','primary'):''}${shellQueuePaused&&failed?btn('输入密码重试','shell-password'):''}${btn(shellActiveRequest?'清空等待项':'清空队列','shell-clear','quiet')}</div></div>`;}
function clearShellSecret(request){if(request)delete request.password;}
function clearShellQueue(){for(const request of pendingShellExtractions)clearShellSecret(request);clearShellSecret(shellActiveRequest);clearShellSecret(shellPasswordRequest);pendingShellExtractions.splice(0,pendingShellExtractions.length,...(shellActiveRequest?[shellActiveRequest]:[]));shellQueuePaused=false;shellQueueInterrupted=true;shellPasswordRequest=null;const input=$('qa-shell-password');if(input){input.value='';close();}if(s.view==='jobs')renderProgress();}
function showShellPassword(){if(shellQueueRunning||tabBusy())throw Error('请先等待或取消当前任务。');const request=pendingShellExtractions[0];if(!request||!shellQueuePaused)throw Error('没有等待重试的解压任务。');shellPasswordRequest=request;dialog('输入密码重试',`<p class="qa-sub">${esc(filename(request.path))}</p><label class="qa-field">密码<input id="qa-shell-password" type="password" autocomplete="off"></label>`,btn('取消','close')+btn('重试','shell-password-submit','primary'));}
async function retryShellExtraction(withPassword=false){if(shellQueueRunning||tabBusy())throw Error('请先等待或取消当前任务。');const request=withPassword?shellPasswordRequest:pendingShellExtractions[0];if(!request||pendingShellExtractions[0]!==request)throw Error('该解压任务已移出队列。');if(withPassword){const input=$('qa-shell-password');if(!input||!input.value)throw Error('请输入密码。');request.password=input.value;input.value='';}request.state='pending';delete request.error;request.cancelRequested=false;shellPasswordRequest=null;shellQueuePaused=false;if(!overlay.hidden)close();await pumpShellExtractions();}
function enqueueShellExtractions(requests){if(!Array.isArray(requests))return;for(const request of requests){if(typeof request?.path!=='string'||typeof request?.dest!=='string'||typeof request?.smart!=='boolean')continue;if(!pendingShellExtractions.some(p=>p.smart===request.smart&&archiveKey(p.path)===archiveKey(request.path)))pendingShellExtractions.push({path:request.path,dest:request.dest,smart:request.smart,state:'pending'});}if(requests.length&&(tabBusy()||!overlay.hidden))toast('解压任务已加入队列，当前操作结束后继续。');if(s.view==='jobs')renderProgress();}
async function pumpShellExtractions(){if(shellQueueRunning||shellQueuePaused||tabBusy()||!overlay.hidden)return;shellQueueRunning=true;shellQueueInterrupted=false;let allSucceeded=true,wantsClose=false,completedItem=false;const canAdvance=()=>overlay.hidden||(completedItem&&overlay.dataset.page==='jobs'&&!$('oa-exit-confirm'));try{while(pendingShellExtractions.length&&!shellQueuePaused&&!tabBusy()&&canAdvance()){const request=pendingShellExtractions[0],tab=archiveKey(s.archive)===archiveKey(request.path)?s:archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(request.path)),overwrite=s.overwrite,recycle=!!s.recycleAfter&&canRecycleArchive(request.path,overwrite),openAfter=!!s.openAfter;wantsClose=!!s.closeAfter;shellActiveRequest=request;request.state='running';let extracted=false;try{const result=await run({operation:'extract',path:request.path,dest:request.dest,smart:request.smart,options:{selected:[],overwrite,password:(request.password!==undefined?request.password:tab?.password)||null,codepage:tab?.codepage||null}},(request.smart?'智能解压 · ':'解压到此处 · ')+filename(request.path),{recycleAfter:recycle});extracted=true;completedItem=true;request.state='complete';clearShellSecret(request);const index=pendingShellExtractions.indexOf(request);if(index>=0)pendingShellExtractions.splice(index,1);if(!await finishExtraction(request.path,request.dest,result,{recycle,openAfter})){allSucceeded=false;shellQueuePaused=!!pendingShellExtractions.length;break;}}catch(e){allSucceeded=false;if(!extracted){request.state=request.cancelRequested?'cancelled':'failed';request.error=String(e);shellQueuePaused=true;}else shellQueuePaused=!!pendingShellExtractions.length;error(e);break;}finally{shellActiveRequest=null;}}const drained=await drainNativeArchives();if(wantsClose&&allSucceeded&&!shellQueueInterrupted&&!shellQueuePaused&&drained&&!pendingShellExtractions.length&&!pendingFolders.length&&!pendingFileGroups.length&&!pendingArchivePaths.length&&!tabBusy()&&!openQueueRunning&&canAdvance())await invoke('window_action',{action:'close'});}finally{shellActiveRequest=null;shellQueueRunning=false;if(s.view==='jobs')renderProgress();}}
function showCompressionFiles(paths) {
  compress();
  $('qa-inputs').value=paths.join('\n');
  refreshSources();
}
let archiveQueueTimer = null;
function schedulePendingQueue() {
  if (archiveQueueTimer !== null) return;
  const pending = pendingArchivePaths.length || pendingFileGroups.length || pendingFolders.length
    || (pendingShellExtractions.length && !shellQueuePaused);
  if (!pending) return;
  archiveQueueTimer = setTimeout(() => {
    archiveQueueTimer = null;
    pumpArchiveQueue().catch(error).finally(schedulePendingQueue);
  }, 350);
}
async function pumpArchiveQueue(){if(shellQueueRunning||openQueueRunning)return;if(pendingFileGroups.length&&!tabBusy()&&overlay.hidden){showCompressionFiles(pendingFileGroups.shift());return;}if(pendingFolders.length&&!tabBusy()&&overlay.hidden){const folder=pendingFolders.shift();showCompressionFiles([folder]);return;}if(pendingShellExtractions.length&&!shellQueuePaused&&!tabBusy()&&overlay.hidden){await pumpShellExtractions();return;}if(!pendingArchivePaths.length||tabBusy()||!overlay.hidden)return;openQueueRunning=true;try{while(pendingArchivePaths.length&&!tabBusy()&&overlay.hidden){const path=pendingArchivePaths.shift();renderArchiveTabs();try{await openArchive(path);}catch(e){openDialog(path);error(e);if(/密码/.test(String(e)))$('qa-open-password')?.focus();break;}}}finally{openQueueRunning=false;renderArchiveTabs();}}
let nativeDrainAgain=false;
async function drainNativeArchives(){if(drainingNativeQueue){nativeDrainAgain=true;return false;}drainingNativeQueue=true;try{do{nativeDrainAgain=false;const requests=await invoke('take_pending_shell_extractions');enqueueShellExtractions(requests);const groups=await invoke('take_pending_compression_files');for(const group of groups){if(group.Err){toast(group.Err);continue;}if(group.Ok?.length)pendingFileGroups.push(group.Ok);}const folders=await invoke('take_pending_folders');for(const p of folders)if(!pendingFolders.includes(p))pendingFolders.push(p);if(folders.length&&(tabBusy()||!overlay.hidden))toast('文件夹已加入压缩队列，当前操作结束后继续。');const paths=await invoke('take_pending_archives');enqueueArchives(paths);}while(nativeDrainAgain);return true;}catch(e){error(e);return false;}finally{drainingNativeQueue=false;schedulePendingQueue();}}

let pendingNestedOpen=null,pendingNestedClose=null;
function selectedNestedMember(){if(!s.archive||tabBusy()||s.selected.size!==1)return null;const name=[...s.selected][0];return visibleEntries().some(e=>e.name===name&&!e.is_dir)?name:null;}
function isNestedArchive(name){return /\.(?:zip|zipx|7z|rar|tar|tgz|gz|bz2|xz|zst|cab|iso|wim|001)$/i.test(name);}
function nestedBanner(){return s.nestedId?`<div class="qa-path oa-nested-bar"><span>${s.conflict?'外层已改变，工作副本已保留':s.dirty?'更改尚未写回外层压缩包':'内层压缩包'} · ${esc(filename(s.parentArchive||''))}</span><span style="flex:1"></span>${btn('保存至外层','save-nested','quiet')}${btn('另存为…','save-as','quiet')}</div>`:'';}
async function syncNestedSessions(){if(!archiveTabs.some(t=>t.nestedId))return;const sessions=await invoke('nested_sessions');for(const tab of archiveTabs){const state=sessions.find(v=>v.id===tab.nestedId);if(state){tab.dirty=state.dirty;tab.conflict=state.conflict;if(tab.id===currentTabId){s.dirty=state.dirty;s.conflict=state.conflict;}}}renderArchiveTabs();}
async function openNestedEntry(member){if(tabBusy())throw Error('请先等待当前任务完成。');const existing=archiveTabs.find(t=>archiveKey(t.parentArchive||'')===archiveKey(s.archive)&&t.nestedMember===member);if(existing){switchArchiveTab(existing.id);return;}const context={parent:s.archive,member,options:{password:s.password||null,codepage:s.codepage||null},session:null,stage:'outer'};if(visibleEntries().find(e=>e.name===member)?.encrypted&&!context.options.password){showNestedPassword(context,'请输入外层压缩包密码。');return;}await continueNestedOpen(context,'');}
async function continueNestedOpen(context,password){pendingNestedOpen=null;try{if(!context.session){if(password)context.options.password=password;context.session=await run({operation:'nested-open',path:context.parent,name:context.member,options:context.options},'打开内层压缩包');const parentTab=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(context.parent));if(parentTab&&context.options.password){parentTab.password=context.options.password;if(parentTab.id===currentTabId)s.password=parentTab.password;}context.stage='inner';password='';}await openArchive(context.session.path,password,{nested:context.session});}catch(e){const message=archiveOpenMessage(e,!!password);if(/password|密码/i.test(message)){showNestedPassword(context,message);}else{if(context.session)await invoke('close_nested',{ids:[context.session.id],discard:false});throw e;}}}
function showNestedPassword(context,message){pendingNestedOpen=context;dialog(context.stage==='outer'?'输入外层压缩包密码':'输入内层压缩包密码',`<p>${esc(filename(context.stage==='outer'?context.parent:context.member))}</p><label class="qa-field">密码<input id="oa-nested-password" type="password" autocomplete="off"></label>`,btn('取消','close')+btn('打开','nested-open-submit','primary'));$('qa-formerror').textContent=message;$('oa-nested-password').focus();}
async function retryNestedOpen(){const context=pendingNestedOpen;if(!context)return;const password=$('oa-nested-password').value;if(!password)throw Error('请输入密码。');await continueNestedOpen(context,password);}
async function saveNested(tab=archiveTabs.find(t=>t.id===currentTabId)){if(!tab?.nestedId)throw Error('当前标签不是内层压缩包。');await run({operation:'nested-save',path:tab.archive},'保存至外层压缩包');await syncNestedSessions();let parent=tab.parentArchive;while(parent){const t=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(parent));if(!t)break;const r=await run({operation:'list',path:t.archive,options:{password:t.password||null,codepage:t.codepage||null}},'刷新外层压缩包',{silent:true});t.entries=r.entries;if(t.id===currentTabId)s.entries=t.entries;parent=t.parentArchive;}navigate('archive');toast('已保存至外层压缩包。');}
function nestedCloseSelection(mode,id){if(mode==='all'||mode==='exit')return archiveTabs.slice();const remove=mode==='others'?archiveTabs.filter(t=>t.id!==id):archiveTabs.filter(t=>t.id===id);if(mode==='others'){let kept=archiveTabs.find(t=>t.id===id);while(kept?.parentArchive){kept=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(kept.parentArchive));const index=remove.indexOf(kept);if(index>=0)remove.splice(index,1);}}else{for(let i=0;i<remove.length;i++)for(const t of archiveTabs){if(!remove.includes(t)&&archiveKey(t.parentArchive||'')===archiveKey(remove[i].archive))remove.push(t);}}return remove;}
function showNestedClose(){const pending=pendingNestedClose;if(!pending)return;dialog('保存内层压缩包的更改？',`<p>以下压缩包的更改尚未写回外层：</p><p>${pending.tabs.filter(t=>t.dirty).map(t=>esc(filename(t.archive))).join('<br>')}</p>`,btn('取消','close')+btn('放弃更改','nested-close-discard','danger')+btn('保存并关闭','nested-close-save','primary'));if(pending.focusOrigin)pending.focusOrigin.dialog=overlay.firstElementChild;}
function closeArchiveTabs(mode,id=currentTabId){const focusOrigin=archiveTabFocusOrigin();(async()=>{if(closingArchiveTabs||finishingArchiveTabs||removingArchiveTabs)return;closingArchiveTabs=true;try{if(tabBusy()){toast('任务正在进行，请先等待或取消后再关闭标签。');return;}saveCurrentTab();await syncNestedSessions();const tabs=nestedCloseSelection(mode,id);const edits=await invoke('external_edits');if(edits.some(e=>mode==='exit'||tabs.some(t=>archiveKey(t.archive)===archiveKey(e.parent)))){await showExternalEdits();return;}pendingNestedClose={tabs,exit:mode==='exit',focusOrigin};if(tabs.some(t=>t.dirty)){showNestedClose();return;}await finishNestedClose(false);}finally{closingArchiveTabs=false;}})().catch(error);}
async function finishNestedClose(save){const pending=pendingNestedClose;if(!pending||tabBusy()||finishingArchiveTabs)return;finishingArchiveTabs=true;try{if(save){for(const tab of pending.tabs){await syncNestedSessions();if(tab.dirty)await saveNested(tab);}}const ids=pending.tabs.filter(t=>t.nestedId).map(t=>t.nestedId);if(ids.length)await invoke('close_nested',{ids,discard:!save});if(!await removeArchiveTabs(pending.tabs,pending.focusOrigin))return;if(pendingNestedClose===pending)pendingNestedClose=null;if(pending.exit)await invoke('window_action',{action:'close'});}catch(e){await syncNestedSessions();showNestedClose();throw e;}finally{finishingArchiveTabs=false;}}

root.querySelectorAll('[data-lucide]').forEach(i=>{i.outerHTML=icon(i.dataset.lucide)});invoke('app_version').then(v=>{appVersion=v;if(s.page==='settings')render();}).catch(error);appearance();render();invoke('file_compression_menu_enabled').then(v=>{fileMenuEnabled=v;if(s.page==='settings')render();}).catch(error);invoke('folder_menu_enabled').then(v=>{folderMenuEnabled=v;if(s.page==='settings')render();}).catch(error);window.__TAURI__.event.listen('open-archives',()=>drainNativeArchives()).then(drainNativeArchives).catch(error);
function showHomeSmartDrop(dragging) {
  const target=$('oa-home-smart-drop');
  if(!target)return;
  const active=dragging&&homeIntakeAvailable();
  target.classList.toggle('oa-drop-active',active);
  target.querySelector('[data-drop-label]').textContent=active?'松开以识别文件':'自动识别压缩包和多层压缩包';
}
for (const type of ['drag-enter','drag-over']) {
  window.__TAURI__.event.listen('tauri://'+type,()=>showHomeSmartDrop(true)).catch(error);
}
window.__TAURI__.event.listen('tauri://drag-leave',()=>showHomeSmartDrop(false)).catch(error);
window.__TAURI__.event.listen('tauri://drag-drop',e=>{
  showHomeSmartDrop(false);
  acceptDroppedPaths(e.payload?.paths||[]);
}).catch(error);
async function showProperties(wholeArchive){
 if(!s.archive)throw Error('请先打开压缩包。');
 const selected=wholeArchive?[]:[...s.selected],archive=s.archive,options={password:s.password||null,codepage:s.codepage||null};
 const details=QingArchiveProperties.describe(s.entries,selected,archive);
 for(const row of details.rows)if(['修改时间','创建时间','访问时间'].includes(row[0])&&row[1]!=='未提供'){row[1]=displayModified(row[1]);row[0]+='（本地）';}
 const markup=rows=>`<dl class="qa-properties">${rows.map(([label,value])=>`<div><dt>${esc(label)}</dt><dd>${esc(value)}</dd></div>`).join('')}</dl>`;
 const query=wholeArchive||selected.length===0;
 dialog(query?'压缩包属性':details.title,markup(details.rows)+(query?'<section id="qa-property-status"><p>正在读取格式属性…</p></section>':''),btn('关闭','close')+(query?btn('取消读取','canceljob'):''));
 if(!query)return;
 const target=$('qa-property-status');
 try{const properties=await run({operation:'archive-properties',path:archive,options},'读取压缩包属性',{silent:true,startJob:async request=>{const id=await invoke('start_operation',{request});if(!target.isConnected)await invoke('cancel_operation',{id}).catch(()=>{});return id;}});
  if(target.isConnected)target.innerHTML=markup(QingArchiveProperties.technical(properties))+(properties.partial?'<p class="qa-sub">部分属性未提供：归档包含无法可靠解析的多行元数据。</p>':'');
 }catch(reason){if(target.isConnected)target.textContent=/cancelled/i.test(String(reason))?'已取消属性读取':'格式属性读取失败：'+String(reason);}
 finally{if(target.isConnected)overlay.querySelector('[data-action="canceljob"]')?.remove();}
}

// Explicit editing leaves image/text double-click previews unchanged.
let externalContext=null,externalCurrent=null;
async function beginExternalEdit(){
 if((await invoke('external_edits')).length){await showExternalEdits();return;}
 const member=selectedNestedMember();if(!member)throw Error('请选中一个文件。');
 externalContext={parent:s.archive,member,options:{password:s.password||null,codepage:s.codepage||null}};
 dialog('外部编辑',`<p>${esc(member)}</p><label class="qa-field">编辑器（留空使用记事本）<input id="oa-editor-path" readonly value="${esc(localStorage.getItem('qing-editor-path')||'')}"></label>${btn('选择编辑器…','external-editor-pick')}${btn('使用记事本','external-editor-default','quiet')}<label class="qa-field">压缩包密码（如有）<input id="oa-editor-password" type="password" autocomplete="off"></label>`,btn('取消','close')+btn('开始编辑','external-launch','primary'));
}
async function launchExternalEdit(){
 const context=externalContext;if(!context)return;const editor=$('oa-editor-path').value;
 context.options.password=$('oa-editor-password').value||context.options.password;localStorage.setItem('qing-editor-path',editor);
 await run({operation:'external-edit',path:context.parent,name:context.member,label:editor,options:context.options},'准备外部编辑');
 const tab=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(context.parent));if(tab&&context.options.password){tab.password=context.options.password;if(tab.id===currentTabId)s.password=tab.password;}
 externalContext=null;await showExternalEdits();
}
async function showExternalEdits(){
 const edits=await invoke('external_edits');externalCurrent=edits[0]||null;
 if(!externalCurrent){dialog('外部编辑','<p>没有待处理的编辑会话。</p>',btn('保留的编辑副本…','external-copies')+btn('关闭','close'));return;}
 const e=externalCurrent;const state={running:'编辑器仍在运行',handoff:'编辑器可能在其他窗口中继续编辑',finished:'启动进程已退出'}[e.state];
 dialog('外部编辑会话',`<p>${esc(e.member)}</p><p>${state} · ${e.changed===true?'已保存的文件有更改':e.changed===false?'已保存的文件未改变':'文件正在使用中'}</p><p class="qa-sub">请先在编辑器中保存。写回仅包含已保存的内容，并结束本次编辑；之后的修改不会自动写回。</p><p class="qa-sub">编辑器可能把文件交给其他窗口，结束会话后工作副本会保留，可从“保留的编辑副本”找回。</p>${e.error?`<p class="qa-sub">${esc(e.error)}</p>`:''}`,btn('返回','close')+btn('检查更改','external-sessions')+btn('另存为…','external-copy')+btn('结束，不写回','external-discard-confirm')+btn('确认写回…','external-save-confirm','primary'));
}
function confirmExternalFinish(save){
 if(!externalCurrent)return;
 dialog(save?'确认写回压缩包？':'结束外部编辑？',`<p>${esc(externalCurrent.member)}</p><p>${save?'将编辑器已经保存的内容写回压缩包。':'此次编辑不会写回压缩包。'}请确认已完成本次编辑。</p><p>工作副本会保留；编辑器之后的修改不会自动写回。</p>`,btn('返回','external-sessions')+btn(save?'写回并结束':'结束，不写回',save?'external-save':'external-discard',save?'primary':'danger'));
}
async function finishExternalEdit(save){
 const e=externalCurrent;if(!e)return;
 if(save)await run({operation:'external-save',path:e.path},'写回编辑内容');
 const retained=await invoke('finish_external_edit',{id:e.id,discard:!save});externalCurrent=null;
 await syncNestedSessions();
 if(save){let parent=e.parent;while(parent){const tab=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(parent));if(!tab)break;const r=await run({operation:'list',path:tab.archive,options:{password:tab.password||null,codepage:tab.codepage||null}},'刷新压缩包',{silent:true});tab.entries=r.entries;if(tab.id===currentTabId)s.entries=r.entries;parent=tab.parentArchive;}render();}
 if(retained)dialog(save?'已写回':'已结束本次编辑',`<p>工作副本仍可由编辑器使用：</p><pre class="qa-properties">${esc(retained)}</pre><p class="qa-sub">之后的修改不会自动写回压缩包。关闭或重启后，可从编辑菜单的“保留的编辑副本”找回。</p>`,btn('保留的编辑副本…','external-copies')+btn('关闭','close'));
 else{close();toast(save?'已写回压缩包。':'已结束本次编辑。');}
}

let retainedEditRecords=[];
async function showRetainedEdits(){
 retainedEditRecords=await invoke('retained_edits');
 if(!retainedEditRecords.length){dialog('保留的编辑副本','<p>没有保留的编辑副本。</p>',btn('关闭','close'));return;}
 dialog('保留的编辑副本',`<p>这里的副本保存在独立目录，不随预览缓存清理。</p><label class="qa-field">文件<select id="oa-retained-edit">${retainedEditRecords.map((r,i)=>`<option value="${i}">${esc(r.member)} — ${esc(r.path)}</option>`).join('')}</select></label>`,btn('关闭','close')+btn('打开所在位置','external-retained-open')+btn('另存为…','external-retained-save','primary'));
}

root.addEventListener('change',e=>{
 if(e.target.id==='qa-update-mode')syncAddMode();
 if(e.target.id==='oa-add-password-mode')syncAddPassword();
});

})();
