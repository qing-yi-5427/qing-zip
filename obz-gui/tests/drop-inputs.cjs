const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const app = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const functions = app.slice(app.indexOf('function appendArchiveInputs('), app.indexOf('function refreshSources('));

function fixture(id) {
  const input = id && {id, value: 'C:\\original', isConnected: true};
  const calls = [];
  const context = {
    deepExtract:{acceptDrop:()=>false}, s: {active: null,page:'files',archive:'C:\\current.zip'}, overlay: {hidden: !id, querySelector: () => input},
    lines: () => input.value.split(/\r?\n/).map(p => p.trim()).filter(Boolean),
    refreshSources: () => calls.push('refresh'),
    enqueueArchives: paths => calls.push(paths),
    toast: () => calls.push('busy')
  };
  vm.runInNewContext(functions, context);
  return {input, calls, context};
}

test('native drops add files and directories to the open creation or update form', () => {
  for (const id of ['qa-inputs', 'qa-edit-inputs']) {
    const {input, calls, context} = fixture(id);
    context.acceptDroppedPaths(['C:\\original', 'D:\\Folder', 'D:\\文件 1.txt',
      'D:\\Folder', 'x\ninjected.txt', null]);
    assert.equal(input.value, 'C:\\original\nD:\\Folder\nD:\\文件 1.txt');
    assert.deepEqual(calls, ['refresh']);
  }
});

test('dropping into the batch list retains its current inputs and view', () => {
  const {input,calls,context}=fixture('qa-batch-inputs');
  Object.assign(context,{s:{page:'batch'},$:(id)=>id===input.id&&input,
    updateBatchPage:()=>calls.push('batch')});
  context.overlay.hidden=true;
  context.acceptDroppedPaths(['C:\\original','C:\\next.zip','C:\\next.zip']);
  assert.equal(input.value,'C:\\original\nC:\\next.zip');
  assert.deepEqual(calls,['batch']);
});

test('other drops still open archives, busy forms and dismissed pickers keep their inputs', () => {
  const plain = fixture(null);
  const paths = ['C:\\one.zip', 'C:\\two.7z'];
  plain.context.acceptDroppedPaths(paths);
  assert.deepEqual(plain.calls, [paths]);
  const {input, calls, context} = fixture('qa-inputs');
  context.s.active = 123;
  context.acceptDroppedPaths(paths);
  input.isConnected = false;
  context.appendArchiveInputs(input, paths);
  assert.deepEqual(calls, ['busy']);
  assert.equal(input.value, 'C:\\original');
  assert.match(app, /event\.listen\('tauri:\/\/drag-drop',e=>\{[\s\S]+?acceptDroppedPaths\(e\.payload\?\.paths\|\|\[\]\);\s*\}/);
});
