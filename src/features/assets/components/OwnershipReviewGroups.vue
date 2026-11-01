<script setup lang="ts">
// O2 决策组队列（设计文档 §7/§8）。分析与分组在后端任务中完成；本组件只读取分页组摘要、按需展开成员，
// 批量动作只提交 groupId + 预览返回的版本/哈希，从不下载或回传资产 ID 列表。所有响应按 projectId + 代次丢弃过期结果。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  ownershipReviewApi, OWNERSHIP_LANE_LABELS, OWNERSHIP_NAV_LABELS,
  type OwnershipAnalysisOverview, type OwnershipGroupAction, type OwnershipGroupDecisionPreview, type OwnershipGroupDecisionResult,
  type OwnershipGroupMember, type OwnershipReviewGroup, type OwnershipRevertPreview,
} from "../ownershipDirectoryApi";
import { useI18n } from "../uiPorts";

const props = defineProps<{ projectId: number; refreshToken?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; applied: [projectId: number] }>();
const { tr } = useI18n();

const GROUP_PAGE_SIZE = 20;
const MEMBER_PAGE_SIZE = 50;
// O3 轮询退避：有进展时 1 秒；无进展每次 ×1.6，上限 10 秒；读取失败每次 ×2，上限 30 秒；有进展立即回到 1 秒。
const POLL_MIN_MS = 1000;
const POLL_IDLE_MAX_MS = 10000;
const POLL_ERROR_MAX_MS = 30000;
function nextPollDelay(current: number, progressed: boolean, failed: boolean) {
  if (failed) return Math.min(Math.max(current, POLL_MIN_MS) * 2, POLL_ERROR_MAX_MS);
  if (progressed) return POLL_MIN_MS;
  return Math.min(Math.round(Math.max(current, POLL_MIN_MS) * 1.6), POLL_IDLE_MAX_MS);
}
// 标签表存 [中文, English]，渲染/调用时再解析，切换语言即时生效；外部表若仍是单一字符串则原样显示。
type LabelPair = readonly [string, string];
function labelText(value: string | LabelPair | undefined, fallback = "") {
  return value === undefined ? fallback : typeof value === "string" ? value : tr(value[0], value[1]);
}
const MODE_REASON_LABELS: Record<string, LabelPair> = { requested_full: ["手动全量", "requested manually"], no_snapshot: ["首次分析", "first analysis"], base_without_watermark: ["基线无水位", "baseline has no watermark"] };
const ACTION_LABELS: Record<OwnershipGroupAction, LabelPair> = { confirm_ownership: ["确认归属（不授权）", "Confirm ownership (no authorization)"], related: ["仅集团/业务关联", "Group/business related only"], third_party: ["第三方服务", "Third-party service"], excluded: ["排除", "Exclude"] };
const READINESS_LABELS: Record<string, LabelPair> = { rules_only: ["仅确定规则", "Deterministic rules only"], model_not_ready: ["模型未接入", "Model not connected"], analysis_running: ["分析进行中", "Analysis running"], stale_snapshot: ["旧快照", "Stale snapshot"], no_snapshot: ["尚无快照", "No snapshot yet"] };
const REASON_LABELS: Record<string, LabelPair> = {
  deleted_or_unlinked: ["已删除或不再关联", "Deleted or unlinked"], left_review_scope: ["已离开复核范围", "Left review scope"], decision_changed: ["归属结论/授权在快照后变化", "Ownership decision/authorization changed after snapshot"],
  evidence_changed: ["证据在快照后变化", "Evidence changed after snapshot"], manual_decision_locked: ["已有人工结论（不覆盖）", "Manual decision exists (not overwritten)"], write_guard_missed: ["写入守卫未命中", "Write guard not matched"],
  changed_after_operation: ["操作之后已被修改（保留现状）", "Modified after the operation (kept as is)"], authorization_not_restored: ["旧值含允许/准入，撤销后不恢复", "Previous value was allowed/eligible; not restored on revert"],
  no_decision_row: ["无结论行", "No decision row"], already_revoked: ["已是撤销状态", "Already revoked"],
};
const BOUNDARY_LABELS: Record<string, LabelPair> = {
  registrable_domain: ["可注册域", "Registrable domain"], exact_host: ["精确主机（共享域名不泛化）", "Exact host (shared domain not generalized)"], private_suffix_tenant: ["托管平台租户", "Hosted platform tenant"], public_suffix_host: ["公共后缀主机", "Public suffix host"], ip: ["IP", "IP"], none: ["无主机", "No host"],
};
const actionLabel = (action: OwnershipGroupAction) => labelText(ACTION_LABELS[action], action);
const reasonLabel = (reason: string) => labelText(REASON_LABELS[reason], reason);

const overview = ref<OwnershipAnalysisOverview | null>(null);
const loadedProjectId = ref(0);
const groups = ref<OwnershipReviewGroup[]>([]);
const groupsTotal = ref(0);
const groupsCursor = ref("");
const groupsRunId = ref(0);
const loadingGroups = ref(false);
const laneFilter = ref("");
const navFilter = ref("");
const regionFilter = ref("");
const expanded = ref<Record<number, { items: OwnershipGroupMember[]; total: number; cursor: string; loading: boolean }>>({});
const preview = ref<OwnershipGroupDecisionPreview | null>(null);
const previewGroup = ref<OwnershipReviewGroup | null>(null);
// 每份预览一个 operationId；网络失败后重试沿用同一个（后端幂等），预览失效后丢弃。
const operationId = ref("");
const lastResult = ref<OwnershipGroupDecisionResult | null>(null);
const starting = ref(false);
const cancelling = ref(false);
const previewing = ref(false);
const applying = ref(false);
// O3：大组写入分块进行；首个调用在预算内返回 applying 时，按操作编号轮询真实进度直至完成。
const pendingOperation = ref<OwnershipGroupDecisionResult | null>(null);
const revertPreview = ref<OwnershipRevertPreview | null>(null);
const revertOperationId = ref("");
const reverting = ref(false);
const revokeTarget = ref<OwnershipReviewGroup | null>(null);
const revokeOperationId = ref("");
const revoking = ref(false);
const pollDelay = ref(POLL_MIN_MS);
let lastProgressKey = "";
let generation = 0;
let pollTimer: ReturnType<typeof setTimeout> | null = null;

