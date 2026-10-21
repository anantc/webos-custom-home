// Collect notices from the locked Cargo dependency sources into the package.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const out = path.join(process.argv[2], 'licenses');
fs.mkdirSync(out, { recursive: true });
const metadata = JSON.parse(execFileSync('cargo', [
  'metadata', '--locked', '--format-version', '1',
  '--filter-platform', 'armv7-unknown-linux-gnueabi',
], { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }));
const index = [];
for (const pkg of metadata.packages) {
  if (pkg.name === 'tvhome-native') continue;
  const name = `${pkg.name}-${pkg.version}`;
  const source = path.dirname(pkg.manifest_path);
  index.push(`${name}: ${pkg.license || 'See license file'}\n${pkg.repository || pkg.source || ''}\n`);
  const files = fs.readdirSync(source).filter(name => /^(LICENSE|LICENCE|COPYING|NOTICE)/i.test(name))
    .map(name => path.join(source, name)).filter(file => fs.statSync(file).isFile());
  if (pkg.license_file) files.push(path.resolve(source, pkg.license_file));
  for (const file of new Set(files)) {
    const dest = path.join(out, name);
    fs.mkdirSync(dest, { recursive: true });
    fs.copyFileSync(file, path.join(dest, path.basename(file)));
  }
}
fs.writeFileSync(path.join(out, 'DEPENDENCIES.txt'), index.join('\n'));
fs.copyFileSync('/opt/webos/SDL3-webos/share/licenses/SDL3/LICENSE.txt', path.join(out, 'SDL3-LICENSE.txt'));
