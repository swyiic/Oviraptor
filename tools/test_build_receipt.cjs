// Build lifecycle tests use synthetic outputs, never start the application.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { spawnSync } = require('node:child_process');

async function fixture(t) {
  const parent = await fs.mkdtemp(path.join(os.tmpdir(), 'oviraptor-build-test-'));
  t.after(() => fs.rm(parent, { recursive: true, force: true }));
  const root = path.join(parent, 'project');
  for (const dir of ['src', 'public', 'scripts', 'tools', 'src-tauri/src', 'src-tauri/resources',
    'src-tauri/icons', 'src-tauri/capabilities', 'src-tauri/tests']) await fs.mkdir(path.join(root, dir), { recursive: true });
  const write = (name, value) => fs.writeFile(path.join(root, name), value);
  for (const file of ['package-lock.json', 'index.html', 'vite.config.ts', 'tsconfig.json',
    'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs']) await write(file, 'fixture');
  await write('package.json', JSON.stringify({ scripts: { build: 'vue-tsc --noEmit && vite build' } }));
  await write('src-tauri/tauri.conf.json', JSON.stringify({ productName: 'Oviraptor',
    build: { beforeBuildCommand: 'npm run build', frontendDist: '../dist' } }));
  await write('src/example.ts', 'source');
  const api = await import('./release/build_receipt.mjs');
  const produce = async (_command, options) => {
    const bundle = path.join(options.targetDir, 'release/bundle/macos/Oviraptor.app/Contents');
    await fs.mkdir(path.join(bundle, 'MacOS'), { recursive: true });
    await fs.writeFile(path.join(bundle, 'Info.plist'), 'synthetic plist');
    await fs.writeFile(path.join(bundle, 'MacOS/oviraptor'), 'synthetic executable', { mode: 0o755 });
    return { exitCode: 0, signal: null };
  };
  return { ...api, parent, root, write, produce,
    run: execute => api.recordBuild({ root, evidenceParent: parent, execute }) };
}

test('input snapshot includes untracked code, resources and lockfiles, not generated outputs', async t => {
  const f = await fixture(t);
  const before = await f.snapshotBuildInputs(f.root);
  for (const dir of ['dist', 'src-tauri/target']) await fs.mkdir(path.join(f.root, dir), { recursive: true });
  await f.write('dist/old.js', 'old');
  assert.equal((await f.snapshotBuildInputs(f.root)).treeSha256, before.treeSha256);
  for (const file of ['src/new.ts', 'src-tauri/resources/new.bin', 'package-lock.json']) {
    await f.write(file, 'changed');
    const current = await f.snapshotBuildInputs(f.root);
    assert.notEqual(current.treeSha256, before.treeSha256);
    assert.ok(current.entries.some(entry => entry.path === file));
  }
});

test('snapshot fails on missing required inputs, symlinks and local dotenv overrides', async t => {
  const f = await fixture(t);
  await fs.rename(path.join(f.root, 'package-lock.json'), path.join(f.root, 'saved-lock'));
  await assert.rejects(f.snapshotBuildInputs(f.root), /missing_build_input/);
  await fs.rename(path.join(f.root, 'saved-lock'), path.join(f.root, 'package-lock.json'));
  await fs.symlink('example.ts', path.join(f.root, 'src/link.ts'));
  await assert.rejects(f.snapshotBuildInputs(f.root), /build_input_entry_symlink/);
  await fs.unlink(path.join(f.root, 'src/link.ts'));
  await f.write('.env.local', 'API_KEY=fixture-secret');
  await assert.rejects(f.snapshotBuildInputs(f.root), /local_env_file_not_supported/);
});

test('build environment does not inherit secrets, compiler wrappers or output overrides', async () => {
  const { buildEnvironment } = await import('./release/build_receipt.mjs');
  const env = buildEnvironment({ PATH: '/approved/bin', HOME: '/fixture', API_KEY: 'secret',
    NODE_OPTIONS: '--require untrusted', TAURI_CONFIG: '{}', RUSTC_WRAPPER: 'untrusted', CARGO_TARGET_DIR: '/old' }, '/new');
  assert.deepEqual(env, { PATH: '/approved/bin', HOME: '/fixture', CI: 'true',
    CARGO_BUILD_JOBS: '1', CARGO_NET_OFFLINE: 'true', CARGO_TARGET_DIR: '/new' });
});

