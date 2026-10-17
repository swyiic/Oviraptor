/** Shared defaults for runtime profile editor (extracted from ConfigDialog). */
export type AgentModelProfile = {
  id: string;
  name: string;
  llm: string;
  apiBase: string;
  apiKey: string;
  localApiKey: string;
  deployment: "cloud" | "local";
};

export function createAgentModelProfile(name = "", sequence = 1): AgentModelProfile {
  return {
    id: globalThis.crypto?.randomUUID?.() || `agent-model-${Date.now()}-${sequence}`,
    name,
    llm: "",
    apiBase: "",
    apiKey: "",
    localApiKey: "",
    deployment: "cloud",
  };
}

export const CONFIG_PROFILE_SETTING_DEFAULTS = {
  modelProfiles: [] as AgentModelProfile[],
  activeModelProfileId: "",
  modelDeployment: "cloud",
  modelApiBase: "",
  modelApiKey: "",
  localApiKey: "",
  agentLocalFullPower: false,
  agentFrontendPacketMode: "balanced",
  agentFrontendPacketBudgetKb: 24,
  agentPromptAuditMode: "off",
  windowsRuntimeDirectory: "C:\\oviraptor\\runtime",
  scriptsDirectory: "",
  configPath: "",
  fofaEmail: "",
  fofaKey: "",
  hackerOneUsername: "",
  hackerOneToken: "",
  proxyUrl: "",
  noProxy: "127.0.0.1,localhost",
  agentBatchSize: 15,
  agentQuickScore: 30,
  agentStandardScore: 55,
  agentDeepScore: 80,
  agentQuickTimeout: 240,
  agentStandardTimeout: 600,
  agentDeepTimeout: 1200,
  agentQuickTokenLimit: 200000,
  agentStandardTokenLimit: 400000,
  agentDeepTokenLimit: 800000,
  agentQuickRequestLimit: 6,
  agentStandardRequestLimit: 14,
  agentDeepRequestLimit: 24,
  agentNoToolTurnLimit: 6,
  agentProxyEnabled: false,
  authorizedProxyPool: [] as string[],
  collectionMode: "all",
  fofaProfile: "professional",
  pageSize: 500,
  maxPages: 0,
  maxDerivedDomains: 200,
  interval: 6,
  collectionTimeout: 45,
  fullHistory: false,
  enableCidr24: false,
  includeWeakFingerprints: false,
  runRefine: true,
  runProbe: true,
  includeOther: true,
  includeWeak: false,
  priorityRate: 20,
  otherRate: 10,
  workers: 96,
  probeTimeout: 6,
  probeTotalTimeout: 8,
  probeRetries: 0,
  contentThreshold: 12,
  gamblingKeywords: ["在线赌博", "博彩平台", "真人视讯", "体育投注"],
  pornKeywords: ["色情网站", "成人网站", "成人视频", "情色直播"],
  negativeKeywords: ["打击赌博", "扫黄打非", "反诈", "公安", "法院"],
  replaceDefaultContentRules: false,
  semgrepRuleRepository: "",
  semgrepRuleReference: "",
  codeqlRuleRepository: "",
  codeqlRuleReference: "",
  nativeAnalyzers: { semgrep: { image: "" }, codeql: { image: "" } },
  owaspBenchmarkRepository: "",
  owaspBenchmarkReference: "",
};
