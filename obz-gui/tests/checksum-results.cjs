const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const view = require('../frontend/checksum-view.js');

function render(result) {
  const nodes = [];
  const doc = {createElement(tag) {
    const node = {tag, ownerDocument:doc, dataset:{}, children:[],
      append(child) { this.children.push(child); },
      replaceChildren() { this.children = []; },
      setAttribute(name,value) { this[name] = value; },
      addEventListener(name,handler) { this['on' + name] = handler; }};
    nodes.push(node);
    return node;
  }};
  const host = doc.createElement('div');
  const source = fs.readFileSync(require.resolve('../frontend/tools.js'), 'utf8');
  const start = source.indexOf('function render(');
  const handlers = source.slice(start, source.indexOf("$('cancel').onclick", start));
  vm.runInNewContext(handlers + "; render('checksum', result);", {
    $: () => host, window:{QingChecksumView:view}, result
  });
  return nodes;
}

test('single explicit and legacy digests keep the path, algorithm labels and individual copy actions', () => {
  const explicit = render({path:'资料/中文.txt', hashes:{MD5:'aa', SHA256:'bb'}});
  assert.ok(explicit.some(n => n.textContent === '资料/中文.txt'));
  assert.deepEqual(explicit.filter(n=>n.tag==='code').map(n=>n.textContent), ['aa','bb']);
  assert.deepEqual(explicit.filter(n=>n.tag==='button').map(n=>n['aria-label']), ['复制 MD5 校验值','复制 SHA256 校验值']);
  const legacy = render({path:'empty',crc32:'0',md5:'a',sha1:'b',sha256:'c'});
  assert.deepEqual(legacy.filter(n => n.tag === 'dt').map(n => n.textContent), ['CRC32','MD5','SHA1','SHA256']);
});

test('multi-file output associates each digest with its source and empty folders keep their explanation', () => {
  const multi = render({checksums:[{path:'first',hashes:{SHA512:'1'}},{path:'second',hashes:{SHA512:'2'}}]});
  assert.equal(multi.find(n=>n.tag==='pre').textContent, 'first\nSHA512: 1\n\nsecond\nSHA512: 2');
  assert.ok(!multi.some(n=>n.id==='hash-reference'));
  const empty = render({checksums:[]});
  assert.ok(empty.some(n=>n.textContent==='文件夹中没有可校验的文件。'));
  assert.ok(!empty.some(n=>n.tag==='code'));
});

test('one reference supports plain digests, labeled values and common checksum files', () => {
  const digest = '1a'.repeat(32);
  const file = {path:'C:/资料/新 项目.zip',hashes:{SHA256:digest}};
  const nodes = render(file), input = nodes.find(n=>n.id==='hash-reference'), verdict = nodes.find(n=>n.id==='hash-verdict');
  for (const reference of [digest.toUpperCase(), 'SHA-256: '+digest, digest+'  新 项目.zip\r\n', digest+' *新 项目.zip', 'SHA256 (新 项目.zip) = '+digest]) {
    input.value=reference; input.oninput();
    assert.equal(verdict.dataset.state,'match');
    assert.match(verdict.textContent,/SHA256/);
  }
  assert.equal(view.compare('aa'.repeat(32),file).state,'different');
  assert.match(view.compare(digest+'  旧名字.zip',file).message,/文件名不同/);
  assert.match(view.compare('SHA3-256: '+digest,file).message,/没有计算/);
  for(const reference of ['abcdefg',digest+'\r\n'+digest,'']) {
    assert.notEqual(view.compare(reference,file).state,'match');
  }
});
