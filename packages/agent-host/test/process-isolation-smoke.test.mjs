import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));

test('G11 smoke: app worker crash does not take down host; call returns structured error', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'oviraptor-process-isolation-'));
  const host = new AgentHost({ dataDir: join(directory, 'host') });
  t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });

  const project = host.store.createProject('Isolation project');
  const sourceDirectory = join(root, 'examples', 'echo-app');
  const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
  const outputDirectory = join(directory, 'echo-package');
  await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
  const installed = await host.apps.install(outputDirectory);
  await host.apps.setEnabled(installed.id, true);
  host.apps.setGrants(installed.id, project.id, ['echo.write', 'echo.read']);

  const first = await host.apps.call(installed.id, 'echo', project.id, { text: 'before-crash' });
  assert.equal(first.structuredContent.projectId, project.id);

  const worker = host.apps.workers.get(installed.id);
  assert.ok(worker?.child?.pid);
  const hostPid = process.pid;
  worker.child.kill('SIGKILL');
  await worker.exitPromise;
  assert.equal(worker.exitObserved, true);
  assert.equal(worker.stopped, true);

  // Host process remains the same live PID and keeps serving store/API state.
  assert.equal(process.pid, hostPid);
  assert.equal(host.apps.list().length, 1);
  assert.equal(host.apps.list()[0].status, 'failed');
  const conversation = host.store.createConversation(project.id);
  assert.equal(typeof conversation.id, 'string');
  assert.ok(conversation.id.length > 0);
  assert.equal((await host.send(conversation.id, { projectId: project.id, text: 'host still alive' })).savedOnly, true);

  await assert.rejects(host.apps.call(installed.id, 'history', project.id, {}), error => {
    assert.ok(['host_app_failed', 'host_app_process_exited', 'host_app_process_unavailable'].includes(error.code), error.code);
    assert.equal(typeof error.code, 'string');
    return true;
  });
});
