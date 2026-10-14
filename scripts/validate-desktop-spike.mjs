import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

// Runs the already-built universal experiment. It never installs an application in /Applications.
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const app = path.join(repo, 'experiments/desktop-spike/src-tauri/target/universal-apple-darwin/release/bundle/macos/AgentUpdate Technical Spike.app');
const binary = path.join(app, 'Contents/MacOS/agentupdate-desktop-spike');
const root = await mkdtemp(path.join(os.tmpdir(), 'agentupdate-age4-'));
function run(executable, args, extraEnv = {}, timeout = 30_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { env: { ...process.env, ...extraEnv }, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', b => { stdout += b; }); child.stderr.on('data', b => { stderr += b; });
    const timer = setTimeout(() => child.kill('SIGTERM'), timeout);
    child.on('error', reject);
    child.on('close', code => {
      clearTimeout(timer);
      if (code !== 0) reject(new Error(`${path.basename(executable)} exited ${code}: ${stderr.slice(-2000)}`));
      else resolve({ stdout: stdout.trim(), stderr: stderr.trim() });
    });
  });
}
const evidence = { date: '2026-10-12', hostArchitecture: process.arch, results: [],
  limitations: ['Intel execution uses Rosetta on Apple Silicon, not physical Intel hardware.',
    'Ad-hoc signature only; Developer ID signing and notarization belong to AGE-16.',
    'macOS 15 minimum is configured, not runtime-tested on macOS 15.'] };
try {
  const slices = (await run('/usr/bin/lipo', ['-archs', binary])).stdout;
  assert(slices.includes('arm64') && slices.includes('x86_64'), 'Universal binary is missing an architecture');
  evidence.architectures = slices;
  evidence.minimumSystemVersion = (await run('/usr/bin/plutil', ['-extract', 'LSMinimumSystemVersion', 'raw', path.join(app, 'Contents/Info.plist')])).stdout;
  assert.equal(evidence.minimumSystemVersion, '15.0');
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]);
  evidence.adHocSignatureVerified = true;
  for (const [flag, architecture, execution] of [['-arm64', 'aarch64', 'native'], ['-x86_64', 'x86_64', 'Rosetta']]) {
    const smoke = JSON.parse((await run('/usr/bin/arch', [flag, binary, '--smoke'])).stdout);
    assert.equal(smoke.architecture, architecture);
    assert(smoke.childProcess && smoke.isolatedFileRoundtrip && smoke.restrictiveFilePermissions);
    const file = path.join(root, `${architecture}.json`);
    await run('/usr/bin/arch', [flag, binary], {
      AGENTUPDATE_SPIKE_AUTOTEST: '1', AGENTUPDATE_SPIKE_EVIDENCE_FILE: file,
    });
    const webview = JSON.parse(await readFile(file, 'utf8'));
    assert.equal(webview.report.architecture, architecture);
    assert(Object.values(webview.webviewChecks).every(Boolean), 'WebView capability checks failed');
    evidence.results.push({ architecture, execution, smoke, webview });
    console.log(`${architecture}: PASS process/files and real WebView permissions (${execution})`);
  }
  evidence.status = 'passed';
} catch (error) {
  evidence.status = 'failed'; evidence.error = error.message;
  throw error;
} finally {
  await mkdir(path.join(repo, 'docs/evidence'), { recursive: true });
  await writeFile(path.join(repo, 'docs/evidence/age-4-desktop.json'), JSON.stringify(evidence, null, 2) + '\n');
  await rm(root, { recursive: true, force: true });
}
