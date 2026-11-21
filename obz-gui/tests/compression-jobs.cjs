const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const start = source.indexOf('let nextCompressionJob=');
const end = source.indexOf('function forgetRecycledArchives(', start);
const QingProgress = require('../frontend/progress.js');
const input = (name, options = {}) => ({operation: 'create', path: `C:\\out\\${name}.zip`, inputs: [`C:\\in\\${name}`], options});
const done = result => ({status: 'complete', result});

function harness(respond) {
  const calls = [], frames = [], passwords = [{value: 'secret'}, {value: 'secret'}];
  const draft = {querySelectorAll: () => passwords};
  const s = {jobs: []};
  let request, id = 0;
  const ui = vm.createContext({
    s, performance, overlay: {querySelector: () => draft}, tabBusy: () => Boolean(s.active),
    closingArchiveTabs: false, finishingArchiveTabs: false, removingArchiveTabs: false,
    close() {}, updateToolbar() {}, renderHistoryJob() {}, schedulePendingQueue() {}, taskError: String,
    QingFileWorkspace: {preservationNotice: () => ''},
    renderInlineTask() {
      const job = s.jobs.at(-1);
      frames.push({stage: job.stage, index: job.current, percent: job.percent, overall:QingProgress.value(job), done:job.overall?.done, background: s.backgroundExtraction});
    },
    updateJobProgress(job, snapshot) { job.percent = snapshot.progress.percent; },
    async invoke(command, args) {
      if (command === 'start_operation') {
        request = args.request;
        calls.push(request);
        return ++id;
      }
      if (command === 'wait_operation') return respond(request, ui);
    }
  });
  vm.runInContext(source.slice(start, end), ui);
  return {ui, s, calls, frames, passwords};
}

test('batch has one inline job with native stage progress and per-archive verified results', async () => {
  const seen = new Set();
  const {ui, s, calls, frames} = harness(request => {
    const key = request.operation + request.path;
    if (!seen.has(key)) {
      seen.add(key);
      return {status: 'running', progress: {percent: 37}};
    }
    return done(request.operation === 'create' ? {outputs: [request.path]} : {valid: true});
  });
  await ui.runCompression([input('one'), input('two')], true);
  assert.deepEqual(calls.map(call => call.operation), ['create', 'test', 'create', 'test']);
  assert.equal(s.jobs.length, 1);
  assert.equal(s.jobs[0].status, 'complete');
  assert.deepEqual(Array.from(s.jobs[0].archives, archive => archive.state), ['verified', 'verified']);
  assert.ok(frames.some(frame => frame.stage === 'verify' && frame.index === 1 && frame.percent === 37 && frame.background));
  assert.ok(frames.some(frame => frame.stage === 'verify' && frame.percent === null));
});

test('cancelling verification preserves created volumes and rechecking never recreates them', async () => {
  let cancelVerification = true;
  const {ui, s, calls} = harness(request => {
    if (request.operation === 'create') return done({outputs: [request.path + '.001', request.path + '.002']});
    return cancelVerification ? {status: 'cancelled'} : done({valid: true});
  });
  await ui.runCompression([input('one'), input('two')], true);
  const job = s.jobs[0];
  assert.equal(job.status, 'cancelled');
  assert.deepEqual(Array.from(job.archives, archive => archive.state), ['unverified', 'pending']);
  assert.equal(job.archives[0].outputs.length, 2);
  cancelVerification = false;
  await ui.recheckCompressionOutput(job, 0);
  assert.equal(job.archives[0].state, 'verified');
  assert.deepEqual(calls.map(call => call.operation), ['create', 'test', 'test']);
  assert.ok(calls.slice(1).every(request => request.path.endsWith('.zip.001')));
});

test('failed batch preserves only unproduced inputs and clears stored passwords', async () => {
  const {ui, s, passwords} = harness(request => request.path.endsWith('two.zip')
    ? {status: 'failed', error: 'output exists'} : done({outputs: [request.path]}));
  await ui.runCompression(['one', 'two', 'three'].map(name => input(name, {password: 'secret'})), false);
  assert.equal(s.jobs[0].status, 'failed');
  assert.deepEqual(Array.from(s.jobs[0].archives, archive => archive.state), ['created', 'createFailed', 'pending']);
  const retry = vm.runInContext('compressionDrafts.get(s.jobs[0])', ui);
  assert.deepEqual(Array.from(retry.inputs), ['C:\\in\\two', 'C:\\in\\three']);
  assert.equal(retry.hadPassword, true);
  assert.ok(passwords.every(input => input.value === ''));
  assert.doesNotMatch(JSON.stringify(s.jobs), /secret/);
});

test('result buttons resolve their own archive, including after a later job has finished', async () => {
  const first = input('first');
  const second = input('second');
  const {ui, s} = harness(request => done({outputs: [request.path]}));
  const events = [];
  ui.openArchive = async p => { events.push(['open', p]); throw Error('file no longer exists'); };
  ui.openDialog = p => events.push(['dialog', p]);
  ui.error = e => events.push(['error', e.message]);
  await ui.runCompression([first, second], false);
  await ui.runCompression([input('later')], false);
  const dataset = {compressionOutput: s.jobs[0].id, archiveIndex: '1', outputAction: 'open'};
  s.active = 'starting';
  await ui.useCompressionResult({dataset});
  assert.deepEqual(events, []);
  s.active = null;
  await ui.useCompressionResult({dataset});
  assert.deepEqual(events, [['open', second.path], ['dialog', second.path], ['error', 'file no longer exists']]);
});

test('multi-archive progress counts acknowledged create and verify steps monotonically', async () => {
 const {ui,frames}=harness(request=>request.operation==='test'?done({valid:true}):done({outputs:[request.path]}));
 await ui.runCompression([input('a'),input('b')],true);
 assert.deepEqual([...new Set(frames.map(frame=>frame.overall))],[0,25,50,75,100]);
 assert.ok(frames.some(frame=>frame.stage==='verify'&&frame.index===1&&frame.overall===75));
});

test('an integrity failure cannot fill the last step',async()=>{
 const {ui,frames,s}=harness(request=>request.operation==='test'?done({valid:false}):done({outputs:[request.path]}));
 await ui.runCompression([input('a')],true);
 assert.equal(s.jobs[0].status,'failed');
 assert.ok(frames.some(frame=>frame.stage==='verify'&&frame.overall===50));
 assert.equal(frames.at(-1).overall,null);
 assert.equal(frames.some(frame=>frame.overall===100),false);
});
