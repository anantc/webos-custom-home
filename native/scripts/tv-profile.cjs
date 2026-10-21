const fs = require('node:fs');
const path = require('node:path');
function name(value) {
  if (!/^[a-z][a-z0-9-]{0,47}$/.test(value || '')) throw new Error('Supply a TV name: lowercase letters, digits and hyphens, starting with a letter (48 characters maximum).');
  return value;
}
function identity(file, expected) {
  const tv = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (tv.name !== name(expected) || typeof tv.host !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9.-]{0,252}$/.test(tv.host)) throw new Error('TV identity does not match the selected profile. See docs/DEVELOPMENT.md.');
  return tv;
}
function device(devices, expected) {
  if (!Array.isArray(devices)) throw new Error("Invalid webOS CLI device list.");
  const matches = devices.filter(d => d.name === expected.name);
  if (matches.length !== 1) throw new Error('Add this named TV to the webOS CLI before installing. See docs/DEVELOPMENT.md.');
  const d = matches[0];
  const info = d.deviceinfo;
  if (!info || info.ip !== expected.host || Number(info.port) !== 9922 || info.user !== 'prisoner') throw new Error('webOS CLI device address/user/port differs from the packaged TV. Correct the named CLI device before installing.');
  return d;
}
if (require.main === module) {
  try {
    const tvName = name(process.argv[2]);
    const root = path.resolve(__dirname, '../..');
    const packaged = identity(path.join(root, 'native/dist', tvName, 'tv.json'), tvName);
    const synced = identity(path.join(root, 'catalog/tvs', tvName, 'tv.json'), tvName);
    if (packaged.host !== synced.host) throw new Error('TV address changed since packaging. See docs/DEVELOPMENT.md.');
    let devices;
    try { devices = JSON.parse(fs.readFileSync(0, 'utf8')); }
    catch { throw new Error('Cannot parse the webOS CLI device list.'); }
    device(devices, packaged);
  } catch (error) {
    console.error(error.message); process.exitCode = 1;
  }
}
module.exports = { name, identity, device };
