// Developer-only SSH protocol fixture. Run in the native builder, which includes ssh2.
const fs=require('node:fs'), os=require('node:os'), path=require('node:path');
const {createHash,generateKeyPairSync}=require('node:crypto'), {spawn}=require('node:child_process');
const {Server,utils}=require(process.env.TVHOME_SSH2_MODULE || '/usr/local/lib/node_modules/@webos-tools/cli/node_modules/ssh2');
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tvhome-ssh-'));
function key(){return generateKeyPairSync('rsa',{modulusLength:2048,privateKeyEncoding:{type:'pkcs1',format:'pem'},publicKeyEncoding:{type:'spki',format:'pem'}}).privateKey;}
const host=key(), client=key(), parsed=utils.parseKey(client);
fs.writeFileSync(path.join(dir,'key'),client,{mode:0o600});
let uploaded=Buffer.alloc(0);
const {tarEntries}=require('../tv/scripts/verify-package.cjs');
const app='/media/developer/apps/usr/palm/applications/com.anant.tvhome.native';
let files=new Map(), running=true, failInstall=false;
function installPackage(bytes) {
  if(bytes.subarray(0,8).toString()!=='!<arch>\n')throw Error('invalid IPK');
  for(let offset=8;offset+60<=bytes.length;) {
    const name=bytes.subarray(offset,offset+16).toString().trim().replace(/\/$/,'');
    const size=Number(bytes.subarray(offset+48,offset+58).toString().trim());
    if(name==='data.tar.gz') {
      // Simulate replacement of the entire old app directory.
      files=new Map(tarEntries(bytes.subarray(offset+60,offset+60+size)).filter(e=>!e.name.endsWith('/')).map(e=>['/media/developer/apps/'+e.name.replace(/^\.\//,''), e.content]));
    }
    offset+=60+size+(size%2);
  }
}
function resetHome() {
  files=new Map([
    [app+'/appinfo.json',Buffer.from(JSON.stringify({id:'com.anant.tvhome.native',version:'0.0.1'}))],
    [app+'/state/preferences.json',Buffer.from(JSON.stringify({pinnedInputs:['hdmi1'],order:['hdmi1','netflix'],city:{name:'Mumbai'}}))],
    [app+'/state/speedtest-result.json',Buffer.from('{"mbps":210}')],
  ]);running=true;
}

const server=new Server({hostKeys:[host],algorithms:{serverHostKey:['ssh-rsa']}},connection=>{
  connection.on('error',()=>{});
  connection.on('authentication',ctx=>{
    if(ctx.method==='publickey' && ctx.username==='prisoner'
       && ctx.key.data.equals(parsed.getPublicSSH())
       && (!ctx.signature || parsed.verify(ctx.blob,ctx.signature,ctx.hashAlgo)===true))ctx.accept();
    else ctx.reject();
  });
  connection.on('ready',()=>connection.on('session',accept=>accept().on('exec',(accept,reject,info)=>{
    const stream=accept(), finish=(text,code=0)=>{stream.write(text);stream.exit(code);stream.end();};
    if(info.command==='fixture-reset-home') {resetHome();finish('');}
    else if(info.command==='fixture-home-running')finish(running?'running':'');
    else if(info.command==='fixture-fail-next-install') {failInstall=true;finish('');}
    else if(info.command==='if pidof tvhome-native >/dev/null; then printf running; fi')finish(running?'running':'');
    else if(info.command.startsWith('if test -e ')) {
      const file=info.command.match(/'([^']+)'/)[1];
      const data=files.get(file);
      finish(data?Buffer.concat([Buffer.from(info.command.includes('printf present')?'present':''),data]):'');
    }
    else if(info.command.startsWith('mkdir -p ') || info.command.startsWith('rm -f '))finish('');
    else if(info.command.startsWith('sha256sum '))finish(createHash('sha256').update(uploaded).digest('hex')+'  package.ipk\n');
    else if(info.command==='true')finish('');
    else if(info.command==='false')finish('fixture failure',1);
    else if(info.command.startsWith("umask 077; cat > ")) {
      const chunks=[];stream.on('data',b=>chunks.push(b));
      stream.on('end',()=>{uploaded=Buffer.concat(chunks);finish('');});
    } else if(info.command==='read-upload')finish(uploaded);
    else if(info.command.includes('luna://com.webos.applicationManager/dev/closeByAppId')) {
      running=false;finish('{"returnValue":true}');
    } else if(info.command.includes('luna://com.webos.appInstallService/dev/install')) {
      if(failInstall) {failInstall=false;finish('{"returnValue":true,"details":{"state":"FAILED"}}');}
      else {installPackage(uploaded);finish('{"returnValue":true,"details":{"state":"INSTALLED"}}');}
    } else if(info.command.includes('luna://com.webos.applicationManager/dev/listApps')) {
      finish('{"returnValue":true,"apps":[{"id":"com.anant.tvhome.native"}]}');
    } else if(info.command.includes('luna://com.webos.applicationManager/launch')) {
      running=true;finish('{"returnValue":true}');
    } else if(info.command.startsWith('/usr/bin/luna-send-pub')) {
      stream.write('{"returnValue":true,"details":{"sta');
      setTimeout(()=>{
        stream.write('te":"INSTALLING"}}\n');
        setTimeout(()=>finish(JSON.stringify({returnValue:true,details:{state:info.command.includes('fixture-fail')?'FAILED':'INSTALLED'}})),10);
      },10);
    } else reject();
  })));
});
server.listen(0,'127.0.0.1',()=>{
  const port=server.address().port;
  fs.writeFileSync(path.join(dir,'known_hosts'),'[127.0.0.1]:'+port+' ssh-rsa '+utils.parseKey(host).getPublicSSH().toString('base64')+'\n');
  fs.writeFileSync(path.join(dir,'port'),String(port));
  const child=spawn('cargo',['test','--locked','--manifest-path','cli/Cargo.toml','--features','portable,bundled-home','real_ssh_fixture','--','--ignored','--nocapture','--test-threads=1'],
    {cwd:path.resolve(__dirname,'..'),stdio:'inherit',env:{...process.env,TVHOME_SSH_FIXTURE:dir}});
  child.on('exit',code=>{server.close();fs.rmSync(dir,{recursive:true,force:true});process.exit(code??1);});
});
