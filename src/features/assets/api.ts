import { invoke } from "@tauri-apps/api/core";
import type {
  AssetEvent,
  AssetPage,
  AssetQuery,
  AssetSelection,
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
