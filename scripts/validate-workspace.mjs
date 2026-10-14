import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import os from 'node:os';import path from 'node:path';import assert from 'node:assert/strict';
const root=await mkdtemp(path.join(os.tmpdir(),'agentupdate-ui-'));
try {
 const executable=path.resolve('experiments/desktop-spike/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/AgentUpdate Validation.app/Contents/MacOS/agentupdate');
 const destination=path.join(root,'result.json');
 await new Promise((resolve,reject)=>{const child=spawn(executable,[],{env:{...process.env,AGENTUPDATE_TEST_DATA_DIR:path.join(root,'app-data'),AGENTUPDATE_VALIDATION_RESULT:destination},stdio:'ignore'});const timer=setTimeout(()=>{child.kill('SIGKILL');reject(new Error('Desktop validation timed out'));},45000);child.on('error',reject);child.on('close',code=>{clearTimeout(timer);code===0?resolve():reject(new Error(`Desktop exited ${code}`));});});
 const result=JSON.parse(await readFile(destination,'utf8'));assert(Object.values(result.checks).every(v=>v===true));
 await writeFile('docs/evidence/age-11-workspace.json',JSON.stringify({checkedAt:new Date().toISOString(),architecture:process.arch,checks:result.checks,scope:'Real React UI and restricted Rust IPC; existing discovery is read-only. Update/rollback service verified separately by isolated real roundtrip; complete UI operation path will be exercised in AGE-15.'},null,2)+'\n');
 console.log('PASS actual desktop render and IPC boundary');
}finally{await rm(root,{recursive:true,force:true});}
