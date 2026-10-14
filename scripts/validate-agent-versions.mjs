import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cp, mkdir, mkdtemp, readFile, writeFile, chmod, lstat, rm, readdir, readlink } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

// Research harness, not a production updater. It only manages its own temporary prefixes.
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
if (process.platform !== 'darwin') throw new Error('This validation requires macOS sandbox-exec.');
const root = await mkdtemp(path.join(os.tmpdir(), 'agentupdate-age3-'));
const output = path.join(repo, 'docs/evidence/age-3-arm64.json');
const tools = [
  { id: 'codex', pkg: '@openai/codex', old: '0.162.0', next: '0.162.1', dir: '.codex', file: 'config.toml', content: 'model = "test-fixture"\n' },
  { id: 'gemini', pkg: '@google/gemini-cli', old: '0.62.0', next: '0.63.0', dir: '.gemini', file: 'settings.json', content: '{"ui":{"theme":"Default"}}\n' },
];
const evidence = { date: '2026-10-12', platform: process.platform, architecture: process.arch,
  node: process.version, sandbox: 'deny network and real user Agent configuration access during probes',
  configuration: 'synthetic fixtures only; no login or inference', results: [] };
const env = { PATH: `${path.dirname(process.execPath)}:/usr/bin:/bin:/usr/sbin:/sbin`,
  HOME: os.homedir(), TMPDIR: root, LANG: 'en_US.UTF-8', CI: '1', NO_COLOR: '1' };
for (const key of ['HTTPS_PROXY', 'HTTP_PROXY', 'https_proxy', 'http_proxy', 'NO_PROXY']) {
  if (process.env[key]) env[key] = process.env[key];
}
const emptyConfig = path.join(root, 'empty.npmrc');
await writeFile(emptyConfig, '');
const globalConfig = path.join(root, 'global.npmrc');
await writeFile(globalConfig, '');
const profile = path.join(root, 'probe.sb');
const quote = s => JSON.stringify(s);
await writeFile(profile, `(version 1)\n(allow default)\n(deny network*)\n(deny file-write* (subpath ${quote(os.homedir())}))\n` +
  ['.codex', '.gemini', '.claude', '.claude.json', '.npmrc', '.ssh', '.config'].map(p =>
    `(deny file-read* (subpath ${quote(path.join(os.homedir(), p))}))`).join('\n') + '\n');

async function run(executable, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { cwd: root, env: { ...env, ...options.env }, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', b => { stdout += b; });
    child.stderr.on('data', b => { stderr += b; });
    const timer = setTimeout(() => child.kill('SIGTERM'), options.timeout ?? 300_000);
    child.on('error', reject);
    child.on('close', code => {
      clearTimeout(timer);
      if (code !== 0) reject(new Error(`${path.basename(executable)} exited ${code}: ${stderr.slice(-2000)}`));
      else resolve({ stdout: stdout.trim(), stderr: stderr.trim() });
    });
  });
}
async function inventory(dir, base = dir) {
  const entries = [];
  for (const name of (await readdir(dir)).sort()) {
    const file = path.join(dir, name), stat = await lstat(file), relative = path.relative(base, file);
    if (stat.isSymbolicLink()) entries.push({ path: relative, type: 'link', target: await readlink(file) });
    else if (stat.isDirectory()) entries.push({ path: relative, type: 'directory', mode: stat.mode & 0o777 }, ...await inventory(file, base));
    else entries.push({ path: relative, type: 'file', mode: stat.mode & 0o777,
      sha256: createHash('sha256').update(await readFile(file)).digest('hex') });
  }
  return entries;
}
async function saveEvidence() {
  await mkdir(path.dirname(output), { recursive: true });
  await writeFile(output, JSON.stringify(evidence, null, 2) + '\n');
}

