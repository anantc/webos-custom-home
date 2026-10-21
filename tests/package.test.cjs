const test = require('node:test');
const assert = require('node:assert/strict');
const { gzipSync } = require('node:zlib');
const { tarEntries, verify } = require('../tv/scripts/verify-package.cjs');
function fixture() {
  const png = size => { const b = Buffer.alloc(24); Buffer.from([137,80,78,71,13,10,26,10]).copy(b); b.writeUInt32BE(size,16); b.writeUInt32BE(size,20); return b; };
  const elf = Buffer.alloc(40); Buffer.from([127,69,76,70,1,1]).copy(elf); elf.writeUInt16LE(40,18); elf.writeUInt32LE(0x200,36);
  const files = {
    'appinfo.json': JSON.stringify({id:'com.anant.tvhome.native',type:'native',icon:'icon.png',largeIcon:'large.png',iconColor:'#000000'}),
    'icon.png':png(80),'large.png':png(130),'state/':'',
    'assets/catalog.json':JSON.stringify({preview:false,apps:[{id:'native.netflix'}]}),
    'bin/tvhome-native':elf,'lib/libSDL3.so.0':'\x7fELF','lib/libfreetype.so.6':'\x7fELF',
    'licenses/SDL3-LICENSE.txt':'license','assets/Inter-OFL.txt':'font license',
  };
  return {data:Object.entries(files).map(([name,value])=>({name:'usr/palm/applications/com.anant.tvhome.native/'+name,mode:name.endsWith('/')?0o777:name.startsWith('bin/')?0o755:0o644,content:Buffer.from(value)})),control:[{name:'./control',content:Buffer.from('Architecture: arm\n')}]};
}
test('package accepts real ARM soft-float app with required icons and notices', () => {
  const {data,control}=fixture();assert.deepEqual(verify(data,control),{id:'com.anant.tvhome.native',apps:1});
});
test('package rejects preview IDs, missing permissions, wrong ABI and missing licenses', () => {
  for(const change of [
    data=>{data.find(e=>e.name.endsWith('catalog.json')).content=Buffer.from(JSON.stringify({preview:true,apps:[{id:'preview.netflix'}]}));},
    data=>{data.find(e=>e.name.endsWith('bin/tvhome-native')).mode=0o644;},
    data=>{data.find(e=>e.name.endsWith('state/')).mode=0o755;},
    data=>{data.splice(data.findIndex(e=>e.name.endsWith('state/')),1);},
    data=>{data.find(e=>e.name.endsWith('bin/tvhome-native')).content.writeUInt32LE(0x400,36);},
    data=>{data.splice(data.findIndex(e=>e.name.endsWith('Inter-OFL.txt')),1);},
  ]) {const {data,control}=fixture();change(data);assert.throws(()=>verify(data,control));}
});
test('tar reader validates checksums and bounds without extracting files', () => {
  const header=Buffer.alloc(512);
  header.write('../never-extracted');header.write('0000755\0',100);header.write('00000000003\0',124);header.fill(32,148,156);
  header.write([...header].reduce((a,b)=>a+b,0).toString(8).padStart(6,'0')+'\0 ',148);
  const archive=Buffer.concat([header,Buffer.from('abc'),Buffer.alloc(509),Buffer.alloc(1024)]);
  const entries=tarEntries(gzipSync(archive));assert.equal(entries[0].name,'../never-extracted');assert.equal(entries[0].content.toString(),'abc');
  archive[0]=0;assert.throws(()=>tarEntries(gzipSync(archive)),/checksum/);
});
