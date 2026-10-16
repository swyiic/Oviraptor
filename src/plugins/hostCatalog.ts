import type { BuiltinPluginId, BuiltinPluginManifest, PluginCapabilityDeclaration, PluginCommandDeclaration, PluginEnabledState } from "./types";

const command = (id: PluginCommandDeclaration["id"], kind: PluginCommandDeclaration["kind"]): PluginCommandDeclaration => ({
  id, kind, scope: "selected-project", requiresModel: false, requiresNetwork: false,
  permission: kind === "read" ? "existing-project-read" : "host-navigation", implementation: "built-in",
});
const capability = (id: string, label: PluginCapabilityDeclaration["label"], requiresModel: boolean,
  requiresNetwork: boolean, requiredProviders: readonly string[] = []): PluginCapabilityDeclaration => ({
  id, label, requiresModel, requiresNetwork, requiredProviders, authority: "existing-domain-gates", hostCallable: false,
});
function freeze<T>(value: T): T {
  if (value && typeof value === "object") {
    for (const item of Object.values(value)) freeze(item);
    Object.freeze(value);
  }
  return value;
}

/**
 * Fallback shell registry mirrored from packages/agent-host/shell-apps.json.
 * Runtime prefers GET /api/apps/shell (and /api/apps) when OVIRAPTOR_APP_HOST=agent-host.
 * Kept in-process so unit harnesses and offline desktop UI stay synchronous.
 */
const FALLBACK_PLUGINS: readonly BuiltinPluginManifest[] = freeze([
  {
    id: "asset", label: ["资产", "Assets"], description: ["管理资产、归属与公开暴露面；本地管理不需要模型。", "Manage assets, ownership and public exposure. Local management needs no model."],
    defaultView: "dashboard", views: ["dashboard", "tasks", "assets", "ownership", "exposure", "changes", "quarantine", "query"], hasLogs: true, requirements: [],
    pages: [
      { id: "dashboard", view: "dashboard", label: ["仪表盘", "Dashboard"] },
      { id: "tasks", view: "tasks", label: ["任务中心", "Tasks"] },
      { id: "assets", view: "assets", label: ["资产", "Assets"] },
      { id: "ownership", view: "ownership", label: ["归属门禁", "Ownership gate"] },
      { id: "exposure", view: "exposure", label: ["公开暴露面", "Public exposure"] },
      { id: "changes", view: "changes", label: ["变化对比", "Changes"] },
      { id: "quarantine", view: "quarantine", label: ["隔离区", "Quarantine"] },
      { id: "query", view: "query", label: ["新建查询", "New query"] },
      { id: "logs", view: "logs", label: ["资产日志", "Asset logs"] },
    ],
    commands: [command("assets.list-link-candidates", "read"), command("assets.open-workspace", "navigate")],
    capabilities: [
      capability("assets.local-management", ["本地查询与整理", "Local search and management"], false, false),
      capability("assets.fofa-collection", ["FOFA 采集", "FOFA collection"], false, true, ["fofa"]),
      capability("assets.probe", ["授权资产探测", "Authorized asset probing"], false, true),
      capability("assets.ownership", ["规则归属分析", "Rule-based ownership"], false, false),
      capability("assets.exposure", ["授权公开暴露面", "Authorized public exposure"], false, true),
    ],
  },
  {
    id: "sentinel", label: ["扫描", "Scanning"], description: ["查看扫描结果，或进入现有工作台发起 Web、源码、灰盒和 CI 调查。", "Review scan results, or use the existing workbench for Web, source, greybox and CI investigations."],
    defaultView: "sentinel", views: ["sentinel"], hasLogs: true, requirements: [],
    pages: [
      { id: "dialog", view: "sentinel", section: "dialog", label: ["调查对话", "Investigation dialog"] },
      { id: "urls", view: "sentinel", section: "results", label: ["这次结果", "This investigation"] },
      { id: "queue", view: "sentinel", section: "queue", label: ["任务与成本", "Tasks and cost"] },
      { id: "skills", view: "sentinel", section: "workbench", label: ["知识与策略", "Knowledge and policy"] },
      { id: "logs", view: "logs", label: ["扫描日志", "Scan logs"] },
    ],
    commands: [command("scanning.list-link-candidates", "read"), command("scanning.open-workspace", "navigate")],
    capabilities: [
      capability("scanning.saved-results", ["已保存的结果与运行历史", "Saved results and run history"], false, false),
      capability("scanning.web-draft", ["Web 调查草稿", "Web investigation draft"], false, false),
      capability("scanning.native-investigation", ["现有 Native 调查执行", "Existing Native investigation execution"], true, true, ["configured-model"]),
    ],
  },
  {
    id: "hackerone", label: ["HackerOne", "HackerOne"], description: ["保留现有 HackerOne 项目与 Scope 看板及配置门禁。", "Existing HackerOne programs and scope board, with its current configuration gate."],
    defaultView: "hackerone", views: ["hackerone"], hasLogs: true, requirements: ["hackerone-profile"],
    pages: [{ id: "hackerone", view: "hackerone", label: ["HackerOne 项目", "HackerOne programs"] }, { id: "logs", view: "logs", label: ["HackerOne 变化", "HackerOne changes"] }],
    commands: [],
    capabilities: [
      capability("hackerone.cached-records", ["已缓存的项目记录", "Cached program records"], false, false),
      capability("hackerone.sync", ["官方 API 同步", "Official API sync"], false, true, ["hackerone"]),
    ],
  },
] satisfies BuiltinPluginManifest[]);

