import { invoke } from "@tauri-apps/api/core";
import type {
  AppSecScanResult,
  BrowserAuthSession,
  InvestigationGraph,
  InvestigationHypothesis,
  InvestigationOverview,
  FofaApiTestResult,
  SecurityRulePack,
  SentinelCheckpoint,
  SentinelFinding,
  SentinelFuseEntry,
  SentinelOpportunity,
  SentinelOverviewStats,
  SentinelScan,
  SentinelScanAttempt,
  SentinelTarget,
  SentinelValidation,
  SentinelValidationWorkItem,
  StrixKnowledgeEntry,
  StrixLearningCandidate,
  StrixLlmTestResult,
  StrixSkill,
  StrixTraceDetail,
  StrixTraceSummary,
  StrixWorkbenchInput,
} from "../../types";

export const sentinelApi = {
  createSentinelScan: (projectId: number, assetIds: number[]) =>
    invoke<SentinelScan>("create_sentinel_scan", { projectId, assetIds }),
  createSentinelUrlScan: (
    projectId: number,
    taskName: string,
    urls: string[],
    scanMode: "quick" | "standard" | "deep" = "standard",
    maxBudgetUsd?: number,
    authSessionId?: string,
    authSessionIds?: string[],
  ) => invoke<SentinelScan>("create_sentinel_url_scan", {
    projectId, taskName, urls, scanMode, maxBudgetUsd, authSessionId, authSessionIds,
  }),
  listBrowserAuthSessions: (projectId: number) =>
    invoke<BrowserAuthSession[]>("list_browser_auth_sessions", { projectId }),
  listSentinelScanAuthSessions: (scanId: string) =>
    invoke<BrowserAuthSession[]>("list_sentinel_scan_auth_sessions", { scanId }),
  openBrowserAuthSession: (input: { id?: string; projectId: number; name: string; entryUrl: string }) =>
    invoke<BrowserAuthSession>("open_browser_auth_session", { input }),
  finishBrowserAuthSession: (sessionId: string) =>
    invoke<BrowserAuthSession>("finish_browser_auth_session", { sessionId }),
  validateBrowserAuthSession: (sessionId: string) =>
    invoke<BrowserAuthSession>("validate_browser_auth_session", { sessionId }),
  deleteBrowserAuthSession: (sessionId: string) =>
    invoke<void>("delete_browser_auth_session", { sessionId }),
  testStrixLlm: (input: {
    llm: string; deployment: "cloud" | "local"; apiBase: string; apiKey: string;
  }) => invoke<StrixLlmTestResult>("test_strix_llm", { input }),
  testFofaApi: (input: { key: string; proxyUrl: string }) =>
    invoke<FofaApiTestResult>("test_fofa_api", { input }),
  listStrixSkills: () => invoke<StrixSkill[]>("list_strix_skills"),
  saveStrixSkill: (input: {
    id?: number; name: string; description: string; instructions: string; enabled: boolean;
  }) => invoke<number>("save_strix_skill", { input }),
  deleteStrixSkill: (skillId: number) => invoke<void>("delete_strix_skill", { skillId }),
  exportStrixSkills: () => invoke<string>("export_strix_skills"),
  importStrixSkills: (path: string) => invoke<number>("import_strix_skills", { path }),
  importSecSkillKnowledge: (path: string) =>
    invoke<Record<string, unknown>>("import_sec_skill_knowledge", { path }),
  ingestStrixKnowledgeSource: (source: string, forceRefresh = false) =>
    invoke<StrixKnowledgeEntry>("ingest_strix_knowledge_source", { source, forceRefresh }),
  listStrixTraces: () => invoke<StrixTraceSummary[]>("list_strix_traces"),
  getStrixTrace: (scanId: string) => invoke<StrixTraceDetail>("get_strix_trace", { scanId }),
  listStrixKnowledge: () => invoke<StrixKnowledgeEntry[]>("list_strix_knowledge"),
  listStrixLearningCandidates: (status?: string) =>
    invoke<StrixLearningCandidate[]>("list_strix_learning_candidates", { status }),
  generateStrixLearningCandidate: (scanId: string) =>
    invoke<StrixLearningCandidate>("generate_strix_learning_candidate", { scanId }),
  reviewStrixLearningCandidate: (candidateId: number, decision: string, targetSkillId?: number) =>
    invoke<StrixLearningCandidate>("review_strix_learning_candidate", { candidateId, decision, targetSkillId }),
  applyStrixLearningCandidate: (candidateId: number) =>
    invoke<number>("apply_strix_learning_candidate", { candidateId }),
  deleteStrixLearningCandidate: (candidateId: number) =>
    invoke<void>("delete_strix_learning_candidate", { candidateId }),
  analyzeStrixTrace: (scanId: string) =>
    invoke<StrixKnowledgeEntry>("analyze_strix_trace", { scanId }),
  aggregateStrixKnowledge: (scanType: string) =>
    invoke<StrixKnowledgeEntry>("aggregate_strix_knowledge", { scanType }),
  deleteStrixKnowledge: (knowledgeId: number) =>
    invoke<void>("delete_strix_knowledge", { knowledgeId }),
  convertStrixKnowledgeToSkill: (knowledgeId: number) =>
    invoke<number>("convert_strix_knowledge_to_skill", { knowledgeId }),
  refineStrixSkillWithKnowledge: (skillId: number) =>
    invoke<number>("refine_strix_skill_with_knowledge", { skillId }),
  exportStrixKnowledge: () => invoke<string>("export_strix_knowledge"),
  importStrixKnowledge: (path: string) => invoke<number>("import_strix_knowledge", { path }),
  listSecurityRulePacks: () => invoke<SecurityRulePack[]>("list_security_rule_packs"),
  saveSecurityRulePack: (input: {
    key: string; name: string; engine: string; repository: string; reference?: string; enabled: boolean;
  }) => invoke<number>("save_security_rule_pack", { input }),
  deleteSecurityRulePack: (packId: number) =>
    invoke<void>("delete_security_rule_pack", { packId }),
  syncSecurityRulePack: (packId: number) =>
    invoke<SecurityRulePack>("sync_security_rule_pack", { packId }),
  startStrixWorkbenchScan: (input: StrixWorkbenchInput) =>
    invoke<SentinelScan>("start_strix_workbench_scan", { input }),
  rescanStrixWorkbenchScan: (scanId: string) =>
    invoke<SentinelScan>("rescan_strix_workbench_scan", { scanId }),
  rescanSentinelScan: (scanId: string) => invoke<SentinelScan>("rescan_sentinel_scan", { scanId }),
  confirmSentinelScan: (scanId: string) => invoke<SentinelScan>("confirm_sentinel_scan", { scanId }),
  pauseSentinelScan: (scanId: string) => invoke<SentinelScan>("pause_sentinel_scan", { scanId }),
  resumeSentinelScan: (scanId: string) => invoke<SentinelScan>("resume_sentinel_scan", { scanId }),
  cancelSentinelScan: (scanId: string) => invoke<void>("cancel_sentinel_scan", { scanId }),
  deleteSentinelScan: (scanId: string) => invoke<void>("delete_sentinel_scan", { scanId }),
  listSentinelScans: (projectId?: number, limit = 300) =>
    invoke<SentinelScan[]>("list_sentinel_scans", { projectId, limit }),
  listSentinelScanAttempts: (scanId: string) =>
    invoke<SentinelScanAttempt[]>("list_sentinel_scan_attempts", { scanId }),
  listSentinelVulnerabilityScanIds: (projectId?: number) =>
    invoke<string[]>("list_sentinel_vulnerability_scan_ids", { projectId }),
  getSentinelRunnerLog: (scanId: string, limit = 300) =>
    invoke<string[]>("get_sentinel_runner_log", { scanId, limit }),
  searchSentinelScanIds: (search: string) =>
    invoke<string[]>("search_sentinel_scan_ids", { search }),
  listSentinelTargets: (projectId?: number, limit = 5000) =>
    invoke<SentinelTarget[]>("list_sentinel_targets", { projectId, limit }),
  listSentinelFuseZone: (projectId?: number) =>
    invoke<SentinelFuseEntry[]>("list_sentinel_fuse_zone", { projectId }),
  saveSentinelFuseReview: (input: {
    id: number; verdict: string; note: string; evidence: string; archived: boolean;
  }) => invoke<void>("save_sentinel_fuse_review", { input }),
  removeSentinelFuseEntry: (entryId: number) =>
    invoke<SentinelScan>("remove_sentinel_fuse_entry", { entryId }),
  listSentinelCheckpoints: (scanId: string) =>
    invoke<SentinelCheckpoint[]>("list_sentinel_checkpoints", { scanId }),
  listSentinelFindings: (scanId: string, kind?: string) =>
    invoke<SentinelFinding[]>("list_sentinel_findings", { scanId, kind }),
  listSentinelOpportunities: (projectId?: number, scanId?: string, status?: string, limit = 500) =>
    invoke<SentinelOpportunity[]>("list_sentinel_opportunities", { projectId, scanId, status, limit }),
  updateSentinelOpportunityStatus: (opportunityId: number, status: string) =>
    invoke<void>("update_sentinel_opportunity_status", { opportunityId, status }),
  getInvestigationGraph: (scanId: string, targetUrl?: string) =>
    invoke<InvestigationGraph>("get_investigation_graph", { scanId, targetUrl }),
  listInvestigationHypotheses: (scanId?: string, status?: string) =>
    invoke<InvestigationHypothesis[]>("list_investigation_hypotheses", { scanId, status }),
  updateInvestigationHypothesis: (hypothesisId: number, status: string) =>
    invoke<void>("update_investigation_hypothesis", { input: { hypothesisId, status } }),
  setInvestigationMutationApproval: (
    hypothesisId: number,
    approved: boolean,
    maxAttempts = 1,
    expiresMinutes = 30,
    note = "",
  ) => invoke<void>("set_investigation_mutation_approval", {
    input: { hypothesisId, approved, maxAttempts, expiresMinutes, note },
  }),
  investigationOverview: (projectId?: number) =>
    invoke<InvestigationOverview>("investigation_overview", { projectId }),
  listAppSecScanResult: (scanId: string) =>
    invoke<AppSecScanResult>("list_appsec_scan_result", { scanId }),
  sentinelOverviewStats: (projectId?: number) =>
    invoke<SentinelOverviewStats>("sentinel_overview_stats", { projectId }),
  listSentinelValidations: (scanId: string) =>
    invoke<SentinelValidation[]>("list_sentinel_validations", { scanId }),
  listAllSentinelValidations: (projectId?: number, limit = 5000) =>
    invoke<SentinelValidation[]>("list_all_sentinel_validations", { projectId, limit }),
  listSentinelValidationWorkItems: (projectId?: number, limit = 5000) =>
    invoke<SentinelValidationWorkItem[]>("list_sentinel_validation_work_items", { projectId, limit }),
  saveSentinelValidation: (input: {
    scanId: string; url: string; findingKey: string; findingKind: string; verdict: string;
    severity: string; note: string; evidence: string;
  }) => invoke<void>("save_sentinel_validation", { input }),
  exportSentinelResults: (scanId: string) => invoke<string>("export_sentinel_results", { scanId }),
  importSentinelResults: (content: string) => invoke<number>("import_sentinel_results", { content }),
  exportSentinelProject: (projectId: number) => invoke<string>("export_sentinel_project", { projectId }),
  importSentinelProject: (path: string) => invoke<number>("import_sentinel_project", { path }),
  syncSentinelResults: () => invoke<number>("sync_sentinel_results"),
};
