const QingProgress=(()=>{
  function sample(progress, previous){
    const {phase,percent,message,work,deep}=progress;
    let value=null;
    let source='';
    let total=null;
    if(!['complete','recycling'].includes(phase)){
      if(Number.isFinite(work?.bytes_total) && work.bytes_total>0 && Number.isFinite(work.bytes_done)){
        source='bytes';value=work.bytes_done;total=work.bytes_total;
      }else if(Number.isFinite(work?.items_total) && work.items_total>0 && Number.isFinite(work.items_done)){
        source='items';value=work.items_done;total=work.items_total;
      }else if(Number.isFinite(percent) && !(phase==='listing' && message==='Identifying archive directory')){
        source='native';value=percent;total=100;
      }
    }
    const key=JSON.stringify([phase, deep?.archive, deep?.depth, source, total]);
    if(value!==null)value=Math.max(0,Math.min(99,Math.floor(100*value/total)));
    if(previous?.key===key && previous.value!==null)value=Math.max(previous.value,value??0);
    return {key,value};
  }
  function complete(job){
    return job.status==='complete' && !job.partial && !job.deep?.cancelled &&
      (!job.deep || (job.deep.finished && !job.deep.recovery_path)) &&
      !(job.result?.valid===false || job.result?.partial || job.result?.cancelled);
  }
  function value(job){
    const successful=complete(job);
    if(!successful && (job.status!=='running' || job.cancelRequested))return null;
    if(job.overall){
      const {done,total,active=0}=job.overall;
      const completed=done+(successful?0:active/100);
      return Math.max(0,Math.min(successful?100:99,Math.floor(100*completed/total)));
    }
    return successful?100:job.percent??null;
  }
  function observeOperation(job){
    if(!job.overall||job.status!=='running'||!Number.isFinite(job.percent))return;
    const operation=job.archives?(job.stage==='verify'?'test':'create'):job.operation;
    const phases={create:['compressing'],test:['testing'],extract:['decoding','decompressing','extracting']};
    if(phases[operation]?.includes(job.phase)){
      job.overall.active=Math.max(job.overall.active??0,job.percent);
    }
  }
  function activity(job){
    return !!(job.overall&&job.status==='running'&&!job.cancelRequested&&job.percent==null);
  }
  function label(job, amount){
    if(amount==null)return job.cancelRequested?'正在取消':'进度待定';
    if(job.overall)return `总进度（步骤） ${job.overall.done}/${job.overall.total} · ${amount}%`;
    return job.status==='running'?`本阶段 ${amount}%`:`${amount}%`;
  }
  function render(node, amount, active=false){
    if(node.hasAttribute('data-progress-activity')!==active)node.toggleAttribute('data-progress-activity',active);
    const current=node.getAttribute('aria-valuenow');
    if(amount===null){
      if(current!==null){
        node.removeAttribute('aria-valuenow');
        node.style.removeProperty('--qa-progress-value');
      }
    }else if(current!==String(amount)){
      node.setAttribute('aria-valuenow',String(amount));
      node.style.setProperty('--qa-progress-value',amount+'%');
    }
  }
  function markup(attributes, amount=null, active=false){
    const measured=amount===null?'':` aria-valuenow="${amount}" style="--qa-progress-value:${amount}%"`;
    return `<div class="qa-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" ${attributes}${measured}${active?' data-progress-activity':''}><span></span></div>`;
  }
  return {sample,value,complete,observeOperation,activity,label,render,markup};
})();
if(typeof module!=='undefined')module.exports=QingProgress;
