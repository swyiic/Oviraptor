// Deliberately requires the actual built binary. Missing native artifacts fail;
// this smoke never turns into a mock/skip and never starts model/target work.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdtemp, mkdir, rm, realpath, access } from 'node:fs/promises';
import { constants } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, isAbsolute } from 'node:path';

const binary = process.env.OVIRAPTOR_SCAN_SERVICE_TEST_BINARY;
assert.ok(binary && isAbsolute(binary), 'An actual built oviraptor-scan-service binary is required');
await access(binary, constants.X_OK);
test('actual native bootstrap process persists an original draft and never offers execution', async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'oviraptor-native-bootstrap-cli-')));
  const [data, configuration, resources] = ['data', 'configuration', 'resources'].map(p => join(root, p));
  await Promise.all([data, configuration, resources].map(path => mkdir(path, { mode: 0o700 })));
  const workers = [];
  t.after(async () => {
    for (const worker of workers) {
      worker.child.stdin.end();
      const timer = setTimeout(() => worker.child.kill(), 3000);
      await worker.exited; clearTimeout(timer);
    }
    await rm(root, { recursive: true, force: true });
  });
  async function start() {
    const child = spawn(binary, ['--data-dir', data, '--configuration-dir', configuration, '--resource-dir', resources], {
      env: { PATH: process.env.PATH, LD_LIBRARY_PATH: process.env.LD_LIBRARY_PATH }, stdio: ['pipe', 'pipe', 'pipe'],
    });
    let stderr = '', next = 0;
    const pending = new Map();
    child.stderr.on('data', chunk => { stderr += chunk; });
    const exited = new Promise(resolve => child.once('close', code => {
      for (const { reject } of pending.values()) reject(Error(`native_exit_${code}`));
      pending.clear(); resolve(code);
    }));
    createInterface({ input: child.stdout }).on('line', line => {
      const message = JSON.parse(line); pending.get(message.id)?.resolve(message); pending.delete(message.id);
    });
    async function request(method, params) {
      const id = ++next; let timer;
      return await new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(Error('native_rpc_timeout')), 10000);
        pending.set(id, { resolve, reject });
        child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
      }).finally(() => clearTimeout(timer));
    }
    const worker = { request, child, exited, stderr: () => stderr };
    workers.push(worker); return worker;
  }
  const hostProject = projectId => ({ projectId, name: 'Host fixture', status: 'active', grantRevision: 1, permissions: ['draft.read', 'draft.write'] });
  const context = (hostBoundProjectId = 91) => ({ hostBoundProjectId, grantRevision: 1, operationId: 'native-create-1' });
  const call = (worker, name, args, ctx = context()) => worker.request('tool/call', { name, arguments: args, context: ctx });
  const worker = await start();
  assert.equal((await worker.request('describe', { adapter: 'scanning-bootstrap-v1' })).result.executionAvailable, false);
  await worker.request('host/project', hostProject(91));
  const args = { title: 'Actual original engine draft', targets: ['https://fixture.invalid/Exact'], scanMode: 'quick' };
  const created = await call(worker, 'engine_draft_create', args);
  assert.equal(created.result.status, 'draft'); assert.equal(created.result.executionStarted, false);
  assert.deepEqual((await call(worker, 'engine_draft_create', args)).result, created.result);
  assert.ok((await call(worker, 'start_scan', {})).error);
  assert.equal((await worker.request('service/status', {})).result.newDrafts, 1);
  assert.equal((await worker.request('service/stop', {})).result.busy, false);
  worker.child.stdin.end(); assert.equal(await worker.exited, 0, worker.stderr());
  const reopened = await start();
  assert.ok((await call(reopened, 'engine_draft_read', { taskId: created.result.taskId })).error);
  await reopened.request('host/project', hostProject(91));
  await reopened.request('host/project', hostProject(92));
  assert.equal((await call(reopened, 'engine_draft_read', { taskId: created.result.taskId })).result.taskId, created.result.taskId);
  assert.ok((await call(reopened, 'engine_draft_read', { taskId: created.result.taskId }, context(92))).error);
  reopened.child.stdin.end(); assert.equal(await reopened.exited, 0, reopened.stderr());
});
