// G4 — incidents, coverage, live progress and unit statistics: the pure wording layer. Nothing
// here talks to the backend. Every function turns a backend answer into lines that cannot
// overstate it: a missing block reads "未报告", a gap never reads as green or "无风险", the
// unit sum is shown next to the group-unique count, and handling actions say they were only
// recorded (the application never contacts anyone).

import type { JsonLike } from "./contracts";
import { useI18n } from "../uiPorts";

// `tr` reads the current language each time it is called, so it is only ever called inside
// functions and getters here, never while the module loads.
const { tr } = useI18n();

type Row = Record<string, unknown>;
type LabelPair = readonly [string, string];

const isRow = (value: unknown): value is Row => typeof value === "object" && value !== null && !Array.isArray(value);
const num = (value: unknown): number | null => (typeof value === "number" && Number.isFinite(value) ? value : null);
const text = (value: unknown): string => (typeof value === "string" ? value : "");

/// A label table whose values are resolved in the current language on every read. It stays a
/// plain `Record<string, string>` for its callers (indexing, `in`, `v-for` all keep working).
function localizedLabels(pairs: Record<string, LabelPair>): Record<string, string> {
  const table: Record<string, string> = {};
  for (const [key, pair] of Object.entries(pairs)) {
    Object.defineProperty(table, key, { enumerable: true, get: () => tr(...pair) });
  }
  return table;
}

/// A constant that is shown as text (`{{ … }}`, template strings): it reads as the current
/// language each time it is turned into a string.
/// Internal lookups: the label in the current language, or undefined for an unknown key.
function pick(pairs: Record<string, LabelPair>, key: string): string | undefined {
  return Object.prototype.hasOwnProperty.call(pairs, key) ? tr(...pairs[key]) : undefined;
}

export const INCIDENT_STATUS_LABELS: Record<string, string> = localizedLabels({
  candidate: ["候选（待复核）", "Candidate (needs review)"],
  reviewing: ["复核中", "Under review"],
  verified: ["已核实", "Verified"],
  dismissed: ["已排除", "Dismissed"],
  excluded_public: ["公开材料已排除", "Excluded as public material"],
  merged: ["已并入其它事件", "Merged into another incident"],
});

export const HANDLING_LABELS: Record<string, string> = localizedLabels({
  none: ["未登记处置", "No handling recorded"],
  notified: ["已登记通知（人工）", "Notification recorded (manual)"],
  takedown_requested: ["已登记下架申请（人工）", "Takedown request recorded (manual)"],
  takedown_reported: ["已登记下架完成（人工，未经复测）", "Takedown recorded as done (manual, not re-probed)"],
  remediating: ["已登记整改（人工）", "Remediation recorded (manual)"],
  retest_recorded: ["已关联复测", "Re-probe linked"],
  closed: ["已登记关闭（人工）", "Closure recorded (manual)"],
});

/// Actions a person records after doing them elsewhere. None of them sends anything.
export const HANDLING_ACTIONS: { value: string; label: string; needsChannel: boolean }[] = [
  { value: "notification_recorded", get label() { return tr("登记：已通知（填写通知了谁、通过什么渠道）", "Record: notified (say who was notified and through which channel)"); }, needsChannel: true },
  { value: "takedown_requested_recorded", get label() { return tr("登记：已提交下架申请", "Record: takedown request submitted"); }, needsChannel: false },
  { value: "takedown_confirmed_recorded", get label() { return tr("登记：对方称已下架", "Record: other party says it was taken down"); }, needsChannel: false },
  { value: "remediation_recorded", get label() { return tr("登记：已整改", "Record: remediated"); }, needsChannel: false },
  { value: "retest_linked", get label() { return tr("关联一轮复测（需要轮次号）", "Link a re-probe round (round number required)"); }, needsChannel: false },
  { value: "closed_recorded", get label() { return tr("登记关闭（需写明依据）", "Record closure (basis required)"); }, needsChannel: false },
  { value: "reopened", get label() { return tr("重新打开", "Reopen"); }, needsChannel: false },
];

/** Read at render time so it follows the current language. */
export const handlingNotice = () => tr("这里只登记人工已经做过的通知、下架或整改；应用不会联系任何人或平台，也不会自动关闭事件。", "This only records notifications, takedowns or remediation already done by hand. The app never contacts any person or platform and never closes an incident on its own.");

export const LINK_LEVEL_LABELS: Record<string, string> = localizedLabels({
  confirmed_link: ["确认关联", "Confirmed link"],
  candidate_link: ["候选关联", "Candidate link"],
  unknown: ["单位未知", "Unit unknown"],
  conflict: ["关联冲突", "Link conflict"],
});

