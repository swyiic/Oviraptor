/** Agent-host runtime + xAI provider settings as returned by GET /api/settings. */
export type AgentRuntimeId = "none" | "grok-acp" | "xai-api";

export type AgentRuntimeInfo = {
  id: AgentRuntimeId;
  state?: string;
  label?: string;
  identity?: string;
  authenticationRequired?: boolean;
  authenticationHint?: string;
};

export type AgentApiProviderView = {
  configured: boolean;
  apiKeyConfigured: boolean;
  authentication?: string;
  oauthAvailable?: boolean;
  provider?: string;
  label?: string;
  baseUrl?: string;
  model?: string;
};

export type AgentHostSettings = {
  runtime: AgentRuntimeInfo;
  modelProvider?: { source?: string; availableHere?: boolean };
  apiProvider?: AgentApiProviderView;
  runtimes?: AgentRuntimeId[];
};

export type AgentHostEndpoint = {
  origin: string;
  session: string;
  source?: string;
};

export const AGENT_RUNTIME_IDS: readonly AgentRuntimeId[] = Object.freeze([
  "none",
  "grok-acp",
  "xai-api",
]);

export function isAgentRuntimeId(value: unknown): value is AgentRuntimeId {
  return typeof value === "string" && (AGENT_RUNTIME_IDS as readonly string[]).includes(value);
}
