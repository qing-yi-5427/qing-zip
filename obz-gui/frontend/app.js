(()=>{'use strict';
const root=document.getElementById('qing-archive'),main=document.getElementById('qa-main'),overlay=document.getElementById('qa-overlay');
const invoke=window.__TAURI__.core.invoke,$=id=>document.getElementById(id),esc=v=>String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const toolIcons={
 wrench:'<path d="M14.7 6.3a4.5 4.5 0 0 0-5.4 5.4L3.8 17.2a2.1 2.1 0 0 0 3 3l5.5-5.5a4.5 4.5 0 0 0 5.4-5.4l-2.5 2.5-2.4-2.4z"/>',
 split:'<path d="M4 12h4c5 0 4-6 9-6h3m-3-3 3 3-3 3M8 12c5 0 4 6 9 6h3m-3-3 3 3-3 3"/>',
 merge:'<path d="M5 5v3a4 4 0 0 0 4 4h10m-4-4 4 4-4 4M5 19v-3a4 4 0 0 1 4-4"/>',
 'book-lock':'<path d="M10 4H6a2 2 0 0 0-2 2v13a2 2 0 0 1 2-2h4m-6 2a2 2 0 0 0 2 2h14"/> <rect x="12" y="9" width="9" height="7" rx="2"/><path d="M14 9V7a2.5 2.5 0 0 1 5 0v2"/>',
 key:'<circle cx="16" cy="8" r="4"/><path d="m13 11-9 9H2v-3l3-3h3v-3l2-2"/>',
 link:'<path d="m10 13 4-4m-6 5-1 1a3 3 0 0 0 4 4l4-4a3 3 0 0 0 0-4m1-1 1-1a3 3 0 0 0-4-4L9 9a3 3 0 0 0 0 4"/>',
 mouse:'<rect x="6" y="3" width="12" height="18" rx="6"/><path d="M12 3v7M6 10h12"/>',
 gauge:'<path d="M5 18a9 9 0 1 1 14 0H5zm7-1 4-5M7 12h1m4-5v1m5 1-1 1"/>'
};
const icon=n=>{const paths={archive:'<rect x="4" y="3" width="16" height="18" rx="3"/><path d="M11 3v3h2v3h-2v3h2v3h-2v3h2"/>','folder-open':'<path d="M3 7V4h6l3 3h9v13H3zM3 10h18"/>','folder-output':'<path d="M3 7V4h6l3 3h9v13H3zM12 10v7m-3-3 3 3 3-3"/>',folder:'<path d="M3 7V4h6l3 3h9v13H3z"/>',plus:'<path d="M12 4v16M4 12h16"/>',x:'<path d="m6 6 12 12M18 6 6 18"/>',check:'<path d="m3 6 2 2 3-4M11 6h10M3 13l2 2 3-4M11 13h10M4 20h17"/>',shield:'<path d="m12 3 8 3v5c0 5-4 8-8 10-4-2-8-5-8-10V6zM8 12l3 3 5-6"/>',file:'<path d="M5 3h9l5 5v13H5zM14 3v6h5M8 13h8M8 17h6"/>',globe:'<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18M5 7h14M5 17h14"/>',home:'<path d="m3 11 9-8 9 8M5 10v11h14V10M10 21v-7h4v7"/>','arrow-up':'<path d="m5 11 7-7 7 7M12 4v16"/>',sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1 1M18 18l1 1M5 19l1-1M18 6l1-1"/>',moon:'<path d="M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11z"/>',grid:'<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',settings:'<path d="M3 6h18M3 12h18M3 18h18"/><circle cx="8" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>',more:'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',chevron:'<path d="m9 5 7 7-7 7"/>',search:'<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>'};return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${toolIcons[n]||paths[n]||paths.file}</svg>`;};

const btn=(label,action,cls='',ico='')=>`<button type="button" class="qa-button ${cls}" data-action="${action}" ${label?'':'aria-label="上级目录"'}>${ico?icon(ico):''}${label}</button>`;
const opts=(values,value)=>values.map(([v,l])=>`<option value="${v}" ${String(v)===String(value)?'selected':''}>${l}</option>`).join('');
const formats=[['zip','ZIP'],['7z','7z'],['zipx','ZIPX · XZ'],['tar','TAR'],['tgz','TAR.GZ'],['tbz2','TAR.BZ2'],['txz','TAR.XZ'],['gzip','GZip'],['bzip2','BZip2'],['xz','XZ'],['zstd','Zstandard（单文件）'],['brotli','Brotli（单文件）'],['lz4','LZ4（单文件）'],['lz5','LZ5（单文件）'],['lizard','Lizard（单文件）'],['wim','WIM'],['sfx','EXE 自解压'],['iso','ISO'],['lzh','LZH']];
const load=(key,fallback)=>{try{return JSON.parse(localStorage.getItem(key))??fallback}catch{return fallback}};
let appVersion='',folderMenuEnabled=false,fileMenuEnabled=false;const pendingFolders=[],pendingFileGroups=[];
const pref=load('qing-preferences',{}),s={page:'files',view:'home',collapsed:new Set(),theme:QingAppearance.preferredMode(pref.theme),material:pref.material||'acrylic',density:pref.density||'relaxed',overwrite:pref.overwrite||'error',sameFolder:pref.sameFolder!==false,recycleAfter:!!pref.recycleAfter,openAfter:!!pref.openAfter,closeAfter:!!pref.closeAfter,archive:'',entries:[],folder:'',flatView:false,search:'',sort:'name',reverse:false,selected:new Set(),jobs:[],active:null,password:'',recent:load('qing-recent',[]),profiles:load('qing-profiles',[]),batchStop:false};
const passwordBook=window.QingPasswordBook({invoke,esc,storage:localStorage});
const deepExtract=window.QingDeepExtract({passwordBook,root,s,pref,esc,btn,dialog,$,run,invoke,appearance,toast,error,archiveParent,
 joinExtractionPath,conflictHelp,sourceDisposition,recycleSummary,
 currentReport:currentDeepReport,canContinue:job=>deepCanContinue(job),
 folderForSource:(path,smart)=>{
  const tab=smart&&archiveTabs.find(tab=>archiveKey(tab.archive)===archiveKey(path));
  return tab?extractionLayout(path,tab.entries,[],true,true).folder:archiveStem(path);
 },
 passwordRequiredForSource:path=>{
  const tab=archiveTabs.find(tab=>archiveKey(tab.archive)===archiveKey(path));
  return !!tab&&extractPasswordRequired(tab.entries,[],tab.headerEncrypted);
 },
 onRecycled:paths=>forgetRecycledArchives(paths),onBrowse:paths=>{close();enqueueArchives(paths);}});
let columns=ArchiveColumns.normalize(load('qing-columns',null)),columnDraft=null;
let toastTimer,returnFocus,menuReturnFocus,menuAnchorRect;
s.listStyle=pref.listStyle==='icons'?'icons':'details';s.iconSize=Math.max(80,Math.min(256,Number(pref.iconSize)||144));
const archiveTabs=[];let currentTabId=null,nextTabId=1,openingArchive=false,secondaryOpen=null,openQueueRunning=false,drainingNativeQueue=false;const pendingArchivePaths=[];const pendingShellExtractions=[];let shellQueueRunning=false,shellQueuePaused=false,shellQueueInterrupted=false,shellActiveRequest=null,shellPasswordRequest=null;
const filename=p=>p.split(/[\\/]/).pop(),size=n=>n<1024?`${n} B`:n<1048576?`${(n/1024).toFixed(1)} KB`:`${(n/1048576).toFixed(1)} MB`;
const entrySize=e=>e.size_known===false?'未知':size(e.size);
function displayModified(value){const text=String(value||'');if(!/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d{1,7})?Z$/.test(text))return text;const date=new Date(text.replace(' ','T').replace(/(\.\d{3})\d+(Z)$/,'$1$2'));if(!Number.isFinite(date.getTime()))return text;const pad=n=>String(n).padStart(2,'0');return String(date.getFullYear()).padStart(4,'0')+'-'+pad(date.getMonth()+1)+'-'+pad(date.getDate())+' '+pad(date.getHours())+':'+pad(date.getMinutes())+':'+pad(date.getSeconds());}
function entryType(entry) {
  const dot = entry.leaf.lastIndexOf('.');
  return dot < 0 ? '' : entry.leaf.slice(dot + 1).toUpperCase();
}
const entryTimes = new WeakMap();
function entryTime(entry, key) {
  let times = entryTimes.get(entry);
  if (!times) entryTimes.set(entry, times = {});
  return times[key] ??= Date.parse((entry[key] || '').replace(' ', 'T'));
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
  return `<th scope="col" data-column="${key}" aria-sort="${direction}"><button type="button" data-sort="${key}">${label}</button></th>`;
}
function showColumns(){columnDraft=ArchiveColumns.normalize(columns);dialog('列设置','<div id="oa-column-list"></div>',btn('恢复默认','columns-reset')+btn('取消','close')+btn('应用','columns-apply','primary'));paintColumns();}
function paintColumns(){const box=$('oa-column-list');box.innerHTML=columnDraft.map((c,i)=>`<div class="oa-column-setting"><label><input type="checkbox" data-column-visible="${c.id}" ${c.visible?'checked':''} ${c.id==='name'?'disabled':''}>${c.label}</label><label>宽度 <input type="number" min="64" max="800" step="8" aria-label="${c.label}宽度" data-column-width="${c.id}" value="${c.width}"></label><button type="button" class="qa-button quiet" data-column-move="${c.id}" data-delta="-1" aria-label="${c.label}前移" ${i===0?'disabled':''}>↑</button><button type="button" class="qa-button quiet" data-column-move="${c.id}" data-delta="1" aria-label="${c.label}后移" ${i===columnDraft.length-1?'disabled':''}>↓</button></div>`).join('');}
function clearToast(){clearTimeout(toastTimer);$('qa-toast').hidden=true;}
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
function home(){
 return `<div class="oa-home-page">
 <section class="oa-home">
  <h1>压缩与解压</h1>
  <div class="oa-home-actions">
   <button type="button" id="oa-home-smart-drop" class="oa-home-smart-drop oa-home-action" data-action="home-smart">
    <span class="oa-home-symbol">${icon('folder-open')}</span>
    <span><strong>打开或解压</strong><small data-drop-label>拖入或选择文件，自动识别压缩包</small></span>
   </button>
   <button type="button" class="oa-home-action" data-action="compress">
    <span class="oa-home-symbol">${icon('archive')}</span>
    <span><strong>新建压缩包</strong><small>添加本地文件或文件夹</small></span>
   </button>
  </div>
 </section>
 ${s.recent.length?`<section class="oa-recent">
  <div class="oa-section-head"><h2>最近打开</h2>${btn('全部记录','recent-menu','quiet')}</div>
  ${s.recent.slice(0,4).map((path,i)=>`<button type="button" class="qa-recent-row" data-recent="${i}" title="${esc(path)}">
   <span class="oa-file-symbol">${icon('archive')}</span>
   <span class="qa-file-copy"><strong>${esc(filename(path))}</strong><small>${esc(path)}</small></span>${icon('chevron')}
  </button>`).join('')}
 </section>`:''}
 </div>`;
}

// Refreshing an archive replaces its entry array. Keep just the current
// folder's projection per live listing, so selection changes can reuse it.
const folderViews = new WeakMap();
const folderIndexes = new WeakMap();
function folderEntry(name, metadata) {
  return {...metadata, name, leaf:name.slice(0,-1).split('/').pop(),
    is_dir:true, size:0, size_known:true};
}
function archiveMemberPath(name) {
  let path = name.includes('\\') ? name.replaceAll('\\','/') : name;
  if(path.startsWith('./'))path=path.slice(2);
  if(path.startsWith('/')||path.endsWith('/'))path=path.replace(/^\/+|\/+$/g,'');
  if(path.includes('//'))path=path.replace(/\/{2,}/g,'/');
  return path;
}
function memberDirectory(name) {
  const path = name.replace(/\/$/,'');
  return path.slice(0,path.lastIndexOf('/')+1);
}
// Archive formats may omit parent directory records. Include each implied parent once.
function flatChildren(entries, folder) {
  const items = new Map();
  for (const entry of entries) {
    const path = archiveMemberPath(entry.name);
    if (!path.startsWith(folder) || path.length <= folder.length) continue;
    const leaf = path.slice(path.lastIndexOf('/')+1);
    if (entry.is_dir) items.set(path+'/', folderEntry(path+'/', entry));
    else items.set(path, entry.leaf===leaf && entry.name===path ? entry : {...entry, name:path, leaf, size_known:entry.size_known !== false});
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
function archiveIndex(entries) {
  let index = folderIndexes.get(entries);
  if (index) return index;
  const folders = new Map([['',new Map()]]);
  const paths = new Set(['']);
  let files = 0;
  for (let i=0; i<entries.length; i++) {
    const entry = entries[i];
    if (!entry.is_dir) files++;
    const path = archiveMemberPath(entry.name);
    if (!path) continue;
    let end = path.length;
    let directory = entry.is_dir;
    let explicit = true;
    while (end > 0) {
      const separator = path.lastIndexOf('/', end - 1);
      const parent = path.slice(0, separator + 1);
      const name = path.slice(0, end) + (directory ? '/' : '');
      let children = folders.get(parent);
      if (!children) folders.set(parent, children = new Map());
      if (children.has(name) && !(directory && explicit)) break;
      children.set(name, explicit ? i : -1);
      if (directory) paths.add(name);
      end = separator;
      directory = true;
      explicit = false;
    }
  }
  index = {folders,paths,files,directories:paths.size-1};
  folderIndexes.set(entries, index);
  return index;
}
function folderChildren(entries, folder) {
  const projection = [];
  for (const [name,index] of archiveIndex(entries).folders.get(folder) || []) {
    if (index===-1 || entries[index].is_dir) {
      projection.push(folderEntry(name, entries[index]));
    } else {
      const entry=entries[index];
      const leaf = name.slice(folder.length);
      if (entry.leaf===leaf && entry.name===name) {
        projection.push(entry);
      } else {
        const projected={};
        for(const key of Object.keys(entry)) if(key!=='name') projected[key]=entry[key];
        projection.push(Object.assign(projected,{name,leaf,is_dir:false,size_known:entry.size_known!==false}));
      }
    }
  }
  return projection;
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
 const viewport=archiveViewport();
 viewport.scrollTop=0;
 viewport.scrollLeft=0;
 rows();
}
function updateArchiveSearch(input){
 if(s.search===input.value)return;
 s.search=input.value;
 applyArchiveFilter();
}
function focusArchiveSearch(){const input=$('qa-search');input?.focus({preventScroll:true});input?.select();}
function clearArchiveSearch(focusRow=false){
 const input=$('qa-search');
 if(!input||!s.search)return;
 input.value='';
 updateArchiveSearch(input);
 if(!focusRow||!focusEntry(s.cursor))input.focus({preventScroll:true});
}
function searchWholeArchive(){
 if(!s.search||s.searchScope==='archive')return;
 s.searchScope='archive';
 $('oa-search-scope').value='archive';
 applyArchiveFilter();
 $('qa-search').focus({preventScroll:true});
}
function locateSearchResult(name){
 if(!s.search||(s.active&&!s.backgroundExtraction)||!positionsFor(visibleEntries()).has(name))return;
 const folder=memberDirectory(name);
 if(!archiveIndex(s.entries).folders.get(folder)?.has(name))return;
 let ancestor=folder;
 while(ancestor){s.collapsed.delete(ancestor);ancestor=memberDirectory(ancestor);}
 gotoArchiveFolder(folder,{name,select:true,focus:true});
}
function syncArchiveSearch(list){
 const status=$('oa-search-results'),message=$('oa-search-count'),clear=$('oa-search-clear');
 const searching=!!s.search;
 status.hidden=clear.hidden=!searching;
 $('oa-search-whole').hidden=!searching||!!list.length||!s.folder||s.searchScope==='archive';
 $('oa-search-locate').hidden=!searching||!list.length;
 if(!searching)return;
 const scope=s.searchScope==='archive'?'整个压缩包':'当前目录及子目录';
 const text=`找到 ${list.length} 项 · ${scope}`;
 if(message.textContent!==text)message.textContent=text;
}
function breadcrumbs(){
 let path='';
 return [`<button type="button" data-breadcrumb="" ${s.folder?'':'aria-current="location"'} title="${esc(s.archive)}">${esc(filename(s.archive))}</button>`,
  ...s.folder.split('/').filter(Boolean).map(part=>{
   path+=part+'/';
   return `<button type="button" data-breadcrumb="${esc(path)}" ${path===s.folder?'aria-current="location"':''} title="${esc(path)}">${esc(part)}</button>`;
  })].join('<span aria-hidden="true">/</span>');
}
function archiveViewport(){return $('oa-archive-scroll')||main;}
function gotoArchiveFolder(path,{name=null,select=false,focus=false}={}){
 s.folder=path;s.search='';s.selected=new Set(select?[name]:[]);s.anchor=select?name:null;s.cursor=name;
 s.page='files';s.view='archive';render();
 const viewport=archiveViewport();viewport.scrollTop=0;viewport.scrollLeft=0;
 updateColumnScroll();
 if(focus&&!focusEntry(s.cursor))root.querySelector('.oa-breadcrumb [aria-current=location]')?.focus({preventScroll:true});
}
function archive(){
 if(!s.archive)return home();
 const details=s.listStyle!=='icons',visibleColumns=columns.filter(column=>column.visible);
 const columnWidths=visibleColumns.map(column=>column.id==='name'?`min(${column.width}px, var(--oa-name-limit))`:`${column.width}px`);
 const colgroup=details?`<colgroup><col class="oa-row-check">${columnWidths.map(width=>`<col style="width:${width}">`).join('')}</colgroup>`:'';
 const tableSize=details?`style="min-width:calc(var(--oa-check-width) + ${columnWidths.join(' + ')})"`:'';
 return `<section class="oa-archive-browser" aria-label="压缩包浏览">
  ${nestedBanner()}
  <div class="qa-path">
   <button type="button" class="qa-button quiet" data-action="up" aria-label="上级目录" title="上级目录" ${s.folder?'':'disabled'}>${icon('arrow-up')}</button>
   <nav class="oa-breadcrumb" aria-label="包内路径">${breadcrumbs()}</nav>
   <div class="oa-archive-search" role="search" aria-label="包内查找">
    <div class="oa-search">${icon('search')}<input type="text" id="qa-search" aria-label="查找文件" aria-controls="qa-rows" aria-keyshortcuts="Control+F" placeholder="查找名称或路径" title="查找名称或路径 (Ctrl+F)" value="${esc(s.search)}">
    <button type="button" id="oa-search-clear" class="qa-button quiet" data-action="clear-archive-search" aria-label="清除查找" title="清除查找 (Esc)" hidden>${icon('x')}</button></div>
    <select id="oa-search-scope" aria-label="查找范围">${opts([['folder','当前目录及子目录'],['archive','整个压缩包']],s.searchScope||'folder')}</select>
   </div>
  </div>
  ${viewControls()}
  <div id="oa-search-results" class="oa-search-results" hidden>
   <span id="oa-search-count" role="status"></span>
   <button type="button" id="oa-search-whole" class="qa-button quiet" data-action="search-whole-archive" hidden>查找整个压缩包</button>
   <button type="button" id="oa-search-locate" class="qa-button quiet" data-action="locate-archive-result" aria-keyshortcuts="Control+Enter" hidden>转到所在目录</button>
  </div>
  <div id="oa-archive-scroll" class="oa-archive-scroll">
   <div class="oa-table-wrap" style="--oa-icon-size:${s.iconSize}px">
    <table ${tableSize} ${details&&visibleColumns[0]?.id==='name'?'data-pinned-name':''} role="grid" aria-label="压缩包文件" aria-multiselectable="true" class="qa-table ${details?'':'oa-icons'}">
     ${colgroup}<thead><tr aria-rowindex="1"><th scope="col"><input id="qa-all" type="checkbox" aria-label="全选当前列表"></th>${visibleColumns.map(c=>sortableColumn(c.id,c.label)).join('')}</tr></thead>
     <tbody id="qa-rows"></tbody>
    </table>
   </div>
   <div id="oa-archive-empty" class="qa-empty oa-archive-empty" hidden></div>
  </div>
  <footer class="oa-archive-selection">
   <div id="qa-selection" class="qa-selection" role="status"></div>
   <div id="oa-selection-actions" class="oa-selection-actions" hidden><span id="oa-selected-count"></span><div class="qa-actions">${btn('解压选中','extract-selected','quiet','folder-output')}${btn('重命名','rename','quiet')}${btn('删除','delete','quiet','x')}</div></div>
  </footer>
 </section>`;
}

const windowThreshold = 256;
function fileRow(e,index){
const label=s.flatView&&!s.search?e.name.slice(s.folder.length).replace(/\/$/,''):e.leaf;
const parent=s.search?memberDirectory(e.name).replace(/\/$/,'')||'根目录':'';
const nameCell=`<td data-column="name"><button type="button" class="qa-filebtn" tabindex="${e.name===s.cursor?0:-1}" data-entry="${esc(e.name)}" title="${esc(s.search?e.name:label)}"><span class="oa-file-symbol ${e.is_dir?'folder':''}" ${!e.is_dir&&isImage(e.name)&&s.listStyle==='icons'?`data-thumbnail="${esc(e.name)}" aria-label="图片缩略图"`:''}>${icon(e.is_dir?'folder':'file')}</span><span class="oa-archive-name"><bdi class="oa-archive-leaf">${esc(label)}</bdi>${parent?`<small class="oa-archive-member-directory"><bdi>${esc(parent)}</bdi></small>`:''}</span></button></td>`;
const cell=id=>{if(id==='name')return nameCell;let value='';if(id==='size')value=e.is_dir?'':entrySize(e);else if(id==='packed_size')value=e.is_dir?'':ArchiveColumns.packedKnown(e)?size(e.packed_size||0):'未提供';else if(id==='type')value=e.is_dir?'文件夹':entryType(e)?entryType(e)+' 文件':'文件';else if(['modified','created','accessed'].includes(id))value=displayModified(e[id])||'未提供';else if(id==='encrypted')value=e.encrypted?'是':'未标记';else value=e[id]||'未提供';if(id==='type'&&e.encrypted)value+=' 🔒';return `<td data-column="${id}" title="${esc(value)}">${esc(value)}</td>`;};
return `<tr aria-rowindex="${index+2}" data-row="${esc(e.name)}" aria-selected="${s.selected.has(e.name)}" class="${s.selected.has(e.name)?'selected':''}"><td><input type="checkbox" tabindex="-1" data-select="${esc(e.name)}" aria-label="选择 ${esc(s.search?e.name:label)}" ${s.selected.has(e.name)?'checked':''}></td>${s.listStyle==='icons'?nameCell:columns.filter(c=>c.visible).map(c=>cell(c.id)).join('')}</tr>`;}

function releaseListingWindow() { listingWindow?.stop(); listingWindow = null; }
function rows() {
  const body = $('qa-rows'), table = body.closest('table'), viewport = archiveViewport(), list = visibleEntries();
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
      listingWindow = new ArchiveIconWindow(body, viewport, list, fileRow, () => positions.get(s.cursor) ?? -1, watchTile, unwatchTile);
    } else listingWindow.paint();
  } else if (windowed) {
    if (!listingWindow) listingWindow = new ArchiveRowWindow(body, viewport, list, fileRow, () => positions.get(s.cursor) ?? -1, columns.filter(c=>c.visible).length+1, JSON.stringify(columns));
    else listingWindow.paint();
  } else {
    body.innerHTML = list.map(fileRow).join('');
  }
  const empty=$('oa-archive-empty');
  empty.hidden=!!list.length;
  if(!list.length)empty.textContent=s.search?'没有匹配的文件或文件夹':s.entries.length?'此文件夹为空':'此压缩包为空';
  syncArchiveSearch(list);
  refreshSelection();
  updateColumnScroll();
  if (s.listStyle !== 'icons') stopThumbnailObserver(); else if (!windowed) observeThumbnails();
  if (restore && positions.has(restore)) focusEntry(restore);
}

const ordinaryRetries=new Map();
const deepSessionJobs=new Map();
const phaseLabels={preflight:'检查输入',listing:'读取目录',validating:'检查文件和目标',
 decoding:'验证并解码',decompressing:'解压压缩流',extracting:'解压文件','staging-files':'写入已验证文件','publishing-files':'保存已验证文件',
 'preparing-extraction':'检查目标路径与临时空间','verifying-stage':'核对临时文件',
 'staged-directory-metadata':'核对临时目录属性','preparing-publication':'准备保存解压文件',
 'output-directory-metadata':'恢复输出目录属性','cleaning-stage':'清理临时文件',
 'verifying-files':'验证归档文件','deep-inspect':'识别压缩包','deep-publish':'保存深度解压结果',recycling:'正在移入回收站',
 copying:'复制并验证文件',publishing:'保存输出',compressing:'正在压缩',testing:'正在检查完整性',complete:'正在确认结果'};
function jobStatus(j){
 if(j.cancelRequested&&j.status==='running')return '正在取消';
 if(j.deep?.session_closed)return '会话已结束';
 if(j.status==='failed')return '失败';
 if(deepCanContinue(j))return '待继续';
 if(j.deep?.cancelled||j.status==='cancelled')return '已取消';
 if(j.partial)return '部分完成';
 if(j.deep&&!j.deep.finished)return '已暂停';
 return {running:'处理中',complete:'已完成'}[j.status];
}
function currentDeepReport(id){
 const job=deepSessionJobs.get(id);
 return job?.deep??job?.deepRecovery;
}
function recordDeepReport(job,report){
 job.deep=report;
 delete job.deepRecovery;
 if(report.finished||report.session_closed)deepSessionJobs.delete(report.deep_session);
 else deepSessionJobs.set(report.deep_session,job);
}
const deepReportPending=report=>!!report&&!report.finished&&!report.session_closed;
const deepCanContinue=job=>job.followedBy==null&&job.status!=='running'&&deepReportPending(job.deep??job.deepRecovery);
function needsAttention(job){
 if(job.followedBy!=null||job.deep?.session_closed)return false;
 return job.status==='failed'||!!job.partial||deepCanContinue(job)||ordinaryRetries.has(job.id);
}
function updateJobProgress(j,v){
 const {phase,message,context,deep,work}=v.progress;
 const sampled=QingProgress.sample(v.progress,j.progressSample);
 Object.assign(j,{status:v.status,phase,percent:sampled.value,progressSample:sampled,message,context,deepProgress:deep,work,error:v.error});
 QingProgress.observeOperation(j);
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
 const parts=[];
 if(work.items_total>0)parts.push(`${work.items_done.toLocaleString()} / ${work.items_total.toLocaleString()} 个文件`);
 if(work.bytes_total!=null)parts.push(`${size(work.bytes_done)} / ${size(work.bytes_total)}`);
 const bps=j.rate?.bps,remaining=work.bytes_total-work.bytes_done;
 if(j.status==='running'&&!j.cancelRequested&&bps>0&&remaining>0&&performance.now()-j.rate.time<2500){
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
 const details=inline===true,compact=details&&!!j.archives;
 const running=j.status==='running',message=j.error?taskError(j.error):j.summary||j.message||(j.work?'':'正在处理…');
 const phase=running?(j.archives?compressionStageLabel(j):phaseLabels[j.phase]||(j.phase?'正在处理':'等待引擎响应')):jobStatus(j);
 const badge=j.status==='failed'?'failed':deepCanContinue(j)?'attention':j.deep?.cancelled||j.status==='cancelled'?'cancelled':
  j.partial||deepReportPending(j.deep)?'attention':j.status;
 const progressJob=!running&&j.overall&&!j.archives?{...j,overall:null}:j;
 const percent=QingProgress.value(progressJob);
 const progressLabel=QingProgress.label(progressJob,percent);
 const stageMetric=running&&j.overall&&!j.cancelRequested?QingProgress.label({status:'running'},j.percent):'';
 const display=running?({'Verifying all archive entries':'正在验证全部归档条目',
  'Decoding archive into temporary storage':'正在解码到临时位置',
  'Decoding RAR into temporary storage on the output volume':'正在解码到目标盘的临时目录',
  'Decoding RAR into the system temporary directory':'正在解码到系统临时目录',
  'Reading archive directory':'正在读取压缩包目录',
  'Opening compressed TAR stream':'正在打开压缩的 TAR 数据流',
  'Reading compressed TAR stream':'正在读取压缩的 TAR 数据流',
  'Verifying archive and staging selected files':'正在验证压缩包并暂存选中内容',
  'Verifying complete archive in bounded memory':'正在验证压缩包内容',
  'Verifying complete solid archive in bounded memory':'正在验证固实压缩包',
  'Creating archive':'','Checking archive integrity':'','Archive created':'正在确认已创建的文件','Archive integrity verified':'正在确认检查结果',
  'Extraction complete':'正在确认解压结果'})[message]??message:message;
 return `<div data-job-card="${j.id}" data-attention="${needsAttention(j)}" class="qa-jobcard${compact?' oa-compression-inline':''}">
  ${details?'':`<div class="qa-jobhead"><strong>${esc(j.label)}</strong><span class="qa-badge" data-state="${badge}">${esc(jobStatus(j))}</span></div>`}
  ${j.path?`<p class="qa-sub">${esc(j.displayPath||j.path)}</p>`:''}
  ${!details&&(running||percent!==null)?`<p class="qa-sub">${esc(phase)} · ${progressLabel}${stageMetric?' · '+stageMetric:''}${j.startedAt==null?'':' · '+elapsedTime(j)}</p>${QingProgress.markup(`data-job-progress="${j.id}" aria-label="${esc(phase+' · '+progressLabel)}"`,percent,QingProgress.activity(j))}`:''}
  ${running&&j.archives?.length>1?`<p class="qa-sub">已处理 ${j.current} 个 · 后续待处理 ${j.archives.length-j.current-1} 个</p>`:''}
  ${running&&j.context?`<div class="qa-jobresult">${esc(j.context)}</div>`:''}
  ${running&&j.work?`<p class="qa-sub">${esc(jobMeasurements(j))}</p>`:''}
  ${display?`<div class="qa-jobresult"${j.status==='failed'&&s.view==='jobs'?' role="alert"':''}>${j.work?.items_done>0&&running?'最近完成 · ':''}${esc(display)}</div>`:''}
  ${j.followedBy!=null?'<p class="qa-sub">已有后续处理记录。</p>':''}
  ${j.archives?compressionResults(j,compact):''}
  ${j.timeWarning?`<details class="qa-sub"><summary>${esc(j.timeWarning)}</summary>${j.timeReasons.map(reason=>`<p>${esc(reason)}</p>`).join('')}</details>`:''}
  ${details?'':running?(j.phase==='recycling'?'<span class="qa-sub">请等待回收完成</span>':j.cancelRequested?'<span class="qa-sub">正在取消…</span>':btn(s.batchRunning&&j.id===s.active?'停止批量处理':'取消任务','canceljob','quiet')):j.outputPath&&!j.deep?`<button class="qa-button" data-job-output="${j.id}" data-action="reveal-output">打开输出位置</button>`:''}
  ${ordinaryRetries.has(j.id)?btn(j.retryLabel,'extract-retry-'+j.id,'primary'):''}
  ${deepExtract.jobActions(j)}
 </div>`;
}
function jobs(){
 const last=s.jobs.at(-1),result=s.batchResult;
 if(result&&result.anchor===last)return shellQueueStatus()+batchResultMarkup(result,true);
 return shellQueueStatus()+(last?jobCard(last):'<div class="qa-empty">还没有任务。</div>');
}
function renderInlineTask(){
 const host=$('oa-task-panel'),job=s.jobs.findLast(j=>j.id===s.inlineTask);
 if(!host)return;
 host.hidden=!job||!!job.inlineDismissed||s.page==='history';
 if(host.hidden)return;
 const result=s.batchResult?.lastJob===job?s.batchResult:null;
 const running=job.status==='running';
 const text=result?result.summary:running&&!job.cancelRequested?(job.archives&&(!job.phase||['compressing','testing'].includes(job.phase))
  ?compressionStageLabel(job):phaseLabels[job.phase]||(job.phase?'正在处理':'等待引擎响应')):jobStatus(job);
 const percent=result?null:QingProgress.value(job);
 const currentArchive=running?job.deepProgress:null;
 const path=result?'':currentArchive?.archive||job.path||job.archives?.[job.current]?.path||'';
 const headline=$('oa-task-toggle'),body=$('oa-task-body'),content=$('oa-task-content');
 const details=$('oa-task-details'),cancel=$('oa-task-cancel'),output=$('oa-task-output');
 const active=document.activeElement;
 const needsAttention=job.status!=='complete'||job.partial||job.archives||deepCanContinue(job)||job.deep?.recovery_path||shellQueuePaused;
 const detailsOpen=job.inlineDetails??(!running&&(result?result.attention:!!needsAttention));
 host.dataset.expanded=String(s.inlineExpanded);
 $('oa-task-title').textContent=result?result.title:job.label;
 const progressLabel=percent!==null||running&&!job.cancelRequested?QingProgress.label(job,percent):'';
 const stageMetric=running&&job.overall&&!job.cancelRequested?QingProgress.label({status:'running'},job.percent):'';
 $('oa-task-compact-phase').textContent=[text,progressLabel,stageMetric].filter(Boolean).join(' · ');
 $('oa-task-phase').textContent=stageMetric?`${text} · ${stageMetric}`:text;
 $('oa-task-metric').textContent=progressLabel;
 const file=$('oa-task-file');
 file.hidden=!path;
 file.textContent=(currentArchive?`第 ${currentArchive.depth} 层 · `:'')+filename(path);
 file.title=path;
 for(const id of ['oa-task-compact-progress','oa-task-progress']){
  const progress=$(id);
  progress.hidden=(!running&&percent===null)||(id==='oa-task-compact-progress'?s.inlineExpanded:!s.inlineExpanded);
  QingProgress.render(progress,percent,QingProgress.activity(job));
 }
 cancel.hidden=!running||job.phase==='recycling'||!!job.cancelRequested;
 cancel.textContent=s.batchRunning&&job.id===s.active?'停止批量处理':'取消任务';
 output.hidden=running||!(result?result.hasOutput:job.outputPath);
 output.dataset.jobOutput=result?'':String(job.id);
 if(result)output.dataset.action='batch-output';else delete output.dataset.action;
 output.textContent=result?'打开输出目录':job.deep?.outputs?.length>1?'打开首个结果':'打开输出位置';
 $('oa-task-dismiss').hidden=running;
 if(running&&(!s.active||s.active===job.id||job===s.compressionJob))$('qa-status-text').textContent=text;
 else if(!s.active)$('qa-status-text').textContent='就绪';
 headline.setAttribute('aria-expanded',String(s.inlineExpanded));
 body.hidden=!s.inlineExpanded;
 details.setAttribute('aria-expanded',String(!!detailsOpen));
 details.textContent=detailsOpen?'收起详情':'任务详情';
 content.hidden=!detailsOpen;
 if(s.inlineExpanded&&active===cancel&&cancel.hidden)(output.hidden?details:output).focus({preventScroll:true});
 if(!s.inlineExpanded||!detailsOpen)return;
 const html=shellQueueStatus()+(result?batchResultMarkup(result,true,false):jobCard(job,true));
 const jobId=String(job.id),replacingJob=content.dataset.jobId!==jobId;
 if(!replacingJob&&content.renderedHtml===html)return;
 if(!replacingJob&&running&&taskDetailSelection(content))return;
 const focus=content.contains(active)?active:null;
 const expanded=new Set(replacingJob?[]:[...content.querySelectorAll('details[open] > summary')].map(summary=>summary.textContent));
 content.innerHTML=html;
 content.renderedHtml=html;
 content.dataset.jobId=jobId;
 if(replacingJob)content.scrollTop=0;
 for(const summary of content.querySelectorAll('details > summary'))if(expanded.has(summary.textContent))summary.parentElement.open=true;
 if(focus){
  const next=[...content.querySelectorAll('button:enabled, summary')].find(control=>control.tagName===focus.tagName
   &&control.textContent===focus.textContent&&Object.entries(focus.dataset).every(([key,value])=>control.dataset[key]===value));
  (next||details).focus({preventScroll:true});
 }
}
function taskDetailSelection(content){
 const selection=document.getSelection();
 return !!selection&&!selection.isCollapsed&&content.contains(selection.anchorNode)&&content.contains(selection.focusNode);
}
function pageHeading(title,backLabel='返回工具箱',backAction='tools-home'){
 return `<div class="oa-page-back">${btn(backLabel,backAction,'quiet','arrow-up')}</div><div class="qa-heading"><h1>${esc(title)}</h1></div>`;
}
const batchDraft={inputs:'',dest:'',operation:'test',overwrite:null};
function batchPage(){
 return pageHeading('批量处理')+`<div id="oa-batch-result" aria-live="polite">${batchResultsMarkup()}</div>
 <section class="oa-batch-form">
  <div class="qa-actions oa-source-heading">
   <label for="qa-batch-inputs">压缩包路径（每行一个）</label><span id="qa-batch-count" class="qa-sub" role="status"></span>
   ${btn('添加压缩包…','batch-pick')}
  </div>
  <textarea id="qa-batch-inputs" rows="3" spellcheck="false">${esc(batchDraft.inputs)}</textarea>
  <label class="qa-field">操作<select id="qa-batch-op">${opts([['test','检查完整性'],['extract','批量解压']],batchDraft.operation)}</select></label>
  <div id="qa-batch-target" hidden>
   <div class="qa-field"><label for="qa-batch-dest">保存到</label><div class="qa-inlinefield oa-extract-target"><input id="qa-batch-dest" value="${esc(batchDraft.dest)}" spellcheck="false">${btn('浏览…','batch-dest')}</div></div>
   <label class="qa-field oa-extract-retry-conflict">同名文件<select id="qa-batch-conflict" aria-describedby="qa-batch-conflict-help">${conflicts(batchDraft.overwrite??s.overwrite)}</select></label>
   <p id="qa-batch-conflict-help" class="qa-sub oa-extract-hint"></p>
   <p class="qa-sub">预计输出目录</p>
   <div id="qa-batch-plan"></div>
  </div>
  <div class="qa-actions">${btn('检查完整性','batch-start','primary')}</div>
  <p class="qa-sub">某个压缩包处理失败后会继续下一项。</p>
 </section>`;
}
function batchResultMarkup(result,compact=false,showOutput=true){
 const saved=result===s.savedBatch;
 const review=result.attention&&compact?btn('查看批量结果','batch-result','quiet'):'';
 const output=showOutput&&result.hasOutput?btn('打开输出目录',saved?'batch-output-saved':'batch-output','quiet'):'';
 const unresolved=compact?[]:result.entries.filter(entry=>entry.status!=='complete');
 const details=unresolved.length?`<details${saved?' open':''}><summary>查看 ${unresolved.length} 项明细</summary>
  <ul class="oa-batch-plan">${unresolved.map(entry=>`<li><strong>${entry.index+1}. ${esc(filename(entry.path))} · ${entry.label}</strong>
  <span class="oa-extract-output">${esc(entry.path)}</span>${entry.status==='pending'&&entry.dest?`<span class="oa-extract-output">${esc(entry.dest)}</span>`:''}${entry.message?`<span class="qa-sub">${esc(entry.message)}</span>`:''}</li>`).join('')}</ul></details>`:'';
 const resume=unresolved.some(entry=>entry.status==='pending')?btn('继续未开始的项目',saved?'batch-resume-saved':'batch-resume','primary'):'';
 const clear=resume&&result.revision?btn('清除待继续项目','batch-discard','quiet'):'';
 const dest=!compact&&result.destination?`<div class="qa-sub oa-extract-output">输出目录 · ${esc(result.destination)}</div>`:'';
 const policy=resume&&result.operation==='extract'?`<p class="qa-sub">同名文件 · ${batchPolicies[result.overwrite]}</p>`:'';
 const warning=result.saveWarning?`<p class="qa-sub" role="status">${esc(result.saveWarning)}</p>`:'';
 const current=saved&&resume?'<p class="qa-sub">继续时会读取列表中路径上的当前压缩包。</p>':'';
 const keep=clear?'<p class="qa-sub">停止新的批量解压时，新批次的待继续项目会替换这份记录。</p>':'';
 return `<section class="oa-batch-outcome"><strong>${result.title}</strong><p>${result.summary}</p>${dest}${policy}${warning}${current}${details}<div class="qa-actions">${resume}${review}${output}${clear}</div>${keep}</section>`;
}
const batchLabels={complete:'成功',partial:'部分完成',failed:'失败',cancelled:'已取消',interrupted:'已中断',pending:'未开始'};
let batchResumeLoad,batchResumeSave,batchResumeClaim;
function describeBatch(plan,lastJob){
 const counts={complete:0,partial:0,failed:0,cancelled:0,interrupted:0,pending:0};
 for(const entry of plan.entries){counts[entry.status]++;entry.label=batchLabels[entry.status];}
 const summary=Object.entries(counts).filter(([,count])=>count).map(([status,count])=>batchLabels[status]+' '+count).join(' · ');
 return {...plan,summary,title:plan.restored?'待继续的批量解压':plan.operation==='extract'?'批量解压结果':'批量检查结果',
  attention:counts.complete!==plan.entries.length,hasOutput:plan.operation==='extract'&&plan.entries.some(entry=>entry.hasOutput),
  lastJob,anchor:s.jobs.at(-1)};
}
function savedBatch(record){
 if(!record)return null;
 const entries=record.plan.entries.map((entry,index)=>({...entry,index}));
 return describeBatch({...record.plan,entries,operation:'extract',revision:record.revision,restored:true});
}
function batchResultsMarkup(compact=false){
 const results=s.batchResult?[s.batchResult]:[];
 if(s.savedBatch&&s.savedBatch.revision!==s.batchResult?.revision)results.push(s.savedBatch);
 const problem=s.batchResumeProblem;
 const messages={
  damaged:'待继续记录已损坏。备份这份记录后，可以保存新的待继续项目。',
  unsupported:'此版本无法识别待继续记录。文件已保留，可用创建它的版本查看。仍可处理新批次。',
  unavailable:'暂时无法读取待继续项目。仍可处理新批次。',
 };
 const warning=problem?`<section><p class="qa-sub" role="status">${messages[problem]}</p>
  ${problem==='damaged'?btn('备份损坏记录','batch-backup-damaged','quiet'):''}</section>`:'';
 return warning+results.map(result=>batchResultMarkup(result,compact)).join('');
}
function renderBatchResults(){
 const host=$('oa-batch-result');
 if(host)host.innerHTML=batchResultsMarkup();
 if(s.page==='history')render();
}
function loadBatchResume(){
 if(batchResumeLoad)return batchResumeLoad;
 batchResumeLoad=invoke('load_batch_resume').then(({state,record})=>{
  s.batchResumeProblem=state==='damaged'||state==='unsupported'?state:null;
  s.savedBatch=savedBatch(record);
 }).catch(()=>{s.batchResumeProblem='unavailable';s.savedBatch=null;}).finally(()=>{batchResumeLoad=null;renderBatchResults();});
 return batchResumeLoad;
}
function sealBatchResume(plan){
 if(plan?.operation!=='extract'||!plan.entries.some(entry=>entry.status==='pending'))return Promise.resolve(true);
 if(!batchResumeSave)batchResumeSave=writeBatchResume(plan).finally(()=>{batchResumeSave=null;});
 return batchResumeSave;
}
async function writeBatchResume(plan){
 const entries=plan.entries.map(({path,dest,status,hasOutput})=>({path,dest,status:status==='running'?'interrupted':status,hasOutput:!!hasOutput}));
 await batchResumeLoad;
 try{
  const record=await invoke('save_batch_resume',{previous:s.savedBatch?.revision??null,plan:{destination:plan.destination,overwrite:plan.overwrite,entries}});
  s.savedBatch=savedBatch(record);
  plan.revision=record.revision;
  delete plan.saveWarning;
  s.batchResumeProblem=null;
  return true;
 }catch{
  plan.saveWarning='未能保存待继续项目。本次会话仍可继续。';
  return false;
 }
}
function showBatchResumeReview(result){
 if(!result)return;
 const pending=result.entries.filter(entry=>entry.status==='pending');
 dialog('确认继续批量解压',`<div id="oa-batch-resume-review" data-revision="${esc(result.revision)}">
  <p>继续时会读取以下路径上的当前压缩包。</p>
  <p>同名文件 · ${batchPolicies[result.overwrite]}</p>
  <p>继续后，目标中已有的同名文件会按原策略被替换。</p>
  <ol class="oa-batch-plan" aria-label="未开始的项目">${pending.map(entry=>`<li><strong>${entry.index+1}. ${esc(filename(entry.path))}</strong>
   <span class="oa-extract-output">压缩包 · ${esc(entry.path)}</span>
   <span class="oa-extract-output">输出目录 · ${esc(entry.dest)}</span></li>`).join('')}</ol>
  </div>`,btn('返回','close')+btn('按替换策略继续','batch-confirm-resume','danger'),'oa-extraction-dialog');
}
async function confirmBatchResume(){
 const revision=$('oa-batch-resume-review')?.dataset.revision;
 const reviewed=s.savedBatch;
 if(!revision||reviewed?.revision!==revision)throw Error('待继续项目已更改，请返回并重新查看。');
 await useBatchResume(reviewed);
}
async function backupDamagedBatchResume(){
 let backup;
 try{backup=await invoke('backup_damaged_batch_resume');}
 catch{throw Error('未能备份损坏记录。请稍后重试。');}
 await loadBatchResume();
 showPreservationNotice('已备份损坏记录。可以保存新的待继续项目。备份文件 '+backup);
}
async function claimSavedBatch(result,discard){
 if(!discard)await invoke('set_batch_resume_running',{active:true});
 try{
  await batchResumeLoad;
  let claimed;
  try{claimed=await invoke('take_batch_resume',{revision:result.revision});}
  catch{throw Error('暂时无法更新待继续项目。请稍后重试。');}
  if(s.savedBatch?.revision===result.revision)s.savedBatch=null;
  if(s.batchResult?.revision===result.revision)s.batchResult=null;
  if(!claimed){await loadBatchResume();throw Error('其他窗口已处理这批待继续项目。');}
  return {...claimed.plan,operation:'extract',entries:claimed.plan.entries.map((entry,index)=>({...entry,index,message:result.entries[index]?.message}))};
 }catch(error){
  if(!discard)await invoke('set_batch_resume_running',{active:false});
  throw error;
 }
}
async function useBatchResume(result,discard=false){
 if(!result)return;
 if(s.active||s.batchRunning)throw Error('已有任务正在运行。');
 if(!result.revision){if(!discard)await runBatch(result);return;}
 let plan;
 if(!discard)s.batchStop=false;
 s.active='starting';
 try{
  const claim=claimSavedBatch(result,discard);
  if(!discard)batchResumeClaim=claim;
  plan=await claim;
 }finally{batchResumeClaim=null;s.active=null;}
 if(discard)renderBatchResults();
 else await runBatch(plan,true);
}

async function saveBatchBeforeExit(box){
 s.batchStop=true;
 if(batchResumeClaim)await batchResumeClaim.catch(()=>{});
 const plan=s.activeBatchPlan||(s.batchResult?.saveWarning?s.batchResult:null);
 if(!plan)return true;
 plan.closing=true;
 if(box?.dataset.withoutBatchResume==='true'||await sealBatchResume(plan))return true;
 plan.closing=false;
 if(!box)return true;
 box.dataset.withoutBatchResume='true';
 box.querySelector('p').textContent='未能保存待继续项目。可以留在本次会话处理，或退出并放弃本批的待继续记录。';
 box.querySelector('[data-action=cancel-exit]').textContent='放弃记录并退出';
 return false;
}
async function runBatch(plan,guarded=false){
 if(s.active||s.batchRunning)throw Error('已有任务正在运行。');
 s.batchResult=null;
 if(!guarded)s.batchStop=false;
 s.batchRunning=true;
 s.activeBatchPlan=plan;
 const counter={done:plan.entries.filter(entry=>entry.status==='complete').length,total:plan.entries.length};
 let lastJob;
 const host=$('oa-batch-result');
 if(host)host.replaceChildren();
 try{
  if(plan.operation==='extract'&&!guarded){
   await invoke('set_batch_resume_running',{active:true});
   guarded=true;
  }
  close();s.view='jobs';render();
  for(const entry of plan.entries){
   if(s.batchStop)break;
   if(entry.status!=='pending')continue;
   entry.status='running';
   let current;
   try{
    await run({operation:plan.operation,path:entry.path,dest:entry.dest,options:{overwrite:plan.overwrite}},
     `批量 ${entry.index+1}/${counter.total} · ${plan.operation==='test'?'检查':'解压'}`,
     {batch:counter,onStart:job=>{current=job;lastJob=job;}});
    entry.status=current.partial?'partial':current.status;
    entry.message=entry.status==='failed'?taskError(current.error):current.partial?current.summary:'';
    entry.hasOutput=!!current.outputPath;
   }catch(e){
    entry.status=current?.status==='cancelled'?'cancelled':'failed';
    entry.message=taskError(e);
   }
  }
 }finally{
  const anchor=s.jobs.at(-1);
  if(plan.closing)await batchResumeSave;
  else await sealBatchResume(plan);
  try{if(guarded)await invoke('set_batch_resume_running',{active:false});}
  finally{
   s.activeBatchPlan=null;
   s.batchRunning=false;
   s.batchResult={...describeBatch(plan,lastJob),anchor};
   renderBatchResults();
   renderProgress();
  }
 }
}
function batchExtractionTarget(dest,path,index){
 return dest+'\\'+(index+1)+'-'+filename(path).replace(/\.[^.]+$/,'');
}
function updateBatchPage(){
 const input=$('qa-batch-inputs');
 if(!input)return;
 const paths=lines(input.id),operation=$('qa-batch-op').value,destination=$('qa-batch-dest'),dest=destination.value.trim();
 const extract=operation==='extract';
 Object.assign(batchDraft,{inputs:input.value,dest:destination.value,operation});
 $('qa-batch-count').textContent=paths.length+' 个压缩包';
 $('qa-batch-target').hidden=!extract;
 const submit=main.querySelector('[data-action=batch-start]');
 submit.textContent=extract?'开始批量解压':'检查完整性';
 submit.disabled=!paths.length||(extract&&!dest);
 if(!extract)return;
 updateExtractConflict('qa-batch-conflict',false,'发现同名文件时停止当前压缩包，并继续下一项。');
 $('qa-batch-plan').innerHTML=paths.length&&dest?`<ul class="oa-batch-plan" aria-label="预计输出目录">${paths.map((path,i)=>
  `<li><strong title="${esc(path)}">${esc(filename(path))}</strong><span class="oa-extract-output">${esc(batchExtractionTarget(dest,path,i))}</span></li>`).join('')}</ul>`:
  `<p class="qa-sub">${paths.length?'请填写目标文件夹。':'请添加压缩包。'}</p>`;
}
function profilesPage(){return pageHeading('压缩预设')+`<p class="qa-sub">保存常用压缩参数，创建时一键载入。不会保存密码。</p><label class="qa-field">预设名称<input id="qa-profile-name" maxlength="80"></label><div class="qa-fieldrow"><label class="qa-field">格式<select id="qa-profile-format">${opts(formats,'zip')}</select></label><label class="qa-field">压缩级别<select id="qa-profile-level">${opts([[0,'仅存储'],[1,'快速'],[5,'标准'],[9,'最大']],5)}</select></label></div>${btn('保存预设','profile-save','primary')}<div class="qa-recent">${s.profiles.map((p,i)=>`<div class="qa-recent-row"><span class="qa-file-copy">${esc(p.name)} · ${esc(p.format)} · ${p.level}</span><button class="qa-button" data-profile="${i}">使用</button><button class="qa-button" data-profile-delete="${i}">删除</button></div>`).join('')}</div>`;}
function settings(){return `<div class="qa-heading"><h1>设置</h1></div><section class="qa-setting-group oa-app-info"><img src="app-icon.png" alt="" width="44" height="44"><div><strong>qing-zip</strong><p id="oa-version">版本 ${esc(appVersion||'…')}</p></div></section><section class="qa-setting-group"><h3>外观</h3><div class="qa-setting-row"><div>应用主题</div><select id="qa-theme" aria-label="应用主题">${opts([['system','跟随系统'],['light','浅色'],['dark','深色']],s.theme)}</select></div><div class="qa-setting-row"><div>窗口材质<p>Mica 适用于 Windows 11。高对比度模式使用纯色。</p></div><select id="qa-material" aria-label="窗口材质">${opts([['acrylic','Acrylic · 通透'],['mica','Mica · 柔和'],['solid','纯色']],s.material)}</select></div><div class="qa-setting-row"><div>列表密度</div><select id="qa-density" aria-label="列表密度">${opts([['relaxed','舒适'],['compact','紧凑']],s.density)}</select></div></section><section class="qa-setting-group"><h3>解压习惯</h3><div class="qa-setting-row"><div>默认重名处理</div><select id="qa-defaultconflict" aria-label="默认重名处理">${conflicts(s.overwrite)}</select></div></section><section class="qa-setting-group"><h3>解压密码本</h3><div class="qa-setting-row"><div>管理解压时自动保存或手动添加的密码</div>${btn('打开密码本','password-book','quiet')}</div></section><section class="qa-setting-group" aria-labelledby="oa-shell-settings-title"><h3 id="oa-shell-settings-title" tabindex="-1">资源管理器右键设置</h3><label class="oa-switch-row"><span>文件夹右键显示“用 qing-zip 压缩”</span><input id="oa-folder-menu" class="oa-switch" role="switch" type="checkbox" ${folderMenuEnabled?'checked':''}></label><label class="oa-switch-row"><span>文件右键显示“用 qing-zip 压缩”</span><input id="oa-file-menu" class="oa-switch" role="switch" type="checkbox" ${fileMenuEnabled?'checked':''}></label>${btn('管理“使用 qing-zip 打开”入口','shell-open-entry','quiet')}</section><div class="qa-actions">${btn('文件关联设置','associations')}${btn('清空最近记录','clear-recent')}</div>`;}
const batchPolicies={error:'停止并报告冲突',rename:'自动重命名',skip:'跳过已有文件',overwrite:'替换已有文件'};
const conflicts=v=>opts(Object.entries(batchPolicies),v);
let passwordBookOrigin = 'files';
function openPasswordBook(){
 if(s.page==='password-book')return;
 passwordBookOrigin=s.page==='jobs'?'history':s.page;
 navigate('password-book');
}
function passwordBookPage(){
 const backLabels={
  files:s.archive?'返回压缩包':'返回文件',
  tools:'返回工具箱',settings:'返回设置',history:'返回任务',
  associations:'返回文件关联',batch:'返回批量处理',profiles:'返回压缩预设',
  tool:'返回'+toolContext?.title
 };
 return pageHeading('解压密码本',backLabels[passwordBookOrigin]||'返回','password-book-back')+passwordBook.pageMarkup;
}
const pagePositions=new Map();
let renderedPage='';
function rememberPage(focus=document.activeElement){
 if(!renderedPage||renderedPage.startsWith('archive:'))return;
 const previous=pagePositions.get(renderedPage);
 const inside=focus!==main&&main.contains(focus);
 const card=inside&&renderedPage==='history'?focus.closest('[data-job-card]'):null;
 const keys=card?['data-job-output','data-path','data-deep-action','data-action']:['id','data-launch-tool','data-view','data-action','data-recent'];
 const key=inside&&keys.find(name=>focus.hasAttribute(name));
 const control=key?'['+key+'="'+CSS.escape(focus.getAttribute(key))+'"]':'';
 const cardSelector=card?'[data-job-card="'+CSS.escape(card.dataset.jobCard)+'"]':'';
 pagePositions.set(renderedPage,{top:main.scrollTop,left:main.scrollLeft,
  focus:[cardSelector,control].filter(Boolean).join(' ')||previous?.focus});
}
function applyPagePosition(page,restoreFocus,position){
 const viewport=page.startsWith('archive:')?archiveViewport():main;
 viewport.scrollTop=position?.top||0;
 viewport.scrollLeft=position?.left||0;
 if(!restoreFocus||!overlay.hidden||page.startsWith('archive:'))return;
 const previous=position?.focus&&main.querySelector(position.focus);
 const target=(previous&&(page!=='history'||previous.getClientRects().length)?previous:null)
  ||(page==='history'&&position?.focus&&$('oa-history-attention'))||main.querySelector('h1')||main;
 if(!target.matches('button,input,select,textarea,a[href]'))target.tabIndex=-1;
 target.focus({preventScroll:true});
}
function render(){if(overlay.dataset.page==='close-confirm')return;releaseListingWindow();stopThumbnailObserver();rememberPage();
 const page=s.page==='files'?(s.archive?'archive:'+currentTabId:'home'):s.page==='tool'?'tool:'+toolContext.op+':'+toolContext.origin:s.page;
 const transition=page!==renderedPage;
 const focusTransition=transition&&renderedPage!=='';
 const preserveHistoryFocus=page==='history'&&!transition&&main.contains(document.activeElement);
 if(focusTransition&&s.inlineTask!=null){
  const previous=s.jobs.findLast(job=>job.id===s.inlineTask);
  if(previous&&previous.status!=='running')previous.inlineDismissed=true;
 }
 const currentTab=archiveTabs.find(tab=>tab.id===currentTabId);
 const viewport=page.startsWith('archive:')?archiveViewport():main;
 const position=page.startsWith('archive:')?(transition?{top:currentTab?.scrollTop,left:currentTab?.scrollLeft}:{top:viewport.scrollTop,left:viewport.scrollLeft}):pagePositions.get(page);
 renderedPage=page;
 const pages={settings,associations:associationPage,batch:batchPage,profiles:profilesPage,tools:toolbox,tool:toolPage,history:historyPage,'password-book':passwordBookPage};
 const keepTool=s.page==='tool'&&!transition&&$('qa-tools')?.src===toolContext.frameUrl;
 if(!keepTool&&(!['batch','password-book'].includes(s.page)||transition)){
  main.innerHTML=s.page==='files'?(s.archive?archive():home()):`<div class="oa-page">${(pages[s.page]||toolbox)()}</div>`;
  if(s.page==='tool'&&toolContext.scope==='archive')mountArchiveTool($('qa-tools'),toolContext);
  if(s.page==='password-book')passwordBook.mountPage(main);
  if(s.page==='batch')updateBatchPage();
 }
 renderChrome();if(s.page==='files'&&s.archive)rows();applyPagePosition(page,focusTransition||preserveHistoryFocus,position);updateColumnScroll();$('qa-status-text').textContent=s.active?'正在处理文件…':'就绪';renderInlineTask();
 const totals=s.page==='files'&&s.archive?archiveIndex(s.entries):null;
 $('oa-total').textContent=totals?`${totals.files} 个文件 · ${totals.directories} 个文件夹`:'';
 if(s.view==='jobs'){const html=jobs();if(!overlay.hidden&&overlay.dataset.page==='jobs'&&$('oa-dialog-content'))refreshJobContent(html);else{dialog('任务进度',`<div id="oa-dialog-content">${html}</div>`,btn('关闭','close'));overlay.dataset.page='jobs';}overlay.querySelectorAll('.qa-dialog-footer [data-action=close]').forEach(b=>b.disabled=!!s.active);}}

function navigate(v){if(!['files','archive','home','tools','tool','history','settings','associations','batch','profiles','jobs','password-book'].includes(v))return;if(homeIntake)stopHomeIntake();if($('qa-tools')?.contentWindow?.obzBusy){toast('请等待或取消当前工具任务。');return;}if(s.active&&v!=='jobs'&&!(s.backgroundExtraction&&['archive','files','history'].includes(v))){toast('请先等待或取消当前任务。');return;}if(renderedPage==='archive:'+currentTabId)saveCurrentTab();rememberPage();close(false);if(v==='archive'||v==='files'||v==='home'){s.page='files';s.view=s.archive?'archive':'home';if(v==='home'){closeArchiveTabs('all');return;}}else{s.page=v;s.view=v;}render();if(v==='batch'||v==='history')loadBatchResume();}

function dialog(title,body,footer,layout=''){clearToast();overlay.dataset.page='';returnFocus=document.activeElement;overlay.hidden=false;overlay.innerHTML=`<section class="qa-dialog ${layout}" role="dialog" aria-modal="true" aria-labelledby="qa-dialog-title"><div class="qa-dialog-header"><h2 id="qa-dialog-title">${esc(title)}</h2>${btn('关闭','close','quiet')}</div>${layout?`<div class="oa-dialog-body">${body}</div>`:body}<div id="qa-formerror" class="qa-error" role="alert"></div><div class="qa-dialog-footer">${footer}</div></section>`;root.querySelector('.qa-shell').inert=true;if($('oa-task-panel'))$('oa-task-panel').inert=true;$('oa-menu').inert=true;$('oa-toolbar').inert=true;$('oa-archive-tabs').inert=true;passwordBook.mountPickers(overlay);overlay.querySelector('input,textarea,button')?.focus();}
function close(restoreFocus=true){if($('qa-open-status')?.closest('.qa-dialog')?.getAttribute('aria-busy')==='true'){
 if(secondaryOpen){secondaryOpen.cancelRequested=true;if(secondaryOpen.id!==null)invoke('cancel_operation',{id:secondaryOpen.id}).catch(error);return;}
 if(s.jobs.some(job=>job.id===s.active&&job.operation==='list')||s.active==='starting'&&!s.backgroundExtraction){action('canceljob').catch(error);return;}
}if(overlay.dataset.page==='home-inspecting'&&homeIntake){stopHomeIntake();return;}if($('qa-edit-inputs')?.closest('.qa-dialog').getAttribute('aria-busy')==='true'&&overlay.dataset.page!=='close-confirm')return;if($('qa-property-status')&&Number.isInteger(s.active))invoke('cancel_operation',{id:s.active}).catch(()=>{});if(pendingNestedOpen&&!openingArchive&&!s.active){const orphan=pendingNestedOpen.session;pendingNestedOpen=null;if(orphan)invoke('close_nested',{ids:[orphan.id],discard:false}).catch(error);}hideMenu();if($('qa-tools')?.contentWindow?.obzBusy){toast('请先等待或取消当前工具任务。');return;}if(s.active&&s.view==='jobs'&&!s.backgroundExtraction){if(overlay.dataset.page==='close-confirm')action('keep-running').catch(error);return;}if(overlay.dataset.page==='home-review')homeReview=null;overlay.hidden=true;overlay.replaceChildren();overlay.dataset.page='';root.querySelector('.qa-shell').inert=false;if($('oa-task-panel'))$('oa-task-panel').inert=false;$('oa-menu').inert=false;$('oa-toolbar').inert=false;$('oa-archive-tabs').inert=false;if(s.view==='jobs')s.view=s.page==='files'?(s.archive?'archive':'home'):s.page;if(restoreFocus){if(returnFocus?.isConnected&&returnFocus.getClientRects().length)returnFocus.focus();else{main.tabIndex=-1;main.focus();}}}

function taskError(e){
 // String(Error(x)) is "Error: x"; the JavaScript class name is not part of the message.
 const text=String(e).replace(/^Error:\s*/,''),directoryError=archiveDirectoryError(text);
 if(directoryError)return directoryError;
 const blocked=/^destination conflict:\s*output parent is a file:\s*(.+)$/i.exec(text);
 if(blocked)return `解压需要创建文件夹，但这个位置已有同名文件。\n${blocked[1]}\n请选择其他保存位置后重试。`;
 const conflict=/^(?:Error:\s*)?destination conflict:\s*(.+)$/i.exec(text);
 if(conflict)return '目标位置已有同名项目。\n'+conflict[1];
 if(/^(?:Error:\s*)?incorrect password or damaged encrypted archive[.!。]?$/i.test(text))return '密码不正确，或加密压缩包已损坏。';
 if(/^(?:Error:\s*)?single-stream format requires exactly one regular file; create a TAR first for multiple files$/i.test(text)){
  return '所选格式只能压缩单个文件，不能直接压缩文件夹。请改用 TAR.GZ、ZIP 或 7z 等格式，或只保留文件来源。';
 }
 const exists=/^(?:Error:\s*)?(?:output archive already exists; use update to modify it|(?:ISO )?output (?:archive |volume )?already exists|output exists: (.+))$/i.exec(text);
 if(exists)return '目标位置已出现同名文件（可能在检查名称后由其他程序创建），已保留该文件，未覆盖。'+(exists[1]?'\n'+exists[1]:'')+'\n重试时会重新检查并选用其他名称，也可更改保存位置。';
 const engine=engineFileProblems(text);
 if(engine)return engine;
 if(/^(?:Error:\s*)?(?:operation cancelled|任务已取消)[.!。]?$/i.test(text))return '任务已取消';
 if(/^(?:Error:\s*)?写回失败，内层工作副本已保留，可重试或另存为：create private stage: [^\r\n]* \(os error (?:39|112)\)$/i.test(text)){
  return '磁盘空间不足。内层更改已保留，释放空间后可重试，也可另存为。';
 }
 if(/^(?:Error:\s*)?写回失败，内层工作副本已保留，可重试或另存为：operation cancelled[.!。]?$/i.test(text))return '任务已取消。内层工作副本已保留，可重试或另存为。';
 if(/^(?:Error:\s*)?写回失败，内层工作副本已保留，可重试或另存为：parent archive or its member changed since opening$/i.test(text)){
  return '外层压缩包或其中的文件已被修改。工作副本已保留，请从有未保存更改的标签另存为。';
 }
 return text;
}
// 7-Zip console failures carry a banner, scan log and progress before the
// WARNING/ERROR lines (also "Open ERROR:", "System ERROR:", "ERRORS:") that
// name the file and the reason. Keep only those lines.
function engineFileProblems(text){
 const failed=/^(?:Error:\s*)?archive engine failed \((-?\d+)\):([\s\S]*)$/i.exec(text);
 if(!failed)return '';
 const lines=failed[2].split(/\r?\n/).map(line=>line.trim()),found=[];
 const add=line=>{if(line&&!found.includes(line))found.push(line);};
 for(let i=0;i<lines.length;i++){
  const marker=/^(?:(?:System|Open)\s+)?(?:WARNINGS?|ERRORS?):\s*(.*)$/i.exec(lines[i]);
  if(!marker||/^\d+$/.test(marker[1]))continue; // "Errors: 1" is the closing count, not a reason.
  add(marker[1]);
  const next=lines.slice(i+1).find(Boolean);
  if(!marker[1]&&next)add(next);
  else if(next&&/^(?:[A-Za-z]:[\\/]|\\\\)/.test(next))add(next);
 }
 if(!found.length)return '';
 const locked=found.some(line=>/另一个程序正在使用|being used by another process|cannot access the file/i.test(line));
 return `归档引擎未能完成（代码 ${failed[1]}）：\n${found.join('\n')}`+(locked?'\n请关闭正在使用该文件的程序后重试。':'');
}
function showPreservationNotice(message){dialog('操作提示',`<p>${esc(message)}</p>`,btn('关闭','close'));}
function error(e){
 const message=taskError(e);
 if(message==='任务已取消'||message==='任务已取消。内层工作副本已保留，可重试或另存为。'){toast(message);return;}
 const field=!overlay.hidden&&$('qa-formerror');
 if(!field){toast(message);return;}
 const shown=overlay.dataset?.page==='jobs'&&overlay.querySelector('.qa-jobcard .qa-jobresult[role="alert"]');
 field.textContent=shown&&shown.textContent===message?'':message;
}
async function copySelectedFiles(){
 const names=[...s.selected];
 if(!names.length)throw Error('请选择至少一个文件或文件夹。');
 const request={operation:'extract',path:s.archive,options:{selected:names,overwrite:'error',password:s.password||null,codepage:s.codepage||null}};
 const dest=await invoke('clipboard_directory');
 try{
  const paths=names.map(name=>dest+'\\'+name.replaceAll('/','\\'));
  if(paths.some(path=>path.length>=32768)||paths.reduce((bytes,path)=>bytes+2*(path.length+1),22)>2*1024*1024)
    throw Error('系统文件列表包含过长路径或超过 2 MiB，请分批选择。');
  await run({...request,dest},'复制文件',{temporaryOutput:true});
  await invoke('publish_clipboard_export',{dest,paths});
 }finally{
  await invoke('discard_clipboard_export',{dest});
 }
 toast('文件已复制到剪贴板，可在资源管理器中粘贴。');
}
async function run(request,label,{silent=false,recycleAfter=false,startJob=null,temporaryOutput=false,onStart=null,onProgress=null,batch=null,deferProgress=false,retryFrom=null,displayPath=null}={}){if(s.active)throw new Error('已有任务正在运行，请先等待或取消。');const background=!silent&&!temporaryOutput&&['extract','deep-extract','deep-resume','deep-discard'].includes(request.operation);
 const startedAt=performance.now();
 s.backgroundExtraction=background;s.active='starting';
 if(background)updateToolbar();
 let id;
 try{
  id=await (startJob?startJob(request):invoke('start_operation',{request,recycleAfter,rememberPassword:!temporaryOutput&&passwordBook.automaticSavingEnabled()}));
 }catch(error){
  s.active=null;s.backgroundExtraction=false;
  if(background)updateToolbar();
  throw error;
 }
 const session=request.deep_session,previous=retryFrom??(session==null?null:deepSessionJobs.get(session));
 const j={id,label,path:request.path||previous?.path||'',displayPath,status:'running',percent:null,startedAt,background,operation:request.operation,overall:batch?{...batch}:null};
 if(previous){
  previous.followedBy=id;
  if(retryFrom)ordinaryRetries.delete(previous.id);
  if(session!=null){
   j.deepRecovery=previous.deep??previous.deepRecovery;
   deepSessionJobs.set(session,j);
  }
 }
 s.jobs.push(j);s.active=id;onStart?.(j);
 let autoExpand,progressTimer;
 const showQueuedProgress=()=>{
  if(s.active!==id||j.status!=='running'||s.view==='jobs')return;
  if(overlay.dataset.page==='close-confirm'){s.view='jobs';return;}
  if(!overlay.hidden)return;
  close();s.view='jobs';render();
 };
 if(background){
  close();s.inlineTask=id;s.inlineExpanded=false;s.inlineToggled=false;renderInlineTask();
  if(previous)renderHistoryJob(previous);
  renderHistoryJob(j);
  autoExpand=setTimeout(()=>{
   if(s.active===id&&!s.inlineToggled){s.inlineExpanded=true;renderInlineTask();}
  },300);
 }else if(!silent){
  if(deferProgress){
   updateToolbar();
   progressTimer=setTimeout(()=>{progressTimer=undefined;showQueuedProgress();},180);
  }else{close();s.view='jobs';render();}
 }try{
 if(batch&&s.batchStop){j.cancelRequested=true;await invoke('cancel_operation',{id}).catch(()=>{});}
 for(;;){let v=await invoke('wait_operation',{id,deferListing:request.operation==='list'});
  if(request.operation==='list'&&v.status==='complete'){
   try{v.result=QingArchiveListing.read(await invoke('take_archive_listing',{id}));}
   catch(error){v={...v,status:'failed',error:String(error)};}
  }
  if(request.operation==='list'&&j.cancelRequested&&v.status==='complete')v={...v,status:'cancelled',error:'operation cancelled'};
  if(v.status==='complete'&&v.result?.recovery_warning)toast(v.result.recovery_warning);
  updateJobProgress(j,v);onProgress?.(j);if(v.status!=='running'){if(v.status==='complete'||v.result?.deep_session){if(v.result?.deep_session)recordDeepReport(j,v.result);if(['add','archive-add','archive-move','archive-copy','delete','rename','comment-set','nested-save'].includes(request.operation))await syncNestedSessions();if(v.result?.valid===false){j.status='failed';j.error='完整性检查未通过。';}if(temporaryOutput){j.summary='文件已准备好';return v.result;}j.outputPath=j.deep?v.result?.outputs?.[0]:v.result?.output_directory||v.result?.destination||v.result?.outputs?.[0];const protection=v.result?.input_atime;if(protection&&(protection.unavailable_handles||protection.unconfirmed_sources||protection.backend_best_effort)){j.timeWarning=QingInputAtime.preservationNotice(v.result);j.timeReasons=[...new Set((protection.warnings||[]).map(w=>w.source+'：'+({alternate_stream_proof:'附加数据检查无法完整保留访问时间',encrypted_source_proof:'加密文件检查无法确认访问时间保护',enhanced_open:'无法取得所需的文件权限',enhanced_reopen:'无法取得所需的文件权限',suppress_updates:'系统未能启用访问时间保护'}[w.phase]||'无法确认访问时间保护')))];if(protection.backend_best_effort)j.timeReasons.push('未能逐项确认外部引擎的访问时间保护');}j.partial=!!(j.partial||(recycleAfter&&!j.deep&&v.result?.recycled!==true)||v.result?.recycle_error||v.result?.partial||v.result?.retained_sources?.length||v.result?.metadata_not_merged?.length);if(batch&&QingProgress.complete(j)){batch.done++;j.overall.done=batch.done;}j.summary=v.result?.summary|| (v.result?.retained_sources?.length?`已移动 ${v.result.moved||0} 项；${v.result.retained_sources.length} 个来源仍保留，目标副本已保留。${v.result.retained_reasons?.[0]?`\n${v.result.retained_reasons[0]}`:''}`:v.result?.metadata_not_merged?.length?`已复制 ${v.result.copied||0} 个文件；${v.result.metadata_not_merged.length} 个文件夹的附加数据未合并，来源完整保留。\n${v.result.metadata_not_merged[0]}`:v.result?.files!==undefined?`已提取 ${v.result.files} 个文件至 ${j.outputPath}`:v.result?.outputs?'已创建：'+v.result.outputs.join('、'):v.result?.valid?'完整性检查通过':'操作完成');if(v.result?.password_book_error)j.summary+='\n解压密码未能保存到密码本。';if(v.result?.recovery_warning)j.summary+='\n'+v.result.recovery_warning;return v.result;}throw new Error(v.error||'任务已取消');}if(background)renderInlineTask();else if(s.view==='jobs')renderProgress();
  else if(deferProgress&&!silent&&progressTimer===undefined)showQueuedProgress();
  renderHistoryJob(j);
 }}catch(error){
  if(['add','archive-add','archive-move','archive-copy','delete','rename','comment-set'].includes(request.operation)){
   await syncNestedSessions().catch(()=>{});
  }
  if(j.status==='running'){j.status='failed';j.error=String(error);}
  throw error;
 }finally{
  if(autoExpand)clearTimeout(autoExpand);
  if(progressTimer!==undefined)clearTimeout(progressTimer);
  j.finishedAt=performance.now();
  await invoke('forget_operation',{id}).catch(()=>{});
  s.active=null;s.backgroundExtraction=false;
  if(background){
   if(!s.inlineToggled&&(j.status==='failed'||j.partial||deepCanContinue(j)||j.deep?.cancelled||j.deep?.recovery_path))s.inlineExpanded=true;
   renderInlineTask();updateToolbar();
  }
  else if(s.view==='jobs')renderProgress();
  else if(deferProgress)updateToolbar();
  renderHistoryJob(j);
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
   renderHistoryJob(job);
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
 const stageCount=requests.length*(verify&&!recheck?2:1);
 const job={id:`compression-${nextCompressionJob++}`,label:recheck?'检查压缩文件':'新建压缩文件',status:'running',startedAt:performance.now(),background:true,
  archives:requests.map(request=>({path:request.path,state:rechecked?'unverified':'pending',outputs:rechecked?.outputs||[],
   encrypted:rechecked?.encrypted||!!request.options?.password})),current:0,stage:recheck?'verify':'create',verify,recheck:!!recheck,overall:stageCount>1?{done:0,total:stageCount}:null};
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
  Object.assign(job,{stage,phase:null,percent:null,progressSample:null,message:'',context:null,work:null,rate:null});
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
    if(job.overall){job.overall.done++;job.overall.active=0;}
    archive.warning=QingInputAtime.preservationNotice(result);
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
    if(job.overall){job.overall.done++;job.overall.active=0;}
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
 draft.querySelector('#qa-formerror').textContent=[issue?taskError(issue):null,hadPassword?'请重新输入密码。':null].filter(Boolean).join(' ');
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
 try{await openArchive(path);}catch(e){showOpenFailure(path,e);}
}

function forgetRecycledArchives(paths){
 const removed=new Set(paths.map(archiveKey));
 s.recent=s.recent.filter(path=>!removed.has(archiveKey(path)));
 localStorage.setItem('qing-recent',JSON.stringify(s.recent));
 for(const tab of [...archiveTabs])if(removed.has(archiveKey(tab.archive)))discardArchiveTab(tab.archive,{preserveJob:true});
 render();
}
async function finishExtraction(source,dest,result,{recycle=false,openAfter=false,closeAfter=false,job=s.jobs.at(-1)}={}){
 if(recycle){
  if(result.recycled){
   s.recent=s.recent.filter(p=>p!==source);
   localStorage.setItem('qing-recent',JSON.stringify(s.recent));
   discardArchiveTab(source,{preserveJob:true});
   if(job)job.summary+='；原压缩包已移入回收站。';
   render();
  }else{
   const message='解压已完成，但原压缩包未移入回收站：'+(result.recycle_error||'原压缩包已保留。');
   if(job){job.summary=message;job.partial=true;renderHistoryJob(job);}
   s.inlineExpanded=true;renderProgress();toast(message);
  }
 }
 if(openAfter)await invoke('reveal_path',{path:result.output_directory||result.destination||dest});
 if(closeAfter&&(!recycle||result.recycled))await invoke('window_action',{action:'close'});
 return !recycle||!!result.recycled;
}
// A background extraction keeps its existing progress/cancellation state.
// The secondary slot is for an unrelated read-only listing.
async function runSecondaryListing(request,onProgress){
 const job={id:null,label:'打开压缩包',path:request.path,status:'running',percent:null,startedAt:performance.now(),operation:'list'};
 secondaryOpen=job;
 try{
  job.id=await invoke('start_operation',{request,recycleAfter:false});
  s.jobs.push(job);
  if(job.cancelRequested)await invoke('cancel_operation',{id:job.id});
  for(;;){
   const v=await invoke('wait_operation',{id:job.id,deferListing:true});
   updateJobProgress(job,v);
   onProgress?.(job);
   if(v.status==='running')continue;
   if(v.status!=='complete'||job.cancelRequested)throw Error(v.error||'operation cancelled');
   const listing=await invoke('take_archive_listing',{id:job.id});
   if(job.cancelRequested)throw Error('operation cancelled');
   return QingArchiveListing.read(listing);
  }
 }catch(e){
  job.status=job.cancelRequested?'cancelled':'failed';
  job.error=String(e);
  throw e;
 }finally{
  job.finishedAt=performance.now();
  if(job.id!==null)await invoke('forget_operation',{id:job.id}).catch(()=>{});
  secondaryOpen=null;
  renderHistoryJob(job);
 }
}
async function openArchive(path,password='',{refresh=false,nested=null,folder='',listing=null,show=true,queued=false,isCurrent=()=>true,onProgress=null}={}){if(openingArchive||(s.active&&!s.backgroundExtraction)||$('qa-tools')?.contentWindow?.obzBusy)throw Error('请先等待或取消当前任务。');openingArchive=true;try{path=listing?.path??await invoke('canonical_archive_path',{path});if(typeof path!=='string'||!path)throw Error('无法解析压缩文件路径。');const existing=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(path));if(existing&&!refresh){openingArchive=false;if(show)switchArchiveTab(existing.id);rememberRecent(path);return;}const codepage=refresh?s.codepage||null:null;
 const request={operation:'list',path,detect_header_encryption:true,options:{password:password||null,codepage}};
 if(s.backgroundExtraction&&(refresh||s.jobs.some(job=>job.id===s.active&&archiveKey(job.path)===archiveKey(path))))
   throw Error('当前任务完成后可重新读取此压缩包。');
 const startJob=queued&&!refresh&&!password?()=>invoke('start_queued_archive_listing',{path}):null;
 const r=listing??await (s.backgroundExtraction?runSecondaryListing(request,onProgress):run(request,'打开压缩包',{silent:!!onProgress,onProgress,startJob,deferProgress:queued}));saveCurrentTab();resetPreviewState();await invoke('clear_preview_cache').catch(()=>{});if(!isCurrent())return;const defaults=load('qing-preferences',{}),tab=existing||{id:String(nextTabId++)};if(nested)Object.assign(tab,{nestedId:nested.id,parentArchive:nested.parent,nestedMember:nested.member,dirty:nested.dirty,conflict:nested.conflict});Object.assign(tab,{archive:path,password,entries:r.entries,headerEncrypted:r.header_encrypted===true,codepage,folder:folder&&archiveIndex(r.entries).paths.has(folder)?folder:'',flatView:refresh&&!!s.flatView,search:'',sort:refresh?s.sort:'name',reverse:refresh?s.reverse:false,selected:new Set(),collapsed:new Set(),anchor:null,cursor:null,focusFile:null,listStyle:refresh?s.listStyle:(defaults.listStyle==='icons'?'icons':'details'),iconSize:refresh?s.iconSize:Math.max(80,Math.min(256,Number(defaults.iconSize)||144)),scrollTop:0,treeScroll:0});if(!existing)archiveTabs.push(tab);currentTabId=tab.id;applyTab(tab);rememberRecent(path);if(show)navigate('archive');}catch(e){throw new Error(archiveOpenMessage(e,!!password));}finally{openingArchive=false;}}


function archiveDirectoryError(message){
 if(/^(?:Error:\s*)?RAR directory exceeds supported entry limit$/i.test(message))return '压缩包目录的条目数超出支持上限，无法完整读取。';
 if(/^(?:Error:\s*)?(?:RAR directory exceeds supported metadata limit|engine listing exceeds supported limit)$/i.test(message))return '压缩包目录的元数据超出支持上限，无法完整读取。';
 if(/^(?:Error:\s*)?RAR archive is missing a required volume$/i.test(message))return '压缩包缺少必要分卷，请将所有分卷放在同一目录后重试。';
 return '';
}
function archivePasswordError(error){return /(?:wrong|incorrect|invalid|enter|empty) password|password (?:is )?(?:required|incorrect|empty)|cannot open encrypted archive|需要密码|请输入密码|密码错误|密码不正确/i.test(String(error));}
function archiveOpenMessage(error,passwordProvided=false){const message=String(error);const directoryError=archiveDirectoryError(message);if(directoryError)return directoryError;if(/wrong password|incorrect password|password is incorrect/i.test(message))return '密码不正确，请重新输入。';if(/enter password|password (?:is )?required|encrypted archive.*password/i.test(message))return passwordProvided?'无法使用此密码读取压缩包，请检查密码后重试。':'此压缩包的文件目录已加密，请输入密码后打开。';return message.replace(/^Error: /,'');}
function repairableZip(path){return /\.(zip|zipx)$/i.test(path);}
function offerRepairAfterOpenFailure(path,failure){
 const button=overlay.querySelector('[data-action=open-repair]');
 if(!button)return;
 const canOffer=repairableZip(path)&&!archivePasswordError(failure)&&taskError(failure)!=='任务已取消';
 button.hidden=!canOffer;
 button.dataset.failedPath=canOffer?path:'';
}
function showOpenFailure(path,failure){
 openDialog(path);
 error(failure);
 offerRepairAfterOpenFailure(path,failure);
}
function openDialog(path='',{allowDeep=false}={}){
 s.view=s.archive?'archive':'home';
 dialog('打开压缩包',`
  <label class="qa-field">文件路径<div class="qa-inlinefield"><input id="qa-open-path" value="${esc(path)}">${btn('浏览…','browse-open')}</div></label>
  <label class="qa-field">密码（需要时填写）<input id="qa-open-password" type="password" autocomplete="off" data-password-book></label>`,
  btn('取消','close')+(allowDeep?btn('改为深度解压','open-submit-deep'):'')+
  '<button type="button" class="qa-button quiet" data-action="open-repair" hidden>修复此 ZIP…</button>'+
  btn('打开','open-submit','primary'),'oa-open-dialog');
}
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
let homePickerOpen=false;
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
  if(homePickerOpen||!homeIntakeAvailable()||tabBusy())return;
  homePickerOpen=true;
  let paths;
  try{
    paths=await invoke('pick_home_files');
  }finally{
    homePickerOpen=false;
  }
  if(paths.length)await routeHomeArchives(paths);
}
async function routeHomeArchives(paths){
  if(!paths.length)return;
  if(homeIntake){
    for(const path of paths)homeIntake.paths.set(archiveKey(path),path);
    return;
  }
  if(!homeIntakeAvailable()||tabBusy()){enqueueArchives(paths);return;}
  const intake={paths:new Map(paths.map(path=>[archiveKey(path),path])),browsed:0,examined:0,path:''};
  homeIntake=intake;
  const isCurrent=()=>homeIntake===intake;
  const showStatus=()=>{
    const status=$('oa-home-identify-status');
    if(status)status.textContent=`正在识别 ${intake.examined+1} / ${intake.paths.size} · ${filename(intake.path)||intake.path}`;
  };
  intake.progress=setTimeout(()=>{
    if(!isCurrent()||!overlay.hidden)return;
    dialog('识别文件','<p id="oa-home-identify-status" class="qa-sub oa-home-identify-status" role="status"></p>'+QingProgress.markup('aria-labelledby="oa-home-identify-status"'),btn('停止识别','close'),'oa-open-dialog');
    overlay.dataset.page='home-inspecting';
    showStatus();
  },220);
  const deep=[], review=[];
  try{
    for(const path of intake.paths.values()){
      intake.path=path;
      showStatus();
      try{
        const body=await invoke('inspect_home_archive',{path});
        if(!isCurrent())return;
        const result=QingArchiveListing.read(body);
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
      intake.examined++;
    }
  }finally{
    if(isCurrent())finishHomeIntake(intake);
  }
  if(review.length){showHomeReview(review,deep,intake.browsed);return;}
  if(deep.length)deepExtract.batch(deep,{allowBrowse:true});
}
function homeReviewReason(reason,compressionSource){
  if(compressionSource==='file')return {text:'可用此文件新建压缩包。',choice:'compress'};
  if(compressionSource==='directory')return {text:'可将整个文件夹作为压缩来源。',choice:'compress'};
  const message=archiveOpenMessage(reason);
  if(/目录的(?:条目数|元数据)超出支持上限|缺少必要分卷/.test(message))return {text:message,choice:'skip'};
  if(/密码|password required/i.test(message))return {text:'需要密码，选择“尝试打开”后输入。',choice:'browse'};
  if(/access (?:is )?denied|permission denied|拒绝访问|权限/i.test(message))return {text:'无法读取文件，请检查访问权限。',choice:'skip'};
  if(/not found|cannot find|no such file|找不到|不存在/i.test(message))return {text:'找不到文件，请确认路径后重新选择。',choice:'skip'};
  if(/路径不能包含链接或重解析点|无效的归档路径/.test(message))return {text:message,choice:'skip'};
  return {text:'无法识别压缩包，请检查格式和文件是否完整。',choice:'skip'};
}
function showHomeReview(review,deep,browsed){
  homeReview={review,deep};
  const identified=[browsed?`已打开 ${browsed} 个压缩包`:'',deep.length?`${deep.length} 个将进入深度解压`:''].filter(Boolean);
  dialog('处理所选文件',`${identified.length?`<p class="qa-sub oa-home-review-summary">${identified.join(' · ')}</p>`:''}
    <div class="oa-home-review-list">${review.map((item,i)=>{
      const {text,choice}=homeReviewReason(item.reason,item.compression_source);
      const options=[['skip','跳过'],['browse','尝试打开'],['deep','加入深度解压']];
      if(choice==='compress')options.unshift(['compress','作为来源压缩']);
      return `<section class="oa-home-review-item">
        <div class="oa-home-review-row">
         <div><strong id="oa-home-review-name-${i}" title="${esc(item.path)}">${esc(filename(item.path.replace(/[\\/]+$/,''))||item.path)}</strong>
         <span class="qa-sub" id="oa-home-review-detail-${i}">${esc(text)}</span></div>
         <select id="oa-home-review-${i}" aria-labelledby="oa-home-review-name-${i}" aria-describedby="oa-home-review-detail-${i}">${opts(options,choice)}</select>
        </div>
        <details><summary>诊断详情</summary>
        <textarea rows="4" readonly aria-label="诊断详情">${esc(item.path+'\n'+item.reason)}</textarea>
        <button type="button" class="qa-button" data-review-copy="${i}">复制诊断</button></details>
      </section>`;
    }).join('')}</div>`,
    btn('取消','close')+btn('继续','home-review-submit','primary'),'oa-extraction-dialog oa-home-review-dialog');
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
  const {review,deep}=homeReview,browse=[],sources=[];
  for(const [i,item] of review.entries()){
    const choice=$('oa-home-review-'+i).value;
    if(choice==='deep')deep.push(item.path);
    if(choice==='browse')browse.push(item.path);
    if(choice==='compress')sources.push(item.path);
  }
  close();
  if(browse.length===1&&review.length===1&&!deep.length&&homeReviewReason(review[0].reason).choice==='browse'){
    openDialog(browse[0],{allowDeep:true});
    $('qa-open-password')?.focus();
    return;
  }
  if(deep.length){
    deepExtract.batch(deep,{allowBrowse:true});
    if(sources.length){
      pendingFileGroups.push(sources);
      schedulePendingQueue();
      toast(`${sources.length} 个压缩来源已排队 · 当前操作结束后打开新建压缩包`);
    }
  }else if(sources.length)showCompressionFiles(sources);
  if(browse.length)enqueueArchives(browse);
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
const compressionSuffixes=[
 ...Object.entries(compressionExtensions),
 ['tgz','.tgz'],['tbz2','.tbz2'],['tbz2','.tbz'],['txz','.txz'],
 ['gzip','.gzip'],['zstd','.zstd'],['brotli','.brotli'],['lizard','.lizard'],
].sort((a,b)=>b[1].length-a[1].length);
const compressionCanEncrypt=format=>['zip','7z','zipx','sfx'].includes(format);
const compressionParent=path=>path.slice(0,Math.max(path.lastIndexOf('/'),path.lastIndexOf('\\'))+1);
const compressionLeaf=path=>filename(path.replace(/[\\/]+$/,''));
function compressionSuffix(path){
 const leaf=filename(path).toLowerCase();
 return compressionSuffixes.find(([,ext])=>leaf.length>ext.length&&leaf.endsWith(ext));
}
function applyCompressionExtension(path,format){
 if(!validCompressionFilename(path))return path;
 const current=compressionSuffix(path);
 return current?.[0]===format?path:(current?path.slice(0,-current[1].length):path)+compressionExtensions[format];
}
function validCompressionFilename(path){
 const leaf=filename(path).replace(/^[A-Za-z]:/,'');
 return !!leaf&&!/[. ]$/.test(leaf)&&!/[<>:"|?*\u0000-\u001f\u007f-\u009f]/.test(leaf);
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
 const list='<div id="oa-sources" class="qa-source-list" role="group" aria-label="来源列表"></div>';
 const compression=inputId==='qa-inputs';
 const actions='<div class="qa-actions oa-source-actions">'+(compression?'<h3 id="oa-compression-source-title">来源</h3>':'')
  +'<span id="oa-source-count" class="qa-sub" role="status"></span>'
  +btn('添加文件…',actionPrefix+'-files')+btn('添加文件夹…',actionPrefix+'-folder')
  +'<button type="button" class="qa-button quiet" data-action="sources-remove" disabled>移除所选</button></div>';
 const paths='<details class="oa-source-paths"><summary>输入或粘贴路径</summary><textarea id="'+inputId+'" rows="3" aria-label="来源路径（每行一个）"></textarea></details>';
 return (compression?actions+list:list+actions)+paths;
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
 $('oa-sources').innerHTML=inputs.map((path,i)=>'<label class="qa-source-row" title="'+esc(path)+'"><input type="checkbox" aria-label="选择 '+esc(path)+'" data-source-index="'+i+'"><span>'+esc(compressionLeaf(path))+'</span><small aria-hidden="true">'+esc(parents[i])+'</small></label>').join('')||'<p class="qa-sub">可将文件或文件夹拖到此窗口。</p>';
 $('oa-source-count').textContent=inputs.length+(input.id==='qa-inputs'?' 项':' 个来源');
 syncSourceSelection();
 if(input.id==='qa-inputs'){
  const save=$('qa-save');
  if(save.dataset.explicit!=='true')save.value=suggestedCompressionPath(inputs,$('qa-format').value,$('oa-separate').checked);
  save.removeAttribute('aria-invalid');
  $('qa-format').removeAttribute('aria-invalid');
  $('qa-formerror').textContent='';
  clearCompressionPlan();
 }
}
function syncSourceSelection(){
 overlay.querySelector('[data-action=sources-remove]').disabled=!overlay.querySelector('[data-source-index]:checked');
}
function syncCompressionFilename(){
 const save=$('qa-save'),hint=$('oa-compression-filename');
 const path=save.value.trim(),name=!$('oa-separate').checked&&validCompressionFilename(path)
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
 action.textContent=$('oa-separate').checked?'预览并创建':'创建压缩包';
}
function syncCompressionSourceHint(){
 const single=window.qingCompression.singleFile($('qa-format').value),separate=$('oa-separate').checked;
 const hint=$('oa-compression-source-hint');
 hint.hidden=!single;
 hint.textContent=separate?'每个来源须为单个文件。':'此格式只能压缩一个文件。多文件可分别压缩，或使用 ZIP 等格式。';
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
 $('qa-format').removeAttribute('aria-invalid');
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
 syncCompressionSourceHint();
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
 method.closest('label').hidden=methods.length===1;
 const algorithms=format==='zip'?['AES256','AES192','AES128','ZipCrypto']:format==='zipx'?['AES256','AES192','AES128']:['AES256'];
 encryption.innerHTML=opts(algorithms.map(v=>[v,v]),algorithms.includes(encryption.value)?encryption.value:'AES256');
 syncCompressionPassword();
 syncCompressionControls();
}
function invalidCompressionField(field,message){
 field.setAttribute('aria-invalid','true');
 field.focus();
 throw Error(message);
}
function validateCompressionSources(format,inputs,separate){
 const addFiles=overlay.querySelector('[data-action=compress-files]');
 if(!inputs.length){
  addFiles.focus();
  throw Error('请先添加要压缩的文件或文件夹。');
 }
 // Folder sources are rejected by plan_compression_outputs, which can see the file system.
 if(window.qingCompression.singleFile(format)&&!separate&&inputs.length!==1){
  invalidCompressionField($('qa-format'),'此格式每个压缩包只接受一个文件。请调整来源或更改格式。');
 }
}
function compressionNumber(id,multiplier=1){
 const field=$(id);
 if(field.disabled||!field.value&&!field.validity.badInput)return undefined;
 const value=Number(field.value)*multiplier;
 if(!field.validity.valid||!Number.isSafeInteger(value)){
  showCompressionAdvanced(true);
  const constraint=field.max?`${field.min}–${field.max}`:`大于或等于 ${field.min}`;
  invalidCompressionField(field,`请为“${field.labels[0].textContent.trim()}”输入${constraint}的整数。`);
 }
 return value;
}
async function confirmCompressionOutputs(format,inputs,volumeBytes){
 const preview=$('oa-compression-preview'),save=$('qa-save'),separate=$('oa-separate').checked;
 const path=save.value.trim();
 if(!path)invalidCompressionField(save,separate?'请选择保存压缩包的文件夹。':'请输入压缩包的保存路径。');
 if(!separate&&!validCompressionFilename(path))invalidCompressionField(save,'请输入有效的压缩包文件名。');
 const parent=path.replace(/[\\/]+$/,'');
 const paths=separate?inputs.map(input=>parent+'\\'+compressionLeaf(input)+compressionExtensions[format]):[applyCompressionExtension(path,format)];
 if(!separate)save.value=paths[0];
 const button=overlay.querySelector('[data-action=startcompress]');
 if(button.disabled)return null;
 const body=preview.closest('.oa-dialog-body');
 const label=button.textContent;
 button.textContent='正在检查输出路径…';
 button.setAttribute('aria-busy','true');
 button.disabled=true;
 body.inert=true;
 try{
  const sources=separate?inputs.map(input=>[input]):[inputs];
  const outputs=await invoke('plan_compression_outputs',{paths,format,volumeBytes:volumeBytes||null,sources});
  if(!preview.isConnected||save.value.trim()!==(separate?path:paths[0])||separate!==$('oa-separate').checked||format!==$('qa-format').value||JSON.stringify(inputs)!==JSON.stringify(lines('qa-inputs')))return null;
  const conflict=outputs.some((output,i)=>output!==paths[i]);
  const key=JSON.stringify({paths,outputs,volumeBytes});
  const previousPlan=preview.dataset.confirmed;
  if((separate||conflict||previousPlan)&&previousPlan!==key){
   preview.innerHTML='<p>'+ (conflict?'存在同名目标，将使用以下名称。':'将创建以下压缩包。')+'</p><ul>'+outputs.map(output=>'<li title="'+esc(output)+'">'+esc(output)+'</li>').join('')+'</ul>';
   preview.dataset.confirmed=key;
   preview.hidden=false;
   preview.scrollIntoView({block:'nearest'});
   return null;
  }
  return outputs;
 }catch(reason){
  if(!preview.isConnected)return null;
  const text=String(reason);
  if(/unsafe archive path|输出文件名无效|输出路径缺少目录/.test(text)){
   body.inert=false;
   invalidCompressionField(save,'输出文件名无效。请检查文件名，或选择其他保存位置。');
  }
  if(/^(?:Error:\s*)?所选格式只能压缩单个文件/.test(text)){body.inert=false;invalidCompressionField($('qa-format'),text.replace(/^Error:\s*/,''));}
  if(/^(?:Error:\s*)?保存位置在来源文件夹内/.test(text)){body.inert=false;invalidCompressionField(save,text.replace(/^Error:\s*/,''));}
  throw reason;
 }finally{
  button.disabled=false;
  button.removeAttribute('aria-busy');
  button.textContent=preview.hidden?label:'确认创建';
  body.inert=false;
  if(preview.isConnected&&document.activeElement===document.body)button.focus({preventScroll:true});
 }
}
const compressionLevels=window.qingCompression.levels;
root.addEventListener('change',event=>{
 const id=event.target.id;
 if(id!=='qa-level'&&id!=='oa-method')return;
 const method=$('oa-method'),level=$('qa-level');
 const value=window.qingCompression.reconcileStore($('qa-format').value,method.value,level.value,id==='qa-level'?'level':'method');
 method.value=value.method;
 level.value=value.level;
},true);
function showCompressionAdvanced(open){
 $('oa-compression-advanced').hidden=!open;
 overlay.querySelector('[data-action=compression-advanced]').setAttribute('aria-expanded',String(open));
}
function syncCompressionControls(){
 const format=$('qa-format').value,seven=['7z','sfx'].includes(format),method=$('oa-method').value;
 const level=$('qa-level'),levels=compressionLevels(format,method),current=Number(level.value);
 const preferred=levels.some(([value])=>value===current)?current:(method||format).toLowerCase()==='lizard'?10:5;
 level.innerHTML=opts(levels,preferred);
 const range=window.qingCompression.wordRange(format,method),store=preferred===0||['copy','store'].includes(method.toLowerCase());
 const canConfigure=(id,enabled)=>{
  const input=$(id);
  input.disabled=!enabled;
  input.closest('label').hidden=!enabled;
 };
 canConfigure('qa-level',format!=='iso');
 canConfigure('qa-volume',window.qingCompression.volumeSupported(format));
 canConfigure('qa-threads',!['iso','lzh','zipx'].includes(format));
 canConfigure('oa-dictionary',seven&&['LZMA2','LZMA'].includes(method||'LZMA2')&&!store);
 canConfigure('oa-solid',seven);
 canConfigure('oa-header-encryption',seven);
 canConfigure('oa-word-size',!!range&&!store);
 canConfigure('oa-solid-block',seven&&!store&&$('oa-solid').value!=='false');
 const word=$('oa-word-size');
 $('oa-word-label').textContent=method.toLowerCase()==='ppmd'?'模型阶数':'字长';
 if(range){
  [word.min,word.max]=range;
  word.placeholder=`自动（${range[0]}–${range[1]}）`;
  if(word.value&&(Number(word.value)<range[0]||Number(word.value)>range[1]))word.value='';
 }
 const times=window.qingCompression.timeSupported(format);
 $('oa-compression-times').hidden=!times;
 for(const id of timePreferenceIds)$(id).disabled=!times;
 const advanced=overlay.querySelector('[data-action=compression-advanced]');
 advanced.hidden=['iso','lzh'].includes(format);
 if(advanced.hidden)showCompressionAdvanced(false);
 syncCompressionSourceHint();
}
const timePreferenceIds=['oa-store-mtime','oa-store-ctime','oa-store-atime','oa-archive-time','oa-preserve-atime'];
function restoreTimePreferences(){const value=load('qing-compression-times',{}),saved=value&&typeof value==='object'&&!Array.isArray(value)?value:{};for(const id of timePreferenceIds){const value=saved[id];if(!$(id))continue;if($(id).type==='checkbox')$(id).checked=value===true;else if([...$(id).options].some(option=>option.value===value))$(id).value=value;}}
function saveTimePreferences(){const saved={};for(const id of timePreferenceIds)if($(id))saved[id]=$(id).type==='checkbox'?$(id).checked:$(id).value;try{localStorage.setItem('qing-compression-times',JSON.stringify(saved));}catch{} }
function compressionFormatOptions(selected){
 const single=([format])=>window.qingCompression.singleFile(format);
 const image=([format])=>['wim','sfx','iso'].includes(format);
 return [
  ['压缩包',formats.filter(item=>!single(item)&&!image(item))],
  ['单文件压缩',formats.filter(single)],
  ['映像与自解压',formats.filter(image)],
 ].map(([label,items])=>`<optgroup label="${label}">${opts(items,selected)}</optgroup>`).join('');
}
function compressionAdvancedFields(){
 const timeChoices=[['','默认'],['true','保存'],['false','不保存']];
 return `<div id="oa-compression-advanced" class="oa-compression-options" hidden>
  <div class="qa-fieldrow">
   <label class="qa-field">分卷大小（MiB）<input id="qa-volume" type="number" min="1" max="${Math.floor(Number.MAX_SAFE_INTEGER/1048576)}" step="1" placeholder="不分卷"></label>
   <label class="qa-field">CPU 线程数<input id="qa-threads" type="number" min="1" max="256" step="1" placeholder="自动"></label>
   <label class="qa-field">压缩方法<select id="oa-method"></select></label>
   <label class="qa-field">字典大小<select id="oa-dictionary">${opts([['','自动'],['1','1 MiB'],['4','4 MiB'],['16','16 MiB'],['32','32 MiB'],['64','64 MiB'],['128','128 MiB'],['256','256 MiB']], '')}</select></label>
   <label class="qa-field"><span id="oa-word-label">字长</span><input id="oa-word-size" type="number" step="1" placeholder="自动"></label>
   <label class="qa-field">固实压缩<select id="oa-solid">${opts([['','自动'],['true','开启'],['false','关闭']], '')}</select></label>
   <label class="qa-field">固实块上限<select id="oa-solid-block">${opts([['','自动'],['1','1 MiB'],['16','16 MiB'],['64','64 MiB'],['256','256 MiB'],['1024','1 GiB'],['4096','4 GiB']], '')}</select></label>
  </div>
  <details id="oa-compression-times">
   <summary>时间信息</summary>
   <div class="qa-fieldrow">
    <label class="qa-field">修改时间<select id="oa-store-mtime">${opts(timeChoices,'')}</select></label>
    <label class="qa-field">创建时间<select id="oa-store-ctime">${opts(timeChoices,'')}</select></label>
    <label class="qa-field">访问时间<select id="oa-store-atime">${opts(timeChoices,'')}</select></label>
    <label class="qa-field">压缩包时间<select id="oa-archive-time">${opts([['default','默认'],['latest','内容最新修改时间']],'default')}</select></label>
   </div>
   <label class="qa-check"><input id="oa-preserve-atime" type="checkbox">保留来源访问时间</label>
  </details>
 </div>`;
}
function compress(profile,inputs=[]){
 dialog('新建压缩包',`
  <section class="oa-compression-section" aria-labelledby="oa-compression-source-title">
   ${sourceFields('qa-inputs','compress')}
  </section>
  <section class="oa-compression-section" aria-labelledby="oa-compression-output-title">
   <div class="oa-compression-section-head">
    <h3 id="oa-compression-output-title">输出</h3>
    <label class="qa-check"><input id="oa-separate" type="checkbox">每个来源单独压缩</label>
   </div>
   <div class="qa-field">
    <div class="oa-compression-save-heading">
     <label id="oa-compression-save-label" for="qa-save">保存为</label>
     <span id="oa-compression-filename" hidden><span></span><span></span></span>
    </div>
    <div class="qa-inlinefield"><input id="qa-save" aria-describedby="oa-compression-filename" placeholder="输入或选择压缩包路径">${btn('浏览…','compress-save')}</div>
   </div>
   <div class="qa-fieldrow">
    <label class="qa-field">格式<select id="qa-format" aria-describedby="oa-compression-source-hint">${compressionFormatOptions(profile?.format||'zip')}</select></label>
    <label class="qa-field">压缩级别<select id="qa-level">${opts([[0,'仅存储'],[1,'快速'],[5,'标准（推荐）'],[9,'最大压缩']],profile?.level??5)}</select></label>
   </div>
   <p id="oa-compression-source-hint" class="qa-sub" hidden></p>
   <section id="oa-compression-preview" class="oa-compression-preview oa-form-notice" aria-live="polite" hidden></section>
  </section>
  <div class="qa-actions oa-compression-disclosures">
   <button class="qa-button" type="button" id="oa-password-button" data-action="password-settings" aria-expanded="false" aria-controls="oa-password-panel">设置密码</button>
   <button class="qa-button" type="button" data-action="compression-advanced" aria-expanded="false" aria-controls="oa-compression-advanced">分卷与高级设置</button>
  </div>
  <div id="oa-password-unsupported" class="oa-form-notice" role="status" hidden><span>当前格式不支持加密，已保留填写的密码。</span>${btn('确认不加密','compression-clear-password','quiet')}</div>
  <div id="oa-password-panel" class="oa-compression-options" hidden>
   <div class="qa-fieldrow">
   <label class="qa-field">密码<input id="qa-new-password" type="password" autocomplete="off"></label>
   <label class="qa-field">再次输入密码<input id="oa-password-again" type="password" autocomplete="off"></label>
   </div>
   <div class="oa-compression-password-settings">
    <label class="qa-field">加密算法<select id="oa-encryption"></select></label>
    <label class="qa-check"><input id="oa-show-password" type="checkbox">显示密码</label>
    <label class="qa-check"><input id="oa-header-encryption" type="checkbox" checked>加密文件名</label>
   </div>
  </div>
  ${compressionAdvancedFields()}`,
  '<label class="qa-check oa-compression-verify"><input id="oa-test-after" type="checkbox" checked>创建后检查完整性</label>'
   +btn('取消','close')+btn('创建压缩包','startcompress','primary'),'oa-compression-dialog');
 $('qa-inputs').value=inputs.join('\n');
 refreshSources();
 methodOptions();
 restoreTimePreferences();
 overlay.querySelector('.oa-compression-dialog').addEventListener('input',event=>{
  const field=event.target;
  if(field.matches('input,select,textarea'))$('qa-formerror').textContent='';
  field.removeAttribute('aria-invalid');
 });
 overlay.querySelector(inputs.length?'#qa-save':'[data-action=compress-files]').focus({preventScroll:true});
}
async function submitCompression(){
 const button=overlay.querySelector('[data-action=startcompress]');
 if(!button||button.disabled)return;
 $('qa-formerror').textContent='';
 for(const field of button.closest('.qa-dialog').querySelectorAll('[aria-invalid=true]'))field.removeAttribute('aria-invalid');
 const inputs=lines('qa-inputs'),format=$('qa-format').value,separate=$('oa-separate').checked,verify=$('oa-test-after').checked;
 validateCompressionSources(format,inputs,separate);
 const password=validateCompressionPassword(format),options={};
 if(!$('qa-level').disabled)options.level=Number($('qa-level').value);
 if(password){
  options.password=password;
  options.encryption=$('oa-encryption').value;
  if(!$('oa-header-encryption').disabled)options.header_encryption=$('oa-header-encryption').checked;
 }
 if($('oa-method').value)options.method=$('oa-method').value;
 const volume=compressionNumber('qa-volume',1048576),threads=compressionNumber('qa-threads'),word=compressionNumber('oa-word-size');
 if(volume!==undefined)options.volume_bytes=volume;
 if(threads!==undefined)options.threads=threads;
 if(word!==undefined)options.word_size=word;
 if(!$('oa-dictionary').disabled&&$('oa-dictionary').value)options.dictionary_bytes=Number($('oa-dictionary').value)*1048576;
 if(!$('oa-solid').disabled&&$('oa-solid').value)options.solid=$('oa-solid').value==='true';
 if(!$('oa-solid-block').disabled&&$('oa-solid-block').value)options.solid_block_bytes=Number($('oa-solid-block').value)*1048576;
 const checkOptions={...options};
 for(const key of ['mtime','ctime','atime']){
  const field=$('oa-store-'+key);
  if(!field.disabled&&field.value!=='')options['store_'+key]=field.value==='true';
 }
 if(!$('oa-archive-time').disabled&&$('oa-archive-time').value==='latest')options.archive_timestamp='latest';
 if(!$('oa-preserve-atime').disabled&&$('oa-preserve-atime').checked)options.preserve_input_atime=true;
 const outputs=await confirmCompressionOutputs(format,inputs,volume);
 if(!outputs)return;
 const requests=outputs.map((path,i)=>({operation:'create',path,inputs:separate?[inputs[i]]:inputs,format,options}));
 await runCompression(requests,verify,checkOptions);
}
function extractSelection(){
 const scope=overlay.querySelector('[name=extract-scope]:checked')?.value;
 return scope==='selected'?[...s.selected]:scope==='folder'?[s.folder]:[];
}
function extractPasswordRequired(entries,selected,headerEncrypted){
 if(headerEncrypted)return true;
 const normalize=name=>name.replaceAll('\\','/');
 const exact=new Set(selected.map(normalize));
 const roots=new Set([...exact].map(name=>name.replace(/\/+$/,'')));
 return entries.some(entry=>{
  if(!entry.encrypted)return false;
  const name=normalize(entry.name);
  if(!exact.size||exact.has(name))return true;
  for(let i=name.lastIndexOf('/');i>=0;i=name.lastIndexOf('/',i-1)){
   if(roots.has(name.slice(0,i)))return true;
   if(i===0)break;
  }
  return false;
 });
}
function syncExtractPassword(selected,deep){
 const required=extractPasswordRequired(s.entries,selected,s.headerEncrypted);
 deepExtract.syncPasswordField('qa-extract-password',{required,optional:deep,cached:s.password});
}
function archiveParent(path){
 const i=Math.max(path.lastIndexOf('\\'),path.lastIndexOf('/'));
 return i<0?'':path.slice(0,i===2&&path[1]===':'?3:i);
}
function archiveStem(path){
 const name=filename(path).replace(/\.\d{3,}$/,'');
 return /\.tar\.(gz|bz2|xz|zst)$/i.test(name)?name.replace(/\.tar\.(gz|bz2|xz|zst)$/i,''):name.replace(/(?<=.)\.[^.]+$/,'').replace(/\.part\d+$/i,'');
}
function joinExtractionPath(base,name){
 if(!base||!name)return base||name;
 return /^[A-Z]:$/i.test(base)?base+name:base.replace(/[\\/]+$/,'')+'\\'+name;
}
function extractionLayout(source,entries,selected,smart,deep){
 const stem=archiveStem(source),lowerAscii=name=>name.replace(/[A-Z]/g,c=>c.toLowerCase());
 if(!smart)return {folder:deep?stem:'',wrapped:''};
 const normalize=name=>name.replaceAll('\\','/');
 const exact=new Set(selected.map(normalize));
 const scope=new Set([...exact].map(n=>n.replace(/\/+$/,'')));
 const selectedPath=name=>{
  if(!scope.size||exact.has(name))return true;
  for(let i=name.lastIndexOf('/');i>0;i=name.lastIndexOf('/',i-1)){
   if(scope.has(name.slice(0,i)))return true;
  }
  return false;
 };
 const expected=lowerAscii(stem);
 let wrapped='';
 for(const entry of entries){
  const raw=normalize(entry.name);
  if(!selectedPath(raw))continue;
  const name=raw.replace(/^\.\//,'').replace(/\/+$/,'');
  const separator=name.indexOf('/'),top=separator<0?name:name.slice(0,separator);
  if(lowerAscii(top)!==expected||(!entry.is_dir&&separator<0))return {folder:stem,wrapped:''};
  wrapped||=top;
 }
 return {folder:deep?wrapped||stem:wrapped?'':stem,wrapped};
}
function conflictHelp(value,deep=false){
 return ({
  error:deep?'遇到保存冲突时暂停，已校验结果留在恢复目录。':'发现同名文件时停止，可在任务中调整后重试。',
  rename:'保留已有文件，将新文件自动改名后保存。',
  overwrite:'用解压出的文件替换目标中的同名文件。',
  skip:deep?'保留已有文件，跳过的结果留在恢复目录。':'保留已有文件，继续解压其他项目。'
 })[value]||'';
}
function extractionPathMarkup({id='oa-expected-output',label='预计输出目录',copyAction='copy-extract-output'}={}){
 return `<div class="oa-extract-location">
  <div class="oa-extract-location-value">
   <span class="qa-sub" id="${id}-label">${label}</span>
   <output class="oa-extract-output" id="${id}" aria-labelledby="${id}-label" aria-live="off" tabindex="0"></output>
  </div>
  <button type="button" class="qa-button quiet" data-action="${copyAction}" aria-label="复制预计输出路径">复制路径</button>
 </div>`;
}
function recycleSummary({statusId,recycleId,recycle=false}){
 return `<div class="oa-extract-disposition oa-extract-summary">
   <span id="${statusId}" class="oa-source-disposition" role="status"></span>
   <label class="oa-disposition-control">成功后回收
    <input id="${recycleId}" class="oa-switch" role="switch" type="checkbox" aria-label="全部成功后将原包移入回收站" aria-describedby="${statusId}" ${recycle?'checked':''}>
   </label>
 </div>`;
}
function sourceDisposition(recycle,{deep=false}={}){
 if(!recycle)return '保留原压缩包。';
 return deep?'全部成功后将符合条件的原包移入回收站。':'全部成功后将原压缩包移入回收站。';
}
function setExtractionPreview(output,base,relative){
 const path=base?joinExtractionPath(base,relative):'';
 output.value=path||'请选择目标文件夹';
 output.title=path;
 output.dataset.path=path;
 output.closest('.oa-extract-location').querySelector('button').disabled=!path;
}
function extractionLocation(source,entries,selected,smart,deep){
 const layout=extractionLayout(source,entries,selected,smart,deep);
 const item=selected.length===1?selected[0].replace(/^\.[\\/]/,'').replaceAll('/','\\').replace(/\\+$/,''):'';
 return {
  item,
  relative:item&&!deep?joinExtractionPath(layout.folder,item):layout.folder||layout.wrapped,
  label:item&&!deep?'预计保存路径':'预计输出目录',
  hint:deep?'按展开后的内容确定实际目录。重名的输出目录会自动编号。':
   layout.wrapped?'智能解压沿用包内同名目录。':smart?'智能解压会创建以压缩包命名的文件夹。':'保留包内的目录结构。'
 };
}
function updateExtractPreview(reuseLayout=false){
 const dest=$('qa-destination')?.value.trim(),output=$('oa-expected-output');
 if(!output)return;
 if(!reuseLayout||output.dataset.relative===undefined){
  const smart=$('qa-smart').checked,deep=$('oa-deep-extract').checked,selected=extractSelection();
  const {item,relative,label,hint}=extractionLocation(s.archive,s.entries,selected,smart,deep);
  output.dataset.relative=relative;
  $(output.id+'-label').textContent=label;
  const detail=$('oa-extract-scope-detail');
  detail.hidden=!item;
  detail.textContent=item?'包内位置 '+item:'';
  detail.title=item;
  $('oa-extract-layout-help').textContent=hint;
 }
 setExtractionPreview(output,dest,output.dataset.relative);
 overlay.querySelector('[data-action=startextract]').disabled=!dest;
}
function updateExtractConflict(id,deep,stopHelp){
 const select=$(id),value=select.value;
 const help=$(id+'-help');
 help.textContent=value==='error'&&stopHelp?stopHelp:conflictHelp(value,deep);
 help.dataset.overwrite=String(value==='overwrite');
}
function labelExtractionShortcut(action){
 const button=overlay.querySelector(`[data-action=${action}]`);
 button.title='Ctrl+Enter';
 button.setAttribute('aria-keyshortcuts','Control+Enter');
}
function extract({recycleOnce=false,forceDeep=false,forceStandard=false}={}){
 if(!s.archive)return openDialog();
 const defaultPath=s.lastDest||archiveParent(s.archive),deep=forceDeep||(!forceStandard&&s.deepExtract);
 const scope=s.selected.size?'selected':s.folder?'folder':'all';
 const radio=(value,label,available=true)=>`<label class="qa-check"><input type="radio" name="extract-scope" value="${value}" ${(recycleOnce?'all':scope)===value?'checked':''} ${available?'':'disabled'}>${label}</label>`;
 dialog(recycleOnce?'解压并移入回收站':forceDeep?'深度解压':'解压文件',`
  <p class="oa-extract-source"><span class="qa-sub">压缩包</span> <strong title="${esc(s.archive)}">${esc(filename(s.archive))}</strong></p>
  <div class="oa-extract-scope" role="group" aria-labelledby="oa-extract-scope-title" aria-describedby="oa-extract-scope-detail"><span id="oa-extract-scope-title">解压范围</span>
   ${radio('all','整个压缩包')}${radio('folder','当前文件夹',!!s.folder)}${radio('selected',`选中项目（${s.selected.size}）`,!!s.selected.size)}
  </div>
  <p id="oa-extract-scope-detail" class="qa-sub oa-extract-scope-detail" tabindex="0" hidden></p>
  <div class="qa-field">
   <label for="qa-destination">保存到</label>
   <div class="qa-inlinefield oa-extract-target"><input id="qa-destination" value="${esc(defaultPath)}" spellcheck="false" placeholder="输入或选择目标文件夹">
    ${btn('原包目录','extract-original','quiet')}${btn('浏览…','extract-dest','','folder-open')}
   </div>
  </div>
  <div class="oa-extract-options">
   <div class="oa-extract-modes" role="group" aria-label="解压方式">
    <label class="qa-check" title="包内已有同名顶层目录时，沿用这层目录"><input id="qa-smart" type="checkbox" ${s.sameFolder?'checked':''}>智能解压</label>
    ${deepExtract.switchMarkup(deep)}
   </div>
   <label class="oa-extract-conflict">同名文件<select id="qa-conflict" aria-describedby="qa-conflict-help">${conflicts(recycleOnce&&s.overwrite==='skip'?'error':s.overwrite)}</select></label>
  </div>
  ${extractionPathMarkup()}
  <p id="oa-extract-layout-help" class="qa-sub oa-extract-hint"></p>
  <p id="qa-conflict-help" class="qa-sub oa-extract-hint" role="status"></p>
  ${deepExtract.passwordMarkup('qa-extract-password','填写内层密码')}
  <details id="oa-destination-details" class="oa-extract-advanced">
   <summary>常用位置与子文件夹</summary>
   <div class="oa-extract-grid">
    <div class="oa-places"><button type="button" data-place="desktop" disabled>桌面</button><button type="button" data-place="documents" disabled>文档</button><button type="button" data-place="pictures" disabled>图片</button><button type="button" data-place="downloads" disabled>下载</button>${btn('上级目录','destination-up')}</div>
    <div id="oa-destination-folders" class="oa-folders" aria-label="子文件夹"></div>
   </div>
   <p id="oa-known-folders-status" class="qa-sub oa-extract-hint" role="status" hidden></p>
  </details>
  <details class="oa-extract-advanced"><summary>更多解压选项</summary>
   <label class="oa-switch-row"><span>完成后打开目标文件夹</span><input id="oa-open-after" class="oa-switch" role="switch" type="checkbox" ${s.openAfter?'checked':''}></label>
   <label class="oa-switch-row"><span>完成后关闭窗口</span><input id="oa-close-after" class="oa-switch" role="switch" type="checkbox" ${s.closeAfter?'checked':''}></label>
   ${recycleOnce?'<p class="qa-sub" id="oa-recycle-once-note">这些选项仅应用于此次解压。</p>':'<label class="qa-check"><input id="oa-remember-extract" type="checkbox" checked>记住解压选项</label>'}
  </details>
  ${deepExtract.limitsMarkup()}
 `,recycleSummary({statusId:'oa-extract-source-status',recycleId:'oa-recycle-after',recycle:recycleOnce||(deep?s.deepRecycleAfter:s.recycleAfter)})+
 btn('取消','close')+btn('开始解压','startextract','primary','folder-output'),'oa-extraction-dialog oa-extraction-form');
 const recycle=$('oa-recycle-after');
 recycle.dataset.wanted=String(recycle.checked);
 recycle.dataset.ordinaryWanted=String(recycleOnce||s.recycleAfter);
 recycle.dataset.deepWanted=String(recycleOnce||s.deepRecycleAfter);
 deepExtract.sync();syncRecycleOption();
 labelExtractionShortcut('startextract');
 $('qa-destination').focus({preventScroll:true});
 const details=$('oa-destination-details');
 details.addEventListener('toggle',()=>{
  if(details.open)prepareDestinationPlaces(details);
  refreshDestinationFolders();
 });
 overlay.querySelector('[data-action=extract-original]').disabled=!archiveParent(s.archive);
}
async function submitExtraction(){
 const dest=$('qa-destination').value.trim();
 if(!dest)throw Error('请选择目标文件夹。');
 const source=s.archive,selected=extractSelection(),sameFolder=$('qa-smart').checked;
 const overwrite=$('qa-conflict').value,recycle=$('oa-recycle-after').checked,deep=$('oa-deep-extract').checked;
 const openAfter=$('oa-open-after').checked,closeAfter=$('oa-close-after').checked;
 if(recycle&&(selected.length||overwrite==='skip'))throw Error('仅全部解压且不跳过已有文件时，才能自动移入回收站。');
 const limits=deep?deepExtract.settings():null;
 const requiresPassword=extractPasswordRequired(s.entries,selected,s.headerEncrypted),input=$('qa-extract-password');
 const password=input.disabled?null:input.value||null;
 if(requiresPassword&&!password){input.focus();throw Error('请输入密码。');}
 if($('oa-remember-extract')?.checked){
  Object.assign(s,{lastDest:dest,sameFolder,overwrite,deepExtract:deep,openAfter,closeAfter});
  if(!$('oa-recycle-after').disabled)s[deep?'deepRecycleAfter':'recycleAfter']=recycle;
  if(limits)s.deepLimits=limits;
  appearance();
 }
 await runExtraction({source,dest,selected,sameFolder,overwrite,recycle,deep,openAfter,closeAfter,limits,password,requiresPassword,codepage:s.codepage||null});
}
async function runExtraction(config,retryFrom=null){
 const {source,dest,selected,sameFolder,overwrite,recycle,deep,openAfter,closeAfter,limits,password,requiresPassword,codepage}=config;
 const entries=s.entries;
 const request={operation:deep?'deep-extract':'extract',path:source,dest,smart:sameFolder,
   ...(deep?{deep:{...limits,dedicated_output:true}}:{}),
   ...(!deep&&selected.length&&!password?{allow_unencrypted_projection:true}:{}),
   options:{selected,overwrite,password,codepage}};
 let job,result;
 try{
  result=await run(request,deep?'深度解压':selected.length?'解压选中':sameFolder?'智能解压':'解压到此处',{
   recycleAfter:recycle,retryFrom,onStart:started=>{job=started;}
  });
 }catch(error){
  if(!deep&&(job?.status==='failed'||retryFrom&&job?.status==='cancelled')){
   const {password:_,...retry}=config;
   const passwordError=archivePasswordError(error);
   const location=retry.location??extractionLocation(source,entries,selected,sameFolder,false);
   ordinaryRetries.set(job.id,{job,...retry,location,requiresPassword:requiresPassword||passwordError});
   job.retryLabel=passwordError?'输入密码重试':/exists|conflict|同名|已存在/i.test(String(error))?'处理冲突并重试':'调整选项并重试';
   s.inlineExpanded=true;renderInlineTask();renderHistoryJob(job);
  }
  throw error;
 }
 if(deep)await deepExtract.finish(result,{openAfter,closeAfter,limits,overwrite});
 else await finishExtraction(source,dest,result,{recycle,openAfter,closeAfter,job});
}
function openExtractionRetry(id){
 const request=ordinaryRetries.get(id);
 if(!request)return;
 const job=request.job,passwordError=job.retryLabel==='输入密码重试';
 dialog('重试解压',`
  <p class="oa-extract-source"><span class="qa-sub">压缩包</span> <strong title="${esc(request.source)}">${esc(filename(request.source))}</strong></p>
  <p class="qa-sub oa-extract-scope-detail" tabindex="0">${esc(request.location.item?`包内位置 ${request.location.item}`:request.selected.length?`原选定范围（${request.selected.length} 项）`:'范围为整个压缩包')}</p>
  ${job?.error?`<p class="oa-extract-problem qa-jobresult">${esc(passwordError?'上次解压需要正确的密码。':taskError(job.error))}</p>`:''}
  <input id="oa-retry-id" type="hidden" value="${id}">
  <div class="qa-field"><label for="oa-retry-dest">保存到</label><div class="qa-inlinefield oa-extract-target"><input id="oa-retry-dest" value="${esc(request.dest)}" spellcheck="false">${btn('浏览…','extract-retry-dest')}</div></div>
  ${extractionPathMarkup({id:'oa-retry-output',label:request.location.label,copyAction:'copy-extract-retry-output'})}
  ${request.requiresPassword?'<label class="qa-field">密码<input id="oa-retry-password" type="password" autocomplete="off" data-password-book></label>':''}
  <label class="qa-field oa-extract-retry-conflict">同名文件<select id="oa-retry-conflict" aria-describedby="oa-retry-conflict-help">${conflicts(request.overwrite)}</select></label>
  <p id="oa-retry-conflict-help" class="qa-sub oa-extract-hint" role="status"></p>
 `,(request.recycle?recycleSummary({statusId:'oa-retry-source-status',recycleId:'oa-retry-recycle',recycle:true}):'')+
 btn('返回','close')+btn('重新解压','extract-retry-submit','primary'),'oa-extraction-dialog oa-extraction-form');
 updateExtractionRetry();
 labelExtractionShortcut('extract-retry-submit');
 (passwordError?$('oa-retry-password'):$('oa-retry-conflict')).focus({preventScroll:true});
}
function updateExtractionRetry(){
 const request=ordinaryRetries.get(Number($('oa-retry-id').value));
 const dest=$('oa-retry-dest').value.trim(),recycle=$('oa-retry-recycle')?.checked;
 const blocked=recycle&&$('oa-retry-conflict').value==='skip';
 setExtractionPreview($('oa-retry-output'),dest,request.location.relative);
 updateExtractConflict('oa-retry-conflict',false);
 const status=$('oa-retry-source-status');
 if(status){
  status.textContent=blocked?'选择其他同名处理方式，或关闭成功后回收。':sourceDisposition(recycle);
  status.dataset.recycle=String(!!recycle&&!blocked);
 }
 const submit=overlay.querySelector('[data-action=extract-retry-submit]');
 submit.textContent=recycle?'重新解压并回收':'重新解压';
 submit.disabled=!dest||blocked;
}
async function submitExtractionRetry(){
 const id=Number($('oa-retry-id').value),request=ordinaryRetries.get(id);
 if(!request)return;
 const dest=$('oa-retry-dest').value.trim(),input=$('oa-retry-password'),password=input?.value||null,overwrite=$('oa-retry-conflict').value;
 if(!dest)throw Error('请选择目标文件夹。');
 if(input&&!password){input.focus();throw Error('请输入密码。');}
 const recycle=!!$('oa-retry-recycle')?.checked;
 if(recycle&&overwrite==='skip')throw Error('请选择其他同名文件处理方式，或关闭成功后回收。');
 const {job,...retry}=request;
 await runExtraction({...retry,dest,password,overwrite,recycle},job);
}
async function quickExtract(){
 if(!s.archive)return;
 const source=s.archive,dest=archiveParent(source),selected=[...s.selected];
 if(!dest||(!selected.length&&(s.search||s.folder||(s.deepExtract?s.deepRecycleAfter:s.recycleAfter)))){extract();return;}
 const requiresPassword=extractPasswordRequired(s.entries,selected,s.headerEncrypted);
 if(requiresPassword&&!s.password){extract();return;}
 await runExtraction({source,dest,selected,sameFolder:s.sameFolder,overwrite:s.overwrite,recycle:false,deep:s.deepExtract,
  openAfter:s.openAfter,closeAfter:s.closeAfter,limits:s.deepLimits,password:requiresPassword?s.password||null:null,requiresPassword,codepage:s.codepage||null});
}
async function entry(name,{preview=false}={}){const e=visibleEntries().find(e=>e.name===name);if(!e)throw Error('此条目不在当前列表中，请重新选择。');if(e.is_dir){gotoArchiveFolder(name,{focus:true});return;}if(s.active&&(!s.backgroundExtraction||!isText(name)||s.jobs.some(job=>job.id===s.active&&archiveKey(job.path)===archiveKey(s.archive))))throw Error('请先等待或取消当前任务。');if(isNestedArchive(name)&&!preview){await openNestedEntry(name);return;}if(isImage(name)&&!preview){if(openingImage)return;openingImage=true;const generation=previewGeneration;toast('正在准备原始图片…');try{await invoke('open_archive_image',previewArgs(name));if(generation===previewGeneration)toast('已用系统默认看图程序打开原始图片。');}finally{openingImage=false;}return;}
 const textFile=isText(name),folder=e.name.slice(0,Math.max(0,e.name.lastIndexOf('/')));
 dialog(e.leaf,`
  <div class="qa-sub oa-preview-details">
   ${folder?`<div>包内目录 <bdi>${esc(folder)}</bdi></div>`:''}
   <div>大小 ${entrySize(e)}</div>
  </div>
  <div id="qa-file-preview" role="status">${textFile?'正在读取文本…':''}</div>`,
  btn('返回','close')+btn('解压此文件','extractfile','primary'),
  textFile?'oa-file-preview-dialog oa-text-dialog':'oa-file-preview-dialog');
 s.focusFile=e.name;
 const target=$('qa-file-preview'),generation=previewGeneration;
 if(textFile){target.innerHTML=`<div class="oa-text-toolbar"><label>文本编码<select id="oa-text-encoding">${opts([['auto','自动检测'],['utf-8','UTF-8'],['gbk','简体中文 GBK'],['utf-16le','UTF-16 LE'],['utf-16be','UTF-16 BE'],['windows-1252','西欧 Windows-1252']],'auto')}</select></label><span id="oa-text-meta" class="oa-preview-meta" role="status"></span></div><pre id="oa-text-preview" class="oa-text-preview" tabindex="0"></pre>`;target.oaTextContext={name,generation,args:previewArgs(name)};await loadTextPreview(target,'auto');}else if(isImage(name)||/\.avif$/i.test(name)){try{const r=await run({operation:'preview',path:s.archive,name,options:archiveOptions()},'图片预览',{silent:true});if(target===$('qa-file-preview')&&generation===previewGeneration){const img=document.createElement('img');img.src=r.data_url;img.alt=e.leaf;img.className='qa-preview';target.append(img);}}catch(e){if(target===$('qa-file-preview'))error(e);}}else{target.textContent='此文件暂不支持内嵌预览，可解压后打开。';}}
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
   <label class="oa-switch-row"><span>保留来源访问时间</span><input id="oa-add-preserve-atime" class="oa-switch" role="switch" type="checkbox" ${window.QingInputAtime.preservePreference()?'checked':''}></label>
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
 const notice=QingInputAtime.preservationNotice(result);
 if(notice)showPreservationNotice(notice);
}
async function action(a){switch(a){
case'close':close();break;case'keep-running':$('oa-exit-confirm')?.remove();overlay.querySelectorAll('.qa-dialog').forEach(e=>e.inert=false);overlay.dataset.page=s.returnPage||'';if(s.view==='jobs')renderProgress();else if(!overlay.querySelector('.qa-dialog')){overlay.hidden=true;root.querySelector('.qa-shell').inert=false;if($('oa-task-panel'))$('oa-task-panel').inert=false;}break;
case'sources-remove':{
 const input=$('qa-inputs')||$('qa-edit-inputs');
 const checked=[...overlay.querySelectorAll('[data-source-index]:checked')];
 if(!checked.length)break;
 const removed=new Set(checked.map(field=>Number(field.dataset.sourceIndex)));
 input.value=lines(input.id).filter((_,i)=>!removed.has(i)).join('\n');
 refreshSources();
 const remaining=overlay.querySelectorAll('[data-source-index]');
 const index=Math.min(Number(checked[0].dataset.sourceIndex),remaining.length-1);
 (remaining[index]||overlay.querySelector('[data-action$="-files"]'))?.focus();
 break;
}
case'password-settings':{const panel=$('oa-password-panel');if(!compressionCanEncrypt($('qa-format').value)){$('oa-password-unsupported').scrollIntoView({block:'nearest'});break;}panel.hidden=!panel.hidden;$('oa-password-button').setAttribute('aria-expanded',String(!panel.hidden));if(!panel.hidden)$('qa-new-password').focus();break;}
case'compression-clear-password':{delete $('qa-new-password').closest('.qa-dialog').dataset.retryPasswordRequired;$('qa-new-password').value='';$('oa-password-again').value='';syncCompressionPassword();$('qa-formerror').textContent='';break;}
case'compression-advanced':{
 const panel=$('oa-compression-advanced'),toggle=overlay.querySelector('[data-action=compression-advanced]');
  showCompressionAdvanced(panel.hidden);
  if(!panel.hidden)toggle.scrollIntoView({block:'nearest'});
 break;
}
case'destination-up':await setDestination(extractionParent($('qa-destination').value));break;case'cancel-exit':{s.batchStop=true;const box=$('oa-exit-confirm');box?.querySelectorAll('button').forEach(b=>b.disabled=true);try{if(!await saveBatchBeforeExit(box)){box?.querySelectorAll('button').forEach(b=>b.disabled=false);return;}if(s.compressionJob){s.compressionJob.cancelRequested=true;if(Number.isInteger(s.active))await invoke('cancel_operation',{id:s.active});while(s.compressionJob)await new Promise(resolve=>setTimeout(resolve,75));}if(await invoke('cancel_and_close')===false){await action('keep-running');const finish=()=>{if(tabBusy()){setTimeout(finish,100);return;}closeArchiveTabs('exit');};finish();}}catch(e){box?.querySelectorAll('button').forEach(b=>b.disabled=false);throw e;}break;}case'home-smart':await pickHomeArchives();break;case'home-review-submit':submitHomeReview();break;case'open-submit-deep':{const path=$('qa-open-path').value.trim(),password=$('qa-open-password').value;if(path)deepExtract.batch([path],{password,allowBrowse:true});break;}case'open':{if(s.backgroundExtraction){openDialog();break;}if(homeIntakeAvailable()){await pickHomeArchives();break;}const paths=await invoke('pick_files');enqueueArchives(paths);break;}case'open-path':openDialog();break;case'browse-open':{const p=await invoke('pick_files');if(p[0]&&$('qa-open-path')){$('qa-open-path').value=p[0];offerRepairAfterOpenFailure('','');}break;}case'open-submit':{
 const pathInput=$('qa-open-path'),passwordInput=$('qa-open-password'),dialogNode=pathInput?.closest('.qa-dialog');
 if(!dialogNode||pathInput.disabled)break;
 const path=pathInput.value.trim(),password=passwordInput.value;
 if(!path)throw Error('请选择压缩包。');
 offerRepairAfterOpenFailure('','');
 passwordBook.closePicker(passwordInput);
 const controls=[pathInput,passwordInput,...dialogNode.querySelectorAll('[data-action=open-submit],[data-action=browse-open],[data-action=open-submit-deep],[data-password-picker]')];
 const message=dialogNode.querySelector('#qa-formerror');
 const closeButton=dialogNode.querySelector('[data-action=close]');
 const loading=document.createElement('div');
 loading.innerHTML='<p id="qa-open-status" class="qa-sub" role="status">正在打开压缩文件 · 进度待定</p>'+QingProgress.markup('aria-labelledby="qa-open-status"');
 message.textContent='';
 message.before(loading);
 dialogNode.setAttribute('aria-busy','true');
 controls.forEach(control=>control.disabled=true);
 closeButton.textContent='取消读取';
 try{
  await openArchive(path,password,{onProgress:job=>{
   const percent=QingProgress.value(job);
   QingProgress.render(loading.lastElementChild,percent);
   loading.firstElementChild.textContent=job.cancelRequested?'正在取消…':`正在打开压缩文件 · ${percent===null?'进度待定':percent+'%'}`;
  }});
 }catch(e){
  if(dialogNode.isConnected&&overlay.contains(dialogNode)){
   message.textContent=taskError(e);
   offerRepairAfterOpenFailure(path,e);
  }else toast(taskError(e));
 }finally{
  if(dialogNode.isConnected&&overlay.contains(dialogNode)){
   controls.forEach(control=>control.disabled=false);
   dialogNode.removeAttribute('aria-busy');
   closeButton.textContent='取消';
   loading.remove();
  }
 }
 break;
}
case'compress':compress();break;
case'compress-files':{
 const input=$('qa-inputs');
 appendArchiveInputs(input,await invoke('pick_files_for_zip'));
 break;
}
case'compress-folder':{
 const input=$('qa-inputs');
 appendArchiveInputs(input,[await invoke('pick_folder')]);
 break;
}
case'compress-save':{
 const save=$('qa-save'),separate=$('oa-separate').checked,format=$('qa-format').value;
 const suggestion=applyCompressionExtension(save.value.trim(),format);
 const path=separate?await invoke('pick_folder'):await invoke('pick_archive_save',{format,suggestion});
 if(path&&save.isConnected&&separate===$('oa-separate').checked&&format===$('qa-format').value){
  save.value=path;
  save.removeAttribute('aria-invalid');
  $('qa-formerror').textContent='';
  changeCompressionSave(save);
 }
 break;
}
case'startcompress':await submitCompression();break;
case'deep-batch':deepExtract.batch();break;case'extract':extract();break;case'extract-selected':if(s.selected.size)extract();else toast('请在当前列表中选择要解压的项目。');break;case'extractfile':s.selected=new Set([s.focusFile]);extract();break;case'extract-original':await setDestination(archiveParent(s.archive));break;
case'extract-dest':{const input=$('qa-destination'),path=await invoke('pick_folder');if(path&&input===$('qa-destination'))await setDestination(path);break;}
case'copy-extract-output':case'copy-extract-retry-output':{const path=$(a==='copy-extract-output'?'oa-expected-output':'oa-retry-output').dataset.path;if(path){await navigator.clipboard.writeText(path);toast('已复制预计路径。');}break;}
case'startextract':await submitExtraction();break;
case'quick-extract':await quickExtract();break;
case'extract-deep':extract({forceDeep:true});break;
case'extract-standard':extract({forceStandard:true});break;
case'extract-menu':showExtractMenu();break;
case'extract-retry-submit':await submitExtractionRetry();break;
case'extract-retry-dest':{const input=$('oa-retry-dest'),path=await invoke('pick_folder');if(path&&input===$('oa-retry-dest')){input.value=path;updateExtractionRetry();}break;}
case'root':gotoArchiveFolder('',{focus:true});break;case'up':if(s.folder)gotoArchiveFolder(memberDirectory(s.folder),{name:s.folder,focus:true});break;case'test':if(s.archive&&!tabBusy())await run({operation:'test',path:s.archive,options:archiveOptions()},'完整性检查');break;
case'shell-continue':await retryShellExtraction();break;case'shell-password':showShellPassword();break;case'shell-password-submit':await retryShellExtraction(true);break;case'shell-clear':clearShellQueue();break;case'task-toggle':s.inlineToggled=true;s.inlineExpanded=!s.inlineExpanded;renderInlineTask();break;
case'task-details':{const job=s.jobs.findLast(job=>job.id===s.inlineTask);if(job){job.inlineDetails=$('oa-task-details').getAttribute('aria-expanded')!=='true';renderInlineTask();}break;}
case'task-dismiss':{const job=s.jobs.findLast(job=>job.id===s.inlineTask);if(job&&job.status!=='running'){const restore=$('oa-task-panel').contains(document.activeElement);job.inlineDismissed=true;renderInlineTask();if(restore)root.querySelector('[data-view=history]').focus({preventScroll:true});}break;}
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
case'shell-settings':openSettingSection('oa-shell-settings-title');break;
case'batch-pick':{const input=$('qa-batch-inputs'),paths=await invoke('pick_files');if(paths.length&&input?.isConnected){
 const existing=lines(input.id),seen=new Set(existing);
 input.value=[...existing,...paths.filter(path=>!seen.has(path))].join('\n');updateBatchPage();
}break;}
case'batch-dest':{const input=$('qa-batch-dest'),path=await invoke('pick_folder');if(path&&input?.isConnected){input.value=path;updateBatchPage();}break;}
case'batch-result':{
 navigate('batch');
 const result=$('oa-batch-result');
 if(result){result.tabIndex=-1;result.focus();result.scrollIntoView({block:'nearest'});}
 break;
}
case'batch-output':if(s.batchResult?.hasOutput)await invoke('reveal_path',{path:s.batchResult.destination});break;
case'batch-output-saved':if(s.savedBatch?.hasOutput)await invoke('reveal_path',{path:s.savedBatch.destination});break;
case'batch-resume':if(s.batchResult?.entries.some(entry=>entry.status==='pending'))await useBatchResume(s.batchResult);break;
case'batch-resume-saved':
 if(s.savedBatch?.overwrite==='overwrite')showBatchResumeReview(s.savedBatch);
 else await useBatchResume(s.savedBatch);
 break;
case'batch-confirm-resume':await confirmBatchResume();break;
case'batch-backup-damaged':await backupDamagedBatchResume();break;
case'batch-discard':await useBatchResume(s.savedBatch,true);break;
case'batch-start':{
 if(s.active||s.batchRunning)throw Error('已有任务正在运行。');
 const paths=lines('qa-batch-inputs'),operation=$('qa-batch-op').value,dest=$('qa-batch-dest').value.trim();
 const overwrite=$('qa-batch-conflict').value;
 if(!paths.length||(operation==='extract'&&!dest))throw Error('请填写压缩包列表和解压目标。');
 const entries=paths.map((path,index)=>({path,index,status:'pending',dest:operation==='extract'?batchExtractionTarget(dest,path,index):''}));
 await runBatch({operation,overwrite,destination:operation==='extract'?dest:'',entries});
 break;
}
case'profile-save':{const name=$('qa-profile-name').value.trim();if(!name)throw Error('请输入预设名称。');s.profiles=s.profiles.filter(p=>p.name!==name);s.profiles.push({name,format:$('qa-profile-format').value,level:Number($('qa-profile-level').value)});localStorage.setItem('qing-profiles',JSON.stringify(s.profiles));render();break;}
}}

const menuGroups={
 file:[['打开压缩文件','open','Ctrl+O'],['输入文件路径…','open-path',''],['最近的文件','recent-menu',''],['新建压缩文件','compress','Ctrl+N'],null,['关闭当前标签','close-archive','Ctrl+W'],['关闭所有标签','close-all-tabs',''],['保存至外层压缩包','save-nested','Ctrl+S'],['另存为…','save-as',''],['删除压缩文件','recycle-archive','Ctrl+Del'],['打开压缩文件位置','reveal',''],null,['压缩包属性','archive-properties',''],['测试压缩文件','test','Ctrl+T'],['解压到…','extract','Ctrl+E'],['解压并移入回收站…','extract-recycle',''],null,['退出','exit','Alt+F4']],
 edit:[['作为压缩包打开','open-as-archive',''],['外部编辑…','external-edit',''],['外部编辑会话…','external-sessions',''],['保留的编辑副本…','external-copies',''],null,['重命名文件','rename','F2'],['添加文件','add','Ins'],['删除文件','delete','Del'],['压缩文件注释','comment','Ctrl+Shift+M'],null,['全选','select-all','Ctrl+A'],['反向选择','invert',''],['按名称选中…','select-mask','小键盘 +'],['按名称取消选中…','unselect-mask','小键盘 −'],['选中同类型','select-type','Alt+小键盘 +'],['取消选中同类型','unselect-type','Alt+小键盘 −'],['复制完整路径','copy-path',''],['将文件复制到剪贴板','copy-files','Ctrl+C'],['从剪贴板粘贴文件','paste-files','Ctrl+V'],null,['属性','properties','Alt+Enter']],
 find:[['查找文件','find','Ctrl+F']],
 options:[['设置','settings','F5'],['解压密码本…','password-book',''],['压缩预设…','profiles','']],
 view:[['启动画面','close-archive','F6'],['树形目录','toggle-tree','F7'],['平铺视图','toggle-flat',''],['文件预览','preview-selected',''],['刷新当前标签','refresh-archive','Ctrl+R'],['状态栏','toggle-status',''],['文件名代码页…','codepage',''],null,['工具栏：大','toolbar-large',''],['工具栏：小','toolbar-small',''],['工具栏：隐藏','toolbar-hidden',''],null,['列表列…','columns',''],['详细信息','details',''],['图标','icons',''],['颜色设置…','settings','']],
 tools:[['修复打不开的 ZIP…','repair-tool',''],['密码恢复…','recover-tool',''],['文件校验值…','checksum-tool',''],['批量处理…','batch',''],['资源管理器右键设置…','shell-settings','']],
 help:[['关于 qing-zip','about','F1'],['使用说明','help','']]
};
function updateToolbar(){root.querySelectorAll('#oa-toolbar [data-action],#oa-primary-action [data-action]').forEach(b=>{const a=b.dataset.action;b.disabled=(!s.archive&&!['open','compress','more-menu'].includes(a))||(!!s.active&&!['toggle-tree'].includes(a)&&!(s.backgroundExtraction&&['open','more-menu'].includes(a)));if(a==='quick-extract'&&b.lastChild)b.lastChild.textContent=quickExtractLabel();if(a==='toggle-tree')b.setAttribute('aria-expanded',String(root.dataset.tree!=='hidden'));});const maintenance=$('oa-archive-maintenance');if(maintenance)maintenance.disabled=!!s.active;if($('oa-selection-actions')){$('oa-selection-actions').hidden=!s.selected.size;$('oa-selected-count').textContent=`已选 ${s.selected.size} 个项目`;$('oa-selection-actions').querySelectorAll('[data-action]').forEach(b=>b.disabled=!!s.active||(b.dataset.action==='rename'&&s.selected.size!==1));}const page=s.page==='password-book'?passwordBookOrigin:s.page;root.querySelectorAll('[data-view]').forEach(b=>{b.setAttribute('aria-current',String(b.dataset.view===(page==='tool'?toolContext.origin:['batch','profiles'].includes(page)?'tools':page==='associations'?'settings':page)));});}
// Sorted descendants are contiguous, so tree expansion does not rescan every folder.
const archiveFolderTrees = new WeakMap();
function folderTree(entries, collapsed) {
  let tree = archiveFolderTrees.get(entries);
  if (!tree) {
    const names = [...archiveIndex(entries).paths].sort();
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
 const arrow=root.querySelector('[data-action=extract-menu]');
 arrow.setAttribute('aria-expanded','true');
 positionMenu(menu,arrow.parentElement,true);
 menu.querySelector('button')?.focus();
}
function renderChrome(){
 renderArchiveTabs();
 root.dataset.page=s.page;
 const browsing=s.page==='files'&&!!s.archive;
 root.dataset.browsing=String(browsing);
 $('oa-toolbar').hidden=!browsing;
 $('oa-heading').hidden=true;
 root.querySelector('.oa-tree-section').hidden=!browsing;
 if(root.querySelector('[data-action=extract-menu][aria-expanded=true]')||$('oa-menu').querySelector('[data-archive-maintenance-menu]'))hideMenu();
 $('oa-toolbar').innerHTML=btn('打开','open','','folder-open')+btn('新建','compress','','plus')+(browsing?`<span class="oa-divider" role="separator" aria-orientation="vertical"></span>${btn('添加','add','','plus')}
 <div class="oa-toolbar-end"><div class="oa-extract-split" role="group" aria-label="解压">
 <button type="button" class="qa-button" data-action="quick-extract">${icon('folder-output')}${quickExtractLabel()}</button>
 <button type="button" class="qa-button" data-action="extract-menu" aria-haspopup="menu" aria-expanded="false" aria-controls="oa-menu" aria-label="更多解压方式" title="更多解压方式">${icon('chevron')}</button>
 </div>${btn('目录树','toggle-tree','quiet','folder')}</div>`:'');
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
 if(index === undefined)return false;
 s.cursor=name;
 if(listingWindow)listingWindow.reveal(index);
 const buttons=rowButtons(),target=buttons.find(b=>b.dataset.entry===name);
 if(!target)return false;
 for(const button of buttons)button.tabIndex=button===target?0:-1;
 target.focus({preventScroll:true});
 // Row selection keeps the user's horizontal column position.
 const viewport=archiveViewport(),bounds=viewport.getBoundingClientRect();
 const header=viewport.querySelector('.qa-table:not(.oa-icons) thead th');
 const top=header?.getBoundingClientRect().bottom??bounds.top;
 const bottom=bounds.top+viewport.clientHeight;
 const row=target.closest('[data-row]').getBoundingClientRect();
 if(row.top<top||row.height>bottom-top)viewport.scrollTop+=row.top-top;
 else if(row.bottom>bottom)viewport.scrollTop+=row.bottom-bottom;
 if(s.listStyle==='details'&&target.getBoundingClientRect().right<=bounds.left){
  target.scrollIntoView({block:'nearest',inline:'nearest'});
 }
 return true;
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
 if(key==='Enter'&&ctrl&&s.search){event.preventDefault();event.stopPropagation();locateSearchResult(name);return;}
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
 const height=row.getBoundingClientRect().height||48,pageRows=Math.max(1,Math.floor(archiveViewport().clientHeight/height)-1);
 const target=list[fileKeyIndex(key,index,list.length,columns,pageRows)].name;
 if(event.shiftKey&&!positions.has(s.anchor))s.anchor=name;
 if(ctrl&&!event.shiftKey)focusEntry(target);else selectRow(target,{shiftKey:event.shiftKey,ctrlKey:ctrl});
}
function handleArchiveSearchKeydown(event){
 if(event.defaultPrevented||isCompositionKey(event)||!overlay.hidden||!event.target.closest('.oa-archive-browser'))return;
 if(event.key==='Escape'&&s.search){
  event.preventDefault();event.stopPropagation();
  clearArchiveSearch(!!event.target.closest('[data-row]'));
 }else if(event.target.id==='qa-search'&&s.search&&!event.altKey&&!event.ctrlKey&&!event.metaKey&&['Enter','ArrowDown'].includes(event.key)){
  const first=visibleEntries()[0];
  if(first){event.preventDefault();focusEntry(first.name);}
 }
}
function handleMenuKeydown(event){
 if(isCompositionKey(event))return;
 const box=$('oa-menu');if(box.hidden||!box.contains(event.target))return;
 const items=[...box.querySelectorAll('button')].filter(b=>!b.disabled),index=items.indexOf(document.activeElement);
 if(event.key==='Escape'||event.key==='Tab'){
  if(event.key==='Escape')event.preventDefault();event.stopImmediatePropagation();event.stopPropagation();
  hideMenu();if(menuReturnFocus?.isConnected)menuReturnFocus.focus();return;
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
function refreshSelection(){
 for(const tr of root.querySelectorAll('[data-row]')){
  const selected=s.selected.has(tr.dataset.row);
  tr.classList.toggle('selected',selected);tr.setAttribute('aria-selected',String(selected));
  const box=tr.querySelector('[data-select]');if(box)box.checked=selected;
 }
 const list=visibleEntries(),status=$('qa-selection'),all=$('qa-all'),locate=$('oa-search-locate');
 if(status){
  const text=s.search?`已选 ${s.selected.size} 项`:`${list.length} 个项目 · 已选 ${s.selected.size} 项`;
  if(status.textContent!==text)status.textContent=text;
 }
 if(all){
  const positions=positionsFor(list);let count=0;
  for(const name of s.selected)if(positions.has(name))count++;
  all.disabled=!list.length;all.checked=list.length>0&&count===list.length;all.indeterminate=count>0&&!all.checked;
 }
 if(locate){
  locate.disabled=s.selected.size!==1;
  locate.title=locate.disabled?'选择一项结果后可定位':'转到所在目录 (Ctrl+Enter)';
 }
 updateToolbar();
}
let toolContext;
function showTool(op,title,{path='',scope='',origin=s.page==='files'?'files':'tools',returnTarget=null}={}){
 if(tabBusy()||closingArchiveTabs||finishingArchiveTabs||removingArchiveTabs){toast('请先等待当前操作结束。');return;}
 const params=new URLSearchParams({operation:op});
 if(path)params.set('path',path);
 if(scope==='archive'){
  params.set('scope',scope);
  if(s.codepage)params.set('codepage',s.codepage);
 }
 const frameUrl=new URL('tools.html?'+params,location.href).href;
 toolContext={op,title,scope,origin,frameUrl,returnTarget,tabId:scope==='archive'?currentTabId:null};
 navigate('tool');
}
function mountArchiveTool(frame,context){
 frame.addEventListener('load',()=>{
  const tab=archiveTabs.find(tab=>tab.id===context.tabId);
  if(!tab)return;
  if(tab.password)frame.contentWindow.setArchiveToolPassword?.(tab.password);
  if(context.op==='preview'){
   const here=[],other=[];
   for(const entry of tab.entries){
    if(entry.is_dir||!isImage(entry.name))continue;
    const name=entry.name.replaceAll('\\','/');
    const folder=name.slice(0,name.lastIndexOf('/')+1);
    (folder===tab.folder?here:other).push(entry.name);
   }
   frame.contentWindow.setArchiveToolImages?.(here.concat(other));
  }
 },{once:true});
}
function openSettingSection(id){
 navigate('settings');
 const heading=$(id);
 heading?.focus({preventScroll:true});
 heading?.scrollIntoView({block:'start'});
}
const archiveMaintenanceActions=[
 ['完整性检查','test','Ctrl+T'],
 ['修复 ZIP 压缩包…','archive-repair','',()=>repairableZip(s.nestedMember||s.archive)],
 ['文件校验值…','archive-checksum',''],
 ['图片预览…','archive-preview','']
];
function showArchiveMaintenance(){
 if(s.page!=='files'||!s.archive||tabBusy())return;
 const anchor=$('oa-archive-maintenance');
 if(!anchor)return;
 rememberMenuFocus();
 anchor.setAttribute('aria-expanded','true');
 const box=$('oa-menu');
 box.innerHTML=`<div class="oa-menu-title" data-archive-maintenance-menu>归档维护</div>`+
  archiveMaintenanceActions.map(([label,action,shortcut,available])=>{
   const enabled=!available||available();
   return `<button type="button" role="menuitem" data-action="${action}" ${enabled?'':'disabled title="仅支持 ZIP 文件"'}><span>${label}</span><small>${shortcut}</small></button>`;
  }).join('');
 positionMenu(box,anchor,true);
 box.querySelector('button:not(:disabled)')?.focus();
}
async function startArchiveTool(action){
 if(s.page!=='files'||!s.archive||tabBusy())return;
 const tools={
  'archive-repair':['repair','修复 ZIP 压缩包'],
  'archive-checksum':['checksum','文件校验值'],
  'archive-preview':['preview','图片预览']
 };
 const available=archiveMaintenanceActions.find(item=>item[1]===action)?.[3];
 if(available&&!available())return;
 const [op,title]=tools[action];
 if(op==='preview'&&s.selected.size){
  const choice=visibleEntries().find(item=>!item.is_dir&&s.selected.has(item.name)&&isImage(item.name));
  if(choice){
   $('oa-archive-maintenance')?.focus({preventScroll:true});
   await entry(choice.name,{preview:true});
   return;
  }
 }
 showTool(op,title,{path:s.archive,scope:'archive',origin:'files'});
}
function startFailedZipRepair(){
 const path=overlay.querySelector('[data-action=open-repair]')?.dataset.failedPath;
 if(!path||path!==$('qa-open-path')?.value.trim())return;
 const recentIndex=returnFocus?.dataset.recent;
 const returnTarget=recentIndex!=null&&s.recent[recentIndex]===path
  ? '[data-recent="'+recentIndex+'"]'
  : returnFocus?.dataset.action==='open'?'#oa-toolbar [data-action=open]':null;
 showTool('repair','修复 ZIP 压缩包',{path,origin:'files',returnTarget});
}
function returnFromTool(){
 const {origin,scope,returnTarget}=toolContext;
 if(origin==='files'){
  navigate('files');
  if(s.page!=='files')return;
  const target=scope==='archive'?$('oa-archive-maintenance'):
   (returnTarget&&root.querySelector(returnTarget))||$('oa-home-smart-drop')||root.querySelector('#oa-toolbar [data-action=open]');
  target?.focus({preventScroll:true});
 }else if(origin==='settings'){
  openSettingSection('oa-shell-settings-title');
 }else navigate('tools');
}
let destinationRequest=0,destinationTimer;
function extractionParent(path){
 const trimmed=path.trim().replaceAll('/','\\').replace(/\\+$/,'');
 if(/^[A-Z]:$/i.test(trimmed)||/^\\\\[^\\]+(?:\\[^\\]+)?$/.test(trimmed))return '';
 return archiveParent(trimmed);
}
async function prepareDestinationPlaces(details){
 const buttons=details.querySelectorAll('[data-place]');
 try{
  const folders=await invoke('known_folders');
  if(details!==$('oa-destination-details'))return;
  s.known=folders;
  buttons.forEach(button=>button.disabled=!folders[button.dataset.place]);
  $('oa-known-folders-status').hidden=true;
 }catch{
  if(details!==$('oa-destination-details'))return;
  buttons.forEach(button=>button.disabled=true);
  const status=$('oa-known-folders-status');
  status.textContent='Windows 常用位置暂不可用。';
  status.hidden=false;
 }
}
function refreshDestinationFolders(delay=0){
 clearTimeout(destinationTimer);
 const request=++destinationRequest,details=$('oa-destination-details'),input=$('qa-destination');
 if(!details?.open||!input)return;
 const path=input.value.trim(),list=$('oa-destination-folders');
 details.querySelector('[data-action=destination-up]').disabled=!extractionParent(path);
 const status=text=>{list.innerHTML=`<p class="qa-sub" role="status">${text}</p>`;};
 if(!path){status('填写目标路径后可浏览子文件夹。');return;}
 status('正在读取子文件夹…');
 const current=()=>request===destinationRequest&&details===$('oa-destination-details')&&details.open&&input.value.trim()===path;
 const load=async()=>{
  if(!current())return;
  try{
   const dirs=await invoke('list_directory',{path});
   if(!current())return;
   list.innerHTML=dirs.map(e=>`<button type="button" data-directory="${esc(e.path)}" title="${esc(e.path)}">${icon('folder')} <span>${esc(e.name)}</span></button>`).join('')||
    '<p class="qa-sub" role="status">没有子文件夹</p>';
  }catch(error){
   if(current())status(String(error).includes('路径不存在')?'此目录尚未建立，解压时会尝试创建。':'无法读取子文件夹，请检查目标路径。');
  }
 };
 if(delay)destinationTimer=setTimeout(load,delay);
 else return load();
}
async function setDestination(path){
 const input=$('qa-destination');
 if(!input||!path)return;
 if(document.activeElement?.dataset?.directory)input.focus({preventScroll:true});
 input.value=path;
 updateExtractPreview(true);
 await refreshDestinationFolders();
}

const baseAction=action;
const backgroundActions=new Set(['canceljob','task-toggle','task-details','close','keep-running','cancel-exit','find','copy-path','root','up',
 'clear-archive-search','search-whole-archive','locate-archive-result','columns-scroll',
 'toggle-tree','toggle-flat','details','icons','reverse-sort','select-all','invert','select-mask','unselect-mask','select-type','unselect-type',
 'menu','app-menu','more-menu','shell-clear','files-home','reveal','reveal-output','about','help','open','open-path','browse-open','open-submit','preview-selected']);
action=async a=>{if(homeIntake&&a!=='close'&&a!=='exit'){return;}if(s.backgroundExtraction&&!backgroundActions.has(a)){toast('请先等待或取消当前任务。');return;}if($('qa-tools')?.contentWindow?.obzBusy&&!['keep-running','cancel-exit','close'].includes(a)){toast('请先等待或取消当前工具任务。');return;}
 if(/^extract-retry-\d+$/.test(a)){openExtractionRetry(Number(a.slice('extract-retry-'.length)));return;}
 if(a==='clear-tool-search'){
  const input=$('oa-tool-search');
  if(input){input.value='';filterTools(input);input.focus();}
  return;
 }
 switch(a){
  case'columns-scroll':{const viewport=archiveViewport();viewport.scrollLeft=viewport.scrollLeft>1?0:viewport.scrollWidth;updateColumnScroll();return;}
  case'clear-archive-search':clearArchiveSearch();return;
  case'search-whole-archive':searchWholeArchive();return;
  case'locate-archive-result':if(s.selected.size===1)locateSearchResult(s.selected.values().next().value);return;
  case'columns':showColumns();return;case'columns-reset':columnDraft=ArchiveColumns.normalize(null);paintColumns();return;case'columns-apply':{columns=ArchiveColumns.normalize(columnDraft);localStorage.setItem('qing-columns',JSON.stringify(columns));for(const tab of archiveTabs){if(!columns.some(c=>c.visible&&c.id===tab.sort)){tab.sort='name';tab.reverse=false;}}if(!columns.some(c=>c.visible&&c.id===s.sort)){s.sort='name';s.reverse=false;}close();render();focusEntry(s.cursor);}return;case'associations':navigate('associations');if(s.page==='associations')await loadAssociations();return;case'associations-refresh':await loadAssociations();return;case'associations-save':await saveAssociations();return;case'default-apps':await invoke('open_default_apps');toast('请在 Windows 默认应用中选择 qing-zip，完成后返回刷新状态。');return;case'more-menu':case'app-menu':showMore();return;case'extract-recycle':extract({recycleOnce:true});return;case'tools-home':navigate('tools');return;case'tool-back':returnFromTool();return;case'files-home':navigate('files');return;case'comment':{if(!s.archive)throw Error('请先打开 ZIP 压缩文件。');const r=await run({operation:'comment-get',path:s.archive},'读取注释',{silent:true});dialog('压缩文件注释',`<label class="qa-field">ZIP 注释<textarea id="oa-comment">${esc(r.comment)}</textarea></label>`,btn('取消','close')+btn('保存','comment-save','primary'));}return;case'comment-save':{const name=$('oa-comment').value;await run({operation:'comment-set',path:s.archive,name},'保存注释');}return;case'settings':navigate('settings');return;case'profiles':navigate('profiles');return;case'batch':navigate('batch');return;case'close-archive':closeArchiveTabs('current');return;case'close-all-tabs':closeArchiveTabs('all');return;case'refresh-archive':if(s.archive)await openArchive(s.archive,s.password,{refresh:true});return;case'properties':await showProperties(false);return;case'archive-properties':await showProperties(true);return;case'save-nested':await saveNested();return;case'nested-open-submit':await retryNestedOpen();return;case'nested-close-save':await finishNestedClose('save');return;case'nested-close-discard':await finishNestedClose('discard');return;case'exit':await invoke('window_action',{action:'close'});return;case'find':focusArchiveSearch();return;case'select-all':visibleEntries().forEach(e=>s.selected.add(e.name));if($('qa-rows'))refreshSelection();return;case'select-mask':case'unselect-mask':if(s.archive)showSelectionMask(a==='select-mask');return;case'select-type':case'unselect-type':selectFocusedType(a==='select-type');return;case'invert':visibleEntries().forEach(e=>s.selected.has(e.name)?s.selected.delete(e.name):s.selected.add(e.name));if($('qa-rows'))refreshSelection();return;case'toggle-flat':if(!s.archive||(tabBusy()&&!s.backgroundExtraction))return;s.flatView=!s.flatView;{const available=positionsFor(visibleEntries());s.selected=new Set([...s.selected].filter(name=>available.has(name)));if(!available.has(s.anchor))s.anchor=null;if(!available.has(s.cursor))s.cursor=null;}s.page='files';s.view='archive';render();archiveViewport().scrollTop=0;return;case'toggle-tree':root.dataset.tree=root.dataset.tree==='hidden'?'shown':'hidden';updateToolbar();return;case'toggle-status':root.dataset.status=root.dataset.status==='hidden'?'shown':'hidden';return;case'toolbar-large':case'toolbar-small':case'toolbar-hidden':root.dataset.toolbar=a.slice(8);return;case'details':case'icons':s.listStyle=a;saveViewPreference();render();focusEntry(s.cursor);return;case'reverse-sort':s.reverse=!s.reverse;rows();syncViewControls();return;case'external-copies':await showRetainedEdits();return;case'external-retained-open':{const record=currentRetainedEdit();if(!record.nested)await invoke('reveal_path',{path:record.path});return;}case'external-retained-save':await saveRetainedEdit();return;case'external-edit':await beginExternalEdit();return;case'external-editor-default':$('oa-editor-path').value='';return;case'external-editor-pick':{const path=await invoke('pick_editor');if(path)$('oa-editor-path').value=path;}return;case'external-launch':await launchExternalEdit();return;case'external-sessions':await showExternalEdits();return;case'external-copy':if(externalCurrent){const dest=await invoke('pick_copy_save',{path:externalCurrent.path});if(dest){await invoke('copy_archive',{path:externalCurrent.path,dest});toast('已另存为 '+dest);}}return;case'external-save-confirm':confirmExternalFinish(true);return;case'external-discard-confirm':confirmExternalFinish(false);return;case'external-save':await finishExternalEdit(true);return;case'external-discard':await finishExternalEdit(false);return;case'open-as-archive':{const member=selectedNestedMember();if(!member)throw Error('请选中一个文件。');await openNestedEntry(member);}return;case'preview-selected':if(s.selected.size!==1)throw Error('请选中一个文件或文件夹。');await entry([...s.selected][0],{preview:true});return;case'codepage':dialog('文件名代码页',`<label class="qa-field">编码<select id="oa-codepage">${opts([['','自动检测'],['65001','UTF-8'],['936','简体中文 GBK'],['950','繁体中文 Big5'],['949','韩文'],['932','日文 Shift-JIS'],['437','OEM 437']],s.codepage||'')}</select></label>`,btn('取消','close')+btn('应用','codepage-apply','primary'));return;case'codepage-apply':s.codepage=$('oa-codepage').value?Number($('oa-codepage').value):null;await openArchive(s.archive,s.password,{refresh:true});return;case'recent-menu':showRecentDialog();return;case'save-as':if(!s.archive)throw Error('请先打开压缩文件。');{if(/\.\d{3,}$/.test(s.archive))throw Error('分卷请先解压后重新压缩，以免遗漏其他卷。');const dest=await invoke('pick_copy_save',{path:s.archive});if(dest){await invoke('copy_archive',{path:s.archive,dest});toast('已另存为 '+dest);}}return;case'reveal-output':{const path=s.jobs.at(-1)?.outputPath;if(path)await invoke('reveal_path',{path});}return;case'reveal':if(s.archive)await invoke('reveal_path',{path:s.archive});return;case'recycle-archive':if(s.archive)dialog('删除压缩文件',`<p>将 ${esc(filename(s.archive))} 移到回收站？</p>`,btn('取消','close')+btn('移到回收站','recycle-confirm','danger'));return;case'recycle-confirm':if(s.nestedId||archiveTabs.some(t=>archiveKey(t.parentArchive||'')===archiveKey(s.archive)))throw Error('请先保存并关闭内层标签，再删除外层压缩包。');await invoke('recycle_paths',{paths:[s.archive]});close();await action('close-archive');return;case'copy-path':await navigator.clipboard.writeText([...s.selected].map(n=>s.archive+' / '+n).join('\n')||s.archive);toast('已复制路径');return;case'paste-files':{const paths=await invoke('clipboard_paths',{paths:null});if(!paths.length)throw Error('剪贴板中没有文件。');showAddForm(paths);}return;case'copy-files':await copySelectedFiles();return;case'password-book':openPasswordBook();return;case'password-book-back':navigate(passwordBookOrigin);return;case'shell-open-entry':showTool('shell-enable','“使用 qing-zip 打开”右键入口',{origin:'settings'});return;case'archive-maintenance':showArchiveMaintenance();return;case'archive-repair':case'archive-checksum':case'archive-preview':await startArchiveTool(a);return;case'open-repair':startFailedZipRepair();return;case'repair-tool':showTool('repair','修复 ZIP 压缩包');return;case'recover-tool':showTool('recover','密码恢复',{origin:'tools'});return;case'checksum-tool':showTool('checksum','文件校验值');return;case'about':dialog('关于 qing-zip',`<p>qing-zip ${esc(appVersion)}</p><p class="qa-sub">本地压缩与解压工具。第三方组件许可随程序附带。</p>`,btn('关闭','close'));return;case'help':dialog('使用说明','<p>打开压缩包后，单击选择条目，双击进入文件夹或查看文件。Ctrl / Shift 可多选。</p><p>方向键移动，Shift+方向键连续选择，空格勾选，回车打开，Shift+F10 打开右键菜单。</p><p>Ctrl+N 新建，Ctrl+E 解压，Ctrl+T 测试，F2 重命名，Del 删除包内条目。</p><p>Ctrl+Tab 切换标签，Ctrl+Shift+Tab 反向切换，Ctrl+W 关闭当前标签，Ctrl+R 刷新当前标签。标签右键可关闭其他或全部。</p><p>文本双击只读预览；图片双击用系统默认看图程序打开原图。大图标模式可用 Ctrl+滚轮调整尺寸。</p><p>工具菜单提供修复、密码恢复、校验值和批量处理。</p>',btn('关闭','close'));return;}return baseAction(a);};
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
    if(!gesture.cancelled&&ownJob()?.status!=='cancelled'&&!/operation cancelled|已取消拖放/i.test(String(e))){status('failed',taskError(e));throw e;}
    status('cancelled','已取消拖放。');
    toast('已取消拖放。');
  } finally {
    releaseCapture();
    if(dragGesture===gesture)dragGesture=null;
    await invoke('discard_drag',{token:gesture.token}).catch(()=>{});
  }
}
const scrollTimers=new WeakMap();root.addEventListener('scroll',e=>{const el=e.target;if(!(el instanceof HTMLElement))return;el.classList.add('oa-scrolling');clearTimeout(scrollTimers.get(el));scrollTimers.set(el,setTimeout(()=>el.classList.remove('oa-scrolling'),900));},true);
root.addEventListener('click',e=>{if(e.target.closest('#oa-menu')||e.target.closest('[data-menu]')||e.target.closest('[data-action=more-menu],[data-action=extract-menu],[data-action=archive-maintenance]'))return;hideMenu();const row=e.target.closest('[data-row]');if(row&&!e.target.closest('button,input'))selectRow(row.dataset.row,e);});
root.addEventListener('change',e=>{const t=e.target,c=columnDraft?.find(c=>c.id===(t.dataset.columnVisible||t.dataset.columnWidth));if(!c)return;if(t.dataset.columnVisible)c.visible=t.checked;if(t.dataset.columnWidth){c.width=Math.max(64,Math.min(800,Number(t.value)||c.width));t.value=c.width;}});
root.addEventListener('click',e=>{const b=e.target.closest('[data-column-move]');if(!b)return;columnDraft=ArchiveColumns.move(columnDraft,b.dataset.columnMove,Number(b.dataset.delta));paintColumns();$('oa-column-list').querySelector(`[data-column-move="${b.dataset.columnMove}"][data-delta="${b.dataset.delta}"]`)?.focus();});
root.addEventListener('contextmenu',e=>{const recent=e.target.closest('[data-recent]');if(recent){e.preventDefault();showRecentContext(recent,e);return;}const tab=e.target.closest('[data-tab-container]');if(tab){e.preventDefault();tab.querySelector('[data-archive-tab]')?.focus();showArchiveTabMenu(tab.dataset.tabContainer,e);return;}const row=e.target.closest('[data-row]');if(row){e.preventDefault();focusEntry(row.dataset.row);if(!s.selected.has(row.dataset.row))selectRow(row.dataset.row,{});showMenu('edit',{getBoundingClientRect:()=>({left:e.clientX,bottom:e.clientY})});}});
document.addEventListener('keydown',e=>{if(e.defaultPrevented||isCompositionKey(e)||!overlay.hidden)return;const key=e.key.toLowerCase(),ctrl=e.ctrlKey||e.metaKey;if(ctrl&&key==='tab'){e.preventDefault();cycleArchiveTab(e.shiftKey?-1:1);return;}if(ctrl&&key==='w'){e.preventDefault();closeArchiveTabs('current');return;}if(ctrl&&key==='f'&&$('qa-search')){e.preventDefault();focusArchiveSearch();return;}if(['INPUT','TEXTAREA','SELECT'].includes(e.target.tagName))return;const map=ctrl?{o:'open',n:'compress',e:'extract',t:'test',w:'close-archive',r:'refresh-archive',f:'find',a:'select-all',c:s.backgroundExtraction?'copy-path':'copy-files',v:'paste-files',s:'save-nested'}:{f1:'about',f2:'rename',f4:'close-archive',f5:'settings',f6:'close-archive',f7:'toggle-tree',delete:'delete',insert:'add',backspace:'up'};const a=e.altKey&&!ctrl&&key==='enter'?'properties':ctrl&&e.shiftKey&&key==='m'?'comment':ctrl&&key==='delete'?'recycle-archive':map[key];if(a&&s.page!=='files'&&!['open','compress','exit','close-archive','settings','about','toggle-tree'].includes(a))return;if(a){e.preventDefault();action(a).catch(error);}});
root.addEventListener('click',async e=>{const b=e.target.closest('button');if(!b)return;try{if(b.dataset.archiveTab!==undefined){switchArchiveTab(b.dataset.archiveTab);return;}if(b.dataset.tabClose!==undefined){closeArchiveTabs('one',b.dataset.tabClose);return;}if(b.dataset.tabCloseOthers!==undefined){closeArchiveTabs('others',b.dataset.tabCloseOthers);return;}if(b.dataset.menu){showMenu(b.dataset.menu,b);return;}if(b.dataset.treeToggle!==undefined){const p=b.dataset.treeToggle;s.collapsed.has(p)?s.collapsed.delete(p):s.collapsed.add(p);renderChrome();focusTreePath(p);return;}if(b.dataset.breadcrumb!==undefined){gotoArchiveFolder(b.dataset.breadcrumb);root.querySelector('.oa-breadcrumb button:last-child')?.focus();return;}if(b.dataset.tree!==undefined){if((s.active&&!s.backgroundExtraction)||$('qa-tools')?.contentWindow?.obzBusy)throw Error('请先等待或取消当前任务。');gotoArchiveFolder(b.dataset.tree);focusTreePath(b.dataset.tree);return;}if(b.dataset.compressionPage){const job=s.jobs.find(job=>job.id===b.dataset.compressionPage);if(job){job.resultPage=Number(b.dataset.resultPage);if(s.page==='history')render();else renderInlineTask();}return;}if(b.dataset.compressionDraft){const job=s.jobs.find(job=>job.id===b.dataset.compressionDraft);if(job)restoreCompressionDraft(job);return;}if(b.dataset.compressionOutput){await useCompressionResult(b);return;}if(b.dataset.jobOutput){const j=s.jobs.find(j=>String(j.id)===b.dataset.jobOutput),path=j?.outputPath;if(path)await invoke('reveal_path',{path});return;}if(b.dataset.launchTool){if(s.active)throw Error('请先等待或取消当前任务。');showTool(b.dataset.launchTool,b.dataset.toolTitle);return;}if(b.dataset.place){await setDestination(s.known?.[b.dataset.place]||'');return;}if(b.dataset.directory){await setDestination(b.dataset.directory);return;}if(b.dataset.window){await invoke('window_action',{action:b.dataset.window});return;}if(b.dataset.view){navigate(b.dataset.view);return;}if(b.dataset.theme){s.theme=b.dataset.theme;appearance();return;}if(b.dataset.recent!==undefined){const path=s.recent[Number(b.dataset.recent)];try{await openArchive(path);}catch(e){showOpenFailure(path,e);}return;}if(b.dataset.entry){selectRow(b.dataset.entry,e);return;}if(b.dataset.sort){s.reverse=s.sort===b.dataset.sort?!s.reverse:false;s.sort=b.dataset.sort;rows();syncViewControls();return;}if(b.dataset.profile!==undefined){compress(s.profiles[Number(b.dataset.profile)]);return;}if(b.dataset.profileDelete!==undefined){s.profiles.splice(Number(b.dataset.profileDelete),1);localStorage.setItem('qing-profiles',JSON.stringify(s.profiles));render();return;}if(b.dataset.action){const closeExpandedMenu=['extract-menu','archive-maintenance'].includes(b.dataset.action)&&b.getAttribute('aria-expanded')==='true';hideMenu();if(!closeExpandedMenu)await action(b.dataset.action)};}catch(e){error(e);}});
root.addEventListener('input',e=>{if(e.target.id==='qa-open-path')offerRepairAfterOpenFailure('','');if(e.target.id==='oa-icon-size')setIconSize(Number(e.target.value));if(['qa-inputs','qa-edit-inputs'].includes(e.target.id))refreshSources();if(e.target.id==='qa-save'){e.target.dataset.explicit=String(!!e.target.value.trim());clearCompressionPlan();}if(['qa-new-password','oa-password-again'].includes(e.target.id))syncCompressionPassword();if(e.target.id==='qa-volume')clearCompressionPlan();if(e.target.id==='qa-search'&&!e.isComposing)updateArchiveSearch(e.target);});
root.addEventListener('change',e=>{const t=e.target;if(t.id==='oa-add-preserve-atime'){window.QingInputAtime.preservePreference(t.checked);return;}if(timePreferenceIds.includes(t.id)){saveTimePreferences();return;}if(t.id==='oa-file-menu'){t.disabled=true;invoke('set_file_compression_menu',{enabled:t.checked}).then(v=>{fileMenuEnabled=v;t.checked=v;}).catch(e=>{t.checked=fileMenuEnabled;error(e);}).finally(()=>t.disabled=false);return;}if(t.id==='oa-folder-menu'){t.disabled=true;invoke('set_folder_menu',{enabled:t.checked}).then(v=>{folderMenuEnabled=v;t.checked=v;}).catch(e=>{t.checked=folderMenuEnabled;error(e);}).finally(()=>t.disabled=false);return;}if(t.id==='oa-text-encoding'){loadTextPreview($('qa-file-preview'),t.value).catch(error);}if(t.id==='oa-search-scope'){s.searchScope=t.value;if(s.search)applyArchiveFilter();}if(t.id==='oa-sort'){s.sort=t.value;rows();syncViewControls();}if(t.dataset.association!==undefined){t.checked?associationSelection.add(t.dataset.association):associationSelection.delete(t.dataset.association);}if(t.id==='qa-format')changeCompressionFormat();if(t.id==='qa-save')changeCompressionSave(t);if(t.id==='oa-separate')changeCompressionMode();if(t.dataset.sourceIndex!==undefined)syncSourceSelection();if(['oa-method','oa-solid','qa-level'].includes(t.id))syncCompressionControls();if(t.id==='oa-show-password'){for(const id of ['qa-new-password','oa-password-again'])$(id).type=t.checked?'text':'password';}if(t.dataset.select){if(lastSelectShift)selectRow(t.dataset.select,{shiftKey:true});else{t.checked?s.selected.add(t.dataset.select):s.selected.delete(t.dataset.select);s.anchor=t.dataset.select;refreshSelection();}lastSelectShift=false;}if(t.id==='qa-all'){visibleEntries().forEach(v=>t.checked?s.selected.add(v.name):s.selected.delete(v.name));refreshSelection();}const k={'qa-theme':'theme','qa-material':'material','qa-density':'density','qa-defaultconflict':'overwrite'}[t.id];if(k){s[k]=t.value;appearance();}});
root.addEventListener('compositionend',event=>{if(event.target.id==='qa-search')updateArchiveSearch(event.target);});
function isCompositionKey(event){return event.isComposing||event.keyCode===229;}
function handleDialogKeydown(e){if(isCompositionKey(e))return;if(e.key==='Escape'){const menu=$('oa-menu');if(!menu.hidden){e.preventDefault();hideMenu();if(menuReturnFocus?.isConnected)menuReturnFocus.focus();}else if(!overlay.hidden)close();}if(e.key==='Tab'&&!overlay.hidden){const list=[...overlay.querySelectorAll('input,textarea,select,button,summary,[tabindex]')].filter(v=>!v.disabled&&v.tabIndex>=0&&v.getClientRects().length);if(e.shiftKey&&document.activeElement===list[0]){e.preventDefault();list.at(-1)?.focus();}else if(!e.shiftKey&&document.activeElement===list.at(-1)){e.preventDefault();list[0]?.focus();}}}
root.addEventListener('keydown',handleDialogKeydown);
function handleExtractionInput({target}){
 if(target.id==='qa-destination'){
  updateExtractPreview(true);
  refreshDestinationFolders(250);
 }
 if(target.id==='oa-retry-dest')updateExtractionRetry();
 if(['qa-batch-inputs','qa-batch-dest'].includes(target.id))updateBatchPage();
}
function handleExtractionChange({target}){
 const {id,name,checked}=target;
 if(id==='oa-deep-extract'){
  const box=$('oa-recycle-after');
  box.dataset.wanted=box.dataset[checked?'deepWanted':'ordinaryWanted'];
  deepExtract.sync();syncRecycleOption();
 }
 if(id==='oa-recycle-after'){
  target.dataset.wanted=String(checked);
  target.dataset[$('oa-deep-extract').checked?'deepWanted':'ordinaryWanted']=target.dataset.wanted;
  syncRecycleOption(true);
 }
 if(id==='qa-smart')updateExtractPreview();
 if(name==='extract-scope'||id==='qa-conflict')syncRecycleOption(id==='qa-conflict');
 if(['oa-retry-conflict','oa-retry-recycle'].includes(id))updateExtractionRetry();
 if(id==='qa-batch-conflict')batchDraft.overwrite=target.value;
 if(['qa-batch-op','qa-batch-conflict'].includes(id))updateBatchPage();
}
function handleExtractionShortcut(event){
 if(event.defaultPrevented||isCompositionKey(event)||event.repeat||event.key!=='Enter'||!event.ctrlKey||event.altKey||event.shiftKey)return;
 if(event.target.closest('.oa-password-choices'))return;
 const button=event.target.closest('.oa-extraction-form')?.querySelector('.qa-dialog-footer .primary');
 if(!button||button.disabled||!button.getClientRects().length)return;
 event.preventDefault();button.click();
}
root.addEventListener('input',handleExtractionInput);
root.addEventListener('change',handleExtractionChange);
root.addEventListener('keydown',handleExtractionShortcut);
$('oa-toolbar').addEventListener('keydown',event=>{
 if(event.defaultPrevented||isCompositionKey(event)||event.key!=='ArrowDown'||event.ctrlKey||event.metaKey||event.shiftKey)return;
 const button=event.target.closest('button');
 if(!button||button.disabled)return;
 if(button.dataset.action==='extract-menu'||(button.dataset.action==='quick-extract'&&event.altKey)){
  event.preventDefault();
  showExtractMenu();
 }
});
root.addEventListener('keydown',event=>{
 if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='c'&&taskDetailSelection($('oa-task-content')))event.stopPropagation();
});
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
window.__TAURI__.event.listen('close-requested',()=>{if(!tabBusy()){closeArchiveTabs('exit');return;}if($('oa-exit-confirm'))return;clearToast();s.returnPage=overlay.dataset.page;overlay.dataset.page='close-confirm';overlay.hidden=false;root.querySelector('.qa-shell').inert=true;if($('oa-task-panel'))$('oa-task-panel').inert=true;overlay.querySelectorAll('.qa-dialog').forEach(e=>e.inert=true);const box=document.createElement('section');box.id='oa-exit-confirm';box.className='qa-dialog';box.style.position='absolute';box.style.zIndex='40';box.innerHTML='<h2>任务正在进行</h2><p>取消当前任务并关闭？已完成的输出会保留。</p><div class="qa-dialog-footer">'+btn('继续任务','keep-running')+btn('取消任务并退出','cancel-exit','danger')+'</div>';overlay.append(box);box.querySelector('button').focus();}).catch(error);

function recycleBlockReason(path,overwrite,{partial=false,deep=false,nested=false}={}){
 if(nested)return '有内层压缩包打开，请先保存并关闭内层标签。';
 if(/\.(?:\d{3,}|[zr]\d{2,})$|\.part\d+\.rar$/i.test(path))return '分卷压缩包无法自动回收。';
 if(!deep&&!/\.(zip|zipx|7z|tar|gz|gzip|bz2|bzip2|xz|tgz|tbz|tbz2|txz)$/i.test(path))return '此格式在普通解压时无法自动回收。';
 if(partial)return '请选择「整个压缩包」以启用回收。';
 if(overwrite==='skip')return '请将同名文件处理改为「跳过已有文件」以外的选项。';
 return '';
}
function canRecycleArchive(path,overwrite){return !recycleBlockReason(path,overwrite);}
function syncRecycleOption(reuseLayout=false){
 const box=$('oa-recycle-after');
 if(!box)return;
 const selected=extractSelection(),partial=selected.length>0,deep=$('oa-deep-extract').checked,overwrite=$('qa-conflict').value;
 syncExtractPassword(selected,deep);
 const nested=!!s.nestedId||archiveTabs.some(tab=>archiveKey(tab.parentArchive||'')===archiveKey(s.archive));
 const reason=recycleBlockReason(s.archive,overwrite,{partial,deep,nested});
 box.disabled=!!reason;
 box.checked=!box.disabled&&box.dataset.wanted==='true';
 const submit=overlay.querySelector('[data-action=startextract]');
 submit.lastChild.textContent=box.checked?(deep?'深度解压并回收':'解压并回收'):deep?'开始深度解压':'开始解压';
 const disposition=$('oa-extract-source-status');
 if(disposition){
  disposition.textContent=reason?`保留原压缩包。${reason}`:sourceDisposition(box.checked,{deep});
  disposition.dataset.recycle=String(box.checked);
 }
 updateExtractConflict('qa-conflict',deep);
 updateExtractPreview(reuseLayout);
}
function replaceSiblings(current,next){
 while(current.previousSibling)current.previousSibling.remove();
 while(current.nextSibling)current.nextSibling.remove();
 while(next.previousSibling)current.before(next.parentNode.firstChild);
 while(next.nextSibling)current.after(next.parentNode.lastChild);
}
function refreshJobMarkup(host,html){
 const current=host.querySelector('[data-job-progress]');
 if(current){
  const template=document.createElement('template');
  template.innerHTML=html;
  const next=template.content.querySelector('[data-job-progress]');
  if(next?.dataset.jobProgress===current.dataset.jobProgress){
   current.setAttribute('aria-label',next.getAttribute('aria-label'));
   const amount=next.getAttribute('aria-valuenow');
   QingProgress.render(current,amount===null?null:Number(amount),next.hasAttribute('data-progress-activity'));
   const currentCard=current.parentNode,nextCard=next.parentNode;
   if(currentCard.parentNode===host)replaceSiblings(currentCard,nextCard);
   replaceSiblings(current,next);
   return;
  }
 }
 host.innerHTML=html;
}
function renderHistoryJob(job){
 if(s.page!=='history')return;
 const card=main.querySelector('[data-job-card="'+job.id+'"]');
 if(!card){
  const list=$('oa-history-list');
  if(list)list.insertAdjacentHTML('afterbegin',jobCard(job));
  else render();
  return;
 }
 const active=document.activeElement;
 const restore=active!==card&&card.contains(active);
 const expanded=new Set([...card.querySelectorAll('details[open] > summary')].map(summary=>summary.textContent));
 const template=document.createElement('template');
 template.innerHTML=jobCard(job);
 const next=template.content.firstElementChild;
 card.dataset.attention=next.dataset.attention;
 const current=card.querySelector('[data-job-progress]');
 const nextBar=next.querySelector('[data-job-progress]');
 if(current&&nextBar){
  current.setAttribute('aria-label',nextBar.getAttribute('aria-label'));
  const amount=nextBar.getAttribute('aria-valuenow');
  QingProgress.render(current,amount===null?null:Number(amount),nextBar.hasAttribute('data-progress-activity'));
  replaceSiblings(current,nextBar);
 }else{
  card.replaceChildren(...next.childNodes);
 }
 for(const summary of card.querySelectorAll('details > summary'))if(expanded.has(summary.textContent))summary.parentElement.open=true;
 if(!overlay.hidden||!(restore||active===card))return;
 if(!card.getClientRects().length){
  $('oa-history-attention')?.focus({preventScroll:true});
 }else if(restore){
  const entries=Object.entries(active.dataset);
  const nextControl=[...card.querySelectorAll('button:enabled, summary')].find(control=>control.tagName===active.tagName
   &&(entries.length?entries.every(([key,value])=>control.dataset[key]===value):control.textContent===active.textContent)
   &&control.getClientRects().length);
  if(!nextControl)card.tabIndex=-1;
  (nextControl||card).focus({preventScroll:true});
 }
}
function refreshJobContent(html){const content=$('oa-dialog-content');if(!content)return false;const focused=document.activeElement,action=content.contains(focused)?focused.dataset.action:null;refreshJobMarkup(content,html);if(action){const target=Array.from(content.querySelectorAll('[data-action]')).find(b=>b.dataset.action===action&&!b.disabled)||content.querySelector('[data-action="reveal-output"]')||overlay.querySelector('.qa-dialog-footer [data-action="close"]');if(target){if(target.dataset.action==='close')target.disabled=!!s.active;if(!target.disabled)target.focus({preventScroll:true});}}return true;}
function renderProgress(){renderInlineTask();if(overlay.dataset.page==='close-confirm')return;if(s.view!=='jobs')return;if(!refreshJobContent(jobs()))render();overlay.querySelectorAll('.qa-dialog-footer [data-action=close]').forEach(b=>b.disabled=!!s.active);$('qa-status-text').textContent=s.active?'正在处理文件…':'就绪';updateToolbar();}
const toolCatalog=[
 ['压缩与解压',[
  ['deep-batch','批量深度解压','自动识别并展开多层压缩包','folder-output'],
  ['batch','批量处理','批量检查完整性或解压','check'],
  ['profiles','压缩预设','保存并复用常用压缩参数','settings'],
  ['repair-tool','修复 ZIP 压缩包','从损坏的 ZIP 或 ZIPX 恢复可读文件','wrench']
 ]],
 ['文件工具',[
  ['file-split','分割文件','将大文件分成多个独立小文件','split',true],
  ['file-combine','合并文件','按卷序还原分割的文件','merge',true],
  ['benchmark','引擎基准测试','测量 LZMA 压缩和解压性能','gauge',true],
  ['capabilities','格式支持','查看可读取和创建的压缩格式','file',true]
 ]],
 ['密码与系统',[
  ['password-book','解压密码本','添加和管理解压密码','book-lock'],
  ['recover-tool','密码恢复','用候选密码或字符集进行有限尝试','key'],
  ['associations','文件关联设置','选择注册格式并设置默认应用','link'],
  ['shell-settings','资源管理器右键设置','管理文件和文件夹的右键操作','mouse']
 ]]
];
function toolbox(){
 const query=s.toolQuery||'';
 const terms=toolSearchTerms(query);
 let found=0;
 const sections=toolCatalog.map(([group,items],index)=>{
  const keys=items.map(([op,title,desc])=>op+' '+title+' '+desc);
  const visible=keys.map(key=>toolMatches(terms,key));
  found+=visible.filter(Boolean).length;
  return `
  <section class="oa-toolgroup" aria-labelledby="oa-toolgroup-${index}" ${visible.some(Boolean)?'':'hidden'}>
   <h2 id="oa-toolgroup-${index}">${group}</h2>
   <div class="oa-toolgrid">${items.map(([op,title,desc,ico,frame],i)=>
    `<button type="button" class="oa-toolcard" aria-labelledby="oa-tool-${op}" aria-describedby="oa-tool-desc-${op}" data-tool-search="${esc(keys[i])}" ${visible[i]?'':'hidden'} ${frame?`data-launch-tool="${op}" data-tool-title="${title}"`:`data-action="${op}"`}>
     <span class="oa-tool-symbol">${icon(ico)}</span>
     <strong id="oa-tool-${op}">${title}</strong>
     <small id="oa-tool-desc-${op}">${desc}</small>
    </button>`
   ).join('')}</div>
  </section>`;
 }).join('');
 return `<div class="qa-heading oa-tool-heading"><h1>工具箱</h1>
  <label class="oa-tool-search">${icon('search')}<input id="oa-tool-search" type="search" aria-label="搜索工具" placeholder="搜索工具" value="${esc(query)}" autocomplete="off"></label></div>
  <div id="oa-tool-count" role="status">${terms.length&&found?`找到 ${found} 个工具`:''}</div>
  <div id="oa-tool-empty" ${found?'hidden':''}><p>没有匹配的工具</p>${btn('显示全部工具','clear-tool-search')}</div>${sections}`;
}
function toolSearchTerms(query){return query.trim().toLowerCase().split(/\s+/).filter(Boolean);}
function toolMatches(terms,text){const key=text.toLowerCase();return terms.every(term=>key.includes(term));}
function filterTools(input){
 s.toolQuery=input.value;
 const terms=toolSearchTerms(input.value);
 let found=0;
 for(const group of main.querySelectorAll('.oa-toolgroup')){
  let matches=0;
  for(const card of group.querySelectorAll('[data-tool-search]')){
   card.hidden=!toolMatches(terms,card.dataset.toolSearch);
   if(!card.hidden)matches++;
  }
  group.hidden=!matches;
  found+=matches;
 }
 $('oa-tool-count').textContent=terms.length&&found?`找到 ${found} 个工具`:'';
 $('oa-tool-empty').hidden=!!found;
}
function toolPage(){
 const {title,origin,frameUrl}=toolContext;
 const backLabel=origin==='files'?(s.archive?'返回压缩包':'返回文件'):origin==='settings'?'返回设置':'返回工具箱';
 return pageHeading(title,backLabel,'tool-back')+`<iframe id="qa-tools" title="${esc(title)}" class="qa-tool-frame" src="${esc(frameUrl)}"></iframe>`;
}
let associationEntries=null,associationSelection=new Set(),associationBusy=false,associationError='';
main.addEventListener('input',event=>{
 if(event.target.id==='oa-tool-search'&&!event.isComposing)filterTools(event.target);
});
main.addEventListener('compositionend',event=>{
 if(event.target.id==='oa-tool-search')filterTools(event.target);
});
main.addEventListener('change',event=>{
 if(event.target.id!=='oa-history-attention')return;
 s.historyAttention=event.target.checked;
 $('oa-history-list').dataset.attentionOnly=String(s.historyAttention);
 const latest=$('oa-history-batch');
 if(latest)latest.hidden=s.historyAttention&&!s.batchResult?.attention&&!s.savedBatch&&!s.batchResumeProblem;
});
root.addEventListener('keydown',event=>{
 if(event.defaultPrevented||isCompositionKey(event)||s.page!=='tools'||!overlay.hidden||!$('oa-menu').hidden)return;
 const input=$('oa-tool-search');
 if((event.ctrlKey||event.metaKey)&&!event.altKey&&event.key.toLowerCase()==='f'){
  event.preventDefault();input.focus();input.select();
 }else if(event.target===input&&event.key==='Escape'&&input.value){
  event.preventDefault();input.value='';filterTools(input);
 }
});
function associationPage(){return `<div class="qa-heading"><div><h1>文件关联</h1><p>让 qing-zip 出现在这些文件的打开方式中。</p></div>${btn('返回设置','settings','quiet')}</div><section class="qa-setting-group"><p class="qa-sub">勾选并保存会注册可用的打开方式；当前默认应用由 Windows 管理，需要你在系统设置中确认。取消勾选仅移除本程序的格式注册，不替你更改其他应用或系统默认选择。</p><div class="qa-actions">${btn('在 Windows 中设为默认','default-apps')}${btn('刷新状态','associations-refresh','quiet')}</div></section><div id="oa-association-status" role="status">${esc(associationError|| (associationBusy?'正在读取或保存文件关联…':''))}</div>${associationEntries?`<div class="oa-association-table"><table><thead><tr><th>注册打开方式</th><th>本程序已注册</th><th>当前默认应用</th></tr></thead><tbody>${associationEntries.map(e=>`<tr><td><label class="qa-check"><input type="checkbox" data-association="${esc(e.extension)}" ${associationSelection.has(e.extension)?'checked':''} ${associationBusy?'disabled':''}>${esc(e.extension.toUpperCase())}</label></td><td>${e.registered?'已注册':'未注册'}</td><td title="${esc(e.current_progid||'尚未设置默认应用')}">${e.is_default?'qing-zip':'其他应用或未设置'}</td></tr>`).join('')}</tbody></table></div><div class="qa-actions oa-spaced"><button class="qa-button primary" data-action="associations-save" ${associationBusy?'disabled':''}>保存所选格式</button><span class="qa-sub">保存后仍可在 Windows 中确认默认应用。</span></div>`:!associationBusy?'<p class="qa-sub">无法读取文件关联，请刷新重试。</p>':''}`;}
async function loadAssociations(){if(associationBusy)return;associationBusy=true;associationError='';if(s.page==='associations')render();try{associationEntries=await invoke('association_status');associationSelection=new Set(associationEntries.filter(e=>e.registered).map(e=>e.extension));}catch(e){associationError=taskError(e);throw e;}finally{associationBusy=false;if(s.page==='associations')render();}}
async function saveAssociations(){if(associationBusy||!associationEntries)return;associationBusy=true;associationError='';const extensions=[...associationSelection];if(s.page==='associations')render();try{associationEntries=await invoke('set_file_associations',{extensions});associationSelection=new Set(associationEntries.filter(e=>e.registered).map(e=>e.extension));toast('已保存格式注册。需要更改默认打开方式时，请在 Windows 设置中确认。');}catch(e){associationError=taskError(e);throw e;}finally{associationBusy=false;if(s.page==='associations')render();}}
function historyPage(){
 const reports=s.batchResult||s.savedBatch||s.batchResumeProblem?batchResultsMarkup(true):'';
 const latest=reports?`<div id="oa-history-batch"${s.historyAttention&&!s.batchResult?.attention&&!s.savedBatch&&!s.batchResumeProblem?' hidden':''}>${reports}</div>`:'';
 const queue=shellQueueStatus()+latest;
 const filter=s.jobs.length?`<label class="qa-check oa-history-filter"><input id="oa-history-attention" type="checkbox" ${s.historyAttention?'checked':''}>只看需处理的任务</label>`:'';
 const header=`<div class="qa-heading oa-history-heading"><div><h1>任务</h1><p>本次会话的压缩、解压与检查记录。</p></div>${filter}</div>`;
 if(!s.jobs.length)return header+(queue||'<div class="qa-empty">还没有任务记录。</div>');
 return header+queue+`<div id="oa-history-list" data-attention-only="${!!s.historyAttention}">${s.jobs.slice().reverse().map(job=>jobCard(job)).join('')}<p class="qa-empty oa-no-attention">当前没有需要处理的任务。</p></div>`;
}
function hideMenu(){
 const menu=$('oa-menu');
 menu.hidden=true;
 root.querySelector('[aria-controls=oa-menu][aria-expanded=true]')?.setAttribute('aria-expanded','false');
}
function rememberMenuFocus(){const box=$('oa-menu');if(!box.contains(document.activeElement))menuReturnFocus=document.activeElement;}
function positionMenu(box,anchor,alignEnd=false){const r=anchor?.getBoundingClientRect?.()||$('oa-toolbar').getBoundingClientRect();menuAnchorRect={left:r.left,bottom:r.bottom};box.hidden=false;box.scrollTop=0;const bounds=box.getBoundingClientRect();box.style.left=Math.max(8,Math.min(alignEnd?r.right-bounds.width:r.left,innerWidth-bounds.width-12))+'px';box.style.top=Math.max(8,Math.min(r.bottom+5,innerHeight-bounds.height-12))+'px';}
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
 const viewport=archiveViewport();
 control.hidden=viewport.scrollWidth<=viewport.clientWidth+1;
 const text=viewport.scrollLeft>1?'← 回到左侧':'查看右侧列 →';
 if(control.textContent!==text)control.textContent=text;
}
main.addEventListener('scroll',updateColumnScroll,{capture:true,passive:true});
new ResizeObserver(updateColumnScroll).observe(main);
main.addEventListener('focusin',event=>{
 if(!event.target.matches(':focus-visible'))return;
 const column=event.target.closest('.qa-table:not(.oa-icons) thead th[data-column]');
 if(!column||column.matches('[data-pinned-name] th[data-column="name"]'))return;
 const table=column.closest('table'),viewport=archiveViewport();
 const pinned=table.querySelector('th[data-column="name"]');
 const left=(table.hasAttribute('data-pinned-name')?pinned:table.querySelector('th:first-child')).getBoundingClientRect().right;
 const right=viewport.getBoundingClientRect().left+viewport.clientWidth;
 const bounds=event.target.getBoundingClientRect();
 if(bounds.left<left)viewport.scrollLeft+=bounds.left-left-4;
 else if(bounds.right>right)viewport.scrollLeft+=bounds.right-right+4;
});
function viewControls(){return `<div class="oa-view-controls"><div class="oa-view-switch" role="group" aria-label="文件显示方式"><button data-action="details" aria-pressed="${s.listStyle!=='icons'}" title="详细信息">${icon('file')}<span>详情</span></button><button data-action="icons" aria-pressed="${s.listStyle==='icons'}" title="大图标和图片缩略图">${icon('grid')}<span>大图标</span></button></div><label class="oa-icon-control" ${s.listStyle==='icons'?'':'hidden'}><span>图标</span><input id="oa-icon-size" type="range" min="80" max="256" step="16" value="${s.iconSize}" aria-label="图标大小"><span id="oa-icon-size-label">${s.iconSize}</span></label><span class="oa-zoom-hint" ${s.listStyle==='icons'?'':'hidden'}>Ctrl + 滚轮调整</span><label class="oa-sort-control"><span>排序</span><select id="oa-sort" aria-label="排序依据">${opts(columns.filter(c=>c.visible).map(c=>[c.id,c.label]),s.sort)}</select><button type="button" data-action="reverse-sort" aria-label="${s.reverse?'改为升序':'改为降序'}" title="${s.reverse?'降序':'升序'}">${s.reverse?'↓':'↑'}</button></label>${s.listStyle==='icons'?'':`<div class="oa-column-controls">${btn('列设置','columns','quiet')}<button id="oa-columns-scroll" type="button" class="qa-button quiet" data-action="columns-scroll" aria-controls="oa-archive-scroll" hidden>查看右侧列 →</button></div>`}<button id="oa-archive-maintenance" type="button" class="qa-button" data-action="archive-maintenance" aria-haspopup="menu" aria-controls="oa-menu" aria-expanded="false">${icon('shield')}归档维护${icon('chevron')}</button></div>`;}
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
    const batch=/\.(zip|rar|7z)$/i.test(s.archive);
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
          paintMatchingThumbnails(name, {error:taskError(error).slice(0,240)});
        }
      });
    }).catch(error => {
      if (current()) tasks.forEach(({name})=>paintMatchingThumbnails(name, {error:taskError(error).slice(0,240)}));
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
  },{root:archiveViewport(),rootMargin:'100px 0px',threshold:0});
  thumbnailObserver=observer;
  if(observeExisting)root.querySelectorAll('[data-thumbnail]').forEach(el=>observer.observe(el));
}
window.addEventListener('resize',()=>{requestAnimationFrame(()=>{const strip=root.querySelector('.oa-tab-list'),active=strip?.querySelector('[aria-selected="true"]')?.closest('.oa-archive-tab');if(strip&&active){const r=strip.getBoundingClientRect(),a=active.getBoundingClientRect();if(a.left<r.left)strip.scrollLeft+=a.left-r.left;else if(a.right>r.right)strip.scrollLeft+=a.right-r.right;}hideMenu();});});
root.addEventListener('wheel',e=>{if(!e.ctrlKey||s.page!=='files'||!s.archive||!overlay.hidden||!e.target.closest('#qa-main'))return;e.preventDefault();e.stopPropagation();if(!e.deltaY)return;if(s.listStyle!=='icons'){s.listStyle='icons';render();}setIconSize(s.iconSize+(e.deltaY<0?16:-16));},{passive:false,capture:true});