export const BASIS_LABELS: Record<string, string> = localizedLabels({
  identical_normalized_full_text: ["规范化全文相同", "Identical normalized full text"],
  identical_full_response_body: ["完整响应体相同", "Identical full response body"],
  single_risk_lead: ["单条风险线索", "Single risk lead"],
  manual_split: ["人工拆分", "Manual split"],
});

export const EVIDENCE_GRADE_LABELS: Record<string, string> = localizedLabels({
  observed_content: ["实际读取了内容", "Content actually read"],
  imported: ["用户导入", "Imported by user"],
  claim: ["情报声称（未见样本前不等于已核实）", "Intel claim (not verified until a sample is seen)"],
  index_only: ["仅索引命中", "Index hit only"],
  snippet_or_metadata: ["摘要或元数据", "Snippet or metadata"],
});

export const COVERAGE_STATE_LABELS: Record<string, string> = localizedLabels({
  unconfigured: ["未配置", "Not configured"],
  not_implemented: ["未实施", "Not implemented"],
  pending_authorization: ["待授权", "Awaiting authorization"],
  planned: ["计划中", "Planned"],
  not_run: ["已配置·尚未执行", "Configured · not run yet"],
  executed: ["已执行", "Executed"],
  executed_no_hit: ["已执行无命中", "Executed, no hits"],
  candidate: ["存在候选", "Has candidates"],
  verified_incident: ["存在已核实事件", "Has verified incidents"],
  partial: ["部分执行", "Partially executed"],
  failed: ["失败", "Failed"],
  cancelled_interrupted: ["取消/中断", "Cancelled/interrupted"],
  not_applicable: ["不适用", "Not applicable"],
});

export const INFO_CATEGORY_LABELS: Record<string, string> = localizedLabels({
  internal_business: ["内部业务", "Internal business"],
  technical_network: ["技术与网络", "Technical and network"],
  source_code: ["源代码", "Source code"],
  credential: ["凭据", "Credentials"],
  employee_data: ["员工信息", "Employee data"],
  customer_data: ["客户信息", "Customer data"],
  storage_misexposure: ["存储误公开", "Misexposed storage"],
  sold_intel: ["出售情报", "Intel for sale"],
  unclassified: ["未分类", "Unclassified"],
});

export interface CoverageCellView {
  category: string;
  state: string;
  label: string;
  tone: "green" | "amber" | "red" | "grey";
}

/// A cell is green only when the backend said `executed_no_hit` with tone green. Anything the
/// page does not recognise is grey, never green.
export function coverageCellView(cell: unknown): CoverageCellView {
  if (!isRow(cell)) return { category: "", state: "unknown", label: tr("未报告", "Not reported"), tone: "grey" };
  const state = text(cell.state);
  const backendTone = text(cell.tone);
  const tone = state === "executed_no_hit" && backendTone === "green"
    ? "green"
    : (["amber", "red"].includes(backendTone) ? backendTone : "grey") as CoverageCellView["tone"];
  return {
    category: text(cell.infoCategory),
    state,
    label: COVERAGE_STATE_LABELS[state] ?? tr(`未知状态 ${state || "（空）"}`, `Unknown state ${state || "(empty)"}`),
    tone,
  };
}

export interface CoverageRowView {
  capability: string;
  sourceKey: string;
  /** The round this row is read from: run, status, start and finish time (P1 时间窗口). */
  window: string;
  label: string;
  state: string;
  reason: string;
  denominator: string;
  integrationPending: boolean;
  cells: CoverageCellView[];
}

const CAPABILITY_LABELS: Record<string, LabelPair> = {
  first_party_web: ["第一方网页", "First-party web pages"],
  certificate_index: ["证书透明度索引", "Certificate transparency index"],
  archive_index: ["历史归档索引", "Web archive index"],
  repository_metadata: ["代码平台仓库元数据", "Code platform repository metadata"],
  code_search: ["代码平台代码搜索", "Code platform code search"],
  explicit_document_url: ["指定公开文档 URL", "Specified public document URLs"],
  file_import: ["文件导入", "File import"],
  public_intel_import: ["公开情报导入", "Public intel import"],
  pdf_docx_ocr: ["PDF/Office/OCR", "PDF/Office/OCR"],
  netdisk_keyword_search: ["网盘/文库关键词搜索", "Cloud drive/document library keyword search"],
  underground_feed: ["地下市场/暗网", "Underground market/dark web"],
};

const COVERAGE_RUN_STATUS: Record<string, LabelPair> = {
  queued: ["排队中", "Queued"], running: ["进行中", "Running"], completed: ["已完成", "Completed"], partial: ["部分完成", "Partially completed"], failed: ["失败", "Failed"], cancelled: ["已取消", "Cancelled"], interrupted: ["已中断", "Interrupted"],
};
const runStatusLabel = (status: string): string => pick(COVERAGE_RUN_STATUS, status) ?? status;

