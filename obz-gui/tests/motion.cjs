const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../frontend/motion.js'), 'utf8');
const flush = () => new Promise(resolve => setImmediate(resolve));

function createHarness() {
  const queries = new Map();
  const decorations = [];
  class Element {
    isConnected = true;
    hidden = false;
    animations = [];
    style = {};
    attributes = new Map();
    setAttribute(name, value) { this.attributes.set(name, value); }
    before(decoration) { decorations.push(decoration); }
    remove() { this.isConnected = false; }
    animate(frames, options) {
      let finish, reject;
      const finished = new Promise((resolve, onReject) => { finish = resolve; reject = onReject; });
      const animation = { frames, options, finished, finish, cancelled: false };
      animation.cancel = () => { animation.cancelled = true; reject(); };
      this.animations.push(animation);
      return animation;
    }
  }
  const window = {};
  vm.runInNewContext(source, {
    window,
    Element,
    matchMedia: query => {
      const listeners = [];
      const media = {
        matches: false,
        addEventListener: (_, callback) => listeners.push(callback),
        change(matches) {
          this.matches = matches;
          listeners.forEach(callback => callback());
        },
      };
      queries.set(query, media);
      return media;
    },
    document: {
      getElementById: () => null,
      createElement: () => new Element(),
    },
    getComputedStyle: () => ({ opacity: '1', getPropertyValue: () => '' }),
  });
  return { Element, decorations, motion: window.QingMotion, reduce: queries.get('(prefers-reduced-motion: reduce)') };
}

test('a new toast cancels the preceding exit and stays visible', async () => {
  const { Element, motion } = createHarness();
  const toast = new Element();
  motion.toastHide(toast);
  const oldExit = toast.animations[0];
  motion.toastShown(toast, false);
  await flush();
  assert.equal(oldExit.cancelled, true);
  assert.equal(toast.hidden, false);

  motion.toastHide(toast);
  const exit = toast.animations.at(-1);
  exit.finish();
  await flush();
  assert.equal(toast.hidden, true);
  assert.equal(exit.cancelled, true);
});

test('closing a dialog adds only an inert, content-free decoration and reopening removes it', async () => {
  const { Element, decorations, motion } = createHarness();
  const overlay = new Element();
  overlay.firstElementChild = { privateValue: 'secret' };
  motion.dialogClosing(overlay);
  const backdrop = decorations[0];
  assert.equal(backdrop.className, 'qa-motion-ghost qa-overlay-ghost');
  assert.equal(backdrop.inert, true);
  assert.equal(backdrop.attributes.get('aria-hidden'), 'true');
  assert.equal(backdrop.firstElementChild, undefined);
  assert.equal(backdrop.attributes.has('id'), false);

  overlay.firstElementChild = new Element();
  motion.dialogShown(overlay, false);
  assert.equal(backdrop.isConnected, false);
  await flush();
});

test('enabling reduced motion settles running exits and bypasses further motion', async () => {
  const { Element, decorations, motion, reduce } = createHarness();
  const overlay = new Element();
  overlay.firstElementChild = new Element();
  motion.dialogClosing(overlay);
  const toast = new Element();
  motion.toastHide(toast);
  reduce.change(true);
  assert.equal(toast.hidden, true);
  assert.equal(decorations[0].isConnected, false);
  assert.equal(toast.animations[0].cancelled, true);

  motion.dialogClosing(overlay);
  assert.equal(decorations.length, 1);
  await flush();
});
