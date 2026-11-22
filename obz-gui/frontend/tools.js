const invoke=(window.__TAURI__||window.parent.__TAURI__).core.invoke,$=id=>document.getElementById(id),lines=id=>$(id).value.split(/\r?\n/).filter(s=>s.length>0);let active=null;
if (window === window.top) {
 const readPreference = () => {
  let theme;
  try { theme = JSON.parse(localStorage.getItem('qing-preferences'))?.theme; } catch {}
  return { theme: QingAppearance.preferredMode(theme), material: 'solid' };
 };
 const updateAppearance = QingAppearance.bind(readPreference, theme => {
  document.documentElement.style.colorScheme = theme;
 }, console.warn);
 window.addEventListener('storage', event => {
  if (event.key === 'qing-preferences') updateAppearance();
 });
 updateAppearance();
}
const params=new URLSearchParams(location.search);
const initialOperation=params.get('operation');
const hasInitialOperation=[...$('operation').options].some(option=>option.value===initialOperation);
const fixedOperation=hasInitialOperation&&params.get('choose')!=='1';
const archiveScope=fixedOperation&&params.get('scope')==='archive'&&!!params.get('path');
const passwordBook=window.QingPasswordBook({invoke});
const shellManager=fixedOperation&&['shell-enable','shell-disable'].includes(initialOperation);
const automaticQueries=new Set(['capabilities']);
const cancellableOperations=new Set(['list','extract','create','test','add','delete','rename','preview','recover','checksum','benchmark','file-split','file-combine']);
const actions={
 list:'查看文件列表',extract:'解压文件',create:'创建压缩包',test:'检查完整性',add:'添加或更新文件',
 delete:'删除包内条目',rename:'重命名包内条目',preview:'预览图片',repair:'修复并另存为',recover:'开始尝试密码',
 'file-split':'分割文件','file-combine':'合并文件',benchmark:'开始基准测试',checksum:'计算校验值',
 'shell-enable':'启用右键入口','shell-disable':'移除右键入口'
};
let archivePreviewImages=null;
let hashDiscovery=null;
const listedHashes=new Set();
function addHashChoice(name){
 if(listedHashes.has(name))return;
 listedHashes.add(name);
 $('hash-algorithm').add(new Option(name,name));
 const label=document.createElement('label'),input=document.createElement('input');
 input.type='checkbox';input.value=name;
 label.append(input,document.createTextNode(name));$('hash-choices').append(label);
}
function loadHashAlgorithms(){
 if(!listedHashes.size){
   $('hash-algorithm').add(new Option('自选多项…','custom'));
   ['CRC32','MD5','SHA1','SHA256'].forEach(addHashChoice);
 }
 if(!hashDiscovery){
   hashDiscovery=invoke('checksum_algorithms').then(algorithms=>{
     $('hash-algorithm').add(new Option('全部可用算法','*'));
     algorithms.forEach(({name})=>addHashChoice(name));
   }).catch(()=>{hashDiscovery=null;});
 }
}
const show=(id,yes)=>{const group=$(id);group.classList.toggle('hidden',!yes);if(['benchmark-group','parts-size-group','parts-total-group'].includes(id))group.querySelectorAll('input,select').forEach(input=>input.disabled=!yes);};const fileOps=['list','extract','test','add','delete','rename','preview','repair','recover','checksum'];
function update(){const op=$('operation').value,creating=op==='create';show('benchmark-group',op==='benchmark');show('path-group',op!=='checksum'&&(fileOps.includes(op)||creating));show('checksum-paths-group',op==='checksum');$('path-label').textContent=creating?'新压缩包保存路径':'压缩包路径';show('dest-group',['extract','repair'].includes(op));$('dest-label').textContent=op==='repair'?'修复后另存为（必须是新文件）':'解压到';show('inputs-group',['create','add','delete','rename'].includes(op));$('inputs-label').textContent=op==='rename'?'旧名称、新名称交替填写，每行一个（如：旧名.txt 换行 新名.txt）':op==='delete'?'要删除的包内完整名称（每行一个）':'输入文件（每行一个完整路径）';show('files',['create','add'].includes(op));show('folder',['create','add'].includes(op));show('format-group',creating);show('hash-group',op==='checksum');if(op==='checksum')loadHashAlgorithms();show('overwrite-group',op==='extract');show('selected-group',op==='extract');show('name-group',op==='preview');show('password-group',!['benchmark','capabilities','checksum','recover','shell-enable','shell-disable'].includes(op));show('recovery-group',op==='recover');show('advanced',['list','extract','test','add','create','preview','delete','rename','repair'].includes(op));$('hint').textContent=op==='repair'?'修复仅恢复能验证的 ZIP 条目；原文件保留不变，不能保证恢复全部内容。':op==='recover'?'按候选顺序进行有上限的尝试；并非保证成功的密码破解。':op==='shell-enable'?'仅增加当前用户的“使用 qing-zip 打开”右键入口，不修改默认打开方式。':creating?'新建操作不会覆盖已有压缩包。GZip、XZ、BZip2 以及 Brotli、LZ4、LZ5、Lizard、Zstandard 单流格式只能直接输入一个文件。':'';}
function updateFormat(){const creating=$('operation').value==='create',format=$('format').value,raw=['zstd','brotli','lz4','lz5','lizard'].includes(format),previous=$('level').value,levels=window.qingCompression.levels(format);$('level').replaceChildren(new Option('默认',''),...levels.map(([value,label])=>new Option(label,String(value))));$('level').value=levels.some(([value])=>String(value)===previous)?previous:'';$('volume').disabled=creating&&raw;if(creating){const encrypted=['zip','zipx','7z','sfx'].includes(format);show('password-group',encrypted);}}
const archiveToolUpdate=update;
update=()=>{archiveToolUpdate();const op=$('operation').value,parts=['file-split','file-combine'].includes(op);show('parts-size-group',op==='file-split');show('parts-total-group',op==='file-combine');if(parts){show('path-group',true);show('dest-group',true);show('password-group',false);$('path-label').textContent=op==='file-split'?'源文件':'首卷文件（.001）';$('dest-label').textContent=op==='file-split'?'保存到文件夹':'合并后保存为';$('hint').textContent=op==='file-split'?'在目标位置创建同名 .parts 文件夹，保留源文件。':'有校验清单时验证全部分块；无校验清单时按卷序合并，无法判断末卷是否缺失。';}};
function updateOperation(){
 update();
 updateFormat();
 const op=$('operation').value;
 show('operation-group',!fixedOperation);
 show('run',!automaticQueries.has(op));
 $('run').textContent=actions[op]||'重新读取';
 $('path').readOnly=archiveScope;
 $('checksum-paths').readOnly=archiveScope;
 show('browse',!archiveScope);
 show('checksum-browse',!archiveScope);
 show('checksum-folder',!archiveScope);
 $('checksum-paths-label').textContent=archiveScope?'待校验的压缩包':'待校验文件或文件夹（每行一个路径）';
 show('tool-back',shellManager);
 $('tool-back').textContent=op==='shell-enable'?'移除右键入口…':'启用右键入口…';
 $('path').required=!archiveScope&&(fileOps.includes(op)&&op!=='checksum'||['create','file-split','file-combine'].includes(op));
 $('dest').required=['extract','repair','file-split','file-combine'].includes(op);
 const scopedImages=op==='preview'&&archivePreviewImages!==null;
 show('name',!scopedImages);
 show('archive-image',scopedImages&&archivePreviewImages.length>0);
 if(scopedImages)show('run',false);
 $('name-label').htmlFor=scopedImages?'archive-image':'name';
 $('name').required=!scopedImages&&op==='preview';
 $('archive-image').required=scopedImages&&archivePreviewImages.length>0;
 if(scopedImages&&!archivePreviewImages.length){
  show('name-group',false);
  $('hint').textContent='当前压缩包中没有可预览的图片。';
 }
}
function selectOperation(op){
 $('operation').value=op;
 passwordBook.closePicker($('password'));
 for(const id of ['password','candidates','charset'])$(id).value='';
 $('status').textContent='';
 $('status').className='';
 $('result').replaceChildren();
 show('progress',false);
 updateOperation();
 if(automaticQueries.has(op))return runOperation();
}
$('operation').onchange=()=>selectOperation($('operation').value);
$('format').onchange=updateOperation;
$('tool-back').onclick=()=>{
 if(window.obzBusy)return;
 const op=$('operation').value;
 return selectOperation(op==='shell-enable'?'shell-disable':'shell-enable');
};
$('browse').onclick=async()=>{if($('operation').value==='create'){$('path').value=await invoke('pick_archive_save',{format:$('format').value})||$('path').value;}else{const f=await invoke(['file-split','file-combine'].includes($('operation').value)?'pick_files_for_zip':'pick_files');if(f.length)$('path').value=f[0];}};
$('browse-dest').onclick=async()=>{const op=$('operation').value;$('dest').value=await invoke(op==='file-combine'?'pick_combined_save':op==='repair'?'pick_save':'pick_folder',op==='file-combine'?{path:$('path').value}:undefined)||$('dest').value;};
$('files').onclick=async()=>{const f=await invoke('pick_files_for_zip');if(f.length)$('inputs').value=[$('inputs').value,...f].filter(Boolean).join('\n');};
$('folder').onclick=async()=>{const p=await invoke('pick_folder');if(p)$('inputs').value=[$('inputs').value,p].filter(Boolean).join('\n');};
function text(content,tag='div'){const el=document.createElement(tag);el.textContent=content;$('result').append(el);return el;}
function render(op,result){
 $('result').replaceChildren();
 if(op==='benchmark'){
 text(`LZMA · 请求参数：${result.parameters.threads} 线程 · ${(2**result.parameters.dictionary_log/1048576).toLocaleString()} MiB 字典 · ${result.parameters.iterations} 轮`);
 const table=document.createElement('table'),head=document.createElement('tr');
 for(const title of ['项目','速度（KiB/s）','CPU（%）','评分（MIPS）']){const th=document.createElement('th');th.textContent=title;head.append(th);}table.append(head);
 for(const [title,score] of [['压缩',result.compressing],['解压',result.decompressing]]){const row=document.createElement('tr');for(const value of [title,score.speed_kib_s,score.cpu_percent,score.rating_mips]){const td=document.createElement('td');td.textContent=typeof value==='number'?value.toLocaleString():value;row.append(td);}table.append(row);}
 $('result').append(table);text(`耗时 ${result.elapsed_seconds.toFixed(2)} 秒`);const details=document.createElement('details'),summary=document.createElement('summary'),pre=document.createElement('pre');summary.textContent='完整引擎报告';pre.textContent=result.engine_report;pre.className='checksum-results';details.append(summary,pre);$('result').append(details);return;}
if(op==='file-split'||op==='file-combine'){text(op==='file-split'?'分割完成':'合并完成','h3');text('保存位置：'+result.output);text('大小：'+result.bytes+' 字节');text('SHA256：'+result.sha256,'pre');if(op==='file-combine')text(result.verified_manifest?'已验证所有分块与完整文件校验值':'无校验清单，已按卷序合并');return;}if(result.data_url){const img=document.createElement('img');img.id='preview';img.src=result.data_url;img.alt=result.name;$('result').append(img);return;}
if(Array.isArray(result.entries)){text(`${result.entries.length} 个条目`);const table=document.createElement('table');const head=document.createElement('tr');for(const s of ['名称','字节','加密','修改时间']){const h=document.createElement('th');h.textContent=s;head.append(h);}table.append(head);for(const e of result.entries){const tr=document.createElement('tr');for(const v of [e.name,e.is_dir?'目录':e.size_known===false?'未知':e.size,e.encrypted?'是':'否',e.modified]){const td=document.createElement('td');td.textContent=v;tr.append(td);}table.append(tr);}$('result').append(table);return;}
if(Array.isArray(result.checksums)||(op==='checksum'&&(result.hashes||result.sha256))){
 if(Array.isArray(result.checksums)&&!result.checksums.length){text('文件夹中没有可校验的文件。');return;}
 const output=(result.checksums||[result]).map(file=>{
   const hashes=file.hashes||Object.fromEntries(['crc32','md5','sha1','sha256'].map(k=>[k.toUpperCase(),file[k]]));
   return [file.path,...Object.entries(hashes).map(([name,value])=>name+': '+value)].join('\n');
 });
 text(output.join('\n\n'),'pre').className='checksum-results';
 return;
}
if(result.hashes){for(const [name,value] of Object.entries(result.hashes))text(name+': '+value);}else if(result.sha256){for(const k of ['crc32','md5','sha1','sha256'])text(k.toUpperCase()+': '+result[k]);}else if(typeof result.valid==='boolean')text(result.valid?'完整性检查通过。':'完整性检查未通过。');else if(result.files!==undefined)text(`已解压 ${result.files} 个文件 → ${result.destination}`);else if(result.outputs)result.outputs.forEach(p=>text('已创建：'+p));else if(result.updated)text('已更新：'+result.updated);else if(result.recovered){text(`恢复 ${result.recovered.length} 项 → ${result.output}`);text(result.complete?'恢复完成。':'部分恢复；原包中的其他内容可能无法恢复。');result.skipped.forEach(s=>text('未恢复：'+s));}else if(op==='recover'){text(`已尝试 ${result.attempts} 次 · ${{found:'找到密码',exhausted:'候选已用尽',limit_reached:'达到尝试上限',cancelled:'已取消'}[result.status]||result.status}`);if(result.password!==null){const input=document.createElement('input');input.type='password';input.readOnly=true;input.value=result.password;input.className='secret';$('result').append(input);const b=text('显示 / 隐藏密码','button');b.type='button';b.onclick=()=>input.type=input.type==='password'?'text':'password';}}else if(result.enabled!==undefined)text(result.enabled?'右键入口已启用。':'右键入口已移除。');else if(result.engine){text('可读取：'+result.engine.read_formats.join('、'));text('可创建：'+result.engine.write_formats.join('、'));text('引擎状态：'+(result.engine.available?'可用':'缺失'));result.engine.limitations.forEach(s=>text(s));}else text('操作完成。');}
$('cancel').onclick=async()=>{
 const id=active;
 if(id===null)return;
 $('cancel').disabled=true;
 $('status').textContent='正在取消…';
 try{
  await invoke('cancel_operation',{id});
 }catch(err){
  if(active===id){
   $('status').textContent=String(err);
   $('status').className='error';
   $('cancel').disabled=false;
  }
 }
};
$('form').onsubmit=event=>{
 event.preventDefault();
 const op=$('operation').value;
 if(op==='preview'&&archivePreviewImages!==null){
  if($('archive-image').value)return runOperation(op,$('archive-image').value);
  return;
 }
 return runOperation();
};
async function runOperation(op=$('operation').value,name=$('name').value){
 if(window.obzBusy)return;
 const options={};if(!$('password-group').classList.contains('hidden')&&$('password').value)options.password=$('password').value;if(!$('advanced').classList.contains('hidden')){for(const id of ['level','threads','codepage'])if($(id).value)options[id]=Number($(id).value);if(!$('volume').disabled&&$('volume').value)options.volume_bytes=Math.floor(Number($('volume').value)*1048576);}if(op==='extract'){options.selected=lines('selected');options.overwrite=$('overwrite').value;}const request={operation:op,path:$('path').value,dest:['extract','repair','file-split','file-combine'].includes(op)?$('dest').value:'',format:$('format').value,inputs:['create','add','delete','rename'].includes(op)?lines('inputs'):[],options,name:op==='preview'?name:'',smart:op==='extract'&&$('smart').checked,candidates:op==='recover'?lines('candidates'):[],max_attempts:Number($('max-attempts').value)||1000};
if(op==='benchmark'){
 request.path='';request.dest='';request.inputs=[];request.benchmark={threads:Number($('bench-threads').value),dictionary_log:Number($('bench-dictionary').value),iterations:Number($('bench-iterations').value)};
}
if(op==='file-split'){const size=Number($('parts-size').value)*Number($('parts-unit').value);if(!Number.isSafeInteger(size)||size<=0){$('status').textContent='请填写有效的分块大小';return;}request.max_bytes=size;}
if(op==='file-combine'&&$('parts-total').value!==''){const size=Number($('parts-total').value);if(!Number.isSafeInteger(size)||size<0){$('status').textContent='请填写有效的总字节数';return;}request.max_bytes=size;}
if(op==='checksum'){
 const paths=archiveScope?[params.get('path')]:lines('checksum-paths'),algorithm=$('hash-algorithm').value;
 if(!paths.length){$('status').textContent='请选择待校验文件。';return;}
 request.path=paths[0];request.inputs=[];
 if(paths.length>1){request.operation='checksum-files';request.inputs=paths;}
 if(algorithm==='custom'){
   request.hashes=[...$('hash-choices').querySelectorAll('input:checked')].map(input=>input.value);
   if(!request.hashes.length){$('status').textContent='请至少选择一种校验算法。';return;}
 }else if(algorithm){request.hashes=[algorithm];}
}
if(op==='recover'&&$('charset').value){request.charset=$('charset').value;request.min_length=Number($('min-length').value);request.max_length=Number($('max-length').value);}
if(['delete','shell-enable','shell-disable'].includes(op)&&!confirm('确认'+actions[op]+'？'))return;
const query=automaticQueries.has(op);
if(archiveScope&&fileOps.includes(op))request.path=params.get('path');
if(query){
 request.path='';
 request.dest='';
 request.inputs=[];
}
let finished=false;
try{
 window.obzBusy=true;
 $('fields').disabled=true;
 $('run').disabled=true;
 $('tool-back').disabled=true;
 $('result').inert=true;
 if(query)show('run',false);
 $('run').setAttribute('aria-busy','true');
 $('result').replaceChildren();
 $('status').className='';
 $('status').textContent=query?'正在读取…':'开始处理 · 进度待定';
 const progress=$('progress');
 QingProgress.render(progress,null);
 show('progress',!query);
 if(op.startsWith('shell-'))request.path=await invoke('current_executable');
 active=await invoke('start_operation',{request});
 const cancellable=cancellableOperations.has(op);
 $('cancel').disabled=!cancellable;
 show('cancel',cancellable);
 if(!archiveScope)$('password').value='';
 $('candidates').value='';
 let measured;
 while(true){
  const snapshot=await invoke('wait_operation',{id:active});
  if(snapshot.status!=='running'){
   if(snapshot.status==='complete'){
    render(op,snapshot.result);
    finished=!query&&QingProgress.complete(snapshot);
    QingProgress.render(progress,finished?100:null);
    $('status').textContent=query?'':finished?'已完成 · 100%':'已返回结果，请查看详情';
   }else if(snapshot.status==='cancelled'){
    $('status').textContent='已取消';
   }else{
    throw new Error(snapshot.error||'任务失败');
   }
   break;
  }
  if(query)continue;
  measured=QingProgress.sample(snapshot.progress,measured);
  QingProgress.render(progress,measured.value);
  $('status').textContent=measured.value===null?'处理中 · 进度待定':`处理中 · 本阶段 ${measured.value}%`;
 }
}catch(err){
 $('status').textContent=String(err);
 $('status').className='error';
 if(query)show('run',true);
 if(op==='preview'&&archivePreviewImages!==null)$('archive-image').value='';
}finally{
 const id=active;
 active=null;
 if(id!==null)await invoke('forget_operation',{id}).catch(()=>{});
 window.obzBusy=false;
 $('fields').disabled=false;
 $('run').disabled=false;
 $('tool-back').disabled=false;
 $('result').inert=false;
 $('run').removeAttribute('aria-busy');
 $('cancel').disabled=true;
 show('cancel',false);
 show('progress',finished);
}
}

