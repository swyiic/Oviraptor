<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { Archive, Eye, MessageCircle, Pause, Play, RefreshCw, RotateCcw, Trash2, X } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelScan } from "../types";
import { sentinelApi } from "../api";
import { useTaskExecutionDetails } from "../execution/useTaskExecutionDetails";
import { useHistoricalImportLedger } from "../results/useHistoricalImportLedger";
import { validTaskPage } from "../results/taskPageContract";
import { createNativeRunStatusLabels } from "./nativeRunStatusLabels";
import AuthorizationControlForm from "./AuthorizationControlForm.vue";
import NativeProcessLogView from "./NativeProcessLogView.vue";
import NativeSdkLogView from "./NativeSdkLogView.vue";
import SourceReviewEvidence from "./SourceReviewEvidence.vue";
import HistoricalJsonImport from "./HistoricalJsonImport.vue";
import {
  createSentinelLabels,
  formatCompactNumber,
  formatNumber,
  latestAttemptLabel,
  routeModeLabel,
  scanInterruption,
  scanSummary,
  scanTitle,
} from "../presentation";

type PreviewTarget = { company: string; url: string; highValue: boolean };
const props = defineProps<{
  scans: SentinelScan[];
  projectId?: number;
  hasMore: boolean;
  loadingMore: boolean;
  preview?: SentinelScan;
  previewTargets: PreviewTarget[];
  attentionCount: number;
  totalTokens: number;
  totalRequests: number;
  tokenScopeLabel: string;
  zeroYieldCount: number;
  zeroYieldTokens: number;
  cacheHitRate: number;
  controlBusy: string;
  highValueCount: (scanId: string) => number;
  // Track A · A1 — per-scan aggregates for the parent task card. targetCount is
  // the scan's URL count; vulnerabilityCount is its vulnerability total, or
  // undefined while that scan's exact count is still being resolved.
  /** Published vulnerabilities for a task; undefined while counts are still loading. */
  vulnerabilityCount: (scanId: string) => number | undefined;
  targetCount: (scanId: string) => number;
  /** The task currently open in 结果与证据. */
  openScanId?: string;
}>();
const emit = defineEmits<{
  preview: [scan: SentinelScan];
  close: [];
  confirm: [scan: SentinelScan];
  pause: [scan: SentinelScan];
  resume: [scan: SentinelScan];
  retry: [scan: SentinelScan];
  remove: [scan: SentinelScan];
  open: [scan: SentinelScan];
  dialog: [scan: SentinelScan];
  loadMore: [];
  archived: [scan: SentinelScan];
}>();
const { t, tr, fmt } = useI18n();
const nativeRunLabels = createNativeRunStatusLabels(tr);
const { statusLabel, retryActionLabel, scanTypeLabel, llmDeploymentLabel } = createSentinelLabels(tr);
const view = ref<"attention" | "history" | "all">("attention");
const {
  importedRuns, importedSelected, importedPreviews, importedLoading, importedPreviewLoading,
  importedError, importedHasMore, importedLoaded, loadImportedRuns, selectImportedRun,
} = useHistoricalImportLedger(view);
let disposed = false;
let archiveSerial = 0;
onBeforeUnmount(() => {
  disposed = true;
  ++archiveSerial;
  ++searchSerial;
  if (searchTimer !== undefined) clearTimeout(searchTimer);
  searchTimer = undefined;
});
const search = ref("");
const searchResults = ref<SentinelScan[]>([]);
const searchLoading = ref(false);
const searchError = ref("");
const archiveBusy = ref("");
watch(() => props.projectId, () => {
  ++archiveSerial;
  archiveBusy.value = "";
}, { flush: "sync" });
const searchHasMore = ref(false);
const searchCursor = ref<Pick<SentinelScan, "updatedAt" | "id">>();
const searchPageSize = 100;
let searchSerial = 0;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
async function loadSearchPage(older = false) {
  if (disposed || !search.value.trim() || (older && (!searchHasMore.value || searchLoading.value))) return;
  const serial = ++searchSerial;
  const query = search.value.trim();
  const currentView = view.value;
  const projectId = props.projectId;
  const cursor = older ? searchCursor.value : undefined;
  searchLoading.value = true;
  searchError.value = "";
  try {
    const page = await sentinelApi.searchSentinelScanPage(projectId, query, currentView, searchPageSize, cursor);
    if (serial !== searchSerial || query !== search.value.trim() || currentView !== view.value || projectId !== props.projectId) return;
    if (!validTaskPage(page, searchPageSize, projectId)) throw new Error("task_search_invalid_page");
    searchResults.value = older ? [...searchResults.value, ...page] : page;
    const last = page[page.length - 1];
    searchCursor.value = last ? { id: last.id, updatedAt: last.updatedAt } : undefined;
    searchHasMore.value = page.length === searchPageSize;
  } catch {
    if (serial === searchSerial) searchError.value = tr("全库搜索失败，请重试", "Searching all tasks failed. Please try again.");
  } finally {
    if (serial === searchSerial) searchLoading.value = false;
  }
}
watch(() => [search.value, view.value, props.projectId] as const, () => {
  ++searchSerial;
  if (searchTimer !== undefined) clearTimeout(searchTimer);
  searchTimer = undefined;
  searchResults.value = [];
  searchCursor.value = undefined;
  searchHasMore.value = false;
  searchError.value = "";
  searchLoading.value = false;
  if (search.value.trim()) searchTimer = setTimeout(() => { searchTimer = undefined; void loadSearchPage(); }, 250);
});
function loadOlderTasks() {
  if (search.value.trim()) void loadSearchPage(true);
  else emit("loadMore");
}
async function toggleArchive(scan: SentinelScan) {
  if (disposed || !scan.projectId || archiveBusy.value
    || (props.projectId !== undefined && scan.projectId !== props.projectId)) return;
  const { id, projectId } = scan;
  const serial = ++archiveSerial;
  const searchVersion = searchSerial;
  archiveBusy.value = id;
  searchError.value = "";
  try {
    const updated = await sentinelApi.archiveSentinelScan(id, projectId, !scan.archivedAt);
    if (serial !== archiveSerial) return;
    if (!updated || updated.id !== id || updated.projectId !== projectId) throw new Error("archive_scope_mismatch");
    if (searchVersion === searchSerial) {
      searchResults.value = searchResults.value.map((item) => item.id === id && item.projectId === projectId ? updated : item);
    }
    emit("archived", updated);
  } catch {
    if (serial === archiveSerial && searchVersion === searchSerial) searchError.value = tr("任务归档更新失败，请刷新后重试", "Couldn't update the task's archive state. Refresh and try again.");
  } finally {
    if (serial === archiveSerial) archiveBusy.value = "";
  }
}
const {
  detailTab, detailLoading, detailErrors, attempts, selectedAttempt,
  runnerLog, runnerLogLoading, runnerLogError, runnerLogConnectionError, mailboxPage, mailboxLoading, mailboxError,
  toolPage, toolLoading, toolError, nativeStatus, historicalPreviews, learningCandidates, knowledge,
  loadTools, loadMailbox, loadAttemptLog, chooseAttempt,
} = useTaskExecutionDetails(props);
const previewPanel = ref<HTMLElement>();
// A row opens the task's results; drafts have none yet, so they open the
// confirmation preview instead.
function openTask(scan: SentinelScan) {
  if (scan.status === "draft") showPreview(scan);
  else emit("open", scan);
}
function showPreview(scan: SentinelScan) {
  emit("preview", scan);
  void nextTick(() => previewPanel.value?.scrollIntoView({ behavior: "smooth", block: "nearest" }));
}
function vulnerabilityText(scan: SentinelScan) {
  if (scan.status === "draft") return "—";
  const count = props.vulnerabilityCount(scan.id);
  return count === undefined ? "—" : formatNumber(count);
}
function vulnerabilityTone(scan: SentinelScan) {
  const count = scan.status === "draft" ? undefined : props.vulnerabilityCount(scan.id);
  return count === undefined ? "idle" : count > 0 ? "found" : "none";
}
function stateTone(status: string) {
  if (status === "completed") return "ok";
  if (["partial", "paused", "pausing", "completed_with_gaps"].includes(status)) return "warn";
  if (["failed", "cancelled", "interrupted"].includes(status)) return "bad";
  if (["queued", "scanning", "running", "pending", "cancel_requested"].includes(status)) return "live";
  return "idle";
}
const historicalStatuses = new Set(["completed", "completed_with_gaps", "cancelled", "failed"]);
const isHistorical = (scan: SentinelScan) => historicalStatuses.has(scan.status);
const historyCount = computed(() => props.scans.filter(isHistorical).length);
const attentionCountLocal = computed(() => props.scans.length - historyCount.value);
const visibleScans = computed(() => (search.value.trim() ? searchResults.value : props.scans).filter((scan) => {
  if (view.value === "history" && !isHistorical(scan)) return false;
  if (view.value === "attention" && isHistorical(scan)) return false;
  return true;
}).sort((left, right) => Number(Boolean(left.archivedAt)) - Number(Boolean(right.archivedAt))));
</script>

