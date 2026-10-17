#!/usr/bin/env node
// Synthetic oviraptor-app-worker: speaks the legacy JSON-RPC frame protocol.
// Used only by host unit tests; never ships as a product binary.
import { lstatSync } from 'node:fs';
import { join } from 'node:path';
import readline from 'node:readline';

const args = process.argv.slice(2);
if (args.length !== 4 || args[0] !== '--app' || args[2] !== '--data-dir') {
  console.error('legacy_bridge_arguments_invalid');
  process.exit(1);
}
const appId = args[1];
const dataDir = args[3];
const snapshot = join(dataDir, 'legacy-input.sqlite3');

function reply(id, payload) {
  process.stdout.write(JSON.stringify(payload) + '\n');
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
rl.on('line', line => {
  let request;
  try { request = JSON.parse(line); }
  catch {
    reply(null, { jsonrpc: '2.0', id: null, error: { code: -32602, message: 'app_protocol_invalid', data: { code: 'app_protocol_invalid' } } });
    return;
  }
  const id = request.id;
  let meta;
  try { meta = lstatSync(snapshot); }
  catch {
    reply(id, { jsonrpc: '2.0', id, error: { code: -32602, message: 'legacy_snapshot_unavailable', data: { code: 'legacy_snapshot_unavailable' } } });
    return;
  }
  if (!meta.isFile() || meta.isSymbolicLink()) {
    reply(id, { jsonrpc: '2.0', id, error: { code: -32602, message: 'legacy_snapshot_unavailable', data: { code: 'legacy_snapshot_unavailable' } } });
    return;
  }
  if (request.method === 'describe') {
    reply(id, {
      jsonrpc: '2.0', id,
      result: {
        protocolVersion: 1, appId, adapter: 'legacy_read_only',
        tools: appId === 'org.oviraptor.assets' ? ['saved_assets_stats'] : ['saved_scans_list'],
        executionAvailable: false,
      },
    });
    return;
  }
  if (request.method !== 'tool/call') {
    reply(id, { jsonrpc: '2.0', id, error: { code: -32602, message: 'legacy_method_unavailable', data: { code: 'legacy_method_unavailable' } } });
    return;
  }
  const name = request.params?.name;
  const project = request.params?.context?.hostBoundProjectId;
  if (appId === 'org.oviraptor.assets' && name === 'saved_assets_stats') {
    reply(id, {
      jsonrpc: '2.0', id,
      result: {
        source: 'explicit_app_private_legacy_snapshot',
        readOnly: true,
        liveExecution: false,
        hostBoundProjectId: project,
        grantRevision: request.params?.context?.grantRevision ?? 0,
        data: { activeTotal: 7, projectTotal: 7, trashedTotal: 0, unit: 'legacy_snapshot', fixture: true },
      },
    });
    return;
  }
  if (appId === 'org.oviraptor.scanning' && name === 'saved_scans_list') {
    reply(id, {
      jsonrpc: '2.0', id,
      result: {
        source: 'explicit_app_private_legacy_snapshot',
        readOnly: true,
        liveExecution: false,
        hostBoundProjectId: project,
        grantRevision: request.params?.context?.grantRevision ?? 0,
        data: { items: [{ id: 'scan-1', projectId: project, name: 'Fixture', status: 'draft' }], limit: 20, coverage: 'bounded_saved_snapshot_projection', executionAvailable: false },
      },
    });
    return;
  }
  reply(id, { jsonrpc: '2.0', id, error: { code: -32602, message: 'legacy_tool_unavailable', data: { code: 'legacy_tool_unavailable' } } });
});
