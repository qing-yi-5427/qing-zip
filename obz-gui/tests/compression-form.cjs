
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const app = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
const formCode = app.slice(app.indexOf('const compressionExtensions='), app.indexOf('const compressionLevels='));
const compressionOptions = fs.readFileSync(path.join(__dirname, '../frontend/compression-options.js'), 'utf8');

function fixture() {
  let scrolled = false;
  const fields = {
    'qa-format': {value: 'zip', setAttribute() {}, focus() {}},
    'oa-method': {value: '', closest: () => ({})},
    'oa-encryption': {value: 'AES256'},
    'oa-password-panel': {hidden: false, closest: () => ({dataset: {}})},
    'oa-password-button': {setAttribute() {}},
    'oa-password-unsupported': {scrollIntoView() { scrolled = true; }},
    'oa-zip-password-notice': {hidden: true},
    'qa-new-password': {value: 'test-only'},
    'oa-password-again': {value: 'test-only'},
  };
  const context = vm.createContext({
    $: id => fields[id],
    filename: input => input.split(/[\\/]/).at(-1),
    opts: () => '',
    syncCompressionControls() {},
    window: {},
  });
  vm.runInContext(compressionOptions + '\n' + formCode, context);
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

test('supported alternate suffixes are preserved and directory values stay intact during format changes', () => {
  const {context} = fixture();
  for (const [input, format] of [
    ['样本.TGZ', 'tgz'], ['backup.tbz', 'tbz2'], ['part.txz', 'txz'],
    ['picture.ZSTD', 'zstd'], ['loose.gzip', 'gzip'],
  ]) {
    assert.equal(context.applyCompressionExtension(input, format), input);
    assert.equal(context.applyCompressionExtension(input, '7z'), input.replace(/\.[^.]+$/, '.7z'));
  }
  for (const directory of ['C:\\temp\\', 'C:', '.', '..']) {
    assert.equal(context.applyCompressionExtension(directory, 'zip'), directory);
    assert.equal(context.validCompressionFilename(directory), false);
  }
  assert.equal(context.validCompressionFilename('C:\\temp\\broken?.zip'), false);
});

test('automatic output location avoids nested input folders independent of source order', () => {
  const {context} = fixture();
  const sources = ['C:/Reports/Project/nested/file.txt', 'C:/REPORTS/Project/'];
  for (const order of [sources, [...sources].reverse()]) {
    assert.equal(context.suggestedCompressionPath(order, 'zip', false).toLowerCase(), 'c:/reports/reports.zip');
    assert.equal(context.suggestedCompressionPath(order, 'zip', true).toLowerCase(), 'c:/reports/');
  }
  assert.equal(context.suggestedCompressionPath(['C:/folder/a.txt', 'C:/'], 'zip', false), '');
  assert.equal(context.suggestedCompressionPath(['//s/share/folder/a.txt', '//s/share/'], 'zip', true), '');
});

test('presets migrate legacy maximum levels and retain explicitly selected current levels', () => {
  const {context} = fixture();
  const options = context.window.qingCompression;
  for (const [format, maximum] of [['lizard', 49], ['lz4', 12], ['lz5', 15], ['brotli', 11]]) {
    assert.equal(options.profileLevel(format, 9), maximum);
    assert.equal(options.profileLevel(format, maximum, 2), maximum);
    assert.equal(options.preferredLevel(format, '', maximum), maximum);
  }
  assert.equal(options.profileLevel('lz4', 9, 2), 9);
  assert.equal(options.profileLevel('lizard', 5), 10);
  assert.equal(options.profileLevel('lzh', 9), 5);
  assert.equal(options.profileLevel('lzh', 0), 0);
  assert.equal(options.profileLevel('iso', 9), undefined);
  assert.equal(options.profileLevel('tar', 9), undefined);
});

test('single-file formats validate combined sources before creating jobs and allow separate file outputs', () => {
  const {context, fields} = fixture();
  let focused;
  context.overlay = {querySelector: () => ({focus() { focused = 'add-files'; }})};
  fields['qa-format'].focus = () => { focused = 'format'; };
  assert.throws(() => context.validateCompressionSources('zip', [], false), /添加要压缩/);
  assert.equal(focused, 'add-files');
  assert.throws(() => context.validateCompressionSources('gzip', ['a.txt', 'b.txt'], false), /只接受一个文件/);
  assert.equal(focused, 'format');
  // Whether a separate source is a folder is decided by plan_compression_outputs.
  assert.doesNotThrow(() => context.validateCompressionSources('gzip', ['C:\\folder'], true));
  assert.doesNotThrow(() => context.validateCompressionSources('gzip', ['a.txt', 'b.txt'], true));
  assert.doesNotThrow(() => context.validateCompressionSources('tgz', ['a.txt', 'b.txt'], false));
});

test('constrained numeric options reveal the advanced field and reject invalid or inexact values', () => {
  const {context, fields} = fixture();
  let focused = false, expanded = false;
  context.showCompressionAdvanced = value => { expanded = value; };
  fields['qa-threads'] = {
    value: '257', min: '1', max: '256', validity: {valid:false},
    labels: [{textContent: 'CPU 线程数'}], setAttribute() {}, focus() { focused = true; },
  };
  assert.throws(() => context.compressionNumber('qa-threads'), /1–256/);
  assert.equal(expanded, true);
  assert.equal(focused, true);
  fields['qa-threads'].disabled = true;
  assert.equal(context.compressionNumber('qa-threads'), undefined);
  const volume = fields['qa-volume'] = {...fields['qa-threads'], disabled: false, value: '512', validity: {valid:true}};
  assert.equal(context.compressionNumber('qa-volume', 1048576), 536870912);
  volume.value = '8796093022208';
  assert.throws(() => context.compressionNumber('qa-volume', 1048576), /整数/);
});

test('a renamed output requires acknowledgment and a changed collision requires a fresh preview', async () => {
  const {context, fields} = fixture();
  const body = {inert: false};
  const preview = fields['oa-compression-preview'] = {
    dataset: {}, hidden: true, isConnected: true, closest: () => body, scrollIntoView() {},
  };
  const button = {disabled: false, textContent: '创建压缩包', setAttribute() {}, removeAttribute() {}};
  const requested = 'D:\\sample.zip', inputs = ['C:\\sample.txt'];
  fields['qa-save'] = {value: requested};
  fields['oa-separate'] = {checked: false};
  context.overlay = {querySelector: () => button};
  context.document = {};
  context.document.activeElement = button;
  context.lines = () => inputs;
  context.esc = String;
  let chosen = 'D:\\sample (2).zip';
  context.invoke = async (command, args) => {
    assert.equal(command, 'plan_compression_outputs');
    assert.deepEqual(Array.from(args.paths), [requested]);
    assert.deepEqual(JSON.parse(JSON.stringify(args.sources)), [inputs]);
    assert.equal(body.inert, true);
    assert.equal(button.disabled, true);
    return [chosen];
  };
  assert.equal(await context.confirmCompressionOutputs('zip', inputs), null);
  assert.equal(preview.hidden, false);
  assert.equal(button.textContent, '确认创建');
  assert.equal(body.inert, false);
  chosen = 'D:\\sample (3).zip';
  assert.equal(await context.confirmCompressionOutputs('zip', inputs), null);
  assert.match(preview.innerHTML, /sample \(3\)\.zip/);
  chosen = requested;
  assert.equal(await context.confirmCompressionOutputs('zip', inputs), null);
  assert.match(preview.innerHTML, /将创建以下压缩包/);
  assert.deepEqual(await context.confirmCompressionOutputs('zip', inputs), [chosen]);
});

test('source problems found while planning point at the field that fixes them', async () => {
  const {context, fields} = fixture();
  const preview = fields['oa-compression-preview'] = {dataset: {}, hidden: true, isConnected: true, closest: () => ({inert: false})};
  const button = {disabled: false, textContent: '创建压缩包', setAttribute() {}, removeAttribute() {}, focus() {}};
  let focused = '';
  fields['qa-save'] = {value: 'D:\\out', setAttribute() {}, focus() { focused = 'save'; }};
  fields['qa-format'].focus = () => { focused = 'format'; };
  fields['oa-separate'] = {checked: true};
  context.overlay = {querySelector: () => button};
  context.document = {activeElement: button};
  context.compressionLeaf = input => input.split('\\').at(-1);
  context.compressionExtensions = {gzip: '.gz', zip: '.zip'};
  const inputs = ['C:\\a.txt', 'C:\\dir'];
  let sent, failure;
  context.invoke = async (_command, args) => { sent = JSON.parse(JSON.stringify(args.sources)); throw failure; };
  failure = '所选格式只能压缩单个文件，不能直接压缩文件夹。请改用 TAR.GZ、ZIP 或 7z 等格式，或移除文件夹来源：\nC:\\dir';
  await assert.rejects(context.confirmCompressionOutputs('gzip', inputs), error => error.message === failure);
  assert.deepEqual(sent, [['C:\\a.txt'], ['C:\\dir']]);
  assert.equal(focused, 'format');
  assert.equal(button.disabled, false);
  failure = '保存位置在来源文件夹内，压缩包会把自己打包进去。请选择来源文件夹以外的位置：\nC:\\dir';
  await assert.rejects(context.confirmCompressionOutputs('zip', inputs), error => error.message === failure);
  assert.equal(focused, 'save');
  assert.equal(preview.hidden, true);
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

test('the add action names replacement when the selection matches an archive entry', () => {
  const {context, fields} = fixture();
  const code = app.slice(app.indexOf('function refreshAddConflicts('), app.indexOf('function syncAddPassword('));
  vm.runInContext(code, context);
  const input = fields['qa-edit-inputs'] = {addContext: {existingNames: new Set(['sample.txt'])}};
  const mode = fields['qa-update-mode'] = {value: 'add'};
  const notice = fields['oa-add-conflicts'] = {};
  const button = {};
  context.overlay = {querySelector: () => button};
  context.refreshAddConflicts(input, ['C:/Sample.txt']);
  assert.equal(button.textContent, '添加并替换');
  assert.match(notice.textContent, /Sample\.txt/);
  mode.value = 'update';
  context.refreshAddConflicts(input, ['C:/Sample.txt']);
  assert.equal(button.textContent, '更新');
  mode.value = 'add';
  context.refreshAddConflicts(input, ['C:/new.txt']);
  assert.equal(button.textContent, '添加');
  assert.equal(notice.hidden, true);
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

test('ZipCrypto rejects passwords that its writer cannot encode before starting', () => {
  const {context, fields} = fixture();
  let focused = '';
  fields['oa-encryption'].focus = () => { focused = 'encryption'; };
  context.methodOptions();
  fields['oa-encryption'].value = 'ZipCrypto';
  for (const password of ['Pass\t123', '中文密码']) {
    fields['qa-new-password'].value = fields['oa-password-again'].value = password;
    assert.throws(() => context.validateCompressionPassword('zip'), /ZipCrypto 只支持.*AES-256/);
  }
  assert.equal(focused, 'encryption');
  fields['oa-encryption'].value = 'AES256';
  assert.equal(context.validateCompressionPassword('zip'), '中文密码');
  fields['oa-encryption'].value = 'ZipCrypto';
  fields['qa-new-password'].value = fields['oa-password-again'].value = 'Pass -123\x7f';
  assert.equal(context.validateCompressionPassword('zip'), 'Pass -123\x7f');
});

test('ZIP and ZIPX names notice belongs to the password disclosure and follows the form format', () => {
  const {context, fields} = fixture();
  const markup = app.slice(app.indexOf('<div id="oa-password-panel"'), app.indexOf('${compressionAdvancedFields()}'));
  assert.match(markup, /^<div id="oa-password-panel"[^>]* hidden>/);
  assert.match(markup, /<p id="oa-zip-password-notice" class="qa-sub" hidden>密码保护文件内容，文件名仍可见。<\/p>\s*<div class="qa-fieldrow">\s*<label[^>]*>密码<input id="qa-new-password"/);
  fields['qa-save'] = {dataset: {}};
  fields['oa-separate'] = {checked: false};
  context.refreshSources = () => {};
  const panel = fields['oa-password-panel'];
  const notice = fields['oa-zip-password-notice'];
  const visible = () => !panel.hidden && !notice.hidden;
  panel.hidden = true;
  context.methodOptions();
  assert.equal(visible(), false);
  panel.hidden = false;
  assert.equal(visible(), true);

  fields['qa-format'].value = 'zipx';
  context.changeCompressionFormat();
  assert.equal(visible(), true);
  for (const format of ['7z', 'sfx', 'tar']) {
    fields['qa-format'].value = format;
    context.changeCompressionFormat();
    assert.equal(notice.hidden, true, format);
  }
  assert.equal(panel.hidden, true);
  fields['qa-format'].value = 'zip';
  context.changeCompressionFormat();
  assert.equal(visible(), false);
  panel.hidden = false;
  assert.equal(visible(), true);
  assert.equal(context.validateCompressionPassword('zip'), 'test-only');
});
