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

function addFixture(invoke) {
 const window = {}, body = {};
 const document = {body, activeElement: body};
 const focus = field => () => { document.activeElement = field; };
 const note = {hidden: true};
 const password = {type: 'password', value: ''};
 const label = {value: ''};
 const toggle = {setAttribute(name, value) { this[name] = value; }};
 const caption = {}, preview = {}, submit = {}, saveStatus = {};
 const reveal = {setAttribute() {}};
 Object.assign(password, {focus: focus(password)});
 Object.assign(label, {focus: focus(label)});
 Object.assign(toggle, {focus: focus(toggle)});
 const fields = {
  '#oa-book-note': note, '[data-book-note-toggle]': toggle, '[data-book-note-caption]': caption, '[data-book-note-preview]': preview,
  '[data-book-password]': password, '[data-book-label]': label, '[data-book-reveal]': reveal,
  '[data-book-submit]': submit, '[data-book-save-status]': saveStatus,
 };
 const form = {
  isConnected: true,
  contains: field => Object.values(fields).includes(field),
  querySelector: selector => fields[selector],
  reset() { password.value = ''; label.value = ''; },
 };
 const elements = {
  '[data-book-add]': form,
  '[data-book-entries]': {replaceChildren() {}, isConnected: false},
  '[data-book-status]': {},
 };
 vm.runInNewContext(read('password-book.js'), {window, document});
 const book = window.QingPasswordBook({invoke});
 book.mountPage({querySelector: selector => elements[selector]});
 return {book, form, document, password, label, note, toggle, caption, preview, reveal, submit, saveStatus};
}

test('new password is required and the note starts in a separate collapsed region', () => {
 const {book} = addFixture(async () => []);
 assert.match(book.pageMarkup, /data-book-password type="password"[^>]+ required/);
 assert.match(book.pageMarkup, /data-book-note-toggle aria-expanded="false" aria-controls="oa-book-note"/);
 assert.match(book.pageMarkup, /data-book-note-caption>添加备注/);
 assert.match(book.pageMarkup, /id="oa-book-note" class="oa-book-note" hidden>/);
 assert.match(book.pageMarkup, /备注（选填）/);
 assert.match(book.pageMarkup, /placeholder="例如：设计部周报"/);
});

test('collapsed note follows the password on save and a failed save retains the masked draft', async () => {
 let fail = true;
 const saved = [];
 const f = addFixture(async (command, args) => {
  assert.equal(command, 'password_book_save');
  assert.equal(f.password.type, 'password');
  saved.push([args.label, args.password]);
  if (fail) throw Error('fixture write error');
 });
 const event = {preventDefault() {}, stopPropagation() {}, key: 'Escape'};
 f.password.value = 'fixture-only';
 f.reveal.onclick();
 assert.equal(f.password.type, 'text');
 f.toggle.onclick();
 assert.equal(f.note.hidden, false);
 assert.equal(f.document.activeElement, f.label);
 f.label.value = '项目归档';
 f.label.onkeydown(event);
 assert.equal(f.toggle['aria-expanded'], 'false');
 assert.equal(f.document.activeElement, f.toggle);
 assert.equal(f.preview.textContent, '随密码保存 · 项目归档');
 await f.form.onsubmit(event);
 assert.match(f.saveStatus.textContent, /无法保存/);
 assert.equal(f.password.value, 'fixture-only');
 assert.equal(f.password.type, 'password');
 assert.equal(f.reveal.textContent, '显示');
 assert.equal(f.label.value, '项目归档');
 assert.equal(f.note.hidden, true);

 fail = false;
 await f.form.onsubmit(event);
 assert.deepEqual(saved, [['项目归档', 'fixture-only'], ['项目归档', 'fixture-only']]);
 assert.equal(f.password.value, '');
 assert.equal(f.label.value, '');
 assert.equal(f.note.hidden, true);
 assert.equal(f.preview.hidden, true);
 assert.equal(f.caption.textContent, '添加备注');
 assert.equal(f.document.activeElement, f.password);
 f.password.value = 'new-fixture';
 await f.form.onsubmit(event);
 assert.deepEqual(saved.at(-1), ['', 'new-fixture']);
});

