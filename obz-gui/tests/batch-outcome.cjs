const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const functions = source.slice(source.indexOf('const batchPolicies='), source.indexOf('const conflicts='))
  + source.slice(source.indexOf('function batchResultMarkup'), source.indexOf('function updateBatchPage'));

function checkpoints({failSave=false}={}) {
  let count=0, record=null;
  return {
    get record(){return record;},
    async invoke(name,args) {
      if(name==='set_batch_resume_running')return;
      if(name==='load_batch_resume')return record?{state:'ready',record:structuredClone(record)}:{state:'empty'};
      if(name==='save_batch_resume') {
        if(failSave||args.previous!==(record?.revision??null))throw Error('record unavailable');
        record={revision:String(++count),plan:structuredClone(args.plan)};
        return structuredClone(record);
      }
      assert.equal(name,'take_batch_resume');
      if(args.revision!==record?.revision)return null;
      const selected=record;
      record=null;
      return structuredClone(selected);
    },
  };
}

function scenario(responses, operation='extract', store=checkpoints()) {
  const calls = [];
  const s = { jobs: [] };
  const modal = {};
  let renders=0;
  const context = vm.createContext({
    s, filename: path.win32.basename, btn: label => label,
    esc: value => String(value).replaceAll('<', '&lt;'), taskError: error => String(error),
    $: id => id==='oa-batch-resume-review'?modal.review:null,
    dialog(title,body,actions){Object.assign(modal,{title,body,actions});},
    showPreservationNotice(message){modal.notice=message;},
    close() {}, render() { renders++; }, renderProgress() {}, invoke:store.invoke,
    async run(request, label, { onStart, batch }) {
      calls.push(request);
      const response = responses.shift();
      if (response.startError) throw Error(response.startError);
      const job = { id: calls.length, ...response };
      s.jobs.push(job);
      onStart(job);
      if (response.stop) s.batchStop = true;
      if (job.status === 'complete' && !job.partial) batch.done++;
      if (job.status === 'cancelled') throw Error('已取消');
    },
  });
  vm.runInContext(functions, context);
  const entries = ['a.zip','b.zip','c.zip'].map((name,index) => ({
    path: name, index, status: 'pending', dest: context.batchExtractionTarget('D:\\out',name,index),
  }));
  const plan = { operation, entries, overwrite: 'error', destination: 'D:\\out' };
  return { calls, s, context, plan, store, modal, get renders(){return renders;} };
}

test('batch keeps startup failures and partially completed jobs visible after the final success', async () => {
  const { context, s, plan } = scenario([
    {startError: '<missing archive>'}, {status: 'complete', partial: true, summary: '部分输出'}, {status: 'complete', outputPath: 'out'},
  ]);
  await context.runBatch(plan);
  assert.equal(s.batchResult.summary, '成功 1 · 部分完成 1 · 失败 1');
  assert.equal(s.batchResult.lastJob, s.jobs.at(-1));
  const markup = context.batchResultMarkup(s.batchResult);
  assert.match(markup, /a.zip · 失败/);
  assert.match(markup, /&lt;missing archive>/);
  assert.match(markup, /b.zip · 部分完成/);
  assert.match(markup, /打开输出目录/);
  assert.doesNotMatch(markup, /继续未开始/);
});

test('resuming the tail keeps the original path, ordinal and policy and does not retry the cancelled item', async () => {
  const { calls, s, context, plan } = scenario([
    {status: 'complete'}, {status: 'cancelled', stop: true}, {status: 'complete', outputPath: 'out'},
  ]);
  await context.runBatch(plan);
  assert.equal(s.batchResult.summary, '成功 1 · 已取消 1 · 未开始 1');
  assert.match(context.batchResultMarkup(s.batchResult), /继续未开始的项目/);
  await context.useBatchResume(s.batchResult);
  assert.equal(s.batchResult.summary, '成功 2 · 已取消 1');
  assert.deepEqual(calls.map(request => request.path), ['a.zip','b.zip','c.zip']);
  assert.equal(calls[2].dest, 'D:\\out\\3-c');
  assert.equal(calls[2].options.overwrite, 'error');
});

test('a failed integrity check remains visible when its native task finished and returned invalid', async () => {
  const { context, s, plan } = scenario([
    {status: 'complete'}, {status: 'failed', error: '完整性检查未通过。'}, {status: 'complete'},
  ], 'test');
  await context.runBatch(plan);
  assert.equal(s.batchResult.summary, '成功 2 · 失败 1');
  assert.match(context.batchResultMarkup(s.batchResult), /完整性检查未通过/);
  assert.equal(s.batchResult.hasOutput, false);
});