if (window.parent !== window) {
 document.documentElement.style.colorScheme = window.parent.document.documentElement.style.colorScheme;
}
if(params.has('path')){$('path').value=params.get('path');$('checksum-paths').value=params.get('path');}if(params.has('codepage'))$('codepage').value=params.get('codepage');
if(window.parent!==window){document.querySelector('h1').hidden=true;document.querySelector('body>p').hidden=true;document.body.style.padding='12px';}

$('checksum-browse').onclick=async()=>{
 const picked=await invoke('pick_files_for_zip');
 if(picked.length)$('checksum-paths').value=[...new Set([...lines('checksum-paths'),...picked])].join('\n');
};
$('hash-algorithm').onchange=()=>show('hash-choices',$('hash-algorithm').value==='custom');

$('checksum-folder').onclick=async()=>{const picked=await invoke('pick_folder');if(picked)$('checksum-paths').value=[...new Set([...lines('checksum-paths'),picked])].join('\n');};

if(initialOperation==='preview'&&params.has('name'))$('name').value=params.get('name');
const defaultOperation=hasInitialOperation?initialOperation:'list';
if(archiveScope){
 if(initialOperation==='preview'){
  $('archive-image').onchange=()=>{
   const name=$('archive-image').value;
   if(name)return runOperation('preview',name);
   $('result').replaceChildren();
   $('status').textContent='';
   $('status').className='';
   show('progress',false);
  };
  window.setArchiveToolImages=names=>{
   if(!Array.isArray(names))return;
   const picker=$('archive-image');
   const initialized=archivePreviewImages!==null;
   const previous=picker.value||$('name').value;
   archivePreviewImages=[...new Set(names.filter(name=>typeof name==='string'&&name))];
   const options=document.createDocumentFragment();
   options.append(new Option('选择图片…',''));
   for(const name of archivePreviewImages)options.append(new Option(name,name));
   picker.replaceChildren(options);
   if(archivePreviewImages.includes(previous))picker.value=previous;
   updateOperation();
   if(!initialized&&picker.value)void runOperation('preview',picker.value);
  };
 }
 window.setArchiveToolPassword=password=>{
  if(typeof password==='string'&&!$('password').value&&!$('password-group').classList.contains('hidden')){
   $('password').value=password;
  }
 };
}
passwordBook.mountPickers($('password-group'));
void selectOperation(defaultOperation);