/// One matrix row with its denominator spelled out: unit, planned, executed, uncovered.
export function coverageRows(coverage: unknown): CoverageRowView[] {
  if (!isRow(coverage) || !Array.isArray(coverage.rows)) return [];
  return coverage.rows.filter(isRow).map((row) => {
    const d = isRow(row.denominator) ? row.denominator : {};
    const unit = text(d.unit) || tr("项", "items");
    const planned = num(d.planned) ?? 0;
    const executed = num(d.executed) ?? 0;
    const uncovered = num(d.uncovered) ?? Math.max(planned - executed, 0);
    const outside = num(d.outsidePlan) ?? 0;
    const parts = [tr(`计划 ${planned} ${unit}`, `Planned ${planned} ${unit}`), tr(`已执行 ${executed}`, `Executed ${executed}`), tr(`未覆盖 ${uncovered}`, `Uncovered ${uncovered}`)];
    if ((num(d.failed) ?? 0) > 0) parts.push(tr(`失败 ${d.failed}`, `Failed ${d.failed}`));
    if ((num(d.blockedQuota) ?? 0) > 0) parts.push(tr(`配额阻塞 ${d.blockedQuota}`, `Blocked by quota ${d.blockedQuota}`));
    if ((num(d.blockedAuthorization) ?? 0) > 0) parts.push(tr(`待授权 ${d.blockedAuthorization}`, `Awaiting authorization ${d.blockedAuthorization}`));
    if (outside > 0) parts.push(tr(`另有 ${outside} 个超出本轮上限未计划`, `${outside} more beyond this round's limit, not planned`));
    const state = text(row.state);
    const w = isRow(row.window) ? row.window : null;
    return {
      capability: text(row.capability),
      sourceKey: text(row.sourceKey),
      window: w
        ? tr(
            `第 ${num(w.runId) ?? "?"} 轮（${runStatusLabel(text(w.runStatus))}）· 开始 ${text(w.createdAt) || "—"} · 结束 ${text(w.completedAt) || "未结束"}`,
            `Round ${num(w.runId) ?? "?"} (${runStatusLabel(text(w.runStatus))}) · started ${text(w.createdAt) || "—"} · finished ${text(w.completedAt) || "not finished"}`,
          )
        : tr("没有计划过这个来源的轮次", "No round has planned this source yet"),
      label: pick(CAPABILITY_LABELS, text(row.capability)) ?? text(row.capability),
      state,
      reason: text(row.reason),
      denominator: parts.join(" · "),
      integrationPending: text(row.integration) === "integration_pending",
      cells: Array.isArray(row.cells) ? row.cells.map(coverageCellView) : [],
    };
  });
}

export function coverageNotice(coverage: unknown, error: string): string {
  if (error) return tr(`覆盖矩阵不可用：${error}（不是“全部覆盖”，也不是“没有风险”）`, `Coverage matrix unavailable: ${error} (this is neither "fully covered" nor "no risk")`);
  if (!isRow(coverage)) return tr("后端未报告覆盖矩阵（integration_pending）", "The backend did not report the coverage matrix (integration_pending)");
  const axis = isRow(coverage.unitAxis) ? coverage.unitAxis : {};
  const unitMissing = text(axis.state) !== "reported";
  return tr(
    `只有全部工作项完成且无命中的格子才显示为绿色；失败、阻塞、取消和超出上限都留在分母里${unitMissing ? "；单位维度未报告" : ""}`,
    `A cell shows green only when all its work items completed with no hits; failed, blocked, cancelled and over-limit items stay in the denominator${unitMissing ? "; unit breakdown not reported" : ""}`,
  );
}

export interface ExposureRecoveryView {
  resumable: boolean;
  pendingItems: number | null;
  pendingUrls: number | null;
  requestsRemaining: number | null;
  bytesRemaining: number | null;
  blockReason: string;
  pausedReason: string;
  cursorMissingItems: number | null;
  cursorInvalidItems: number | null;
  unknownUrls: number | null;
  deferredDiscoveryCount: number | null;
  usageIsCumulative: boolean;
}

/** Recovery controls follow a current backend receipt, never the saved run status alone. */
export function readRunRecovery(live: unknown): ExposureRecoveryView | null {
  if (!isRow(live) || !isRow(live.recovery) || typeof live.recovery.resumable !== "boolean") return null;
  const value = live.recovery;
  const count = (key: string) => { const n = num(value[key]); return n !== null && n >= 0 ? n : null; };
  return {
    resumable: value.resumable === true, pendingItems: count("pendingItems"), pendingUrls: count("pendingUrls"),
    requestsRemaining: count("requestsRemaining"), bytesRemaining: count("bytesRemaining"),
    blockReason: text(value.blockReason), pausedReason: text(value.pausedReason),
    cursorMissingItems: count("cursorMissingItems"), cursorInvalidItems: count("cursorInvalidItems"),
    unknownUrls: count("unknownUrls"), deferredDiscoveryCount: count("deferredDiscoveryCount"),
    usageIsCumulative: value.usageIsCumulative === true,
  };
}

