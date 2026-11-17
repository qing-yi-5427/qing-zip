(function(scope){
'use strict';
const definitions=[['name','名称',280],['packed_size','压缩后',104],['size','大小',104],['type','类型',94],['modified','修改日期',160],['created','创建日期',160],['accessed','访问日期',160],['crc','CRC',110],['method','压缩方法',150],['encrypted','加密',90],['attributes','属性',120],['comment','注释',200]].map(([id,label,width],i)=>({id,label,width,visible:i<5}));
function normalize(value){const result=[],seen=new Set();for(const item of Array.isArray(value)?value:[]){const def=definitions.find(d=>d.id===item?.id);if(!def||seen.has(def.id))continue;seen.add(def.id);result.push({...def,visible:def.id==='name'||item.visible===true,width:Math.max(64,Math.min(800,Number.isFinite(Number(item.width))?Math.round(Number(item.width)):def.width))});}for(const d of definitions)if(!seen.has(d.id))result.push({...d});return result;}
function packedKnown(e){return e.packed_size>0||(e.size_known!==false&&e.size===0);}
function move(columns,id,delta){const out=normalize(columns),i=out.findIndex(c=>c.id===id),j=i+delta;if(i>=0&&j>=0&&j<out.length)[out[i],out[j]]=[out[j],out[i]];return out;}
const api={definitions,normalize,packedKnown,move};if(typeof module!=='undefined')module.exports=api;else scope.ArchiveColumns=api;
})(globalThis);