test('a reopened batch is inert until chosen, and another stop keeps the remaining original destinations', async () => {
  const first=scenario([{status:'complete'}, {status:'cancelled',stop:true}]);
  first.plan.overwrite='rename';
  first.plan.entries.push({path:'d.zip',index:3,status:'pending',dest:'D:\\out\\4-d'});
  first.plan.password='must not persist';
  await first.context.runBatch(first.plan);
  assert.doesNotMatch(JSON.stringify(first.store.record),/password|must not persist|message/);
  const second=scenario([{status:'cancelled',stop:true}], 'extract', first.store);
  const competing=scenario([], 'extract', first.store);
  await second.context.loadBatchResume();
  await competing.context.loadBatchResume();
  assert.deepEqual(second.calls,[]);
  assert.match(second.context.batchResultsMarkup(), /待继续的批量解压/);
  assert.match(second.context.batchResultsMarkup(), /自动重命名/);
  await second.context.useBatchResume(second.s.savedBatch);
  assert.deepEqual(second.calls.map(request=>request.path),['c.zip']);
  await assert.rejects(competing.context.useBatchResume(competing.s.savedBatch),/其他窗口/);
  assert.deepEqual(competing.calls,[]);
  const third=scenario([{status:'complete',outputPath:'out'}], 'extract', first.store);
  await third.context.loadBatchResume();
  await third.context.useBatchResume(third.s.savedBatch);
  assert.equal(third.calls[0].path,'d.zip');
  assert.equal(third.calls[0].dest,'D:\\out\\4-d');
  assert.equal(third.calls[0].options.overwrite,'rename');
  assert.equal(third.s.batchResult.summary,'成功 2 · 已取消 2');
  assert.equal(first.store.record,null);
});

test('ordinary extraction stays usable if saving its unstarted projects fails', async () => {
  const {context,s,plan,calls}=scenario([
    {status:'complete'}, {status:'cancelled',stop:true}, {status:'complete'},
  ], 'extract', checkpoints({failSave:true}));
  await context.runBatch(plan);
  assert.match(context.batchResultMarkup(s.batchResult),/本次会话仍可继续/);
  await context.useBatchResume(s.batchResult);
  assert.deepEqual(calls.map(request=>request.path),['a.zip','b.zip','c.zip']);
});

test('checking other archives leaves the saved extraction available', async () => {
  const first=scenario([{status:'cancelled',stop:true}]);
  await first.context.runBatch(first.plan);
  const revision=first.store.record.revision;
  const check=scenario([{status:'complete'},{status:'complete'},{status:'complete'}],'test',first.store);
  await check.context.loadBatchResume();
  await check.context.runBatch(check.plan);
  assert.equal(first.store.record.revision,revision);
  assert.match(check.context.batchResultsMarkup(),/批量检查结果/);
  assert.match(check.context.batchResultsMarkup(),/待继续的批量解压/);
});

test('closing an active batch seals only the tail as eligible, and clearing it launches no job', async () => {
  const {context,s,store,plan,calls}=scenario([]);
  plan.entries[0].status='complete';
  plan.entries[1].status='running';
  s.activeBatchPlan=plan;
  assert.equal(await context.saveBatchBeforeExit(),true);
  assert.deepEqual(store.record.plan.entries.map(entry=>entry.status),['complete','interrupted','pending']);
  const reopened=scenario([], 'extract', store);
  await reopened.context.loadBatchResume();
  await reopened.context.useBatchResume(reopened.s.savedBatch,true);
  assert.equal(store.record,null);
  assert.deepEqual(calls,[]);
  assert.deepEqual(reopened.calls,[]);
});