const RECOVERY_BLOCKS: Record<string, LabelPair> = {
  legacy_plan_missing: ["旧任务缺少冻结计划；历史保留，可新建一轮继续采集", "This older job has no frozen plan; history is kept, start a new round to continue collection"],
  legacy_checkpoint_missing: ["旧任务未保存 URL 断点，无法无损恢复；历史保留，可新建一轮从入口继续采集", "This older job saved no URL checkpoint, so it cannot resume losslessly; history is kept, start a new round from the entry points to continue collection"],
  frozen_settings_invalid: ["冻结配置无法验证，未继续执行", "The frozen settings could not be verified; not resumed"],
  authorization_scope_unavailable: ["当前授权范围不可用，未继续执行", "The current authorization scope is unavailable; not resumed"],
  frozen_scope_changed: ["当前范围与原计划不一致；请核对授权后新建一轮，历史保留", "The current scope differs from the original plan; check the authorization and start a new round (history is kept)"],
  live_claim: ["执行器仍持有当前任务；等待其释放后再核对恢复状态", "An executor still holds this job; wait for it to release before checking the resume state"],
  executor_active: ["执行器仍在运行，无需重复恢复", "The executor is still running; no need to resume again"],
  terminal_state: ["当前轮次已经结束", "This round has already ended"],
  request_budget_exhausted: ["本轮累计请求额度已用尽；断点保留，需新建一轮继续采集", "This round's cumulative request budget is used up; the checkpoint is kept, start a new round to continue collection"],
  byte_budget_exhausted: ["本轮累计字节额度已用尽；断点保留，需新建一轮继续采集", "This round's cumulative byte budget is used up; the checkpoint is kept, start a new round to continue collection"],
  cursor_invalid: ["已保存断点无法验证，未猜测或重放请求", "The saved checkpoint could not be verified; no request was guessed or replayed"],
  no_pending_work: ["当前轮次没有可继续的未完成工作", "This round has no unfinished work to resume"],
};
export function recoveryBlockLabel(code: string): string {
  return pick(RECOVERY_BLOCKS, code) ?? (code ? tr(`无法恢复：${code}`, `Cannot resume: ${code}`) : "");
}

