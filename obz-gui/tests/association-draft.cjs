const assert = require('node:assert/strict');
const fs = require('node:fs');
const test = require('node:test');
const vm = require('node:vm');

const app = fs.readFileSync(require.resolve('../frontend/app.js'), 'utf8');
const state = app.match(/let associationEntries=[^\n]+/)[0];
const handlers = app.slice(app.indexOf('function changedAssociations()'), app.indexOf('function historyPage()'));

test('refresh merges unsaved additions and removals with external updates and saves once', async () => {
  let registered = ['.7z'];
  const saves = [];
  const entries = () => ['.zip', '.7z', '.rar'].map(extension => ({
    extension, registered: registered.includes(extension),
    current_progid: extension === '.rar' ? 'Other.App' : null,
  }));
  const context = vm.createContext({
    s: {page:'associations'}, render(){}, toast(){}, taskError:String, esc:String, btn:()=>'', $:()=>null,
    main:{querySelectorAll:()=>[], querySelector:()=>null},
    async invoke(command, options) {
      if (command === 'set_file_associations') {
        registered = [...options.extensions];
        saves.push(registered);
      } else {
        assert.equal(command, 'association_status');
      }
      return entries();
    },
  });
  vm.runInContext(state + '\n' + handlers, context);
  await context.loadAssociations();
  vm.runInContext("associationSelection.add('.zip'); associationSelection.delete('.7z')", context);
  registered.push('.rar');
  await context.loadAssociations();
  assert.match(context.associationPage(), /有 2 项更改待保存/);
  assert.match(context.associationPage(), /data-association="\.rar" checked/);
  assert.match(context.associationPage(), /data-association="\.zip" checked/);
  assert.doesNotMatch(context.associationPage(), /data-association="\.7z" checked/);
  assert.match(context.associationPage(), /其他应用</);
  assert.match(context.associationPage(), /未设置</);
  await context.saveAssociations();
  await context.saveAssociations();
  assert.deepEqual(saves, [['.zip', '.rar']]);
  vm.runInContext("associationSelection.delete('.rar')", context);
  context.revertAssociationChanges();
  assert.equal(vm.runInContext('changedAssociations().length', context), 0);
  assert.equal(saves.length, 1);
});
