const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const vm = require('node:vm');

const source = fs.readFileSync(require.resolve('../frontend/app.js'), 'utf8');
const catalog = source.slice(source.indexOf('const toolCatalog='), source.indexOf('function toolPage()'));

test('returning to a filtered toolbox matches only the named tools and their descriptions', () => {
  const state = {};
  const context = vm.createContext({s:state, esc:String, icon:()=>'', btn:()=>''});
  vm.runInContext(catalog,context);
  for (const [query,expected] of [
    ['密码',['password-book','recover-tool']],
    [' ZIP  修复 ',['repair-tool']],
    ['校验文件大小',[]],
    ...['sha-256','SHA256','MD5','哈希','校验值'].map(term=>[term,['checksum-tool']]),
  ]) {
    state.toolQuery=query;
    const html = context.toolbox();
    const cards = [...html.matchAll(/<button type="button" class="oa-toolcard"\s+([^>]+)>/g)];
    const visible = cards.filter(([,attrs]) => !/\bhidden\b/.test(attrs));
    assert.deepEqual(visible.map(([,attrs])=>attrs.match(/aria-labelledby="oa-tool-([^"]+)"/)[1]),expected);
    assert.equal(html.includes('id="oa-tool-empty" hidden'),expected.length>0);
  }
});
