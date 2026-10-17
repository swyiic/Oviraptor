/** Session model presets + provider resolution (mirrors agent-host normalizeProviderModel). */
export const DEEPSEEK_BASE_URL = "https://api.deepseek.com";
export const XAI_BASE_URL = "https://api.x.ai/v1";

export type SessionProviderId = "xai" | "openai-compatible";
export type SessionModelPreset = { id: string; label: string; provider: SessionProviderId; baseUrl: string; model: string };

export const SESSION_MODEL_PRESETS: readonly SessionModelPreset[] = Object.freeze([
  { id: "deepseek-v4-flash", label: "DeepSeek V4 Flash", provider: "openai-compatible", baseUrl: DEEPSEEK_BASE_URL, model: "deepseek-v4-flash" },
  { id: "deepseek-chat", label: "DeepSeek Chat", provider: "openai-compatible", baseUrl: DEEPSEEK_BASE_URL, model: "deepseek-chat" },
  { id: "grok-4.7", label: "xAI Grok 4.7", provider: "xai", baseUrl: XAI_BASE_URL, model: "grok-4.7" },
]);

/** A DeepSeek model always goes to the DeepSeek openai-compatible endpoint; Grok to xAI. */
export function resolveProviderForModel(raw: string, provider?: SessionProviderId, baseUrl?: string): { provider: SessionProviderId; baseUrl?: string; model: string } {
  const model = raw.trim();
  if (/deepseek/i.test(model)) {
    const bare = model.replace(/^(openai|deepseek)\//i, "");
    if (provider === "openai-compatible" && baseUrl && !/api\.x\.ai/i.test(baseUrl)) return { provider, baseUrl, model: bare };
    return { provider: "openai-compatible", baseUrl: DEEPSEEK_BASE_URL, model: bare };
  }
  if (/^(xai\/)?grok/i.test(model)) return { provider: "xai", baseUrl: XAI_BASE_URL, model: model.replace(/^xai\//i, "") };
  return { provider: provider ?? "xai", baseUrl, model };
}

export function providerDisplayName(provider?: string, baseUrl?: string): string {
  if (/deepseek/i.test(baseUrl || "")) return "DeepSeek";
  if (provider === "openai-compatible") return "OpenAI 兼容";
  return "xAI Grok";
}
