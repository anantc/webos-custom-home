const test = require('node:test');
const assert = require('node:assert/strict');
const { ordered, signature, next } = require('../catalog/layout.js');
const { forApp } = require('../catalog/presentation.js');
const config = { preferredApps: ['apple-tv', 'netflix', 'prime-video'], hiddenApps: ['home'], dockApps: [] };
const apps = [
  { id: 'home', title: 'Home' }, { id: 'unusual-native-id', title: 'Apple TV' },
  { id: 'amazon', title: 'Prime Video' }, { id: 'native.netflix', title: 'Netflix' },
  { id: 'browser', title: 'Browser' }
];
test('branding changes presentation without replacing actual TV launch IDs', () => {
  assert.deepEqual(ordered(apps, config, null, forApp).map(a => a.id), ['unusual-native-id', 'native.netflix', 'amazon', 'browser']);
  assert.equal(forApp({ title: 'YouTube TV', id: 'youtube.leanback.ytv.v1' }), null);
  assert.equal(forApp({ title: 'JioHotstar', id: 'native.42' }).key, 'jiohotstar');
});
test('saved order survives app removals/additions, rejects duplicates and hidden apps', () => {
  const saved = { signature: signature(config), ids: ['browser', 'deleted', 'home', 'browser', 'amazon'] };
  const result = ordered(apps.concat(apps[2], { id: 'new', title: 'New app' }), config, saved, forApp);
  assert.deepEqual(result.map(a => a.id), ['browser', 'amazon', 'unusual-native-id', 'native.netflix', 'new']);
  assert.equal(apps.length, 5, 'ordering must not mutate the synced source');
});
test('a changed preference list resets order; explicit native IDs take priority', () => {
  const changed = { ...config, dockApps: ['browser', 'not-installed'] };
  const saved = { signature: signature(config), ids: ['amazon'] };
  assert.equal(ordered(apps, changed, saved, forApp)[0].id, 'browser');
});
test('corrupt or incomplete persisted order falls back to preferences', () => {
  assert.equal(ordered(apps, config, { signature: signature(config), ids: 'bad' }, forApp)[0].id, 'unusual-native-id');
});
test('navigation handles ragged rows, row boundaries, top controls and an empty TV', () => {
  assert.equal(next(5, 'right', 8, 6), 5);
  assert.equal(next(6, 'left', 8, 6), 6);
  assert.equal(next(5, 'down', 8, 6), 7);
  assert.equal(next(7, 'down', 8, 6), 7);
  assert.equal(next(7, 'up', 8, 6), 1);
  assert.equal(next(2, 'up', 8, 6), -1);
  assert.equal(next(0, 'down', 0, 6), -1);
});
test('all navigation destinations are valid across 4–6 columns and 1–40 apps', () => {
  for (let cols = 4; cols <= 6; cols++) for (let count = 1; count <= 40; count++) {
    for (let i = 0; i < count; i++) for (const dir of ['left', 'right', 'up', 'down']) {
      const target = next(i, dir, count, cols);
      assert.ok(target >= -1 && target < count, `${cols}/${count}/${i}/${dir}`);
      if (target === -1) assert.ok(dir === 'up' && i < cols);
    }
  }
});
