// Package an already-built host binary and the licenses of its locked dependencies.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const { platformName, version } = require('./install.cjs');
const root = path.resolve(__dirname, '..');
const release = version(/^version = "([^"]+)"$/m.exec(fs.readFileSync(path.join(__dirname,'Cargo.toml'),'utf8'))[1]);
const tag = process.env.RELEASE_TAG;
if (tag && tag !== `v${release}`) throw new Error('Release tag must match cli/Cargo.toml version.');
const platform = platformName(process.platform, process.arch);
const name = `tvhome-${platform}`;
const executable = path.resolve(process.env.CARGO_TARGET_DIR || path.join(__dirname,'target'),'release/tvhome');
if (execFileSync(executable,['--version'],{encoding:'utf8'}).trim() !== `tvhome ${release}`) throw new Error('Binary version differs from manifest.');
const notices = fs.readFileSync(path.resolve(process.env.TVHOME_LICENSES || path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname,'target'),'release-licenses.txt')));
if (!execFileSync(executable,['--licenses'],{maxBuffer:32*1024*1024}).equals(notices)) throw new Error('Executable does not embed the current notices.');
const output = path.join(__dirname,'dist'); fs.mkdirSync(output,{recursive:true});
const binary = fs.readFileSync(executable);
const templateName = 'com.anant.tvhome.native_'+JSON.parse(fs.readFileSync(path.join(root,'tv/packaging/appinfo.json'),'utf8')).version+'_arm.ipk';
if (!binary.includes(fs.readFileSync(path.join(root,'tv/dist/template',templateName)))) throw new Error('Executable does not embed the current Home template. Rebuild with bundled-home.');
for (const [file,bytes] of [[name,binary],[`${name}.licenses.txt`,notices]]) {
  fs.writeFileSync(path.join(output,file),bytes,{mode:file===name?0o755:0o644});
  fs.writeFileSync(path.join(output,`${file}.sha256`),`${createHash('sha256').update(bytes).digest('hex')}  ${file}\n`);
}
console.log(`Packaged ${name} ${release}, license notices and SHA-256 checksums.`);
