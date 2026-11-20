
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const app = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const formCode = app.slice(app.indexOf('const compressionExtensions='), app.indexOf('const compressionLevels='));

function fixture() {
  let scrolled = false;
  const fields = {
    'qa-format': {value: 'zip'},
    'oa-method': {value: ''},
    'oa-encryption': {value: 'AES256'},
    'oa-password-panel': {hidden: false, closest: () => ({dataset: {}})},
    'oa-password-button': {setAttribute() {}},
    'oa-password-unsupported': {scrollIntoView() { scrolled = true; }},
    'qa-new-password': {value: 'test-only'},
    'oa-password-again': {value: 'test-only'},
  };
  const context = vm.createContext({
    $: id => fields[id],
    filename: input => input.split(/[\\/]/).at(-1),
    opts: () => '',
    syncCompressionControls() {},
  });
  vm.runInContext(formCode, context);
  return {context, fields, scrolled: () => scrolled};
}

test('auto names and format switches preserve the basename and replace composite suffixes', () => {
  const {context} = fixture();
  assert.equal(context.suggestedCompressionPath(['C:\\Reports\\项目.txt'], 'zip', false),
    'C:\\Reports\\项目.txt.zip');
  assert.equal(context.suggestedCompressionPath(['C:\\Reports\\项目.txt', 'C:\\Reports\\样例.txt'], 'tgz', false),
    'C:\\Reports\\Reports.tar.gz');
  assert.equal(context.suggestedCompressionPath(['C:\\Reports\\项目.txt'], 'zip', true), 'C:\\Reports\\');
  assert.equal(context.applyCompressionExtension('D:\\personal\\my.backup.TAR.GZ', '7z'),
    'D:\\personal\\my.backup.7z');
  assert.equal(context.applyCompressionExtension('D:\\personal\\my.backup.unknown', 'tgz'),
    'D:\\personal\\my.backup.unknown.tar.gz');
  assert.equal(context.compressionSuffix('D:\\folder.zip\\my.backup.TAR.GZ')[0], 'tgz');
});

test('same basenames show the nearest parents that distinguish their paths', () => {
  const {context} = fixture();
  const paths = [
    'C:\\shared\\Invoices\\Client A\\Current\\source.txt',
    'C:\\shared\\Invoices\\Client B\\Current\\source.txt',
    'C:\\shared\\Invoices\\Different\\source.txt',
    'C:\\shared\\Invoices\\Client B\\Current\\another.txt',
  ];
  assert.deepEqual(Array.from(context.sourceParentLabels(paths)),
    ['Client A\\Current', 'Client B\\Current', 'Different', 'Current']);
});

test('ZIP to TAR to 7z blocks unencrypted submission and preserves both passwords', () => {
  const {context, fields, scrolled} = fixture();

  context.methodOptions();
  assert.equal(context.validateCompressionPassword('zip'), 'test-only');
  fields['qa-format'].value = 'tar';
  context.methodOptions();
  assert.equal(fields['oa-password-unsupported'].hidden, false);
  assert.equal(fields['oa-password-panel'].hidden, true);
  assert.throws(() => context.validateCompressionPassword('tar'), /请选择支持加密的格式/);
  assert.equal(scrolled(), true);
  assert.equal(fields['qa-new-password'].value, 'test-only');
  assert.equal(fields['oa-password-again'].value, 'test-only');

  fields['qa-format'].value = '7z';
  context.methodOptions();
  assert.equal(fields['oa-password-unsupported'].hidden, true);
  assert.equal(fields['oa-encryption'].disabled, false);
  assert.equal(context.validateCompressionPassword('7z'), 'test-only');
});
