import {spawn} from 'node:child_process';import {mkdir,mkdtemp,writeFile,readFile,rm,symlink,readdir,stat} from 'node:fs/promises';import {createHash} from 'node:crypto';import {createRequire} from 'node:module';import path from 'node:path';import os from 'node:os';import assert from 'node:assert/strict';
const repo=path.resolve('.'),arch=process.argv[2]??'arm64';
const root=await mkdtemp(path.join(os.tmpdir(),`agentupdate-ui-${arch}-`));
const node=arch==='x86_64'?path.join(repo,'.validation/node-x64/node-v22.20.0-darwin-x64/bin/node'):process.execPath;
const npm=arch==='x86_64'?path.join(repo,'.validation/node-x64/node-v22.20.0-darwin-x64/lib/node_modules/npm/bin/npm-cli.js'):(await import('node:child_process')).execFileSync('which',['npm'],{encoding:'utf8'}).trim();
const cache=path.join(repo,'.validation/agent-npm-cache');
const env={...process.env,HOME:root,npm_config_arch:arch==='x86_64'?'x64':'arm64',npm_config_cpu:arch==='x86_64'?'x64':'arm64',npm_config_os:'darwin'};delete env.CODEX_HOME;delete env.GEMINI_CLI_HOME;
function run(exe,args,extra={},timeout=600000){return new Promise((resolve,reject)=>{const child=spawn(exe,args,{cwd:repo,env:{...env,...extra},stdio:['ignore','pipe','pipe']});let output='',error='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>error+=b);const timer=setTimeout(()=>{child.kill('SIGKILL');reject(new Error('Timed out'));},timeout);child.on('error',e=>{clearTimeout(timer);reject(e);});child.on('close',code=>{clearTimeout(timer);code===0?resolve(output):reject(new Error(`Process exited ${code}: ${error.slice(-1000)} ${output.slice(-2000)}`));});});}
async function tree(root){const entries=[];async function visit(rel){const current=path.join(root,rel);for(const entry of await readdir(current,{withFileTypes:true})){const name=path.join(rel,entry.name);const full=path.join(root,name);if(entry.isDirectory()){entries.push([name,'dir',(await stat(full)).mode&0o777]);await visit(name);}else if(entry.isSymbolicLink()){entries.push([name,'link',await(await import('node:fs/promises')).readlink(full)]);}else{entries.push([name,'file',(await stat(full)).mode&0o777,createHash('sha256').update(await readFile(full)).digest('hex')]);}}}await visit('');return entries.sort((a,b)=>a[0].localeCompare(b[0]));}
const evidence={checkedAt:new Date().toISOString(),architecture:arch,execution:arch==='x86_64'?'Rosetta on Apple Silicon':'native Apple Silicon',catalogSource:'Pinned previously fetched official npm metadata and artifacts, revalidated by production code',isolation:'Temporary HOME, explicit npm prefixes, artificial configuration; no daily Agent changes'};
try{
 assert.equal((await run(node,['-p','process.arch'])).trim(),arch==='x86_64'?'x64':'arm64');
 const userConfig=path.join(root,'user.npmrc'),globalConfig=path.join(root,'global.npmrc');await writeFile(userConfig,'');await writeFile(globalConfig,'');await mkdir(path.join(root,'catalog'));
 const npmRoot=(await import('node:child_process')).execFileSync('npm',['root','--global'],{encoding:'utf8'}).trim();
 const cacache=createRequire(path.join(npmRoot,'npm/package.json'))('cacache');
 const before={};
 for(const [agent,pkg,old,target] of [['codex','@openai/codex','0.162.0','0.162.1'],['gemini','@google/gemini-cli','0.62.0','0.63.0']]){
  await run(node,[npm,'install','--global','--prefix',path.join(root,agent),`${pkg}@${old}`,'--offline','--ignore-scripts','--no-audit','--no-fund','--cache',cache,'--userconfig',userConfig,'--globalconfig',globalConfig,'--cpu',arch==='x86_64'?'x64':'arm64','--os','darwin']);await symlink(node,path.join(root,agent,'bin/node'));
  const metadata=JSON.parse((await cacache.get(path.join(cache,'_cacache'),'make-fetch-happen:request-cache:https://registry.npmjs.org/'+pkg.replace('/','%2f'))).data);
  const version=metadata.versions[target],oldVersion=metadata.versions[old];
  await writeFile(path.join(root,'catalog',agent+'.json'),JSON.stringify({name:pkg,'dist-tags':{latest:target},versions:{[old]:oldVersion,[target]:version}}));
  const bytes=(await cacache.get(path.join(cache,'_cacache'),'make-fetch-happen:request-cache:'+version.dist.tarball)).data;await writeFile(path.join(root,'catalog',agent+'.tgz'),bytes);
  before[agent]=await tree(path.join(root,agent,'lib/node_modules',pkg));
 }
 for(const [dir,name,content] of [['.codex','config.toml','model = "fixture"\n'],['.gemini','settings.json','{"fixture":true}\n']]){await mkdir(path.join(root,dir));await writeFile(path.join(root,dir,name),content,{mode:0o640});}
 const data=path.join(root,'app-data');await mkdir(data);const now=Date.now(),tag='v0.10.0',name='AgentUpdate-0.10.0-macos-universal.dmg';
 await writeFile(path.join(data,'release-cache.json'),JSON.stringify({schemaVersion:1,checkedAt:now,successfulAt:now,latest:{version:'0.10.0',tag,publishedAt:'2026-10-12T00:00:00Z',notes:'Synthetic newer release for UI check. No public release created.',url:'https://github.com/zero-times/agentupdate/releases/tag/'+tag,assets:[{name,url:'https://github.com/zero-times/agentupdate/releases/download/'+tag+'/'+name}]},etag:null,errorCode:null,retryAfter:0}),{mode:0o600});
 const destination=path.join(root,'result.json');const binary=path.join(repo,'experiments/desktop-spike/src-tauri/target/universal-apple-darwin/release/bundle/macos/AgentUpdate Validation.app/Contents/MacOS/agentupdate');
 await run('/usr/bin/arch',[arch==='x86_64'?'-x86_64':'-arm64',binary],{PATH:[path.join(root,'codex/bin'),path.join(root,'gemini/bin'),path.dirname(node),'/usr/bin','/bin'].join(':'),AGENTUPDATE_TEST_DATA_DIR:data,AGENTUPDATE_TEST_CATALOG_DIR:path.join(root,'catalog'),AGENTUPDATE_TEST_OFFLINE:'1',AGENTUPDATE_NPM_CACHE:cache,AGENTUPDATE_VALIDATION_RESULT:destination,AGENTUPDATE_VALIDATION_WORKFLOW:'1'},480000);
 const result=JSON.parse(await readFile(destination,'utf8'));assert.equal(result.architecture,arch==='x86_64'?'x86_64':'aarch64');assert(Object.values(result.checks).every(v=>v===true));assert.equal(result.transactions.length,2);assert(result.transactions.every(tx=>tx.stage==='restored'));
 for(const [agent,pkg] of [['codex','@openai/codex'],['gemini','@google/gemini-cli']])assert.deepEqual(await tree(path.join(root,agent,'lib/node_modules',pkg)),before[agent]);
 assert.equal(await readFile(path.join(root,'.codex/config.toml'),'utf8'),'model = "fixture"\n');assert.equal(await readFile(path.join(root,'.gemini/settings.json'),'utf8'),'{"fixture":true}\n');
 evidence.result=result;evidence.originalPackageTreesExact=true;evidence.status='passed';console.log(`PASS real React UI selects, confirms batch, updates, restores both tools, exports and checks retention/release (${arch})`);
}catch(error){evidence.status='failed';evidence.error=error.message;throw error;}
finally{await writeFile(path.join(repo,`docs/evidence/age-15-ui-${arch}.json`),JSON.stringify(evidence,null,2)+'\n');await rm(root,{recursive:true,force:true});}