/// G4-2 the live lines. A percent is shown only when the backend gave one, with its basis.
export function liveRunLines(live: unknown, error: string): string[] {
  if (error) return [tr(`实时进度不可用：${error}`, `Live progress unavailable: ${error}`)];
  if (!isRow(live)) return [];
  const lines: string[] = [];
  const progress = isRow(live.progress) ? live.progress : {};
  const percent = num(progress.percent);
  const currentSource = text(live.currentSource);
  const head = tr(
    `第 ${live.runId} 轮 · ${text(live.status)} · 阶段 ${text(live.stage) || "—"}${currentSource ? " · 当前来源 " + currentSource : ""}`,
    `Round ${live.runId} · ${text(live.status)} · stage ${text(live.stage) || "—"}${currentSource ? " · current source " + currentSource : ""}`,
  );
  lines.push(head);
  const done = num(progress.done) ?? 0;
  const total = num(progress.total) ?? 0;
  lines.push(
    percent === null
      ? tr(`工作项 ${done} / ${total}（尚无可计算的进度）`, `Work items ${done} / ${total} (no progress to calculate yet)`)
      : tr(`工作项 ${done} / ${total}（${percent}%，${text(progress.note) || "按工作项计"}）`, `Work items ${done} / ${total} (${percent}%, ${text(progress.note) || "counted by work item"})`),
  );
  if (Array.isArray(live.sources)) {
    for (const source of live.sources.filter(isRow)) {
      lines.push(tr(`· ${text(source.sourceKey)}：${num(source.completed) ?? 0} / ${num(source.total) ?? 0} 完成`, `· ${text(source.sourceKey)}: ${num(source.completed) ?? 0} / ${num(source.total) ?? 0} done`));
    }
  }
  const usage = isRow(live.usage) ? live.usage : {};
  const budget = num(usage.budgetRequests);
  const budgetBytes = num(usage.budgetBytes);
  lines.push(tr(
    `请求 ${num(usage.requests) ?? 0}${budget !== null ? " / 预算 " + budget : ""} · 字节 ${num(usage.bytes) ?? 0}${budgetBytes !== null ? " / 预算 " + budgetBytes : ""}`,
    `Requests ${num(usage.requests) ?? 0}${budget !== null ? " / budget " + budget : ""} · bytes ${num(usage.bytes) ?? 0}${budgetBytes !== null ? " / budget " + budgetBytes : ""}`,
  ));
  if (text(live.lastPersistedAt)) lines.push(tr(`最近落盘 ${text(live.lastPersistedAt)}`, `Last saved to disk ${text(live.lastPersistedAt)}`));
  if (live.cancelRequested === true) lines.push(tr("已请求暂停：当前请求退出后保存断点，已采集结果保留", "Pause requested: a checkpoint is saved once the current request exits; collected results are kept"));
  const gaps = num(live.gapsTotal) ?? 0;
  if (gaps > 0) lines.push(tr(`未闭合工作项 ${gaps} 个${live.gapsTruncated === true ? "（只列出前 20 个）" : ""}`, `${gaps} work items still open${live.gapsTruncated === true ? " (only the first 20 are listed)" : ""}`));
  const recovery = readRunRecovery(live);
  if (recovery) {
    const reported = (value: number | null) => value === null ? tr("未报告", "not reported") : value.toLocaleString();
    lines.push(tr(
      `待续采 ${reported(recovery.pendingUrls)} 个 URL / ${reported(recovery.pendingItems)} 个工作项 · 剩余请求 ${reported(recovery.requestsRemaining)} 次 · 剩余字节 ${reported(recovery.bytesRemaining)}`,
      `Left to collect: URLs ${reported(recovery.pendingUrls)} / work items ${reported(recovery.pendingItems)} · requests left ${reported(recovery.requestsRemaining)} · bytes left ${reported(recovery.bytesRemaining)}`,
    ));
    lines.push(tr(`未知在途请求 ${reported(recovery.unknownUrls)} · 待继续发现 ${reported(recovery.deferredDiscoveryCount)}`, `Unknown in-flight requests ${reported(recovery.unknownUrls)} · deferred discovery ${reported(recovery.deferredDiscoveryCount)}`));
    if (recovery.pausedReason) lines.push(recovery.pausedReason);
    if (recovery.resumable) lines.push(tr("可按冻结计划恢复：已保存 URL 不重跑，未知在途请求不自动重放，用量累计", "Can resume on the frozen plan: saved URLs are not re-run, unknown in-flight requests are not replayed automatically, and usage keeps accumulating"));
    else if (recovery.blockReason) lines.push(recoveryBlockLabel(recovery.blockReason));
    if ((recovery.cursorMissingItems ?? 0) > 0) lines.push(tr(`${recovery.cursorMissingItems} 个旧工作项没有 URL 断点；保留历史，不能宣称可无损恢复`, `${recovery.cursorMissingItems} older work items have no URL checkpoint; history is kept, but a lossless resume cannot be claimed`));
  } else if (["partial", "interrupted", "cancelled"].includes(text(live.status))) {
    lines.push(tr("后端未报告有效断点；历史记录保留，可新建一轮从入口继续采集", "The backend reported no valid checkpoint; history is kept, start a new round from the entry points to continue collection"));
  }
  return lines;
}

