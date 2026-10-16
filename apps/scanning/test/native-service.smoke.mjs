/** Explicit actual-binary smoke. Not auto-discovered without a built worker.
 * No target/provider execution: the only scan_start is before model setup and
 * must leave the original attempt/run/financial tables empty.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, readFile, realpath, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { DatabaseSync } from 'node:sqlite';
import { pathToFileURL } from 'node:url';
import { packageNativeScanning } from '../package-native.mjs';

const permissions = ['investigation.draft.write', 'investigation.read', 'investigation.execute', 'investigation.control', 'configuration.read', 'configuration.write'];
const scope = (permission, operationId, projectId = 71) => ({ hostBoundProjectId: projectId, grantRevision: 1, operationId, appRevision: 1, permissions, permission, project: { id: projectId, name: `Synthetic ${projectId}`, status: 'active' } });

function peer(executable, data, directory, home) {
  // Empty library-path and credential/proxy environment by construction. This
  // is the real production worker, not a mock native protocol peer.
  const child = spawn(executable, ['--app-worker', '--data-dir', data, '--package-dir', directory], {
    cwd: directory, env: { PATH: process.env.PATH, HOME: home, TMPDIR: home, LANG: 'C.UTF-8' }, stdio: ['pipe', 'pipe', 'pipe'],
  });
  let id = 0, stderr = '', notifications = 0;
  const pending = new Map();
  const exited = new Promise(resolve => child.once('exit', (code, signal) => resolve({ code, signal })));
  child.stderr.setEncoding('utf8'); child.stderr.on('data', text => { stderr = (stderr + text).slice(-8192); });
  const lines = createInterface({ input: child.stdout });
  lines.on('line', text => {
    let frame;
    try { frame = JSON.parse(text); } catch { for (const item of pending.values()) item.reject(Error('non_protocol_stdout')); return; }
    if (frame.method) { notifications++; return; }
    const item = pending.get(frame.id); if (!item) return;
    pending.delete(frame.id); clearTimeout(item.timer);
    if (frame.error) item.reject(Object.assign(Error(frame.error.data?.code ?? 'rpc_error'), { code: frame.error.data?.code }));
    else item.resolve(frame.result);
  });
  child.once('exit', (code, signal) => { for (const item of pending.values()) { clearTimeout(item.timer); item.reject(Error(`worker_exited_${code ?? signal}`)); } pending.clear(); });
  const rpc = (method, params) => new Promise((resolve, reject) => {
    const requestId = ++id;
    const timer = setTimeout(() => { pending.delete(requestId); reject(Error('actual_native_smoke_rpc_timeout')); }, 15000);
    pending.set(requestId, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: requestId, method, params }) + '\n');
  });
  return { rpc, async start() {
    const description = await rpc('describe', { hostApiVersion: '2.0.0' });
    assert.equal(description.appId, 'org.oviraptor.scanning');
    assert.equal(description.protocolVersion, 2);
    assert.deepEqual(await rpc('lifecycle/start', { dataDir: data, packageDir: directory, context: { hostApiVersion: '2.0.0', appId: 'org.oviraptor.scanning' } }), { dataSchemaVersion: 2 });
    return description;
  }, detach() { child.stdin.end(); }, async stop() {
    const status = await rpc('lifecycle/status', {}); assert.equal(status.busy, false);
    const stop = await rpc('lifecycle/stop', {}); assert.equal(stop.busy, false);
    child.stdin.end(); const result = await exited; assert.deepEqual(result, { code: 0, signal: null });
    return { ...result, notifications, stderr };
  } };
}

export async function smokeNativeScanning({ binary, evidenceDirectory }) {
  const root = resolve(evidenceDirectory); await mkdir(root, { recursive: false, mode: 0o700 });
  const directory = join(root, 'package'), data = join(root, 'data'), home = join(root, 'home');
  for (const path of [data, home]) await mkdir(path, { mode: 0o700 });
  await packageNativeScanning({ binary: await realpath(binary), outputDirectory: directory });
  const manifest = JSON.parse(await readFile(join(directory, 'manifest.json'), 'utf8'));
  const executable = join(directory, manifest.entry);
  const outcomes = [], peers = [];
  try {
  const first = peer(executable, data, directory, home);
  peers.push(first);
  const description = await first.start();
  outcomes.push('actual_sdk2_describe_and_lifecycle_start');
  const call = (name, args, permission, op, project = 71, method = 'action/call') => first.rpc(method, { name, arguments: args, context: scope(permission, op, project) });
  const configuration = await call('configuration_status', {}, 'configuration.read', 'configuration-before');
  assert.equal(configuration.configured, false);
  const input = { title: 'Actual service smoke', targets: ['https://fixture.invalid/Exact'], scanMode: 'standard' };
  const draft = await call('scan_draft_create', input, 'investigation.draft.write', 'create-once');
  assert.equal(draft.creationReceipt.liveState, false); assert.equal(draft.creationReceipt.executionStarted, false);
  assert.deepEqual(await call('scan_draft_create', input, 'investigation.draft.write', 'create-once'), draft);
  const taskId = draft.taskId; assert.equal(typeof taskId, 'string');
  outcomes.push('original_draft_atomic_receipt_and_duplicate_operation');
  const tool = await call('scan_status', { taskId }, 'investigation.read', 'status-tool', 71, 'tool/call');
  assert.equal(tool.structuredContent.status, 'draft'); assert.equal(tool.structuredContent.attemptNumber, 0);
  const listed = await call('scan_list', {}, 'investigation.read', 'list-one');
  assert.equal(listed.total, 1); assert.equal(listed.items[0].taskId, taskId);
  await assert.rejects(call('scan_status', { taskId }, 'investigation.read', 'cross-project', 72), /scan_task_scope_unavailable/u);
  outcomes.push('real_tool_projection_saved_list_and_cross_project_refusal');
  const startup = await call('scan_start', { taskId }, 'investigation.execute', 'missing-model-start');
  assert.equal(startup.outcome.accepted, false); assert.equal(startup.outcome.disposition, 'rejected');
  assert.deepEqual(await call('scan_start', { taskId }, 'investigation.execute', 'missing-model-start'), startup);
  const db = new DatabaseSync(join(data, 'engine-bootstrap-v1', 'oviraptor.sqlite3'), { readOnly: true });
  try {
    for (const table of ['sentinel_scan_attempts', 'agent_runs', 'agent_root_budget_attempts', 'native_branch_dispatches']) assert.equal(db.prepare(`SELECT COUNT(*) AS n FROM ${table}`).get().n, 0, table);
  } finally { db.close(); }
  outcomes.push('real_original_missing_model_admission_refused_no_attempt_root_or_dispatch');
  await assert.rejects(call('configure_model', { llm: 'fixture', deployment: 'local', apiBase: 'http://127.0.0.1:1/v1', apiKey: '' }, 'configuration.write', 'not-a-model-tool', 71, 'tool/call'), /app_method_unknown/u);
  const configured = await call('configure_model', { llm: 'fixture', deployment: 'local', apiBase: 'http://127.0.0.1:1/v1', apiKey: '' }, 'configuration.write', 'configure-operator');
  assert.equal(configured.configured, true); assert.equal(configured.providerRequestSent, false);
  outcomes.push('real_operator_configuration_single_resolver_no_provider_call');
  const stopped = await first.stop();
  const second = peer(executable, data, directory, home); peers.push(second); await second.start();
  const read = await second.rpc('action/call', { name: 'scan_status', arguments: { taskId }, context: scope('investigation.read', 'reopened-status') });
  assert.equal(read.status, 'draft'); assert.equal(read.attemptNumber, 0);
  const reopenedList = await second.rpc('action/call', { name: 'scan_list', arguments: {}, context: scope('investigation.read', 'reopened-list') });
  assert.equal(reopenedList.items[0].taskId, taskId);
  const secondStopped = await second.stop();
  outcomes.push('same_build_private_store_reopened_task_found_without_rebirth_or_execution');
  const report = { binarySha256: createHash('sha256').update(await readFile(executable)).digest('hex'), manifestSha256: createHash('sha256').update(await readFile(join(directory, 'manifest.json'))).digest('hex'), protocolVersion: description.protocolVersion, cases: outcomes, passed: outcomes.length, first: stopped, second: secondStopped, fullScanningParity: false, liveScanExecutionTested: false, notes: ['No real model/target request; startup refusal happens before operator model configuration.', 'Uses real packaged native executable and SQLite, not a mock AppHandle/native peer.'] };
  await writeFile(join(root, 'result.json'), JSON.stringify(report, null, 2) + '\n');
  return report;
  } finally {
    // EOF asks the SDK to retain a busy/unknown owner. Never SIGKILL it or
    // interpret a diagnostic cleanup as a settled scan/financial receipt.
    for (const process of peers) process.detach();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2), options = {};
  const names = new Map([['--binary', 'binary'], ['--evidence-dir', 'evidenceDirectory']]);
  if (args.length !== 4) throw Error('usage: --binary ABS --evidence-dir NEW_ABS_DIRECTORY');
  for (let i = 0; i < args.length; i += 2) {
    const key = names.get(args[i]); if (!key || options[key]) throw Error('smoke_arguments_invalid'); options[key] = args[i + 1];
  }
  console.log(JSON.stringify(await smokeNativeScanning(options)));
}
