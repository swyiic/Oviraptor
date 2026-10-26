export type ProjectSeedId = number;
export interface ProjectSeedQuery { projectId:number; search:string; kind:string; trust:string; page:number; pageSize:number }
export interface ProjectSeed {
 id:ProjectSeedId;kind:string;value:string;wildcard?:boolean;trustStatus:string;trustReason:string;
 firstSeenRound:number|null;firstSeenAt:string|null;lastSeenAt:string|null;lastSeenRound?:number|null;
 sourceTypes:string[];assetCount:number;observationCount:number;hunterEligible?:boolean;
 subjects?:{name:string;field:string;source:string;host?:string;certificateSha256?:string|null}[];
 historicalBaseline?:boolean;
 verificationCoverage?:{verifiedEvidence:number;totalEvidence:number};
 manualConclusion?:{accepted:number;rejected:number;reviewed:number;total:number;acceptanceRate:number|null};
}
export interface ProjectSeedPage {
 projectId:number;ready:boolean;state:string;reason?:string;items:ProjectSeed[];page:number;pageSize:number;total:number|null;
 summary?:{total:number;verified:number;needsEvidence:number;conflicts:number;excluded:number;accepted?:number;ready?:number;historicalBaseline:number}|null;
 exclusionBatches?:{id:string;origin:string;count:number;restored:boolean;createdAt:string}[];
 rounds?:ProjectSeedRound[];roundsTotal?:number;
}
export interface ProjectSeedRound {number:number;state:string;startedAt:string|null;completedAt:string|null;reason?:string;plannedQueries?:number|null}
export interface ProjectSeedRoundPage {projectId?:number;items:ProjectSeedRound[];total:number;page:number;pageSize:number;historyBeforeTracking?:string}
export interface ProjectSeedObservation {
 id:ProjectSeedId;sourceType:string;sourceRef?:string;round:number|null;seenAt:string|null;historicalBaseline?:boolean;
 queryKind?:string|null;queryValue?:string|null;queryId?:string|null;page?:number|null;spentPoints?:number|null;costUnknown?:boolean;
 assetCount?:number|null;assetId?:number|null;assetIds?:number[];evidence?:Record<string,unknown>|string|null;
}
export interface ProjectSeedDetail {
 projectId:number;seed:ProjectSeed;observations:ProjectSeedObservation[];observationsTotal:number;page:number;pageSize:number;
}
