const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const profile = require('../tv/scripts/tv-profile.cjs');
const root = path.resolve(__dirname, '..');
test('installation rejects a default, mismatched or ambiguous CLI TV', () => {
  const expected = {name:'bedroom',host:'192.0.2.11'};
  const device = {name:'bedroom',deviceinfo:{ip:'192.0.2.11',port:'9922',user:'prisoner'},details:{passphrase:'private-fixture'}};
  assert.equal(profile.device([device], expected), device);
  assert.throws(()=>profile.device([{...device,name:'living-room'}], expected));
  assert.throws(()=>profile.device([{...device,deviceinfo:{...device.deviceinfo,ip:'192.0.2.10'}}], expected));
  assert.throws(()=>profile.device([{...device,deviceinfo:{...device.deviceinfo,port:22}}], expected));
  assert.throws(()=>profile.device([device,device], expected));
  for (const name of ['', '../bedroom', 'Living Room', '-tv']) assert.throws(()=>profile.name(name));
});
test('asset preparation selects each TV catalog and its own fallback icons', t => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(),'tvhome-catalog-test-'));
  t.after(()=>fs.rmSync(tmp,{recursive:true,force:true}));
  for (const dir of ['tv/scripts','tv/artwork','catalog/fonts']) fs.mkdirSync(path.join(tmp,dir),{recursive:true});
  for (const file of ['tv/scripts/prepare-assets.cjs','tv/scripts/tv-profile.cjs','catalog/config.js','catalog/layout.js','catalog/presentation.js']) fs.copyFileSync(path.join(root,file),path.join(tmp,file));
  for (const file of ['Inter.ttf','Inter-OFL.txt']) fs.writeFileSync(path.join(tmp,'catalog/fonts',file),'font fixture');
  for (const name of ['living-room','bedroom']) {
    const dir = path.join(tmp,'catalog/tvs',name); fs.mkdirSync(path.join(dir,'icons'),{recursive:true});
    fs.writeFileSync(path.join(dir,'tv.json'), JSON.stringify({name,host:name+'.example'}));
    fs.writeFileSync(path.join(dir,'icons/fallback.png'), name);
    fs.writeFileSync(path.join(dir,'apps.js'),'window.TV_APPS = '+JSON.stringify({apps:[{id:name+'.app',title:name,icon:'icons/fallback.png'}],inputs:[]}));
    execFileSync(process.execPath,[path.join(tmp,'tv/scripts/prepare-assets.cjs'),name]);
    const catalog=JSON.parse(fs.readFileSync(path.join(tmp,'tv/assets/catalog.json')));
    assert.equal(catalog.apps[0].id,name+'.app');
    assert.equal(fs.readFileSync(path.join(tmp,'tv/assets',catalog.apps[0].artwork),'utf8'),name);
  }
  assert.throws(()=>profile.identity(path.join(tmp,'catalog/tvs/bedroom/tv.json'),'living-room'));
  assert.throws(()=>execFileSync(process.execPath,[path.join(tmp,'tv/scripts/prepare-assets.cjs'),'missing'],{stdio:'pipe'}));
});

test('installer sends only the chosen package and does not install on a target mismatch', t => {
  const tmp=fs.mkdtempSync(path.join(os.tmpdir(),'tvhome-install-test-'));
  t.after(()=>fs.rmSync(tmp,{recursive:true,force:true}));
  const name='bedroom';
  for(const dir of ['tv/scripts','tv/dist/bedroom','catalog/tvs/bedroom','mock-bin']) fs.mkdirSync(path.join(tmp,dir),{recursive:true});
  for(const file of ['tv/install.sh','tv/scripts/tv-profile.cjs']) fs.copyFileSync(path.join(root,file),path.join(tmp,file));
  const identity={name,host:'192.0.2.11'};
  for(const dir of ['tv/dist/bedroom','catalog/tvs/bedroom']) fs.writeFileSync(path.join(tmp,dir,'tv.json'),JSON.stringify(identity));
  fs.writeFileSync(path.join(tmp,'tv/dist/bedroom/com.anant.tvhome.native_0.1.6_arm.ipk'),'fixture');
  const devices=path.join(tmp,'devices.json'), log=path.join(tmp,'commands.jsonl');
  for(const action of ['setup-device','install','launch']) {
    const code=action==='setup-device'
      ? 'process.stdout.write(fs.readFileSync(process.env.TVHOME_TEST_DEVICES));'
      : 'fs.appendFileSync(process.env.TVHOME_TEST_LOG,JSON.stringify([process.argv[1],...process.argv.slice(2)])+"\\n");';
    fs.writeFileSync(path.join(tmp,'mock-bin','ares-'+action),'#!'+process.execPath+'\nconst fs=require("node:fs");\n'+code,{mode:0o755});
  }
  const env={...process.env,PATH:path.join(tmp,'mock-bin')+path.delimiter+process.env.PATH,TVHOME_TEST_DEVICES:devices,TVHOME_TEST_LOG:log};
  const d={name,deviceinfo:{ip:identity.host,port:'9922',user:'prisoner'}};
  fs.writeFileSync(devices,JSON.stringify([d]));
  execFileSync('bash',[path.join(tmp,'tv/install.sh'),name],{env,stdio:'pipe'});
  const calls=fs.readFileSync(log,'utf8');
  assert.ok(calls.includes('dist/bedroom/com.anant.tvhome.native_0.1.6_arm.ipk'));
  assert.ok(calls.includes('"-d","bedroom"'));
  fs.unlinkSync(log);
  fs.writeFileSync(devices,JSON.stringify([{...d,deviceinfo:{...d.deviceinfo,ip:'192.0.2.10'}}]));
  assert.throws(()=>execFileSync('bash',[path.join(tmp,'tv/install.sh'),name],{env,stdio:'pipe'}));
  assert.ok(!fs.existsSync(log),'no install or launch after a mismatched TV');
});
