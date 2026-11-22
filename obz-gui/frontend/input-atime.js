(function(global){'use strict';
let preserveAtime;
function preservationNotice(result){
 const report=result?.input_atime;if(!report)return '';
 const notes=[];
 if(report.unavailable_handles>0)notes.push('部分来源文件未能启用访问时间保护。');
 if(report.unconfirmed_sources>0||report.backend_best_effort)notes.push('部分来源文件的访问时间保护无法逐项确认。');
 return notes.join('');
}
function preservePreference(value){
 if(typeof value==='boolean'){
  preserveAtime=value;
  try{global.localStorage.setItem('qing-add-preserve-atime',JSON.stringify(value));}catch{}
 }else if(preserveAtime===undefined){
  try{preserveAtime=JSON.parse(global.localStorage.getItem('qing-add-preserve-atime'))===true;}catch{preserveAtime=false;}
 }
 return preserveAtime;
}
const api={preservationNotice,preservePreference};if(typeof module!=='undefined'&&module.exports)module.exports=api;global.QingInputAtime=api;
})(typeof window!=='undefined'?window:globalThis);
