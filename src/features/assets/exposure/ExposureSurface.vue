<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { Download, ExternalLink, Plus, Search, ShieldAlert, Pause, X, Bot, RefreshCw } from "@lucide/vue";
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { assetApi } from "../api";
import { exposureApi } from "./api";
import type { Project } from "../../workspaces/types";
import type { ExposureFindingDraft } from "./contracts";
import { buildGradeRequest, intelGradeLine, observationTlsWarning, observationHttpWarning, sourceStatusView } from "./contracts";
import { useExposureSurface } from "./useExposureSurface";
import { useExposureEvidence } from "./useExposureEvidence";
import { submitAssetsFormOnEnter } from "../formInteraction";
import { useI18n } from "../uiPorts";
import ExposureSettingsPanel from "./ExposureSettingsPanel.vue";
import ExposureClustersPanel from "./ExposureClustersPanel.vue";
import ExposureReviewPanel from "./ExposureReviewPanel.vue";
import ExposureIncidentsPanel from "./ExposureIncidentsPanel.vue";
import ExposureCoverageDrill from "./ExposureCoverageDrill.vue";
import { INFO_CATEGORY_LABELS, coverageNotice, coverageRows, liveRunLines, readRunRecovery, newOperationId, unitStatisticsLines } from "./incidents";
import "../../../exposure-surface.css";
import "../../../exposure-runs.css";
import "../../../exposure-tabs.css";

const props = defineProps<{ projects: Project[]; selectedProjectId?: number; agentAvailable?: boolean }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; analyze:[value:{projectId:number;prompt:string}] }>();
const { tr } = useI18n();

