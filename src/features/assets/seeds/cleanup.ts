export interface CleanupItem {
  domain?:string; key?:string; status?:string; state?:string; reason?:string;
  evidence?:unknown; evidenceCount?:number; sources?:unknown[];
}
export interface CleanupBatch { id?:string; batchId?:string; domains?:unknown[]; count?:number; restored?:boolean; restoredAt?:string|null; }
export interface CleanupResearchError {
  kind:string; target:string; error:string; attempts:number; nextAt:number; taskNextAt?:number; state?:string;
}
export interface SeedCleanup {
  projectId:number; runId?:string; id?:string; organization?:string; phase?:string; enabled?:boolean;
  reason?:string; outcomeUnknown?:boolean; operationId?:string;
  operation?:{id:string;state:string;result?:unknown};
  snapshot?:{rawIndexSeeds?:number;projectLedgerSeeds?:number;groups?:number;processed?:number;total?:number};
  exclusions?:{activeDomainFamilies:number;restoredDomainFamilies:number;batchCount:number};
  remainingReasons?:Record<string,number>; counts?:Record<string,number>; items?:CleanupItem[]; batches?:CleanupBatch[];
  model?:{name?:string;requests?:number;unknownRequests?:number;inputTokens?:number;outputTokens?:number;pendingGroups?:number};
  research?:{website?:{completed:number;pending:number;retrying:number};completed?:number;pending?:number;pendingQueued?:number;running?:number;retrying?:number;parked?:number;exhausted?:number;skipped?:number;attemptedTargets?:number;untriedTargets?:number;due?:number;nextAt?:number|null;errors?:number;providerWaits?:{provider:string;nextAt:number;error:string}[];recentErrors?:CleanupResearchError[]};
  total?:number; page?:number; pageSize?:number; hunterRequests?:number; assetsDeleted?:number;
}
