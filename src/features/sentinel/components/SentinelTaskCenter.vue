<script setup lang="ts">
import { Activity, Eye, Pause, Play, RefreshCw, Trash2, X } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelScan } from "../../../types";
import {
  createSentinelLabels,
  formatCompactNumber,
  formatNumber,
  latestAttemptLabel,
  routeModeLabel,
  scanInterruption,
  scanTitle,
} from "../presentation";

type PreviewTarget = { company: string; url: string; highValue: boolean };
defineProps<{
  scans: SentinelScan[];
  preview?: SentinelScan;
  previewTargets: PreviewTarget[];
  attentionCount: number;
  totalTokens: number;
  totalRequests: number;
  zeroYieldCount: number;
  zeroYieldTokens: number;
  cacheHitRate: number;
  controlBusy: string;
  highValueCount: (scanId: string) => number;
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
}>();
const { tr } = useI18n();
const { statusLabel, retryActionLabel, scanTypeLabel, llmDeploymentLabel } = createSentinelLabels(tr);
</script>

<template>
  <div class="task-cost-strip">
      <article><span>当前任务</span><strong>{{ scans.length }}</strong><small>{{ attentionCount }} 个需要确认、继续或复盘</small></article>
      <article><span>累计 Token</span><strong>{{ formatCompactNumber(totalTokens) }}</strong><small>{{ formatNumber(totalRequests) }} 次模型请求</small></article>
      <article :class="{ warning: zeroYieldCount }"><span>零漏洞产出</span><strong>{{ zeroYieldCount }}</strong><small>{{ formatNumber(zeroYieldTokens) }} Token 需要复盘</small></article>
      <article><span>缓存命中率</span><strong>{{ cacheHitRate }}%</strong><small>{{ cacheHitRate >= 50 ? "上下文复用正常" : "复用偏低，优先检查续跑策略" }}</small></article>
  </div>
  <div class="sentinel-queue-layout task-center-stack">
      <section class="panel sentinel-panel">
        <div class="panel-heading">
          <div><span class="eyebrow">TASK CENTER</span><h3>待扫与历史任务</h3><p>任务列表保持全宽；点击任务后在下方展开详情，不再长期占用右侧一列。</p></div>
        </div>
        <div class="task-center-list">
          <article v-for="scan in scans" :key="scan.id" :class="{ active: preview?.id === scan.id }" @click="emit('preview', scan)">
            <span class="sentinel-status" :class="scan.status"><Activity :size="15" /></span>
            <div>
              <strong>{{ scanTitle(scan) }}</strong><b v-if="highValueCount(scan.id)" class="high-value-badge">高 {{ highValueCount(scan.id) }}</b>
              <small>{{ scan.projectName }} · {{ scan.id }}</small>
              <em>{{ scanTypeLabel(scan.scanType) }}<template v-if="scan.scanType === 'web'"> · 任务模式：{{ routeModeLabel(scan.requestedScanMode || 'standard') }}</template> · {{ llmDeploymentLabel(scan) }} · {{ latestAttemptLabel(scan, statusLabel) }} · {{ scan.updatedAt }}</em>
              <div v-if="scanInterruption(scan)" class="task-stop-summary" :class="scanInterruption(scan)?.tone"><b>{{ scanInterruption(scan)?.title }}</b><span>{{ scanInterruption(scan)?.detail }}</span><small>{{ scanInterruption(scan)?.action }}</small></div>
            </div>
            <div class="task-center-actions">
              <button v-if="scan.status === 'draft'" class="button primary compact" @click.stop="emit('confirm', scan)"><Play :size="13" />确认扫描</button>
              <button v-if="scan.status === 'scanning' || scan.status === 'pausing'" class="button warning compact" :disabled="controlBusy === scan.id" @click.stop="emit('pause', scan)"><Pause :size="13" />{{ scan.status === "pausing" ? "正在停止" : "停止并保留" }}</button>
              <button v-else-if="scan.status === 'paused'" class="button secondary compact" :disabled="controlBusy === scan.id" @click.stop="emit('resume', scan)"><Play :size="13" />继续扫描</button>
              <button v-else-if="scan.status !== 'draft'" class="button ghost compact" :disabled="controlBusy === scan.id" @click.stop="emit('retry', scan)"><RefreshCw :size="13" />{{ retryActionLabel(scan) }}</button>
              <button class="button danger compact" @click.stop="emit('remove', scan)"><Trash2 :size="13" />{{ ["scanning", "pausing"].includes(scan.status) ? "强制删除" : "删除" }}</button>
            </div>
          </article>
          <div v-if="!scans.length" class="empty-state">暂无任务</div>
        </div>
      </section>
      <aside v-if="preview" class="panel task-preview task-preview-inline">
          <header><div><span class="eyebrow">TASK PREVIEW</span><h3>{{ scanTitle(preview) }}</h3><div class="task-preview-facts"><span class="status-chip" :class="preview.latestAttemptStatus || preview.status">{{ latestAttemptLabel(preview, statusLabel) }}</span><span>{{ scanTypeLabel(preview.scanType) }}</span><span v-if="preview.scanType === 'web'">任务模式：{{ routeModeLabel(preview.requestedScanMode || 'standard') }}</span><code :title="preview.id">{{ preview.id }}</code></div></div><button class="icon-button" @click="emit('close')"><X :size="15" /></button></header>
          <div class="preview-url-list">
            <strong>任务目标 · {{ previewTargets.length }}</strong>
            <div v-for="row in previewTargets" :key="row.url" class="preview-target-row"><span>{{ row.company }}</span><code>{{ row.url }}</code><b v-if="row.highValue" class="high-value-badge">高</b></div>
            <div v-if="!previewTargets.length" class="empty-state small">没有可预览目标</div>
          </div>
          <div v-if="scanInterruption(preview)" class="task-stop-summary preview-stop-summary" :class="scanInterruption(preview)?.tone"><b>{{ scanInterruption(preview)?.title }}</b><span>{{ scanInterruption(preview)?.detail }}</span><small>{{ scanInterruption(preview)?.action }}</small></div>
          <details class="task-technical"><summary>任务文件与运行信息</summary><code>{{ preview.taskPath || "确认扫描后生成" }}</code></details>
          <footer>
            <button v-if="preview.status === 'draft'" class="button primary" @click="emit('confirm', preview)"><Play :size="14" />确认并启动扫描</button>
            <button class="button ghost" @click="emit('open', preview)"><Eye :size="14" />打开结果页</button>
          </footer>
      </aside>
  </div>
</template>
