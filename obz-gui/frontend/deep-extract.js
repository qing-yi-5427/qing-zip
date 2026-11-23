(()=>{'use strict';
const gib=1073741824;
const defaults={depth:8,archives:256,files:100000,bytes:20*gib,reserve_bytes:gib,recovery_directory:null};
window.QingDeepExtract=({passwordBook,root,s,pref,esc,btn,dialog,$,run,invoke,appearance,toast,error,archiveParent,joinExtractionPath,folderForSource,conflictHelp,recycleSummary,sourceDisposition,onRecycled,onBrowse,passwordRequiredForSource,currentReport,canContinue})=>{
 s.deepExtract=!!pref.deepExtract;s.deepRecycleAfter=!!pref.deepRecycleAfter;s.deepLimits={...defaults,...pref.deepLimits};
 const sessionOptions=new Map();
 const button=(label,action,extra='',variant='')=>`<button type="button" class="qa-button ${variant}" data-deep-action="${action}" ${extra}>${label}</button>`;
 function passwordMarkup(id,text,label='密码'){
  return `<div id="${id}-reveal" class="qa-actions oa-extract-password" hidden>${button(text,'enter-password',`data-password-input="${id}" aria-controls="${id}"`,'quiet')}</div><label class="qa-field" hidden>${label}<input id="${id}" type="password" autocomplete="off" data-password-book disabled></label>`;
 }
 function showPasswordField(input){
  input.disabled=false;
  input.closest('label').hidden=false;
  $(input.id+'-reveal').hidden=true;
 }
 function syncPasswordField(id,{required=false,optional=false,cached=''}={}){
  const input=$(id),reveal=$(id+'-reveal');
  if(!optional)delete reveal.dataset.expanded;
  if(required||(optional&&reveal.dataset.expanded==='true')){
   if(required&&input.disabled)input.value=cached;
   showPasswordField(input);
  }else{
   input.value='';
   input.disabled=true;
   input.closest('label').hidden=true;
   passwordBook?.closePicker(input);
   reveal.hidden=!optional;
  }
 }
 function limitsMarkup(limits=s.deepLimits,{resume=false}={}){
  return `<details id="oa-deep-limits" class="oa-extract-advanced">
   <summary>${resume?'调整本次解压的限制':'深度解压限制与恢复位置'}</summary>
   <p class="qa-sub">原包计为第 1 层。累计文件数和解码量包含处理中的文件及重试。</p>
   <div class="qa-fieldrow oa-deep-limits-grid">
    <label class="qa-field">最大层数<input id="oa-deep-depth" type="number" min="1" max="64" value="${esc(limits.depth)}"></label>
    <label class="qa-field">最多压缩包<input id="oa-deep-archives" type="number" min="1" max="10000" value="${esc(limits.archives)}"></label>
    <label class="qa-field">累计文件数量<input id="oa-deep-files" type="number" min="1" value="${esc(limits.files)}"></label>
    <label class="qa-field">累计解码量（GiB）<input id="oa-deep-bytes" type="number" min="0" step="any" value="${esc(limits.bytes/gib)}"></label>
    <label class="qa-field">保留磁盘空间（GiB）<input id="oa-deep-reserve" type="number" min="0" step="any" value="${esc(limits.reserve_bytes/gib)}"></label>
   </div>
   ${resume?'':`<div class="qa-field"><label for="oa-deep-recovery">恢复位置</label><div class="qa-inlinefield oa-extract-target">
    <input id="oa-deep-recovery" value="${esc(limits.recovery_directory||'')}" placeholder="默认使用本机应用数据文件夹" spellcheck="false">${button('浏览…','pick-recovery')}
   </div></div><p class="qa-sub">中断后，未保存的已校验结果会留在恢复目录。已保存的文件留在目标位置。</p>`}
  </details>`;
 }
 function switchMarkup(enabled=s.deepExtract){
  return `<label class="qa-check"><input id="oa-deep-extract" type="checkbox" ${enabled?'checked':''}>深度解压</label>`;
 }
 function settings(){
  if(!$('oa-deep-depth'))return {...s.deepLimits};
  const number=(id,label,{min=1,max=Number.MAX_SAFE_INTEGER,scale=1}={})=>{
   const input=$('oa-deep-'+id);
   const raw=Number(input.value),value=scale===1?raw:Math.round(raw*scale);
   if(!Number.isSafeInteger(value)||raw<0||value<min||value>max){
    $('oa-deep-limits').open=true;
    input.focus();
    throw Error(label);
   }
   return value;
  };
  return {
   depth:number('depth','最大层数需为 1–64 的整数。',{max:64}),
   archives:number('archives','压缩包数量需为 1–10000 的整数。',{max:10000}),
   files:number('files','累计文件数量需为正整数。'),
   bytes:number('bytes','累计解码量需为有效的正数。',{scale:gib}),
   reserve_bytes:number('reserve','保留磁盘空间需为有效的非负数。',{min:0,scale:gib}),
   recovery_directory:$('oa-deep-recovery')?.value.trim()||null
  };
 }
 function sync(){
  const limits=$('oa-deep-limits');
  if(limits)limits.hidden=!$('oa-deep-extract')?.checked;
 }
 function batch(paths=[],{allowBrowse=false,password=""}={}){
  if(s.active){toast('请先等待或取消当前任务。');return;}
  dialog('批量深度解压',`
   <div class="qa-actions oa-source-heading"><strong id="oa-deep-source-label">压缩包与预计输出目录</strong><span id="oa-deep-count" class="qa-sub" role="status"></span>${button('添加压缩包…','pick-inputs')}</div>
   <div id="oa-deep-list" class="oa-deep-list" role="group" aria-labelledby="oa-deep-source-label"></div>
   <div class="qa-field">
    <label for="oa-deep-destination">保存到</label>
    <div class="qa-inlinefield oa-extract-target"><input id="oa-deep-destination" placeholder="留空时保存到各原包所在目录" aria-describedby="oa-deep-plan-help" spellcheck="false">${button('浏览…','pick-dest')}</div>
   </div>
   <div class="oa-extract-options">
    <div class="oa-extract-modes"><label class="qa-check" title="包内已有同名顶层目录时，沿用这层目录"><input id="oa-deep-smart" type="checkbox" ${s.sameFolder?'checked':''}>智能解压</label></div>
    <label class="oa-extract-conflict">同名文件<select id="oa-deep-conflict" aria-describedby="oa-deep-conflict-help">${conflictChoices(s.overwrite)}</select></label>
   </div>
   <p id="oa-deep-plan-help" class="qa-sub oa-extract-hint">每个原包使用独立目录。展开后确定实际目录，重名时自动编号。分卷归并后使用同一目录。</p>
   <p id="oa-deep-conflict-help" class="qa-sub oa-extract-hint" role="status"></p>
   ${passwordMarkup('oa-deep-password','填写解压密码')}
   <details class="oa-extract-advanced oa-deep-paste">
    <summary>粘贴或输入路径</summary>
    <textarea id="oa-deep-inputs" rows="3" aria-label="压缩包路径（每行一个）" placeholder="每行输入一个压缩包路径" spellcheck="false"></textarea>
   </details>
   <details class="oa-extract-advanced"><summary>更多解压选项</summary>
    <label class="oa-switch-row"><span>完成后打开目标文件夹</span><input id="oa-deep-open-after" class="oa-switch" role="switch" type="checkbox" ${s.openAfter?'checked':''}></label>
    <label class="oa-switch-row"><span>完成后关闭窗口</span><input id="oa-deep-close-after" class="oa-switch" role="switch" type="checkbox" ${s.closeAfter?'checked':''}></label>
    <label class="qa-check"><input id="oa-deep-remember" type="checkbox" checked>记住解压选项</label>
    <p class="qa-sub">按内容识别压缩包。Office、APK、JAR、EPUB 等容器保持完整。</p>
   </details>
   ${limitsMarkup()}
  `,recycleSummary({statusId:'oa-deep-source-status',recycleId:'oa-deep-recycle',recycle:s.deepRecycleAfter})+
   btn('取消','close')+(allowBrowse?button('改为打开压缩包','browse-archives'):'')+
   button('开始深度解压','start-batch','title="Ctrl+Enter" aria-keyshortcuts="Control+Enter"','primary'),'oa-extraction-dialog oa-extraction-form');
  if(password)$('oa-deep-password-reveal').dataset.expanded='true';
  acceptDrop(paths);
  if(password)$('oa-deep-password').value=password;
  (paths.length?$('oa-deep-destination'):root.querySelector('[data-deep-action=pick-inputs]')).focus({preventScroll:true});
 }
 const inputRows=()=>$('oa-deep-inputs').value.split(/\r?\n/);
 const inputPaths=()=>[...new Set(inputRows().map(path=>path.trim()).filter(Boolean))];
 function batchPreview(reuseFolders=false){
  const dest=$('oa-deep-destination').value.trim(),smart=$('oa-deep-smart').checked,seen=new Set();
  for(const [index,line] of inputRows().entries()){
   const source=line.trim(),output=$('oa-deep-path-'+index);
   if(!output)continue;
   if(!reuseFolders||output.dataset.source!==source){
    output.dataset.source=source;
    output.dataset.folder=source?folderForSource(source,smart):'';
   }
   const path=source?joinExtractionPath(dest||archiveParent(source),output.dataset.folder):'';
   output.textContent=source&&seen.has(source)?'重复路径，本批次处理一次。':path||'输入压缩包路径后显示预计目录。';
   output.title=path;
   seen.add(source);
  }
 }
 function syncConflict(){
  const value=$('oa-deep-conflict').value,help=$('oa-deep-conflict-help');
  help.textContent=conflictHelp(value,true);
  help.dataset.overwrite=String(value==='overwrite');
 }
 function syncBatchActions(paths=inputPaths()){
  syncConflict();
  const recycle=$('oa-deep-recycle').checked;
  const blocked=recycle&&$('oa-deep-conflict').value==='skip';
  const disposition=$('oa-deep-source-status'),submit=root.querySelector('[data-deep-action=start-batch]');
  disposition.textContent=blocked?'选择其他同名处理方式，或关闭成功后回收。':sourceDisposition(recycle,{deep:true});
  disposition.dataset.recycle=String(recycle&&!blocked);
  submit.textContent=recycle&&!blocked?'深度解压并回收':'开始深度解压';
  submit.disabled=blocked||!paths.length;
 }
 function syncBatchSources(){
  const paths=inputPaths();
  $('oa-deep-count').textContent=paths.length+' 个来源';
  syncPasswordField('oa-deep-password',{required:paths.some(passwordRequiredForSource),optional:true});
  syncBatchActions(paths);
 }
 function renderInputs(){
  $('oa-deep-list').innerHTML=inputRows().map((path,index)=>path.trim()?`<div class="oa-deep-source">
   <input value="${esc(path)}" title="${esc(path)}" spellcheck="false" aria-label="压缩包路径 ${index+1}" aria-describedby="oa-deep-path-${index}" data-deep-input="${index}">
   ${button('移除','remove-input',`data-index="${index}" aria-label="移除压缩包 ${index+1}"`,'quiet')}
   <p id="oa-deep-path-${index}" class="oa-deep-plan-path oa-extract-output"></p>
  </div>`:'').join('')||'<p class="qa-sub oa-deep-empty">拖入压缩包、粘贴路径或浏览添加。</p>';
  batchPreview();
  syncBatchSources();
 }
 function updateInput(index,path){
  const field=$('oa-deep-inputs'),paths=inputRows();
  if(path===null){
   paths.splice(index,1);
   field.value=paths.join('\n');
   renderInputs();
   (root.querySelector('[data-deep-input="'+index+'"]')||root.querySelector('[data-deep-action=pick-inputs]')).focus({preventScroll:true});
  }else{
   paths[index]=path;
   field.value=paths.join('\n');
   batchPreview(true);
   syncBatchSources();
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
  return [['error','暂停并保留待处理项目'],['rename','保留两份，自动改名'],['overwrite','替换已有文件'],['skip','跳过已有文件']].map(([id,label])=>`<option value="${id}" ${value===id?'selected':''}>${label}</option>`).join('');
 }
 const needsPassword=item=>/密码|password/i.test(item?.reason||'');
 const pathDetails=(label,paths)=>!paths?.length?'':`<details class="qa-sub oa-deep-path-details"><summary>${label}（${paths.length}）</summary>${paths.map(path=>`<p class="oa-extract-output">${esc(path)}</p>`).join('')}</details>`;
 const outputLinks=paths=>!paths?.length?'':`<div class="oa-deep-outputs">${paths.map(path=>button(esc(path),'output',`data-path="${esc(path)}" title="打开输出位置"`)).join('')}</div>`;
 const revealRecovery=path=>path?button('打开恢复目录','recovery',`data-path="${esc(path)}"`):'';
 function manageButton(report){
  let label='继续保存或结束会话';
  if(report.pending?.some(needsPassword))label='输入密码并继续';
  else if(report.pending?.length)label=`处理 ${report.pending.length} 个待完成项目`;
  else if(report.publish_error)label=/同名|已存在|conflict|already exists/i.test(report.publish_error)?'处理保存冲突':'检查后继续保存';
  return button(label,'manage',`data-session="${report.deep_session}"`);
 }
 function jobActions(job){
  const v=job.deep,recovery=v??job.deepRecovery;
  if(!recovery)return '';
  const recoveryLocation=job.followedBy!=null&&recovery.recovery_path?
   pathDetails('当时的恢复目录',[recovery.recovery_path]):revealRecovery(recovery.recovery_path);
  return `
   ${v?.finished&&v.archives?`<p class="qa-sub oa-deep-counts" data-archive-count="${v.archives}" data-max-depth="${v.max_depth}">识别 ${v.archives} 个压缩包 · 最深 ${v.max_depth} 层</p>`:''}
   ${pathDetails('保留的原包',v?.retained_originals)}
   ${outputLinks(v?.outputs)}
   ${pathDetails('保留的压缩包',v?.retained_archives)}
   ${pathDetails('保留的同名匹配文件',v?.retained_matching_files)}
   ${canContinue(job)?`<div class="qa-actions">${manageButton(recovery)}</div>`:''}
   ${recoveryLocation}`;
 }
 function manage(id){
  if(s.active){toast('请先等待或取消当前任务。');return;}
  const v=currentReport(id);
  if(!v){toast('此会话已结束，仍可打开恢复目录。');return;}
  const pending=v.pending||[],first=pending.find(needsPassword)||pending[0],options=sessionOptions.get(id);
  dialog('继续深度解压',`
   <input id="oa-deep-session" type="hidden" value="${id}">
   <p class="qa-sub oa-extract-hint">${esc(v.summary)}</p>
   ${pending.length?`
    <label class="qa-field">待处理项目（${pending.length}）<select id="oa-deep-node" aria-describedby="oa-deep-node-source oa-deep-node-reason">
     ${pending.map((p,index)=>`<option value="${p.node}" ${p===first?'selected':''}>${index+1}. ${esc(p.name||p.source)}</option>`).join('')}
    </select></label>
    <div class="oa-extract-problem">
     <p id="oa-deep-node-source" class="oa-extract-output"></p>
     <p id="oa-deep-node-reason"></p>
    </div>
    ${passwordMarkup('oa-deep-password','填写此包密码','此压缩包的密码')}
   `:''}
   ${v.publish_error?`<p class="oa-extract-problem">${esc(v.publish_error)}</p>`:''}
   <label class="qa-field oa-extract-retry-conflict">保存时的同名文件<select id="oa-deep-conflict" aria-describedby="oa-deep-conflict-help">${conflictChoices(options?.overwrite||'error')}</select></label>
   <p id="oa-deep-conflict-help" class="qa-sub oa-extract-hint" role="status"></p>
   ${v.recovery_path?`<div class="oa-extract-location"><div class="oa-extract-location-value"><span class="qa-sub">恢复目录</span><div class="oa-extract-output" tabindex="0">${esc(v.recovery_path)}</div></div>${revealRecovery(v.recovery_path)}</div>`:''}
   ${v.outputs?.length?`<details class="oa-extract-advanced"><summary>已保存的位置（${v.outputs.length}）</summary>${outputLinks(v.outputs)}</details>`:''}
   ${pending.length?limitsMarkup(options?.limits,{resume:true}):''}
  `,`<div id="oa-deep-manage-actions" class="qa-actions oa-deep-recovery-actions">
   ${button('结束会话','discard-review','aria-controls="oa-deep-discard-review"','quiet')}
   ${btn('返回','close')}
   ${pending.length?button('保留此包并继续','skip'):''}
   ${button(pending.length?'重试所选项目':'继续保存','resume','title="Ctrl+Enter" aria-keyshortcuts="Control+Enter"','primary')}
  </div><div id="oa-deep-discard-review" class="oa-deep-discard-review" hidden>
   <p>结束后可打开恢复目录。原包与已保存文件会保留，如需继续解压请开启新任务。</p>
   <div class="qa-actions">${button('返回当前会话','discard-cancel')}${button('确认结束会话','discard','','danger')}</div>
  </div>`,'oa-extraction-dialog oa-extraction-form');
  syncConflict();
  if(first)syncPendingNode();
  (needsPassword(first)?$('oa-deep-password'):$('oa-deep-node')||$('oa-deep-conflict')).focus({preventScroll:true});
 }
 function syncPendingNode(clearPassword=false){
  const id=Number($('oa-deep-session').value),node=Number($('oa-deep-node').value);
  const item=currentReport(id)?.pending?.find(item=>item.node===node);
  if(!item)return;
  $('oa-deep-node-source').textContent=item.source||item.name;
  $('oa-deep-node-reason').textContent=item.reason;
  if(clearPassword){
   const input=$('oa-deep-password');
   input.value='';
   delete $('oa-deep-password-reveal').dataset.expanded;
   passwordBook?.closePicker(input);
  }
  syncPasswordField('oa-deep-password',{required:needsPassword(item),optional:true});
 }
 function showDiscardReview(show){
  const actions=$('oa-deep-manage-actions'),review=$('oa-deep-discard-review');
  actions.hidden=show;review.hidden=!show;
  (show?review.querySelector('[data-deep-action=discard-cancel]'):actions.querySelector('[data-deep-action=discard-review]')).focus();
 }
 async function finish(result,options){
  const id=result.deep_session;
  if(result.recycled_originals?.length)onRecycled(result.recycled_originals);
  if(options)sessionOptions.set(id,{...sessionOptions.get(id),...options});
  const {openAfter=false,closeAfter=false}=sessionOptions.get(id)||{};
  if(result.finished||result.session_closed)sessionOptions.delete(id);
  if(openAfter&&result.finished&&!result.cancelled&&result.outputs?.length)await invoke('reveal_path',{path:result.outputs[0]});
  if(closeAfter&&result.finished&&!result.partial&&!result.cancelled&&!result.recycle_error)await invoke('window_action',{action:'close'});
  if(!result.finished&&!result.session_closed)toast(result.cancelled?'已取消，恢复目录已保留。':'还有待处理项目，可在任务中继续。');
 }
 root.addEventListener('input',e=>{
  const t=e.target;
  if(t.id==='oa-deep-inputs')renderInputs();
  if(t.id==='oa-deep-destination')batchPreview(true);
  if(t.dataset.deepInput!==undefined){
   t.title=t.value;
   updateInput(Number(t.dataset.deepInput),t.value.trim());
  }
 });
 root.addEventListener('change',e=>{
  const {id}=e.target;
  if(id==='oa-deep-smart')batchPreview();
  if(id==='oa-deep-node')syncPendingNode(true);
  if(id==='oa-deep-recycle'||id==='oa-deep-conflict'&&$('oa-deep-inputs'))syncBatchActions();
  else if(id==='oa-deep-conflict')syncConflict();
 });
 async function startBatch(){
  const inputs=inputPaths(),dest=$('oa-deep-destination').value.trim(),overwrite=$('oa-deep-conflict').value;
  if(!inputs.length)throw Error('请选择至少一个压缩包。');
  const recycleAfter=$('oa-deep-recycle').checked;
  if(recycleAfter&&overwrite==='skip')throw Error('选择其他同名处理方式，或关闭成功后回收。');
  const limits=settings(),deep={...limits,dedicated_output:true};
  const smart=$('oa-deep-smart').checked,input=$('oa-deep-password'),password=input.disabled?null:input.value||null;
  const openAfter=$('oa-deep-open-after').checked,closeAfter=$('oa-deep-close-after').checked;
  if($('oa-deep-remember').checked){
   Object.assign(s,{deepLimits:limits,deepRecycleAfter:recycleAfter,sameFolder:smart,overwrite,openAfter,closeAfter});
   appearance();
  }
  input.value='';
  const result=await run({operation:'deep-extract',inputs,dest,smart,deep,options:{overwrite,password}},'批量深度解压',{recycleAfter});
  await finish(result,{openAfter,closeAfter,limits,overwrite});
 }
 async function resumeSession(skip){
  const id=Number($('oa-deep-session').value),node=$('oa-deep-node'),input=$('oa-deep-password');
  const overwrite=$('oa-deep-conflict').value,password=input?.value||null,limits=node?settings():null;
  if(input)input.value='';
  const result=await run({operation:'deep-resume',deep_session:id,deep_node:node?Number(node.value):null,deep_skip:skip,
   ...(limits?{deep:limits}:{}),options:{password,overwrite}},'继续深度解压');
  await finish(result,limits?{limits,overwrite}:{overwrite});
 }
 root.addEventListener('click',async e=>{
  const target=e.target.closest('[data-deep-action]');
  if(!target||target.disabled)return;
  e.preventDefault();
  try{
   const action=target.dataset.deepAction;
   switch(action){
   case'browse-archives':onBrowse(inputPaths());break;
   case'enter-password':{
    const input=$(target.dataset.passwordInput);
    $(input.id+'-reveal').dataset.expanded='true';
    showPasswordField(input);
    input.focus();
    break;
   }
   case'pick-inputs':{
    const input=$('oa-deep-inputs'),paths=await invoke('pick_files');
    if(input===$('oa-deep-inputs')&&paths.length)acceptDrop(paths);
    break;
   }
   case'pick-dest':{
    const input=$('oa-deep-destination'),path=await invoke('pick_folder');
    if(path&&input===$('oa-deep-destination')){input.value=path;batchPreview(true);}
    break;
   }
   case'remove-input':updateInput(Number(target.dataset.index),null);break;
   case'pick-recovery':{
    const input=$('oa-deep-recovery'),path=await invoke('pick_folder');
    if(path&&input===$('oa-deep-recovery'))input.value=path;
    break;
   }
   case'output':case'recovery':await invoke('reveal_path',{path:target.dataset.path});break;
   case'manage':manage(Number(target.dataset.session));break;
   case'start-batch':await startBatch();break;
   case'discard-review':showDiscardReview(true);break;
   case'discard-cancel':showDiscardReview(false);break;
   case'discard':{
    if($('oa-deep-discard-review').hidden)return;
    const id=Number($('oa-deep-session').value),input=$('oa-deep-password');
    if(input)input.value='';
    await finish(await run({operation:'deep-discard',deep_session:id,options:{}},'结束深度解压会话'));
    break;
   }
   case'resume':case'skip':await resumeSession(action==='skip');break;
   }
  }catch(cause){error(cause);}
 });
 return {switchMarkup,limitsMarkup,settings,sync,passwordMarkup,syncPasswordField,batch,acceptDrop,jobActions,finish};
};
})();
