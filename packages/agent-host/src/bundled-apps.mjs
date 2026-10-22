import { access, mkdir, readFile, rm } from 'node:fs/promises';
import { constants as fsConstants, existsSync } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { compareVersion } from './upgrades.mjs';
import { createDirectoryPackage, validatePackageDirectory } from '../../app-sdk/src/index.mjs';
import { hostError } from './store.mjs';

const STATE_KEY = 'bundled_apps_state';
const MANIFEST_NAME = 'bundled-apps.json';
const FIRST_PARTY_APPS = new Set(['org.oviraptor.assets', 'org.oviraptor.scanning']);
const permissionContract = manifest => JSON.stringify(manifest.permissions.map(({ id, description }) => ({ id, description })).sort((a, b) => a.id.localeCompare(b.id)));

function restoreCompatibleBundledGrants(host, app, previous) {
  const db = host.store.db;
  db.exec('BEGIN IMMEDIATE');
  try {
    const current = host.store.application(app.id);
    if (!current.enabled || current.revision !== app.revision || current.digest !== app.digest) throw hostError('host_app_revision_conflict', 409);
    for (const saved of previous) {
      const grant = host.store.grant(app.id, saved.projectId);
      // The internal disable and upgrade each revoke once. A later explicit
      // permission change must win instead of being overwritten by restoration.
      if (grant.revision !== saved.revision + 2 || grant.permissions.length || grant.disclosureRuntime !== saved.disclosureRuntime) throw hostError('host_grant_revision_conflict', 409);
      host.apps.setGrants(app.id, saved.projectId, saved.permissions, grant.revision, saved.disclosureRuntime);
    }
    db.exec('COMMIT');
  } catch (error) { db.exec('ROLLBACK'); throw error; }
}

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

function rootLooksPackaged(root) {
  return existsSync(join(root, 'packages', 'agent-host', MANIFEST_NAME))
    || existsSync(join(root, '.agent-build', 'apps'))
    || existsSync(join(root, 'apps'));
}

/** Resolve directory that contains packaged apps for this host layout. */
export function bundledAppsRoot({ dataDir, overrideRoot } = {}) {
  if (typeof overrideRoot === 'string' && overrideRoot) return resolve(overrideRoot);
  if (process.env.OVIRAPTOR_BUNDLED_APPS_ROOT) return resolve(process.env.OVIRAPTOR_BUNDLED_APPS_ROOT);
  // Distribution / monorepo: packages/agent-host/src -> ../../.. = repo or package root
  const fromHost = fileURLToPath(new URL('../../..', import.meta.url));
  const candidates = [fromHost, process.cwd()];
  if (typeof dataDir === 'string' && dataDir) {
    candidates.push(resolve(dataDir, '..', '..'));
    candidates.push(resolve(dataDir, '..'));
  }
  for (const candidate of candidates) {
    if (candidate && rootLooksPackaged(candidate)) return resolve(candidate);
  }
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

async function readable(path) {
  try { await access(path, fsConstants.R_OK); return true; }
  catch { return false; }
}

async function isPackagedDirectory(path) {
  return readable(join(path, 'manifest.json'));
}

async function packageFromSource(root, sourceDirectory) {
  const specPath = join(sourceDirectory, 'app.package.json');
  if (!(await readable(specPath))) return null;
  const spec = JSON.parse(await readFile(specPath, 'utf8'));
  if (!spec?.manifest?.id || !spec?.manifest?.version) return null;
  const outputDirectory = join(root, '.agent-build', 'apps', `${spec.manifest.id}-${spec.manifest.version}`);
  if (await isPackagedDirectory(outputDirectory)) return outputDirectory;
  await mkdir(join(root, '.agent-build', 'apps'), { recursive: true });
  await rm(outputDirectory, { recursive: true, force: true });
  await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
  return outputDirectory;
}

/**
 * Resolve a packaged app directory for install.
 * Prefers prebuilt packages; when only source (apps/assets) exists, packages into .agent-build/apps.
 */
export async function resolvePackageDirectory(root, relative) {
  if (isAbsolute(relative)) return relative;
  const leaf = relative.split('/').pop();
  const bare = leaf.replace(/-\d+\.\d+\.\d+$/, '');
  const shortName = bare.replace(/^org\.oviraptor\./, '');
  const packagedCandidates = [
    resolve(root, relative),
    resolve(root, '.agent-build', relative),
    resolve(root, '.agent-build', 'apps', leaf),
  ];
  for (const candidate of packagedCandidates) {
    if (await isPackagedDirectory(candidate)) return candidate;
  }
  const sourceCandidates = [
    resolve(root, 'apps', shortName),
    resolve(root, 'apps', bare),
  ];
  for (const source of sourceCandidates) {
    if (!(await readable(join(source, 'app.package.json')))) continue;
    const packaged = await packageFromSource(root, source);
    if (packaged) return packaged;
  }
  return packagedCandidates[0];
}

/**
 * Install and optionally enable apps listed in bundled-apps.json.
 * Idempotent: already-installed apps are not re-copied; enable is attempted only when requested and not yet enabled.
 * Failures are recorded and do not throw; host startup must continue.
 */
export async function ensureBundledApps(host, { root = bundledAppsRoot({ dataDir: host.dataDir }), onEvent, force = false } = {}) {
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
      let existing = host.store.applications().find(app => app.id === entry.id);
      let retainedGrants;
      // Startup keeps installed data intact. Only the explicit update action may upgrade.
      if (existing && force) {
        const source = await resolvePackageDirectory(root, entry.package);
        const checked = await validatePackageDirectory(source);
        if (checked.manifest.id !== existing.id) throw hostError('host_app_upgrade_identity_mismatch');
        if (compareVersion(checked.manifest.version, existing.version) > 0) {
          if (FIRST_PARTY_APPS.has(existing.id) && existing.builtin && next.builtin && existing.enabled && next.enable
            && permissionContract(existing.manifest) === permissionContract(checked.manifest)) retainedGrants = host.store.grants(existing.id);
          if (existing.enabled) await host.apps.setEnabled(existing.id, false);
          existing = host.store.application(existing.id);
          await host.apps.upgrade(existing.id, source, { expectedRevision: existing.revision });
          existing = host.store.application(existing.id);
        }
      }
      if (existing) {
        host.store.db.prepare('UPDATE applications SET builtin=?,uninstallable=? WHERE id=?')
          .run(Number(next.builtin), Number(next.uninstallable), entry.id);
        if (next.enable && (!existing.enabled || force)) {
          await host.apps.setEnabled(entry.id, true);
          if (retainedGrants) restoreCompatibleBundledGrants(host, host.store.application(entry.id), retainedGrants);
          next.lastStatus = 'enabled';
          next.error = null;
        } else {
          next.lastStatus = existing.enabled ? 'enabled' : 'installed';
          next.error = null;
        }
      } else {
        const source = await resolvePackageDirectory(root, entry.package);
        if (!(await isPackagedDirectory(source))) {
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
