#!/usr/bin/env node
// Download and verify a published executable for this platform.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { createHash } = require('node:crypto');
const repository = 'anantc/webos-custom-home';
function platformName(platform, arch) {
  if (!['darwin', 'linux'].includes(platform) || !['arm64', 'x64'].includes(arch)) {
    throw new Error('Downloads support macOS and Linux on ARM64 or x64.');
  }
  return `${platform}-${arch}`;
}
function version(value) {
  if (!/^\d+\.\d+\.\d+$/.test(value || '')) throw new Error('Version must look like 0.2.0.');
  return value;
}
function options(args) {
  const result = {};
  while (args.length) {
    const option = args.shift();
    if (option === '--help') { result.help = true; continue; }
    if (!['--version', '--dir'].includes(option) || !args.length || args[0].startsWith('--')) {
      throw new Error('Usage: node cli/install.cjs [--version 0.2.0] [--dir /path/to/bin]');
    }
    result[option.slice(2)] = args.shift();
  }
  return result;
}
function verify(bytes, checksum, name) {
  const match = /^([a-f0-9]{64})  ([A-Za-z0-9._-]+)\n?$/.exec(checksum.toString('utf8'));
  if (!match || match[2] !== name || createHash('sha256').update(bytes).digest('hex') !== match[1]) {
    throw new Error(`Checksum verification failed for ${name}; nothing was installed.`);
  }
}
async function download(url, limit) {
  const response = await fetch(url, {signal: AbortSignal.timeout(120000)});
  if (!response.ok) {
    if (response.status === 404) throw new Error('This tvhome release is not published yet. A maintainer must publish its downloads first.');
    throw new Error(`Download failed (HTTP ${response.status}).`);
  }
  if (new URL(response.url).protocol !== 'https:') throw new Error('Download must use HTTPS.');
  const chunks = []; let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > limit) throw new Error('Download exceeded its size limit.');
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
function atomicWrite(file, bytes, mode) {
  fs.mkdirSync(path.dirname(file), {recursive:true});
  const tmp = fs.mkdtempSync(path.join(path.dirname(file), '.tvhome-install-'));
  try {
    const staged = path.join(tmp, 'download');
    fs.writeFileSync(staged, bytes, {mode});
    fs.renameSync(staged, file);
  } finally { fs.rmSync(tmp, {recursive:true, force:true}); }
}
async function install({release, platform, arch, directory, notices, fetchBytes = download}) {
  const name = `tvhome-${platformName(platform, arch)}`;
  const tag = `v${version(release)}`;
  const base = `https://github.com/${repository}/releases/download/${tag}`;
  const license = `${name}.licenses.txt`;
  const [binary, checksum, licenseText, licenseChecksum] = await Promise.all([
    fetchBytes(`${base}/${name}`, 64*1024*1024),
    fetchBytes(`${base}/${name}.sha256`, 1024),
    fetchBytes(`${base}/${license}`, 16*1024*1024),
    fetchBytes(`${base}/${license}.sha256`, 1024),
  ]);
  verify(binary, checksum, name);
  verify(licenseText, licenseChecksum, license);
  // Check both payloads before changing an existing installation. Never run a download here.
  atomicWrite(path.join(notices, `${tag}-${name}.txt`), licenseText, 0o644);
  const executable = path.join(directory, 'tvhome');
  atomicWrite(executable, binary, 0o755);
  return executable;
}
async function main() {
  const args = options(process.argv.slice(2));
  if (args.help) {
    console.log('Download and install tvhome.\nUsage: node cli/install.cjs [--version 0.2.0] [--dir /path/to/bin]');
    return;
  }
  const release = args.version || /^version = "([^"]+)"$/m.exec(fs.readFileSync(path.join(__dirname,'Cargo.toml'),'utf8'))?.[1];
  const executable = await install({release, platform:process.platform, arch:process.arch,
    directory:path.resolve(args.dir || path.join(os.homedir(),'.local/bin')),
    notices:path.join(os.homedir(),'.tvhome/notices')});
  console.log(`Installed ${executable}\nRun it with --help. Add its directory to PATH if needed.\nFor existing background jobs, rerun tvhome --tv NAME install for each TV to update their copies.`);
}
if (require.main === module) main().catch(error=>{console.error(error.message); process.exitCode=1;});
module.exports = {platformName, version, options, verify, install};
