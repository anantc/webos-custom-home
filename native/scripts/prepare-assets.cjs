// Prepare native assets from the selected local catalog.
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname, '../..');
const out = path.resolve(__dirname, '../assets');
const preview = process.argv.includes('--preview');
const profile = require('./tv-profile.cjs');
const tv = preview ? null : profile.name(process.argv[2]);
const catalogDir = preview ? path.join(root, 'catalog') : path.join(root, 'catalog/tvs', tv);
if (!preview) profile.identity(path.join(catalogDir, 'tv.json'), tv);
const ctx = {window: {}};
vm.createContext(ctx);
for (const name of ['config.js', 'presentation.js', 'layout.js']) {
  vm.runInContext(fs.readFileSync(path.join(root, 'catalog', name), 'utf8'), ctx);
}
let data;
if (preview) {
  data = {apps: ctx.window.Presentation.brands.map(b => ({id:'preview.'+b.key,title:b.name})), inputs:[]};
} else {
  const source = path.join(catalogDir, 'apps.js');
  if (!fs.existsSync(source)) throw new Error('TV catalog missing. Run tvhome --tv NAME sync from the repository root, then package again. See docs/DEVELOPMENT.md.');
  vm.runInContext(fs.readFileSync(source, 'utf8'), ctx);
  data = ctx.window.TV_APPS;
}
fs.mkdirSync(out,{recursive:true});
const files = [];
const apps = ctx.window.HomeLayout.ordered(data.apps,ctx.window.CONFIG,null,ctx.window.Presentation.forApp)
  .filter(a=>a.id!=='com.anant.tvhome.native').map((a,i)=>{
    const b = ctx.window.Presentation.forApp(a);
    const replacement = b && ['png','jpg'].map(ext => path.join(root,'native/artwork',b.key+'.'+ext)).find(p => fs.existsSync(p));
    const src = b ? 'artwork/'+b.key+'.jpg' : a.icon;
    let file = '';
    if (replacement || src) {
      const base = b ? path.join(root, 'catalog') : catalogDir;
      const source = replacement || path.resolve(base, src);
      if (!replacement && !source.startsWith(base+path.sep)) throw new Error('Invalid icon path');
      if (fs.existsSync(source)) {
        file = 'art-'+i+path.extname(source);
        fs.copyFileSync(source,path.join(out,file)); files.push(file);
      }
    }
    return {id:a.id,title:b?b.name:a.title,artwork:file,contain:replacement?false:!!(b?b.square:true),color:b?b.color:[95,113,158],background:b&&b.key==='apple-tv'?[0,0,0]:undefined};
  });
fs.copyFileSync(path.join(root,'catalog/fonts/Inter.ttf'),path.join(out,'Inter.ttf'));
fs.copyFileSync(path.join(root,'catalog/fonts/Inter-OFL.txt'),path.join(out,'Inter-OFL.txt'));
fs.writeFileSync(path.join(out,'catalog.json'),JSON.stringify({preview,apps,inputs:data.inputs||[]},null,2));
for(const f of fs.readdirSync(out)) if(f.startsWith('art-')&&!files.includes(f)) fs.unlinkSync(path.join(out,f));
console.log(`Prepared ${apps.length} ${preview?'example':'synced'} apps.`);
