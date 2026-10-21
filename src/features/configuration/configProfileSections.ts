import type { AgentModelProfile } from "./configProfileDefaults";

/** Each editor section names its consumer; machine environment is managed separately. */
export type ConfigProfileSection = "models" | "assetApis" | "network" | "agent" | "collection" | "probe" | "rules" | "securityRules";
export type ConfigPolicyReference = { policyId: string; revision: number; kind: string };
export const INDEPENDENT_POLICY_SECTIONS: readonly ConfigProfileSection[] = ["models", "assetApis", "collection", "probe", "rules", "agent", "network", "securityRules"];

function secretField(key: string): boolean {
  const normalized = key.replace(/[_\-\s]/g, "").toLowerCase();
  return /(?:authorization|apikey|fofakey|accesstoken|refreshtoken|idtoken|authtoken|token|password|passwd|secret|secretkey|credential|credentials|privatekey|cookie|setcookie)$/.test(normalized);
}

function redactedUrl(value: string): string {
  // Proxy pool entries retain TAG|URL and order. URL parsing keeps IPv6 and
  // percent-encoded credentials out of preview without string-splitting '@'.
  const divider = value.indexOf("|");
  const prefix = divider >= 0 ? value.slice(0, divider + 1) : "";
  const raw = divider >= 0 ? value.slice(divider + 1) : value;
  if (!raw.includes("://")) return value;
  try {
    const url = new URL(raw.trim());
    let redacted = false;
    if (url.username || url.password) { url.username = "configured"; url.password = ""; redacted = true; }
    for (const key of [...url.searchParams.keys()]) if (secretField(key)) { url.searchParams.set(key, "[configured]"); redacted = true; }
    return redacted ? prefix + url.toString() : value;
  } catch { return raw.includes("@") ? prefix + "[invalid URL hidden]" : value; }
}

/** Presentation only: the exact payload stays private for the explicit write. */
export function redactedConfigPreview(value: unknown): unknown {
  if (typeof value === "string") return redactedUrl(value);
  if (Array.isArray(value)) return value.map(redactedConfigPreview);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([key, field]) => [key,
    secretField(key) ? (field ? "[configured]" : "") : redactedConfigPreview(field),
  ]));
  return value;
}

export function configPolicyReference(settings: Record<string, any>, section: ConfigProfileSection): ConfigPolicyReference | undefined {
  return settings._configurationRefs?.[section];
}

export const CONFIG_PROFILE_SECTIONS: ReadonlyArray<{ key: ConfigProfileSection; label: readonly [string, string] }> = [
  { key: "assetApis", label: ["资产与平台 API", "Asset & platform APIs"] },
  { key: "collection", label: ["资产采集", "Asset collection"] },
  { key: "probe", label: ["资产探测", "Asset probing"] },
  { key: "rules", label: ["资产内容规则", "Asset content rules"] },
  { key: "models", label: ["模型连接", "Model connections"] },
  { key: "agent", label: ["Nest 扫描策略", "Nest scan policy"] },
  { key: "network", label: ["业务网络与代理", "Business network & proxies"] },
  { key: "securityRules", label: ["代码分析与规则包", "Code analysis & rule packs"] },
];

/** Patch ownership for existing profiles. Missing snapshot fields stay missing. */
export const sectionSettingKeys: Record<ConfigProfileSection, readonly string[]> = {
  models: ["modelProfiles", "activeModelProfileId", "modelName", "modelDeployment", "modelApiBase", "modelApiKey", "localApiKey"],
  assetApis: ["fofaEmail", "fofaKey", "configPath"],
  network: ["proxyUrl", "noProxy", "agentProxyEnabled", "authorizedProxyPool"],
  agent: ["agentLocalFullPower", "agentPromptAuditMode", "agentFrontendPacketMode", "agentFrontendPacketBudgetKb", "agentQuickScore", "agentStandardScore", "agentDeepScore", "agentQuickTimeout", "agentStandardTimeout", "agentDeepTimeout", "agentQuickTokenLimit", "agentStandardTokenLimit", "agentDeepTokenLimit", "agentQuickRequestLimit", "agentStandardRequestLimit", "agentDeepRequestLimit", "agentNoToolTurnLimit"],
  collection: ["fofaProfile", "pageSize", "maxPages", "interval", "collectionTimeout", "fullHistory", "enableCidr24", "collectionCache", "precisionFirst"],
  probe: ["runRefine", "runProbe", "includeOther", "includeWeak", "priorityRate", "otherRate", "workers", "probeTimeout", "probeTotalTimeout", "probeRetries", "maxBodyBytes", "allowPrivate", "strictTls", "schemeFallback", "perHostInterval"],
  rules: ["gamblingKeywords", "pornKeywords", "negativeKeywords", "contentThreshold", "replaceDefaultContentRules"],
  securityRules: ["nativeAnalyzers"],
};

export function pickSectionSettings(settings: Record<string, unknown>, section: ConfigProfileSection): Record<string, unknown> {
  return Object.fromEntries(sectionSettingKeys[section].filter(key => Object.prototype.hasOwnProperty.call(settings, key)).map(key => [key, settings[key]]));
}

/** Legacy model metadata is not a preview schema: show supported fields only. */
export function configPolicyPreviewSettings(settings: Record<string, unknown>, section: ConfigProfileSection): Record<string, unknown> {
  const preview = pickSectionSettings(settings, section);
  if (section === "models" && Array.isArray(preview.modelProfiles)) {
    const fields = ["id", "name", "llm", "deployment", "apiBase", "apiKey", "localApiKey"];
    preview.modelProfiles = preview.modelProfiles.map(value => value && typeof value === "object"
      ? Object.fromEntries(fields.filter(key => Object.prototype.hasOwnProperty.call(value, key)).map(key => [key, (value as Record<string, unknown>)[key]])) : {});
  }
  if (section === "securityRules" && preview.nativeAnalyzers && typeof preview.nativeAnalyzers === "object") {
    const analyzers = preview.nativeAnalyzers as Record<string, { image?: unknown }>;
    preview.nativeAnalyzers = Object.fromEntries(["semgrep", "codeql"].map(engine => [engine, { image: String(analyzers[engine]?.image ?? "") }]));
  }
  return preview;
}

/** Settings are JSON DTOs; serialization also unwraps nested Vue proxies. */
export function cloneConfigProfileSettings<T extends Record<string, unknown>>(settings: T): T {
  return JSON.parse(JSON.stringify(settings));
}

export function resolveConfigProfileSection(section?: string): ConfigProfileSection {
  return CONFIG_PROFILE_SECTIONS.find(item => item.key === section)?.key ?? "assetApis";
}

/** Local names and unknown metadata do not change the model connection. */
export function modelConnectionSignature(profile: AgentModelProfile): string {
  return JSON.stringify([profile.id, profile.llm, profile.apiBase, profile.apiKey, profile.localApiKey, profile.deployment]);
}

export function modelProfilesRequiringTest(initial: readonly AgentModelProfile[], current: readonly AgentModelProfile[]): AgentModelProfile[] {
  const originals = new Map(initial.map(profile => [profile.id, modelConnectionSignature(profile)]));
  return current.filter(profile => originals.get(profile.id) !== modelConnectionSignature(profile));
}

export type RulePackDraft = { repository: string; reference: string; enabled: boolean };
export function rulePackSignature(draft: RulePackDraft): string {
  return JSON.stringify([draft.repository.trim(), draft.reference.trim(), draft.enabled]);
}
