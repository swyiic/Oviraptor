<script setup lang="ts">
import { Activity, Eye, Pause, Play, RefreshCw, Trash2, X } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { SentinelScan } from "../../../types";
import {
  createSentinelLabels,
  formatCompactNumber,
  formatNumber,
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
const { statusLabel, scanTypeLabel, llmDeploymentLabel } = createSentinelLabels(tr);
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
              <em>{{ scanTypeLabel(scan.scanType) }} · {{ llmDeploymentLabel(scan) }} · {{ statusLabel(scan.status) }} · {{ scan.attemptCount ? `第 ${scan.attemptCount} 次执行` : "尚未执行" }} · {{ scan.updatedAt }}</em>
            </div>
            <div class="task-center-actions">
              <button v-if="scan.status === 'draft'" class="button primary compact" @click.stop="emit('confirm', scan)"><Play :size="13" />确认扫描</button>
              <button class="button ghost compact" @click.stop="emit('preview', scan)"><Eye :size="13" />预览</button>
              <button v-if="scan.status === 'scanning' || scan.status === 'pausing'" class="button warning compact" :disabled="controlBusy === scan.id" @click.stop="emit('pause', scan)"><Pause :size="13" />{{ scan.status === "pausing" ? "再次停止" : "立即暂停" }}</button>
              <button v-else-if="scan.status === 'paused'" class="button secondary compact" :disabled="controlBusy === scan.id" @click.stop="emit('resume', scan)"><Play :size="13" />继续扫描</button>
              <button v-else-if="scan.status !== 'draft'" class="button ghost compact" :disabled="controlBusy === scan.id" @click.stop="emit('retry', scan)"><RefreshCw :size="13" />继续当前任务</button>
              <button class="button danger compact" @click.stop="emit('remove', scan)"><Trash2 :size="13" />{{ ["scanning", "pausing"].includes(scan.status) ? "强制删除" : "删除" }}</button>
            </div>
          </article>
          <div v-if="!scans.length" class="empty-state">暂无任务</div>
        </div>
      </section>
      <aside v-if="preview" class="panel task-preview task-preview-inline">
          <header><div><span class="eyebrow">TASK PREVIEW</span><h3>{{ scanTitle(preview) }}</h3></div><button class="icon-button" @click="emit('close')"><X :size="15" /></button></header>
          <dl>
            <div><dt>状态</dt><dd><span class="status-chip" :class="preview.status">{{ statusLabel(preview.status) }}</span></dd></div>
            <div><dt>任务类型</dt><dd>{{ scanTypeLabel(preview.scanType) }}</dd></div>
            <div><dt>执行次数</dt><dd>{{ preview.attemptCount || 0 }} 次</dd></div>
            <div><dt>任务 ID</dt><dd><code>{{ preview.id }}</code></dd></div>
            <div><dt>任务文件</dt><dd><code>{{ preview.taskPath || "确认扫描后生成" }}</code></dd></div>
          </dl>
          <div class="preview-url-list">
            <strong>任务目标 · {{ previewTargets.length }}</strong>
            <div v-for="row in previewTargets" :key="row.url" class="preview-target-row"><span>{{ row.company }}</span><code>{{ row.url }}</code><b v-if="row.highValue" class="high-value-badge">高</b></div>
            <div v-if="!previewTargets.length" class="empty-state small">没有可预览目标</div>
          </div>
          <footer>
            <button v-if="preview.status === 'draft'" class="button primary" @click="emit('confirm', preview)"><Play :size="14" />确认并启动扫描</button>
            <button class="button ghost" @click="emit('open', preview)"><Eye :size="14" />打开结果页</button>
            <button v-if="preview.status === 'scanning' || preview.status === 'pausing'" class="button warning" :disabled="controlBusy === preview.id" @click="emit('pause', preview)"><Pause :size="14" />{{ preview.status === "pausing" ? "再次停止" : "立即暂停" }}</button>
            <button v-else-if="preview.status === 'paused'" class="button secondary" :disabled="controlBusy === preview.id" @click="emit('resume', preview)"><Play :size="14" />继续扫描</button>
            <button v-else-if="preview.status !== 'draft'" class="button ghost" :disabled="controlBusy === preview.id" @click="emit('retry', preview)"><RefreshCw :size="14" />继续当前任务</button>
            <button class="button danger" @click="emit('remove', preview)"><Trash2 :size="14" />{{ ["scanning", "pausing"].includes(preview.status) ? "强制停止并删除" : "删除任务" }}</button>
          </footer>
      </aside>
  </div>
</template>
