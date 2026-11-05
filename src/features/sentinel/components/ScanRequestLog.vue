<script setup lang="ts">
import { onUnmounted, ref, watch } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const props = defineProps<{ scanId: string; active: boolean }>();
const { tr } = useI18n();
const rows = ref<Array<{ tool: string; method: string; url: string; param: string; status: string; errorClass: string; startedAt: string; diff: string }>>([]);
const error = ref("");
let timer: number | undefined;

async function load() {
  if (!props.scanId) return;
  try {
    rows.value = await sentinelApi.listScanRequestLog(props.scanId);
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

watch(() => [props.scanId, props.active], () => {
  if (timer) window.clearInterval(timer);
  timer = undefined;
  if (!props.active) return;
  void load();
  timer = window.setInterval(() => void load(), 4000);
}, { immediate: true });

onUnmounted(() => { if (timer) window.clearInterval(timer); });
</script>

<template>
  <section class="request-log">
    <header>
      <strong>{{ tr("已保存的接口测试", "Saved API tests") }}</strong>
      <small>{{ tr("只显示这次任务里 Agent 实际发出的请求。参数变化相对同一地址的上一发。", "Only requests this task's agent actually sent. Parameter changes are compared with the previous call to the same URL.") }}</small>
    </header>
    <p v-if="error" class="error">{{ error }}</p>
    <p v-else-if="!rows.length" class="muted">{{ tr("还没有保存的请求。", "No saved requests yet.") }}</p>
    <table v-else>
      <thead>
        <tr>
          <th>{{ tr("方法", "Method") }}</th>
          <th>URL</th>
          <th>{{ tr("参数", "Param") }}</th>
          <th>{{ tr("状态", "Status") }}</th>
          <th>{{ tr("和上一发", "Vs previous") }}</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(row, index) in rows" :key="`${row.startedAt}-${index}`">
          <td>{{ row.method }}</td>
          <td class="url" :title="row.url">{{ row.url || row.tool }}</td>
          <td>{{ row.param || "—" }}</td>
          <td>{{ row.status }}<small v-if="row.errorClass"> {{ row.errorClass }}</small></td>
          <td>{{ row.diff }}</td>
        </tr>
      </tbody>
    </table>
  </section>
</template>

<style scoped>
.request-log { display: flex; flex-direction: column; gap: 8px; margin-bottom: 12px; }
.request-log header small, .muted { color: var(--muted, #666); }
table { width: 100%; border-collapse: collapse; font-size: 12px; }
th, td { border-bottom: 1px solid var(--border, #eee); padding: 6px 4px; text-align: left; vertical-align: top; }
.url { max-width: 360px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.error { color: #b4332f; }
</style>
