import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { cp, mkdir, mkdtemp, readFile, readdir, realpath, rename, rm, writeFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { DESKTOP_HOST_RESOURCE, DISTRIBUTION_RECEIPT, prepareDesktopAgentHost, repositoryRoot, requireNode24, verifyDesktopDistribution } from './prepare_desktop_agent_host.mjs';

let directory, distribution;
const run = script => new Promise((resolve, reject) => {
  const child = spawn(process.execPath, [join(repositoryRoot, script)], { cwd: repositoryRoot, stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = ''; child.stderr.on('data', chunk => { stderr += chunk; });
  child.once('error', reject); child.once('exit', code => code === 0 ? resolve() : reject(new Error(`${script} failed: ${stderr}`)));
});

before(async () => {
  requireNode24();
  directory = await realpath(await mkdtemp(join(tmpdir(), 'oviraptor-desktop-host-')));
  // Reuse the existing app/agent build entry points. They write ignored artifacts.
  await run('tools/build_app_packages.mjs');
  await run('packages/agent-ui/scripts/build.mjs');
  await run('packages/agent-host/src/build.mjs');
  const original = join(directory, 'original');
  await prepareDesktopAgentHost({ outputDirectory: original });
  distribution = join(directory, 'relocated'); await rename(original, distribution);
});
after(async () => { if (directory) await rm(directory, { recursive: true, force: true }); });

test('desktop resources retain all package-relative imports and only the explicitly listed builtin Apps', async () => {
  const receipt = await verifyDesktopDistribution(distribution);
  const manifest = JSON.parse(await readFile(join(distribution, 'packages/agent-host/bundled-apps.json'), 'utf8'));
  assert.deepEqual((await readdir(join(distribution, 'apps'))).sort(), manifest.apps.map(app => app.package.slice(5)).sort());
  assert.deepEqual(Object.keys(receipt.packageVersions).sort(), ['agent-host', 'agent-ui', 'app-sdk', 'runtime-acp', 'skills-library']);
  assert.equal(receipt.minimumNodeMajor, 24); assert.equal(receipt.nodeRuntimeBundled, false);
  assert(!JSON.stringify(receipt).includes(repositoryRoot));
  assert(!receipt.files.some(file => /(^|\/)node(?:\.exe)?$|node_modules|host\.sqlite|model-provider\.json/.test(file.path)));
});

test('staging rejects stale build inputs rather than overwriting an existing good distribution', async () => {
  const repository = join(directory, 'stale-repository');
  for (const path of ['packages/agent-host/src', '.agent-build/agent-first/packages/agent-host/src']) await mkdir(join(repository, path), { recursive: true });
  await writeFile(join(repository, 'packages/agent-host/src/cli.mjs'), 'new source');
  await writeFile(join(repository, '.agent-build/agent-first/packages/agent-host/src/cli.mjs'), 'old build');
  const output = join(directory, 'preserved'); await mkdir(output); await writeFile(join(output, 'keep.txt'), 'good distribution');
  await assert.rejects(prepareDesktopAgentHost({ repository, outputDirectory: output }), /stale build directory/);
  assert.equal(await readFile(join(output, 'keep.txt'), 'utf8'), 'good distribution');
  await assert.rejects(prepareDesktopAgentHost({ outputDirectory: join(repositoryRoot, 'packages') }), /ignored desktop resource directory/);
});

test('a missing package dependency or a changed file fails resource verification', async () => {
  const missing = join(directory, 'missing'); await cp(distribution, missing, { recursive: true });
  await rm(join(missing, 'packages/runtime-acp/index.mjs'));
  await assert.rejects(verifyDesktopDistribution(missing));
  const modified = join(directory, 'modified'); await cp(distribution, modified, { recursive: true });
  await writeFile(join(modified, 'packages/agent-host/src/provider.mjs'), 'export const syntheticTamper = true;');
  await assert.rejects(verifyDesktopDistribution(modified), /distribution digest mismatch/);
});

test('the Tauri build hook stages resources and Node 20 has no accepted fallback', async () => {
  const config = JSON.parse(await readFile(join(repositoryRoot, 'src-tauri/tauri.conf.json'), 'utf8'));
  const scripts = JSON.parse(await readFile(join(repositoryRoot, 'package.json'), 'utf8')).scripts;
  assert(config.bundle.resources.includes(`${DESKTOP_HOST_RESOURCE}/`));
  assert.equal(config.build.beforeBuildCommand, 'npm run build:desktop');
  assert.match(scripts['build:desktop'], /build:desktop-agent-host/);
  assert.match(scripts['build:desktop-agent-host'], /build:app-packages.*build:agent-first.*prepare_desktop_agent_host/);
  assert.throws(() => requireNode24('20.19.0'), /Node.js 24\+/); assert.doesNotThrow(() => requireNode24('24.0.0'));
});

test('a relocated packaged CLI serves its matching UI, builtin Apps and skills with runtime none and an empty home', { timeout: 30_000 }, async () => {
  const home = join(directory, 'empty-home'), cwd = join(directory, 'empty-cwd'), data = join(directory, 'private-data');
  await mkdir(home); await mkdir(cwd);
  const token = 'a'.repeat(64);
  const child = spawn(process.execPath, [join(distribution, 'packages/agent-host/src/cli.mjs'), '--runtime', 'none', '--data-dir', data, '--session-token', token, '--port', '0'], {
    cwd,
    env: { PATH: process.env.PATH ?? '', HOME: home, USERPROFILE: home, TMPDIR: directory, OVIRAPTOR_BUNDLED_APPS_ROOT: distribution },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stderr = '', output = '';
  child.stderr.on('data', chunk => { stderr += chunk; });
  const exited = once(child, 'exit');
  try {
    const origin = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`staged CLI startup timeout: ${stderr}`)), 15_000);
      child.once('error', error => { clearTimeout(timer); reject(error); });
      child.once('exit', code => { clearTimeout(timer); reject(new Error(`staged CLI exited ${code}: ${stderr}`)); });
      child.stdout.on('data', chunk => {
        output += chunk;
        const match = output.match(/Oviraptor host: (http:\/\/127\.0\.0\.1:\d+)/);
        if (match) { clearTimeout(timer); resolve(match[1]); }
      });
    });
    const headers = { 'X-Oviraptor-Session': token };
    const status = await (await fetch(`${origin}/api/status`, { headers })).json();
    assert.equal(status.runtime.id, null); assert.deepEqual(status.activeRuns, []);
    assert.equal((await fetch(origin)).status, 200); assert.equal((await fetch(`${origin}/src/main.js`)).status, 200);
    const apps = await (await fetch(`${origin}/api/apps`, { headers })).json();
    assert.equal(apps.items.length, 2); assert(apps.items.every(app => app.enabled && app.manifest.schemaVersion === 1));
    const installed = await fetch(`${origin}/api/skills/bundled`, { method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' }, body: '{}' });
    assert.equal(installed.status, 200, await installed.text());
    const skills = await (await fetch(`${origin}/api/skills`, { headers })).json();
    assert(skills.items.length > 0);
    const settings = await (await fetch(`${origin}/api/settings`, { headers })).json();
    assert.equal(settings.apiProvider.configured, false); assert.equal(settings.runtime.id, 'none');
    const receipt = JSON.parse(await readFile(join(distribution, DISTRIBUTION_RECEIPT), 'utf8'));
    assert.equal(receipt.nodeRuntimeBundled, false);
  } finally {
    child.kill('SIGTERM');
    const timeout = setTimeout(() => child.kill('SIGKILL'), 3_000);
    await exited; clearTimeout(timeout);
  }
});