/// G4-1 the statistics lines: group-unique next to the per-unit sum, provinces, professional
/// subsidiaries apart, unknown unit, and "未报告" where the backend had nothing.
export function unitStatisticsLines(stats: unknown, error: string): string[] {
  if (error) return [tr(`单位统计不可用：${error}`, `Unit statistics unavailable: ${error}`)];
  if (!isRow(stats)) return [tr("后端未报告单位统计（integration_pending）", "The backend did not report unit statistics (integration_pending)")];
  const lines: string[] = [];
  const group = isRow(stats.groupUnique) ? stats.groupUnique : {};
  const open = num(group.open) ?? 0;
  const sum = num(stats.sumOfUnitInvolvement) ?? 0;
  lines.push(tr(
    `集团唯一事件 ${open} 起（未关闭）；单位涉及数之和 ${sum}${sum > open ? "（一起事件涉及多个单位时各计一次，所以大于集团数）" : ""}`,
    `Unique group incidents: ${open} (open); sum of unit involvement: ${sum}${sum > open ? " (an incident that involves several units counts once per unit, so the sum exceeds the group count)" : ""}`,
  ));
  if (Array.isArray(stats.provinces)) {
    for (const row of stats.provinces.filter(isRow)) {
      lines.push(tr(
        `· ${text(row.legalName)}：唯一事件 ${num(row.uniqueIncidents) ?? 0}（确认 ${num(row.confirmed) ?? 0} / 候选 ${num(row.candidate) ?? 0} / 冲突 ${num(row.conflict) ?? 0}）`,
        `· ${text(row.legalName)}: unique incidents ${num(row.uniqueIncidents) ?? 0} (confirmed ${num(row.confirmed) ?? 0} / candidate ${num(row.candidate) ?? 0} / conflict ${num(row.conflict) ?? 0})`,
      ));
    }
  }
  if (Array.isArray(stats.professionalSubsidiaries) && stats.professionalSubsidiaries.length) {
    const subsidiaries = stats.professionalSubsidiaries.filter(isRow).map((row) => `${text(row.legalName)} ${(num(row.confirmed) ?? 0) + (num(row.candidate) ?? 0)}`);
    lines.push(tr(`专业公司单列：${subsidiaries.join("，")}`, `Professional subsidiaries, listed separately: ${subsidiaries.join(", ")}`));
  }
  lines.push(tr(`单位未知的事件 ${num(stats.unknownUnitIncidents) ?? 0} 起`, `Incidents with unknown unit: ${num(stats.unknownUnitIncidents) ?? 0}`));
  const dedup = isRow(stats.dedup) ? stats.dedup : {};
  lines.push(tr(
    `风险线索 ${num(dedup.riskLeads) ?? 0} 条 → 去重后事件 ${num(dedup.openIncidents) ?? 0} 起，有效传播位置 ${num(dedup.activeLocations) ?? 0} 处`,
    `Risk leads ${num(dedup.riskLeads) ?? 0} → incidents after dedup ${num(dedup.openIncidents) ?? 0}, active locations ${num(dedup.activeLocations) ?? 0}`,
  ));
  const assets = isRow(stats.assets) ? stats.assets : {};
  if (text(assets.state) !== "reported") lines.push(tr("资产数：未报告", "Asset count: not reported"));
  else if (Array.isArray(assets.perUnit) && assets.perUnit.length) {
    const perUnit = assets.perUnit.filter(isRow).map((row) => `${text(isRow(row.unit) ? row.unit.legalName : "")} ${num(row.assets) ?? 0}`);
    lines.push(tr(
      `资产按单位（归属模块逐资产投影）：${perUnit.join("，")}；未分配单位 ${num(assets.unassigned) ?? 0}`,
      `Assets by unit (per-asset projection from the ownership module): ${perUnit.join(", ")}; no unit assigned ${num(assets.unassigned) ?? 0}`,
    ));
  }
  return lines;
}

export interface IncidentListItem {
  id: number;
  title: string;
  status: string;
  revision: number;
  line: string;
  reviewNeeded: boolean;
}

export function incidentLine(item: unknown): IncidentListItem | null {
  if (!isRow(item) || num(item.id) === null || num(item.revision) === null) return null;
  const units = isRow(item.units) ? item.units : {};
  const conflicts = num(units.conflict) ?? 0;
  const unitPart = units.unknown === true
    ? tr("单位未知", "Unit unknown")
    : tr(
        `单位 确认 ${num(units.confirmed) ?? 0} / 候选 ${num(units.candidate) ?? 0}${conflicts > 0 ? " / 冲突 " + conflicts : ""}`,
        `Units confirmed ${num(units.confirmed) ?? 0} / candidate ${num(units.candidate) ?? 0}${conflicts > 0 ? " / conflict " + conflicts : ""}`,
      );
  const status = text(item.status);
  return {
    id: num(item.id) as number,
    title: text(item.title),
    status,
    revision: num(item.revision) as number,
    reviewNeeded: item.reviewNeeded === true,
    line: [
      INCIDENT_STATUS_LABELS[status] ?? status,
      tr(`${num(item.activeLocations) ?? 0} 个位置`, `${num(item.activeLocations) ?? 0} locations`),
      BASIS_LABELS[text(item.basis)] ?? text(item.basis),
      EVIDENCE_GRADE_LABELS[text(item.evidenceGrade)] ?? text(item.evidenceGrade),
      unitPart,
      HANDLING_LABELS[text(item.handlingState)] ?? text(item.handlingState),
    ].filter(Boolean).join(" · "),
  };
}

export interface ActionForm {
  action: "review" | "record_action" | "link_entity" | "merge" | "split";
  /** merge: the incident this one is merged into, with the revision the page read for it. */
  targetIncidentId?: number;
  targetExpectedRevision?: number;
  /** split: the active locations that move to a new incident. */
  locationIds?: number[];
  status?: string;
  handling?: string;
  channel?: string;
  externalReference?: string;
  retestRunId?: number;
  entityId?: number;
  linkLevel?: string;
  evidenceObservationIds?: number[];
  actor: string;
  reason: string;
}