const ready = computed(() => loadedProjectId.value === props.projectId && !!overview.value);
const snapshot = computed(() => overview.value?.snapshot ?? null);
const activeRun = computed(() => overview.value?.activeRun ?? null);
const queue = computed(() => overview.value?.queue ?? null);
const stale = computed(() => !!overview.value?.readiness.includes("stale_snapshot"));
const increment = computed(() => (queue.value ? queue.value.newSinceSnapshot + queue.value.changedSinceSnapshot : 0));
const isCurrent = (projectId: number, token: number) => token === generation && projectId === props.projectId;
const newOperationId = () => (globalThis.crypto?.randomUUID ? globalThis.crypto.randomUUID() : `op-${Date.now()}-${Math.random().toString(36).slice(2, 12)}`);

function stopPolling() { if (pollTimer) { clearTimeout(pollTimer); pollTimer = null; } }
function schedulePoll(projectId: number, token: number, progressKey: string | null, failed = false) {
  stopPolling();
  const progressed = progressKey !== null && progressKey !== lastProgressKey;
  if (progressKey !== null) lastProgressKey = progressKey;
  pollDelay.value = nextPollDelay(pollDelay.value, progressed, failed);
  pollTimer = setTimeout(() => {
    if (!isCurrent(projectId, token)) return;
    if (pendingOperation.value) void pollOperation(); else void loadOverview();
  }, pollDelay.value);
}
function runKindText(run: { runKind?: string; baseRunId?: number | null; modeReason?: string; reclassifiedAssets?: number; carriedAssets?: number; resumeCount?: number }) {
  const resumed = run.resumeCount ? tr(` · 崩溃/重启后接管 ${run.resumeCount} 次`, ` · taken over after crash/restart ${run.resumeCount} time(s)`) : "";
  if (run.runKind === "incremental") return tr(`增量（基于快照 #${run.baseRunId}，重新分类 ${(run.reclassifiedAssets ?? 0).toLocaleString()} · 沿用 ${(run.carriedAssets ?? 0).toLocaleString()}）${resumed}`, `Incremental (based on snapshot #${run.baseRunId}, reclassified ${(run.reclassifiedAssets ?? 0).toLocaleString()} · carried over ${(run.carriedAssets ?? 0).toLocaleString()})${resumed}`);
  const reason = run.modeReason ?? "";
  const changedVersions = reason.split(":").slice(1).join(":");
  const label = labelText(MODE_REASON_LABELS[reason], reason.startsWith("versions_changed") ? tr(`版本变化：${changedVersions}`, `versions changed: ${changedVersions}`) : reason);
  return tr(`全量${label ? "（" + label + "）" : ""}${resumed}`, `Full${label ? " (" + label + ")" : ""}${resumed}`);
}
function resetState() {
  generation++; stopPolling();
  overview.value = null; loadedProjectId.value = 0; groups.value = []; groupsTotal.value = 0; groupsCursor.value = ""; groupsRunId.value = 0;
  expanded.value = {}; preview.value = null; previewGroup.value = null; operationId.value = ""; lastResult.value = null;
  pendingOperation.value = null; revertPreview.value = null; revertOperationId.value = ""; revokeTarget.value = null; revokeOperationId.value = "";
  pollDelay.value = POLL_MIN_MS; lastProgressKey = "";
}
function laneLabel(lane: string) { return labelText(OWNERSHIP_LANE_LABELS[lane], lane); }
function navLabel(nav: string) { return labelText(OWNERSHIP_NAV_LABELS[nav], nav); }
function groupTitle(group: OwnershipReviewGroup) {
  const who = group.candidateLabel || tr("未识别单位", "Unidentified entity");
  const tag = group.candidateSource === "label_candidate" ? tr("（标签候选）", " (label candidate)") : group.candidateSource === "network_clue" ? tr("（网络线索，不是法人）", " (network clue, not a legal entity)") : group.candidateSource === "directory_unbound" ? tr("（目录候选，项目未绑定）", " (directory candidate, project not bound)") : "";
  return `${who}${tag} / ${labelText(BOUNDARY_LABELS[group.boundaryKind], group.boundaryKind)}${group.boundaryValue ? ` ${group.boundaryValue}` : ""}`;
}
function permissionText(group: OwnershipReviewGroup) {
  const manual = group.decisionSource === "manual";
  return tr(
    `授权 ${group.authorizationStatus}，暴露面准入${group.exposureEligible ? "是" : "否"}${manual ? "，人工结论 " + group.decisionStatus : ""}`,
    `Authorization ${group.authorizationStatus}, exposure eligible: ${group.exposureEligible ? "yes" : "no"}${manual ? ", manual decision " + group.decisionStatus : ""}`,
  );
}

