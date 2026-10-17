// Host-managed read-only legacy bridge.
// Stages legacy-input.sqlite3 and, when an oviraptor-app-worker (or test adapter)
// can serve it, exposes legacy_* tools. Never lists those tools when unavailable.
import { spawn } from 'node:child_process';
import { accessSync, constants as fsConstants, existsSync, lstatSync, realpathSync } from 'node:fs';
import { copyFile, mkdir, rename, rm, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { randomUUID } from 'node:crypto';

export const LEGACY_INPUT_FILENAME = 'legacy-input.sqlite3';
export const ASSETS_APP_ID = 'org.oviraptor.assets';
export const SCANNING_APP_ID = 'org.oviraptor.scanning';

const EMPTY_SCHEMA = Object.freeze({
  type: 'object',
  properties: {},
  required: [],
  additionalProperties: false,
});

/** Tool specs re-registered only when probeLegacyBridge().available. */
export const LEGACY_TOOL_BY_APP = Object.freeze({
  [ASSETS_APP_ID]: Object.freeze({
    name: 'legacy_snapshot_stats',
    description:
      'Read-only aggregate stats from a host-staged legacy desktop snapshot (legacy-input.sqlite3). Present only when the host-managed bridge can serve that snapshot; never implies live reachability or execution.',
    inputSchema: EMPTY_SCHEMA,
    permission: 'inventory.read',
    rustName: 'saved_assets_stats',
  }),
  [SCANNING_APP_ID]: Object.freeze({
    name: 'legacy_saved_scans',
    description:
      'Read-only list of saved scans from a host-staged legacy desktop snapshot (legacy-input.sqlite3). Present only when the host-managed bridge can serve that snapshot; never starts the scan engine.',
    inputSchema: EMPTY_SCHEMA,
    permission: 'investigation.read',
    rustName: 'saved_scans_list',
  }),
});

export function legacyInputPath(dataDir) {
  if (typeof dataDir !== 'string' || !dataDir || !isAbsolute(dataDir)) return null;
  return join(dataDir, LEGACY_INPUT_FILENAME);
}

/** True only for a regular, non-symlink file named legacy-input.sqlite3. */
export function isLegacySnapshotStaged(dataDir) {
  const path = legacyInputPath(dataDir);
  if (!path) return false;
  try {
    const meta = lstatSync(path);
    return meta.isFile() && !meta.isSymbolicLink();
  } catch {
    return false;
  }
}

/**
 * Resolve oviraptor-app-worker path. Prefer explicit option, then
 * OVIRAPTOR_APP_WORKER env. Does not invent a path from PATH alone.
 */
export function resolveLegacyAppWorkerPath({ workerPath } = {}) {
  const candidate = typeof workerPath === 'string' && workerPath
    ? workerPath
    : (typeof process.env.OVIRAPTOR_APP_WORKER === 'string' ? process.env.OVIRAPTOR_APP_WORKER : '');
  if (!candidate || !isAbsolute(candidate)) return null;
  try {
    accessSync(candidate, fsConstants.X_OK);
    const meta = lstatSync(candidate);
    if (!meta.isFile() || meta.isSymbolicLink()) return null;
    return realpathSync(candidate);
  } catch {
    return null;
  }
}

/**
 * Bridge is available only when a staged snapshot exists AND a host-side
 * transport can serve it (injectable adapter for tests, or executable worker).
 * Snapshot alone is not enough — that would revive always-unavailable listings.
 */
export function probeLegacyBridge({ appId, dataDir, workerPath, adapter } = {}) {
  if (!LEGACY_TOOL_BY_APP[appId]) {
    return { available: false, code: 'legacy_app_unsupported' };
  }
  if (!isLegacySnapshotStaged(dataDir)) {
    return { available: false, code: 'legacy_snapshot_unavailable' };
  }
  if (typeof adapter === 'function') {
    return { available: true, mode: 'adapter', tool: LEGACY_TOOL_BY_APP[appId] };
  }
  const resolved = resolveLegacyAppWorkerPath({ workerPath });
  if (!resolved) {
    return { available: false, code: 'legacy_app_worker_unavailable' };
  }
  return { available: true, mode: 'worker', workerPath: resolved, tool: LEGACY_TOOL_BY_APP[appId] };
}

/**
 * Atomically stage a regular file as dataDir/legacy-input.sqlite3.
 * Rejects symlinks and non-files. Does not open or migrate the DB.
 */
export async function stageLegacyInputSnapshot(dataDir, sourcePath) {
  if (typeof dataDir !== 'string' || !isAbsolute(dataDir)) {
    throw Object.assign(Error('host_legacy_data_dir_invalid'), { code: 'host_legacy_data_dir_invalid' });
  }
  if (typeof sourcePath !== 'string' || !isAbsolute(sourcePath)) {
    throw Object.assign(Error('host_legacy_source_invalid'), { code: 'host_legacy_source_invalid' });
  }
  let sourceMeta;
  try { sourceMeta = lstatSync(sourcePath); }
  catch {
    throw Object.assign(Error('host_legacy_source_missing'), { code: 'host_legacy_source_missing' });
  }
  if (!sourceMeta.isFile() || sourceMeta.isSymbolicLink()) {
    throw Object.assign(Error('host_legacy_source_invalid'), { code: 'host_legacy_source_invalid' });
  }
  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  const canonicalDir = realpathSync(dataDir);
  const target = join(canonicalDir, LEGACY_INPUT_FILENAME);
  const staging = join(canonicalDir, `.legacy-staging-${randomUUID()}`);
  try {
    await copyFile(sourcePath, staging);
    await rename(staging, target);
  } catch (error) {
    await rm(staging, { force: true }).catch(() => {});
    throw error;
  }
  return { dataDir: canonicalDir, path: target, staged: true };
}

function normalizeLegacyResult(rustEnvelope, context) {
  const data = rustEnvelope && typeof rustEnvelope === 'object' && rustEnvelope.data && typeof rustEnvelope.data === 'object'
    ? rustEnvelope.data
    : (rustEnvelope && typeof rustEnvelope === 'object' ? rustEnvelope : {});
  return {
    available: true,
    executionAvailable: false,
    readOnly: true,
    source: rustEnvelope?.source ?? 'explicit_app_private_legacy_snapshot',
    hostBoundProjectId: context?.hostBoundProjectId,
    grantRevision: context?.grantRevision,
    ...data,
  };
}

/**
 * Create a legacyBridge adapter suitable for createApplication({ legacyBridge })
 * or host-side tool routing. Spawns oviraptor-app-worker per call when mode=worker.
 */
export function createHostLegacyBridge({ appId, dataDir, workerPath, adapter, timeoutMs = 15000 } = {}) {
  const probe = probeLegacyBridge({ appId, dataDir, workerPath, adapter });
  if (!probe.available) {
    return async () => ({ available: false, code: probe.code, executionAvailable: false });
  }
  if (probe.mode === 'adapter') {
    return async input => adapter(input);
  }
  const binary = probe.workerPath;
  const rustName = LEGACY_TOOL_BY_APP[appId].rustName;
  return async function hostLegacyBridge(input) {
    const requested = typeof input?.name === 'string' ? input.name : rustName;
    // Only the declared projection for this app; reject cross-app smuggling.
    if (requested !== rustName && requested !== LEGACY_TOOL_BY_APP[appId].name) {
      return { available: false, code: 'legacy_tool_unavailable', executionAvailable: false };
    }
    const context = input?.context ?? {};
    const args = input?.arguments && typeof input.arguments === 'object' && !Array.isArray(input.arguments)
      ? input.arguments
      : {};
    const frame = {
      jsonrpc: '2.0',
      id: 1,
      method: 'tool/call',
      params: {
        name: rustName,
        arguments: args,
        context: {
          hostBoundProjectId: context.hostBoundProjectId,
          grantRevision: context.grantRevision ?? 0,
          operationId: typeof context.operationId === 'string' ? context.operationId : 'legacy-read',
        },
      },
    };
    const response = await invokeLegacyWorker(binary, appId, dataDir, frame, timeoutMs);
    if (response?.error) {
      const code = response.error?.data?.code ?? response.error?.message ?? 'legacy_read_failed';
      return { available: false, code: typeof code === 'string' ? code : 'legacy_read_failed', executionAvailable: false };
    }
    return normalizeLegacyResult(response?.result, context);
  };
}

function invokeLegacyWorker(binary, appId, dataDir, frame, timeoutMs) {
  return new Promise((resolvePromise, reject) => {
    let settled = false;
    // Native product binary: argv is --app/--data-dir. .mjs/.js fixtures (tests) run under process.execPath.
    const scriptWorker = /\.(mjs|cjs|js)$/u.test(binary);
    const child = spawn(
      scriptWorker ? process.execPath : binary,
      scriptWorker ? [binary, '--app', appId, '--data-dir', dataDir] : ['--app', appId, '--data-dir', dataDir],
      {
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
        env: { PATH: process.env.PATH ?? '' },
      },
    );
    let buffer = '';
    const fail = code => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try { child.kill(); } catch { /* ignore */ }
      reject(Object.assign(Error(code), { code }));
    };
    const timer = setTimeout(() => fail('legacy_bridge_timeout'), timeoutMs);
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      buffer += chunk;
      const index = buffer.indexOf('\n');
      if (index < 0) return;
      const line = buffer.slice(0, index);
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try { child.stdin.end(); } catch { /* ignore */ }
      try { child.kill(); } catch { /* ignore */ }
      try {
        resolvePromise(JSON.parse(line));
      } catch {
        reject(Object.assign(Error('legacy_protocol_invalid'), { code: 'legacy_protocol_invalid' }));
      }
    });
    child.on('error', () => fail('legacy_app_worker_failed'));
    child.stderr.on('data', () => {});
    child.stdin.write(JSON.stringify(frame) + '\n', error => {
      if (error) fail('legacy_bridge_transport_failed');
    });
  });
}

export function legacyToolResult(structured) {
  return {
    content: [{ type: 'text', text: JSON.stringify(structured) }],
    structuredContent: structured,
  };
}

/**
 * Blocker note for docs/tests: Node app workers cannot spawn oviraptor-app-worker
 * (FS-only permission). Full in-worker createApplication({legacyBridge}) injection
 * from the host stdio path therefore requires a new host↔worker RPC, which is
 * deferred. This module is the staged-file + host-spawned worker path instead.
 */
export const LEGACY_BRIDGE_TRANSPORT_BLOCKER = Object.freeze({
  code: 'legacy_host_to_worker_inject_deferred',
  summary:
    'Installed Node workers have no child-process grant and cannot spawn oviraptor-app-worker. Host stages legacy-input.sqlite3 and routes legacy_* tools itself when the worker binary is available; createApplication({legacyBridge}) remains the trusted in-process embedding seam (tests / future host composition).',
});

/** Resolve the app data directory the host would use for an installed app row. */
export function appDataDirectory(root, app) {
  if (app?.data_path && typeof app.data_path === 'string' && isAbsolute(app.data_path)) return app.data_path;
  if (!root || !app?.id) return null;
  return join(root, 'data', app.id);
}
