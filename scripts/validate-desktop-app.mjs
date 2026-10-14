import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const version = JSON.parse(await readFile(path.join(repo, 'package.json'), 'utf8')).version;
const target = process.env.CARGO_TARGET_DIR || path.join(repo, 'src-tauri/target');
const bundleRoot = path.join(target, 'universal-apple-darwin/release/bundle/macos');
const validationApp = path.join(bundleRoot, 'AgentUpdate Validation.app');
const binary = path.join(validationApp, 'Contents/MacOS/agentupdate');
const temporary = await mkdtemp(path.join(os.tmpdir(), 'agentupdate-age5-'));

function run(executable, args, env = {}, timeout = 120_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '', timedOut = false;
    child.stdout.on('data', b => { stdout += b; });
    child.stderr.on('data', b => { stderr += b; });
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, timeout);
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', code => {
      clearTimeout(timer);
      if (code !== 0 || timedOut) reject(new Error(`${path.basename(executable)} ${timedOut ? 'timed out' : `exited ${code}`}: ${stderr.slice(-1500)}`));
      else resolve(stdout.trim());
    });
  });
}

const evidence = {
  checkedAt: new Date().toISOString(), hostArchitecture: process.arch, results: [],
  limitations: [
    'x86_64 executes through Rosetta on Apple Silicon; physical Intel hardware remains untested.',
    'Minimum macOS 15 is configured, not tested on macOS 15 hardware.',
    'Ad-hoc signed development build; Developer ID and notarization belong to AGE-16.',
    'This boundary check does not execute an Agent update or rollback; separate workflow checks cover those operations.',
  ],
};
try {
  assert.equal(process.platform, 'darwin');
  for (const bundle of [validationApp]) {
    const slices = await run('/usr/bin/lipo', ['-archs', path.join(bundle, 'Contents/MacOS/agentupdate')]);
    assert(slices.includes('arm64') && slices.includes('x86_64'));
    assert.equal(await run('/usr/bin/plutil', ['-extract', 'LSMinimumSystemVersion', 'raw', path.join(bundle, 'Contents/Info.plist')]), '15.0');
    assert.equal(await run('/usr/bin/plutil', ['-extract', 'CFBundleShortVersionString', 'raw', path.join(bundle, 'Contents/Info.plist')]), version);
    await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', bundle]);
  }
  evidence.universalBundles = ['AgentUpdate Validation.app'];
  evidence.adHocSignaturesVerified = true;
  const config = JSON.parse(await readFile(path.join(repo, 'src-tauri/tauri.conf.json'), 'utf8'));
  const capability = JSON.parse(await readFile(path.join(repo, 'src-tauri/capabilities/main.json'), 'utf8'));
  assert.deepEqual(config.app.security.capabilities, ['main']);
  assert.deepEqual(capability.permissions, ['allow-get-app-overview', 'allow-get-agent-support', 'allow-discover-agents', 'allow-list-agent-versions', 'allow-select-agent-version', 'allow-create-update-plan', 'allow-get-plan-history', 'allow-execute-update', 'allow-preview-rollback', 'allow-rollback-update', 'allow-execute-plan', 'allow-cancel-plan', 'allow-inspect-interrupted-tasks', 'allow-get-restore-details', 'allow-export-transaction', 'allow-cleanup-restore', 'allow-reveal-export', 'allow-check-app-release', 'allow-check-desktop-apps', 'allow-open-desktop-app-channel', 'allow-open-app-release', 'allow-get-cli-projects', 'allow-save-cli-project', 'allow-open-cli', 'allow-install-cli', 'allow-cli-install-history', 'allow-rollback-cli-install', 'allow-prepare-app-install', 'allow-install-app', 'allow-app-install-history', 'allow-finish-app-install', 'allow-rollback-app-install']);
  assert.equal(capability.remote, undefined);
  for (const [flag, architecture, execution] of [['-arm64', 'aarch64', 'native'], ['-x86_64', 'x86_64', 'Rosetta']]) {
    const destination = path.join(temporary, `${architecture}.json`);
    const testHome = path.join(temporary, `home-${architecture}`);
    await mkdir(testHome);
    const testEnvironment = { HOME: testHome, PATH: '/usr/bin:/bin:/usr/sbin:/sbin', CODEX_HOME: '', GEMINI_CLI_HOME: '', CLAUDE_CONFIG_DIR: '', AGENTUPDATE_VALIDATION_RESULT: destination, AGENTUPDATE_TEST_DATA_DIR: path.join(testHome, 'app-data') };
    await run('/usr/bin/arch', [flag, binary], testEnvironment);
    const result = JSON.parse(await readFile(destination, 'utf8'));
    assert.equal(result.overview.platform.architecture, architecture);
    assert.equal(result.overview.appVersion, version);
    assert(Object.values(result.checks).every(value => value === true), 'Real WebView checks failed');
    assert(result.overview.agents.every(agent => agent.detectionState === 'not_scanned' && !agent.updateEnabled));
    evidence.results.push({ architecture, execution, ...result });
    console.log(`${architecture}: PASS real React/Rust calls, input validation and capability restrictions (${execution})`);
  }
  evidence.status = 'passed';
} catch (error) {
  evidence.status = 'failed'; evidence.error = error.message;
  throw error;
} finally {
  await mkdir(path.join(repo, 'docs/evidence'), { recursive: true });
  await writeFile(path.join(repo, `docs/evidence/v${version}-desktop.json`), JSON.stringify(evidence, null, 2) + '\n');
  await rm(temporary, { recursive: true, force: true });
}
