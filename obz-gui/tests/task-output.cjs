const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

test('clicking a task result reveals that task output, including numeric native job IDs', async () => {
  const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
  const handler = source.split('\n').find(line => line.startsWith("root.addEventListener('click',async") && line.includes('dataset.jobOutput'));
  assert.ok(handler);
  let click;
  const calls = [];
  const jobs = [
    {id: 7, outputPath: 'D:\\extracted'},
    {id: 8, outputPath: 'D:\\new archive.zip'}
  ];
  vm.runInNewContext(handler, {
    root: {addEventListener: (_, fn) => { click = fn; }},
    s: {jobs}, invoke: (command, args) => calls.push([command, args.path]),
    error: e => { throw e; }
  });
  for (const id of ['8', '7', '42']) {
    await click({target: {closest: () => ({dataset: {jobOutput: id}})}});
  }
  assert.deepEqual(calls, [['reveal_path', jobs[1].outputPath], ['reveal_path', jobs[0].outputPath]]);
});

test('listing results belong to the caller and are not retained in task history', async () => {
  const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
  const run = source.split('\n').find(line => line.startsWith('async function run('));
  const result = {entries: [{name: 'member.txt'}]};
  const state = {jobs: []};
  const context = vm.createContext({
    s: state,
    invoke: async command => command === 'wait_operation' ? {status:'complete', progress:{}, result} : 1
  });
  vm.runInContext(run, context);
  assert.equal(await context.run({operation:'list'}, 'list', {silent:true}), result);
  assert.equal(state.jobs.length, 1);
  assert.equal(state.jobs[0].status, 'complete');
  assert.ok(!JSON.stringify(state.jobs).includes('member.txt'));
});
