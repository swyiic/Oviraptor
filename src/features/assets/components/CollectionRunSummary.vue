<script setup lang="ts">
import { computed } from "vue";
import type { JobRun } from "../types";
import { collectionNotes, collectionRetention, collectionSummary } from "../collectionPresentation";
import { useI18n } from "../uiPorts";

const props = defineProps<{ run: JobRun; compact?: boolean }>();
const { tr } = useI18n();
const summary = computed(() => collectionSummary(props.run.collectionSummary));
// While the run is still active (running / queued / pausing / cancelling) the
// collection is simply IN PROGRESS, not "incomplete" — showing an amber
// "coverage incomplete" badge mid-run reads like an error. Only judge
// complete/incomplete once the run has reached a terminal state.
const active = computed(() => ["running", "queued", "cancel_requested", "pause_requested"].includes(props.run.status));
const coverage = computed(() =>
  active.value ? "in_progress"
  : !summary.value ? "unknown"
  : summary.value.coverageComplete ? "complete" : "incomplete");
const notes = computed(() => summary.value ? collectionNotes(summary.value, tr) : []);
const retention = computed(() => collectionRetention(summary.value?.retention));
const count = (value: number | null) => value === null ? tr("未知", "Unknown") : value.toLocaleString();
</script>

<template>
  <div class="collection-run-summary" :class="{ compact }" :data-job-status="run.status" :data-collection-coverage="coverage" :data-collection-execution="summary ? (summary.executionComplete ? 'complete' : 'incomplete') : 'unknown'">
    <div class="collection-evidence-heading">
      <span class="collection-coverage" :class="coverage">{{ coverage === 'in_progress' ? tr('采集进行中','Collecting…') : coverage === 'complete' ? tr('采集覆盖完整','Collection coverage complete') : coverage === 'incomplete' ? tr('采集覆盖不完整','Collection coverage incomplete') : tr('采集覆盖未知','Collection coverage unknown') }}</span>
      <span v-if="summary && !active" class="collection-execution">{{ summary.executionComplete ? tr('采集查询已完成','Collection queries completed') : tr('采集查询未完成','Collection queries incomplete') }}</span>
    </div>
    <div v-if="summary" class="collection-metrics">
      <span data-collection-metric="queries">{{tr('查询','Queries')}} {{count(summary.completedQueries)}} / {{count(summary.queries)}}</span>
      <span data-collection-metric="candidates">{{tr('保留候选','Retained candidates')}} {{count(summary.candidates)}}</span>
      <span data-collection-metric="importedCandidates">{{tr('已入库','Imported')}} {{count(summary.importedCandidates)}}</span>
      <span data-collection-metric="probedAssets">{{tr('已探测','Probed')}} {{count(summary.probedAssets)}}</span>
    </div>
    <details v-if="summary && (notes.length || summary.failedQueries || summary.truncatedQueries || summary.partialQueries || summary.invalidRecords || summary.reusedPages || summary.retention != null)" class="collection-details">
      <summary>{{tr('查看覆盖明细','Coverage details')}}</summary>
      <div class="collection-metrics">
        <span v-if="summary.failedQueries" data-collection-metric="failedQueries">{{tr('失败查询','Failed queries')}} {{count(summary.failedQueries)}}</span>
        <span v-if="summary.truncatedQueries" data-collection-metric="truncatedQueries">{{tr('截断查询','Truncated queries')}} {{count(summary.truncatedQueries)}}</span>
        <span v-if="summary.partialQueries" data-collection-metric="partialQueries">{{tr('部分返回','Partial queries')}} {{count(summary.partialQueries)}}</span>
        <span v-if="summary.invalidRecords" data-collection-metric="invalidRecords">{{tr('无效记录','Invalid records')}} {{count(summary.invalidRecords)}}</span>
        <span v-if="summary.reusedPages" data-collection-metric="reusedPages">{{tr('复用页','Reused pages')}} {{count(summary.reusedPages)}}</span>
      </div>
      <small v-if="notes.length" class="collection-notes">{{notes.join(' · ')}}</small>
      <div v-if="retention" class="collection-retention">
        <small>{{tr('仅应用登记的采集目录及控制元数据；其他任务和未登记目录不计入此预算。','Only registered collection directories and control metadata are covered; other jobs and unregistered directories are outside this budget.')}}</small>
        <span data-retention-metric="runBytes">{{tr('本任务字节','Run bytes')}} {{count(retention.runBytes)}} / {{count(retention.maxRunBytes)}}</span>
        <span data-retention-metric="projectBytes">{{tr('本项目字节','Project bytes')}} {{count(retention.projectBytes)}} / {{count(retention.maxProjectBytes)}}</span>
        <span data-retention-metric="globalBytes">{{tr('已登记全局字节','Registered global bytes')}} {{count(retention.globalBytes)}} / {{count(retention.maxGlobalBytes)}}</span>
        <span data-retention-metric="expiredFiles">{{tr('已回收文件','Reclaimed files')}} {{count(retention.expiredFiles)}} · {{count(retention.expiredBytes)}} {{tr('字节','bytes')}}</span>
        <span data-retention-metric="unmanagedRuns">{{tr('未管理任务','Unmanaged runs')}} {{count(retention.unmanagedRuns)}} · {{tr('未知文件','Unknown files')}} {{count(retention.unknownFiles)}}</span>
        <span data-retention-metric="limits">{{tr('每任务页数上限','Pages per run limit')}} {{count(retention.maxPages)}} · {{tr('保留秒数上限','Retention seconds limit')}} {{count(retention.maxAgeSeconds)}}</span>
        <small v-if="retention.historyTruncated" data-retention-history-truncated>{{tr('控制历史已截断，当前明细不代表全部历史。','Control history was truncated; these details are not a complete history.')}}</small>
      </div>
      <small v-else-if="summary.retention != null" class="collection-notes">{{tr('保留预算证据无法核实。','Retention budget evidence could not be verified.')}}</small>
    </details>
    <small v-if="!summary" class="collection-notes">{{tr('此记录没有可验证的覆盖证据，执行完成不等于采集完整。','This record has no verifiable coverage evidence. A completed job does not prove complete collection.')}}</small>
  </div>
