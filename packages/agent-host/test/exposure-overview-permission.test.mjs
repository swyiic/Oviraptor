import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const repository = fileURLToPath(new URL('../../../', import.meta.url));

async function installAssets(t) {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-exposure-perm-'));
  const host = new AgentHost({ dataDir: join(directory, 'host') });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
  const project = host.store.createProject('Exposure permission project');
  const sourceDirectory = join(repository, 'apps', 'assets');
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const outputDirectory = join(directory, 'assets-package');
  await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
  const installed = await host.apps.install(outputDirectory);
  await host.apps.setEnabled(installed.id, true);
  return { host, project, installed, spec };
}

test('G10: inventory.exposure.read gates exposure_overview; inventory.read alone cannot see or call it', async t => {
  const { host, project, installed } = await installAssets(t);
  assert.equal(host.apps.capabilities(project.id).some(tool => tool.name.endsWith('::exposure_overview')), false);

  host.apps.setGrants(installed.id, project.id, ['inventory.read']);
  const withRead = host.apps.capabilities(project.id).map(tool => tool.name);
  assert.equal(withRead.some(name => name.endsWith('::exposure_overview')), false);
  assert.ok(withRead.some(name => name.endsWith('::inventory_stats')));
  await assert.rejects(host.apps.call(installed.id, 'exposure_overview', project.id, {}), error => {
    assert.equal(error.code, 'host_tool_permission_denied');
    assert.equal(error.status, 403);
    return true;
  });

  host.apps.setGrants(installed.id, project.id, ['inventory.exposure.read']);
  const withExposure = host.apps.capabilities(project.id);
  assert.deepEqual(withExposure.map(tool => tool.name).sort(), [`${installed.id}::exposure_overview`]);
  const overview = (await host.apps.call(installed.id, 'exposure_overview', project.id, {})).structuredContent;
  assert.equal(overview.kind, 'exposure_overview');
  assert.equal(overview.hostBoundProjectId, project.id);
  assert.equal(overview.available, false);
  assert.equal(overview.runs.total, 0);
  assert.equal(overview.findings.total, 0);
  assert.equal(Object.hasOwn(overview, 'payload'), false);
});