const TAB_FIELDS=['nestedId','parentArchive','nestedMember','dirty','conflict','archive','entries','password','headerEncrypted','codepage','folder','flatView','search','searchScope','sort','reverse','selected','collapsed','anchor','cursor','focusFile','listStyle','iconSize','lastDest'];
function archiveKey(path){return String(path).replaceAll('/','\\').replace(/^\\\\\?\\/,'').replace(/^[a-z]:/, drive=>drive.toUpperCase());}
let recentContextPath='',recentContextRow=null;
function showRecentDialog(){dialog('最近的文件',s.recent.map((p,i)=>`<button class="qa-recent-row" data-recent="${i}">${esc(p)}</button>`).join('')||'<p>没有最近记录</p>',btn('清空记录','clear-recent')+btn('关闭','close'));overlay.dataset.page='recent';}
function updateRecent(paths){const inDialog=overlay.dataset.page==='recent';s.recent=paths;localStorage.setItem('qing-recent',JSON.stringify(s.recent));hideMenu();render();if(inDialog)showRecentDialog();}
function showRecentContext(row,event){rememberMenuFocus();const path=s.recent[Number(row.dataset.recent)];if(!path)return;recentContextPath=path;recentContextRow=row;const box=$('oa-menu');box.inert=false;box.innerHTML='<button role="menuitem" data-action="remove-recent">删除记录</button><button role="menuitem" data-action="clear-recent">清空记录</button>';positionMenu(box,{getBoundingClientRect:()=>({left:event.clientX,bottom:event.clientY})});box.querySelector('button').focus();}
root.addEventListener('keydown',handleMenuKeydown,true);
root.addEventListener('keydown',handleArchiveSearchKeydown);
root.addEventListener('keydown',handleFileKeydown);
function rememberRecent(path){if(archiveTabs.some(t=>t.nestedId&&archiveKey(t.archive)===archiveKey(path)))return;s.recent=[path,...s.recent.filter(p=>archiveKey(p)!==archiveKey(path))].slice(0,12);localStorage.setItem('qing-recent',JSON.stringify(s.recent));}
function tabBusy(){return !!s.active||s.activeBatchPlan?.operation==='extract'||!!homeIntake||openingArchive||openingImage||!!$('qa-tools')?.contentWindow?.obzBusy;}
function saveCurrentTab(){const tab=archiveTabs.find(t=>t.id===currentTabId);if(!tab)return;for(const key of TAB_FIELDS)tab[key]=s[key] instanceof Set?new Set(s[key]):s[key];if(renderedPage==='archive:'+tab.id){const viewport=archiveViewport();tab.scrollTop=viewport.scrollTop;tab.scrollLeft=viewport.scrollLeft;tab.treeScroll=root.querySelector('.oa-tree-section').scrollTop;}}
function applyTab(tab){for(const key of TAB_FIELDS)s[key]=tab[key] instanceof Set?new Set(tab[key]):tab[key];if(!s.selected)s.selected=new Set();if(!s.collapsed)s.collapsed=new Set();}
function emptyArchiveState(){Object.assign(s,{nestedId:null,parentArchive:null,nestedMember:null,dirty:false,conflict:false,archive:'',entries:[],password:'',headerEncrypted:false,codepage:null,folder:'',flatView:false,search:'',sort:'name',reverse:false,selected:new Set(),collapsed:new Set(),anchor:null,cursor:null,focusFile:null,lastDest:undefined,listStyle:pref.listStyle==='icons'?'icons':'details',iconSize:Math.max(80,Math.min(256,Number(pref.iconSize)||144))});currentTabId=null;}
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
  if(restoreScroll){const viewport=archiveViewport();viewport.scrollTop=tab?.scrollTop||0;viewport.scrollLeft=tab?.scrollLeft||0;root.querySelector('.oa-tree-section').scrollTop=tab?.treeScroll||0;}
  const button=root.querySelector(`[data-archive-tab="${id}"]`),row=rowButtons().find(b=>b.dataset.entry===s.cursor);
  const target=origin.tabs&&button?button:row||main;if(target===main)main.tabIndex=-1;target.focus({preventScroll:true});
  button?.scrollIntoView({block:'nearest',inline:'nearest'});
 });
}
function switchArchiveTab(id){const origin=archiveTabFocusOrigin();hideMenu();const tab=archiveTabs.find(t=>t.id===id);if(!tab)return;if(tabBusy()&&!s.backgroundExtraction){toast('任务正在进行，请先等待或取消后再切换标签。');return;}saveCurrentTab();resetPreviewState();currentTabId=id;applyTab(tab);close();s.page='files';s.view='archive';render();restoreArchiveTabFocus(tab,origin);}
function wipeTab(tab){tab.password='';tab.entries=[];tab.selected?.clear();tab.collapsed?.clear();}
let removingArchiveTabs=false,closingArchiveTabs=false,finishingArchiveTabs=false;
async function removeArchiveTabs(remove,origin=archiveTabFocusOrigin()){
 if(removingArchiveTabs)return false;hideMenu();if(tabBusy()){toast('任务正在进行，请先等待或取消后再关闭标签。');return;}saveCurrentTab();if(!remove.length)return true;removingArchiveTabs=true;
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
function clearShellQueue(){for(const request of pendingShellExtractions)clearShellSecret(request);clearShellSecret(shellActiveRequest);clearShellSecret(shellPasswordRequest);pendingShellExtractions.splice(0,pendingShellExtractions.length,...(shellActiveRequest?[shellActiveRequest]:[]));shellQueuePaused=false;shellQueueInterrupted=true;shellPasswordRequest=null;const input=$('qa-shell-password');if(input){input.value='';close();}if(s.page==='history')render();else renderProgress();}
function showShellPassword(){if(shellQueueRunning||tabBusy())throw Error('请先等待或取消当前任务。');const request=pendingShellExtractions[0];if(!request||!shellQueuePaused)throw Error('没有等待重试的解压任务。');shellPasswordRequest=request;dialog('输入密码重试',`<p class="qa-sub">${esc(filename(request.path))}</p><label class="qa-field">密码<input id="qa-shell-password" type="password" autocomplete="off" data-password-book></label>`,btn('取消','close')+btn('重试','shell-password-submit','primary'));}
async function retryShellExtraction(withPassword=false){if(shellQueueRunning||tabBusy())throw Error('请先等待或取消当前任务。');const request=withPassword?shellPasswordRequest:pendingShellExtractions[0];if(!request||pendingShellExtractions[0]!==request)throw Error('该解压任务已移出队列。');if(withPassword){const input=$('qa-shell-password');if(!input||!input.value)throw Error('请输入密码。');request.password=input.value;input.value='';}request.state='pending';delete request.error;request.cancelRequested=false;shellPasswordRequest=null;shellQueuePaused=false;if(!overlay.hidden)close();await pumpShellExtractions();}
function enqueueShellExtractions(requests){if(!Array.isArray(requests))return;for(const request of requests){if(typeof request?.path!=='string'||typeof request?.dest!=='string'||typeof request?.smart!=='boolean')continue;if(!pendingShellExtractions.some(p=>p.smart===request.smart&&archiveKey(p.path)===archiveKey(request.path)))pendingShellExtractions.push({path:request.path,dest:request.dest,smart:request.smart,state:'pending'});}if(requests.length&&(tabBusy()||!overlay.hidden))toast('解压任务已加入队列，当前操作结束后继续。');if(s.page==='history')render();else renderProgress();}
async function pumpShellExtractions(){if(shellQueueRunning||shellQueuePaused||tabBusy()||!overlay.hidden)return;shellQueueRunning=true;shellQueueInterrupted=false;let allSucceeded=true,wantsClose=false,completedItem=false;const canAdvance=()=>overlay.hidden||(completedItem&&overlay.dataset.page==='jobs'&&!$('oa-exit-confirm'));try{while(pendingShellExtractions.length&&!shellQueuePaused&&!tabBusy()&&canAdvance()){const request=pendingShellExtractions[0],tab=archiveKey(s.archive)===archiveKey(request.path)?s:archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(request.path)),overwrite=s.overwrite,recycle=!!s.recycleAfter&&canRecycleArchive(request.path,overwrite),openAfter=!!s.openAfter;wantsClose=!!s.closeAfter;shellActiveRequest=request;request.state='running';let extracted=false;try{const result=await run({operation:'extract',path:request.path,dest:request.dest,smart:request.smart,options:{selected:[],overwrite,password:(request.password!==undefined?request.password:tab?.password)||null,codepage:tab?.codepage||null}},(request.smart?'智能解压 · ':'解压到此处 · ')+filename(request.path),{recycleAfter:recycle});extracted=true;completedItem=true;request.state='complete';clearShellSecret(request);const index=pendingShellExtractions.indexOf(request);if(index>=0)pendingShellExtractions.splice(index,1);if(!await finishExtraction(request.path,request.dest,result,{recycle,openAfter})){allSucceeded=false;shellQueuePaused=!!pendingShellExtractions.length;break;}}catch(e){allSucceeded=false;if(!extracted){request.state=request.cancelRequested?'cancelled':'failed';request.error=taskError(e);shellQueuePaused=true;}else shellQueuePaused=!!pendingShellExtractions.length;error(e);break;}finally{shellActiveRequest=null;}}const drained=await drainNativeArchives();if(wantsClose&&allSucceeded&&!shellQueueInterrupted&&!shellQueuePaused&&drained&&!pendingShellExtractions.length&&!pendingFolders.length&&!pendingFileGroups.length&&!pendingArchivePaths.length&&!tabBusy()&&!openQueueRunning&&canAdvance())await invoke('window_action',{action:'close'});}finally{shellActiveRequest=null;shellQueueRunning=false;if(s.page==='history')render();else renderProgress();}}
function showCompressionFiles(paths) {
  compress(undefined,paths);
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
async function pumpArchiveQueue(){if(shellQueueRunning||openQueueRunning)return;if(pendingFileGroups.length&&!tabBusy()&&overlay.hidden){showCompressionFiles(pendingFileGroups.shift());return;}if(pendingFolders.length&&!tabBusy()&&overlay.hidden){const folder=pendingFolders.shift();showCompressionFiles([folder]);return;}if(pendingShellExtractions.length&&!shellQueuePaused&&!tabBusy()&&overlay.hidden){await pumpShellExtractions();return;}if(!pendingArchivePaths.length||tabBusy()||!overlay.hidden)return;openQueueRunning=true;try{while(pendingArchivePaths.length&&!tabBusy()&&overlay.hidden){const path=pendingArchivePaths.shift();renderArchiveTabs();try{await openArchive(path,'',{queued:true});}catch(e){showOpenFailure(path,e);if(/密码/.test(String(e)))$('qa-open-password')?.focus();break;}}}finally{openQueueRunning=false;renderArchiveTabs();}}
let nativeDrainAgain=false;
async function drainNativeArchives(){if(drainingNativeQueue){nativeDrainAgain=true;return false;}drainingNativeQueue=true;try{do{nativeDrainAgain=false;const requests=await invoke('take_pending_shell_extractions');enqueueShellExtractions(requests);const groups=await invoke('take_pending_compression_files');for(const group of groups){if(group.Err){toast(group.Err);continue;}if(group.Ok?.length)pendingFileGroups.push(group.Ok);}const folders=await invoke('take_pending_folders');for(const p of folders)if(!pendingFolders.includes(p))pendingFolders.push(p);if(folders.length&&(tabBusy()||!overlay.hidden))toast('文件夹已加入压缩队列，当前操作结束后继续。');const paths=await invoke('take_pending_archives');enqueueArchives(paths);}while(nativeDrainAgain);return true;}catch(e){error(e);return false;}finally{drainingNativeQueue=false;schedulePendingQueue();}}

let pendingNestedOpen=null,pendingNestedClose=null;
function selectedNestedMember(){if(!s.archive||tabBusy()||s.selected.size!==1)return null;const name=[...s.selected][0];return visibleEntries().some(e=>e.name===name&&!e.is_dir)?name:null;}
function isNestedArchive(name){return /\.(?:zip|zipx|7z|rar|tar|tgz|gz|bz2|xz|zst|cab|iso|wim|001)$/i.test(name);}
function nestedBanner(){return s.nestedId?`<div class="qa-path oa-nested-bar"><span>${s.conflict?'外层已改变，工作副本已保留':s.dirty?'更改尚未写回外层压缩包':'内层压缩包'} · ${esc(filename(s.parentArchive||''))}</span><span style="flex:1"></span>${btn('保存至外层','save-nested','quiet')}${btn('另存为…','save-as','quiet')}</div>`:'';}
async function syncNestedSessions(){if(!archiveTabs.some(t=>t.nestedId))return;const sessions=await invoke('nested_sessions');for(const tab of archiveTabs){const state=sessions.find(v=>v.id===tab.nestedId);if(state){tab.dirty=state.dirty;tab.conflict=state.conflict;if(tab.id===currentTabId){s.dirty=state.dirty;s.conflict=state.conflict;}}}renderArchiveTabs();}
async function openNestedEntry(member){if(tabBusy())throw Error('请先等待当前任务完成。');const existing=archiveTabs.find(t=>archiveKey(t.parentArchive||'')===archiveKey(s.archive)&&t.nestedMember===member);if(existing){switchArchiveTab(existing.id);return;}const context={parent:s.archive,member,options:{password:s.password||null,codepage:s.codepage||null},session:null,stage:'outer'};if(visibleEntries().find(e=>e.name===member)?.encrypted&&!context.options.password){showNestedPassword(context,'请输入外层压缩包密码。');return;}await continueNestedOpen(context,'');}
async function continueNestedOpen(context,password){pendingNestedOpen=null;try{if(!context.session){if(password)context.options.password=password;context.session=await run({operation:'nested-open',path:context.parent,name:context.member,options:context.options},'打开内层压缩包');const parentTab=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(context.parent));if(parentTab&&context.options.password){parentTab.password=context.options.password;if(parentTab.id===currentTabId)s.password=parentTab.password;}context.stage='inner';password='';}await openArchive(context.session.path,password,{nested:context.session});}catch(e){const message=archiveOpenMessage(e,!!password);if(/password|密码/i.test(message)){showNestedPassword(context,message);}else{if(context.session)await invoke('close_nested',{ids:[context.session.id],discard:false});throw e;}}}
function showNestedPassword(context,message){pendingNestedOpen=context;dialog(context.stage==='outer'?'输入外层压缩包密码':'输入内层压缩包密码',`<p>${esc(filename(context.stage==='outer'?context.parent:context.member))}</p><label class="qa-field">密码<input id="oa-nested-password" type="password" autocomplete="off" data-password-book></label>`,btn('取消','close')+btn('打开','nested-open-submit','primary'));$('qa-formerror').textContent=message;$('oa-nested-password').focus();}
async function retryNestedOpen(){const context=pendingNestedOpen;if(!context)return;const password=$('oa-nested-password').value;if(!password)throw Error('请输入密码。');await continueNestedOpen(context,password);}
function nestedSaveDisplayPath(tab){
 const members=[];
 while(tab.nestedId){
  members.unshift(tab.nestedMember);
  const parent=archiveTabs.find(item=>archiveKey(item.archive)===archiveKey(tab.parentArchive));
  if(!parent)return [filename(tab.parentArchive),...members].join(' › ');
  tab=parent;
 }
 return [tab.archive,...members].join(' › ');
}
async function saveNested(tab=archiveTabs.find(t=>t.id===currentTabId)){
 if(!tab?.nestedId)throw Error('当前标签不是内层压缩包。');
 try{
  await run({operation:'nested-save',path:tab.archive},'保存至外层压缩包',{displayPath:nestedSaveDisplayPath(tab)});
 }catch(error){
  // A descendant can already be saved into a parent copy when the outer write fails.
  await syncNestedSessions().catch(()=>{});
  throw error;
 }
 await syncNestedSessions();
 let parent=tab.parentArchive;
 while(parent){
  const t=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(parent));if(!t)break;
  const r=await run({operation:'list',path:t.archive,options:{password:t.password||null,codepage:t.codepage||null}},'刷新外层压缩包',{silent:true});
  t.entries=r.entries;if(t.id===currentTabId)s.entries=t.entries;parent=t.parentArchive;
 }
 navigate('archive');toast('已保存至外层压缩包。');
}
function nestedCloseSelection(mode,id){if(mode==='all'||mode==='exit')return archiveTabs.slice();const remove=mode==='others'?archiveTabs.filter(t=>t.id!==id):archiveTabs.filter(t=>t.id===id);if(mode==='others'){let kept=archiveTabs.find(t=>t.id===id);while(kept?.parentArchive){kept=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(kept.parentArchive));const index=remove.indexOf(kept);if(index>=0)remove.splice(index,1);}}else{for(let i=0;i<remove.length;i++)for(const t of archiveTabs){if(!remove.includes(t)&&archiveKey(t.parentArchive||'')===archiveKey(remove[i].archive))remove.push(t);}}return remove;}
function showNestedClose(){const pending=pendingNestedClose;if(!pending)return;dialog('保存内层压缩包的更改？',`<p>以下压缩包的更改尚未写回外层：</p><p>${pending.tabs.filter(t=>t.dirty).map(t=>esc(filename(t.archive))).join('<br>')}</p>`,btn('取消','close')+btn('放弃更改','nested-close-discard','danger')+btn('保存并关闭','nested-close-save','primary'));if(pending.focusOrigin)pending.focusOrigin.dialog=overlay.firstElementChild;}
function closeArchiveTabs(mode,id=currentTabId){const focusOrigin=archiveTabFocusOrigin();(async()=>{if(closingArchiveTabs||finishingArchiveTabs||removingArchiveTabs)return;closingArchiveTabs=true;try{if(tabBusy()){toast('任务正在进行，请先等待或取消后再关闭标签。');return;}saveCurrentTab();await syncNestedSessions();const tabs=nestedCloseSelection(mode,id);const edits=await invoke('external_edits');if(edits.some(e=>mode==='exit'||tabs.some(t=>archiveKey(t.archive)===archiveKey(e.parent)))){await showExternalEdits();return;}pendingNestedClose={tabs,exit:mode==='exit',focusOrigin};if(tabs.some(t=>t.dirty)){showNestedClose();return;}await finishNestedClose('close');}finally{closingArchiveTabs=false;}})().catch(error);}
async function finishNestedClose(mode){const save=mode==='save';const pending=pendingNestedClose;if(!pending||tabBusy()||finishingArchiveTabs)return;finishingArchiveTabs=true;try{if(save){for(const tab of pending.tabs){await syncNestedSessions();if(tab.dirty)await saveNested(tab);}}const ids=pending.tabs.filter(t=>t.nestedId).map(t=>t.nestedId);if(ids.length)await invoke('close_nested',{ids,discard:mode==='discard'});if(!await removeArchiveTabs(pending.tabs,pending.focusOrigin))return;if(pendingNestedClose===pending)pendingNestedClose=null;if(pending.exit)await invoke('window_action',{action:'close'});}catch(e){await syncNestedSessions().catch(()=>{});showNestedClose();throw e;}finally{finishingArchiveTabs=false;}}