export type AppHostMode = "agent-host" | "legacy";

export interface HostShellAppItem extends BuiltinPluginManifest {
  hostAppId?: string | null;
  installed?: boolean;
  enabled?: boolean;
  icon?: string;
  status?: string;
}

export interface HostAppsClient {
  listShellApps: () => Promise<HostShellAppItem[]>;
  setAppEnabled?: (hostAppId: string, enabled: boolean) => Promise<void>;
}

const SHELL_IDS: readonly BuiltinPluginId[] = ["asset", "sentinel", "hackerone"];
const HOST_APP_IDS: Readonly<Record<Exclude<BuiltinPluginId, "hackerone">, string>> = {
  asset: "org.oviraptor.assets",
  sentinel: "org.oviraptor.scanning",
};

let activePlugins: readonly BuiltinPluginManifest[] = FALLBACK_PLUGINS;
let hostEnabledOverlay: Partial<Record<BuiltinPluginId, boolean>> | undefined;

function readViteEnv(name: string): string | undefined {
  try {
    // Built dynamically so CJS harnesses that eval transpiled TS never see a static import.meta token.
    const reader = new Function("key", "try { const env = import.meta && import.meta.env; return env && typeof env[key] === 'string' ? env[key] : undefined; } catch { return undefined; }");
    const value = reader(name);
    return typeof value === "string" ? value : undefined;
  } catch { return undefined; }
}
function envValue(name: string): string | undefined {
  try {
    const nodeProcess = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process;
    if (nodeProcess?.env && typeof nodeProcess.env[name] === "string") return nodeProcess.env[name];
  } catch { /* Browser builds may omit process. */ }
  const fromVite = readViteEnv(name) || readViteEnv(`VITE_${name}`);
  if (fromVite) return fromVite;
  try {
    const injected = (globalThis as { __OVIRAPTOR_ENV__?: Record<string, string | undefined> }).__OVIRAPTOR_ENV__;
    if (injected && typeof injected[name] === "string") return injected[name];
  } catch { /* Ignore. */ }
  return undefined;
}

/** Dual-track switch. Default agent-host: navigation is owned by /api/apps(+ /shell). */
export function resolveAppHostMode(override?: string): AppHostMode {
  const value = (override || envValue("OVIRAPTOR_APP_HOST") || envValue("VITE_OVIRAPTOR_APP_HOST") || "agent-host").trim().toLowerCase();
  return value === "legacy" ? "legacy" : "agent-host";
}

export function hostAppIdForPlugin(id: BuiltinPluginId): string | undefined {
  return id === "hackerone" ? undefined : HOST_APP_IDS[id];
}

export function pluginIdForHostApp(hostAppId: string): BuiltinPluginId | undefined {
  const entry = (Object.entries(HOST_APP_IDS) as [BuiltinPluginId, string][]).find(([, value]) => value === hostAppId);
  return entry?.[0];
}

