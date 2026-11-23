const assert = require('node:assert/strict');
const fs = require('node:fs');
const test = require('node:test');
const vm = require('node:vm');

const read = name => fs.readFileSync(require.resolve('../frontend/' + name), 'utf8');
const app = read('app.js');
const section = (from, to) => app.slice(app.indexOf(from), app.indexOf(to));

test('toolbox and settings route to the same password book and return to their own pages', () => {
  const source = section('function pageHeading(', 'function batchPage(')
    + section('function settings()', 'function rememberPage(')
    + section('function render(){', 'function dialog(')
    + section('const toolCatalog=', 'function toolPage(')
    + 'function action(a){switch(a){'
    + section("case'password-book':", "case'shell-open-entry':")
    + '}}; ({action,navigate})';
  const s = {page: 'tools'};
  const main = {};
  const noop = () => {};
  let mounts = 0;
  const {action, navigate} = vm.runInNewContext(source, {
    s, main, overlay: {dataset: {}}, homeIntake: false, archiveTabs: [], currentTabId: null, toolContext: null,
    passwordBook: {pageMarkup: '<form data-book-add></form>', mountPage: () => mounts++},
    appVersion: 'fixture', folderMenuEnabled: false, fileMenuEnabled: false,
    esc: String, icon: noop, opts: noop,
    btn: (label, action) => `<button data-action="${action}">${label}</button>`,
    $: id => id === 'qa-tools' ? null : {},
    batchPage: noop, profilesPage: noop, associationPage: noop, historyPage: noop, toolPage: noop,
    rememberPage: noop, applyPagePosition: noop, updateColumnScroll: noop, releaseListingWindow: noop,
    stopThumbnailObserver: noop, renderChrome: noop, renderInlineTask: noop, close: noop,
  });
  for (const [origin, label] of [['tools', '工具箱'], ['settings', '设置']]) {
    navigate(origin);
    assert.match(main.innerHTML, /data-action="password-book"/);
    assert.doesNotMatch(main.innerHTML, /data-book-add|vault-tool/);
    action('password-book');
    assert.equal(s.page, 'password-book');
    assert.match(main.innerHTML, /<h1>解压密码本<\/h1>.*data-book-add/);
    assert.match(main.innerHTML, new RegExp(`data-action="password-book-back">返回${label}`));
    action('password-book-back');
    assert.equal(s.page, origin);
  }
  assert.equal(mounts, 2);
});

test('the shared list assigns unlabelled legacy records their IDs and distinguishes duplicate labels', async () => {
  const window = {};
  const rows = [];
  const list = {isConnected: true, replaceChildren: () => { rows.length = 0; }, append: row => rows.push(row)};
  const form = {querySelector: () => ({})};
  const elements = {'[data-book-add]': form, '[data-book-entries]': list, '[data-book-status]': {}};
  vm.runInNewContext(read('password-book.js'), {
    window,
    document: {
      createElement: () => ({
        querySelector: () => ({}),
        querySelectorAll: () => [{setAttribute() {}}, {setAttribute() {}}],
      }),
    },
  });
  const book = window.QingPasswordBook({esc: String, invoke: async command => {
    assert.equal(command, 'password_book_list');
    return [{id: 'legacy-key', label: ''}, {id: 'auto', label: 'archive.zip'}, {id: 'manual', label: 'archive.zip'}];
  }});
  book.mountPage({querySelector: selector => elements[selector]});
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(rows.map(row => row.innerHTML.match(/<strong>(.*?)<\/strong>/)[1]),
    ['legacy-key', 'archive.zip · 1', 'archive.zip · 2']);
});

test('the password book lets users disable automatic saving across windows', () => {
  const window = {};
  vm.runInNewContext(read('password-book.js'), {window});
  const preference = new Map();
  const storage = {
    getItem: name => preference.get(name),
    setItem: (name,value) => preference.set(name,value),
  };
  const options = {storage,invoke: async () => []};
  const book = window.QingPasswordBook(options);
  const otherWindow = window.QingPasswordBook(options);
  const toggle = {};
  const elements = {
    '[data-book-automatic]':toggle, '[data-book-preference-status]':{},
    '[data-book-add]':{querySelector:()=>({})},
    '[data-book-entries]':{replaceChildren(){},isConnected:false},
    '[data-book-status]':{},
  };
  book.mountPage({querySelector: selector => elements[selector]});
  assert.equal(toggle.checked,true);
  toggle.checked = false;
  toggle.onchange();
  assert.equal(otherWindow.automaticSavingEnabled(),false);
  storage.setItem = () => { throw Error('storage unavailable'); };
  toggle.checked = true;
  toggle.onchange();
  assert.equal(toggle.checked,false);
  assert.match(elements['[data-book-preference-status]'].textContent,/无法保存/);
});
