export type ViewKey =
  | "dashboard"
  | "projects"
  | "query"
  | "assets"
  | "quarantine"
  | "hackerone"
  | "sentinel"
  | "changes"
  | "tasks"
  | "logs"
  | "settings";

export interface Project {
  id: number;
  name: string;
  description: string;
  status: string;
  assetCount: number;
  pendingCount: number;
  targetCount: number;
  assetRunCount: number;
  scanCount: number;
  vulnerabilityCount: number;
  validationCount: number;
  activeFuseCount: number;
  lastRunAt?: string;
  lastScanAt?: string;
  createdAt: string;
  updatedAt: string;
}
export interface ProjectImpact {
  assetCount: number;
  assetEventCount: number;
  targetCount: number;
  assetRunCount: number;
  savedViewCount: number;
  sentinelScanCount: number;
  sentinelTargetCount: number;
  findingCount: number;
  validationCount: number;
  opportunityCount: number;
  fuseCount: number;
  appsecVulnerabilityCount: number;
  knowledgeCount: number;
  learningCandidateCount: number;
  browserAuthSessionCount: number;
  totalRecords: number;
}

export interface BrowserAuthSession {
  id: string;
  projectId: number;
  name: string;
  entryUrl: string;
  finalUrl: string;
  status: "capturing" | "valid" | "needs_check" | "invalid" | "expired" | string;
  scopeHosts: string[];
  cookieCount: number;
  headerCount: number;
  storageCount: number;
  capturedRequestCount: number;
  lastValidatedAt: string;
  expiresAt: string;
  lastError: string;
  createdAt: string;
  updatedAt: string;
}

export interface ConfigProfile {
  id: number;
  name: string;
  description: string;
  isDefault: boolean;
  settings: Record<string, any>;
  createdAt: string;
  updatedAt: string;
}

export interface Target {
  id: number;
  projectId: number;
  targetType: string;
  value: string;
  enabled: boolean;
  createdAt: string;
}

export interface DashboardStats {
  projectCount: number;
  assetCount: number;
  aliveCount: number;
  pendingCount: number;
  newCount: number;
  changedCount: number;
  blockedCount: number;
  runningJobs: number;
}

export interface Asset {
  id: number;
  projectId: number;
  assetKey: string;
  company: string;
  host: string;
  link: string;
  ip: string;
  port: string;
  protocol: string;
  domain: string;
  title: string;
  statusCode: string;
  probeOutcome: string;
  probeEntryState: string;
  reviewTier: string;
  contentCategory: string;
  score: string;
  decision: string;
  note: string;
  isDeleted: boolean;
  firstSeen: string;
  lastSeen: string;
  lastAlive?: string;
  extra: Record<string, string>;
  sentinelStatus: string;
  sentinelScanCount: number;
  sentinelSentAt?: string;
  projectFirstSeen: string;
  projectLastSeen: string;
  lastRunId?: number;
  deletedAt?: string;
  projectName: string;
}

export interface FilterCondition {
  field: string;
  operator: string;
  value: string;
  join: "and" | "or";
}

export interface AssetQuery {
  projectId?: number;
  search: string;
  conditions: FilterCondition[];
  page: number;
  pageSize: number;
  includeDeleted: boolean;
  deletedView?: "active" | "trash" | "all" | string;
  probeView?: string;
  sentinelView?: string;
  decisionView?: string;
  sortBy?: string;
  sortDirection?: "asc" | "desc" | string;
}

export interface AssetSummary {
  all: number;
  pending: number;
  uncertain: number;
  confirmed: number;
  rejected: number;
  notApplicable: number;
  sentToStrix: number;
}

export interface AssetPage {
  items: Asset[];
  total: number;
  page: number;
  pageSize: number;
  summary: AssetSummary;
}

export interface AssetSelection {
  projectId: number;
  assetId: number;
}

export interface ContentRuleApplyResult {
  ruleId: number;
  keyword: string;
  matchedAssets: number;
  matchedProjectAssets: number;
}