const project = computed(() => props.projects.find((item) => item.id === props.selectedProjectId));
/// The exposure surface talks to its own IPC wrapper; only the asset ledger read for the
/// bound-asset picker comes directly from the Assets owner, without modifying it.
const surfaceApi = { ...exposureApi, listAssets: assetApi.listAssets };
const surface = useExposureSurface({
  projectId: () => props.selectedProjectId,
  active: () => true,
  api: surfaceApi,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  summary, findings, runs, runsTotal, hasMoreRuns, loadingRunsMore, loadMoreRuns, exporting, sources, boundAssets, selectedRunId, statusFilter, searchFilter,
  findingsTotal, hasMore, loading, loadingMore, stale, readError, hasSnapshot, starting,
  boundAssetsTotal, boundAssetsTruncated, activeRun, scopeNotice, beyondPlanNotice, limitsNotice, unitLines,
  lastExport, cancelling, resultsUpdated,
  dispositionFilter, dispositionFilters, badgeFor, classificationNotice,
  windowNotice,
  load, loadMore, showSourcesFor, startScan, cancelScan, save, exportAll,
  loadBoundAssets,
} = surface;

const evidence = useExposureEvidence({
  projectId: () => props.selectedProjectId,
  active: () => true,
  api: exposureApi,
  onResumed: surface.loadFresh,
  notify: (kind, text) => emit("notify", kind, text),
});
const {
  pending, pendingLoading, pendingLoadingMore, pendingNotice, evidence: evidencePackage, evidenceFor, resuming,
  evidenceLoading, headline, reviewNeeded, resumeRun,
  loadPending, loadMorePending, openEvidence, closeEvidence, pendingError, evidenceError,
} = evidence;

const view = ref<"overview" | "runs" | "findings" | "sources" | "pending" | "clusters" | "review" | "incidents" | "coverage" | "settings">("overview");
/// G4-2 refreshed with the summary on every poll while a round is active; G4-1 matrix and unit lines.
const liveLines = computed(() => (summary.value ? liveRunLines(summary.value.liveRun, summary.value.liveRunError ?? "") : []));
const coverageView = computed(() => (summary.value ? coverageRows(summary.value.coverage) : []));
/// P1 覆盖矩阵钻取：点哪一行就钻取哪个来源（时间窗口、单位、工作项明细在钻取面板里）。
const drillSource = ref("");
const coverageLine = computed(() => (summary.value ? coverageNotice(summary.value.coverage, summary.value.coverageError ?? "") : ""));
const unitStatLines = computed(() => (summary.value ? unitStatisticsLines(summary.value.unitStatistics, summary.value.unitStatisticsError ?? "") : []));
const latestLiveRunId = computed(() => {
  const live = summary.value?.liveRun as { runId?: unknown } | null | undefined;
  return typeof live?.runId === "number" ? live.runId : null;
});
const resumableLive = computed(() => {
  const live = summary.value?.liveRun as { runId?: number; recovery?: unknown } | null | undefined;
  return live && readRunRecovery(live)?.resumable === true && typeof live.runId === "number" ? live.runId : null;
});
// §E5 the review panel works on one finding picked from the page already on screen.
const reviewFindingId = ref<number | undefined>(undefined);
const showAdd = ref(false);
const saving = ref(false);
const blankForm = () => ({ assetId: undefined as number | undefined, category: "source_code", title: "", sourceType: "public_web", sourceUrl: "", evidenceExcerpt: "", severity: "medium", confidence: 70 });
const form = ref(blankForm());
let projectGeneration=0;
watch(showAdd,(open)=>{if(open)void loadBoundAssets();});
function askAgent(findingId?:number):void {
 const projectId=props.selectedProjectId;if(!projectId||!props.agentAvailable)return;
 // Only owner-issued IDs enter the draft. Evidence and URLs stay behind scoped tools.
 const focus=findingId?tr(`重点读取当前页面所选 findingId=${findingId} 的元数据，`, `Focus on the metadata of the finding selected on this page (findingId=${findingId}). `):tr('分页查看当前项目的暴露线索与任务状态，', "Page through this project's exposure leads and task status. ");
 emit('analyze',{projectId,prompt:tr(`请只读复核当前工作空间的公开暴露面。${focus}从本项目真实任务编号读取 exposure_task_status 的 collectionEvidence，核对保存的响应、传输失败与证书异常计数，即使没有线索也要说明采集缺口。按严重性、当前准入状态、是否缺少证据和后续处置优先级整理建议。只引用当前项目工具实际返回的记录，注明历史观察与当前状态的区别；查不到就说明缺口。核对 run.recovery 中的真实断点、待续采 URL、剩余请求/字节与未知在途请求，不能仅凭 partial、cancelled 或 interrupted 就声称可恢复。证书异常或同域 HTTP 下采集的响应身份未经验证；HTTP 传输未加密，不能据此确认归属；HTTP 200 不等于泄露，零线索不等于安全。不要把线索直接确认为泄露，不试用凭据，不修改复核或整改结论，不启动、恢复采集，不联网，也不扩大范围。若元数据不足，列出应在证据面板核对的项目，不猜测正文。`, `Do a read-only review of the current workspace's public exposure. ${focus}Using this project's real task IDs, read collectionEvidence from exposure_task_status and check the counts of saved responses, transport failures and certificate anomalies; describe collection gaps even if there are no leads. Organize recommendations by severity, current eligibility status, missing evidence and follow-up handling priority. Cite only records actually returned by this project's tools, and distinguish historical observations from current state; if something cannot be found, report the gap. Check the real checkpoint, URLs pending collection, remaining requests/bytes and unknown in-flight requests in run.recovery; do not claim a run is resumable just because it is partial, cancelled or interrupted. Responses collected with certificate anomalies or over same-host HTTP have unverified identity; HTTP is unencrypted and cannot confirm ownership; HTTP 200 does not mean a leak, and zero leads does not mean safe. Do not confirm a lead as a leak directly, do not try credentials, do not change review or remediation decisions, do not start or resume collection, do not go online, and do not expand scope. If metadata is insufficient, list what should be checked in the evidence panel instead of guessing the content.`)});
}
const categoryLabel = (value: string) =>
  ({ source_code: tr("源代码泄漏", "Source code leak"), document: tr("内部文档", "Internal document"), credential: tr("凭据线索", "Credential lead"), storage: tr("公开存储", "Public storage"), repository: tr("代码仓库", "Code repository"), metadata: tr("敏感元数据", "Sensitive metadata"), other: tr("其他", "Other") } as Record<string, string>)[value] || value;
// G1-0: the caps come from the project's own settings, with their basis named.
const limitsLine = computed(() => limitsNotice.value);

async function submit(): Promise<void> {
  if (!props.selectedProjectId || saving.value || project.value?.status==='archived') return;
  const generation=projectGeneration;
  saving.value = true;
  const input: ExposureFindingDraft = { ...form.value, projectId: props.selectedProjectId };
  if (await save(input)) {
    if(generation!==projectGeneration)return;
    showAdd.value = false;
    form.value = blankForm();
    emit("notify", "success", tr("公开暴露线索已归并保存，仍需人工复核", "Public exposure lead merged and saved; it still needs manual review"));
  }
  if(generation===projectGeneration)saving.value = false;
}

// 换工作空间时必须丢掉上一个项目的证据与待归并队列，晚到的响应由 composable 的代号拦截。
watch(
  () => props.selectedProjectId,
  () => {
    projectGeneration++;showAdd.value=false;saving.value=false;form.value=blankForm();reviewFindingId.value=undefined;
    gradeNotice.value='';gradeBusy.value=false;gradeActor.value='';gradeReason.value='';gradeSample.value='';gradeEntity.value='';
    closeEvidence();
    if (view.value === "pending") void loadPending();
  },
);

async function openTab(next: typeof view.value): Promise<void> {
  view.value = next;
  if (next === "pending" && !pending.value) await loadPending();
}

async function showEvidence(findingId: number): Promise<void> {
  await openEvidence(findingId);
}

async function chooseRun(event: Event): Promise<void> {
  const raw = (event.target as HTMLSelectElement).value;
  await showSourcesFor(raw === "" ? null : Number(raw));
}

async function openRunSources(runId: number): Promise<void> {
  view.value = "sources";
  await showSourcesFor(runId);
}

/// G0/P1-4: a finding's review is written only through the scoped panel (revision, operator
/// and reason); the list button just opens that panel on the finding already on screen.
async function openReview(findingId: number): Promise<void> {
  reviewFindingId.value = findingId;
  await openTab("review");
}

/// §E5 the review panel only ever works on a finding that is already on screen.
function pickReviewFinding(event: Event): void {
  const raw = (event.target as HTMLSelectElement).value;
  reviewFindingId.value = raw === "" ? undefined : Number(raw);
}


const gradeActor = ref("");
const gradeReason = ref("");
const gradeChoice = ref("claim_only");
const gradeSample = ref("");
const gradeEntity = ref("");
const gradeBusy = ref(false);
const gradeNotice = ref("");
let gradeEpoch = 0;
watch(() => [props.selectedProjectId, evidenceFor.value], () => {
  gradeEpoch += 1; gradeBusy.value = false; gradeNotice.value = "";
  gradeReason.value = ""; gradeSample.value = ""; gradeEntity.value = ""; gradeChoice.value = "claim_only";
}, { flush: "sync" });

function positiveId(raw: string): number | undefined {
  const text = raw.trim();
  if (!/^[1-9][0-9]*$/.test(text)) return undefined;
  return Number(text);
}

async function submitGrade(): Promise<void> {
  const pkg = evidencePackage.value;
  const projectId = props.selectedProjectId;
  if (!pkg || !projectId || !pkg.intelGrade || gradeBusy.value || project.value?.status==='archived') return;
  const generation=projectGeneration, ticket=gradeEpoch;
  const built = buildGradeRequest({
    findingId: pkg.finding.id,
    expectedGrade: pkg.intelGrade,
    grade: gradeChoice.value,
    actor: gradeActor.value,
    reason: gradeReason.value,
    sampleObservationId: positiveId(gradeSample.value),
    entityId: positiveId(gradeEntity.value),
  }, newOperationId());
  if (!built.ok) {
    gradeNotice.value = built.error;
    return;
  }
  gradeBusy.value = true;
  try {
    await exposureApi.gradeExposureIntel(projectId, built.payload);
    if(generation!==projectGeneration || ticket!==gradeEpoch)return;
    gradeNotice.value = tr("已记录改级（不会自动关闭线索，也不会访问来源）", "Regrade recorded (the lead is not closed automatically and the source is not visited)");
    gradeReason.value = "";
    await openEvidence(pkg.finding.id);
  } catch (reason) {
    if(generation!==projectGeneration || ticket!==gradeEpoch)return;
    gradeNotice.value = tr(`改级未写入：${reason instanceof Error ? reason.message : String(reason)}`, `Regrade not saved: ${reason instanceof Error ? reason.message : String(reason)}`);
  } finally {
    if(generation===projectGeneration && ticket===gradeEpoch)gradeBusy.value = false;
  }
}
</script>

<template>
  <div class="exposure-surface">
    <div class="section-toolbar">
      <div>
        <p>{{ tr(`${scopeNotice}；只读取已通过归属与授权门禁的公开资产，证书透明度与历史归档只提供线索。`, `${scopeNotice}. Only public assets that passed the ownership and authorization gates are read; certificate transparency and historical archives only provide leads.`) }}</p>
        <small class="exposure-limits">{{ limitsLine }}</small>
        <small v-if="lastExport" class="exposure-limits">{{ lastExport }}</small>
        <p v-if="windowNotice" class="exposure-gap">{{ windowNotice }}</p>
      </div>
      <div>
        <button v-if="agentAvailable" class="button secondary" :disabled="!project" @click="askAgent()"><Bot :size="15"/>{{ tr('AI 梳理 · 前往对话', 'AI triage · open chat') }}</button>
        <button v-refresh-feedback="{scope:selectedProjectId,error:readError}" class="button ghost" :disabled="!project||loading" @click="load"><RefreshCw :size="15"/>{{loading?tr('正在刷新…', 'Refreshing…'):tr('刷新', 'Refresh')}}</button>
        <button v-if="activeRun" class="button danger" :disabled="cancelling||activeRun.status==='cancel_requested'" @click="cancelScan"><Pause :size="14" />{{cancelling||activeRun?.status==='cancel_requested'?tr('正在保存断点…', 'Saving checkpoint…'):tr('暂停采集', 'Pause collection')}}</button>
        <button v-else-if="resumableLive !== null" class="button primary" :disabled="resuming !== null || stale" @click="resumeRun(resumableLive)">{{resuming === resumableLive ? tr('正在恢复…', 'Resuming…') : tr('继续断点采集', 'Resume from checkpoint')}}</button>
        <button v-else class="button primary" :disabled="!project || !hasSnapshot || project.status==='archived' || starting || stale || summary?.eligibleAssetsPlanned === 0" :title="tr(`被动采集受上限约束：${limitsLine}`, `Passive collection is capped: ${limitsLine}`)" @click="startScan">
          <Search :size="15" />{{ starting ? tr("正在启动…", "Starting…") : loading && !hasSnapshot ? tr("正在核对采集范围…", "Checking collection scope…") : summary?.eligibleAssetsPlanned === 0 ? tr("暂无准入资产", "No eligible assets") : tr("开始被动采集", "Start passive collection") }}
        </button>
        <button class="button ghost" :disabled="!project || exporting !== null" :title="tr('线索 CSV：原 14 列加分类、事件与整改列，导出后读回核验行数与 SHA-256', 'Leads CSV: the original 14 columns plus classification, incident and remediation columns; read back after export to verify the row count and SHA-256')" @click="exportAll('findings')"><Download :size="15" />{{exporting==='findings'?tr('正在导出并核验…', 'Exporting and verifying…'):tr('导出线索', 'Export leads')}}</button>
        <button class="button ghost" :disabled="!project || exporting !== null" :title="tr('事件台账 CSV：rowKind 区分汇总/事件/位置/单位关联/覆盖，汇总行与明细可对账', 'Incident ledger CSV: rowKind distinguishes summary/incident/location/organization link/coverage rows; summary rows reconcile with the details')" @click="exportAll('ledger')"><Download :size="15" />{{exporting==='ledger'?tr('正在导出并核验…', 'Exporting and verifying…'):tr('导出事件台账', 'Export incident ledger')}}</button>
        <button class="button ghost" :disabled="!project||project.status==='archived'" @click="showAdd = !showAdd"><Plus :size="15" />{{ tr('录入线索', 'Add lead') }}</button>
      </div>
    </div>

    <div v-if="!project" class="panel exposure-empty"><ShieldAlert :size="25" /><strong>{{ tr('请先选择工作空间', 'Select a workspace first') }}</strong><span>{{ tr('暴露面线索不能跨公司混合。', 'Exposure leads cannot be mixed across companies.') }}</span></div>
    <template v-else>
      <p v-if="hasSnapshot && summary?.projectActive && summary?.eligibleAssetsPlanned === 0" class="exposure-workflow-help" role="status">{{ tr('当前没有同时满足归属和授权条件的资产。先在“归属门禁”确认并授权目标，再开始采集；历史线索仍可查看，0 条结果不代表没有暴露风险。', 'No assets currently meet both the ownership and authorization requirements. Confirm and authorize targets in “Ownership review” first, then start collection. Past leads remain viewable, and 0 results does not mean there is no exposure risk.') }}</p>
      <div v-if="stale" class="panel exposure-stale"><strong>{{ tr('显示的是最近一次可信快照', 'Showing the latest trusted snapshot') }}</strong><span>{{ readError }}</span></div>
      <nav class="exposure-tabs">
        <button :class="{ active: view === 'overview' }" @click="view = 'overview'">{{ tr('总览', 'Overview') }}</button>
        <button :class="{ active: view === 'runs' }" @click="view = 'runs'">{{ tr('采集任务', 'Collection jobs') }}</button>
        <button :class="{ active: view === 'findings' }" @click="view = 'findings'">{{ tr('线索复核', 'Lead review') }}</button>
        <details class="exposure-more"><summary>{{ tr('更多工具', 'More tools') }}{{!['overview','runs','findings'].includes(view)?tr(' · 已打开', ' · open'):''}}</summary><div>
        <button :class="{ active: view === 'sources' }" @click="view = 'sources'">{{ tr('数据源', 'Data sources') }}</button>
        <button :class="{ active: view === 'pending' }" @click="openTab('pending')">{{ tr('待归并', 'Pending merge') }}</button>
        <button :class="{ active: view === 'clusters' }" @click="view = 'clusters'">{{ tr('内容簇', 'Content clusters') }}</button>
        <button :class="{ active: view === 'review' }" @click="openTab('review')">{{ tr('复核整改', 'Review and remediation') }}</button>
        <button :class="{ active: view === 'incidents' }" @click="view = 'incidents'">{{ tr('事件台账', 'Incident ledger') }}</button>
        <button :class="{ active: view === 'coverage' }" @click="view = 'coverage'">{{ tr('覆盖与单位', 'Coverage and organizations') }}</button>
        <button :class="{ active: view === 'settings' }" @click="view = 'settings'">{{ tr('采集配置', 'Collection settings') }}</button>
        </div></details>
      </nav>
      <p class="exposure-workflow-help">{{ tr('采集已准入资产 → AI 梳理线索 → 复核与整改。新增范围与明确结论由你决定；历史记录不等于当前仍然暴露。', 'Collect eligible assets → AI triages leads → review and remediate. You decide on any scope additions and final conclusions; historical records do not mean something is still exposed.') }}</p>

      <section v-if="view === 'overview'" class="exposure-summary">
        <button @click="statusFilter = ''; view = 'findings'; load()"><span>{{ tr('准入资产', 'Eligible assets') }}</span><strong>{{ summary?.eligibleAssetsTotal ?? 0 }}</strong></button>
        <button @click="statusFilter = 'new'; view = 'findings'; load()"><span>{{ tr('新线索', 'New leads') }}</span><strong>{{ summary?.findingsNew ?? 0 }}</strong></button>
        <button @click="statusFilter = 'reviewing'; view = 'findings'; load()"><span>{{ tr('复核中', 'Under review') }}</span><strong>{{ summary?.findingsReviewing ?? 0 }}</strong></button>
        <button @click="statusFilter = 'confirmed'; view = 'findings'; load()"><span>{{ tr('已确认', 'Confirmed') }}</span><strong>{{ summary?.findingsConfirmed ?? 0 }}</strong></button>
        <button class="risk" @click="view = 'findings'"><span>{{ tr('高风险待处置', 'Open high-risk') }}</span><strong>{{ summary?.highRiskOpen ?? 0 }}</strong></button>
        <p v-if="beyondPlanNotice" class="exposure-gap">{{ beyondPlanNotice }}</p>
        <p v-if="classificationNotice" class="exposure-classification">{{ classificationNotice }}</p>
        <ul v-if="unitLines.length" class="exposure-units">
          <li v-for="(line, index) in unitLines" :key="index">{{ line }}</li>
        </ul>
        <ul v-if="liveLines.length" class="exposure-live">
          <li v-for="(line, index) in liveLines" :key="index">{{ line }}</li>
        </ul>
      </section>

      <section v-if="view === 'runs'" class="panel exposure-runs">
        <header><strong>{{ tr('采集任务', 'Collection jobs') }}</strong><small>{{ tr(`已显示 ${runs.length} / 共 ${runsTotal} 轮`, `Showing ${runs.length} of ${runsTotal} runs`) }}</small></header>
        <div v-for="run in runs" :key="run.id" :class="{ selected: run.id === selectedRunId }">
          <strong>#{{ run.id }} · {{ run.status }}</strong>
          <span>{{ run.stage }}<template v-if="run.currentSource"> · {{ run.currentSource }}</template> · {{ tr(`${run.scannedAssets} / ${run.eligibleAssets} 工作项 · ${run.fetchedResources} 条观察 · ${run.findings} 新线索`, `${run.scannedAssets} / ${run.eligibleAssets} work items · ${run.fetchedResources} observations · ${run.findings} new leads`) }}</span>
          <small v-if="run.error">{{ run.error }}</small>
          <button @click="openRunSources(run.id)">{{ tr('查看该轮数据源', "View this run's data sources") }}</button>
          <button v-if="resumableLive === run.id" :disabled="resuming !== null || stale" @click="resumeRun(run.id)">{{resuming === run.id ? tr('正在恢复…', 'Resuming…') : tr('继续该轮断点', 'Resume this run from checkpoint')}}</button>
          <small v-else-if="['partial', 'interrupted', 'cancelled'].includes(run.status)" class="exposure-resume-note">{{ run.id === latestLiveRunId ? tr('恢复条件见总览；历史与已保存证据保留', 'See Overview for resume conditions; history and saved evidence are kept') : tr('历史任务；未读取到可恢复断点，可保留历史并新建一轮采集', 'Past job; no resumable checkpoint was found. Keep the history and start a new collection run.') }}</small>
        </div>
        <div v-if="!runs.length">{{ tr('当前工作空间还没有采集任务记录', 'No collection jobs in this workspace yet') }}</div>
        <footer v-if="hasMoreRuns"><button :disabled="loadingRunsMore || loading" @click="loadMoreRuns">{{loadingRunsMore?tr('正在读取…', 'Loading…'):tr('加载更早的任务', 'Load earlier jobs')}}</button></footer>
      </section>

      <section v-if="view === 'sources'" class="panel exposure-runs">
        <label class="exposure-run-pick">
          <span>{{ tr('查看哪一轮的数据源', 'Show data sources for run') }}</span>
          <select :value="selectedRunId ?? ''" @change="chooseRun">
            <option value="">{{ tr('未选择', 'None selected') }}</option>
            <option v-for="run in runs" :key="run.id" :value="run.id">#{{ run.id }} · {{ run.status }}</option>
          </select>
        </label>
        <div v-for="source in sources" :key="source.sourceKey" :class="['exposure-source-row', sourceStatusView(source).tone]">
          <strong>{{ sourceStatusView(source).label }} · {{ sourceStatusView(source).status }}</strong>
          <span>{{ tr(`${source.itemCount} 条结果 · ${source.completedAt}`, `${source.itemCount} results · ${source.completedAt}`) }}</span>
          <em v-if="sourceStatusView(source).reason" class="exposure-source-reason">{{ sourceStatusView(source).reason }}</em>
          <small v-if="source.error">{{ source.error }}</small>
        </div>
        <div v-if="!sources.length">{{ tr('所选任务尚无数据源执行记录', 'The selected job has no data source runs yet') }}</div>
      </section>

      <section v-if="view === 'review'" class="panel exposure-review-pick">
        <label>
          <span>{{ tr(`复核哪条线索（只列出当前已读到的 ${findings.length} / 共 ${findingsTotal} 条）`, `Lead to review (lists only the ${findings.length} of ${findingsTotal} already loaded)`) }}</span>
          <select :value="reviewFindingId ?? ''" @change="pickReviewFinding">
            <option value="">{{ tr('未选择', 'None selected') }}</option>
            <option v-for="item in findings" :key="item.id" :value="item.id">{{ item.title }}（{{ item.status }}）</option>
          </select>
        </label>
        <p v-if="hasMore" class="exposure-gap">{{ tr('线索列表还有更多页未读取；这里不会把全部线索拉进页面，请先加载更多或在列表里搜索。', 'The lead list has more pages that have not been loaded. This does not pull every lead onto the page; load more or search the list first.') }}</p>
      </section>
      <ExposureReviewPanel
        v-if="view === 'review'"
        :project-id="props.selectedProjectId"
        :finding-id="reviewFindingId"
        @notify="(kind, text) => emit('notify', kind, text)"
      />

      <ExposureIncidentsPanel v-if="view === 'incidents'" :project-id="props.selectedProjectId" @notify="(kind, text) => emit('notify', kind, text)" />

      <section v-if="view === 'coverage'" class="panel exposure-coverage">
        <header><strong>{{ tr('覆盖矩阵', 'Coverage matrix') }}</strong><small>{{ coverageLine }}</small></header>
        <table v-if="coverageView.length">
          <thead>
            <tr><th>{{ tr('来源能力', 'Source capability') }}</th><th>{{ tr('状态 · 分母 · 时间窗口', 'Status · denominator · time window') }}</th><th v-for="(label, key) in INFO_CATEGORY_LABELS" :key="key">{{ label }}</th><th>{{ tr('钻取', 'Drill-down') }}</th></tr>
          </thead>
          <tbody>
            <tr v-for="row in coverageView" :key="row.capability">
              <th>{{ row.label }}<small v-if="row.integrationPending"> · {{ tr('待接入', 'pending integration') }}</small></th>
              <td><strong>{{ row.state }}</strong><small>{{ row.denominator }}</small><small>{{ row.window }}</small><em>{{ row.reason }}</em></td>
              <td v-for="cell in row.cells" :key="cell.category" :class="['exposure-cell', cell.tone]">{{ cell.label }}</td>
              <td><button :data-drill="row.sourceKey" :disabled="!row.sourceKey" @click="drillSource = row.sourceKey">{{ tr('明细', 'Details') }}</button></td>
            </tr>
          </tbody>
        </table>
        <ExposureCoverageDrill
          :project-id="props.selectedProjectId"
          :rows="coverageView"
          :source="drillSource"
          @close="drillSource = ''"
          @notify="(kind, text) => emit('notify', kind, text)"
        />
        <header><strong>{{ tr('单位统计', 'Organization statistics') }}</strong></header>
        <ul class="exposure-units">
          <li v-for="(line, index) in unitStatLines" :key="index">{{ line }}</li>
        </ul>
      </section>

      <ExposureClustersPanel v-if="view === 'clusters'" :project-id="props.selectedProjectId" @notify="(kind, text) => emit('notify', kind, text)" />

      <ExposureSettingsPanel v-if="view === 'settings'" :project-id="props.selectedProjectId" @notify="(kind, text) => emit('notify', kind, text)" />

      <section v-if="showAdd" class="panel exposure-add">
        <header><strong>{{ tr('录入或导入公开线索', 'Add or import a public lead') }}</strong><button @click="showAdd = false"><X :size="16" /></button></header>
        <div class="exposure-form">
          <label>
            <span>{{ tr(`绑定准入资产（可选，共 ${boundAssetsTotal} 个）`, `Bind to an eligible asset (optional, ${boundAssetsTotal} total)`) }}</span>
            <select v-model="form.assetId"><option :value="undefined">{{ tr('主体级线索', 'Entity-level lead') }}</option><option v-for="asset in boundAssets" :key="asset.id" :value="asset.id">{{ asset.domain || asset.host || asset.link }}</option></select>
          </label>
          <p v-if="boundAssetsTruncated" class="exposure-gap">{{ tr(`下拉只加载了前 ${boundAssets.length} 个准入资产，可继续翻页选择更早的条目。`, `The dropdown loaded only the first ${boundAssets.length} eligible assets; page further to choose earlier entries.`) }}</p>
          <label><span>{{ tr('类型', 'Category') }}</span><select v-model="form.category"><option value="source_code">{{ tr('源代码泄漏', 'Source code leak') }}</option><option value="document">{{ tr('内部文档', 'Internal document') }}</option><option value="credential">{{ tr('凭据线索', 'Credential lead') }}</option><option value="storage">{{ tr('公开存储', 'Public storage') }}</option><option value="repository">{{ tr('代码仓库', 'Code repository') }}</option><option value="metadata">{{ tr('敏感元数据', 'Sensitive metadata') }}</option><option value="other">{{ tr('其他', 'Other') }}</option></select></label>
          <label><span>{{ tr('风险', 'Risk') }}</span><select v-model="form.severity"><option value="low">{{ tr('低', 'Low') }}</option><option value="medium">{{ tr('中', 'Medium') }}</option><option value="high">{{ tr('高', 'High') }}</option><option value="critical">{{ tr('严重', 'Critical') }}</option></select></label>
          <label class="wide"><span>{{ tr('标题', 'Title') }}</span><input v-model="form.title" :placeholder="tr('面向复核人员的清晰描述', 'A clear description for reviewers')" /></label>
          <label class="wide"><span>{{ tr('公开来源 URL', 'Public source URL') }}</span><input v-model="form.sourceUrl" placeholder="https://…" /></label>
          <label class="wide"><span>{{ tr('证据摘要（后端统一脱敏，请勿粘贴完整密钥）', 'Evidence summary (the backend redacts it; do not paste full secrets)') }}</span><textarea v-model="form.evidenceExcerpt" rows="4" :placeholder="tr('记录文件路径、字段名、脱敏片段和可复现位置', 'Record file paths, field names, redacted snippets and reproducible locations')"></textarea></label>
        </div>
        <footer><button class="button primary" :disabled="saving" @click="submit">{{ tr('保存并自动去重', 'Save and deduplicate') }}</button></footer>
      </section>

      <section v-if="view === 'overview' || view === 'findings'" class="panel exposure-list">
        <div v-if="resultsUpdated" class="exposure-results-update" role="status">{{ tr('采集有新结果，当前列表保留阅读位置。', 'Collection has new results; the list keeps your reading position.') }}<button v-refresh-feedback="{scope:selectedProjectId,error:readError}" :disabled="loading" @click="load">{{ tr('刷新线索', 'Refresh leads') }}</button></div>
        <header>
          <div><strong>{{ tr('暴露线索', 'Exposure leads') }}</strong><small>{{ tr(`${findings.length} / ${findingsTotal} 条当前筛选结果`, `${findings.length} / ${findingsTotal} matching the current filter`) }}</small></div>
          <label><Search :size="14" /><input v-model="searchFilter" :placeholder="tr('搜索标题、来源或证据', 'Search title, source or evidence')" @keyup.enter="load" /></label>
          <select v-model="dispositionFilter" class="exposure-disposition-filter" :title="tr('G1-3 按分类处置筛选', 'G1-3: filter by classification disposition')" @change="load()">
            <option v-for="option in dispositionFilters" :key="option.value" :value="option.value">{{ option.label }}</option>
          </select>
        </header>
        <article v-for="item in findings" :key="item.id">
          <div class="exposure-risk" :class="item.severity">{{ item.severity }}</div>
          <div class="exposure-main">
            <div><span>{{ categoryLabel(item.category) }}</span><strong>{{ item.title }}</strong><em>{{ tr(`${item.confidence}% 可信度`, `${item.confidence}% confidence`) }}</em></div>
            <a :href="item.sourceUrl" target="_blank">{{ item.sourceUrl }} <ExternalLink :size="11" /></a>
            <p>{{ item.evidenceExcerpt || tr('尚未填写证据摘要', 'No evidence summary yet') }}</p>
            <small>{{ item.assetLabel || tr('主体级公开线索', 'Entity-level public lead') }} · {{ item.sourceType }} · {{ tr(`首次 ${item.firstSeenAt} · 最近 ${item.lastSeenAt}`, `first seen ${item.firstSeenAt} · last seen ${item.lastSeenAt}`) }}</small>
          </div>
          <div class="exposure-actions">
            <span :class="`status-${item.status}`">{{ item.status }}</span>
            <span class="exposure-disposition" :class="badgeFor(item.id).tone" :title="badgeFor(item.id).title">{{ badgeFor(item.id).label }}</span>
            <button :title="tr('在复核整改面板中按队列版本、操作者和依据写入结论', 'Record a decision in the review and remediation panel with the queue revision, operator and basis')" @click="openReview(item.id)">{{ tr('复核…', 'Review…') }}</button>
            <button v-if="agentAvailable" @click="askAgent(item.id)">{{ tr('AI 分析', 'AI analysis') }}</button>
            <button :class="{ active: evidenceFor === item.id }" @click="showEvidence(item.id)">{{ tr('证据', 'Evidence') }}</button>
          </div>
        </article>
        <div v-if="loading" class="exposure-empty">{{ tr('正在加载…', 'Loading…') }}</div>
        <div v-else-if="readError && !hasSnapshot" class="exposure-error">
          <strong>{{ tr('线索列表没有读到，不能显示为“没有线索”', 'The lead list could not be loaded; this does not mean “no leads”') }}</strong>
          <span>{{ readError }}</span>
        </div>
        <div v-else-if="!findings.length" class="exposure-empty">{{ tr('当前筛选没有公开暴露线索', 'No public exposure leads match the current filter') }}</div>
        <footer v-if="hasMore" class="exposure-more"><button :disabled="loadingMore" @click="loadMore">{{ loadingMore ? tr("正在加载…", "Loading…") : tr("加载更多线索", "Load more leads") }}</button></footer>
      </section>
      <section v-if="view === 'pending'" class="panel exposure-runs">
        <header><strong>{{ tr('外部索引带回的待归并结果', 'Pending-merge results from external indexes') }}</strong><small>{{ pendingNotice }}</small></header>
        <p class="exposure-gap">{{ tr('这些名字只进入待归并队列：不自动成为资产，不自动通过归属门禁，也不自动获得任何采集授权。', 'These names only enter the pending-merge queue: they do not automatically become assets, pass the ownership gate or receive any collection authorization.') }}</p>
        <div v-for="row in pending?.items ?? []" :key="row.id">
          <strong>{{ row.entity }}</strong>
          <span>{{ row.sourceKey }} · {{ row.kind }} · {{ row.status }} · {{ row.createdAt }}</span>
          <small>{{ tr(`来源 ${row.evidenceUrl}`, `Source ${row.evidenceUrl}`) }}</small>
        </div>
        <div v-if="pendingLoading" class="exposure-empty">{{ tr('正在加载…', 'Loading…') }}</div>
        <div v-else-if="pendingError" class="exposure-error">
          <strong>{{ tr('待归并队列没有读到', 'Could not load the pending-merge queue') }}</strong>
          <span>{{ pendingError }}</span>
        </div>
        <div v-else-if="!pending?.items.length" class="exposure-empty">{{ pendingNotice }}</div>
        <footer v-if="pending?.hasMore" class="exposure-more"><button :disabled="pendingLoading || pendingLoadingMore" @click="loadMorePending">{{pendingLoadingMore?tr('正在加载…', 'Loading…'):tr('加载更多待归并结果', 'Load more pending-merge results')}}</button></footer>
      </section>

      <section v-if="evidenceLoading && !evidencePackage" class="panel exposure-evidence" role="status"><header><strong>{{ tr('正在读取所选线索的证据…', 'Loading evidence for the selected lead…') }}</strong><button :aria-label="tr('取消读取证据', 'Cancel loading evidence')" @click="closeEvidence"><X :size="16"/></button></header></section>
      <section v-if="evidenceFor !== null && !evidenceLoading && !evidencePackage && evidenceError" class="panel exposure-evidence" role="alert"><header><strong>{{ tr('证据未能读取', 'Could not load evidence') }}</strong><button :aria-label="tr('关闭证据错误', 'Dismiss evidence error')" @click="closeEvidence"><X :size="16"/></button></header><p class="exposure-error">{{evidenceError}}</p><button class="button secondary" @click="showEvidence(evidenceFor)">{{ tr('重试读取证据', 'Retry loading evidence') }}</button></section>
      <section v-if="evidencePackage" class="panel exposure-evidence">
        <header>
          <div>
            <strong>{{ tr(`证据包 · ${evidencePackage.finding.title}`, `Evidence package · ${evidencePackage.finding.title}`) }}</strong>
            <small>{{ headline }}</small>
          </div>
          <button @click="closeEvidence"><X :size="16" /></button>
        </header>
        <p v-if="intelGradeLine(evidencePackage)" class="exposure-gap">{{ intelGradeLine(evidencePackage) }}</p>
        <div v-if="evidencePackage.intelGrade" role="form" class="exposure-grade" @keydown="submitAssetsFormOnEnter($event, submitGrade, gradeBusy)">
          <strong>{{ tr('人工改级', 'Manual regrade') }}</strong>
          <p>{{ tr(`当前等级只读：${evidencePackage.intelGrade}。改级只写记录，不访问声明的来源，也不自动关闭线索。`, `Current grade (read-only): ${evidencePackage.intelGrade}. A regrade only writes a record; it does not visit the declared source or close the lead automatically.`) }}</p>
          <label>{{ tr('新等级', 'New grade') }}
            <select v-model="gradeChoice">
              <option value="claim_only">{{ tr('仅声称', 'Claim only') }}</option>
              <option value="sample_supported">{{ tr('样本支持', 'Sample supported') }}</option>
              <option value="human_verified">{{ tr('已人工核验', 'Manually verified') }}</option>
            </select>
          </label>
          <label>{{ tr('操作人', 'Operator') }}<input v-model="gradeActor" maxlength="80" /></label>
          <label>{{ tr('理由', 'Reason') }}<input v-model="gradeReason" maxlength="500" /></label>
          <label>{{ tr('样本观察', 'Sample observation') }}
            <select v-model="gradeSample">
              <option value="">{{ tr('不选', 'None') }}</option>
              <option v-for="row in evidencePackage.observations" :key="row.id" :value="String(row.id)">#{{ row.id }}</option>
            </select>
          </label>
          <label>{{ tr('目录单位编号', 'Directory organization ID') }}<input v-model="gradeEntity" inputmode="numeric" :placeholder="tr('可选', 'Optional')" /></label>
          <button type="button" :disabled="gradeBusy" @click="submitGrade">{{ tr('写入等级', 'Save grade') }}</button>
          <small v-if="gradeNotice">{{ gradeNotice }}</small>
        </div>
        <p v-if="reviewNeeded" class="exposure-gap">{{ tr(`内容或规则已变化（版本 ${reviewNeeded.revision}，规则 ${reviewNeeded.ruleVersion}）：上一人工结论仍保留，但必须重新复核。`, `Content or rules changed (revision ${reviewNeeded.revision}, rules ${reviewNeeded.ruleVersion}): the previous manual decision is kept but must be reviewed again.`) }}</p>
        <p v-if="evidenceLoading" class="exposure-empty">{{ tr('正在读取证据…', 'Loading evidence…') }}</p>
        <div class="exposure-observations">
          <div v-for="row in evidencePackage.observations" :key="row.id">
            <strong>{{ row.kind }} · {{ row.statusCode || '—' }}</strong>
            <span>{{ row.url }}</span>
            <p v-if="observationTlsWarning(row)" class="exposure-tls-warning" role="status">{{ observationTlsWarning(row) }}</p>
            <p v-if="observationHttpWarning(row)" class="exposure-tls-warning" role="status">{{ observationHttpWarning(row) }}</p>
            <small>
              {{ tr(`第 ${row.runId} 轮 · ${row.sourceKey} · 抓取 ${row.queriedAt}`, `Run ${row.runId} · ${row.sourceKey} · fetched ${row.queriedAt}`) }}
              · {{ tr(`内容 ${row.contentHash.slice(0, 12)} · 响应头 ${row.headersHash.slice(0, 12)}`, `content ${row.contentHash.slice(0, 12)} · headers ${row.headersHash.slice(0, 12)}`) }}
              · {{ tr(`${row.bodyBytes} 字节${row.truncated ? "（已截断）" : ""}`, `${row.bodyBytes} bytes${row.truncated ? " (truncated)" : ""}`) }}
              · {{ row.currentAccessible === null ? tr("当前可访问性未知（仅索引命中）", "current accessibility unknown (index hit only)") : row.currentAccessible ? tr("当前可访问", "currently accessible") : tr("当前不可访问", "currently inaccessible") }}
              <template v-if="row.indexedAt"> · {{ tr(`收录 ${row.indexedAt}`, `indexed ${row.indexedAt}`) }}</template>
              <template v-if="row.boundary"> · {{ tr(`边界 ${row.boundary}`, `boundary ${row.boundary}`) }}</template>
              · {{ tr(`规则 ${row.ruleVersion} · 脱敏 ${row.redactionVersion}`, `rules ${row.ruleVersion} · redaction ${row.redactionVersion}`) }}
            </small>
          </div>
          <div v-if="!evidencePackage.observations.length" class="exposure-empty">{{ tr('这条线索还没有绑定观察记录（旧版采集留下的线索）', 'This lead has no linked observations yet (it was left by an older collection version)') }}</div>
        </div>
        <div class="exposure-versions">
          <strong>{{ tr('版本轨迹', 'Revision history') }}</strong>
          <div v-for="row in evidencePackage.versions" :key="row.revision">
            <span>v{{ row.revision }} · {{ row.severity }} · {{ row.contentHash.slice(0, 12) }} · {{ row.createdAt }}</span>
            <small>{{ tr(`人工状态 ${row.manualStatus || "—"}${row.reviewNeeded ? " · 需复核" : ""} · 规则 ${row.ruleVersion}`, `Manual status ${row.manualStatus || "—"}${row.reviewNeeded ? " · needs review" : ""} · rules ${row.ruleVersion}`) }}</small>
          </div>
          <div v-if="!evidencePackage.versions.length" class="exposure-empty">{{ tr('尚无版本记录', 'No revisions yet') }}</div>
        </div>
      </section>
    </template>
  </div>
</template>