const macTest = process.platform === 'darwin' ? test : test.skip;
macTest('successful local observation records actual command, stable inputs and fresh output without claiming acceptance', async t => {
  const f = await fixture(t);
  let invocation;
  const report = await f.run(async (command, options) => {
    invocation = { command, options };
    assert.deepEqual(await fs.readdir(options.targetDir), []);
    return f.produce(command, options);
  });
  assert.equal(report.status, 'build_observed');
  assert.equal(report.inputsUnchanged, true);
  assert.equal(report.sourceBinding, 'not_verified');
  assert.equal(report.installedValidation, 'not_performed');
  assert.equal(report.runtimeValidation, 'not_performed');
  assert.deepEqual(invocation.command.slice(2), ['build', '--bundles', 'app', '--ci', '--', '--locked', '--offline', '-j', '1']);
  assert.equal(invocation.options.env.CARGO_BUILD_JOBS, '1');
  assert.ok(report.bundleRoot.startsWith(report.evidenceDir + path.sep));
  assert.equal(JSON.parse(await fs.readFile(report.receiptPath, 'utf8')).status, 'build_observed');
  for (const file of ['receipt.json', 'inputs-before.json', 'inputs-after.json', 'bundle.json'])
    assert.equal((await fs.stat(path.join(report.evidenceDir, file))).mode & 0o777, 0o600);
});

macTest('source changes during a successful builder invalidate the observation', async t => {
  const f = await fixture(t);
  const report = await f.run(async (...args) => { await f.write('src/example.ts', 'changed'); return f.produce(...args); });
  assert.equal(report.status, 'failed');
  assert.equal(report.inputsUnchanged, false);
  assert.match(report.failure, /build_inputs_changed/);
  assert.equal(report.bundleTreeSha256, undefined);
});

macTest('failure, signal and successful no-op cannot adopt a preexisting bundle', async t => {
  const f = await fixture(t);
  await f.produce([], { targetDir: path.join(f.root, 'src-tauri/target') });
  for (const result of [{ exitCode: 7, signal: null }, { exitCode: null, signal: 'SIGTERM' }, { exitCode: 0, signal: null }]) {
    const report = await f.run(async () => result);
    assert.equal(report.status, 'failed');
    assert.equal(report.bundleTreeSha256, undefined);
    assert.equal(JSON.parse(await fs.readFile(report.receiptPath, 'utf8')).status, 'failed');
  }
});

macTest('unsupported configuration and preflight errors never invoke the builder', async t => {
  const f = await fixture(t);
  let calls = 0;
  await f.write('src-tauri/tauri.macos.conf.json', '{}');
  const report = await f.run(async () => { calls++; return { exitCode: 0 }; });
  assert.equal(report.status, 'failed');
  assert.match(report.failure, /unsupported_build_override/);
  assert.equal(calls, 0);
  await assert.rejects(f.recordBuild({ root: f.root, evidenceParent: f.root }), /outside_project/);
});

macTest('builder exceptions and output symlinks remain failed local receipts', async t => {
  const f = await fixture(t);
  const broken = await f.run(async () => { throw new Error('fixture builder unavailable'); });
  assert.equal(broken.status, 'failed');
  const linked = await f.run(async (command, options) => {
    const result = await f.produce(command, options);
    await fs.symlink('/nonexistent', path.join(options.targetDir, 'release/bundle/macos/Oviraptor.app/Contents/link'));
    return result;
  });
  assert.equal(linked.status, 'failed');
  assert.match(linked.failure, /bundle_entry_symlink/);
});

macTest('real subprocess runner records log and terminal exit using a synthetic CLI', async t => {
  const f = await fixture(t);
  await fs.mkdir(path.join(f.root, 'node_modules/@tauri-apps/cli'), { recursive: true });
  await f.write('node_modules/@tauri-apps/cli/tauri.js', 'console.log("fixture-build-log"); process.exitCode = 9;');
  const report = await f.recordBuild({ root: f.root, evidenceParent: f.parent });
  assert.equal(report.status, 'failed');
  assert.equal(report.build.exitCode, 9);
  assert.equal(report.build.signal, null);
  assert.match(await fs.readFile(report.logPath, 'utf8'), /fixture-build-log/);
});

test('CLI rejects missing project without performing a build', () => {
  const result = spawnSync(process.execPath, [path.join(__dirname, 'release/build_receipt.mjs')], { encoding: 'utf8' });
  assert.equal(result.status, 1);
});

async function observationFixture(t) {
  const f = await fixture(t);
  const receipt = await f.run(f.produce);
  const installedPath = path.join(f.parent, 'Installed.app');
  await fs.cp(receipt.bundleRoot, installedPath, { recursive: true });
  const { verifyBuildObservation } = await import('./release/verify_build_observation.mjs');
  return { ...f, receipt, installedPath,
    verify: () => verifyBuildObservation({ root: f.root, receiptPath: receipt.receiptPath, installedPath }),
    save: (name, value) => fs.writeFile(path.join(receipt.evidenceDir, name), JSON.stringify(value)) };
}