export interface JobRun {
  id: number;
  projectId: number;
  profileId?: number;
  projectName: string;
  name: string;
  pipeline: string;
  status: string;
  stage: string;
  progress: number;
  processed: number;
  total: number;
  outputDir: string;
  error: string;
  startedAt?: string;
  finishedAt?: string;
  createdAt: string;
}

export interface AppSettings {
  reminderDays: number;
  customIcon: boolean;
  deduplicatedAssets: number;
}

export interface StaleProject {
  projectId: number;
  projectName: string;
  daysSinceUpdate?: number;
  lastRunAt?: string;
}

export interface InterruptedJob {
  runId: number;
  projectId: number;
  projectName: string;
  profileId?: number;
  name: string;
  pipeline: string;
  createdAt: string;
}

export interface StartupStatus {
  reminderDays: number;
  staleProjects: StaleProject[];
  interruptedJobs: InterruptedJob[];
}

export interface LogEntry {
  id: number;
  runId?: number;
  level: string;
  stage: string;
  message: string;
  createdAt: string;
}

export interface AssetEvent {
  id: number;
  projectId: number;
  assetId: number;
  assetKey: string;
  company: string;
  host: string;
  eventType: string;
  summary: string;
  runId?: number;
  createdAt: string;
}

export interface JobProgressEvent {
  runId: number;
  status: string;
  stage: string;
  progress: number;
  message: string;
}

export interface ToastMessage {
  id: number;
  type: "success" | "error" | "info";
  text: string;
}

export interface HackerOneProgram {
  id: string;
  handle: string;
  name: string;
  iconUrl: string;
  policy: string;
  submissionState: string;
  programState: string;
  offersBounties: boolean;
  openScope: boolean;
  fastPayments: boolean;
  safeHarbor: boolean;
  collaboration: boolean;
  lastSyncedAt: string;
  bookmarked: boolean;
  scopeCount: number;
}
export interface HackerOneScope {
  id: string;
  assetType: string;
  assetIdentifier: string;
  eligibleForSubmission: boolean;
  eligibleForBounty: boolean;
  maxSeverity: string;
  instruction: string;
  updatedAt?: string;
}
export interface HackerOneExclusion {
  id: string;
  category: string;
  details: string;
  updatedAt?: string;
}
export interface HackerOneDetail {
  program: HackerOneProgram;
  scopes: HackerOneScope[];
  exclusions: HackerOneExclusion[];
}
export interface HackerOneEvent {
  id: number;
  programHandle: string;
  eventType: string;
  summary: string;
  createdAt: string;
}
export interface SentinelScan {
  id: string;
  projectId?: number;
  projectName: string;
  status: string;
  currentCheckpoint: string;
  taskPath: string;
  previousScanId: string;
  llmRequests: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  scanType: string;
  taskName: string;
  sourcePath: string;
  skillNames: string;
  attemptCount: number;
  createdAt: string;
  updatedAt: string;
  llmModel: string;
  llmDeployment: "cloud" | "local" | "unknown" | string;
  llmFullPower: boolean;
}
export interface SentinelScanAttempt {
  scanId: string;
  attemptNumber: number;
  status: string;
  stage: string;
  checkpoint: string;
  stopReason: string;
  workDir: string;
  llmRequests: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  startedAt: string;
  finishedAt: string;
  updatedAt: string;
}
export interface StrixLlmTestResult {
  ok: boolean;
  status: string;
  message: string;
  model: string;
  deployment: "cloud" | "local" | string;
}
export interface FofaApiTestResult {
  ok: boolean;
  status: string;
  message: string;
  account: string;
  plan: string;
}
export interface StrixSkill {
  id: number;
  name: string;
  description: string;
  instructions: string;
  builtin: boolean;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}