<template>
  <div class="task-cost-strip task-center-cost">
      <article><span>{{ tr("当前任务", "Current tasks") }}</span><strong>{{ scans.length }}</strong><small>{{ tr(`${attentionCount} 个需要确认、继续或复盘`, `${attentionCount} need confirming, resuming, or review`) }}</small></article>
      <article>
        <span>{{ tr("已加载任务 Token", "Loaded-task tokens") }}</span>
        <strong>{{ formatCompactNumber(totalTokens) }}</strong>
        <small>{{ tr(`${formatNumber(totalRequests)} 次模型请求`, `${formatNumber(totalRequests)} model requests`) }}</small>
        <small>{{ tr("已加载任务", "Loaded tasks") }} · {{ tokenScopeLabel }}</small>
        <small>{{ tr("用量沿用概览部署范围，不随下方搜索或状态筛选改变。", "Usage follows the overview deployment scope, not the search or status filters below.") }}</small>
      </article>
      <article :class="{ warning: zeroYieldCount }">
        <span>{{ tr("未关联漏洞记录", "No linked finding records") }}</span>
        <strong>{{ zeroYieldCount }}</strong>
        <small>{{ formatNumber(zeroYieldTokens) }} Token · {{ tr("当前索引未关联记录，不代表无漏洞或执行完整。", "No records in the current association index; not proof of no vulnerabilities or complete execution.") }}</small>
      </article>
      <article>
        <span>{{ tr("缓存命中率", "Cache hit rate") }}</span><strong>{{ cacheHitRate }}%</strong>
        <small>{{ tr("缓存输入 / 输入总计；不作为执行质量结论", "Cached input / total input; not an execution-quality verdict") }}</small>
      </article>
  </div>
  <div class="sentinel-queue-layout task-center-stack">
      <section class="panel sentinel-panel">
        <div class="panel-heading">
          <div><p>{{ t.scanning.taskListIntro }}</p></div>
        </div>
        <div class="task-view-controls">
          <div class="segmented" role="group" :aria-label="tr('任务视图', 'Task view')">
            <button type="button" :class="{ active: view === 'attention' }" @click="view = 'attention'">{{ tr("待处理", "Active") }} <span :title="hasMore ? tr('仅已加载任务数量', 'Count of loaded tasks only') : tr('全部任务数量', 'Count of all tasks')">{{ attentionCountLocal }}{{ hasMore ? '+' : '' }}</span></button>
            <button type="button" :class="{ active: view === 'history' }" @click="view = 'history'">{{ tr("已结束", "Ended") }} <span :title="hasMore ? tr('仅已加载任务数量', 'Count of loaded tasks only') : tr('全部任务数量', 'Count of all tasks')">{{ historyCount }}{{ hasMore ? '+' : '' }}</span></button>
            <button type="button" :class="{ active: view === 'all' }" @click="view = 'all'">{{ tr("全部", "All") }} <span :title="hasMore ? tr('仅已加载任务数量', 'Count of loaded tasks only') : tr('全部任务数量', 'Count of all tasks')">{{ scans.length }}{{ hasMore ? '+' : '' }}</span></button>
          </div>
          <input v-model="search" type="search" :aria-label="tr('全库搜索任务', 'Search all tasks')" :placeholder="tr('全库搜索任务、目标或编号', 'Search all tasks, targets, or IDs')" />
        </div>
        <small v-if="searchError" role="alert">{{ searchError }}</small>
        <small v-if="searchLoading" role="status">{{ tr("正在搜索完整任务历史…", "Searching the full task history…") }}</small>
        <div class="task-center-list">
          <article v-for="scan in visibleScans" :key="scan.id" class="task-row" :class="{ active: preview?.id === scan.id, opened: openScanId === scan.id, archived: Boolean(scan.archivedAt) }" @click="openTask(scan)">
            <div class="task-vuln-count" :class="vulnerabilityTone(scan)"><strong>{{ vulnerabilityText(scan) }}</strong><small>{{ t.scanning.vulnerabilities }}</small></div>
            <div class="task-row-main">
              <div class="task-row-title">
                <button type="button" class="task-title-button" :title="scan.id" @click.stop="openTask(scan)">{{ scanTitle(scan) }}</button>
                <span class="task-state" :class="stateTone(scan.status)">{{ statusLabel(scan.status) }}</span><b v-if="scan.archivedAt" class="status-chip">{{ t.scanning.row.archived }}</b><b v-if="highValueCount(scan.id)" class="high-value-badge">{{ fmt(t.scanning.row.high, { count: highValueCount(scan.id) }) }}</b>
              </div>
              <div class="task-row-facts">
                <span>{{ scan.projectName }}</span>
                <span v-if="targetCount(scan.id)">{{ t.scanning.targets }} <b>{{ formatNumber(targetCount(scan.id)) }}</b></span>
                <span>Token <b>{{ formatCompactNumber(scan.totalTokens) }}</b></span>
                <span>{{ scan.updatedAt }}</span>
                <code>#{{ scan.id.slice(0, 8) }}</code>
              </div>
              <em>{{ scanTypeLabel(scan.scanType) }}<template v-if="scan.scanType === 'web'"> · {{ fmt(t.scanning.row.mode, { mode: routeModeLabel(scan.requestedScanMode || 'standard') }) }}</template> · {{ llmDeploymentLabel(scan) }} · {{ latestAttemptLabel(scan, statusLabel) }}</em>
              <div v-if="scanInterruption(scan)" class="task-stop-summary" :class="scanInterruption(scan)?.tone"><b>{{ scanInterruption(scan)?.title }}</b><span>{{ scanInterruption(scan)?.detail }}</span><small>{{ scanInterruption(scan)?.action }}</small></div>
            </div>
            <div class="task-center-actions">
              <button v-if="scan.status === 'draft'" class="button primary compact" :disabled="Boolean(scan.archivedAt)" @click.stop="emit('confirm', scan)"><Play :size="13" />{{ t.scanning.row.confirmScan }}</button>
              <button v-if="scan.status === 'scanning' || scan.status === 'pausing'" class="button warning compact" :disabled="controlBusy === scan.id" @click.stop="emit('pause', scan)"><Pause :size="13" />{{ scan.status === "pausing" ? t.scanning.row.stopping : t.scanning.row.stopKeep }}</button>
              <button v-else-if="scan.status === 'paused' || scan.status === 'partial'" class="button secondary compact" :disabled="controlBusy === scan.id || Boolean(scan.archivedAt)" @click.stop="emit('resume', scan)"><Play :size="13" />{{ scan.status === 'partial' ? t.scanning.row.resumePartial : t.scanning.row.resume }}</button>
              <button v-else-if="scan.status !== 'draft' && !scan.administrativeClosureRecorded" class="button ghost compact" :disabled="controlBusy === scan.id || Boolean(scan.archivedAt)" @click.stop="emit('retry', scan)"><RefreshCw :size="13" />{{ retryActionLabel(scan) }}</button>
              <button v-if="scan.scanType === 'web' && scan.status !== 'draft'" class="button ghost compact" @click.stop="emit('dialog', scan)"><MessageCircle :size="13" />{{ t.scanning.row.run }}</button>
              <button type="button" class="button ghost compact" :aria-pressed="preview?.id === scan.id" @click.stop="showPreview(scan)"><Eye :size="13" />{{ t.scanning.row.details }}</button>
              <button v-if="scan.archivedAt || isHistorical(scan)" class="button ghost compact" :disabled="archiveBusy === scan.id || !scan.projectId" @click.stop="toggleArchive(scan)"><RotateCcw v-if="scan.archivedAt" :size="13" /><Archive v-else :size="13" />{{ scan.archivedAt ? t.scanning.row.unarchive : t.scanning.row.archive }}</button>
              <button v-if="!scan.administrativeClosureRecorded && !scan.closureHandoffRecorded" class="button danger compact" @click.stop="emit('remove', scan)"><Trash2 :size="13" />{{ ["scanning", "pausing"].includes(scan.status) ? t.scanning.row.forceDelete : t.scanning.row.delete }}</button>
            </div>
          </article>
          <div v-if="!visibleScans.length && !searchLoading && !searchError" class="empty-state">{{ search ? tr('全库没有匹配的任务，请换个关键词。', 'No tasks match. Try another keyword.') : scans.length ? tr('当前筛选没有任务，切换视图即可查看历史记录。', 'No tasks in this filter. Switch views to see past tasks.') : tr('暂无任务', 'No tasks yet') }}</div>
          <button v-if="search ? searchHasMore : hasMore" type="button" class="button ghost" :disabled="search ? searchLoading : loadingMore" @click="loadOlderTasks">{{ searchLoading || loadingMore ? tr('正在读取…', 'Loading…') : tr('加载更早任务', 'Load older tasks') }}</button>
        </div>
      </section>
      <section v-if="view !== 'attention'" class="panel sentinel-panel" :aria-label="tr('历史导入产物', 'Imported historical artifacts')">
        <div class="panel-heading">
          <div><h3>{{ tr("历史产物 · 只读", "Historical artifacts · read-only") }}</h3><p>{{ tr("旧 JSON 等产物独立于活动任务展示；候选与覆盖记录未经当前 Reviewer 审核，不计入 Native 已确认漏洞或执行覆盖。", "Old JSON and similar artifacts are shown apart from active tasks. Their candidates and coverage records have not been reviewed by the current Reviewer and do not count toward Native confirmed vulnerabilities or execution coverage.") }}</p></div>
          <button v-refresh-feedback="{busy:importedLoading,error:importedError}" type="button" class="button ghost compact" :disabled="importedLoading" @click="loadImportedRuns()"><RefreshCw :size="13" />{{ tr("刷新", "Refresh") }}</button>
        </div>
        <HistoricalJsonImport @imported="loadImportedRuns(false, true)" />
        <p v-if="importedError" role="alert">{{ importedError }}</p>
        <p v-if="importedLoading && !importedLoaded" role="status">{{ tr("正在读取历史导入记录…", "Loading historical import records…") }}</p>
        <p v-if="importedLoaded && !importedRuns.length">{{ tr("没有可展示的历史导入记录。", "No historical import records to show.") }}</p>
        <p v-if="projectId !== undefined || search.trim()">{{ tr("此清单是全局导入台账，不受上方任务项目筛选或任务搜索影响。", "This list is the global import ledger and ignores the project filter and task search above.") }}</p>
        <div class="task-center-list">
          <article v-for="run in importedRuns" :key="run.rowId" :class="{ active: importedSelected?.rowId === run.rowId }">
            <span class="sentinel-status completed"><Archive :size="15" /></span>
            <div>
              <strong>{{ run.title || run.scanId || tr('未命名历史产物', 'Untitled historical artifact') }}</strong><b class="status-chip">{{ tr("未审核 · 只读", "Unreviewed · read-only") }}</b>
              <small>{{ tr(`${run.scanId} · 记录状态 ${run.status} · 第 ${run.attemptNumber} 次`, `${run.scanId} · Record status ${run.status} · Attempt ${run.attemptNumber}`) }}</small>
              <em>{{ tr(`候选 ${run.findingCandidates} · 覆盖记录 ${run.coverageRecords} · 导入 ${run.importedAt}`, `Candidates ${run.findingCandidates} · Coverage records ${run.coverageRecords} · Imported ${run.importedAt}`) }}</em>
            </div>
            <div class="task-center-actions"><button type="button" class="button ghost compact" @click="selectImportedRun(run)"><Eye :size="13" />{{ importedSelected?.rowId === run.rowId ? tr('收起预览', 'Hide preview') : tr('查看摘要', 'View summary') }}</button></div>
          </article>
        </div>
        <button v-if="importedHasMore" type="button" class="button ghost compact" :disabled="importedLoading" @click="loadImportedRuns(true)">{{ tr("加载更早的导入产物", "Load older imported artifacts") }}</button>
        <section v-if="importedSelected" class="task-attempt-log" :aria-label="tr('历史产物只读摘要', 'Read-only summary of historical artifacts')">
          <header><strong>{{ tr(`${importedSelected.title || importedSelected.scanId} · 只读摘要`, `${importedSelected.title || importedSelected.scanId} · Read-only summary`) }}</strong></header>
          <p v-if="importedPreviewLoading" role="status">{{ tr("正在读取脱敏摘要…", "Loading redacted summaries…") }}</p>
          <p v-else-if="!importedPreviews.length">{{ tr("暂无当前可展示记录；已撤销的投影不会显示。", "No records to show right now; revoked projections are not shown.") }}</p>
          <ol v-else class="task-detail-records task-mailbox-records">
            <li v-for="item in importedPreviews" :key="item.membershipId"><strong>{{ item.title || item.kind }}{{ item.severity ? ` · ${item.severity}` : '' }}</strong><span v-if="item.target">{{ item.target }}</span><small>{{ tr(`${item.producer} · ${item.kind} · 未审核 / 只读`, `${item.producer} · ${item.kind} · Unreviewed / read-only`) }}</small></li>
          </ol>
          <small v-if="importedPreviews.length === 300">{{ tr("这里只展示前 300 条脱敏摘要；完整清单仍保留在导入台账中。", "Only the first 300 redacted summaries are shown here; the full list stays in the import ledger.") }}</small>
        </section>
      </section>
      <aside v-if="preview" ref="previewPanel" class="panel task-preview task-preview-inline">
          <header><div><h3>{{ scanTitle(preview) }}</h3><div class="task-preview-facts"><span class="status-chip" :class="preview.latestAttemptStatus || preview.status">{{ latestAttemptLabel(preview, statusLabel) }}</span><span>{{ scanTypeLabel(preview.scanType) }}</span><span v-if="preview.scanType === 'web'">{{ tr(`任务模式：${routeModeLabel(preview.requestedScanMode || 'standard')}`, `Task mode: ${routeModeLabel(preview.requestedScanMode || 'standard')}`) }}</span><code :title="preview.id">{{ preview.id }}</code></div></div><button class="icon-button" type="button" :aria-label="tr('关闭任务详情', 'Close task details')" @click="emit('close')"><X :size="15" /></button></header>
          <div v-if="detailTab === 'overview'" class="preview-url-list">
            <strong>{{ tr(`任务目标 · ${previewTargets.length}`, `Task targets · ${previewTargets.length}`) }}</strong>
            <div v-for="row in previewTargets" :key="row.url" class="preview-target-row"><span>{{ row.company }}</span><code>{{ row.url }}</code><b v-if="row.highValue" class="high-value-badge">{{ tr("高", "High") }}</b></div>
            <div v-if="!previewTargets.length" class="empty-state small">{{ tr("没有可预览目标", "No targets to preview") }}</div>
          </div>
          <div v-if="detailTab === 'overview' && scanInterruption(preview)" class="task-stop-summary preview-stop-summary" :class="scanInterruption(preview)?.tone"><b>{{ scanInterruption(preview)?.title }}</b><span>{{ scanInterruption(preview)?.detail }}</span><small>{{ scanInterruption(preview)?.action }}</small></div>
          <p v-else-if="detailTab === 'overview'" class="task-preview-summary">{{ scanSummary(preview) || (preview.status === 'completed' ? tr('本轮已结束，证据、执行尝试与协作消息仍保存在任务中。', 'This run has ended; its evidence, attempts, and collaboration messages remain saved in the task.') : tr('尚无执行摘要。', 'No run summary yet.')) }}</p>
          <AuthorizationControlForm v-if="detailTab === 'overview' && preview.status === 'draft' && preview.scanType === 'web'" :scan-id="preview.id" :targets="previewTargets" />
          <details v-if="detailTab === 'overview'" class="task-technical"><summary>{{ tr("任务文件与运行信息", "Task files and run info") }}</summary><code>{{ preview.taskPath || tr("确认扫描后生成", "Generated after the scan is confirmed") }}</code></details>
          <footer>
            <button v-if="preview.status === 'draft'" class="button primary" @click="emit('confirm', preview)"><Play :size="14" />{{ tr("确认并启动扫描", "Confirm and start scan") }}</button>
            <button class="button ghost" @click="emit('open', preview)"><Eye :size="14" />{{ tr("打开结果页", "Open results page") }}</button>
            <button v-if="preview.scanType === 'web' && preview.status !== 'draft'" class="button ghost" @click="emit('dialog', preview)"><MessageCircle :size="14" />{{ tr("查看执行记录", "View run records") }}</button>
          </footer>
          <div class="task-detail-tabs" role="tablist" :aria-label="tr('单任务详情', 'Task details')">
            <button v-for="tab in (['overview', 'execution', 'evidence', 'learning'] as const)" :key="tab" type="button" role="tab" :aria-selected="detailTab === tab" :class="{ active: detailTab === tab }" @click="detailTab = tab">{{ { overview: tr('概况', 'Overview'), execution: tr('执行', 'Execution'), evidence: tr('证据', 'Evidence'), learning: tr('学习', 'Learning') }[tab] }}</button>
          </div>
          <div class="task-detail-body" role="tabpanel">
            <p v-if="detailLoading">{{ tr("正在读取任务记录…", "Loading task records…") }}</p>
            <p v-if="detailErrors.length" class="task-detail-error" role="alert">{{ detailErrors.join(tr('；', '; ')) }}</p>
            <template v-if="detailTab === 'overview'">
              <p>{{ tr(`任务编号 ${preview.id} · ${attempts.length} 次执行 · ${formatNumber(preview.totalTokens)} Token。切换标签不会创建或重启任务。`, `Task ID ${preview.id} · ${attempts.length} attempts · ${formatNumber(preview.totalTokens)} tokens. Switching tabs does not create or restart the task.`) }}</p>
              <p v-if="nativeStatus?.stopDiagnostic && nativeStatus.stopDiagnostic.code !== 'in_progress'">{{nativeRunLabels.diagnosticLabel(nativeStatus.stopDiagnostic.code)}} · {{nativeRunLabels.actionLabel(nativeStatus.stopDiagnostic.nextAction)}}</p>
            </template>
            <template v-else-if="detailTab === 'execution'">
              <p v-if="!detailLoading && !attempts.length">{{ tr("尚无执行尝试；草稿任务不会产生运行记录。", "No attempts yet; draft tasks do not produce run records.") }}</p>
              <ol v-else class="task-detail-records">
                <li v-for="attempt in attempts" :key="attempt.attemptNumber"><strong>{{ tr(`第 ${attempt.attemptNumber} 次 · ${statusLabel(attempt.status)}`, `Attempt ${attempt.attemptNumber} · ${statusLabel(attempt.status)}`) }}</strong><span>{{ attempt.executionMode }} · {{ attempt.stage }} · {{ attempt.checkpoint || tr('无检查点', 'No checkpoint') }}</span><small>{{ attempt.startedAt }} → {{ attempt.finishedAt || tr('进行中', 'In progress') }} · {{ attempt.stopReason || tr('无停止原因', 'No stop reason') }}</small><small>{{ tr(`${formatNumber(attempt.totalTokens)} Token · ${attempt.llmRequests} 次模型请求`, `${formatNumber(attempt.totalTokens)} tokens · ${attempt.llmRequests} model requests`) }}</small><button type="button" class="button ghost compact" :aria-pressed="selectedAttempt === attempt.attemptNumber" @click="chooseAttempt(attempt.attemptNumber)">{{ selectedAttempt === attempt.attemptNumber ? tr('当前查看此轮', 'Viewing this attempt') : tr('查看此轮日志', 'View attempt log') }}</button></li>
              </ol>
              <section v-if="selectedAttempt !== undefined" class="task-attempt-log" :aria-label="tr('选中执行轮次的日志', 'Selected attempt log')">
                <header><strong>{{ tr(`第 ${selectedAttempt} 次执行 · 日志尾部`, `Attempt ${selectedAttempt} · Log tail`) }}</strong><button v-refresh-feedback="{busy:runnerLogLoading,error:runnerLogError}" type="button" class="button ghost compact" :disabled="runnerLogLoading" @click="preview && loadAttemptLog(preview.id, selectedAttempt)">{{ tr("刷新日志", "Refresh log") }}</button></header>
                <p v-if="runnerLogConnectionError" role="status">{{ runnerLogConnectionError }}</p>
                <p v-if="runnerLogLoading && !runnerLog">{{ tr("正在读取此轮日志…", "Loading this attempt's log…") }}</p>
                <p v-if="runnerLogError" role="alert">{{ runnerLogError }}</p>
                <template v-if="runnerLog">
                  <small>{{ runnerLog.source === 'attempt_work_dir' ? tr('此轮独立目录', 'Attempt directory') : tr('当前任务目录', 'Current task directory') }} · {{ runnerLog.status }} · {{ runnerLog.updatedAt }}</small>
                  <p v-if="runnerLog.status !== 'ready'">{{ runnerLog.message || tr('此轮没有可展示的日志', 'No log to show for this attempt') }}</p>
                  <pre v-else>{{ runnerLog.lines.join('\n') }}</pre>
                  <small v-if="runnerLog.status === 'ready'">{{ tr("最多显示末尾 200 行；完整证据以原结果页与持久化记录为准。", "Shows up to the last 200 lines; for full evidence, the original results page and saved records are authoritative.") }}</small>
                </template>
                <NativeProcessLogView v-if="preview" :scan-id="preview.id" :attempt="selectedAttempt" :active="detailTab==='execution'" />
                <NativeSdkLogView v-if="preview" :scan-id="preview.id" :attempt="selectedAttempt" :active="detailTab==='execution'" />
              </section>
              <section v-if="selectedAttempt !== undefined" class="task-attempt-log" :aria-label="tr('选中执行轮次的智能体通信', 'Selected attempt agent messages')">
                <header><strong>{{ tr(`第 ${selectedAttempt} 次执行 · 智能体通信`, `Attempt ${selectedAttempt} · Agent messages`) }}</strong><button v-refresh-feedback="{busy:mailboxLoading,error:mailboxError}" type="button" class="button ghost compact" :disabled="mailboxLoading" @click="preview && loadMailbox(preview.id, selectedAttempt)">{{ tr("刷新消息", "Refresh messages") }}</button></header>
                <p v-if="mailboxLoading && !mailboxPage">{{ tr("正在读取此轮消息…", "Loading this attempt's messages…") }}</p>
                <p v-if="mailboxError" role="alert">{{ mailboxError }}</p>
                <p v-if="mailboxPage && !mailboxPage.messages.length">{{ tr("此轮没有已保存的智能体通信；旧任务可能没有多智能体记录。", "No saved agent messages for this attempt; older tasks may have no multi-agent records.") }}</p>
                <ol v-if="mailboxPage?.messages.length" class="task-detail-records task-mailbox-records">
                  <li v-for="message in mailboxPage.messages" :key="message.id">
                    <strong>{{ message.fromRole }} → {{ message.toRole }} · {{ message.kind }}</strong>
                    <span>{{ message.summary }}</span>
                    <small>{{ tr(`${message.createdAt} · ${message.deliveryState} / ${message.ackState} · 证据版本 ${message.evidenceRevision}`, `${message.createdAt} · ${message.deliveryState} / ${message.ackState} · Evidence revision ${message.evidenceRevision}`) }}</small>
                    <small v-if="message.correlationId">{{ tr(`话题 ${message.correlationId} · 分派 ${message.assignmentId || '无'}`, `Thread ${message.correlationId} · Assignment ${message.assignmentId || 'none'}`) }}</small>
                  </li>
                </ol>
                <button v-if="mailboxPage?.hasOlder" type="button" class="button ghost compact" :disabled="mailboxLoading" @click="preview && loadMailbox(preview.id, selectedAttempt, true)">{{ tr("读取更早的消息", "Load older messages") }}</button>
                <small>{{ tr("只展示此轮已落库的脱敏消息摘要。证据缺口提案与评估不等于已派发补证或已生成新事实；须以新证据版本和 Reviewer 复审结果为准。人工指令与完整证据请到协作页或原结果页核对。", "Only redacted summaries of messages saved for this attempt are shown. Evidence gap proposals and assessments do not mean follow-up evidence was dispatched or new facts were produced; rely on new evidence revisions and Reviewer re-review results. Check manual instructions and full evidence on the collaboration page or the original results page.") }}</small>
              </section>
              <section v-if="selectedAttempt !== undefined" class="task-attempt-log" :aria-label="tr('选中执行轮次的工具调用', 'Selected attempt tool calls')">
                <header><strong>{{ tr(`第 ${selectedAttempt} 次执行 · 工具调用`, `Attempt ${selectedAttempt} · Tool calls`) }}</strong><button v-refresh-feedback="{busy:toolLoading,error:toolError}" type="button" class="button ghost compact" :disabled="toolLoading" @click="preview && loadTools(preview.id, selectedAttempt)">{{ tr("刷新工具记录", "Refresh tool records") }}</button></header>
                <p v-if="toolLoading && !toolPage">{{ tr("正在读取此轮工具记录…", "Loading this attempt's tool records…") }}</p>
                <p v-if="toolError" role="alert">{{ toolError }}</p>
                <p v-if="toolPage && !toolPage.invocations.length">{{ tr("此轮没有已保存的工具调用。", "No saved tool calls for this attempt.") }}</p>
                <ol v-if="toolPage?.invocations.length" class="task-detail-records task-mailbox-records">
                  <li v-for="call in toolPage.invocations" :key="call.id">
                    <strong>{{ call.role }} · {{ call.toolName }} · {{ call.status }}</strong>
                    <template v-if="call.origin === 'source_receipt'">
                      <small>{{ tr(`源码工具计划记录于 ${call.recordedAt}（模型响应落库时间，不是执行开始时间）`, `Source tool plan recorded at ${call.recordedAt} (when the model response was saved, not when execution started)`) }}</small>
                      <small v-if="call.status === 'planned'">{{ tr("计划待执行 · 尚无执行完成回执，不代表正在执行。", "Planned, not yet run · No completion receipt yet; this does not mean it is running.") }}</small>
                      <small v-else-if="call.receiptIntegrity === 'unverified'">{{ tr("回执无法核对 · 不计为已完成，也不能据此重放。", "Receipt can't be verified · Not counted as completed and can't be replayed from it.") }}</small>
                      <small v-else>{{ tr(`完成回执 ${call.finishedAt}`, `Completion receipt ${call.finishedAt}`) }} · {{ call.status === 'refused' ? tr('工具拒绝，未获得成功结果', 'Tool refused; no successful result') : tr('本地回执一致，不等于独立审查通过', 'Local receipt matches; not an independent review pass') }}</small>
                      <small v-if="call.resultKind === 'control'">{{ tr("阶段结束标记，不计为漏洞证据。", "Stage end marker; not counted as vulnerability evidence.") }}</small>
                    </template>
                    <small v-else>{{ tr(`${call.startedAt} → ${call.finishedAt || '尚无终态'} · 策略 ${call.policyDecision || '未记录'}`, `${call.startedAt} → ${call.finishedAt || 'No final state yet'} · Policy ${call.policyDecision || 'Not recorded'}`) }}</small>
                    <small>{{ tr(`请求证据 ${call.hasRequestArtifact ? '已登记' : '无引用'} · 响应证据 ${call.hasResponseArtifact ? '已登记' : '无引用'}`, `Request evidence ${call.hasRequestArtifact ? 'registered' : 'no reference'} · Response evidence ${call.hasResponseArtifact ? 'registered' : 'no reference'}`) }}</small>
                    <span v-if="call.errorClass">{{ tr(`失败类型：${call.errorClass}`, `Failure type: ${call.errorClass}`) }}</span>
                    <small>{{ tr(`运行 ${call.runId} · 调用 ${call.invocationId}`, `Run ${call.runId} · Call ${call.invocationId}`) }}</small>
                  </li>
                </ol>
                <button v-if="toolPage?.hasOlder" type="button" class="button ghost compact" :disabled="toolLoading" @click="preview && loadTools(preview.id, selectedAttempt, true)">{{ tr("读取更早的调用", "Load older calls") }}</button>
                <small>{{ tr("这里只显示持久化状态和证据引用是否存在；“已登记”不等于已审核或可安全重放。不会展示工具输入、响应正文或认证材料。", "Only the saved status and whether evidence references exist are shown here; “registered” does not mean reviewed or safe to replay. Tool inputs, response bodies, and credentials are never shown.") }}</small>
              </section>
              <p v-if="nativeStatus?.stopDiagnostic?.obligations.length && selectedAttempt === nativeStatus.attemptNumber">{{ tr("当前轮次待核对事项：", "Items to verify for this attempt: ") }}{{ nativeStatus.stopDiagnostic.obligations.map(item => `${item.kind}: ${item.status}`).join(tr('；', '; ')) }}{{ nativeStatus.stopDiagnostic.obligationsTruncated ? tr('（还有未展示项）', ' (more not shown)') : '' }}</p>
            </template>
            <template v-else-if="detailTab === 'evidence'">
              <template v-if="preview.scanType !== 'web'">
                <label v-if="attempts.length">{{ tr("查看源码审查轮次", "Source review attempt") }}
                  <select :value="selectedAttempt" :aria-label="tr('源码审查执行轮次', 'Source review attempt')" @change="chooseAttempt(Number(($event.target as HTMLSelectElement).value))"><option v-for="attempt in attempts" :key="attempt.attemptNumber" :value="attempt.attemptNumber">{{ tr(`第 ${attempt.attemptNumber} 次 · ${statusLabel(attempt.status)}`, `Attempt ${attempt.attemptNumber} · ${statusLabel(attempt.status)}`) }}</option></select>
                </label>
                <SourceReviewEvidence v-if="selectedAttempt !== undefined" :scan-id="preview.id" :attempt-number="selectedAttempt" />
              </template>
              <p v-if="preview.scanType === 'web' && nativeStatus">{{ tr(`候选 ${nativeStatus.findingCandidateCount} 条 · 审核 ${nativeStatus.reviewStatus} · ${nativeStatus.reviewGateSatisfied ? '审查门禁已满足' : '审查门禁未满足或不适用'}。`, `${nativeStatus.findingCandidateCount} candidates · Review ${nativeStatus.reviewStatus} · ${nativeStatus.reviewGateSatisfied ? 'Review gate satisfied' : 'Review gate not satisfied or not applicable'}.`) }}</p>
              <section v-if="historicalPreviews.length" class="task-attempt-log" :aria-label="tr('历史导入只读预览', 'Read-only preview of historical imports')">
                <header><strong>{{ tr("历史产物 · 只读预览", "Historical artifacts · read-only preview") }}</strong></header>
                <p>{{ tr(`以下 ${historicalPreviews.length} 条来自旧 JSON 等产物，均未经过当前独立 Reviewer 审核，不能当作已确认漏洞或新任务授权。仅展示脱敏摘要，完整原件不在此处开放。`, `The ${historicalPreviews.length} items below come from old JSON and similar artifacts. None has been reviewed by the current independent Reviewer, so they can't be treated as confirmed vulnerabilities or as authorization for new tasks. Only redacted summaries are shown; the full originals are not available here.`) }}</p>
                <ol class="task-detail-records task-mailbox-records">
                  <li v-for="item in historicalPreviews" :key="item.membershipId">
                    <strong>{{ item.title || item.kind }}{{ item.severity ? ` · ${item.severity}` : '' }}</strong>
                    <span v-if="item.target">{{ item.target }}</span>
                    <small>{{ tr(`${item.producer} · ${item.kind} · 第 ${item.attemptNumber} 次记录 · 未审核 / 只读`, `${item.producer} · ${item.kind} · Attempt ${item.attemptNumber} record · Unreviewed / read-only`) }}</small>
                  </li>
                </ol>
                <small v-if="historicalPreviews.length === 300">{{ tr("最多展示 300 条；其余历史产物仍保留在导入记录中，此处不是完整清单。", "Shows up to 300 items; the remaining historical artifacts stay in the import records, so this is not the full list.") }}</small>
              </section>
              <p>{{ tr("事实、反证、接口与漏洞保留在同一任务的结果页；这里不复制第二份结论，也不把待补证当作已确认。", "Facts, counter-evidence, endpoints, and vulnerabilities stay on this task's results page; this view does not copy a second verdict or treat items awaiting evidence as confirmed.") }}</p>
              <button type="button" class="button ghost compact" @click="emit('open', preview)">{{ tr("查看原始证据与历史结果", "View original evidence and historical results") }}</button>
            </template>
            <template v-else>
              <p>{{ tr("这里只列出本任务已保存的候选与知识；不会自动创建永久 Skill。跨任务复用需独立复核与容量治理。", "Only candidates and knowledge saved for this task are listed here; no permanent Skill is created automatically. Reuse across tasks needs independent review and capacity governance.") }}</p>
              <p v-if="!detailLoading && !learningCandidates.length && !knowledge.length">{{ tr("本任务暂无可展示的学习记录，不代表已经完成分析。", "No learning records to show for this task yet; this does not mean the analysis is complete.") }}</p>
              <div class="task-detail-records" v-if="learningCandidates.length || knowledge.length">
                <article v-for="item in learningCandidates" :key="`candidate-${item.id}`"><strong>{{ tr(`候选 · ${item.title} · ${item.status}`, `Candidate · ${item.title} · ${item.status}`) }}</strong><p>{{ item.summary }}</p><small>{{ tr(`${item.updatedAt} · 来源任务 ${item.scanId}`, `${item.updatedAt} · Source task ${item.scanId}`) }}</small></article>
                <article v-for="item in knowledge" :key="`knowledge-${item.id}`"><strong>{{ tr(`知识 · ${item.title}${item.skillId ? ' · 已关联技能' : ''}`, `Knowledge · ${item.title}${item.skillId ? ' · Linked skill' : ''}`) }}</strong><p>{{ item.summary }}</p><small>{{ tr(`${item.updatedAt} · 来源任务 ${item.scanId}`, `${item.updatedAt} · Source task ${item.scanId}`) }}</small></article>
              </div>
            </template>
          </div>
      </aside>
  </div>
</template>

<style scoped src="./taskCenterPresentation.css"></style>
