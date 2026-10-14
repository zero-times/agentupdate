import { spawn } from 'node:child_process';
import { mkdir,mkdtemp,writeFile,readFile,rm,symlink } from 'node:fs/promises';
import path from 'node:path';import os from 'node:os';import {fileURLToPath} from 'node:url';import {createRequire} from 'node:module';
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const root=await mkdtemp(path.join(os.tmpdir(),'agentupdate-workflow-real-'));
const phase=process.argv[2]??'single';
const evidence={checkedAt:new Date().toISOString(),architecture:process.arch,isolation:'Temporary explicit prefixes and artificial configuration; no daily Agent modifications or model requests',phase,artifactSource:'Previously fetched official npm cache; source URL and SHA512 revalidated by the production updater'};
function run(executable,args,env={},timeout=600000){return new Promise((resolve,reject)=>{const child=spawn(executable,args,{cwd:repo,env:{...process.env,...env},stdio:['ignore','pipe','pipe']});let output='',error='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>error+=b);const timer=setTimeout(()=>child.kill('SIGKILL'),timeout);child.on('error',e=>{clearTimeout(timer);reject(e)});child.on('close',code=>{clearTimeout(timer);code===0?resolve(output):reject(new Error(`Validation exited ${code}: ${error.slice(-1200)} ${output.slice(-2000)}`))});});}
try{
 const config=path.join(root,'user.npmrc'),globalConfig=path.join(root,'global.npmrc');await writeFile(config,'');await writeFile(globalConfig,'');
 for(const [agent,pkg,version] of [['codex','@openai/codex','0.162.0'],['gemini','@google/gemini-cli','0.62.0']]){
  await run('npm',['install','--global','--prefix',path.join(root,agent),`${pkg}@${version}`,'--offline','--ignore-scripts','--no-audit','--no-fund','--cache',path.join(repo,'.validation/agent-npm-cache'),'--userconfig',config,'--globalconfig',globalConfig],{HOME:root});await symlink(process.execPath,path.join(root,agent,'bin/node'));
 }
 await mkdir(path.join(root,'targets'));
 for(const [agent,pkg,version] of [['codex','@openai/codex','0.162.1'],['gemini','@google/gemini-cli','0.63.0']]){
  const npmRoot=(await run('npm',['root','--global'])).trim();
  const cacache=createRequire(path.join(npmRoot,'npm/package.json'))('cacache');
  const cache=path.join(repo,'.validation/agent-npm-cache/_cacache');
  const metadata=JSON.parse((await cacache.get(cache,'make-fetch-happen:request-cache:https://registry.npmjs.org/'+pkg.replace('/','%2f'))).data);
  const dist=metadata.versions[version].dist;
  const bytes=(await cacache.get(cache,'make-fetch-happen:request-cache:'+dist.tarball)).data;
  await writeFile(path.join(root,'targets',agent+'.json'),JSON.stringify({version,prerelease:false,tarball:dist.tarball,integrity:dist.integrity}));
  await writeFile(path.join(root,'targets',agent+'.tgz'),bytes);
 }
 for(const [directory,name,content] of [['.codex','config.toml','model = "fixture"\n'],['.gemini','settings.json','{"fixture":true}\n']]){await mkdir(path.join(root,directory));await writeFile(path.join(root,directory,name),content,{mode:0o640});}
 const results=path.join(root,'results.json');
 await run(path.join(repo,'.validation/cargo-run'),['test','--manifest-path','src-tauri/Cargo.toml','--release','--locked','--offline',...(phase==='faults'?['--features','desktop-validation']:[]),'--test','npm_updates',phase==='single'?'real_single_updates':phase==='batch'?'real_batch':phase==='faults'?'real_faults':'real_roundtrip','--','--ignored','--nocapture'],{AGENTUPDATE_DISCOVERY_FIXTURE:root,AGENTUPDATE_NPM_CACHE:path.join(repo,'.validation/agent-npm-cache'),AGENTUPDATE_UPDATE_EVIDENCE:results,HTTPS_PROXY:'http://127.0.0.1:7890'});
 evidence.results=JSON.parse(await readFile(results,'utf8'));evidence.status='passed';console.log(`PASS real Codex/Gemini ${phase} workflow`);
}catch(error){evidence.status='failed';evidence.error=error.message;throw error;}
finally{await mkdir(path.join(repo,'docs/evidence'),{recursive:true});await writeFile(path.join(repo,'docs/evidence',phase==='single'?'age-9-updates.json':phase==='batch'?'age-12-batch.json':phase==='faults'?'age-15-real-faults.json':'age-10-rollback.json'),JSON.stringify(evidence,null,2)+'\n');await rm(root,{recursive:true,force:true});}