test('reviewing a resumed replacement shows the current inputs and exact original destinations before claiming', async () => {
  const first=scenario([{status:'complete'},{status:'cancelled',stop:true}]);
  first.plan.overwrite='overwrite';
  await first.context.runBatch(first.plan);
  const reopened=scenario([{status:'complete'}], 'extract', first.store);
  await reopened.context.loadBatchResume();
  const {context, s, calls, modal}=reopened;
  context.showBatchResumeReview(s.savedBatch);
  assert.match(modal.body,/同名文件 · 替换已有文件/);
  assert.ok(modal.body.includes('压缩包 · c.zip') && modal.body.includes('输出目录 · D:\\out\\3-c'));
  assert.match(modal.body,/当前压缩包/);
  assert.match(modal.actions,/按替换策略继续/);
  assert.deepEqual(calls,[]);
  assert.equal(first.store.record.revision,s.savedBatch.revision);
  modal.review={dataset:{revision:'stale'}};
  await assert.rejects(context.confirmBatchResume(),/重新查看/);
  assert.deepEqual(calls,[]);
  modal.review.dataset.revision=s.savedBatch.revision;
  await context.confirmBatchResume();
  assert.equal(calls.length,1);
  assert.equal(calls[0].options.overwrite,'overwrite');
});

test('damaged records offer an explicit backup while unfamiliar versions remain untouched', async () => {
  let state='unsupported', backedUp=false;
  const store={async invoke(name) {
    if(name==='load_batch_resume')return {state};
    assert.equal(name,'backup_damaged_batch_resume');
    assert.equal(state,'damaged');
    backedUp=true;
    state='empty';
    return 'damaged-copy.json';
  }};
  const {context,modal}=scenario([], 'extract', store);
  await context.loadBatchResume();
  assert.match(context.batchResultsMarkup(),/此版本无法识别/);
  assert.doesNotMatch(context.batchResultsMarkup(),/备份损坏记录/);
  state='damaged';
  await context.loadBatchResume();
  assert.match(context.batchResultsMarkup(),/备份损坏记录/);
  assert.equal(backedUp,false);
  await context.backupDamagedBatchResume();
  assert.match(modal.notice,/damaged-copy.json/);
  assert.doesNotMatch(context.batchResultsMarkup(),/备份损坏记录/);
});

test('a close during a delayed seal waits for the one durable write and leaves the next archive pending', async () => {
  let finish;
  let saving=0;
  const base=checkpoints();
  const lifecycle=[];
  const delayed=new Promise(resolve=>{finish=resolve;});
  const store={...base,async invoke(name,args){
    if(name==='set_batch_resume_running')lifecycle.push(args.active);
    if(name==='save_batch_resume'){saving++;await delayed;}
    return base.invoke(name,args);
  }};
  const {context,plan,calls}=scenario([{status:'complete'},{status:'cancelled',stop:true}], 'extract',store);
  const batch=context.runBatch(plan);
  while(!saving)await new Promise(resolve=>setImmediate(resolve));
  const close=context.saveBatchBeforeExit();
  assert.deepEqual(lifecycle,[true]);
  assert.equal(saving,1);
  assert.equal(calls.length,2);
  finish();
  assert.equal(await close,true);
  await batch;
  assert.deepEqual(lifecycle,[true,false]);
  assert.equal(saving,1);
  assert.deepEqual(base.record.plan.entries.map(entry=>entry.status),['complete','cancelled','pending']);
});

test('normal exit while claiming a paused queue does not dispatch another archive', async () => {
  const first=scenario([{status:'cancelled',stop:true}]);
  await first.context.runBatch(first.plan);
  let release;
  let claiming=false;
  const paused=new Promise(resolve=>{release=resolve;});
  const store={async invoke(name,args) {
    if(name==='take_batch_resume'){claiming=true;await paused;}
    return first.store.invoke(name,args);
  }};
  const reopened=scenario([{status:'complete'}], 'extract', store);
  await reopened.context.loadBatchResume();
  const resume=reopened.context.useBatchResume(reopened.s.savedBatch);
  while(!claiming)await new Promise(resolve=>setImmediate(resolve));
  const exit=reopened.context.saveBatchBeforeExit();
  release();
  assert.equal(await exit,true);
  await resume;
  assert.deepEqual(reopened.calls,[]);
  assert.deepEqual(first.store.record.plan.entries.map(entry=>entry.status),['cancelled','pending','pending']);
});

test('an unchanged pending-record read leaves existing task history nodes intact', async () => {
  const current=scenario([]);
  current.s.page='history';
  await current.context.loadBatchResume();
  assert.equal(current.renders,0);
  const paused=scenario([{status:'cancelled',stop:true}], 'extract', current.store);
  await paused.context.runBatch(paused.plan);
  await current.context.loadBatchResume();
  assert.equal(current.renders,1);
  assert.match(current.context.batchResultsMarkup(),/待继续的批量解压/);
  await current.context.loadBatchResume();
  assert.equal(current.renders,1);
});
