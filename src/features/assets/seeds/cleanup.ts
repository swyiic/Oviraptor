export interface CleanupItem {
  domain?:string; key?:string; status?:string; state?:string; reason?:string;
  evidence?:unknown; evidenceCount?:number; sources?:unknown[];
}
export interface CleanupBatch { id?:string; batchId?:string; domains?:unknown[]; count?:number; restored?:boolean; restoredAt?:string|null; }
export interface SeedCleanup {
  projectId:number; runId?:string; id?:string; organization?:string; phase?:string; enabled?:boolean;
  reason?:string; outcomeUnknown?:boolean; operationId?:string;
  operation?:{id:string;state:string;result?:unknown};
  snapshot?:{rawIndexSeeds?:number;projectLedgerSeeds?:number;groups?:number;processed?:number;total?:number};
  counts?:Record<string,number>; items?:CleanupItem[]; batches?:CleanupBatch[];
  model?:{name?:string;requests?:number;unknownRequests?:number;inputTokens?:number;outputTokens?:number};
  research?:{completed?:number;pending?:number;errors?:number};
  total?:number; page?:number; pageSize?:number; hunterRequests?:number; assetsDeleted?:number;
}
