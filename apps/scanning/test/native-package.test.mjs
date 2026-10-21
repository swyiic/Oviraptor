import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createNativeManifest } from '../native-manifest.mjs';
import { packageNativeScanning } from '../package-native.mjs';
import { validateManifest, validatePackageDirectory, validateToolArguments, validateActionArguments } from '../../../packages/app-sdk/src/index.mjs';
import { buildNativePage, nativePageInputManifest, readNativePageArtifact } from '../build-native-page.mjs';

test('native scanning declares real execution separately from the retained draft-only Node package', async () => {
  const original = JSON.parse(await readFile(new URL('../app.package.json', import.meta.url)));
  assert.equal(original.manifest.schemaVersion, 1);
  assert.equal(original.manifest.tools.some(tool => tool.name === 'scan_start'), false);
  const manifest = createNativeManifest();
  assert.equal(manifest.schemaVersion, 2);
  assert.equal(manifest.runtime.kind, 'native');
  assert.equal(manifest.dataSchemaVersion, 2);
  assert.deepEqual(manifest.tools.map(tool => tool.name), ['scan_workbench_start', 'scan_list', 'scan_draft_create', 'scan_draft_read', 'scan_status', 'scan_start', 'scan_pause', 'scan_resume', 'scan_retry', 'scan_draft_cancel']);
  assert.equal(manifest.tools.some(tool => tool.name === 'configure_model'), false);
  assert.equal(manifest.actions.find(action => action.name === 'configure_model').scope, 'application');
  assert.equal(manifest.actions.find(action => action.name === 'scan_start').scope, 'project');
  const files = [manifest.entry, 'native.html'].map(path => ({ path, sha256: 'a'.repeat(64) }));
  assert.equal(validateManifest({ ...manifest, files }).id, 'org.oviraptor.scanning');
  const task = manifest.tools.find(tool => tool.name === 'scan_start').inputSchema;
  validateToolArguments(task, { taskId: 'original-task' });
  for (const extra of [{ projectId: 99 }, { model: 'caller-selected' }, { dataDir: '/tmp/caller' }, { resumePaid: true }]) {
    assert.throws(() => validateToolArguments(task, { taskId: 'original-task', ...extra }), { code: 'app_arguments_invalid' });
  }
});

test('native scanning explicitly declares page handoff without granting a model tool or implicit execution', () => {
  const manifest = createNativeManifest();
  const handoff = manifest.permissions.filter(permission => permission.id === 'ui.handoff');
  assert.equal(handoff.length, 1);
  assert.match(handoff[0].description, /explicit review/u);
  assert.equal(manifest.tools.some(spec => spec.permission === 'ui.handoff'), false);
  assert.equal(manifest.actions.some(spec => spec.permission === 'ui.handoff'), false);
  assert.equal(manifest.tools.find(spec => spec.name === 'scan_draft_create').inputSchema.properties.targets.maxItems, 32);
  assert.equal(manifest.actions.find(spec => spec.name === 'scan_draft_create').inputSchema.properties.targets.maxItems, 200);
  assert.equal(manifest.actions.find(spec => spec.name === 'source_import').permission, 'source.import');
  assert.equal(manifest.tools.find(spec => spec.name === 'scan_workbench_start').permission, 'investigation.execute');
  const files = [manifest.entry, 'native.html'].map(path => ({ path, sha256: 'a'.repeat(64) }));
  assert.equal(validateManifest({ ...manifest, files }).permissions.filter(permission => permission.id === 'ui.handoff').length, 1);
});

