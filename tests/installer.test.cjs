const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const {createHash} = require('node:crypto');
const {platformName, version, options, verify, install} = require('../cli/install.cjs');
function fixture(platform) {
  const name = `tvhome-${platform}`;
  const files = {[name]:Buffer.from('executable fixture; must never be run'),[`${name}.licenses.txt`]:Buffer.from('license fixture')};
  for (const [name,bytes] of Object.entries(files)) files[`${name}.sha256`] = Buffer.from(`${createHash('sha256').update(bytes).digest('hex')}  ${name}\n`);
  return files;
}
function paths(t) {
  const temp=fs.mkdtempSync(path.join(os.tmpdir(),'tvhome-download-test-'));
  t.after(()=>fs.rmSync(temp,{recursive:true,force:true}));
  return {directory:path.join(temp,'bin'),notices:path.join(temp,'notices')};
}
test('all four supported platforms map to explicit assets and invalid input fails before download', async t=>{
  for(const platform of ['darwin','linux']) for(const arch of ['arm64','x64']) assert.equal(platformName(platform,arch),`${platform}-${arch}`);
  for(const [platform,arch] of [['win32','x64'],['linux','arm'],['darwin','ia32']]) assert.throws(()=>platformName(platform,arch));
  for(const bad of ['', '../main', 'latest', '1.2.3?x', '1.2']) assert.throws(()=>version(bad));
  assert.deepEqual(options(['--version','0.2.0','--dir','/a path/bin']),{version:'0.2.0',dir:'/a path/bin'});
  assert.throws(()=>options(['--dir'])); assert.throws(()=>options(['--bad']));
  let called=false;
  await assert.rejects(install({...paths(t),release:'../main',platform:'darwin',arch:'arm64',fetchBytes:async()=>{called=true;}}));
  assert.equal(called,false);
});
test('installer verifies both payloads, preserves modes and uses the requested release', async t=>{
  const destination=paths(t), files=fixture('darwin-arm64'), urls=[];
  const executable=await install({...destination,release:'0.2.0',platform:'darwin',arch:'arm64',fetchBytes:async url=>{
    urls.push(url); return files[path.basename(new URL(url).pathname)];
  }});
  assert.equal(fs.readFileSync(executable,'utf8'),files['tvhome-darwin-arm64'].toString());
  assert.ok(fs.statSync(executable).mode & 0o100);
  assert.equal(fs.readdirSync(destination.notices).length,1);
  assert.equal(fs.readdirSync(destination.directory).join(),'tvhome');
  assert.equal(urls.length,4);
  assert.ok(urls.every(url=>url.startsWith('https://github.com/anantc/webos-custom-home/releases/download/v0.2.0/')));
});
test('a corrupt binary or notice never replaces the installed executable', async t=>{
  for(const bad of ['tvhome-linux-x64','tvhome-linux-x64.licenses.txt']) {
    const destination=paths(t),files=fixture('linux-x64');
    fs.mkdirSync(destination.directory,{recursive:true});
    fs.writeFileSync(path.join(destination.directory,'tvhome'),'old version');
    files[bad]=Buffer.from('corrupt download');
    await assert.rejects(install({...destination,release:'0.2.0',platform:'linux',arch:'x64',fetchBytes:async url=>files[path.basename(new URL(url).pathname)]}),/Checksum verification failed/);
    assert.equal(fs.readFileSync(path.join(destination.directory,'tvhome'),'utf8'),'old version');
    assert.ok(!fs.existsSync(destination.notices));
  }
});
test('missing release and wrong checksum filename fail without creating files', async t=>{
  const destination=paths(t),files=fixture('linux-arm64');
  await assert.rejects(install({...destination,release:'0.2.0',platform:'linux',arch:'arm64',fetchBytes:async()=>{throw new Error('Release unavailable');}}),/Release unavailable/);
  assert.ok(!fs.existsSync(destination.directory));
  assert.throws(()=>verify(files['tvhome-linux-arm64'],files['tvhome-linux-arm64.sha256'],'tvhome-linux-x64'),/Checksum/);
});
