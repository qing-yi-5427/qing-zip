(()=>{'use strict';
function compressionLevels(format,method=''){const key=(method||format).toLowerCase(),levels=key==='lizard'?[10,19,29,49]:key==='lz4'?[1,5,9,12]:key==='lz5'?[1,5,9,15]:key==='brotli'?[0,1,5,9,11]:[0,1,5,9],extended=['zstd','brotli','lz4','lz5','lizard','kanzi'].includes(key);if(key==='lizard')return levels.map(value=>[value,value===10?'10 · 默认':value===49?'49 · 最大压缩':String(value)]);return levels.map((value,index)=>[value,`${value} · ${index===levels.length-1?'最大压缩':value===0&&!extended?'仅存储':value===5||key==='lizard'&&value===10?'标准（推荐）':index<2?'快速':'高压缩'}`]);}
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
const rawFormats=['zstd','brotli','lz4','lz5','lizard'];
function singleFile(format){return ['gzip','bzip2','xz',...rawFormats].includes(format);}
function volumeSupported(format){return !['tgz','tbz2','txz','iso','lzh',...rawFormats].includes(format);}
window.qingCompression=Object.freeze({levels:compressionLevels,wordRange,timeSupported,singleFile,volumeSupported});
})();
