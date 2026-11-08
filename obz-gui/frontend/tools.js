const invoke=(window.__TAURI__||window.parent.__TAURI__).core.invoke,$=id=>document.getElementById(id),lines=id=>$(id).value.split(/\r?\n/).filter(s=>s.length>0);let active=null;
const show=(id,yes)=>$(id).classList.toggle('hidden',!yes);const fileOps=['list','extract','test','add','delete','rename','preview','repair','recover','checksum'];
function update(){const op=$('operation').value,creating=op==='create',vault=op.startsWith('vault');show('path-group',op!=='checksum'&&(fileOps.includes(op)||creating));show('checksum-paths-group',op==='checksum');$('path-label').textContent=creating?'新压缩包保存路径':'压缩包路径';show('dest-group',['extract','repair'].includes(op));$('dest-label').textContent=op==='repair'?'修复后另存为（必须是新文件）':'解压到';show('inputs-group',['create','add','delete','rename'].includes(op));$('inputs-label').textContent=op==='rename'?'旧名称、新名称交替填写，每行一个（如：旧名.txt 换行 新名.txt）':op==='delete'?'要删除的包内完整名称（每行一个）':'输入文件（每行一个完整路径）';show('files',['create','add'].includes(op));show('folder',['create','add'].includes(op));show('format-group',creating);show('hash-group',op==='checksum');if(op==='checksum')$('path-label').textContent='文件路径';show('overwrite-group',op==='extract');show('selected-group',op==='extract');show('name-group',op==='preview'||['vault-put','vault-delete'].includes(op));$('name-label').textContent=vault?'密码 ID':'图片在包内的完整名称';show('label-group',op==='vault-put');show('password-group',!['capabilities','checksum','recover','vault-list','vault-delete','shell-enable','shell-disable'].includes(op));show('password-id-group',!vault&&!['capabilities','checksum','recover','shell-enable','shell-disable'].includes(op));show('recovery-group',op==='recover');show('advanced',!vault&&['list','extract','test','add','create','preview','delete','rename','repair'].includes(op));$('hint').textContent=op==='repair'?'修复仅恢复能验证的 ZIP 条目；原文件保留不变，不能保证恢复全部内容。':op==='recover'?'按候选顺序进行有上限的尝试；并非保证成功的密码破解。':op==='shell-enable'?'仅增加当前用户的“使用 qing-zip 打开”右键入口，不修改默认打开方式。':creating?'新建操作不会覆盖已有压缩包。GZip、XZ、BZip2 以及 Brotli、LZ4、LZ5、Lizard、Zstandard 单流格式只能直接输入一个文件。':'';}
function updateFormat(){const creating=$('operation').value==='create',format=$('format').value,raw=['zstd','brotli','lz4','lz5','lizard'].includes(format),previous=$('level').value,levels=window.qingCompression.levels(format);$('level').replaceChildren(new Option('默认',''),...levels.map(([value,label])=>new Option(label,String(value))));$('level').value=levels.some(([value])=>String(value)===previous)?previous:'';$('volume').disabled=creating&&raw;if(creating){const encrypted=['zip','zipx','7z','sfx'].includes(format);show('password-group',encrypted);show('password-id-group',encrypted);}}
const updateOperation=()=>{update();updateFormat();};$('operation').onchange=updateOperation;$('format').onchange=updateOperation;updateOperation();
$('browse').onclick=async()=>{if($('operation').value==='create'){$('path').value=await invoke('pick_archive_save',{format:$('format').value})||$('path').value;}else{const f=await invoke('pick_files');if(f.length)$('path').value=f[0];}};
$('browse-dest').onclick=async()=>{$('dest').value=await invoke($('operation').value==='repair'?'pick_save':'pick_folder')||$('dest').value;};
$('files').onclick=async()=>{const f=await invoke('pick_files_for_zip');if(f.length)$('inputs').value=[$('inputs').value,...f].filter(Boolean).join('\n');};
$('folder').onclick=async()=>{const p=await invoke('pick_folder');if(p)$('inputs').value=[$('inputs').value,p].filter(Boolean).join('\n');};
function text(content,tag='div'){const el=document.createElement(tag);el.textContent=content;$('result').append(el);return el;}
function render(op,result){$('result').replaceChildren();if(result.data_url){const img=document.createElement('img');img.id='preview';img.src=result.data_url;img.alt=result.name;$('result').append(img);return;}
if(Array.isArray(result.entries)){text(`${result.entries.length} 个条目`);const table=document.createElement('table');const head=document.createElement('tr');for(const s of ['名称','字节','加密','修改时间']){const h=document.createElement('th');h.textContent=s;head.append(h);}table.append(head);for(const e of result.entries){const tr=document.createElement('tr');for(const v of [e.name,e.is_dir?'目录':e.size_known===false?'未知':e.size,e.encrypted?'是':'否',e.modified]){const td=document.createElement('td');td.textContent=v;tr.append(td);}table.append(tr);}$('result').append(table);return;}
if(Array.isArray(result.checksums)){
 const output=result.checksums.map(file=>{
   const hashes=file.hashes||Object.fromEntries(['crc32','md5','sha1','sha256'].map(k=>[k.toUpperCase(),file[k]]));
   return [file.path,...Object.entries(hashes).map(([name,value])=>name+': '+value)].join('\n');
 });
 text(output.join('\n\n'),'pre').className='checksum-results';
 return;
}
if(result.hashes){for(const [name,value] of Object.entries(result.hashes))text(name+': '+value);}else if(result.sha256){for(const k of ['crc32','md5','sha1','sha256'])text(k.toUpperCase()+': '+result[k]);}else if(result.valid)text('完整性检查通过。');else if(result.files!==undefined)text(`已解压 ${result.files} 个文件 → ${result.destination}`);else if(result.outputs)result.outputs.forEach(p=>text('已创建：'+p));else if(result.updated)text('已更新：'+result.updated);else if(result.recovered){text(`恢复 ${result.recovered.length} 项 → ${result.output}`);text(result.complete?'恢复完成。':'部分恢复；原包中的其他内容可能无法恢复。');result.skipped.forEach(s=>text('未恢复：'+s));}else if(op==='recover'){text(`已尝试 ${result.attempts} 次 · ${{found:'找到密码',exhausted:'候选已用尽',limit_reached:'达到尝试上限',cancelled:'已取消'}[result.status]||result.status}`);if(result.password!==null){const input=document.createElement('input');input.type='password';input.readOnly=true;input.value=result.password;input.className='secret';$('result').append(input);const b=text('显示 / 隐藏密码','button');b.type='button';b.onclick=()=>input.type=input.type==='password'?'text':'password';}}else if(Array.isArray(result)){if(!result.length)text('没有保存的密码。');result.forEach(e=>text(`${e.id} · ${e.label}`));}else if(result.saved)text('密码已加密保存：'+result.saved);else if(result.removed)text('密码条目已删除：'+result.removed);else if(result.enabled!==undefined)text(result.enabled?'右键入口已启用。':'右键入口已移除。');else if(result.engine){text('可读取：'+result.engine.read_formats.join('、'));text('可创建：'+result.engine.write_formats.join('、'));text('引擎状态：'+(result.engine.available?'可用':'缺失'));result.engine.limitations.forEach(s=>text(s));}else text('操作完成。');}
$('cancel').onclick=async()=>{if(active!==null){await invoke('cancel_operation',{id:active});$('status').textContent='正在取消…';$('cancel').disabled=true;}};
$('form').onsubmit=async event=>{event.preventDefault();const op=$('operation').value;const options={};if(!$('password-group').classList.contains('hidden')&&$('password').value)options.password=$('password').value;if(!$('advanced').classList.contains('hidden')){for(const id of ['level','threads','codepage'])if($(id).value)options[id]=Number($(id).value);if(!$('volume').disabled&&$('volume').value)options.volume_bytes=Math.floor(Number($('volume').value)*1048576);}if(op==='extract'){options.selected=lines('selected');options.overwrite=$('overwrite').value;}const request={operation:op,path:$('path').value,dest:$('dest').value,format:$('format').value,inputs:lines('inputs'),options,name:$('name').value,label:$('label').value,smart:$('smart').checked,candidates:op==='recover'?lines('candidates'):[],max_attempts:Number($('max-attempts').value)||1000};if(!$('password-id-group').classList.contains('hidden')&&$('password-id').value)request.password_id=$('password-id').value;
if(op==='checksum'){
 const paths=lines('checksum-paths'),algorithm=$('hash-algorithm').value;
 if(!paths.length){$('status').textContent='请选择待校验文件。';return;}
 request.path=paths[0];request.inputs=[];
 if(paths.length>1){request.operation='checksum-files';request.inputs=paths;}
 if(algorithm==='custom'){
   request.hashes=[...$('hash-choices').querySelectorAll('input:checked')].map(input=>input.value);
   if(!request.hashes.length){$('status').textContent='请至少选择一种校验算法。';return;}
 }else if(algorithm){request.hashes=[algorithm];}
}
if(op==='recover'&&$('charset').value){request.charset=$('charset').value;request.min_length=Number($('min-length').value);request.max_length=Number($('max-length').value);}
if(['delete','vault-delete','shell-enable','shell-disable'].includes(op)&&!confirm('确认执行“'+$('operation').selectedOptions[0].textContent+'”？'))return;
try{if(op.startsWith('shell-'))request.path=await invoke('current_executable');window.obzBusy=true;$('fields').disabled=true;$('run').disabled=true;$('result').replaceChildren();$('status').className='';$('status').textContent='开始处理…';show('progress',true);$('progress').removeAttribute('value');active=await invoke('start_operation',{request});$('cancel').disabled=!['list','extract','create','test','add','delete','rename','preview','recover','checksum'].includes(op);$('password').value='';$('candidates').value='';while(true){const s=await invoke('wait_operation',{id:active});if(s.progress.percent!==null)$('progress').value=s.progress.percent;else $('progress').removeAttribute('value');if(s.status!=='running'){if(s.status==='complete'){render(op,s.result);$('status').textContent='已完成';}else{throw new Error(s.error||'任务已取消');}break;}$('status').textContent=s.progress.percent!==null?`处理中 · ${s.progress.percent}%`:'处理中…';}}catch(err){$('status').textContent=String(err);$('status').className='error';}finally{if(active!==null){await invoke('forget_operation',{id:active}).catch(()=>{});active=null;}window.obzBusy=false;$('fields').disabled=false;$('run').disabled=false;$('cancel').disabled=true;show('progress',false);}};
document.documentElement.style.colorScheme=window.parent!==window?window.parent.document.getElementById('qing-archive').style.colorScheme:'light';
const params=new URLSearchParams(location.search);if(params.has('operation'))$('operation').value=params.get('operation');if(params.has('path')){$('path').value=params.get('path');$('checksum-paths').value=params.get('path');}if(params.has('codepage'))$('codepage').value=params.get('codepage');updateOperation();
if(window.parent!==window){document.querySelector('h1').hidden=true;document.querySelector('body>p').hidden=true;document.body.style.padding='12px';}

$('checksum-browse').onclick=async()=>{
 const picked=await invoke('pick_files_for_zip');
 if(picked.length)$('checksum-paths').value=[...new Set([...lines('checksum-paths'),...picked])].join('\n');
};
$('hash-algorithm').onchange=()=>show('hash-choices',$('hash-algorithm').value==='custom');
invoke('checksum_algorithms').then(algorithms=>{
 const select=$('hash-algorithm');
 select.add(new Option('自选多项…','custom'));
 select.add(new Option('全部可用算法','*'));
 for(const {name} of algorithms){
   select.add(new Option(name,name));
   const label=document.createElement('label'),input=document.createElement('input');
   input.type='checkbox';input.value=name;
   label.append(input,document.createTextNode(name));$('hash-choices').append(label);
 }
}).catch(()=>{});
