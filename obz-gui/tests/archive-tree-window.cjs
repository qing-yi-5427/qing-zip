const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../frontend/archive-tree-window.js'), 'utf8');
const Tree = vm.runInNewContext(source + '\nArchiveTreeWindow');

function press(tree, index, key, {shift=false, toggle=false}={}) {
  let handled = false;
  tree.moveFocus({
    key, shiftKey:shift,
    target:{
      closest:() => ({dataset:{treeIndex:String(index)}}),
      hasAttribute:() => toggle
    },
    preventDefault:() => { handled = true; },
    stopPropagation:() => {}
  });
  return handled;
}

test('virtual folder tree reveals distant rows before moving keyboard focus', () => {
  const tree = Object.create(Tree.prototype);
  tree.rows = Array.from({length:10000}, (_, i) => ({hasChildren:i%2===0}));
  tree.viewport = {scrollTop:0, clientHeight:200};
  tree.height = 40;
  tree.top = () => tree.viewport.scrollTop;
  const focused = [];
  tree.paint = () => {
    tree.nodes = new Map();
    const start = Math.floor(tree.top()/tree.height);
    for (let i = start; i < Math.min(tree.rows.length, start+5); i++) {
      tree.nodes.set(i, {querySelector:selector => ({focus:() => focused.push([i, selector])})});
    }
  };
  assert.equal(press(tree, 0, 'End'), true);
  assert.equal(press(tree, 9999, 'ArrowUp'), true);
  assert.equal(press(tree, 9998, 'Home'), true);
  assert.deepEqual(focused, [[9999,'[data-tree]'],[9998,'[data-tree]'],[0,'[data-tree]']]);

  assert.equal(press(tree, 0, 'Tab', {toggle:true}), false);
  assert.equal(press(tree, 0, 'Tab', {shift:true}), false);
  assert.equal(press(tree, 1, 'Tab'), true);
  assert.deepEqual(focused.at(-1), [2,'[data-tree-toggle]']);
  assert.equal(press(tree, 9999, 'Tab'), false);
});