function asPluginManifest(item: HostShellAppItem): BuiltinPluginManifest | undefined {
  if (!item || typeof item !== "object") return undefined;
  if (!SHELL_IDS.includes(item.id as BuiltinPluginId)) return undefined;
  if (!Array.isArray(item.label) || item.label.length !== 2) return undefined;
  if (!Array.isArray(item.description) || item.description.length !== 2) return undefined;
  if (!Array.isArray(item.views) || !item.defaultView) return undefined;
  if (!Array.isArray(item.pages)) return undefined;
  return freeze({
    id: item.id as BuiltinPluginId,
    label: item.label,
    description: item.description,
    defaultView: item.defaultView,
    views: item.views,
    hasLogs: Boolean(item.hasLogs),
    requirements: Array.isArray(item.requirements) ? item.requirements.filter((value): value is "hackerone-profile" => value === "hackerone-profile") : [],
    pages: item.pages,
    commands: Array.isArray(item.commands) ? item.commands as BuiltinPluginManifest["commands"] : [],
    capabilities: Array.isArray(item.capabilities) ? item.capabilities as BuiltinPluginManifest["capabilities"] : [],
  });
}

/** Replace the in-memory shell catalog from a host /api/apps/shell payload. */
export function applyHostShellApps(items: readonly HostShellAppItem[]): readonly BuiltinPluginManifest[] {
  const next: BuiltinPluginManifest[] = [];
  const enabled: Partial<Record<BuiltinPluginId, boolean>> = {};
  for (const id of SHELL_IDS) {
    const raw = items.find(item => item.id === id);
    const mapped = raw ? asPluginManifest(raw) : FALLBACK_PLUGINS.find(item => item.id === id);
    if (!mapped) continue;
    next.push(mapped);
    if (raw && typeof raw.enabled === "boolean" && id !== "hackerone") enabled[id] = raw.enabled;
  }
  for (const fallback of FALLBACK_PLUGINS) if (!next.some(item => item.id === fallback.id)) next.push(fallback);
  activePlugins = freeze(next);
  hostEnabledOverlay = Object.keys(enabled).length ? enabled : undefined;
  return activePlugins;
}

export function resetHostShellApps(): void {
  activePlugins = FALLBACK_PLUGINS;
  hostEnabledOverlay = undefined;
}

/** Live shell plugin table. Starts as the host registry fallback; refresh via applyHostShellApps. */
export const BUILTIN_PLUGINS: readonly BuiltinPluginManifest[] = new Proxy([] as BuiltinPluginManifest[], {
  get(_target, property, receiver) {
    const list = activePlugins as unknown as Record<string | symbol, unknown>;
    const value = Reflect.get(list, property, receiver);
    return typeof value === "function" ? value.bind(activePlugins) : value;
  },
  has(_target, property) { return property in (activePlugins as object); },
  ownKeys() { return Reflect.ownKeys(activePlugins as object); },
  getOwnPropertyDescriptor(_target, property) {
    return Reflect.getOwnPropertyDescriptor(activePlugins as object, property);
  },
}) as unknown as readonly BuiltinPluginManifest[];

export function isBuiltinPluginId(value: unknown): value is BuiltinPluginId {
  return typeof value === "string" && activePlugins.some(plugin => plugin.id === value);
}

export function getBuiltinPlugin(id: BuiltinPluginId): BuiltinPluginManifest {
  const plugin = activePlugins.find(item => item.id === id);
  if (!plugin) throw Error("plugin_unknown");
  return plugin;
}

export function hostEnabledOverlayState(): Partial<Record<BuiltinPluginId, boolean>> | undefined {
  return hostEnabledOverlay;
}

/** Merge preference storage with optional host enablement overlay (agent-host mode). */
export function mergeHostEnabledState(preferences: PluginEnabledState): PluginEnabledState {
  if (!hostEnabledOverlay) return preferences;
  return {
    asset: hostEnabledOverlay.asset ?? preferences.asset,
    sentinel: hostEnabledOverlay.sentinel ?? preferences.sentinel,
    hackerone: preferences.hackerone,
  };
}

function defaultOrigin(): string | undefined {
  return envValue("OVIRAPTOR_HOST_ORIGIN") || envValue("VITE_OVIRAPTOR_HOST_ORIGIN")
    || (typeof globalThis !== "undefined" && "location" in globalThis
      ? (globalThis as { location?: { origin?: string } }).location?.origin
      : undefined);
}