/// Builds the payload `act_on_exposure_incident` expects, or explains why it cannot.
export function buildIncidentAction(
  incident: { id: number; revision: number },
  form: ActionForm,
  operationId: string,
): { ok: true; payload: JsonLike } | { ok: false; error: string } {
  const reason = form.reason.trim();
  const actor = form.actor.trim();
  if (!/^[A-Za-z0-9_-]{8,80}$/.test(operationId)) return { ok: false, error: tr("操作编号无效", "Invalid operation ID") };
  if (!actor) return { ok: false, error: tr("请填写操作人", "Enter who is performing the action") };
  if (reason.length < 4) return { ok: false, error: tr("每个动作都要写明理由（至少 4 个字）", "Every action needs a reason (at least 4 characters)") };
  const base: Record<string, unknown> = {
    operationId,
    action: form.action,
    incidentId: incident.id,
    expectedRevision: incident.revision,
    actor,
    reason,
  };
  if (form.action === "review") {
    if (!form.status || form.status === "merged" || !INCIDENT_STATUS_LABELS[form.status]) return { ok: false, error: tr("请选择复核结论", "Choose a review decision") };
    base.status = form.status;
  } else if (form.action === "record_action") {
    const spec = HANDLING_ACTIONS.find((row) => row.value === form.handling);
    if (!spec) return { ok: false, error: tr("请选择要登记的处置", "Choose the handling to record") };
    if (spec.needsChannel && !(form.channel ?? "").trim()) return { ok: false, error: tr("登记通知要写明通知了谁、通过什么渠道（应用不会代为联系）", "A notification record must say who was notified and through which channel (the app does not contact anyone for you)") };
    if (spec.value === "retest_linked" && !Number.isInteger(form.retestRunId)) return { ok: false, error: tr("关联复测需要轮次号", "Linking a re-probe needs a round number") };
    base.handling = spec.value;
    if ((form.channel ?? "").trim()) base.channel = (form.channel ?? "").trim();
    if ((form.externalReference ?? "").trim()) base.externalReference = (form.externalReference ?? "").trim();
    if (spec.value === "retest_linked") base.retestRunId = form.retestRunId;
  } else if (form.action === "merge") {
    if (!Number.isInteger(form.targetIncidentId) || (form.targetIncidentId as number) <= 0) return { ok: false, error: tr("请填写要并入的目标事件编号", "Enter the ID of the incident to merge into") };
    if (form.targetIncidentId === incident.id) return { ok: false, error: tr("不能把事件合并到它自己", "An incident cannot be merged into itself") };
    if (!Number.isInteger(form.targetExpectedRevision)) return { ok: false, error: tr("目标事件的版本未读取，请先读取目标事件", "The target incident's revision has not been read; load the target incident first") };
    base.targetIncidentId = form.targetIncidentId;
    base.targetExpectedRevision = form.targetExpectedRevision;
  } else if (form.action === "split") {
    const ids = [...new Set(form.locationIds ?? [])];
    if (!ids.length || ids.length > 500 || !ids.every((id) => Number.isInteger(id) && id > 0)) return { ok: false, error: tr("请勾选要拆出的位置（1~500 个）", "Select the locations to split off (1–500)") };
    base.locationIds = ids;
  } else {
    if (!Number.isInteger(form.entityId) || !form.linkLevel || !LINK_LEVEL_LABELS[form.linkLevel]) return { ok: false, error: tr("请选择组织目录实体与关联级别", "Choose an organization directory entity and a link level") };
    if (form.linkLevel === "confirmed_link" && !(form.evidenceObservationIds ?? []).length) return { ok: false, error: tr("确认关联需要本事件里的观察作为证据", "A confirmed link needs observations from this incident as evidence") };
    base.entityId = form.entityId;
    base.linkLevel = form.linkLevel;
    base.evidenceObservationIds = form.evidenceObservationIds ?? [];
  }
  return { ok: true, payload: base as JsonLike };
}

/// A fresh operation id per submit; a retried submit reuses the id it already sent.
export function newOperationId(random: () => string = () => globalThis.crypto.randomUUID()): string {
  return `ui-${random().replace(/[^A-Za-z0-9-]/g, "").slice(0, 60)}`;
}

// ───────────── P1 覆盖矩阵钻取：时间窗口 × 单位 × 工作项明细 ─────────────

export const COVERAGE_WINDOWS: { days: number; label: string }[] = [
  { days: 0, get label() { return tr("最近一轮", "Latest round"); } },
  { days: 7, get label() { return tr("最近 7 天", "Last 7 days"); } },
  { days: 30, get label() { return tr("最近 30 天", "Last 30 days"); } },
  { days: 90, get label() { return tr("最近 90 天", "Last 90 days"); } },
];

const WORK_ITEM_STATUS: Record<string, LabelPair> = {
  completed: ["已完成", "Completed"], failed: ["失败", "Failed"], orphaned: ["失联", "Orphaned"], cancelled: ["已取消", "Cancelled"], skipped_unconfigured: ["未配置跳过", "Skipped (not configured)"],
  blocked_quota: ["配额阻塞", "Blocked by quota"], blocked_authorization: ["授权阻塞", "Blocked by authorization"], pending: ["待执行", "Pending"], claimed: ["执行中", "Running"],
};
const workItemStatusLabel = (status: string): string => pick(WORK_ITEM_STATUS, status) ?? status;