root.querySelectorAll('[data-lucide]').forEach(i=>{i.outerHTML=icon(i.dataset.lucide)});invoke('app_version').then(v=>{appVersion=v;if(s.page==='settings')render();}).catch(error);appearance();render();invoke('file_compression_menu_enabled').then(v=>{fileMenuEnabled=v;if(s.page==='settings')render();}).catch(error);invoke('folder_menu_enabled').then(v=>{folderMenuEnabled=v;if(s.page==='settings')render();}).catch(error);window.__TAURI__.event.listen('open-archives',()=>drainNativeArchives()).then(drainNativeArchives).catch(error);
function showHomeSmartDrop(dragging) {
  const target=$('oa-home-smart-drop');
  if(!target)return;
  const active=dragging&&homeIntakeAvailable();
  target.classList.toggle('oa-drop-active',active);
  target.querySelector('[data-drop-label]').textContent=active?'松开以识别文件':'拖入或选择文件，自动识别压缩包';
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
 }catch(reason){if(target.isConnected)target.textContent=/cancelled/i.test(String(reason))?'已取消属性读取':'格式属性读取失败：'+taskError(reason);}
 finally{if(target.isConnected)overlay.querySelector('[data-action="canceljob"]')?.remove();}
}

// Explicit editing leaves image/text double-click previews unchanged.
let externalContext=null,externalCurrent=null,externalSessions=[];
async function beginExternalEdit(){
 if((await invoke('external_edits')).length){await showExternalEdits();return;}
 const member=selectedNestedMember();if(!member)throw Error('请选中一个文件。');
 externalContext={parent:s.archive,member,options:{password:s.password||null,codepage:s.codepage||null}};
 dialog('外部编辑',`<p>${esc(member)}</p><label class="qa-field">编辑器（留空使用记事本）<input id="oa-editor-path" readonly value="${esc(localStorage.getItem('qing-editor-path')||'')}"></label>${btn('选择编辑器…','external-editor-pick')}${btn('使用记事本','external-editor-default','quiet')}<label class="qa-field">压缩包密码（如有）<input id="oa-editor-password" type="password" autocomplete="off"></label>`,btn('取消','close')+btn('开始编辑','external-launch','primary'));
}
async function launchExternalEdit(){
 const context=externalContext;if(!context)return;const editor=$('oa-editor-path').value;
 context.options.password=$('oa-editor-password').value||context.options.password;localStorage.setItem('qing-editor-path',editor);
 const opened=await run({operation:'external-edit',path:context.parent,name:context.member,label:editor,options:context.options},'准备外部编辑');
 const tab=archiveTabs.find(t=>archiveKey(t.archive)===archiveKey(context.parent));if(tab&&context.options.password){tab.password=context.options.password;if(tab.id===currentTabId)s.password=tab.password;}
 externalContext=null;await showExternalEdits(opened?.id);
}
async function showExternalEdits(id=externalCurrent?.id){
 externalSessions=await invoke('external_edits');
 showExternalSession(id);
}
function showExternalSession(id){
 externalCurrent=externalSessions.find(edit=>edit.id===id)||externalSessions[0]||null;
 if(!externalCurrent){dialog('外部编辑','<p>没有待处理的编辑会话。</p>',btn('保留的编辑副本…','external-copies')+btn('关闭','close'));return;}
 const e=externalCurrent;const state={running:'编辑器仍在运行',handoff:'编辑器可能在其他窗口中继续编辑',finished:'启动进程已退出'}[e.state];
 const chooser=externalSessions.length>1?`<label class="qa-field">编辑会话<select id="oa-external-session">${externalSessions.map(edit=>`<option value="${edit.id}" ${edit.id===e.id?'selected':''}>${esc(filename(edit.parent))} / ${esc(edit.member)}</option>`).join('')}</select></label>`:`<p>${esc(e.member)}</p>`;
 dialog('外部编辑会话',`${chooser}<p>${state} · ${e.changed===true?'已保存的文件有更改':e.changed===false?'已保存的文件未改变':'文件正在使用中'}</p><p class="qa-sub">请先在编辑器中保存。写回仅包含已保存的内容，并结束本次编辑；之后的修改不会自动写回。</p><p class="qa-sub">编辑器可能把文件交给其他窗口，结束会话后工作副本会保留，可从“保留的编辑副本”找回。</p>${e.error?`<p class="qa-sub">${esc(e.error)}</p>`:''}`,btn('返回','close')+btn('检查更改','external-sessions')+btn('另存为…','external-copy')+btn('结束，不写回','external-discard-confirm')+btn('确认写回…','external-save-confirm','primary'));
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
function currentRetainedEdit(){return retainedEditRecords[Number($('oa-retained-edit').value)];}
function updateRetainedActions(){
 const record=currentRetainedEdit(),source=$('oa-retained-source');
 if(source)source.textContent=record?.nested?record.label:'';
 const button=overlay.querySelector('[data-action="external-retained-open"]');
 if(button)button.disabled=!!record?.nested;
}
async function saveRetainedEdit(){
 const record=currentRetainedEdit();
 const dest=await invoke('pick_copy_save',{path:record.path});
 if(dest){
  const saved=await invoke('save_retained_edit',{selection:record.selection,dest});
  toast('已另存为 '+saved.path+(saved.cleanup_warning?'。恢复登记暂未清除。':''));
  if(record.nested&&!saved.cleanup_warning)await showRetainedEdits().catch(()=>{});
 }
}
async function showRetainedEdits(){
 retainedEditRecords=await invoke('retained_edits');
 if(!retainedEditRecords.length){dialog('保留的编辑副本','<p>没有保留的编辑副本。</p>',btn('关闭','close'));return;}
 const notice=retainedEditRecords.some(record=>record.nested)
  ?'<p>内层归档工作副本暂存在系统临时目录，请在系统清理前另存。另存不会改动外层压缩包。</p>'
  :'<p>这里的副本保存在独立目录，不随预览缓存清理。</p>';
 dialog('保留的编辑副本',`${notice}<label class="qa-field">文件<select id="oa-retained-edit">${retainedEditRecords.map((r,i)=>`<option value="${i}">${esc(r.label)}</option>`).join('')}</select></label><p class="qa-sub" id="oa-retained-source"></p>`,btn('关闭','close')+btn('打开所在位置','external-retained-open')+btn('另存为…','external-retained-save','primary'));
 updateRetainedActions();
}

root.addEventListener('change',e=>{
 if(e.target.id==='oa-external-session')showExternalSession(Number(e.target.value));
 if(e.target.id==='oa-retained-edit')updateRetainedActions();
 if(e.target.id==='qa-update-mode')syncAddMode();
 if(e.target.id==='oa-add-password-mode')syncAddPassword();
});

})();