</template>

<style scoped>
.collection-run-summary { margin-top:8px; display:grid; gap:6px; font-size:var(--text-micro); line-height:1.5; }
.collection-evidence-heading,.collection-metrics { display:flex; flex-wrap:wrap; align-items:center; gap:6px 12px; }
.collection-coverage { border-radius:var(--radius-xs); padding:2px 6px; font-weight:600; background:var(--surface-soft,var(--ui-surface-sunken)); color:var(--muted,var(--ui-muted)); }
.collection-coverage.complete { background:color-mix(in srgb, var(--green) 9%, var(--ui-surface)); color:color-mix(in srgb, var(--green) 82%, var(--ui-ink)); }
.collection-coverage.incomplete { background:color-mix(in srgb, var(--amber) 11%, var(--ui-surface)); color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); }
.collection-coverage.in_progress { background:color-mix(in srgb, var(--blue) 9%, var(--ui-surface)); color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); }
.collection-execution,.collection-metrics,.collection-notes { color:var(--muted,var(--ui-muted)); }
.collection-notes { overflow-wrap:anywhere; font-size:var(--text-micro); }
.collection-details summary { cursor:pointer; color:var(--muted,var(--ui-muted)); }.collection-details .collection-metrics { margin:4px 0; }
.collection-retention { display:grid; gap:3px; margin-top:6px; color:var(--muted,var(--ui-muted)); overflow-wrap:anywhere; }.collection-retention small { font-size:var(--text-micro); }
.compact .collection-metrics { font-size:var(--text-micro); gap:3px 10px; }
</style>
