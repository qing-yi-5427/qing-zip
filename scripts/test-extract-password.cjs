const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../obz-gui/frontend/app.js'), 'utf8');
const passwordLogic = source.slice(
  source.indexOf('function extractPasswordRequired('),
  source.indexOf('function archiveParent('),
);
const label = { hidden: true };
const input = { disabled: true, value: '', closest: () => label };
const entries = [
  { name: 'public/readme.txt', encrypted: false },
  { name: 'private\\nested\\secret.txt', encrypted: true },
  { name: 'separately-encrypted/', encrypted: true },
];
const state = { entries, password: 'fixture-pass', headerEncrypted: false };
const { extractPasswordRequired, syncExtractPassword } = vm.runInNewContext(
  passwordLogic + '\n({ extractPasswordRequired, syncExtractPassword })',
  { s: state, $: () => input },
);

test('matches all entries, exact selections, and descendant boundaries', () => {
  assert.equal(extractPasswordRequired(entries, [], false), true);
  assert.equal(extractPasswordRequired(entries, ['public/'], false), false);
  assert.equal(extractPasswordRequired(entries, ['priv'], false), false);
  assert.equal(extractPasswordRequired(entries, ['private/'], false), true);
  assert.equal(extractPasswordRequired(entries, ['public/readme.txt', 'private/nested/secret.txt'], false), true);
  assert.equal(extractPasswordRequired(entries, ['separately-encrypted/'], false), true);
  assert.equal(extractPasswordRequired([], [], false), false);
  assert.equal(extractPasswordRequired(entries, ['public/'], true), true);
});

test('scope changes remove the plaintext field and clear its value', () => {
  syncExtractPassword(['public/'], false);
  assert.equal(label.hidden, true);
  assert.equal(input.disabled, true);
  assert.equal(input.value, '');

  syncExtractPassword(['private/'], false);
  assert.equal(label.hidden, false);
  assert.equal(input.disabled, false);
  assert.equal(input.value, state.password);
  input.value = 'edited-fixture-pass';
  syncExtractPassword([], false);
  assert.equal(input.value, 'edited-fixture-pass');

  syncExtractPassword(['public/'], false);
  assert.equal(label.hidden, true);
  assert.equal(input.disabled, true);
  assert.equal(input.value, '');
});

test('deep extraction keeps password pre-entry for inner archives', () => {
  syncExtractPassword(['public/'], true);
  assert.equal(label.hidden, false);
  assert.equal(input.disabled, false);
  syncExtractPassword(['public/'], false);
  assert.equal(label.hidden, true);
});
