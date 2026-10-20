/** Agent-host runtime + xAI provider settings as returned by GET /api/settings. */
export type AgentRuntimeId = "none" | "grok-acp" | "xai-api";

export type AgentRuntimeInfo = {
  id: AgentRuntimeId;
  state?: string;
  label?: string;
  identity?: string;
  authenticationRequired?: boolean;
  authenticationHint?: string;
  message?: string;
};

export type AgentOAuthPending = {
  status?: string;
  userCode?: string;
  verificationUri?: string;
  verificationUriComplete?: string;
  expiresAt?: number;
  error?: string;
};

export type AgentApiProviderView = {
  configured: boolean;
  apiKeyConfigured: boolean;
  oauthConfigured?: boolean;
  oauthSaved?: boolean;
  authentication?: string | null;
  authType?: "api-key" | "oauth" | "none" | null;
  connectionRef?: { policyId: string; revision: number; kind: "models"; modelProfileId: string } | null;
  oauthAvailable?: boolean;
  oauthExpiresAt?: number;
  provider?: string;
  label?: string;
  baseUrl?: string;
  model?: string;
};

export type AgentAnswerMode = "quality" | "speed";
export type AgentNetworkMode = "system" | "direct" | "manual";
export type AgentNetworkRoute = {
  mode: AgentNetworkMode;
  target: string;
  source: string;
  reason: string;
  proxyEndpoint: string | null;
  proxyCredentialsConfigured?: boolean;
  environmentVariable?: string;
};
export type AgentNetworkSettings = {
  valid: boolean;
  mode: AgentNetworkMode;
  noProxy?: string;
  manualProxyConfigured?: boolean;
  manualProxyEndpoint?: string | null;
  manualProxyCredentialsConfigured?: boolean;
  inheritedEnvironmentConfigured?: boolean;
  systemSource?: "inherited-startup-environment";
  transport?: "injected" | "host-network-owner";
  routes: { provider?: AgentNetworkRoute; oauth?: AgentNetworkRoute };
  lastRequest?: (AgentNetworkRoute & { startedAt: string; outcome: "pending" | "response" | "error" | "cancelled"; httpStatus?: number }) | null;
  error?: string;
};
export type AgentNetworkUpdate = { mode: AgentNetworkMode; proxyUrl?: string; noProxy?: string; clearProxy?: boolean };

export type AgentTaskStep = { id: string; revision: number; title: string; status: string; dependsOn: string[]; evidenceRefs: string[]; verified: boolean; outcomeUnknown?: boolean };
export type AgentTaskEvidence = { id: string; runId: string; toolName: string; status: string; summary: string; provenance: string; outcomeUnknown?: boolean; startedAt: string; finishedAt?: string };
export type AgentTaskChild = {
  id: string; runId: string; parentRunId: string; kind: "research" | "review"; goal: string;
  status: "running" | "reported" | "interrupted" | "cancelled";
  resultRef: string; provenance: "advisory-child-report";
  report?: { provenance: "advisory-child-report"; authority: "none"; parentTaskId: string; childTaskId: string; runId: string; status: string; text: string; evidenceRefs: string[] };
};
type AgentCriterionScalar = string | number | boolean | null;
export type AgentTaskCriterion =
  | { id: string; kind: "file_exists"; artifactId: string }
  | { id: string; kind: "file_contains"; artifactId: string; contains: string }
  | { id: string; kind: "tool_json_equals"; toolName: string; path: string[]; expected: AgentCriterionScalar; evidenceId?: string }
  | { id: string; kind: "tool_read_equals"; toolName: string; arguments?: Record<string, unknown>; path: string[]; expected: AgentCriterionScalar };
export type AgentTaskAcceptance = {
  provenance: "host-finite-checks"; ok: boolean; runId: string; taskRevision: number; stateDigest: string;
  checks: Array<{ criterionId: string; ok: boolean; scope: string; evidenceId?: string; artifactId?: string; toolName?: string }>;
  evidenceRefs: string[]; checkedAt: string; limitations: string[];
};
export type AgentTask = {
  id: string; revision: number; conversationId: string; projectId: number; goal: string;
  parentTaskId?: string; parentRunId?: string;
  status: "running" | "interrupted" | "awaiting_input" | "completed";
  steps: AgentTaskStep[]; evidence: AgentTaskEvidence[];
  artifacts: Array<{ id: string; label: string; location: string; evidenceRefs: string[]; provenance: "model-reported" }>;
  summary: string; outcomeUnknown: boolean; createdAt: string; updatedAt: string;
  children?: AgentTaskChild[]; criteria?: AgentTaskCriterion[];
  acceptance?: AgentTaskAcceptance | null; acceptanceValid?: boolean;
  readOnlyAnswer?: { kind: "read_only_answer"; runId: string; userMessageId: string; assistantMessageId: string; historicalWorkPending: boolean } | null;
};

export type AgentHostSettings = {
  runtime: AgentRuntimeInfo;
  modelProvider?: { source?: string; availableHere?: boolean };
  apiProvider?: AgentApiProviderView;
  network?: AgentNetworkSettings;
  oauth?: AgentOAuthPending | null;
  runtimes?: AgentRuntimeId[];
  answerMode?: AgentAnswerMode;
  answerModes?: AgentAnswerMode[];
  answerModeOptions?: {
    temperature?: number;
    maxSteps?: number;
    maxTokens?: number;
    toolHintGuard?: boolean;
  };
};

export type AgentHostEndpoint = {
  origin: string;
  session: string;
  source?: string;
};

export type AgentOAuthStartResult = AgentHostSettings & {
  status: string;
  userCode?: string;
  verificationUri?: string;
  verificationUriComplete?: string;
  expiresIn?: number;
  expiresAt?: number;
  error?: string;
};

export const AGENT_RUNTIME_IDS: readonly AgentRuntimeId[] = Object.freeze([
  "none",
  "grok-acp",
  "xai-api",
]);

export function isAgentRuntimeId(value: unknown): value is AgentRuntimeId {
  return typeof value === "string" && (AGENT_RUNTIME_IDS as readonly string[]).includes(value);
}
