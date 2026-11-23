const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const functions = source.slice(source.indexOf('function batchResultMarkup'), source.indexOf('function updateBatchPage'));

function scenario(responses, operation='extract') {
  const calls = [];
  const s = { jobs: [] };
  const context = vm.createContext({
    s, filename: path.win32.basename, btn: label => label,
    esc: value => String(value).replaceAll('<', '&lt;'), taskError: error => String(error),
    $: () => null, close() {}, render() {}, renderProgress() {},
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
  return { calls, s, context, plan };
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
  await context.runBatch(s.batchResult);
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