function defaultSession(): string | undefined {
  return envValue("OVIRAPTOR_HOST_SESSION") || envValue("VITE_OVIRAPTOR_HOST_SESSION") || undefined;
}

type AgentHostEndpoint = { origin: string; session: string; source?: string };

/** Resolve origin/session from env injection or the desktop sidecar command. */
export async function resolveHostCredentials(): Promise<{ origin?: string; session?: string }> {
  const fromEnvOrigin = envValue("OVIRAPTOR_HOST_ORIGIN") || envValue("VITE_OVIRAPTOR_HOST_ORIGIN");
  const fromEnvSession = envValue("OVIRAPTOR_HOST_SESSION") || envValue("VITE_OVIRAPTOR_HOST_SESSION");
  if (fromEnvOrigin && fromEnvSession) return { origin: fromEnvOrigin, session: fromEnvSession };
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const endpoint = await invoke<AgentHostEndpoint | null>("agent_host_endpoint");
    if (endpoint && typeof endpoint.origin === "string" && typeof endpoint.session === "string") {
      const bag = (globalThis as { __OVIRAPTOR_ENV__?: Record<string, string | undefined> });
      bag.__OVIRAPTOR_ENV__ = {
        ...(bag.__OVIRAPTOR_ENV__ ?? {}),
        OVIRAPTOR_HOST_ORIGIN: endpoint.origin,
        OVIRAPTOR_HOST_SESSION: endpoint.session,
        OVIRAPTOR_APP_HOST: "agent-host",
      };
      return { origin: endpoint.origin, session: endpoint.session };
    }
  } catch { /* Browser / missing sidecar keeps env-only credentials. */ }
  return { origin: fromEnvOrigin || defaultOrigin(), session: fromEnvSession || defaultSession() };
}

/** HTTP client for the agent-host /api/apps surface used by the desktop shell. */
export async function createHostAppsClient(options: {
  origin?: string;
  session?: string;
  fetchImpl?: typeof fetch;
} = {}): Promise<HostAppsClient> {
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  const resolved = (options.origin && options.session) ? { origin: options.origin, session: options.session } : await resolveHostCredentials();
  const origin = (options.origin ?? resolved.origin ?? "").replace(/\/$/, "");
  const session = options.session ?? resolved.session ?? "";
  async function request(path: string, init: RequestInit = {}): Promise<unknown> {
    if (!fetchImpl) throw Error("host_apps_fetch_unavailable");
    if (!origin) throw Error("host_apps_origin_missing");
    if (!session) throw Error("host_apps_session_missing");
    if (!path.startsWith("/api/")) throw Error("host_apps_path_invalid");
    const response = await fetchImpl(`${origin}${path}`, {
      ...init,
      credentials: "same-origin",
      headers: {
        "X-Oviraptor-Session": session,
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        ...(init.headers as Record<string, string> | undefined),
      },
    });
    const text = await response.text();
    let value: unknown = null;
    try { value = text ? JSON.parse(text) : null; } catch { throw Error("host_apps_invalid_response"); }
    if (!response.ok) throw Error(typeof (value as { error?: { code?: string } })?.error === "object"
      ? String((value as { error?: { code?: string } }).error?.code || "host_apps_request_failed")
      : "host_apps_request_failed");
    return value;
  }
  return {
    async listShellApps() {
      const value = await request("/api/apps/shell") as { items?: HostShellAppItem[] };
      if (!Array.isArray(value?.items)) throw Error("host_apps_invalid_list");
      return value.items;
    },
    async setAppEnabled(hostAppId, enabled) {
      await request(`/api/apps/${encodeURIComponent(hostAppId)}/state`, {
        method: "POST",
        body: JSON.stringify({ enabled }),
      });
    },
  };
}

/** Refresh catalog from host. Returns false when fallback remains in use. */
export async function refreshShellCatalogFromHost(client?: HostAppsClient): Promise<boolean> {
  if (resolveAppHostMode() === "legacy") {
    resetHostShellApps();
    return false;
  }
  try {
    const api = client ?? await createHostAppsClient();
    const items = await api.listShellApps();
    applyHostShellApps(items);
    return true;
  } catch {
    // Keep last good / fallback catalog so the desktop shell stays navigable offline.
    return false;
  }
}
