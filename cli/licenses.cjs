// Collect notices before compiling so the executable can install them offline.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const target = /^host: (.+)$/m.exec(execFileSync('rustc',['-vV'],{encoding:'utf8'}))[1];
const metadata = JSON.parse(execFileSync('cargo',['metadata','--locked','--format-version','1','--features','portable,bundled-home','--filter-platform',target],
  {cwd:__dirname, encoding:'utf8',maxBuffer:32*1024*1024}));
const sections = [fs.readFileSync(path.join(root,'LICENSE'),'utf8'), fs.readFileSync(path.join(root,'native/dist/template/home-licenses.txt'),'utf8')];
for (const pkg of metadata.packages) {
  if (pkg.name === 'tvhome') continue;
  const source = path.dirname(pkg.manifest_path);
  const files = fs.readdirSync(source).filter(n=>/^(LICENSE|LICENCE|COPYING|NOTICE)/i.test(n))
    .map(n=>path.join(source,n)).filter(file=>fs.statSync(file).isFile());
  if (pkg.license_file) files.push(path.resolve(source,pkg.license_file));
  // The vendored OpenSSL source stores its own Apache license below the crate root.
  if (pkg.name === 'openssl-src') {
    for (const n of ['LICENSE.txt','LICENSE']) {
      const file = path.join(source,'openssl',n); if (fs.existsSync(file)) files.push(file);
    }
  }
  if (pkg.name === 'libz-sys') files.push(path.join(source,'src','zlib','LICENSE'));
  if (pkg.name === 'libssh2-sys') {
    const file = path.join(source,'libssh2','COPYING');
    if (fs.existsSync(file)) files.push(file);
  }
  sections.push(`\n${'='.repeat(72)}\n${pkg.name} ${pkg.version}\n${pkg.license || 'See license text'}\n${pkg.repository || pkg.source || ''}\n`);
  for (const file of new Set(files)) sections.push(fs.readFileSync(file,'utf8'));
}

const output = path.resolve(process.env.TVHOME_LICENSES || path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname,'target'),'release-licenses.txt'));
fs.mkdirSync(path.dirname(output),{recursive:true});
fs.writeFileSync(output,sections.join('\n'));
console.log('Prepared embedded license notices.');
