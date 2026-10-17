import type { AgentHostEndpoint, AgentHostSettings } from "./types";
import { providerDisplayName, resolveProviderForModel } from "./providerPresets";

export type AgentHostMessage = { id: string; role: "user" | "assistant"; content: string; status: string; runId?: string | null; createdAt: string; failureCode?: string };
export type AgentHostConversation = { id: string; projectId: number; title: string; createdAt: string };
export type AgentHostEvent = { type: string; conversationId?: string; runId?: string; messageId?: string; delta?: string; status?: string; content?: string; title?: string; code?: string; [key: string]: unknown };

/** Header label for the session page: runtime + model from the live agent-host, never the legacy desktop profile. */
export function agentRuntimeHeader(settings: AgentHostSettings | null | undefined): string {
  if (!settings || settings.runtime?.id !== "xai-api") return "";
  const provider = settings.apiProvider;
  const name = providerDisplayName(provider?.provider, provider?.baseUrl);
  const label = name === "xAI Grok" ? (provider?.oauthConfigured ? "xAI Grok (OAuth)" : "xAI Grok (API key)") : name;
  const model = provider?.model;
  return model ? `${label} · ${model}` : label;
}

/** True when the desktop session page should route to the agent-host runtime. */
export function usesAgentHostRuntime(settings: AgentHostSettings | null | undefined): boolean {
  return settings?.runtime?.id === "xai-api";
}

export function createAgentHostChatClient(endpoint: AgentHostEndpoint, fetchImpl: typeof fetch = globalThis.fetch.bind(globalThis)) {
  const headers = { "X-Oviraptor-Session": endpoint.session, "Content-Type": "application/json" };
  async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
    const response = await fetchImpl(`${endpoint.origin}${path}`, { ...init, headers: { ...headers, ...(init.headers as Record<string, string> | undefined) } });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) {
      const raw = (body as { error?: string | { code?: string } })?.error;
      throw Error((typeof raw === "object" ? raw?.code : raw) || `agent_host_http_${response.status}`);
    }
    return body as T;
  }
  return {
    settings: () => request<AgentHostSettings>("/api/settings"),
    /** Switch the session model; provider/baseUrl follow the model (DeepSeek → openai-compatible). */
    async setModel(model: string, apiKey?: string): Promise<AgentHostSettings> {
      const resolved = resolveProviderForModel(model);
      await request("/api/settings/provider", { method: "POST", body: JSON.stringify({ ...resolved, ...(apiKey?.trim() ? { apiKey: apiKey.trim() } : {}) }) });
      return request<AgentHostSettings>("/api/settings");
    },
    /** Map a desktop workspace to an agent-host project by stable externalId (desktop project.id). */
    async ensureProject(name: string, externalId?: string | number): Promise<number> {
      if (externalId != null && String(externalId)) {
        const project = await request<{ id: number }>("/api/projects", {
          method: "POST",
          body: JSON.stringify({ name, ensureByExternalId: String(externalId) }),
        });
        return project.id;
      }
      const { items } = await request<{ items: { id: number; name: string }[] }>("/api/projects");
      const found = items.find(item => item.name === name);
      if (found) return found.id;
      return (await request<{ id: number }>("/api/projects", { method: "POST", body: JSON.stringify({ name }) })).id;
    },
    conversations: (projectId: number) => request<{ items: AgentHostConversation[] }>(`/api/conversations?projectId=${projectId}`).then(r => r.items),
    createConversation: (projectId: number, title?: string) => request<AgentHostConversation>("/api/conversations", { method: "POST", body: JSON.stringify({ projectId, title }) }),
    messages: (conversationId: string, projectId: number) => request<{ items: AgentHostMessage[] }>(`/api/conversations/${encodeURIComponent(conversationId)}/messages?projectId=${projectId}`).then(r => r.items),
    send: (conversationId: string, projectId: number, text: string) => request<unknown>(`/api/conversations/${encodeURIComponent(conversationId)}/messages`, { method: "POST", body: JSON.stringify({ projectId, text }) }),
    cancel: (conversationId: string) => request<unknown>("/api/runtime/cancel", { method: "POST", body: JSON.stringify({ conversationId }) }),
    listApps: () => request<{ items: Array<{ id: string; enabled: boolean; version?: string; manifest?: { name?: string; permissions?: Array<{ id: string; description?: string }>; tools?: Array<{ name: string; permission?: string }> }; grants?: Array<{ projectId: number; permissions: string[]; revision: number; disclosureRuntime?: string }> }> }>("/api/apps"),
    setAppEnabled: (appId: string, enabled: boolean) => request(`/api/apps/${encodeURIComponent(appId)}/state`, { method: "POST", body: JSON.stringify({ enabled }) }),
    setGrants: (appId: string, input: { projectId: number; permissions: string[]; expectedRevision?: number; disclosureAccepted: true; expectedRuntimeIdentity: string }) =>
      request(`/api/apps/${encodeURIComponent(appId)}/grants`, { method: "POST", body: JSON.stringify(input) }),
    ensureBundled: () => request("/api/apps/bundled/ensure", { method: "POST", body: JSON.stringify({ force: true }) }),

    /** SSE over fetch (EventSource cannot send the session header). Returns a stop function. */
    events(onEvent: (event: AgentHostEvent) => void): () => void {
      const abort = new AbortController();
      void (async () => {
        try {
          const response = await fetchImpl(`${endpoint.origin}/api/events`, { headers: { "X-Oviraptor-Session": endpoint.session }, signal: abort.signal });
          const reader = response.body?.getReader(); if (!reader) return;
          const decoder = new TextDecoder(); let buffer = "";
          for (;;) {
            const { value, done } = await reader.read(); if (done) break;
            buffer += decoder.decode(value, { stream: true });
            let index;
            while ((index = buffer.indexOf("\n\n")) >= 0) {
              const chunk = buffer.slice(0, index); buffer = buffer.slice(index + 2);
              const line = chunk.split("\n").find(item => item.startsWith("data: "));
              if (line) { try { onEvent(JSON.parse(line.slice(6))); } catch { /* ignore malformed */ } }
            }
          }
        } catch { /* aborted or host gone */ }
      })();
      return () => abort.abort();
    },
  };
}
export type AgentHostChatClient = ReturnType<typeof createAgentHostChatClient>;
