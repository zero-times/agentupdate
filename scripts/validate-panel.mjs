import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,readFile,writeFile,rm} from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
const repo=path.resolve('.');
const target=process.env.CARGO_TARGET_DIR||path.join(repo,'src-tauri/target');
const binary=path.join(target,'universal-apple-darwin/release/bundle/macos/AgentUpdate Validation.app/Contents/MacOS/agentupdate');
const root=await mkdtemp(path.join(os.tmpdir(),'agentupdate-panel-'));
const evidence={checkedAt:new Date().toISOString(),status:'running',limitations:['Tray callbacks are dispatched by the development probe; no physical mouse click on the macOS menu bar is asserted.','Native Apple Silicon; physical Intel and clean macOS 15 not tested.']};
try {
 const destination=path.join(root,'result.json');await mkdir(path.join(root,'data'));
 await new Promise((resolve,reject)=>{
  const child=spawn(binary,[],{env:{...process.env,HOME:root,PATH:'/usr/bin:/bin:/usr/sbin:/sbin',CODEX_HOME:'',CLAUDE_CONFIG_DIR:'',GEMINI_CLI_HOME:'',AGENTUPDATE_TEST_OFFLINE:'1',AGENTUPDATE_TEST_DATA_DIR:path.join(root,'data'),AGENTUPDATE_VALIDATION_RESULT:destination,AGENTUPDATE_VALIDATION_PANEL:'1'},stdio:['ignore','pipe','pipe']});
  let stderr='';child.stdout.on('data',()=>{});child.stderr.on('data',b=>stderr+=b);
  const timer=setTimeout(()=>{child.kill('SIGKILL');reject(new Error('Panel WebView timed out: '+stderr.slice(-1000)));},60000);
  child.on('error',e=>{clearTimeout(timer);reject(e);});child.on('close',code=>{clearTimeout(timer);code===0?resolve():reject(new Error(`Validation exited ${code}: ${stderr.slice(-1000)}`));});
 });
 evidence.result=JSON.parse(await readFile(destination,'utf8'));
 assert(Object.keys(evidence.result.checks).length>=12&&Object.values(evidence.result.checks).every(Boolean),JSON.stringify(evidence.result));
 evidence.status='passed';console.log('PASS native panel hide/show, close retention, minimized restore, busy quit guard, collapsed recovery and completed progress removal.');
}catch(error){evidence.status='failed';evidence.error=error.message;throw error;}
finally{await writeFile(path.join(repo,'docs/evidence/age-25-panel.json'),JSON.stringify(evidence,null,2)+'\n');await rm(root,{recursive:true,force:true});}
