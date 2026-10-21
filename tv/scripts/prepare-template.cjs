// Build a TV-independent payload; tvhome supplies the live catalog before installation.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname, '../..');
const out = path.join(root, 'tv/assets');
const ctx = {window: {}};
vm.createContext(ctx);
for (const name of ['config.js', 'presentation.js']) {
  vm.runInContext(fs.readFileSync(path.join(root, 'catalog', name), 'utf8'), ctx);
}
fs.rmSync(out, {recursive:true, force:true});
fs.mkdirSync(out, {recursive:true});
const brands = ctx.window.Presentation.brands.map(b => {
  const replacement = ['png','jpg'].map(ext=>path.join(root,'tv/artwork',b.key+'.'+ext)).find(fs.existsSync);
  const source = replacement || path.join(root,'catalog/artwork',b.key+'.jpg');
  const artwork = 'brand-'+b.key+path.extname(source);
  fs.copyFileSync(source,path.join(out,artwork));
  return {key:b.key,name:b.name,pattern:(b.match.ignoreCase?'(?i)':'')+b.match.source,
    artwork,contain:replacement?false:!!b.square,color:b.color,
    background:b.key==='apple-tv'?[0,0,0]:null};
});
for(const name of ['Inter.ttf','Inter-OFL.txt']) fs.copyFileSync(path.join(root,'catalog/fonts',name),path.join(out,name));
fs.writeFileSync(path.join(out,'catalog.json'),JSON.stringify({preview:false,setupTemplate:true,apps:[],inputs:[]},null,2));
fs.writeFileSync(path.join(out,'setup.json'),JSON.stringify({schema:1,config:ctx.window.CONFIG,brands},null,2));
console.log('Prepared universal Home setup template.');
