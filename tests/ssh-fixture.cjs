// Developer-only SSH protocol fixture. Run in the native builder, which includes ssh2.
const fs=require('node:fs'), os=require('node:os'), path=require('node:path');
const {generateKeyPairSync}=require('node:crypto'), {spawn}=require('node:child_process');
const {Server,utils}=require(process.env.TVHOME_SSH2_MODULE || '/usr/local/lib/node_modules/@webos-tools/cli/node_modules/ssh2');
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'tvhome-ssh-'));
function key(){return generateKeyPairSync('rsa',{modulusLength:2048,privateKeyEncoding:{type:'pkcs1',format:'pem'},publicKeyEncoding:{type:'spki',format:'pem'}}).privateKey;}
const host=key(), client=key(), parsed=utils.parseKey(client);
fs.writeFileSync(path.join(dir,'key'),client,{mode:0o600});
let uploaded=Buffer.alloc(0);
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
    if(info.command==='true')finish('');
    else if(info.command==='false')finish('fixture failure',1);
    else if(info.command.startsWith("umask 077; cat > ")) {
      const chunks=[];stream.on('data',b=>chunks.push(b));
      stream.on('end',()=>{uploaded=Buffer.concat(chunks);finish('');});
    } else if(info.command==='read-upload')finish(uploaded);
    else if(info.command.startsWith('/usr/bin/luna-send-pub')) {
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
  const child=spawn('cargo',['test','--locked','--manifest-path','cli/Cargo.toml','--features','portable,bundled-home','ssh::tests::real_ssh_fixture','--','--ignored','--nocapture'],
    {cwd:path.resolve(__dirname,'..'),stdio:'inherit',env:{...process.env,TVHOME_SSH_FIXTURE:dir}});
  child.on('exit',code=>{server.close();fs.rmSync(dir,{recursive:true,force:true});process.exit(code??1);});
});