test('native package contains the pinned engine resources and page client without executing its input binary', { skip: process.platform !== 'linux' }, async t => {
  const root = await mkdtemp(join(tmpdir(), 'scanning-native-package-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const outputDirectory = join(root, 'package');
  // /usr/bin/true is only a valid ELF packaging fixture. It is never launched
  // and supplies no evidence that the actual scanning service has run.
  await packageNativeScanning({ binary: '/usr/bin/true', outputDirectory });
  const packaged = await validatePackageDirectory(outputDirectory);
  const names = packaged.manifest.files.map(file => file.path);
  for (const name of ['bin/oviraptor-scan-service', 'native.html', 'resources/workers/9_frontend_runtime_probe.cjs', 'resources/workers/8_js_ast_analyzer.cjs', 'resources/workers/babel-parser.cjs', 'resources/workers/BABEL_PARSER_LICENSE.txt', 'resources/config/browser-locations.json']) assert.ok(names.includes(name), name);
  for (const name of ['native-controller.mjs', 'native-page.mjs', 'page-client.mjs', 'native.css', 'build-native-page.mjs']) assert.equal(names.includes(name), false, name);
  assert.deepEqual(await readFile(join(outputDirectory, 'native.html'), 'utf8'), await readNativePageArtifact());
  assert.equal(names.some(name => name.startsWith('sdk/')), false);
  assert.deepEqual(await readFile(join(outputDirectory, 'resources/workers/9_frontend_runtime_probe.cjs')), await readFile(new URL('../../../src-tauri/resources/workers/9_frontend_runtime_probe.cjs', import.meta.url)));
  await writeFile(join(outputDirectory, 'native.html'), 'tampered page');
  await assert.rejects(validatePackageDirectory(outputDirectory), { code: 'app_package_hash_mismatch' });
});

test('author build is deterministic inline IIFE and CSS for an opaque iframe with no runtime import or modal dependency', async () => {
  const built = await buildNativePage({ write: false });
  assert.equal(await readNativePageArtifact(), built);
  assert.equal(/<script[^>]+(?:src=|type="module")/iu.test(built), false);
  assert.equal(/<link[^>]+stylesheet/iu.test(built), false);
  assert.equal(/<form\b|type="submit"/iu.test(built), false);
  assert.equal(/addEventListener\(["']submit/u.test(built), false);
  assert.equal(/\b(?:import\s*\(|fetch\s*\(|window\.confirm\s*\()/u.test(built), false);
  assert.ok(built.includes('scan_confirmation_stale'));
  assert.ok(built.includes('oviraptor.app-ui.v2'));
  assert.ok(built.includes('id="confirm-execution"'));
  assert.ok(built.includes('id="handoff-review"'));
  assert.ok(built.includes('id="review-handoff"'));
});

test('author artifact digest tracks the actual transitive browser import closure, including handoff validation', async () => {
  const built = await buildNativePage({ write: false });
  const inputs = nativePageInputManifest(built);
  for (const path of ['native.html', 'native.css', 'build-native-page.mjs', 'native-page.mjs', 'native-controller.mjs', '../../packages/app-sdk/src/page-client.mjs', '../../packages/app-sdk/src/handoff.mjs']) assert.ok(inputs.includes(path), path);
  assert.equal(new Set(inputs).size, inputs.length);
  assert.throws(() => nativePageInputManifest(built.replace(/scanning-page-inputs:[A-Za-z0-9+/=]+/u, `scanning-page-inputs:${Buffer.from(JSON.stringify(['/outside/file'])).toString('base64')}`)), /scanning_native_page_input_manifest_invalid/u);
});

test('native package builder refuses symlink binary and leaves the destination absent', { skip: process.platform === 'win32' }, async t => {
  const root = await mkdtemp(join(tmpdir(), 'scanning-native-symlink-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await symlink('/usr/bin/true', join(root, 'alias'));
  await assert.rejects(packageNativeScanning({ binary: join(root, 'alias'), outputDirectory: join(root, 'package') }), /native_package_binary_invalid/u);
  await assert.rejects(readFile(join(root, 'package', 'manifest.json')), { code: 'ENOENT' });
});

test('operator draft schema preserves original 200 capacity while model schema remains 32 and source import stays operator-only',()=>{
  const manifest=createNativeManifest();
  const targets=Array.from({length:200},(_,n)=>`https://fixture.invalid/${n}`);
  const tool=manifest.tools.find(spec=>spec.name==='scan_draft_create').inputSchema;
  const action=manifest.actions.find(spec=>spec.name==='scan_draft_create').inputSchema;
  validateToolArguments(tool,{title:'Model32',targets:targets.slice(0,32)});
  assert.throws(()=>validateToolArguments(tool,{title:'Model33',targets:targets.slice(0,33)}),{code:'app_arguments_invalid'});
  validateActionArguments(action,{title:'Operator200',targets});
  assert.throws(()=>validateActionArguments(action,{title:'Operator201',targets:[...targets,'https://fixture.invalid/201']}),{code:'app_arguments_invalid'});
  assert.equal(manifest.tools.some(spec=>spec.name==='source_import'),false);
  const source=manifest.actions.find(spec=>spec.name==='source_import');assert.equal(source.scope,'project');assert.equal(source.permission,'source.import');
  assert.equal(manifest.tools.find(spec=>spec.name==='scan_workbench_start').inputSchema.properties.urls.maxItems,128);
  assert.equal(manifest.actions.find(spec=>spec.name==='scan_workbench_start').inputSchema.properties.urls.maxItems,200);
  for(const name of ['sourcePath','projectId','apiKey','authValue','diffBase','commitSha']){
    const spec=manifest.tools.find(spec=>spec.name==='scan_workbench_start').inputSchema;
    assert.throws(()=>validateToolArguments(spec,{sourceRef:'f9b7e667-3c18-4567-89ab-123456789012',title:'Source',scanType:'code',[name]:'forged'}),{code:'app_arguments_invalid'});
  }
});