async function loadOverview(options: { reloadGroups?: boolean } = {}) {
  const projectId = props.projectId; const token = generation;
  try {
    const next = await ownershipReviewApi.readOwnershipAnalysis(projectId);
    if (!isCurrent(projectId, token)) return;
    overview.value = next; loadedProjectId.value = projectId;
    const snapshotId = next.snapshot?.id ?? 0;
    if (snapshotId && (options.reloadGroups || snapshotId !== groupsRunId.value)) await loadGroups(true);
    if (!snapshotId) { groups.value = []; groupsTotal.value = 0; groupsCursor.value = ""; groupsRunId.value = 0; }
    stopPolling();
    if (pendingOperation.value) schedulePoll(projectId, token, null);
    else if (next.activeRun) schedulePoll(projectId, token, `${next.activeRun.id}:${next.activeRun.status}:${next.activeRun.processedAssets}`);
    else { pollDelay.value = POLL_MIN_MS; lastProgressKey = ""; }
  } catch (error) {
    if (!isCurrent(projectId, token)) return;
    emit("notify", "error", String(error));
    // 正在跟踪的任务读取失败：按错误退避继续，而不是放弃进度显示或密集重试。
    if (overview.value?.activeRun || pendingOperation.value) schedulePoll(projectId, token, null, true);
  }
}
async function pollOperation() {
  const projectId = props.projectId; const token = generation; const current = pendingOperation.value;
  if (!current) return;
  try {
    const view = await ownershipReviewApi.readOwnershipOperation(projectId, current.operationId);
    if (!isCurrent(projectId, token) || pendingOperation.value?.operationId !== current.operationId) return;
    const result: OwnershipGroupDecisionResult = { ...current, applied: view.applied, skipped: view.skipped, conflicted: view.conflicted, failed: view.failed, reasons: view.reasons, status: view.status === "completed" ? "completed" : view.status === "failed" ? "failed" : "applying", totalItems: view.totalItems, doneItems: view.doneItems };
    if (view.status === "applying") { pendingOperation.value = result; schedulePoll(projectId, token, `${view.operationId}:${view.doneItems}`); return; }
    pendingOperation.value = null; lastResult.value = result; pollDelay.value = POLL_MIN_MS;
    if (view.status === "failed") emit("notify", "error", tr(`组操作 ${view.operationId} 失败：${view.error}（已写入的分块保留，未授予任何权限）`, `Group operation ${view.operationId} failed: ${view.error} (chunks already written are kept; no permissions were granted)`));
    else { emit("notify", "success", tr(`组操作完成：写入 ${result.applied}，跳过 ${result.skipped}，冲突 ${result.conflicted}，失败 ${result.failed}`, `Group operation complete: ${result.applied} written, ${result.skipped} skipped, ${result.conflicted} conflicted, ${result.failed} failed`)); emit("applied", projectId); }
    await loadOverview({ reloadGroups: true });
  } catch (error) {
    if (isCurrent(projectId, token)) { emit("notify", "error", String(error)); schedulePoll(projectId, token, null, true); }
  }
}
async function loadGroups(reset: boolean) {
  const projectId = props.projectId; const token = generation; const run = overview.value?.snapshot;
  if (!run || loadingGroups.value && !reset) return;
  loadingGroups.value = true;
  try {
    const page = await ownershipReviewApi.pageOwnershipGroups({ projectId, runId: run.id, lane: laneFilter.value, navBucket: navFilter.value, region: regionFilter.value.trim(), pageSize: GROUP_PAGE_SIZE, cursor: reset ? "" : groupsCursor.value });
    if (!isCurrent(projectId, token) || page.snapshot.id !== overview.value?.snapshot?.id) return;
    groups.value = reset ? page.items : [...groups.value, ...page.items];
    groupsTotal.value = page.total; groupsCursor.value = page.nextCursor; groupsRunId.value = page.snapshot.id;
    if (reset) expanded.value = {};
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) loadingGroups.value = false; }
}
function applyFilters() { void loadGroups(true); }
async function startAnalysis() {
  const projectId = props.projectId; const token = generation;
  if (starting.value) return;
  starting.value = true;
  try {
    await ownershipReviewApi.startOwnershipAnalysis(projectId);
    if (isCurrent(projectId, token)) await loadOverview();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) starting.value = false; }
}
async function cancelAnalysis() {
  const projectId = props.projectId; const token = generation; const run = activeRun.value;
  if (!run || cancelling.value) return;
  cancelling.value = true;
  try {
    await ownershipReviewApi.cancelOwnershipAnalysis(projectId, run.id);
    if (isCurrent(projectId, token)) { emit("notify", "info", tr(`分析任务 #${run.id} 已请求取消`, `Cancellation requested for analysis task #${run.id}`)); await loadOverview(); }
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) cancelling.value = false; }
}
async function loadMembers(group: OwnershipReviewGroup, more = false) {
  const projectId = props.projectId; const token = generation;
  const current = expanded.value[group.id];
  if (!more && current) { const next = { ...expanded.value }; delete next[group.id]; expanded.value = next; return; }
  if (current?.loading) return;
  expanded.value = { ...expanded.value, [group.id]: { items: current?.items ?? [], total: current?.total ?? group.memberCount, cursor: current?.cursor ?? "", loading: true } };
  try {
    const page = await ownershipReviewApi.pageOwnershipGroupMembers({ projectId, runId: group.runId, groupId: group.id, pageSize: MEMBER_PAGE_SIZE, cursor: more ? current?.cursor ?? "" : "" });
    if (!isCurrent(projectId, token) || !expanded.value[group.id]) return;
    expanded.value = { ...expanded.value, [group.id]: { items: more ? [...(current?.items ?? []), ...page.items] : page.items, total: page.total, cursor: page.nextCursor, loading: false } };
  } catch (error) {
    if (isCurrent(projectId, token)) { const next = { ...expanded.value }; delete next[group.id]; expanded.value = next; emit("notify", "error", String(error)); }
  }
}
async function previewDecision(group: OwnershipReviewGroup, action: OwnershipGroupAction) {
  const projectId = props.projectId; const token = generation;
  if (previewing.value || applying.value) return;
  previewing.value = true; preview.value = null; previewGroup.value = null; operationId.value = "";
  try {
    const next = await ownershipReviewApi.previewOwnershipGroupDecision({ projectId, runId: group.runId, groupId: group.id, action, note: "" });
    if (!isCurrent(projectId, token)) return;
    preview.value = next; previewGroup.value = group; operationId.value = newOperationId();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) previewing.value = false; }
}
function closePreview() { preview.value = null; previewGroup.value = null; operationId.value = ""; }
async function applyDecision() {
  const projectId = props.projectId; const token = generation; const current = preview.value; const opId = operationId.value;
  if (!current || !opId || applying.value || current.projectId !== projectId) return;
  applying.value = true;
  try {
    const result = await ownershipReviewApi.applyOwnershipGroupDecision({
      operationId: opId, projectId, runId: current.runId, groupId: current.groupId, groupVersion: current.groupVersion,
      policyVersion: current.policyVersion, directoryVersion: current.directoryVersion, bindingVersion: current.bindingVersion,
      action: current.action, note: current.note, expectedActionHash: current.expectedActionHash,
    });
    if (!isCurrent(projectId, token)) { emit("notify", "success", tr(`项目 #${projectId} 的组操作已完成：写入 ${result.applied} 条`, `Group operation for project #${projectId} completed: ${result.applied} written`)); return; }
    closePreview();
    if (result.status === "applying") {
      // 大组：已在预算内提交部分分块，后台继续；只显示真实进度，完成前不向下游发布任何“整组已允许”。
      pendingOperation.value = result; pollDelay.value = POLL_MIN_MS; lastProgressKey = "";
      emit("notify", "info", tr(`${actionLabel(result.action)}：大组分块写入中 ${result.doneItems ?? 0} / ${result.totalItems ?? 0}（后台继续，可随时离开页面）`, `${actionLabel(result.action)}: writing large group in chunks ${result.doneItems ?? 0} / ${result.totalItems ?? 0} (continues in the background; you can leave this page)`));
      schedulePoll(projectId, token, `${result.operationId}:${result.doneItems ?? 0}`);
      return;
    }
    lastResult.value = result;
    emit("notify", "success", tr(`${actionLabel(result.action)}：写入 ${result.applied}，跳过 ${result.skipped}，冲突 ${result.conflicted}，失败 ${result.failed}${result.replayed ? "（重复提交，未再次写入）" : ""}`, `${actionLabel(result.action)}: ${result.applied} written, ${result.skipped} skipped, ${result.conflicted} conflicted, ${result.failed} failed${result.replayed ? " (duplicate submission, not written again)" : ""}`));
    emit("applied", projectId);
    await loadOverview({ reloadGroups: true });
  } catch (error) {
    const text = String(error);
    if (!isCurrent(projectId, token)) { emit("notify", "error", tr(`项目 #${projectId}：${text}`, `Project #${projectId}: ${text}`)); return; }
    if (/OWNERSHIP_(PREVIEW_STALE|SNAPSHOT_STALE|OPERATION_PAYLOAD_MISMATCH|GROUP_ACTION_NOT_ALLOWED)/.test(text)) {
      closePreview();
      emit("notify", "error", tr(`${text}（预览已失效，请重新预览）`, `${text} (preview is no longer valid; preview again)`));
      await loadOverview({ reloadGroups: true });
    } else {
      emit("notify", "error", tr(`${text}（可重试；同一操作编号重复提交不会重复写入）`, `${text} (safe to retry; resubmitting the same operation ID won't write twice)`));
    }
  } finally { if (token === generation) applying.value = false; }
}