export interface CoverageDrillView {
  windowLabel: string;
  rounds: string[];
  unitState: "reported" | "not_reported" | "not_asset_scoped" | "unknown";
  unitNote: string;
  units: { entityId: number; line: string; gaps: number }[];
  items: { id: number; line: string; gap: boolean }[];
  total: number;
  hasMore: boolean;
  nextCursor: string;
}

/// Reads one drill-down page. Anything malformed is an error, never an empty (= "no gaps") view.
export function coverageDrillView(value: unknown): CoverageDrillView {
  if (!isRow(value) || !Array.isArray(value.items) || typeof value.total !== "number" || typeof value.hasMore !== "boolean") {
    throw new Error(tr("覆盖钻取返回结构无效", "The coverage drill-down returned an invalid structure"));
  }
  const next = text(value.nextCursor);
  if (value.hasMore && !next) throw new Error(tr("覆盖钻取声称还有下一页却没有游标", "The coverage drill-down claims another page but gave no cursor"));
  const window = isRow(value.window) ? value.window : {};
  const rounds = (Array.isArray(value.rounds) ? value.rounds : []).filter(isRow).map((round) => {
    const counts = isRow(round.itemCounts) ? round.itemCounts : {};
    const parts = Object.entries(counts).map(([status, count]) => `${workItemStatusLabel(status)} ${count}`);
    const source = text(round.sourceResult);
    return tr(
      `第 ${num(round.runId) ?? "?"} 轮（${runStatusLabel(text(round.runStatus))}）${text(round.createdAt)} → ${text(round.completedAt) || "未结束"}：${parts.join(" · ") || "无工作项"}${source ? " · 来源结果 " + source : ""}`,
      `Round ${num(round.runId) ?? "?"} (${runStatusLabel(text(round.runStatus))}) ${text(round.createdAt)} → ${text(round.completedAt) || "not finished"}: ${parts.join(" · ") || "no work items"}${source ? " · source result " + source : ""}`,
    );
  });
  const axis = isRow(value.unitAxis) ? value.unitAxis : {};
  const unitState = (["reported", "not_reported", "not_asset_scoped"].includes(text(axis.state)) ? text(axis.state) : "unknown") as CoverageDrillView["unitState"];
  const units = unitState === "reported" && Array.isArray(axis.rows)
    ? axis.rows.filter(isRow).map((row) => ({
        entityId: num(row.entityId) ?? 0,
        gaps: num(row.gaps) ?? 0,
        line: tr(`${text(row.legalName)}：${num(row.completed) ?? 0} / ${num(row.total) ?? 0} 完成，缺口 ${num(row.gaps) ?? 0}`, `${text(row.legalName)}: ${num(row.completed) ?? 0} / ${num(row.total) ?? 0} done, gaps ${num(row.gaps) ?? 0}`),
      }))
    : [];
  const axisReason = text(axis.reason);
  const unitNote = unitState === "reported"
    ? tr("单位来自归属模块逐资产投影；未归属的资产单列，不按名称或地区推断", "Units come from the ownership module's per-asset projection; assets without an owner are listed separately, never inferred from names or regions")
    : unitState === "not_asset_scoped"
      ? text(axis.label) || tr("这个来源不按资产计划，不按单位分摊", "This source is not planned per asset, so it is not split by unit")
      : tr(`单位维度未报告${axisReason ? "：" + axisReason : ""}（不是 0）`, `Unit breakdown not reported${axisReason ? ": " + axisReason : ""} (this is not 0)`);
  return {
    windowLabel: text(window.label) || tr("最近一轮", "Latest round"),
    rounds,
    unitState,
    unitNote,
    units,
    items: value.items.filter(isRow).map((row) => {
      const status = text(row.status);
      const target = text(row.origin) || text(row.entity) || tr(`资产 ${num(row.assetId) ?? "—"}`, `Asset ${num(row.assetId) ?? "—"}`);
      const reason = text(row.reason);
      return {
        id: num(row.id) ?? 0,
        gap: status !== "completed",
        line: tr(
          `#${num(row.id)} 第 ${num(row.runId)} 轮 · ${target} · ${workItemStatusLabel(status)}${reason ? " · " + reason : ""} · 请求 ${num(row.requests) ?? 0}`,
          `#${num(row.id)} round ${num(row.runId)} · ${target} · ${workItemStatusLabel(status)}${reason ? " · " + reason : ""} · requests ${num(row.requests) ?? 0}`,
        ),
      };
    }),
    total: value.total,
    hasMore: value.hasMore,
    nextCursor: next,
  };
}
