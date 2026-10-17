import {
  AGENT_RUNTIME_IDS,
  isAgentRuntimeId,
  type AgentAnswerMode,
  type AgentHostSettings,
  type AgentOAuthStartResult,
  type AgentRuntimeId,
} from "./types";

export type SettingsFetch = typeof fetch;

export type HostSettingsClient = {
  getSettings(): Promise<AgentHostSettings>;
  setRuntime(runtime: AgentRuntimeId, options?: { answerMode?: AgentAnswerMode }): Promise<AgentHostSettings>;
  setAnswerMode(answerMode: AgentAnswerMode): Promise<AgentHostSettings>;
  setProvider(input: {
    model?: string;
    baseUrl?: string;
    apiKey?: string;
  }): Promise<AgentHostSettings>;
  startOAuth(input?: { referrer?: string; background?: boolean }): Promise<AgentOAuthStartResult>;
  pollOAuth(): Promise<AgentOAuthStartResult>;
  logoutOAuth(): Promise<AgentHostSettings>;
};

/** Build a session-authenticated client for agent-host /api/settings*. */
export function createHostSettingsClient(options: {
  origin: string;
  session: string;
  fetchImpl?: SettingsFetch;
}): HostSettingsClient {
  const origin = options.origin.replace(/\/$/, "");
  const session = options.session;
  const fetchImpl = options.fetchImpl ?? globalThis.fetch?.bind(globalThis);
  if (!origin) throw Error("agent_host_origin_missing");
  if (!session) throw Error("agent_host_session_missing");
  if (!fetchImpl) throw Error("agent_host_fetch_unavailable");

  async function request(path: string, init: RequestInit = {}): Promise<unknown> {
    if (!path.startsWith("/api/")) throw Error("agent_host_path_invalid");
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
    try {
      value = text ? JSON.parse(text) : null;
    } catch {
      throw Error("agent_host_invalid_response");
    }
    if (!response.ok) {
      const code =
        typeof (value as { error?: { code?: string } | string })?.error === "object"
          ? String((value as { error?: { code?: string } }).error?.code || "agent_host_request_failed")
          : typeof (value as { error?: string })?.error === "string"
            ? String((value as { error: string }).error)
            : "agent_host_request_failed";
      throw Error(code);
    }
    return value;
  }

  function assertSettings(value: unknown): AgentHostSettings {
    const runtimeId = (value as AgentHostSettings | null)?.runtime?.id;
    if (!isAgentRuntimeId(runtimeId)) {
      throw Error("agent_host_unsupported_runtime");
    }
    return value as AgentHostSettings;
  }

  return {
    async getSettings() {
      return assertSettings(await request("/api/settings"));
    },
    async setRuntime(runtime, options = {}) {
      if (!AGENT_RUNTIME_IDS.includes(runtime)) throw Error("agent_host_runtime_invalid");
      const body: Record<string, string> = { runtime };
      if (options.answerMode === "quality" || options.answerMode === "speed") body.answerMode = options.answerMode;
      return assertSettings(
        await request("/api/settings", {
          method: "POST",
          body: JSON.stringify(body),
        }),
      );
    },
    async setAnswerMode(answerMode) {
      if (answerMode !== "quality" && answerMode !== "speed") throw Error("agent_host_answer_mode_invalid");
      return assertSettings(
        await request("/api/settings", {
          method: "POST",
          body: JSON.stringify({ answerMode }),
        }),
      );
    },
    async setProvider(input) {
      const body: Record<string, string> = { provider: "xai" };
      if (input.model?.trim()) body.model = input.model.trim();
      if (input.baseUrl?.trim()) body.baseUrl = input.baseUrl.trim();
      if (input.apiKey?.trim()) body.apiKey = input.apiKey.trim();
      await request("/api/settings/provider", {
        method: "POST",
        body: JSON.stringify(body),
      });
      return this.getSettings();
    },
    async startOAuth(input = {}) {
      const value = await request("/api/settings/provider/oauth/start", {
        method: "POST",
        body: JSON.stringify(input),
      });
      return value as AgentOAuthStartResult;
    },
    async pollOAuth() {
      const value = await request("/api/settings/provider/oauth/poll", {
        method: "POST",
        body: "{}",
      });
      return value as AgentOAuthStartResult;
    },
    async logoutOAuth() {
      return assertSettings(
        await request("/api/settings/provider/oauth/logout", {
          method: "POST",
          body: "{}",
        }),
      );
    },
  };
}
