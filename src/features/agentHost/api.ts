import { invoke } from "@tauri-apps/api/core";
import { createHostSettingsClient, type HostSettingsClient } from "./settingsClient";
import type { AgentHostEndpoint } from "./types";

export type { HostSettingsClient } from "./settingsClient";
export type { AgentHostEndpoint, AgentHostSettings, AgentRuntimeId } from "./types";

/**
 * Resolve agent-host origin+session via the desktop sidecar command.
 * Returns null when the sidecar is unavailable (browser / failed start).
 */
export async function resolveAgentHostEndpoint(): Promise<AgentHostEndpoint | null> {
  try {
    const endpoint = await invoke<AgentHostEndpoint | null>("agent_host_endpoint");
    if (
      endpoint &&
      typeof endpoint.origin === "string" &&
      endpoint.origin.trim() &&
      typeof endpoint.session === "string" &&
      endpoint.session.trim()
    ) {
      return {
        origin: endpoint.origin.trim().replace(/\/$/, ""),
        session: endpoint.session.trim(),
        source: endpoint.source,
      };
    }
  } catch {
    /* Sidecar missing or invoke unavailable outside Tauri. */
  }
  return null;
}

/** Open a settings client when the sidecar endpoint is live; otherwise null. */
export async function openAgentHostSettingsClient(
  fetchImpl?: typeof fetch,
): Promise<{ endpoint: AgentHostEndpoint; client: HostSettingsClient } | null> {
  const endpoint = await resolveAgentHostEndpoint();
  if (!endpoint) return null;
  return {
    endpoint,
    client: createHostSettingsClient({
      origin: endpoint.origin,
      session: endpoint.session,
      fetchImpl,
    }),
  };
}

export const agentHostSettingsApi = {
  resolveEndpoint: resolveAgentHostEndpoint,
  openClient: openAgentHostSettingsClient,
};
