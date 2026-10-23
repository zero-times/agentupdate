import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readlink, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
assert.equal(process.platform, 'darwin');
assert(process.argv[2], 'Provide the output directory from package:mac.');
const output = path.resolve(process.argv[2]);
const manifest = JSON.parse(await readFile(path.join(output, 'release-manifest.json'), 'utf8'));
assert(['candidate-built', 'signed-built'].includes(manifest.status));
const diskName = `${manifest.product}-${manifest.version}-macos-universal.dmg`;
const archiveName = `${manifest.product}-${manifest.version}-macos-universal.app.zip`;
const root = await mkdtemp(path.join(os.tmpdir(), 'agentupdate-package-'));
const mount = path.join(root, 'mounted');
let mounted = false;
const evidence = {
  checkedAt: new Date().toISOString(), version: manifest.version, tag: manifest.tag,
  artifactDirectory: path.relative(repo, output), signingKind: manifest.signing.kind,
  sourceSha256: manifest.sourceSha256, results: [], status: 'running',
  releaseAcceptanceComplete: false,
  limitations: [
    'Temporary HOME and Applications on the development Mac; not a clean macOS 15 system.',
    'x86_64 on Apple Silicon uses Rosetta; not physical Intel hardware.',
    'Launch survival does not verify packaged multi-tool update interaction.',
    'Candidate ad-hoc integrity is not Developer ID signing or Gatekeeper acceptance.',
  ],
};

function run(executable, args, env = process.env, timeout = 60_000) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    const timer = setTimeout(() => { child.kill('SIGKILL'); }, timeout);
    child.stdout.on('data', bytes => { stdout += bytes; });
    child.stderr.on('data', bytes => { stderr += bytes; });
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', code => {
      clearTimeout(timer);
      code === 0 ? resolve(stdout.trim()) : reject(new Error(`${path.basename(executable)} exited ${code}: ${stderr.slice(-1500)}`));
    });
  });
}

async function inspectApp(app) {
  const plist = path.join(app, 'Contents/Info.plist');
  assert.equal(await run('/usr/bin/plutil', ['-extract', 'CFBundleShortVersionString', 'raw', plist]), manifest.version);
  assert.equal(await run('/usr/bin/plutil', ['-extract', 'CFBundleIdentifier', 'raw', plist]), 'work.x5brain.agentupdate');
  assert.equal(await run('/usr/bin/plutil', ['-extract', 'LSMinimumSystemVersion', 'raw', plist]), '15.0');
  const binary = path.join(app, 'Contents/MacOS/agentupdate');
  assert.deepEqual((await run('/usr/bin/lipo', ['-archs', binary])).split(/\s+/).sort(), ['arm64', 'x86_64']);
  assert.equal(createHash('sha256').update(await readFile(binary)).digest('hex'), manifest.binarySha256);
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]);
}

async function launch(binary, architecture) {
  const home = path.join(root, `home-${architecture}`);
  await mkdir(home);
  const env = { ...process.env, HOME: home, PATH: '/usr/bin:/bin:/usr/sbin:/sbin', TMPDIR: `${root}/` };
  for (const key of Object.keys(env)) if (/^(AGENTUPDATE_|CODEX_|GEMINI_CLI_HOME|npm_config_|NPM_CONFIG_)/.test(key)) delete env[key];
  const child = spawn('/usr/bin/arch', [architecture === 'arm64' ? '-arm64' : '-x86_64', binary], { env, stdio: 'ignore' });
  const closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
  let launchError;
  child.on('error', error => { launchError = error; });
  try {
    await new Promise(resolve => setTimeout(resolve, 8000));
    if (launchError) throw launchError;
    assert(child.exitCode === null && child.signalCode === null, `${architecture} exited during launch.`);
    const sample = path.join(root, `${architecture}.sample.txt`);
    await run('/usr/bin/sample', [String(child.pid), '1', '1', '-file', sample]);
    const content = await readFile(sample, 'utf8');
    const codeType = /^Code Type:\s*(.+)$/m.exec(content)?.[1];
    assert(codeType, 'Sample must report actual execution architecture.');
    assert(architecture === 'arm64' ? /ARM[-_]?64/i.test(codeType) : /X86[-_]?64/i.test(codeType), codeType);
    evidence.results.push({ architecture, codeType, startupSurvivedSeconds: 8, execution: architecture === 'arm64' ? 'native Apple Silicon' : 'Rosetta on Apple Silicon' });
  } finally {
    child.kill('SIGTERM');
    const timer = setTimeout(() => child.kill('SIGKILL'), 5000);
    await closed;
    clearTimeout(timer);
  }
}

try {
  const sums = new Map((await readFile(path.join(output, 'SHA256SUMS'), 'utf8')).trim().split('\n').map(line => [line.slice(66), line.slice(0, 64)]));
  for (const artifact of manifest.artifacts) {
    assert.equal(path.basename(artifact.name), artifact.name);
    const file = path.join(output, artifact.name);
    const sha256 = createHash('sha256').update(await readFile(file)).digest('hex');
    assert.equal(sha256, artifact.sha256);
    assert.equal(sha256, sums.get(artifact.name));
    assert.equal((await stat(file)).size, artifact.bytes);
  }
  evidence.artifactHashesVerified = true;
  evidence.artifacts = manifest.artifacts;
  await run('/usr/bin/unzip', ['-tq', path.join(output, archiveName)]);
  const extracted = path.join(root, 'zip');
  await run('/usr/bin/ditto', ['-x', '-k', path.join(output, archiveName), extracted]);
  await inspectApp(path.join(extracted, `${manifest.product}.app`));
  evidence.zipExtractionVerified = true;
  await run('/usr/bin/hdiutil', ['verify', path.join(output, diskName)]);
  await mkdir(mount);
  await run('/usr/bin/hdiutil', ['attach', '-readonly', '-nobrowse', '-mountpoint', mount, path.join(output, diskName)]);
  mounted = true;
  assert.equal(await readlink(path.join(mount, 'Applications')), '/Applications');
  await inspectApp(path.join(mount, `${manifest.product}.app`));
  const installed = path.join(root, 'Applications', `${manifest.product}.app`);
  await run('/usr/bin/ditto', [path.join(mount, `${manifest.product}.app`), installed]);
  await run('/usr/bin/hdiutil', ['detach', mount]);
  mounted = false;
  await inspectApp(installed);
  evidence.dmgMountAndTemporaryInstallVerified = true;
  for (const architecture of ['arm64', 'x86_64']) await launch(path.join(installed, 'Contents/MacOS/agentupdate'), architecture);
  await rm(installed, { recursive: true });
  await assert.rejects(stat(installed), { code: 'ENOENT' });
  evidence.temporaryUninstallVerified = true;
  evidence.status = 'passed';
  console.log('PASS ZIP extraction, DMG install/copy, native/Rosetta production startup and temporary uninstall.');
} catch (error) {
  evidence.status = 'failed';
  evidence.error = error.message;
  throw error;
} finally {
  if (mounted) await run('/usr/bin/hdiutil', ['detach', mount]);
  await rm(root, { recursive: true, force: true });
  await mkdir(path.join(repo, 'docs/evidence'), { recursive: true });
  await writeFile(process.env.AGENTUPDATE_PACKAGE_EVIDENCE || path.join(repo, 'docs/evidence/v' + manifest.version + '-package.json'), `${JSON.stringify(evidence, null, 2)}\n`);
  await writeFile(path.join(output, 'package-validation.json'), `${JSON.stringify(evidence, null, 2)}\n`);
}
