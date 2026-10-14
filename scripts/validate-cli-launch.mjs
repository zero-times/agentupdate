import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, realpath, readFile, writeFile, rm, access } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const target = process.env.CARGO_TARGET_DIR || path.join(repo, 'src-tauri/target');
const binary = process.env.AGENTUPDATE_LAUNCH_VALIDATION_BINARY || path.join(target, 'release/bundle/macos/AgentUpdate Validation.app/Contents/MacOS/agentupdate');
const root = await realpath(await mkdtemp(path.join(os.tmpdir(), 'agentupdate-launch-ui-')));
const first = path.join(root, 'first project');
const second = path.join(root, '中文 project \' " ; $(touch unsafe) `touch unsafe`');
const resultFile = path.join(root, 'result.json');
const evidence = { checkedAt: new Date().toISOString(), status: 'running', limitations: ['Native Apple Silicon macOS; physical Intel not tested. Starts an isolated fixture CLI, not a live Agent.'] };
try {
  await mkdir(path.join(root, '.local/bin'), {recursive:true});
  await mkdir(first); await mkdir(second);
  await writeFile(path.join(root, '.local/bin/codex'), '#!/bin/sh\nif [ "$1" = "--version" ]; then printf "codex-cli 1.0.0\\n"; else printf "%s" "$PWD" > launched-cwd; fi\n', {mode:0o700});
  await writeFile(path.join(root, '.local/bin/dsh'), '#!/bin/sh\nif [ "$1" = "--version" ]; then printf "dsh 1.0.0\\n"; else [ "$1" = "--profile" ] && [ -n "$2" ] || exit 2; printf "%s\\n" "$2" >> launched-profiles; fi\n', {mode:0o700});
  await new Promise((resolve, reject) => {
    const child = spawn(binary, [], {env: {...process.env, HOME:root, PATH:'/usr/bin:/bin:/usr/sbin:/sbin', CODEX_HOME:'', GEMINI_CLI_HOME:'', CLAUDE_CONFIG_DIR:'', AGENTUPDATE_TEST_DATA_DIR:path.join(root,'data'), AGENTUPDATE_TEST_DISCOVERY_DIR:path.join(root,'.local/bin'), AGENTUPDATE_TEST_OFFLINE:'1', AGENTUPDATE_VALIDATION_RESULT:resultFile, AGENTUPDATE_VALIDATION_CLI_LAUNCH:'1'}, stdio:['ignore','pipe','pipe']});
    let stderr = ''; child.stderr.on('data', b => {stderr += b;}); child.stdout.on('data', () => {});
    const timer = setTimeout(() => {child.kill('SIGKILL');reject(new Error('CLI launch WebView timed out: '+stderr.slice(-1000)));},120000);
    child.on('error', error => {clearTimeout(timer);reject(error);});
    child.on('close', code => {clearTimeout(timer);code === 0 ? resolve() : reject(new Error('Validation app exited '+code+': '+stderr.slice(-1000)));});
  });
  const result = JSON.parse(await readFile(resultFile, 'utf8'));
  assert(Object.values(result.checks).every(v => v === true));
  const deadline = Date.now()+10000;
  while (true) {
    try { assert.equal(await readFile(path.join(second,'launched-cwd'),'utf8'),second); break; }
    catch (error) { if (Date.now()>deadline) throw error; await new Promise(resolve => setTimeout(resolve,100)); }
  }
  for (const directory of [root,first,second]) await assert.rejects(access(path.join(directory,'unsafe')));
  assert.deepEqual(JSON.parse(await readFile(path.join(root,'data/cli-projects.json'),'utf8')),[second]);
  assert.equal(await readFile(path.join(second,'launched-profiles'),'utf8'), 'web\nmy-tui\n');
  evidence.result = result; evidence.nativeTerminalCwdExact = true; evidence.deepseekProfileArgumentsExact = true; evidence.noShellInjection = true; evidence.status = 'passed';
  console.log('PASS real WebView: recent projects, editing, adding, invalid paths, exact CLI instance, native Terminal cwd and special characters');
} catch (error) {evidence.status='failed';evidence.error=error.message;throw error;}
finally {
  await writeFile(path.join(repo,'docs/evidence/age-24-cli-launch.json'),JSON.stringify(evidence,null,2)+'\n');
  await rm(root,{recursive:true,force:true});
}