async function startFull() {
  const projectId = props.projectId; const token = generation;
  if (starting.value) return;
  starting.value = true;
  try {
    await ownershipReviewApi.startOwnershipAnalysis(projectId, "full");
    if (isCurrent(projectId, token)) await loadOverview();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) starting.value = false; }
}
// 组级撤销：撤销一次已完成的组归属操作；只恢复之后未被改动的成员，绝不恢复允许/准入。
async function previewRevert() {
  const projectId = props.projectId; const token = generation; const target = lastResult.value;
  if (!target || reverting.value) return;
  reverting.value = true; revertPreview.value = null; revertOperationId.value = "";
  try {
    const next = await ownershipReviewApi.previewOwnershipGroupRevert({ projectId, targetOperationId: target.operationId, note: "" });
    if (!isCurrent(projectId, token)) return;
    revertPreview.value = next; revertOperationId.value = newOperationId();
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", String(error)); }
  finally { if (token === generation) reverting.value = false; }
}
function closeRevert() { revertPreview.value = null; revertOperationId.value = ""; }
async function applyRevert() {
  const projectId = props.projectId; const token = generation; const current = revertPreview.value; const opId = revertOperationId.value;
  if (!current || !opId || reverting.value || current.projectId !== projectId) return;
  reverting.value = true;
  try {
    const result = await ownershipReviewApi.applyOwnershipGroupRevert({ operationId: opId, projectId, targetOperationId: current.targetOperationId, note: current.note, expectedRevertHash: current.expectedRevertHash });
    if (!isCurrent(projectId, token)) return;
    closeRevert();
    if (result.status === "applying") { pendingOperation.value = result; schedulePoll(projectId, token, `${result.operationId}:${result.doneItems ?? 0}`); return; }
    lastResult.value = result;
    const notRestored = result.reasons.authorization_not_restored;
    emit("notify", "success", tr(`已撤销组操作 ${current.targetOperationId}：恢复 ${result.applied}，保留现状 ${result.conflicted}${notRestored ? "；" + notRestored + " 条旧授权未恢复" : ""}`, `Reverted group operation ${current.targetOperationId}: ${result.applied} restored, ${result.conflicted} kept as is${notRestored ? "; " + notRestored + " previous authorizations not restored" : ""}`));
    emit("applied", projectId);
    await loadOverview({ reloadGroups: true });
  } catch (error) {
    if (!isCurrent(projectId, token)) return;
    if (/OWNERSHIP_(PREVIEW_STALE|REVERT_NOT_ALLOWED|OPERATION_PAYLOAD_MISMATCH)/.test(String(error))) closeRevert();
    emit("notify", "error", String(error));
  } finally { if (token === generation) reverting.value = false; }
}
// 撤销授权：后端先在一个事务内把成员置为已撤销/不准入（下游立即阻断），再更新展示索引。
function askRevoke(group: OwnershipReviewGroup) { revokeTarget.value = group; revokeOperationId.value = newOperationId(); }
function closeRevoke() { revokeTarget.value = null; revokeOperationId.value = ""; }
async function applyRevoke() {
  const projectId = props.projectId; const token = generation; const group = revokeTarget.value; const opId = revokeOperationId.value;
  if (!group || !opId || revoking.value) return;
  revoking.value = true;
  try {
    const result = await ownershipReviewApi.revokeOwnershipGroupAuthorization({ operationId: opId, projectId, runId: group.runId, groupId: group.id, note: "" });
    if (!isCurrent(projectId, token)) return;
    closeRevoke();
    emit("notify", "success", tr(`已撤销授权 ${result.revoked} 条（已阻断下游暴露面）；已是撤销 ${result.alreadyRevoked}，无结论行 ${result.withoutDecision}`, `Revoked authorization for ${result.revoked} (downstream exposure blocked); ${result.alreadyRevoked} already revoked, ${result.withoutDecision} without a decision row`));
    emit("applied", projectId);
    await loadOverview({ reloadGroups: true });
  } catch (error) { if (isCurrent(projectId, token)) emit("notify", "error", tr(`${String(error)}（可重试；同一操作编号不会重复撤销）`, `${String(error)} (safe to retry; the same operation ID won't revoke twice)`)); }
  finally { if (token === generation) revoking.value = false; }
}

watch(() => props.projectId, () => { resetState(); void loadOverview(); });
watch(() => props.refreshToken, () => { if (ready.value) void loadOverview(); });
onMounted(() => { void loadOverview(); });
onBeforeUnmount(() => { generation++; stopPolling(); });
</script>

<template>
  <section class="panel ownership-groups">
    <header>
      <div><strong>{{ tr("归属决策组队列", "Ownership decision groups") }}</strong><small>{{ tr("后台按候选单位、域名/托管边界、支持与反证、人工结论和授权状态分组；组确认只写归属事实，不授予扫描或暴露面权限。", "Grouped in the background by candidate entity, domain/hosting boundary, supporting and counter evidence, manual decision and authorization status. Confirming a group only records ownership facts and grants no scan or exposure permissions.") }}</small></div>
      <span>
        <template v-for="item in overview?.readiness ?? []" :key="item"><em class="ownership-groups-tag" :class="`tag-${item}`">{{ labelText(READINESS_LABELS[item], item) }}</em></template>
        <button class="button ghost" :disabled="!ready || starting || !!activeRun" :title="tr('自动：版本未变时按数据水位增量更新，否则全量', 'Auto: incremental update from the data watermark when versions are unchanged, otherwise a full run')" @click="startAnalysis">{{ activeRun ? tr("分析中", "Analyzing") : snapshot ? tr("更新分析", "Update analysis") : tr("开始分析", "Start analysis") }}</button>
        <button v-if="snapshot && !activeRun" class="button ghost" :disabled="!ready || starting" @click="startFull">{{ tr("全量重算", "Rerun in full") }}</button>
        <button v-if="activeRun" class="button ghost" :disabled="cancelling || activeRun.cancelRequested" @click="cancelAnalysis">{{ activeRun.cancelRequested ? tr("正在取消", "Cancelling") : tr("取消分析", "Cancel analysis") }}</button>
      </span>
    </header>
    <div v-if="!ready" class="ownership-groups-empty">{{ tr("加载分析状态…", "Loading analysis status…") }}</div>
    <template v-else>
      <div v-if="activeRun" class="ownership-groups-progress">{{ tr(`分析任务 #${activeRun.id} ${activeRun.status === "queued" ? "排队中" : "运行中"}：已处理 ${activeRun.processedAssets.toLocaleString()} / ${activeRun.totalAssets.toLocaleString()} 条 · ${runKindText(activeRun)}`, `Analysis task #${activeRun.id} ${activeRun.status === "queued" ? "queued" : "running"}: processed ${activeRun.processedAssets.toLocaleString()} / ${activeRun.totalAssets.toLocaleString()} · ${runKindText(activeRun)}`) }}<template v-if="snapshot">{{ tr(`；下方仍为旧快照 #${snapshot.id}`, `; the list below still shows old snapshot #${snapshot.id}`) }}</template><small>{{ tr(` · 下次刷新 ${Math.round(pollDelay / 100) / 10} 秒`, ` · next refresh in ${Math.round(pollDelay / 100) / 10}s`) }}</small></div>
      <div v-if="pendingOperation" class="ownership-groups-progress">{{ tr(`组操作 ${pendingOperation.operationId} 分块写入中：${(pendingOperation.doneItems ?? 0).toLocaleString()} / ${(pendingOperation.totalItems ?? 0).toLocaleString()}（写入 ${pendingOperation.applied} · 冲突 ${pendingOperation.conflicted}）；完成前不发布整组结果`, `Group operation ${pendingOperation.operationId} writing in chunks: ${(pendingOperation.doneItems ?? 0).toLocaleString()} / ${(pendingOperation.totalItems ?? 0).toLocaleString()} (written ${pendingOperation.applied} · conflicted ${pendingOperation.conflicted}); whole-group results are not published until it finishes`) }}<small>{{ tr(` · 下次刷新 ${Math.round(pollDelay / 100) / 10} 秒`, ` · next refresh in ${Math.round(pollDelay / 100) / 10}s`) }}</small></div>
      <div v-if="overview?.latestRun && ['failed','cancelled'].includes(overview.latestRun.status) && overview.latestRun.id !== snapshot?.id" class="ownership-groups-progress">{{ tr(`最近一次分析 #${overview.latestRun.id} ${overview.latestRun.status === "failed" ? "失败：" + overview.latestRun.error : "已取消"}（已处理 ${overview.latestRun.processedAssets.toLocaleString()} 条，未发布快照）`, `Latest analysis #${overview.latestRun.id} ${overview.latestRun.status === "failed" ? "failed: " + overview.latestRun.error : "cancelled"} (processed ${overview.latestRun.processedAssets.toLocaleString()}; no snapshot published)`) }}</div>
      <div v-if="!snapshot" class="ownership-groups-empty">{{ tr("尚无完成的分析快照。没有快照不代表没有待办——请先开始分析。", "No completed analysis snapshot yet. No snapshot doesn't mean nothing is pending — start an analysis first.") }}</div>
      <template v-else-if="queue">
        <div v-if="stale" class="ownership-groups-stale">{{ tr(`旧快照：快照之后 ${overview?.staleReasons.join("、")} 已变化。组确认会被拒绝，请重新分析。`, `Stale snapshot: ${overview?.staleReasons.join(", ")} changed after the snapshot. Group confirmations will be rejected; run the analysis again.`) }}</div>
        <div class="ownership-groups-stats">
          <div><small>{{ tr("待复核资产", "Assets needing review") }}</small><strong>{{ queue.pendingAssets.toLocaleString() }}</strong></div>
          <div><small>{{ tr("待复核组", "Groups needing review") }}</small><strong>{{ queue.pendingGroups.toLocaleString() }}</strong></div>
          <div><small>{{ tr("冲突组", "Conflict groups") }}</small><strong>{{ queue.conflictGroups.toLocaleString() }}</strong><span>{{ tr(`反证组 ${queue.counterEvidenceGroups.toLocaleString()}`, `Counter-evidence groups ${queue.counterEvidenceGroups.toLocaleString()}`) }}</span></div>
          <div><small>{{ tr("已完成组", "Completed groups") }}</small><strong>{{ queue.completedGroups.toLocaleString() }}</strong><span>{{ tr("含人工结论组", "Includes groups with manual decisions") }}</span></div>
          <div><small>{{ tr("未处理增量", "Unprocessed changes") }}</small><strong>{{ increment.toLocaleString() }}</strong><span>{{ tr(`新增 ${queue.newSinceSnapshot.toLocaleString()} · 变化 ${queue.changedSinceSnapshot.toLocaleString()}`, `New ${queue.newSinceSnapshot.toLocaleString()} · changed ${queue.changedSinceSnapshot.toLocaleString()}`) }}</span></div>
        </div>
        <p class="ownership-groups-meta">{{ tr(`快照 #${snapshot.id} · ${runKindText(snapshot)} · 完成于 ${snapshot.finishedAt} · 成员 ${queue.memberTotal.toLocaleString()} / 组计数合计 ${queue.groupMemberSum.toLocaleString()} · 策略 v${snapshot.versions.policyVersion} · 目录 v${snapshot.versions.directoryVersion} · 绑定 v${snapshot.versions.bindingVersion} · ${snapshot.versions.algorithmVersion}`, `Snapshot #${snapshot.id} · ${runKindText(snapshot)} · finished ${snapshot.finishedAt} · members ${queue.memberTotal.toLocaleString()} / group count total ${queue.groupMemberSum.toLocaleString()} · policy v${snapshot.versions.policyVersion} · directory v${snapshot.versions.directoryVersion} · binding v${snapshot.versions.bindingVersion} · ${snapshot.versions.algorithmVersion}`) }}</p>
        <div class="ownership-groups-filters">
          <select v-model="laneFilter" @change="applyFilters"><option value="">{{ tr("全部类型", "All types") }}</option><option v-for="(label, key) in OWNERSHIP_LANE_LABELS" :key="key" :value="key">{{ labelText(label, key) }}</option></select>
          <select v-model="navFilter" @change="applyFilters"><option value="">{{ tr("全部单位导航", "All entity tiers") }}</option><option v-for="(label, key) in OWNERSHIP_NAV_LABELS" :key="key" :value="key">{{ labelText(label, key) }}</option></select>
          <input v-model="regionFilter" :placeholder="tr('仅看某地域，例如 浙江', 'Filter by region, e.g. Zhejiang')" @keyup.enter="applyFilters"/>
          <button class="button ghost" @click="applyFilters">{{ tr("筛选", "Filter") }}</button>
          <small>{{ tr(`共 ${groupsTotal.toLocaleString()} 组，已加载 ${groups.length.toLocaleString()}；冲突与反证组优先`, `${groupsTotal.toLocaleString()} groups, ${groups.length.toLocaleString()} loaded; conflict and counter-evidence groups first`) }}</small>
        </div>
        <div v-if="lastResult" class="ownership-groups-result">{{ tr(`最近操作 ${lastResult.operationId}：写入 ${lastResult.applied} · 跳过 ${lastResult.skipped} · 冲突 ${lastResult.conflicted} · 失败 ${lastResult.failed}`, `Latest operation ${lastResult.operationId}: written ${lastResult.applied} · skipped ${lastResult.skipped} · conflicted ${lastResult.conflicted} · failed ${lastResult.failed}`) }}<template v-for="(value, key) in lastResult.reasons" :key="key"> · {{ reasonLabel(key) }} {{ value }}</template>
          <button v-if="String(lastResult.action) !== 'revert' && lastResult.status !== 'applying'" :disabled="reverting || applying" @click="previewRevert">{{ tr("预览撤销此次组操作", "Preview reverting this group operation") }}</button></div>
        <div v-if="revertPreview" class="ownership-group-preview" role="dialog" :aria-label="tr('组操作撤销预览', 'Group operation revert preview')">
          <header><strong>{{ tr(`撤销组操作 ${revertPreview.targetOperationId}（${revertPreview.targetAction}）`, `Revert group operation ${revertPreview.targetOperationId} (${revertPreview.targetAction})`) }}</strong></header>
          <p>{{ tr(`该操作写入 ${revertPreview.appliedInTarget} 条：可恢复 `, `The operation wrote ${revertPreview.appliedInTarget}: `) }}<strong>{{ revertPreview.revertible }}</strong>{{ tr(` 条；之后已被修改、保留现状 ${revertPreview.conflicted} 条`, ` can be restored; ${revertPreview.conflicted} were modified afterward and stay as they are`) }}<template v-for="(value, key) in revertPreview.reasons" :key="key">{{ tr(`（${reasonLabel(key)} ${value}）`, ` (${reasonLabel(key)} ${value})`) }}</template></p>
          <p>{{ tr(`授权：撤销不会恢复任何“允许/准入”；旧值含允许/准入的 ${revertPreview.authorizationNotRestored} 条恢复为未授权、不准入。`, `Authorization: reverting never restores any “allowed/eligible” state; ${revertPreview.authorizationNotRestored} items whose previous value was allowed/eligible are restored as unauthorized and not eligible.`) }}</p>
          <ul><li v-for="note in revertPreview.notes" :key="note">{{ note }}</li></ul>
          <footer><button class="button ghost" :disabled="reverting" @click="closeRevert">{{ tr("取消", "Cancel") }}</button><button class="button primary" :disabled="reverting || !revertPreview.revertible" @click="applyRevert">{{ reverting ? tr("提交中", "Submitting") : tr(`确认撤销 ${revertPreview.revertible} 条`, `Confirm revert of ${revertPreview.revertible}`) }}</button></footer>
        </div>
        <div v-if="revokeTarget" class="ownership-group-preview" role="dialog" :aria-label="tr('撤销授权确认', 'Confirm authorization revocation')">
          <header><strong>{{ tr(`撤销授权 · ${groupTitle(revokeTarget)}`, `Revoke authorization · ${groupTitle(revokeTarget)}`) }}</strong></header>
          <p>{{ tr(`将把该组 ${revokeTarget.memberCount.toLocaleString()} 条成员中已有结论的资产置为：授权 = 已撤销，暴露面准入 = 否。下游暴露面采集立即停止使用这些资产（先阻断，再更新本页显示）；归属结论保持不变。`, `Assets with a decision among this group's ${revokeTarget.memberCount.toLocaleString()} members will be set to: authorization = revoked, exposure eligible = no. Downstream exposure collection stops using these assets immediately (blocked first, then this page updates); ownership decisions stay unchanged.`) }}</p>
          <p>{{ tr("撤销后不会被任何后台恢复或组撤销重新授权；需要恢复只能逐条重新授权。", "Once revoked, no background job or group revert will re-authorize them; to restore, re-authorize assets one by one.") }}</p>
          <footer><button class="button ghost" :disabled="revoking" @click="closeRevoke">{{ tr("取消", "Cancel") }}</button><button class="button primary exclude" :disabled="revoking" @click="applyRevoke">{{ revoking ? tr("提交中", "Submitting") : tr("确认撤销授权", "Confirm revocation") }}</button></footer>
        </div>

        <article v-for="group in groups" :key="group.id" class="ownership-group-card" :class="`lane-${group.lane}`">
          <header>
            <div><strong>{{ groupTitle(group) }}</strong><small>{{ navLabel(group.navBucket) }}<template v-if="group.regionClue">{{ tr(` · 地域线索 ${group.regionClue}`, ` · region clue ${group.regionClue}`) }}</template> · {{ group.relationKind }}</small></div>
            <span class="ownership-group-lane">{{ laneLabel(group.lane) }}</span>
          </header>
          <p>{{ tr(`成员 ${group.memberCount.toLocaleString()} 条 · ${group.distinctHosts.toLocaleString()} 个主机`, `${group.memberCount.toLocaleString()} members · ${group.distinctHosts.toLocaleString()} hosts`) }}<template v-if="group.appliedOperations">{{ tr(` · 已执行组操作 ${group.appliedOperations} 次`, ` · group operations applied ${group.appliedOperations} time(s)`) }}</template><template v-if="group.authorizationRevoked">{{ tr(` · 已撤销授权 ${group.authorizationRevoked} 条`, ` · authorization revoked for ${group.authorizationRevoked}`) }}</template></p>
          <p class="ownership-group-reason">{{ group.reason }}</p>
          <p v-if="group.counterKind" class="ownership-group-counter">{{ tr(`反证：${group.counterKind}（不隐藏，不能整组确认归属）`, `Counter evidence: ${group.counterKind} (not hidden; ownership can't be confirmed for the whole group)`) }}</p>
          <p class="ownership-group-permission">{{ tr(`现有权限：${permissionText(group)}`, `Current permissions: ${permissionText(group)}`) }}</p>
          <ul class="ownership-group-samples">
            <li v-for="sample in group.samples" :key="sample.assetId"><code>{{ sample.host || sample.link || `#${sample.assetId}` }}</code> {{ sample.company || tr("无公司", "No company") }} · {{ sample.current.status }}<em v-if="sample.changedSinceSnapshot">{{ tr(" · 快照后已变化", " · changed since snapshot") }}</em></li>
          </ul>
          <small class="ownership-group-sample-rule">{{ tr(`代表样本 ${group.samples.length} / ${group.memberCount}：${group.sampleRule}`, `Representative samples ${group.samples.length} / ${group.memberCount}: ${group.sampleRule}`) }}</small>
          <footer>
            <button @click="loadMembers(group)">{{ expanded[group.id] ? tr("收起成员", "Hide members") : tr(`查看全部 ${group.memberCount.toLocaleString()} 条成员`, `View all ${group.memberCount.toLocaleString()} members`) }}</button>
            <button v-for="action in group.allowedActions" :key="action" :class="{ confirm: action === 'confirm_ownership', exclude: action === 'excluded' }" :disabled="previewing || applying || stale" @click="previewDecision(group, action)">{{ tr(`预览${actionLabel(action)}`, `Preview: ${actionLabel(action)}`) }}</button>
            <button v-if="group.authorizationStatus === 'allowed' || group.exposureEligible" class="exclude" :disabled="revoking || applying" @click="askRevoke(group)">{{ tr("撤销授权", "Revoke authorization") }}</button>
            <small v-if="!group.allowedActions.length">{{ tr("已有人工结论：请在下方资产明细中逐条修改", "Manual decision exists: change assets one by one in the asset details below") }}</small>
          </footer>
          <div v-if="expanded[group.id]" class="ownership-group-members">
            <table>
              <thead><tr><th>{{ tr("资产", "Asset") }}</th><th>{{ tr("主机 / 可注册域", "Host / registrable domain") }}</th><th>{{ tr("公司", "Company") }}</th><th>{{ tr("当前结论", "Current decision") }}</th><th>{{ tr("授权 / 准入", "Authorization / eligibility") }}</th></tr></thead>
              <tbody><tr v-for="member in expanded[group.id].items" :key="member.assetId" :class="{ changed: member.changedSinceSnapshot }">
                <td>#{{ member.assetId }}</td><td><code>{{ member.host }}</code><small> {{ member.registrableDomain }} · {{ member.suffixSection }}</small></td>
                <td>{{ member.company || tr("无", "None") }}</td><td>{{ member.current.status }}<small v-if="member.current.source"> · {{ member.current.source }}</small><em v-if="member.changedSinceSnapshot">{{ tr(" · 快照后已变化", " · changed since snapshot") }}</em></td>
                <td>{{ member.current.authorizationStatus }} / {{ member.current.exposureEligible ? tr("准入", "Eligible") : tr("不准入", "Not eligible") }}</td>
              </tr></tbody>
            </table>
            <button v-if="expanded[group.id].cursor" :disabled="expanded[group.id].loading" @click="loadMembers(group, true)">{{ tr(`加载更多（${expanded[group.id].items.length} / ${expanded[group.id].total}）`, `Load more (${expanded[group.id].items.length} / ${expanded[group.id].total})`) }}</button>
            <small v-else>{{ tr(`已显示全部 ${expanded[group.id].total} 条`, `Showing all ${expanded[group.id].total}`) }}</small>
          </div>
        </article>
        <button v-if="groupsCursor" class="button ghost ownership-groups-more" :disabled="loadingGroups" @click="loadGroups(false)">{{ loadingGroups ? tr("加载中…", "Loading…") : tr("加载更多组", "Load more groups") }}</button>
      </template>

      <div v-if="preview && previewGroup" class="ownership-group-preview" role="dialog" :aria-label="tr('组批量预览', 'Group batch preview')">
        <header><strong>{{ tr(`预览：${actionLabel(preview.action)} · ${groupTitle(previewGroup)}`, `Preview: ${actionLabel(preview.action)} · ${groupTitle(previewGroup)}`) }}</strong></header>
        <p>{{ tr(`快照 #${preview.runId} · 组 v${preview.groupVersion} · 成员 ${preview.memberCount}：将写入 `, `Snapshot #${preview.runId} · group v${preview.groupVersion} · members ${preview.memberCount}: will write `) }}<strong>{{ preview.applicable }}</strong>{{ tr(` 条，跳过 ${preview.skipped}，冲突（快照后变化，不写入）${preview.conflicted}`, `, skip ${preview.skipped}, conflicts (changed since snapshot, not written) ${preview.conflicted}`) }}</p>
        <p v-if="Object.keys(preview.reasons).length">{{ tr("排除原因：", "Exclusion reasons: ") }}<template v-for="(value, key) in preview.reasons" :key="key">{{ tr(`${reasonLabel(key)} ${value}；`, `${reasonLabel(key)} ${value}; `) }}</template></p>
        <p v-if="preview.excludedSamples.length">{{ tr("被排除样本：", "Excluded samples: ") }}<template v-for="item in preview.excludedSamples" :key="item.assetId">{{ tr(`#${item.assetId}（${reasonLabel(item.reason)}） `, `#${item.assetId} (${reasonLabel(item.reason)}) `) }}</template></p>
        <p>{{ tr(`拟改变：归属 = ${preview.proposed.status}（人工锁定）；授权 = ${preview.proposed.authorization === "unchanged" ? "保持各资产现状" : "拒绝"}；暴露面准入 = ${preview.proposed.exposureEligible === "unchanged" ? "保持各资产现状" : "否"}`, `Proposed change: ownership = ${preview.proposed.status} (manually locked); authorization = ${preview.proposed.authorization === "unchanged" ? "keep each asset's current value" : "denied"}; exposure eligible = ${preview.proposed.exposureEligible === "unchanged" ? "keep each asset's current value" : "no"}`) }}</p>
        <p>{{ tr("现有授权分布：", "Current authorization breakdown: ") }}<template v-for="(value, key) in preview.currentAuthorization" :key="key">{{ tr(`${key} ${value}；`, `${key} ${value}; `) }}</template></p>
        <ul><li v-for="note in preview.notes" :key="note">{{ note }}</li></ul>
        <footer><button class="button ghost" :disabled="applying" @click="closePreview">{{ tr("取消", "Cancel") }}</button><button class="button primary" :disabled="applying || !preview.applicable" @click="applyDecision">{{ applying ? tr("提交中", "Submitting") : tr(`确认对 ${preview.applicable} 条执行`, `Apply to ${preview.applicable}`) }}</button></footer>
      </div>
    </template>
  </section>
</template>

<style scoped>
.ownership-groups { overflow: hidden; }
.ownership-groups > header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 16px 20px; border-bottom: 1px solid var(--ui-line); background: var(--ui-surface); }
.ownership-groups > header > div { display: grid; gap: 3px; }
.ownership-groups header strong { font-size:var(--text-body); }
.ownership-groups header small { color: var(--ui-muted); font-size:var(--text-micro); }
.ownership-groups > header > span { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
.ownership-groups-tag { font-style: normal; font-size:var(--text-micro); padding: 2px 6px; border-radius: var(--radius-md); background: var(--ui-surface-sunken); color: var(--ui-muted); }
.ownership-groups-tag.tag-stale_snapshot, .ownership-groups-stale { background: color-mix(in srgb, var(--amber) 11%, var(--ui-surface)); color: color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); }
.ownership-groups-empty, .ownership-groups-progress, .ownership-groups-stale, .ownership-groups-result, .ownership-groups-meta { margin: 10px 20px; font-size:var(--text-meta); color: var(--ui-muted); }
.ownership-groups-stale { padding: 8px 10px; border-radius: var(--radius-sm); }
.ownership-groups-stats { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 8px; margin: 10px 20px; }
.ownership-groups-stats > div { display: grid; gap: 2px; padding: 10px; border: 1px solid var(--ui-line); border-radius: var(--radius-md); }
.ownership-groups-stats small, .ownership-groups-stats span { color: var(--ui-muted); font-size:var(--text-micro); }
.ownership-groups-stats strong { font-size:var(--text-heading); }
.ownership-groups-filters { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin: 0 20px 10px; font-size:var(--text-meta); }
.ownership-group-card { margin: 0 20px 10px; padding: 10px 12px; border: 1px solid var(--ui-line); border-left: 3px solid var(--ui-line-strong); border-radius: var(--radius-md); font-size:var(--text-meta); }
.ownership-group-card.lane-conflict { border-left-color: #d64545; }
.ownership-group-card.lane-counter_evidence { border-left-color: #e08a00; }
.ownership-group-card.lane-consistent { border-left-color: #2f9e6b; }
.ownership-group-card > header { display: flex; justify-content: space-between; gap: 8px; }
.ownership-group-card > header > div { display: grid; gap: 2px; }
.ownership-group-card p { margin: 4px 0; }
.ownership-group-counter { color: color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); }
.ownership-group-lane { font-size:var(--text-micro); white-space: nowrap; }
.ownership-group-samples { margin: 4px 0; padding-left: 16px; }
.ownership-group-sample-rule { color: var(--ui-muted); font-size:var(--text-micro); }
.ownership-group-card footer { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
.ownership-group-members { margin-top: 8px; max-height: 320px; overflow: auto; }
.ownership-group-members table { width: 100%; border-collapse: collapse; }
.ownership-group-members td, .ownership-group-members th { padding: 3px 6px; border-bottom: 1px solid var(--ui-line); text-align: left; }
.ownership-group-members tr.changed { background: color-mix(in srgb, var(--amber) 8%, var(--ui-surface)); }
.ownership-group-preview { margin: 10px 20px; padding: 12px; border: 1px solid color-mix(in srgb, var(--blue) 38%, var(--ui-line)); border-radius: var(--radius-md); background: var(--ui-surface); font-size:var(--text-meta); }
.ownership-group-preview footer { display: flex; justify-content: flex-end; gap: 8px; }
.ownership-groups-more { margin: 0 20px 12px; }
</style>
