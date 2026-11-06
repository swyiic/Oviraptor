<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";
import type { ScanAgentStats } from "../types";

const props = defineProps<{ scanId: string }>();
const { tr } = useI18n();
const stats = ref<ScanAgentStats>();
const loading = ref(false);
const error = ref("");
const attemptFilter = ref<number | "all">("all");

const tokenPrimary = computed(() => {
  const t = stats.value?.tokens;
  if (!t) return 0;
  return t.ledgerTotal || t.runSummedTokens || 0;
});

async function load() {
  if (!props.scanId) return;
  loading.value = true;
  error.value = "";
  try {
    const attempt = attemptFilter.value === "all" ? undefined : attemptFilter.value;
    stats.value = await sentinelApi.getScanAgentStats(props.scanId, attempt);
  } catch (e) {
    error.value = String(e);
    stats.value = undefined;
  } finally {
    loading.value = false;
  }
}

async function exportJson() {
  if (!props.scanId) return;
  try {
    const json = await sentinelApi.exportScanAgentStats(props.scanId);
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `scan-${props.scanId.slice(0, 8)}-stats.json`;
    anchor.click();
    URL.revokeObjectURL(url);
  } catch (e) {
    error.value = String(e);
  }
}

watch(() => [props.scanId, attemptFilter.value], () => void load(), { immediate: true });
</script>

<template>
  <section v-if="scanId" class="scan-stats panel">
    <header>
      <div>
        <strong>{{ tr("扫描成本与产出", "Scan cost & output") }}</strong>
        <small>{{ tr("只读聚合：Token、工具调用、漏洞与待办门", "Read-only rollup: tokens, tools, findings, pending gates") }}</small>
      </div>
      <div class="scan-stats-actions">
        <select v-model="attemptFilter" @change="load">
          <option value="all">{{ tr("全部轮次", "All attempts") }}</option>
          <option v-for="row in stats?.attempts ?? []" :key="row.attemptNumber" :value="row.attemptNumber">
            {{ tr(`第 ${row.attemptNumber} 次`, `Attempt ${row.attemptNumber}`) }}
          </option>
        </select>
        <button type="button" class="button ghost compact" :disabled="loading" @click="load">{{ tr("刷新", "Refresh") }}</button>
        <button type="button" class="button secondary compact" :disabled="!stats" @click="exportJson">{{ tr("导出 JSON", "Export JSON") }}</button>
      </div>
    </header>
    <div v-if="stats" class="scan-stats-grid">
      <article><span>Token</span><strong>{{ tokenPrimary.toLocaleString() }}</strong><small>{{ tr("账本", "Ledger") }} {{ stats.tokens.ledgerTotal }} · {{ tr("运行合计", "Runs") }} {{ stats.tokens.runSummedTokens }}</small></article>
      <article><span>{{ tr("模型请求", "Model requests") }}</span><strong>{{ (stats.tokens.ledgerModelRequests || stats.tokens.runSummedRequests).toLocaleString() }}</strong></article>
      <article><span>{{ tr("工具调用", "Tool calls") }}</span><strong>{{ stats.toolInvocationsTotal.toLocaleString() }}</strong></article>
      <article><span>{{ tr("待答 / 高危", "Pending Q / HR") }}</span><strong>{{ stats.operator.pendingQuestions }} / {{ stats.operator.pendingHighRiskActions }}</strong></article>
    </div>
    <div v-if="stats?.tools.length" class="scan-stats-tools">
      <strong>{{ tr("工具画像", "Tool profile") }}</strong>
      <button
        v-for="tool in stats.tools.slice(0, 16)"
        :key="tool.toolName"
        type="button"
        class="chip"
        :title="JSON.stringify(tool.errorClasses)"
      >
        {{ tool.toolName }} ×{{ tool.calls }}
      </button>
    </div>
    <details v-if="stats?.findingRollup?.length" class="scan-stats-findings">
      <summary>{{ tr("漏洞 ↔ 合约工具归因", "Finding ↔ contract tool attribution") }} ({{ stats.findingRollup.length }})</summary>
      <table>
        <thead>
          <tr>
            <th>#</th>
            <th>{{ tr("严重度", "Severity") }}</th>
            <th>{{ tr("深挖", "Deep") }}</th>
            <th>{{ tr("迭代/请求", "Iter/req") }}</th>
            <th>{{ tr("合约工具", "Contract tools") }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in stats.findingRollup.slice(0, 40)" :key="row.findingId">
            <td>{{ row.findingId }}</td>
            <td>{{ row.severity }}</td>
            <td>{{ row.deepStatus }}</td>
            <td>{{ row.iterationsUsed }}/{{ row.requestsUsed }}</td>
            <td class="mono" :title="JSON.stringify(row.toolsByName)">
              {{ row.contractToolCalls ? Object.entries(row.toolsByName).map(([n, c]) => `${n}×${c}`).join(", ") : "—" }}
            </td>
          </tr>
        </tbody>
      </table>
    </details>
    <p v-if="loading" class="muted">{{ tr("加载中…", "Loading…") }}</p>
    <p v-if="error" class="error">{{ error }}</p>
  </section>
</template>

<style scoped>
.scan-stats { padding: 14px 16px; margin: 12px 0; display: flex; flex-direction: column; gap: 12px; }
.scan-stats header { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 10px; align-items: flex-start; }
.scan-stats header small { display: block; color: var(--muted, #666); margin-top: 4px; }
.scan-stats-actions { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.scan-stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: 10px; }
.scan-stats-grid article { border: 1px solid var(--border, #e5e5e5); border-radius: 8px; padding: 10px; display: flex; flex-direction: column; gap: 4px; }
.scan-stats-grid span { font-size: 11px; color: var(--muted, #777); }
.scan-stats-grid strong { font: 600 22px/1.2 var(--font-mono, monospace); }
.scan-stats-grid small { font-size: 10px; color: var(--muted, #888); }
.scan-stats-tools { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
.scan-stats-tools .chip { border: 1px solid var(--border, #ddd); border-radius: 999px; padding: 4px 10px; background: var(--surface, #fff); font: 11px var(--font-mono, monospace); cursor: default; }
.error { color: #b4332f; }
.muted { color: var(--muted, #777); }
.scan-stats-findings table { width: 100%; border-collapse: collapse; font-size: 11px; margin-top: 8px; }
.scan-stats-findings th, .scan-stats-findings td { border: 1px solid var(--border, #e8e8e8); padding: 4px 6px; text-align: left; }
.scan-stats-findings .mono { font-family: var(--font-mono, monospace); max-width: 280px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