function pickerFixture(invoke) {
 const window = {}, events = [], attributes = {};
 const document = {};
 const focus = element => () => { document.activeElement = element; };
 const button = {setAttribute: (name,value) => { attributes[name] = value; }};
 button.focus = focus(button);
 let status, list, handlers = {};
 const choices = {
  hidden: true,
  set innerHTML(_) {
   status = {isConnected: true, textContent: ''};
   list = {nodes: [], append(node) { this.nodes.push(node); }, setAttribute() {}, removeAttribute() {}};
  },
  querySelector(selector) { return selector === 'ul' ? list : status; },
  querySelectorAll() { return list?.nodes.map(node => node.item) ?? []; },
  replaceChildren() { status.isConnected = false; list.nodes = []; },
 };
 let picker = {
  append() {}, insertAdjacentHTML() {},
  querySelector: selector => selector === '[data-password-picker]' ? button : choices,
  addEventListener: (name,callback) => { handlers[name] = callback; },
 };
 const label = {before() {}};
 const input = {
  id: 'fixture-password', type: 'password', value: '', isConnected: true,
  closest: selector => selector === 'label' ? label : null,
  matches: () => false,
  addEventListener: (name,callback) => { handlers[name] = callback; },
  dispatchEvent(event) { events.push([event.type,event.bubbles]); handlers[event.type]?.(event); },
 };
 input.focus = focus(input);
 document.createElement = tag => {
  if (tag === 'div') return picker;
  if (tag === 'li') return {append(item) { this.item = item; }};
  const item = {dataset: {}, lastElementChild: {}, setAttribute(name, value) { this[name] = value; }};
  item.focus = focus(item);
  return item;
 };
 vm.runInNewContext(read('password-book.js'), {window, document, Event});
 const book = window.QingPasswordBook({invoke});
 book.mountPickers({querySelectorAll: () => [input]});
 const key = (target, name) => {
  const event = {key: name, target, preventDefault() { this.prevented = true; }, stopPropagation() {}};
  handlers.keydown(event);
  return event.prevented;
 };
 return {book, input, button, choices, document, events, key, attributes, status: () => status};
}

test('password chooser lists only labels, supports navigation and dispatches callbacks on explicit selection', async () => {
 const calls = [];
 const f = pickerFixture(async (command,args) => {
  calls.push(command);
  if (command === 'password_book_list') return [
   {id: 'first', label: '测试归档'}, {id: 'second', label: '测试归档'},
  ];
  assert.equal(command, 'password_book_get');
  assert.equal(args.id, 'second');
  return 'fixture-secret';
 });
 f.button.focus();
 await f.button.onclick();
 const [first, second] = f.choices.querySelectorAll();
 assert.deepEqual(calls, ['password_book_list']);
 assert.deepEqual([first.lastElementChild.textContent, second.lastElementChild.textContent], ['测试归档 · 1','测试归档 · 2']);
 assert.equal(f.document.activeElement, first);
 assert.equal(f.key(first, 'End'), true);
 assert.equal(f.document.activeElement, second);
 assert.equal(f.key(second, 'ArrowUp'), true);
 assert.equal(f.document.activeElement, first);
 assert.equal(f.key(first, 'ArrowDown'), true);
 assert.equal(f.document.activeElement, second);
 await second.onclick();
 assert.deepEqual(calls, ['password_book_list', 'password_book_get']);
 assert.deepEqual(f.events, [['input',true],['change',true]]);
 assert.equal(f.input.value, 'fixture-secret');
 assert.equal(f.input.type, 'password');
 assert.equal(f.choices.hidden, true);
 assert.equal(f.document.activeElement, f.input);
});

test('a cancelled password fetch cannot overwrite manual input or a reopened chooser', async () => {
 let finishSelection;
 const f = pickerFixture(async command => {
  if (command === 'password_book_list') return [{id:'example', label:'样例'}];
  return new Promise(resolve => { finishSelection = resolve; });
 });
 f.button.focus();
 await f.button.onclick();
 const initial = f.choices.querySelectorAll()[0];
 const selection = initial.onclick();
 assert.equal(f.key(initial, 'Escape'), true);
 assert.equal(f.document.activeElement, f.button);
 assert.equal(f.attributes['aria-expanded'], 'false');
 f.input.value = 'manual';
 f.input.dispatchEvent(new Event('input', {bubbles: true}));
 await f.button.onclick();
 finishSelection('stale fixture secret');
 await selection;
 assert.equal(f.input.value, 'manual');
 assert.equal(f.choices.hidden, false);
 assert.equal(f.attributes['aria-expanded'], 'true');
 assert.deepEqual(f.events,[['input',true]]);
});

test('empty password chooser explains manual entry without loading a password', async () => {
 const f = pickerFixture(async command => {
  assert.equal(command, 'password_book_list');
  return [];
 });
 f.button.focus();
 await f.button.onclick();
 assert.match(f.status().textContent, /暂无记录.*手动输入密码/);
 assert.equal(f.choices.hidden, false);
 assert.equal(f.document.activeElement, f.button);
});
