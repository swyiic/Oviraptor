import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const MANIFEST_PATH = join(dirname(fileURLToPath(import.meta.url)), '..', 'shell-apps.json');

function loadManifest() {
  const parsed = JSON.parse(readFileSync(MANIFEST_PATH, 'utf8'));
  if (parsed?.schemaVersion !== 1 || !Array.isArray(parsed.apps)) throw Object.assign(new Error('host_shell_apps_invalid'), { code: 'host_shell_apps_invalid', status: 500 });
  return parsed;
}

const manifest = loadManifest();

const command = (id, kind) => ({
  id, kind, scope: 'selected-project', requiresModel: false, requiresNetwork: false,
  permission: kind === 'read' ? 'existing-project-read' : 'host-navigation', implementation: 'built-in',
});

function freeze(value) {
  if (value && typeof value === 'object') {
    for (const item of Object.values(value)) freeze(item);
    Object.freeze(value);
  }
  return value;
}

/** Compile-time shell contributions owned by the agent host (not the Tauri catalog). */
export function shellAppEntries() {
  return manifest.apps;
}

export function shellModuleByHostAppId(hostAppId) {
  if (typeof hostAppId !== 'string') return undefined;
  return manifest.apps.find(entry => entry.hostAppId === hostAppId);
}

export function toBuiltinPluginManifest(entry) {
  return freeze({
    id: entry.shellModuleId,
    hostAppId: entry.hostAppId,
    label: entry.label,
    description: entry.description,
    defaultView: entry.defaultView,
    views: entry.views,
    hasLogs: entry.hasLogs,
    requirements: entry.requirements ?? [],
    pages: entry.pages,
    commands: (entry.commands ?? []).map(item => command(item.id, item.kind)),
    capabilities: (entry.capabilities ?? []).map(item => ({
      id: item.id,
      label: item.label,
      requiresModel: Boolean(item.requiresModel),
      requiresNetwork: Boolean(item.requiresNetwork),
      requiredProviders: item.requiredProviders ?? [],
      authority: 'existing-domain-gates',
      hostCallable: false,
    })),
  });
}

export function shellBuiltinPlugins() {
  return freeze(manifest.apps.map(toBuiltinPluginManifest));
}

/**
 * Desktop shell catalog: every registered shell module, merged with live /api/apps state when present.
 * HackerOne remains shell-only until it has a host package.
 */
export function shellCatalog(appsList = []) {
  const byId = new Map((appsList ?? []).map(app => [app.id, app]));
  return {
    schemaVersion: 1,
    items: manifest.apps.map(entry => {
      const live = entry.hostAppId ? byId.get(entry.hostAppId) : undefined;
      const plugin = toBuiltinPluginManifest(entry);
      return {
        ...plugin,
        hostAppId: entry.hostAppId,
        icon: entry.icon,
        installed: entry.hostAppId ? Boolean(live) : true,
        // Desktop Vue workspaces (asset/sentinel) stay navigable even when the
        // packaged host app is not yet installed; missing packages must not
        // blank the sidebar. Live.enabled still wins once installed.
        enabled: entry.hostAppId ? (live ? Boolean(live.enabled) : true) : true,
        uninstallable: live ? live.uninstallable !== false && !live.builtin : false,
        builtin: entry.hostAppId ? Boolean(live?.builtin ?? true) : true,
        version: live?.version ?? null,
        status: live?.status ?? (entry.hostAppId ? 'missing' : 'shell-only'),
        hostPages: live?.pages ?? [],
      };
    }),
  };
}

export function shellEnrichmentForApp(appId) {
  const entry = shellModuleByHostAppId(appId);
  if (!entry) return undefined;
  return {
    shellModuleId: entry.shellModuleId,
    shell: {
      label: entry.label,
      description: entry.description,
      defaultView: entry.defaultView,
      views: entry.views,
      hasLogs: entry.hasLogs,
      requirements: entry.requirements ?? [],
      icon: entry.icon,
      pages: entry.pages,
    },
  };
}
