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
  authentication?: string | null;
  oauthAvailable?: boolean;
  oauthExpiresAt?: number;
  provider?: string;
  label?: string;
  baseUrl?: string;
  model?: string;
};

export type AgentAnswerMode = "quality" | "speed";

export type AgentHostSettings = {
  runtime: AgentRuntimeInfo;
  modelProvider?: { source?: string; availableHere?: boolean };
  apiProvider?: AgentApiProviderView;
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
