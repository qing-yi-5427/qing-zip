(()=>{'use strict';
function compressionLevels(format,method=''){
 if(format==='lzh')return [[0,'仅存储'],[5,'标准（推荐）']];
 const key=(method||format).toLowerCase(),levels=key==='lizard'?[10,19,29,49]:key==='lz4'?[1,5,9,12]:key==='lz5'?[1,5,9,15]:key==='brotli'?[0,1,5,9,11]:[0,1,5,9],extended=['zstd','brotli','lz4','lz5','lizard','kanzi'].includes(key);
 if(key==='lizard')return levels.map(value=>[value,value===10?'10 · 默认':value===49?'49 · 最大压缩':String(value)]);
 return levels.map((value,index)=>[value,`${value} · ${index===levels.length-1?'最大压缩':value===0&&!extended?'仅存储':value===5?'标准（推荐）':index<2?'快速':'高压缩'}`]);
}
function wordRange(format,method=''){
 if(!['zip','7z','sfx'].includes(format))return null;
 const name=(method||(format==='zip'?'Deflate':'LZMA2')).toLowerCase();
 if(['lzma','lzma2'].includes(name))return [5,273];
 if(format==='zip'&&name==='deflate')return [3,258];
 if(format==='zip'&&name==='deflate64')return [3,257];
 if(name==='ppmd')return [2,format==='zip'?16:32];
 return null;
}
function timeSupported(format){return ['zip','7z','sfx'].includes(format);}
function levelSupported(format){return !['iso','tar'].includes(format);}
function preferredLevel(format,method,value){
 const levels=compressionLevels(format,method);
 const current=value===''||value==null?NaN:Number(value);
 const fallback=(method||format).toLowerCase()==='lizard'?10:5;
 return levels.some(([level])=>level===current)?current:fallback;
}
function profileLevel(format,value,levelVersion){
 if(!levelSupported(format))return undefined;
 if(levelVersion!==2){
  // The previous editor offered 0/1/5/9 for every format, with 9 labeled "maximum".
  const levels=compressionLevels(format);
  if(Number(value)===9)return levels.at(-1)[0];
  if(format==='lizard'&&[0,1].includes(Number(value)))return levels[0][0];
 }
 return preferredLevel(format,'',value);
}
const rawFormats=['zstd','brotli','lz4','lz5','lizard'];
function singleFile(format){return ['gzip','bzip2','xz',...rawFormats].includes(format);}
function volumeSupported(format){return !['tgz','tbz2','txz','iso','lzh',...rawFormats].includes(format);}
// A named encoder overrides -mx in 7-Zip. Keep the two controls in
// agreement so that "only store" really produces uncompressed entries, and a
// compressing level never silently keeps the store method selected for 0,
// which stays hidden while the advanced settings are collapsed.
function reconcileStore(format,method,level,changed){
 const store={zip:'Store',zipx:'Store','7z':'Copy',sfx:'Copy'}[format];
 if(!store)return {method,level};
 if(changed==='level'){
  if(Number(level)===0)return {method:store,level:'0'};
  if(method===store)return {method:'',level};
 }
 if(changed==='method'){
  if(method===store)return {method,level:'0'};
  if(Number(level)===0)return {method,level:'5'};
 }
 return {method,level};
}
window.qingCompression=Object.freeze({levels:compressionLevels,wordRange,timeSupported,levelSupported,preferredLevel,profileLevel,singleFile,volumeSupported,reconcileStore});
})();
