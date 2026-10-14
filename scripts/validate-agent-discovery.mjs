import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, writeFile, readFile, rm, symlink } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = await mkdtemp(path.join(os.tmpdir(), 'agentupdate-discovery-real-'));
const evidence = { checkedAt: new Date().toISOString(), architecture: process.arch, isolation: 'Temporary explicit npm prefixes; sandboxed version probes; no login or model requests' };
function run(command, args, env = {}, timeout = 180_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: repo, env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '', error = '';
    child.stdout.on('data', bytes => { output += bytes; }); child.stderr.on('data', bytes => { error += bytes; });
    const timer = setTimeout(() => child.kill('SIGKILL'), timeout);
    child.on('error', e => { clearTimeout(timer); reject(e); });
    child.on('close', code => { clearTimeout(timer); code === 0 ? resolve(output) : reject(new Error(`${command} exited ${code}: ${error.slice(-1500)} ${output.slice(-1000)}`)); });
  });
}
try {
  const config = path.join(root, 'user.npmrc'), globalConfig = path.join(root, 'global.npmrc');
  await writeFile(config, ''); await writeFile(globalConfig, '');
  for (const [agent, pkg, version] of [['codex', '@openai/codex', '0.162.0'], ['gemini', '@google/gemini-cli', '0.62.0']]) {
    await run('npm', ['install', '--global', '--prefix', path.join(root, agent), `${pkg}@${version}`, '--offline', '--ignore-scripts', '--no-audit', '--no-fund', '--cache', path.join(repo, '.validation/agent-npm-cache'), '--userconfig', config, '--globalconfig', globalConfig]);
    await symlink(process.execPath, path.join(root, agent, 'bin/node'));
  }
  const installed = path.join(root, 'discovery.json'), versions = path.join(root, 'versions.json');
  await run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--release', '--locked', '--offline', '--test', 'agent_discovery', 'real_npm_instances', '--', '--ignored'], {
    AGENTUPDATE_DISCOVERY_FIXTURE: root, AGENTUPDATE_DISCOVERY_EVIDENCE: installed,
  });
  evidence.instances = JSON.parse(await readFile(installed, 'utf8'));
  assert(evidence.instances.every(instance => instance.supported && instance.versionVerified));
  await run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--release', '--locked', '--offline', '--test', 'agent_discovery', 'live_official_versions', '--', '--ignored'], { AGENTUPDATE_VERSION_EVIDENCE: versions });
  evidence.officialVersions = JSON.parse(await readFile(versions, 'utf8'));
  evidence.status = 'passed'; console.log('PASS real Codex/Gemini discovery and official version queries');
} catch (error) { evidence.status = 'failed'; evidence.error = error.message; throw error; }
finally {
  await mkdir(path.join(repo, 'docs/evidence'), { recursive: true });
  await writeFile(path.join(repo, 'docs/evidence/age-6-discovery.json'), JSON.stringify(evidence, null, 2) + '\n');
  await rm(root, { recursive: true, force: true });
}
