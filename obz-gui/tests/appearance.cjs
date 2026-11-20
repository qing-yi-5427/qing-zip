const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../frontend/appearance.js'), 'utf8');
const settle = () => new Promise(resolve => setImmediate(resolve));

function harness(systemDark = false) {
  const events = new Map(), mediaEvents = new Map(), paints = [], errors = [], requests = [];
  const preference = { theme: 'system', material: 'acrylic' };
  const contrast = { matches: false, addEventListener: (name, callback) => mediaEvents.set(name, callback) };
  const window = {
    matchMedia: query => query.includes('forced-colors') ? contrast : { matches: systemDark },
    __TAURI__: {
      core: { invoke: (command, body) => new Promise(resolve => requests.push({ command, body, resolve })) },
      event: { listen: async (name, callback) => { events.set(name, callback); } },
    },
  };
  vm.runInNewContext(source, { window });
  const api = window.QingAppearance;
  const update = api.bind(() => preference, (...paint) => paints.push(paint), err => errors.push(err));
  return { api, contrast, errors, events, mediaEvents, paints, preference, requests, update };
}

test('new preferences follow Windows and old explicit light/dark selections survive', async () => {
  const { api, paints, preference, requests, update } = harness(true);
  assert.equal(api.preferredMode(undefined), 'system');
  assert.equal(api.preferredMode('light'), 'light');
  assert.equal(api.preferredMode('dark'), 'dark');
  assert.deepEqual(paints, [['dark', 'acrylic']]);
  preference.theme = 'light';
  update();
  await settle();
  assert.equal(requests[0].body.theme, 'light');
  requests[0].resolve({ dark: false, material: 'acrylic' });
  await settle();
  assert.deepEqual(paints.at(-1), ['light', 'acrylic']);
});

test('system mode reads the native result instead of the previous WebView media state', async () => {
  const { events, paints, preference, requests, update } = harness(true);
  preference.theme = 'dark';
  update();
  await settle();
  requests[0].resolve({ dark: true, material: 'acrylic' });
  events.get('tauri://theme-changed')();
  await settle();
  assert.equal(requests.length, 1);
  preference.theme = 'system';
  update();
  await settle();
  assert.equal(requests[1].body.theme, 'system');
  requests[1].resolve({ dark: false, material: 'acrylic' });
  await settle();
  assert.deepEqual(paints.at(-1), ['light', 'acrylic']);
  events.get('tauri://theme-changed')();
  await settle();
  requests[2].resolve({ dark: true, material: 'acrylic' });
  await settle();
  assert.deepEqual(paints.at(-1), ['dark', 'acrylic']);
});

test('a newer preference wins and forced colors selects solid without saving it', async () => {
  const { contrast, errors, mediaEvents, paints, preference, requests, update } = harness();
  update();
  await settle();
  preference.theme = 'dark';
  preference.material = 'mica';
  contrast.matches = true;
  mediaEvents.get('change')();
  requests[0].resolve({ dark: false, material: 'acrylic' });
  await settle();
  assert.equal(requests[1].body.theme, 'dark');
  assert.equal(requests[1].body.material, 'solid');
  assert.equal(preference.material, 'mica');
  assert.equal(paints.length, 1);
  requests[1].resolve({ dark: true, material: 'solid' });
  await settle();
  assert.deepEqual(paints.at(-1), ['dark', 'solid']);
  assert.deepEqual(errors, []);
});
