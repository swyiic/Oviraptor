import type { AgentHostEndpoint, AgentHostSettings, AgentNetworkSettings, AgentNetworkUpdate, AgentTask, AgentAccessMode, AgentAccessPolicy } from "./types";
import { providerDisplayName, resolveProviderForModel } from "./providerPresets";

export type AgentHostMessage = { id: string; role: "user" | "assistant"; content: string; status: string; runId?: string | null; createdAt: string; failureCode?: string; stopReason?: string };
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
    network: () => request<AgentNetworkSettings>("/api/settings/network"),
    setNetwork: (input: AgentNetworkUpdate) => request<AgentHostSettings>("/api/settings/network", { method: "POST", body: JSON.stringify(input) }),
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
    deleteConversation: (conversationId: string, projectId: number) => request<unknown>(`/api/conversations/${encodeURIComponent(conversationId)}?projectId=${projectId}`, { method: "DELETE" }),
    messages: (conversationId: string, projectId: number) => request<{ items: AgentHostMessage[] }>(`/api/conversations/${encodeURIComponent(conversationId)}/messages?projectId=${projectId}`).then(r => r.items),
    send: (conversationId: string, projectId: number, text: string, taskId?: string, confirmedIntent?: string) => request<unknown>(`/api/conversations/${encodeURIComponent(conversationId)}/messages`, { method: "POST", body: JSON.stringify({ projectId, text, ...(taskId ? { taskId } : {}), ...(confirmedIntent ? { confirmedIntent } : {}) }) }),
    prepareTask: (conversationId: string, projectId: number, choice: string, scope: string) => request<{ id: string; scopeHash: string; startsScan: boolean }>(`/api/conversations/${encodeURIComponent(conversationId)}/approvals`, { method: "POST", body: JSON.stringify({ projectId, choice, scope }) }),
    confirmTask: (conversationId: string, projectId: number, approvalId: string, scopeHash: string) => request<{ id: string; status: string; startsScan: boolean }>(`/api/conversations/${encodeURIComponent(conversationId)}/approvals/${encodeURIComponent(approvalId)}/confirm`, { method: "POST", body: JSON.stringify({ projectId, scopeHash }) }),
    task: (conversationId: string, projectId: number, taskId?: string) => request<{ task: AgentTask | null }>(`/api/conversations/${encodeURIComponent(conversationId)}/task?projectId=${projectId}${taskId ? `&taskId=${encodeURIComponent(taskId)}` : ""}`).then(result => result.task),
    continueTask: (conversationId: string, projectId: number, taskId: string, text?: string) => request<unknown>(`/api/conversations/${encodeURIComponent(conversationId)}/task/continue`, { method: "POST", body: JSON.stringify({ projectId, taskId, ...(text ? { text } : {}) }) }),
    cancelChild: (conversationId: string, projectId: number, taskId: string, childTaskId: string, parentRunId: string) => request<{ task: AgentTask; childTask: AgentTask }>(`/api/conversations/${encodeURIComponent(conversationId)}/task/children/${encodeURIComponent(childTaskId)}/cancel`, { method: "POST", body: JSON.stringify({ projectId, taskId, parentRunId }) }),
    cancel: (conversationId: string) => request<unknown>("/api/runtime/cancel", { method: "POST", body: JSON.stringify({ conversationId }) }),
    status: () => request<AgentHostEvent>("/api/status"),
    decidePermission: (id: string, optionId: string) => request<unknown>(`/api/runtime/permissions/${encodeURIComponent(id)}`, { method: "POST", body: JSON.stringify({ optionId }) }),
    resolveAuthorization: (conversationId: string, accepted: boolean, scope: { requestId?: string; runId?: string; remember?: boolean } = {}) => request<unknown>("/api/runtime/authorization", { method: "POST", body: JSON.stringify({ conversationId, accepted, ...scope }) }),
    accessPolicy: (projectId:number) => request<AgentAccessPolicy>(`/api/runtime/access-policy?projectId=${projectId}`),
    setAccessPolicy: (input:{projectId:number;mode:AgentAccessMode;expectedRevision:number;expectedRuntimeIdentity:string;operatorConfirmed:boolean}) => request<AgentAccessPolicy>('/api/runtime/access-policy',{method:'POST',body:JSON.stringify(input)}),
    workspaceGrants: (conversationId: string, projectId: number) => request<{ permissions: string[] }>(`/api/runtime/workspace-grants?conversationId=${encodeURIComponent(conversationId)}&projectId=${projectId}`),
    clearWorkspaceGrants: (conversationId: string, projectId: number) => request<{ cleared: boolean }>("/api/runtime/workspace-grants", { method: "DELETE", body: JSON.stringify({ conversationId, projectId }) }),
    steer: (conversationId: string, text: string) => request<unknown>("/api/runtime/steer", { method: "POST", body: JSON.stringify({ conversationId, text }) }),
    listApps: () => request<{ items: Array<{ id: string; enabled: boolean; version?: string; manifest?: { name?: string; permissions?: Array<{ id: string; description?: string }>; tools?: Array<{ name: string; permission?: string }> }; grants?: Array<{ projectId: number; permissions: string[]; revision: number; disclosureRuntime?: string }> }> }>("/api/apps"),
    setAppEnabled: (appId: string, enabled: boolean) => request(`/api/apps/${encodeURIComponent(appId)}/state`, { method: "POST", body: JSON.stringify({ enabled }) }),
    setGrants: (appId: string, input: { projectId: number; permissions: string[]; expectedRevision?: number; disclosureAccepted: true; expectedRuntimeIdentity: string }) =>
      request(`/api/apps/${encodeURIComponent(appId)}/grants`, { method: "POST", body: JSON.stringify(input) }),
    ensureBundled: () => request<{ items: Array<{ id: string; error?: string | null }> }>("/api/apps/bundled/ensure", { method: "POST", body: JSON.stringify({ force: true }) }),

    /** SSE over fetch (EventSource cannot send the session header). Returns a stop function. */
    events(onEvent: (event: AgentHostEvent) => void): () => void {
      const abort = new AbortController();
      void (async () => {
        while (!abort.signal.aborted) { try {
          const response = await fetchImpl(`${endpoint.origin}/api/events`, { headers: { "X-Oviraptor-Session": endpoint.session }, signal: abort.signal });
          if (!response.ok) throw Error(`agent_host_http_${response.status}`);
          const reader = response.body?.getReader(); if (!reader) throw Error("agent_host_stream_unavailable");
          onEvent({ type: "connection.status", connected: true });
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
        } catch { /* Reconnect below; cancellation is handled by the abort signal. */ }
          if (abort.signal.aborted) break;
          onEvent({ type: "connection.status", connected: false });
          await new Promise<void>(resolve => {
            const done = () => { clearTimeout(timer); abort.signal.removeEventListener("abort", done); resolve(); };
            const timer = setTimeout(done, 1500);
            abort.signal.addEventListener("abort", done, { once: true });
          });
        }
      })();
      return () => abort.abort();
    },
  };
}
export type AgentHostChatClient = ReturnType<typeof createAgentHostChatClient>;
