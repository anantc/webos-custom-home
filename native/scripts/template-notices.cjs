// Collect the installed TV payload's notices alongside the generic release template.
const fs=require('node:fs'), path=require('node:path');
const stage=process.argv[2], output=process.argv[3], sections=[];
function add(relative) {
  const full=path.join(stage,relative);
  if (fs.statSync(full).isDirectory()) {
    for (const name of fs.readdirSync(full).sort()) add(path.join(relative,name));
  } else sections.push('\n=== Home TV package: '+relative+' ===\n'+fs.readFileSync(full,'utf8'));
}
for (const name of ['LICENSE','NOTICE','licenses','vendor','assets/Inter-OFL.txt','artwork-sources.json']) add(name);
fs.writeFileSync(output,sections.join('\n'));
