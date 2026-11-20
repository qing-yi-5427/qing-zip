(()=>{'use strict';
const defaults={depth:8,archives:256,files:100000,bytes:20*1073741824,reserve_bytes:1073741824,recovery_directory:null};
window.QingDeepExtract=({root,s,pref,esc,btn,dialog,$,run,invoke,appearance,toast,archiveParent,archiveStem,estimatedArchiveFolder,onRecycled})=>{
 s.deepExtract=!!pref.deepExtract;s.deepRecycleAfter=!!pref.deepRecycleAfter;s.deepLimits={...defaults,...pref.deepLimits};
 const completionOptions=new Map();
 const button=(label,action,extra='')=>`<button type="button" class="qa-button${action==='start-batch'?' primary':''}" data-deep-action="${action}" ${extra}>${label}</button>`;
 function limitsMarkup(){const v=s.deepLimits;return `<details id="oa-deep-limits" class="oa-extract-advanced"><summary>深度解压限制与恢复位置</summary><p class="qa-sub">层数从原包计为第 1 层。累计解码量包含中间文件和重试；大文件可提高限制。已知大小先按选定内容估算，解压后核对；GZip、BZip2、XZ 解码另有限量写入。</p><div class="qa-fieldrow"><label class="qa-field">最大层数<input id="oa-deep-depth" type="number" min="1" max="64" value="${esc(v.depth)}"></label><label class="qa-field">最多压缩包<input id="oa-deep-archives" type="number" min="1" max="10000" value="${esc(v.archives)}"></label></div><div class="qa-fieldrow"><label class="qa-field">累计文件数量<input id="oa-deep-files" type="number" min="1" value="${esc(v.files)}"></label><label class="qa-field">累计解码量（GiB）<input id="oa-deep-bytes" type="number" min="0.001" step="0.1" value="${esc(v.bytes/1073741824)}"></label></div><div class="qa-fieldrow"><label class="qa-field">保留磁盘空间（GiB）<input id="oa-deep-reserve" type="number" min="0" step="0.1" value="${esc(v.reserve_bytes/1073741824)}"></label></div><label class="qa-field">恢复位置（留空使用本机应用数据文件夹）<div class="qa-inlinefield"><input id="oa-deep-recovery" value="${esc(v.recovery_directory||'')}">${button('选择…','pick-recovery')}</div></label><p class="qa-sub">密码只保留在当前会话内。中断后，已校验结果保留在恢复目录；部分保存后取消会保留已保存文件。</p></details>`;}
 function switchMarkup(enabled=s.deepExtract){return `<label class="oa-switch-row"><span>深度解压<span class="qa-sub"> · 自动展开内层压缩包</span></span><input id="oa-deep-extract" class="oa-switch" role="switch" type="checkbox" ${enabled?'checked':''}></label><p id="oa-deep-note" class="qa-sub" ${enabled?'':'hidden'}>按内容识别无后缀、改名和带前缀的压缩包。文档与应用容器保持完整。</p>`;}
 function settings(remember=false){
  if(!$('oa-deep-depth'))return {...s.deepLimits};
  const number=(id,integer=false)=>{const value=Number($(id).value);if(!Number.isFinite(value)||value<0||(integer&&!Number.isInteger(value)))throw Error('请填写有效的深度解压限制。');return value;};
  const value={depth:number('oa-deep-depth',true),archives:number('oa-deep-archives',true),files:number('oa-deep-files',true),bytes:Math.round(number('oa-deep-bytes')*1073741824),reserve_bytes:Math.round(number('oa-deep-reserve')*1073741824),recovery_directory:$('oa-deep-recovery').value.trim()||null};
  if(!value.depth||value.depth>64||!value.archives||value.archives>10000||!value.files||!value.bytes)throw Error('层数需为 1–64，压缩包数量需为 1–10000，累计文件与解码量需大于零。');
  if(remember){s.deepLimits=value;appearance();}return value;
 }
 function sync(){const enabled=!!$('oa-deep-extract')?.checked;const note=$('oa-deep-note'),limits=$('oa-deep-limits');if(note)note.hidden=!enabled;if(limits)limits.hidden=!enabled;}
 function batch(paths=[]){
  if(s.active){toast('请先等待或取消当前任务。');return;}
  dialog('批量深度解压',`
   <div class="qa-actions oa-source-heading"><strong>压缩包</strong><span id="oa-deep-count" class="qa-sub"></span>${button('选择文件…','pick-inputs')}</div>
   <div id="oa-deep-list" class="oa-deep-list"></div>
   <details class="oa-extract-advanced"><summary>粘贴或输入多行路径</summary><textarea id="oa-deep-inputs" rows="3" aria-label="压缩包路径（每行一个）"></textarea></details>
   <label class="qa-field">保存到<div class="qa-inlinefield"><input id="oa-deep-destination" placeholder="各原包所在的文件夹">${button('浏览…','pick-dest')}</div></label>
   <p class="qa-sub oa-output-preview">预计输出 <output id="oa-deep-preview"></output></p><p class="qa-sub">同名目录已存在时会自动编号。</p>
   <label class="qa-field">密码（可选）<input id="oa-deep-password" type="password" autocomplete="off" placeholder="用于原包及内层压缩包"></label>
   <label class="oa-switch-row"><span>全部成功后将原包移入回收站</span><input id="oa-deep-recycle" class="oa-switch" role="switch" type="checkbox" ${s.deepRecycleAfter?'checked':''}></label>
   <details class="oa-extract-advanced"><summary>更多解压选项</summary>
    <label class="oa-switch-row"><span>智能解压</span><input id="oa-deep-smart" class="oa-switch" role="switch" type="checkbox" ${s.sameFolder?'checked':''}></label>
    <label class="qa-field">遇到同名文件<select id="oa-deep-conflict">${conflictChoices(s.overwrite)}</select></label>
    <label class="oa-switch-row"><span>完成后打开目标文件夹</span><input id="oa-deep-open-after" class="oa-switch" role="switch" type="checkbox" ${s.openAfter?'checked':''}></label>
    <label class="oa-switch-row"><span>完成后关闭窗口</span><input id="oa-deep-close-after" class="oa-switch" role="switch" type="checkbox" ${s.closeAfter?'checked':''}></label>
    <p class="qa-sub">分卷会合并为一个任务。Office、APK、JAR、EPUB 等容器保持完整。</p>
    ${limitsMarkup()}
   </details>
   <label class="qa-check"><input id="oa-deep-remember" type="checkbox" checked>记住解压选项</label>
  `,btn('取消','close')+button('开始深度解压','start-batch'),'oa-extraction-dialog');
  acceptDrop(paths);
 }
 const inputPaths=()=>[...new Set($('oa-deep-inputs').value.split(/\r?\n/).map(path=>path.trim()).filter(Boolean))];
 function batchPreview(){
  const dest=$('oa-deep-destination').value.trim(), paths=inputPaths();
  const output=$('oa-deep-preview');
  if(paths.length===1){
   const [source]=paths,base=dest||archiveParent(source);
   const folder=source===s.archive?estimatedArchiveFolder(source,s.entries,[],$('oa-deep-smart').checked,true):archiveStem(source);
   output.textContent=base?base.replace(/[\\/]+$/,'')+'\\'+folder:folder;
  }else{
   output.textContent=paths.length?`${paths.length} 个来源 · ${dest||'各原包所在的文件夹'} · 独立子文件夹`:'添加压缩包后显示';
  }
  output.title=output.textContent;
  root.querySelector('[data-deep-action=start-batch]').textContent=$('oa-deep-recycle').checked?'深度解压并回收':'开始深度解压';
 }
 function renderInputs(){
  const paths=$('oa-deep-inputs').value.split(/\r?\n/);
  $('oa-deep-count').textContent=inputPaths().length+' 个来源';
  $('oa-deep-list').innerHTML=paths.map((path,index)=>path.trim()?`<div class="oa-deep-source"><input value="${esc(path)}" title="${esc(path)}" aria-label="压缩包路径 ${index+1}" data-deep-input="${index}">${button('移除','remove-input',`data-index="${index}" aria-label="移除压缩包 ${index+1}"`)}</div>`:'').join('')||'<p class="qa-sub">拖入压缩包，或使用“选择文件”。</p>';
  batchPreview();
 }
 function updateInput(index,path){
  const field=$('oa-deep-inputs'),paths=field.value.split(/\r?\n/);
  if(path===null){
   paths.splice(index,1);
   field.value=paths.join('\n');
   renderInputs();
  }else{
   paths[index]=path;
   field.value=paths.join('\n');
   $('oa-deep-count').textContent=inputPaths().length+' 个来源';
   batchPreview();
  }
 }
 function acceptDrop(paths){
  const field=$('oa-deep-inputs');
  if(!field)return false;
  field.value=[...new Set([...inputPaths(),...paths])].join('\n');
  renderInputs();
  return true;
 }
 function conflictChoices(value){
  return [['error','停止并保留待处理项目'],['rename','保留两份，自动改名'],['overwrite','覆盖'],['skip','跳过，并保留恢复结果']].map(([id,label])=>`<option value="${id}" ${value===id?'selected':''}>${label}</option>`).join('');
 }
 function jobActions(job){const v=job.deep;if(!v)return '';return `${v.finished&&v.archives?`<p class="qa-sub oa-deep-counts" data-archive-count="${v.archives}" data-max-depth="${v.max_depth}">识别 ${v.archives} 个压缩包 · 最深 ${v.max_depth} 层</p>`:''}${v.retained_originals?.length?`<details class="qa-sub"><summary>保留的原包（${v.retained_originals.length}）</summary>${v.retained_originals.map(path=>`<p>${esc(path)}</p>`).join('')}</details>`:''}${v.outputs?.length?'<div class="oa-deep-outputs">'+v.outputs.map(path=>button(esc(path),'output',`data-path="${esc(path)}" title="打开输出位置"`)).join('')+'</div>':''}${v.retained_archives?.length?`<details class="qa-sub"><summary>保留的压缩包（${v.retained_archives.length}）</summary>${v.retained_archives.map(source=>`<p>${esc(source)}</p>`).join('')}</details>`:''}${v.retained_matching_files?.length?`<details class="qa-sub"><summary>保留的同名匹配文件（${v.retained_matching_files.length}）</summary>${v.retained_matching_files.map(source=>`<p>${esc(source)}</p>`).join('')}</details>`:''}<div class="qa-actions">${!v.finished&&!v.session_closed?button(v.pending?.some(p=>/密码|password/i.test(p.reason))?'输入密码并继续':v.publish_error?'处理保存冲突':v.pending?.length?`处理 ${v.pending.length} 个待完成项目`:'继续保存或结束会话','manage',`data-session="${v.deep_session}"`):''}${v.recovery_path?button('打开恢复目录','recovery',`data-path="${esc(v.recovery_path)}"`):''}</div>`;}
 function latest(id){return [...s.jobs].reverse().find(j=>j.deep?.deep_session===id)?.deep;}
 function manage(id){const v=latest(id);if(!v||v.finished||v.session_closed){toast('此会话已结束；恢复目录仍可直接打开。');return;}
  if(s.active){toast('请先等待或取消当前任务。');return;}
  const pending=v.pending||[],passwordNode=pending.find(p=>/密码|password/i.test(p.reason));
  dialog('继续深度解压',`<input id="oa-deep-session" type="hidden" value="${id}"><p class="qa-sub">${esc(v.summary)}</p>${pending.length?`<label class="qa-field">待处理项目<select id="oa-deep-node">${pending.map(p=>`<option value="${p.node}" ${p===passwordNode?'selected':''}>${esc(p.source||p.name)} · ${esc(p.reason)}</option>`).join('')}</select></label><label class="qa-field">此压缩包的密码<input id="oa-deep-password" type="password" autocomplete="off"></label>`:''}${v.publish_error?`<p class="qa-sub">${esc(v.publish_error)}</p>`:''}<label class="qa-field">继续保存时的同名文件处理<select id="oa-deep-conflict"><option value="error">停止并保留待处理项目</option><option value="rename">保留两份，自动改名</option><option value="overwrite">覆盖</option><option value="skip">跳过，并保留恢复结果</option></select></label>${limitsMarkup()}<p class="qa-sub">已完成的解压不会重复。结束会话会保留原压缩包和恢复目录。</p>`,btn('返回','close')+button('结束并保留恢复目录','discard')+(pending.length?button('保留此包并继续','skip'):'')+button(pending.length?'重试所选项目':'继续保存','resume'),'oa-extraction-dialog');
  if(passwordNode)$('oa-deep-password')?.focus();
 }
 async function finish(result,options){if(result.recycled_originals?.length)onRecycled(result.recycled_originals);if(options)completionOptions.set(result.deep_session,options);const {openAfter=false,closeAfter=false}=completionOptions.get(result.deep_session)||{};if(openAfter&&result.finished&&!result.cancelled&&result.outputs?.length)await invoke('reveal_path',{path:result.outputs[0]});if(closeAfter&&result.finished&&!result.partial&&!result.cancelled&&!result.recycle_error)await invoke('window_action',{action:'close'});if(!result.finished&&!result.session_closed)toast(result.cancelled?'已取消，恢复目录已保留。':'还有待处理项目，可在任务中继续。');if(result.finished||result.session_closed)completionOptions.delete(result.deep_session);}
 root.addEventListener('input',e=>{
  if(e.target.id==='oa-deep-inputs')renderInputs();
  if(e.target.id==='oa-deep-destination')batchPreview();
 });
 root.addEventListener('change',e=>{
  const t=e.target;
  if(t.id==='oa-deep-recycle'||t.id==='oa-deep-smart')batchPreview();
  if(t.dataset.deepInput!==undefined)updateInput(Number(t.dataset.deepInput),t.value.trim());
 });
 root.addEventListener('click',async e=>{const target=e.target.closest('[data-deep-action]');if(!target)return;e.preventDefault();try{switch(target.dataset.deepAction){
  case'pick-inputs':acceptDrop(await invoke('pick_files'));break;
  case'pick-dest':{const path=await invoke('pick_folder');if(path&&$('oa-deep-destination')){$('oa-deep-destination').value=path;batchPreview();}break;}
  case'remove-input':updateInput(Number(target.dataset.index),null);break;
  case'pick-recovery':{const path=await invoke('pick_folder');if(path)$('oa-deep-recovery').value=path;break;}
  case'output':case'recovery':await invoke('reveal_path',{path:target.dataset.path});break;
  case'manage':manage(Number(target.dataset.session));break;
  case'start-batch':{const inputs=inputPaths(),dest=$('oa-deep-destination').value.trim(),overwrite=$('oa-deep-conflict').value;if(!inputs.length)throw Error('请选择至少一个压缩包。');
    const recycleAfter=$('oa-deep-recycle').checked;
    if(recycleAfter&&overwrite==='skip')throw Error('自动回收需要完整解压，请选择另一种同名文件处理方式。');
    const limits=settings(),deep={...limits,dedicated_output:true},smart=$('oa-deep-smart').checked,password=$('oa-deep-password').value||null;
    const openAfter=$('oa-deep-open-after').checked,closeAfter=$('oa-deep-close-after').checked;
    if($('oa-deep-remember').checked){
     Object.assign(s,{deepLimits:limits,deepRecycleAfter:recycleAfter,sameFolder:smart,overwrite,openAfter,closeAfter});
     appearance();
    }
    $('oa-deep-password').value='';
    const result=await run({operation:'deep-extract',inputs,dest,smart,deep,options:{overwrite,password}},'批量深度解压',{recycleAfter});await finish(result,{openAfter,closeAfter});break;}
  case'discard':{const id=Number($('oa-deep-session').value);if($('oa-deep-password'))$('oa-deep-password').value='';await finish(await run({operation:'deep-discard',deep_session:id,options:{}},'结束深度解压会话'));break;}
  case'resume':case'skip':{const id=Number($('oa-deep-session').value),node=$('oa-deep-node')?Number($('oa-deep-node').value):null,password=$('oa-deep-password')?.value||null,overwrite=$('oa-deep-conflict').value,deep=settings();if($('oa-deep-password'))$('oa-deep-password').value='';const result=await run({operation:'deep-resume',deep_session:id,deep_node:node,deep_skip:target.dataset.deepAction==='skip',deep,options:{password,overwrite}},'继续深度解压');await finish(result);break;}
 }}catch(error){toast(String(error));}});
 return {switchMarkup,limitsMarkup,settings,sync,batch,acceptDrop,jobActions,finish};
};
})();