macTest('observation verifies three local snapshots without claiming release acceptance', async t => {
  const f = await observationFixture(t);
  const report = await f.verify();
  assert.equal(report.localObservationsMatch, true);
  assert.deepEqual(report.checks, { currentInputsMatchRecorded: true,
    builtBundleMatchesRecorded: true, installedMatchesBuiltBundle: true });
  assert.equal(report.sourceBinding, 'not_verified');
  assert.equal(report.runtimeValidation, 'not_performed');
  assert.equal(report.releaseAcceptance, 'not_verified');
  assert.equal(report.assurance, 'unsigned_local_observation');
  assert.equal((await fs.readdir(f.receipt.evidenceDir)).includes('verification.json'), false);
});

macTest('observation distinguishes source, recorded bundle and installation drift', async t => {
  const f = await observationFixture(t);
  await f.write('src/example.ts', 'changed after build');
  let report = await f.verify();
  assert.deepEqual(report.checks, { currentInputsMatchRecorded: false,
    builtBundleMatchesRecorded: true, installedMatchesBuiltBundle: true });
  await fs.writeFile(path.join(f.receipt.bundleRoot, 'Contents/Info.plist'), 'changed bundle');
  report = await f.verify();
  assert.equal(report.checks.builtBundleMatchesRecorded, false);
  assert.equal(report.checks.installedMatchesBuiltBundle, false);
  assert.equal(report.localObservationsMatch, false);
  assert.deepEqual(report.bundleComparison.differences, [{ path: 'Contents/Info.plist', kind: 'content_changed' }]);
  await fs.cp(f.receipt.bundleRoot, f.installedPath, { recursive: true });
  assert.equal((await f.verify()).checks.installedMatchesBuiltBundle, true);
  assert.equal((await f.verify()).localObservationsMatch, false);
});

macTest('observation rejects unsuccessful, inconsistent and redirected receipt claims', async t => {
  const f = await observationFixture(t);
  for (const mutation of [{ status: 'failed' }, { status: 'in_progress' }, { schemaVersion: 2 },
    { build: { exitCode: 1, signal: null } }, { build: { exitCode: 0, signal: 'SIGTERM' } },
    { inputsUnchanged: false }, { inputsAfterSha256: '0'.repeat(64) },
    { root: f.parent }, { evidenceDir: f.parent }, { bundleRoot: f.installedPath }]) {
    await f.save('receipt.json', { ...f.receipt, ...mutation });
    await assert.rejects(f.verify(), /observation_/);
  }
});

macTest('observation checks saved manifest bodies and refuses malformed or missing evidence', async t => {
  const f = await observationFixture(t);
  for (const name of ['inputs-before.json', 'inputs-after.json', 'bundle.json']) {
    const file = path.join(f.receipt.evidenceDir, name);
    const original = await fs.readFile(file, 'utf8');
    const altered = JSON.parse(original);
    altered.entries[0].size = '999';
    await f.save(name, altered);
    await assert.rejects(f.verify(), /observation_manifest/);
    await fs.writeFile(file, '{');
    await assert.rejects(f.verify(), /observation_json/);
    await fs.unlink(file);
    await assert.rejects(f.verify(), /ENOENT/);
    await fs.writeFile(file, original);
  }
});

macTest('observation rejects evidence links, oversized receipts and linked output directories', async t => {
  const f = await observationFixture(t);
  const original = await fs.readFile(f.receipt.receiptPath);
  const copy = path.join(f.parent, 'receipt-copy.json');
  await fs.writeFile(copy, original);
  await fs.unlink(f.receipt.receiptPath);
  await fs.symlink(copy, f.receipt.receiptPath);
  await assert.rejects(f.verify(), /ELOOP|observation_/);
  await fs.unlink(f.receipt.receiptPath);
  await fs.writeFile(f.receipt.receiptPath, ' '.repeat(65537));
  await assert.rejects(f.verify(), /observation_json_size/);
  await fs.writeFile(f.receipt.receiptPath, original);
  const target = path.join(f.receipt.evidenceDir, 'target');
  const moved = path.join(f.parent, 'moved-target');
  await fs.rename(target, moved);
  await fs.symlink(moved, target);
  await assert.rejects(f.verify(), /observation_directory/);
});

macTest('observation CLI separates matching, drift and invalid evidence exits', async t => {
  const f = await observationFixture(t);
  const script = path.join(__dirname, 'release/verify_build_observation.mjs');
  const run = (...args) => spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' });
  const args = [f.root, f.receipt.receiptPath, f.installedPath];
  assert.equal(run(...args).status, 0);
  await fs.writeFile(path.join(f.installedPath, 'Contents/Info.plist'), 'installed drift');
  const drift = run(...args);
  assert.equal(drift.status, 2);
  assert.equal(JSON.parse(drift.stdout).checks.installedMatchesBuiltBundle, false);
  assert.equal(run().status, 1);
  await f.save('receipt.json', {});
  assert.equal(run(...args).status, 1);
});