export interface SecurityRuleChange {
  status: "added" | "modified" | "deleted";
  path: string;
}
export interface SecurityRulePack {
  id: number;
  key: string;
  name: string;
  engine: string;
  repository: string;
  reference: string;
  localPath: string;
  previousVersion: string;
  version: string;
  enabled: boolean;
  builtin: boolean;
  status: string;
  lastSyncAt: string;
  error: string;
  addedCount: number;
  modifiedCount: number;
  deletedCount: number;
  changeSummary: SecurityRuleChange[];
  progress: number;
  progressStage: string;
  progressMessage: string;
}
export interface StrixTraceToolStat {
  name: string;
  calls: number;
  results: number;
}
export interface StrixTraceSummary {
  scanId: string;
  taskName: string;
  projectName: string;
  status: string;
  scanType: string;
  model: string;
  runCount: number;
  agentCount: number;
  messageCount: number;
  reasoningCount: number;
  toolCallCount: number;
  toolResultCount: number;
  llmRequests: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  hookedRequestCount: number;
  exactRequestCapture: boolean;
  usageEntryCount: number;
  usageAgentCount: number;
  tokenUsageEstimated: boolean;
  instructionHash: string;
  tools: StrixTraceToolStat[];
  knowledgeId?: number;
  createdAt: string;
  updatedAt: string;
}
export interface StrixTraceEvent {
  id: string;
  sessionId: string;
  callId: string;
  targetUrl: string;
  eventType: string;
  role: string;
  name: string;
  status: string;
  detail: string;
  detailSize: number;
  detailTruncated: boolean;
  createdAt: string;
}
export interface StrixTraceDetail {
  summary: StrixTraceSummary;
  events: StrixTraceEvent[];
  promptAudit?: StrixPromptAudit;
}
export interface StrixPromptAudit {
  captureMode: "metadata" | "full" | string;
  source: string;
  captureLevel: string;
  exactModelRequest: boolean;
  model: string;
  deployment: "cloud" | "local" | string;
  fullPower: boolean;
  recordedAt: string;
  instructionSha256: string;
  instructionChars: number;
  instruction?: string;
  notice: string;
}
export interface StrixKnowledgeEntry {
  id: number;
  scanId: string;
  projectId?: number;
  title: string;
  summary: string;
  patterns: Record<string, unknown>;
  sourceHash: string;
  skillId?: number;
  createdAt: string;
  updatedAt: string;
}
export interface StrixLearningCandidate {
  id: number;
  scanId: string;
  projectId?: number;
  scanType: string;
  title: string;
  summary: string;
  candidate: Record<string, any>;
  status: "pending" | "accepted" | "rejected" | "applied" | string;
  targetSkillId?: number;
  sourceHash: string;
  createdAt: string;
  reviewedAt: string;
  updatedAt: string;
}
export interface StrixWorkbenchInput {
  projectId: number;
  taskName: string;
  scanType: "web" | "code" | "greybox" | "cicd";
  urls: string[];
  sourcePath: string;
  skillIds: number[];
  instruction: string;
  scanMode: "quick" | "standard" | "deep";
  scopeMode: "auto" | "diff" | "full";
  diffBase: string;
  maxBudgetUsd?: number;
  environment: string;
  authProfileName: string;
  authType: "none" | "cookie" | "bearer" | "header";
  authHeaderName: string;
  authValue: string;
  authSessionId: string;
  authSessionIds: string[];
  ciProvider: string;
  repositoryUrl: string;
  branch: string;
  commitSha: string;
  buildId: string;
  maxCritical: number;
  maxHigh: number;
  blockRelease: boolean;
}
export interface AppSecVulnerability {
  id: number;
  projectId: number;
  fingerprint: string;
  title: string;
  vulnerabilityType: string;
  severity: string;
  status: string;
  confidence: string;
  asset: string;
  environment: string;
  url: string;
  httpMethod: string;
  parameter: string;
  file: string;
  symbol: string;
  startLine: number;
  correlationScore: number;
  correlation: Record<string, any>;
  firstSeen: string;
  lastSeen: string;
  owner: string;
}
export interface AppSecVulnerabilitySource {
  id: number;
  vulnerabilityId: number;
  scanId: string;
  findingId?: number;
  sourceType: string;
  sourceKey: string;
  engine: string;
  evidence: Record<string, any>;
  createdAt: string;
}
export interface AppSecScanContext {
  scanId: string;
  environment: string;
  authProfileName: string;
  authType: string;
  authenticated: boolean;
  ciProvider: string;
  repositoryUrl: string;
  branch: string;
  commitSha: string;
  buildId: string;
  policy: Record<string, any>;
  gateStatus: string;
  gateReason: string;
  createdAt: string;
  updatedAt: string;
}
export interface AppSecScanResult {
  vulnerabilities: AppSecVulnerability[];
  sources: AppSecVulnerabilitySource[];
  context?: AppSecScanContext;
}
export interface SentinelTarget {
  id: number;
  projectId: number;
  scanId?: string;
  company: string;
  url: string;
  status: string;
  valueScore: number;
  scanMode: string;
  routingReason: string;
  createdAt: string;
  updatedAt: string;
  scanCount: number;
}
export interface SentinelFuseEntry {
  id: number;
  projectId: number;
  assetId?: number;
  company: string;
  url: string;
  sourceScanId: string;
  reason: string;
  verdict: string;
  note: string;
  evidence: string;
  archived: boolean;
  createdAt: string;
  updatedAt: string;
}
export interface SentinelCheckpoint {
  scanId: string;
  url: string;
  stage: string;
  rawJson: string;
  updatedAt: string;
}
export interface SentinelFinding {
  id: number;
  scanId: string;
  targetUrl: string;
  stage: string;
  kind: string;
  recordKey: string;
  title: string;
  severity: string;
  recordJson: string;
  updatedAt: string;
}
export interface SentinelOpportunity {
  id: number;
  projectId?: number;
  scanId: string;
  targetUrl: string;
  opportunityKey: string;
  category: string;
  title: string;
  score: number;
  status: "queued" | "ready" | "in_progress" | "validated" | "dismissed" | "exhausted" | string;
  confidence: string;
  why: string[];
  evidence: Array<Record<string, any>>;
  recommendedAction: Record<string, any>;
  source: string;
  record: Record<string, any>;
  firstSeen: string;
  lastSeen: string;
}
export interface InvestigationNode {
  id: number;
  scanId: string;
  targetUrl: string;
  nodeKey: string;
  nodeType: "target" | "identity" | "page_state" | "action" | "api" | "parameter" | "hypothesis" | string;
  label: string;
  confidence: string;
  valueScore: number;
  status: string;
  payload: Record<string, any>;
  firstSeen: string;
  lastSeen: string;
}
export interface InvestigationEdge {
  id: number;
  scanId: string;
  targetUrl: string;
  sourceKey: string;
  relation: string;
  targetKey: string;
  confidence: string;
  evidence: Record<string, any> | any[];
  createdAt: string;
}
export interface InvestigationAction {
  id: number;
  scanId: string;
  targetUrl: string;
  actionKey: string;
  stateKey: string;
  actionType: string;
  label: string;
  outcome: string;
  valueScore: number;
  protocol: Record<string, any>;
  createdAt: string;
  updatedAt: string;
}
export interface InvestigationApiModel {
  id: number;
  scanId: string;
  targetUrl: string;
  apiKey: string;
  method: string;
  url: string;
  normalizedPath: string;
  source: string;
  confidence: string;
  authScope: string;
  parameters: string[];
  requestSchema: Record<string, any>;
  responseSchema: Record<string, any>;
  stateKeys: string[];
  actionKeys: string[];
  identityKeys: string[];
  observedCount: number;
  baselineStatus: "new" | "changed" | "unchanged" | string;
  payload: Record<string, any>;
  updatedAt: string;
}
export interface InvestigationHypothesis {
  id: number;
  projectId?: number;
  scanId: string;
  targetUrl: string;
  hypothesisKey: string;
  category: string;
  title: string;
  status: string;
  score: number;
  confidence: string;
  contract: Record<string, any>;
  evidence: any[] | Record<string, any>;
  decision: Record<string, any>;
  mutationApproval: {
    approved?: boolean;
    active?: boolean;
    scope?: Record<string, any>;
    maxAttempts?: number;
    note?: string;
    expiresAt?: string;
    updatedAt?: string;
  };
  sourceOpportunityKey: string;
  createdAt: string;
  updatedAt: string;
}
export interface InvestigationIdentityDiff {
  id: number;
  scanId: string;
  targetUrl: string;
  apiKey: string;
  leftIdentityKey: string;
  rightIdentityKey: string;
  differenceType: string;
  riskScore: number;
  status: string;
  matrix: Record<string, any>;
  createdAt: string;
}
export interface InvestigationMetrics {
  scanId: string;
  targetUrl: string;
  nodeCount: number;
  edgeCount: number;
  stateCount: number;
  actionCount: number;
  apiCount: number;
  parameterCount: number;
  hypothesisCount: number;
  addedCount: number;
  changedCount: number;
  removedCount: number;
  duplicateCount: number;
  informationGain: number;
  tokenWorthy: boolean;
  stopReason: string;
  decision: Record<string, any>;
  updatedAt: string;
}
export interface InvestigationGraph {
  scanId: string;
  targetUrl: string;
  nodes: InvestigationNode[];
  edges: InvestigationEdge[];
  actions: InvestigationAction[];
  apis: InvestigationApiModel[];
  hypotheses: InvestigationHypothesis[];
  identityDiffs: InvestigationIdentityDiff[];
  metrics?: InvestigationMetrics;
}
export interface InvestigationOverview {
  targetCount: number;
  nodeCount: number;
  edgeCount: number;
  apiCount: number;
  parameterCount: number;
  hypothesisCount: number;
  readyHypothesisCount: number;
  identityDiffCount: number;
  tokenWorthyCount: number;
  averageInformationGain: number;
  factCount: number;
  promotedStrategyCount: number;
}
export interface SentinelOverviewStats {
  taskCount: number;
  urlCount: number;
  fingerprintCount: number;
  apiCount: number;
  endpointCount: number;
  vulnerabilityCount: number;
  highRiskCount: number;
  validatedCount: number;
  pendingVulnerabilityCount: number;
  vulnerableUrlCount: number;
  activeFuseCount: number;
  opportunityCount: number;
  readyOpportunityCount: number;
}
export interface SentinelValidation {
  id: number;
  scanId: string;
  url: string;
  findingKey: string;
  findingKind: string;
  verdict: string;
  severity: string;
  note: string;
  evidence: string;
  createdAt: string;
  updatedAt: string;
}
export interface SentinelValidationWorkItem {
  findingId: number;
  scanId: string;
  projectId?: number;
  projectName: string;
  taskName: string;
  url: string;
  findingKey: string;
  findingKind: string;
  title: string;
  originalSeverity: string;
  recordJson: string;
  validationId?: number;
  verdict: string;
  confirmedSeverity: string;
  note: string;
  evidence: string;
  updatedAt: string;
}
export interface EnvironmentDependency {
  name: string;
  command: string;
  version: string;
  available: boolean;
  detail: string;
}
export interface EnvironmentReport {
  os: string;
  arch: string;
  python: string;
  node: string;
  redisCli: string;
  strixCli: string;
  dockerCli: string;
  dockerDaemon: string;
  dependencies: EnvironmentDependency[];
  checkedAt: string;
}

export interface StrixUpdateStatus {
  installed: boolean;
  executable: string;
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  checkedAt: string;
  releaseUrl: string;
  checkError: string;
}

export interface LocalWorkerSettings {
  enabled: boolean;
  port: number;
  accessToken: string;
  tailscaleIp: string;
  endpoint: string;
  running: boolean;
  status: string;
}

export interface RemoteWorkerNode {
  id: number;
  name: string;
  endpoint: string;
  accessToken: string;
  enabled: boolean;
  lastSeenAt?: string;
  lastSyncAt?: string;
  lastError: string;
  createdAt: string;
  updatedAt: string;
}

export interface WorkerHealth {
  service: string;
  version: string;
  hostname: string;
  os: string;
  arch: string;
  tailscaleIp: string;
  runningScans: number;
  completedScans: number;
  checkedAt: string;
}
