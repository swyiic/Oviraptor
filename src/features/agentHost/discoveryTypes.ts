export type DiscoveryWitness = { appRevision: number; grantRevision: number; policyRevision: number; runtimeIdentity: string };
export type DiscoveryVerificationSource = { kind: string; host: string; organization: string; assetId?: number; certificateSha256?: string; sourceUrl?: string; certificateId?: string; observedAt?: string; validFrom?: string; validUntil?: string };
export type DiscoveryVerification = { status: 'bound_local_evidence'; sources: DiscoveryVerificationSource[] };
export type DiscoveryFamily = { root: string; shared: boolean; scopes: string[]; observedCount: number; reasons: string[]; evidenceGroups: number; verification?: DiscoveryVerification };
export type DiscoveryPipeline = {
  phase: string; round: number; collectionRunId?: number | null; enrichmentRunId?: string | null; allowHunter: boolean;
  counts: Record<string, number>;
  hunter: { queries: number; pages: number; spentPoints?: number | null; costUnknown: boolean };
  organizationEvidence?: { supported: boolean; cached?: boolean; networkVerified?: boolean; sourceUrl?: string; names?: string[]; error?: string; checkedAt?: number; retryAt?: number } | null;
  reason: string;
};
export type DiscoveryView = {
  runId: string | null; phase: string; enabled: boolean; organization: string; includeIsolated: boolean; reason: string; enrichmentError?: string;
  confirmationWitness: DiscoveryWitness;
  pipeline?: DiscoveryPipeline | null;
  proposal: null | { token: string; roots: string[]; groups: DiscoveryFamily[]; removedNoLiveEvidence?: number; heldForVerification?: number; removedUnverified?: number; verificationPolicyVersion?: number; networkVerified?: boolean };
  review?: { run: null | { processedAssets: number; totalAssets: number; groups: number; queueCounts: Record<string, number>; requests: number; estimatedUsage?: boolean; inputTokens: number; outputTokens: number } };
  enrichment?: { run: null | { status: string; counts: Record<string, number> } } | null;
};
