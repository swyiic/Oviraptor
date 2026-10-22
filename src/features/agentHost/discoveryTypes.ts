export type DiscoveryWitness = { appRevision: number; grantRevision: number; policyRevision: number; runtimeIdentity: string };
export type DiscoveryFamily = { root: string; shared: boolean; scopes: string[]; observedCount: number; reasons: string[]; evidenceGroups: number };
export type DiscoveryView = {
  runId: string | null; phase: string; enabled: boolean; organization: string; includeIsolated: boolean; reason: string; enrichmentError?: string;
  confirmationWitness: DiscoveryWitness;
  proposal: null | { token: string; roots: string[]; groups: DiscoveryFamily[]; removedNoLiveEvidence?: number };
  review?: { run: null | { processedAssets: number; totalAssets: number; groups: number; queueCounts: Record<string, number>; requests: number; estimatedUsage?: boolean; inputTokens: number; outputTokens: number } };
  enrichment?: { run: null | { status: string; counts: Record<string, number> } } | null;
};
