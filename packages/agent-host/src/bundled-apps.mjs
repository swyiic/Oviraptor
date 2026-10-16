import { access, readFile } from 'node:fs/promises';
import { constants as fsConstants } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { hostError } from './store.mjs';

const STATE_KEY = 'bundled_apps_state';
const MANIFEST_NAME = 'bundled-apps.json';

function readState(store) {
  const row = store.db.prepare('SELECT value FROM host_settings WHERE key=?').get(STATE_KEY);
  if (!row) return { schemaVersion: 1, items: {} };
  try {
    const parsed = JSON.parse(row.value);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return { schemaVersion: 1, items: {} };
    return { schemaVersion: 1, items: parsed.items && typeof parsed.items === 'object' && !Array.isArray(parsed.items) ? parsed.items : {} };
  } catch {
    return { schemaVersion: 1, items: {} };
  }
}

function writeState(store, state) {
  store.db.prepare('INSERT OR REPLACE INTO host_settings(key,value) VALUES(?,?)').run(STATE_KEY, JSON.stringify({ schemaVersion: 1, items: state.items }));
}

function itemView(id, entry, live) {
  return {
    id,
    builtin: entry?.builtin !== false,
    uninstallable: entry?.uninstallable !== false,
    enableRequested: Boolean(entry?.enable),
    installed: Boolean(live),
    enabled: Boolean(live?.enabled),
    version: live?.version ?? null,
    status: entry?.lastStatus ?? (live ? (live.enabled ? 'enabled' : 'installed') : 'pending'),
    error: entry?.error ?? null,
  };
}

/** Resolve directory that contains packaged apps for this host layout. */
export function bundledAppsRoot({ dataDir, overrideRoot } = {}) {
  if (typeof overrideRoot === 'string' && overrideRoot) return resolve(overrideRoot);
  if (process.env.OVIRAPTOR_BUNDLED_APPS_ROOT) return resolve(process.env.OVIRAPTOR_BUNDLED_APPS_ROOT);
  // Distribution: packages/agent-host/src -> ../../.. = distribution root
  const fromHost = fileURLToPath(new URL('../../..', import.meta.url));
  return fromHost;
}

export function bundledAppsManifestPath(root) {
  return join(root, 'packages', 'agent-host', MANIFEST_NAME);
}

export async function loadBundledAppsManifest(root) {
  const path = bundledAppsManifestPath(root);
  let text;
  try { text = await readFile(path, 'utf8'); }
  catch (error) {
    if (error.code === 'ENOENT') return { schemaVersion: 1, apps: [], path, missing: true };
    throw error;
  }
  const parsed = JSON.parse(text);
  if (parsed?.schemaVersion !== 1 || !Array.isArray(parsed.apps)) throw hostError('host_bundled_apps_manifest_invalid');
  for (const app of parsed.apps) {
    if (!app || typeof app.id !== 'string' || typeof app.package !== 'string') throw hostError('host_bundled_apps_manifest_invalid');
  }
  return { ...parsed, path, missing: false };
}

async function resolvePackageDirectory(root, relative) {
  if (isAbsolute(relative)) return relative;
  const candidates = [resolve(root, relative), resolve(root, '.agent-build', relative)];
  for (const candidate of candidates) {
    try { await access(candidate, fsConstants.R_OK); return candidate; }
    catch { /* try next */ }
  }
  return candidates[0];
}

/**
 * Install and optionally enable apps listed in bundled-apps.json.
 * Idempotent: already-installed apps are not re-copied; enable is attempted only when requested and not yet enabled.
 * Failures are recorded and do not throw; host startup must continue.
 */
export async function ensureBundledApps(host, { root = bundledAppsRoot({ dataDir: host.dataDir }), onEvent } = {}) {
  const emit = onEvent ?? (event => host.emit?.(event));
  const state = readState(host.store);
  const manifest = await loadBundledAppsManifest(root);
  const results = [];
  if (manifest.missing) {
    writeState(host.store, state);
    return { root, items: results, missingManifest: true };
  }
  for (const entry of manifest.apps) {
    const prior = state.items[entry.id] ?? {};
    const next = {
      ...prior,
      id: entry.id,
      package: entry.package,
      builtin: entry.builtin !== false,
      uninstallable: entry.uninstallable !== false,
      enable: entry.enable !== false,
      lastStatus: prior.lastStatus ?? 'pending',
      error: prior.error ?? null,
    };
    try {
      const existing = host.store.applications().find(app => app.id === entry.id);
      if (existing) {
        host.store.db.prepare('UPDATE applications SET builtin=?,uninstallable=? WHERE id=?')
          .run(Number(next.builtin), Number(next.uninstallable), entry.id);
        if (next.enable && !existing.enabled) {
          await host.apps.setEnabled(entry.id, true);
          next.lastStatus = 'enabled';
          next.error = null;
        } else {
          next.lastStatus = existing.enabled ? 'enabled' : 'installed';
          next.error = null;
        }
      } else {
        const source = await resolvePackageDirectory(root, entry.package);
        try { await access(source, fsConstants.R_OK); }
        catch {
          const code = 'host_bundled_package_missing';
          next.lastStatus = 'error';
          next.error = code;
          emit({ type: 'application.bundled_failed', appId: entry.id, code });
          state.items[entry.id] = next;
          results.push(itemView(entry.id, next, null));
          continue;
        }
        const installed = await host.apps.install(source);
        host.store.db.prepare('UPDATE applications SET builtin=?,uninstallable=? WHERE id=?')
          .run(Number(next.builtin), Number(next.uninstallable), entry.id);
        if (next.enable) {
          await host.apps.setEnabled(entry.id, true);
          next.lastStatus = 'enabled';
        } else {
          next.lastStatus = 'installed';
        }
        next.error = null;
        emit({ type: 'applications.changed', appId: installed.id });
      }
    } catch (error) {
      const code = typeof error?.code === 'string' ? error.code : 'host_bundled_install_failed';
      next.lastStatus = 'error';
      next.error = code;
      emit({ type: 'application.bundled_failed', appId: entry.id, code });
    }
    state.items[entry.id] = next;
    const live = host.store.applications().find(app => app.id === entry.id);
    results.push(itemView(entry.id, next, live));
  }
  writeState(host.store, state);
  return { root, items: results, missingManifest: false };
}

export function bundledAppsStatus(host) {
  const state = readState(host.store);
  const live = new Map(host.store.applications().map(app => [app.id, app]));
  const items = Object.values(state.items).map(entry => itemView(entry.id, entry, live.get(entry.id)));
  // Include live builtins that were marked without a state row (manual installs).
  for (const app of live.values()) {
    if (app.builtin && !state.items[app.id]) items.push(itemView(app.id, { builtin: true, uninstallable: app.uninstallable !== false, enable: app.enabled, lastStatus: app.enabled ? 'enabled' : 'installed' }, app));
  }
  items.sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  return { schemaVersion: 1, items };
}
