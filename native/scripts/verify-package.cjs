// Read archives in memory; never extract paths from the package onto the host.
const assert = require('node:assert/strict');
const { gunzipSync } = require('node:zlib');
const { execFileSync } = require('node:child_process');

function tarEntries(gzip) {
  const bytes = gunzipSync(gzip, { maxOutputLength: 256 * 1024 * 1024 });
  const entries = [];
  let longName, pax = {};
  const text = (start, size) => bytes.subarray(start, start + size).toString('utf8').split('\0')[0];
  for (let offset = 0; offset + 512 <= bytes.length;) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every(b => b === 0)) break;
    const checksum = [...header].reduce((sum, b, i) => sum + (i >= 148 && i < 156 ? 32 : b), 0);
    assert.equal(parseInt(text(offset + 148, 8).trim(), 8), checksum, 'Invalid tar checksum');
    const size = parseInt(text(offset + 124, 12).trim(), 8);
    const mode = parseInt(text(offset + 100, 8).trim(), 8);
    assert.ok(Number.isSafeInteger(size) && size >= 0 && offset + 512 + size <= bytes.length, 'Invalid tar size');
    const content = bytes.subarray(offset + 512, offset + 512 + size);
    const type = text(offset + 156, 1);
    if (type === 'L') longName = content.toString('utf8').replace(/\0.*$/s, '');
    else if (type === 'x') {
      // POSIX extended names are byte-length-prefixed, including the trailing newline.
      for (let i = 0; i < content.length;) {
        const space = content.indexOf(32, i);
        const length = Number(content.subarray(i, space).toString());
        assert.ok(space > i && length > space - i + 1 && i + length <= content.length, 'Invalid PAX record');
        const record = content.subarray(space + 1, i + length - 1).toString('utf8');
        const equals = record.indexOf('=');
        pax[record.slice(0, equals)] = record.slice(equals + 1);
        i += length;
      }
    } else {
      const prefix = text(offset + 345, 155);
      const name = pax.path || longName || (prefix ? prefix + '/' : '') + text(offset, 100);
      entries.push({ name, mode, content });
      longName = undefined; pax = {};
    }
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return entries;
}
function verify(data, control, template = false) {
  const controlFile = control.find(e => e.name === 'control' || e.name.endsWith('/control'));
  assert.ok(controlFile && /^Architecture: arm\r?$/m.test(controlFile.content.toString()), 'Wrong package architecture');
  const member = suffix => {
    const matches = data.filter(e => e.name === suffix || e.name.endsWith('/' + suffix));
    assert.equal(matches.length, 1, `Expected exactly one ${suffix}`);
    return matches[0];
  };
  const read = suffix => member(suffix).content;
  const info = JSON.parse(read('appinfo.json'));
  assert.equal(info.id, 'com.anant.tvhome.native');
  assert.equal(info.type, 'native');
  assert.match(info.iconColor, /^#[0-9a-fA-F]{6}$/);
  for (const [key, size] of [['icon', 80], ['largeIcon', 130]]) {
    const icon = read(info[key]);
    assert.deepEqual(icon.subarray(0, 8), Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
    assert.equal(icon.readUInt32BE(16), size); assert.equal(icon.readUInt32BE(20), size);
  }
  const state = data.find(e => e.name.endsWith('/com.anant.tvhome.native/state/'));
  assert.ok(state && (state.mode & 0o777) === 0o777, 'App state directory must be writable by the jailed app');
  const catalog = JSON.parse(read('assets/catalog.json'));
  assert.equal(catalog.preview, false);
  if (template) {
    assert.equal(catalog.setupTemplate, true);
    assert.equal(catalog.apps.length, 0);
    const policy = JSON.parse(read('assets/setup.json'));
    assert.equal(policy.schema, 1);
    assert.ok(policy.brands.length);
    for (const brand of policy.brands) assert.ok(read('assets/' + brand.artwork).length);
  } else {
    assert.ok(!catalog.setupTemplate);
    assert.ok(catalog.apps.length && catalog.apps.every(a => typeof a.id === 'string' && a.id && !a.id.startsWith('preview.')));
  }
  const binary = read('bin/tvhome-native');
  assert.deepEqual(binary.subarray(0, 6), Buffer.from([127, 69, 76, 70, 1, 1]));
  assert.equal(binary.readUInt16LE(18), 40);
  const flags = binary.readUInt32LE(36);
  assert.ok((flags & 0x200) && !(flags & 0x400), 'Expected soft-float ARM ABI');
  assert.ok(member('bin/tvhome-native').mode & 0o111, 'Native app must be executable');
  for (const lib of ['lib/libSDL3.so.0', 'lib/libfreetype.so.6']) assert.equal(read(lib).subarray(0, 4).toString(), '\x7fELF');
  assert.ok(!data.some(e => e.name.includes('libstdc++.so')));
  assert.ok(read('licenses/SDL3-LICENSE.txt').length);
  assert.ok(read('assets/Inter-OFL.txt').length);
  return { id: info.id, apps: catalog.apps.length };
}
if (require.main === module) {
  const archive = name => tarEntries(execFileSync('ar', ['p', process.argv[2], name], { maxBuffer: 256 * 1024 * 1024 }));
  const result = verify(archive('data.tar.gz'), archive('control.tar.gz'), process.argv.includes('--template'));
  console.log(`Verified ARM package: ${result.id}, ${result.apps} real app IDs, executable and bundled libraries.`);
}
module.exports = { tarEntries, verify };
