import { invoke } from "@tauri-apps/api/core";
import type {
  AssetEvent,
  AssetPage,
  AssetQuery,
  AssetSelection,
  AssetOwnershipProfile,
  AssetOwnershipSummary,
  AssetOwnershipAssessmentResult,
  ExposureFinding,
  ExposureSummary,
  ExposureRun,
  ExposureSourceResult,
  ContentRuleApplyResult,
  JobRun,
  LogEntry,
  Target,
} from "../../types";

export const assetApi = {
  importTargets: (projectId: number, targetType: string, values: string[]) =>
    invoke<number>("import_targets", { input: { projectId, targetType, values } }),
  listTargets: (projectId: number) => invoke<Target[]>("list_targets", { projectId }),
  removeTarget: (targetId: number) => invoke<void>("remove_target", { targetId }),
  listAssets: (query: AssetQuery) => invoke<AssetPage>("list_assets", { query }),
  addContentRule: (keyword: string, sourceAssetId?: number) =>
    invoke<ContentRuleApplyResult>("add_content_rule", { input: { keyword, sourceAssetId } }),
  updateDecision: (projectId: number, assetIds: number[], decision: string, note = "") =>
    invoke<void>("update_decision", { input: { projectId, assetIds, decision, note } }),
  updateAssetDecisions: (selections: AssetSelection[], decision: string, note = "") =>
    invoke<number>("update_asset_decisions", { input: { selections, decision, note } }),
  getAssetOwnershipProfile: (projectId: number) =>
    invoke<AssetOwnershipProfile>("get_asset_ownership_profile", { projectId }),
  saveAssetOwnershipProfile: (input: AssetOwnershipProfile) =>
    invoke<AssetOwnershipProfile>("save_asset_ownership_profile", { input }),
  getAssetOwnershipSummary: (projectId: number) =>
    invoke<AssetOwnershipSummary>("get_asset_ownership_summary", { projectId }),
  assessAssetOwnership: (projectId: number) =>
    invoke<AssetOwnershipAssessmentResult>("assess_asset_ownership", { projectId }),
  updateAssetOwnership: (selections: AssetSelection[], status: string, note = "", learnDomainRule = false) =>
    invoke<number>("update_asset_ownership", { input: { selections, status, note, learnDomainRule } }),
  exposureSummary: (projectId: number) => invoke<ExposureSummary>("exposure_summary", { projectId }),
  listExposureFindings: (projectId: number, status = "", search = "") => invoke<ExposureFinding[]>("list_exposure_findings", { projectId, status, search }),
  saveExposureFinding: (input: { projectId:number; assetId?:number; category:string; title:string; sourceType:string; sourceUrl:string; evidenceExcerpt:string; severity:string; confidence:number }) => invoke<number>("save_exposure_finding", { input }),
  reviewExposureFinding: (id:number, status:string, note="") => invoke<void>("review_exposure_finding", { input:{ id,status,note } }),
  startExposureScan: (projectId:number) => invoke<number>("start_exposure_scan", { projectId }),
  listExposureRuns: (projectId:number) => invoke<ExposureRun[]>("list_exposure_runs", { projectId }),
  listExposureSourceResults: (runId:number) => invoke<ExposureSourceResult[]>("list_exposure_source_results", { runId }),
  cancelExposureScan: (runId:number) => invoke<void>("cancel_exposure_scan", { runId }),
  exportExposureFindings: (projectId:number) => invoke<{path:string;rows:number}>("export_exposure_findings", { projectId }),
  softDeleteAssets: (projectId: number, assetIds: number[], deleted: boolean) =>
    invoke<void>("soft_delete_assets", { projectId, assetIds, deleted }),
  softDeleteAssetSelections: (selections: AssetSelection[], deleted: boolean) =>
    invoke<number>("soft_delete_asset_selections", { input: { selections, deleted } }),
  listRuns: (projectId?: number, limit = 100) =>
    invoke<JobRun[]>("list_runs", { projectId, limit }),
  listLogs: (runId?: number, limit = 500, projectId?: number) =>
    invoke<LogEntry[]>("list_logs", { runId, projectId, limit }),
  listEvents: (projectId?: number, eventType?: string, limit = 500) =>
    invoke<AssetEvent[]>("list_asset_events", { projectId, eventType, limit }),
  startJob: (projectId: number, profileId: number, name: string, pipeline: string) =>
    invoke<number>("start_job", { input: { projectId, profileId, name, pipeline } }),
  resumeJob: (runId: number) => invoke<number>("resume_job", { runId }),
  cancelJob: (runId: number) => invoke<void>("cancel_job", { runId }),
  exportAssets: (query: AssetQuery, fields: string[], chineseHeaders = true) =>
    invoke<{ path: string; rows: number }>("export_assets", {
      request: { query, fields, chineseHeaders },
    }),
};