try {
  for (const tool of tools) {
    const start = performance.now(), working = path.join(root, tool.id), prefix = path.join(working, 'prefix');
    const configRoot = path.join(working, 'fixture'), configDir = path.join(configRoot, tool.dir);
    await mkdir(configDir, { recursive: true, mode: 0o700 });
    await chmod(configDir, 0o700);
    const configFile = path.join(configDir, tool.file);
    await writeFile(configFile, tool.content, { mode: 0o600 });
    const before = await inventory(configRoot);
    const cache = path.join(repo, '.validation', 'agent-npm-cache');
    const install = version => run('npm', ['install', '--global', '--prefix', prefix,
      `${tool.pkg}@${version}`, '--registry=https://registry.npmjs.org', '--ignore-scripts',
      '--no-audit', '--no-fund', '--fetch-retries=2', '--fetch-timeout=30000',
      '--fetch-retry-mintimeout=1000', '--fetch-retry-maxtimeout=4000',
      '--cache', cache, '--userconfig', emptyConfig, '--globalconfig', globalConfig]);
    const probe = args => run('/usr/bin/sandbox-exec', ['-f', profile, path.join(prefix, 'bin', tool.id), ...args],
      { timeout: 30_000, env: { GEMINI_CLI_HOME: configRoot } });
    console.log(`${tool.id}: installing ${tool.old} in isolated prefix`);
    await install(tool.old);
    const oldVersion = (await probe(['--version'])).stdout;
    assert(oldVersion.includes(tool.old), 'Original executable version mismatch');
    await probe(['--help']);
    const versionMetadata = JSON.parse((await run('npm', ['view', `${tool.pkg}@${tool.old}`, 'dist', '--json',
      '--registry=https://registry.npmjs.org', '--cache', cache, '--userconfig', emptyConfig, '--globalconfig', globalConfig])).stdout);
    const backup = path.join(working, 'backup');
    await mkdir(backup);
    await cp(prefix, path.join(backup, 'prefix'), { recursive: true, verbatimSymlinks: true, preserveTimestamps: true });
    await cp(configRoot, path.join(backup, 'config'), { recursive: true, verbatimSymlinks: true, preserveTimestamps: true });
    assert.deepEqual(await inventory(path.join(backup, 'config')), before);
    const originalInstall = await inventory(prefix);
    assert.deepEqual(await inventory(path.join(backup, 'prefix')), originalInstall);
    console.log(`${tool.id}: updating to ${tool.next}`);
    await install(tool.next);
    const newVersion = (await probe(['--version'])).stdout;
    assert(newVersion.includes(tool.next), 'Updated executable version mismatch');
    assert.deepEqual(await inventory(configRoot), before, 'Install changed synthetic configuration');
    // Model a failed migration. Verify restore removes only our own synthetic managed scope.
    await writeFile(configFile, 'SIMULATED_FAILED_MIGRATION\n');
    await chmod(configFile, 0o644);
    await writeFile(path.join(configDir, 'simulated-new-managed-file'), 'fixture');
    await rm(prefix, { recursive: true });
    await cp(path.join(backup, 'prefix'), prefix, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true });
    await rm(configRoot, { recursive: true });
    await cp(path.join(backup, 'config'), configRoot, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true });
    // No npm command or network request is involved in restoration or these probes.
    const restoredVersion = (await probe(['--version'])).stdout;
    assert(restoredVersion.includes(tool.old), 'Restored executable version mismatch');
    await probe(['--help']);
    assert.deepEqual(await inventory(configRoot), before, 'Configuration bytes/permissions/missing file state mismatch');
    assert.deepEqual(await inventory(prefix), originalInstall, 'Old install snapshot mismatch');
    evidence.results.push({ tool: tool.id, package: tool.pkg, from: tool.old, to: tool.next,
      before: oldVersion, upgraded: newVersion, restored: restoredVersion,
      source: versionMetadata.tarball, packageIntegrity: versionMetadata.integrity,
      installSnapshotEntries: originalInstall.length, configurationSnapshot: before,
      checks: { version: true, help: true, offlineRestore: true, restoredInstallHashes: true,
        restoredConfigurationBytesAndModes: true, restoredMissingManagedFile: true },
      durationSeconds: Math.round((performance.now() - start) / 1000) });
    await saveEvidence();
    console.log(`${tool.id}: PASS upgrade and offline restore`);
  }
  evidence.status = 'passed';
  await saveEvidence();
  console.log(`Evidence: ${output}`);
} catch (error) {
  evidence.status = 'failed';
  evidence.error = error.message;
  await saveEvidence();
  throw error;
} finally {
  // The path was created above by mkdtemp; never remove a user-supplied prefix.
  await rm(root, { recursive: true, force: true });
}
