/** Configuration center section keys (2026-10-17 refactor). */
export const CONFIG_CENTER_SECTIONS = [
  "runtime-model",
  "skills",
  "apps",
  "data-permissions",
  "profile-settings",
  "advanced",
] as const;

export type ConfigCenterSection = (typeof CONFIG_CENTER_SECTIONS)[number];

/** Legacy App.vue keys → new sections (deep links / managePlugins keep working). */
export const CONFIG_SECTION_ALIASES: Record<string, ConfigCenterSection> = {
  "agent-runtime": "runtime-model",
  "scan-models": "runtime-model",
  models: "runtime-model",
  profiles: "profile-settings",
  "legacy-scopes": "advanced",
  rules: "advanced",
  workers: "advanced",
  runtime: "advanced",
  plugins: "apps",
  "runtime-model": "runtime-model",
  skills: "skills",
  apps: "apps",
  "data-permissions": "data-permissions",
  advanced: "advanced",
  "profile-settings": "profile-settings",
};

/** A category changes only its own profile fields; machine settings keep their owners. */
export const PROFILE_SETTINGS_NAV = [
  { route: "asset-apis", section: "assetApis", label: ["资产 API", "Asset APIs"], description: ["FOFA 账户与 Key 独立保存，新的资产任务读取此类默认配置。", "FOFA accounts and keys save independently; new asset jobs read this category’s default."] },
  { route: "collection", section: "collection", label: ["资产采集", "Asset collection"], description: ["采集页大小、范围与请求节奏，新的资产任务读取此类默认配置。", "Collection page size, scope and request timing for new asset jobs using this category’s default."] },
  { route: "probe", section: "probe", label: ["资产探测", "Asset probing"], description: ["探测速率、并发与超时。这里的并发数是探测请求数。", "Probe rate, request concurrency and timeouts."] },
  { route: "content-rules", section: "rules", label: ["内容分类", "Content classification"], description: ["资产内容关键词与阈值；项目页面隔离规则另行管理。", "Asset content keywords and thresholds. Project page isolation has its own settings."] },
  { route: "nest-strategy", section: "agent", label: ["Nest Agent 策略", "Nest agent policy"], description: ["扫描分层、预算与请求审计独立配置，新任务冻结此类默认版本。", "Scan tiers, budgets and request auditing have independent defaults, frozen for new tasks."] },
  { route: "code-rules", section: "securityRules", label: ["代码规则库", "Code rule libraries"], description: ["代码规则包是全局配置；分析器镜像使用此类独立默认，两者分别保存。", "Code rule packs are global. Analyzer images use this category’s independent default and save separately."] },
  { route: "business-network", section: "network", label: ["业务网络", "Business networking"], description: ["资产 API 与扫描网络参数。模型连接和用途在统一模型页管理，聊天网络仍独立于业务网络。", "Asset API and scan network settings. Connections and usage are managed on the unified Models page; chat networking remains independent of business networking."] },
] as const;

export function resolveConfigSection(value: string | null | undefined): ConfigCenterSection {
  if (!value) return "runtime-model";
  if (value === "scan-models" || value === "models") return "runtime-model";
  if (PROFILE_SETTINGS_NAV.some(item => item.route === value)) return "profile-settings";
  return CONFIG_SECTION_ALIASES[value] ?? "runtime-model";
}

export function isConfigCenterSection(value: string): value is ConfigCenterSection {
  return (CONFIG_CENTER_SECTIONS as readonly string[]).includes(value);
}

/** Advanced subsection when landing via a legacy alias. */
export type AdvancedSubSection = "profiles" | "rules" | "workers" | "runtime" | "legacy-models";

export function advancedSubSectionFromAlias(value: string | null | undefined): AdvancedSubSection {
  if (value === "rules") return "rules";
  if (value === "workers") return "workers";
  if (value === "runtime") return "runtime";
  if (value === "profiles") return "profiles";
  return "profiles";
}
