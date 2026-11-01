<script setup lang="ts">
import { Activity } from "@lucide/vue";
import type { SentinelFinding } from "../../types";
import { json, text } from "../../presentation";
import { useI18n } from "../../../../i18n";

defineProps<{
  runtimeFeatureRows: SentinelFinding[];
  runtimeActionRows: SentinelFinding[];
  observedMutationRows: SentinelFinding[];
}>();

const { tr } = useI18n();
</script>

<template>
  <section class="result-block runtime-exploration-block">
    <div class="block-title">
      <Activity :size="17" />
      <div>
        <strong>{{ tr('自动探索轨迹', 'Automated exploration trace') }}</strong>
        <small>{{ tr('保留每个页面状态、触发动作及其新增请求；写操作只观察并中止，不自动提交。', 'Keeps every page state, the actions triggered, and the requests they added. Write operations are only observed and aborted, never submitted.') }}</small>
      </div>
      <div class="runtime-exploration-counts">
        <span>{{ tr(`${runtimeFeatureRows.length} 个状态`, `${runtimeFeatureRows.length} states`) }}</span>
        <span>{{ tr(`${runtimeActionRows.length} 次动作`, `${runtimeActionRows.length} actions`) }}</span>
        <span>{{ tr(`${observedMutationRows.length} 个写请求`, `${observedMutationRows.length} write requests`) }}</span>
      </div>
    </div>
    <div v-if="runtimeFeatureRows.length || runtimeActionRows.length" class="runtime-exploration-grid">
      <article class="runtime-state-column">
        <header>{{ tr('页面与功能状态', 'Page and feature states') }}</header>
        <div v-for="item in runtimeFeatureRows" :key="item.id" class="runtime-trace-card">
          <div>
            <b>{{ json(item.recordJson).stateId || item.title }}</b>
            <span>{{ tr(`深度 ${json(item.recordJson).depth ?? 0}`, `Depth ${json(item.recordJson).depth ?? 0}`) }}</span>
          </div>
          <strong>{{ json(item.recordJson).title || tr('未命名页面', 'Untitled page') }}</strong>
          <code :title="json(item.recordJson).url">{{ json(item.recordJson).url }}</code>
          <p v-if="json(item.recordJson).highValueLabels?.length">
            {{ tr(`高价值功能：${json(item.recordJson).highValueLabels.join("、")}`, `High-value features: ${json(item.recordJson).highValueLabels.join(", ")}`) }}
          </p>
          <small>
            {{ tr(`${json(item.recordJson).interactiveCount || 0} 个可触发控件 · ${json(item.recordJson).formCount || 0} 个表单`, `${json(item.recordJson).interactiveCount || 0} interactive controls · ${json(item.recordJson).formCount || 0} forms`) }}
            <template v-if="json(item.recordJson).fieldNames?.length">
              · {{ tr(`字段 ${json(item.recordJson).fieldNames.join("、")}`, `Fields ${json(item.recordJson).fieldNames.join(", ")}`) }}
            </template>
          </small>
        </div>
      </article>
      <article class="runtime-action-column">
        <header>{{ tr('触发动作与请求增量', 'Triggered actions and new requests') }}</header>
        <div v-for="item in runtimeActionRows" :key="item.id" class="runtime-trace-card">
          <div>
            <b>{{ json(item.recordJson).id || item.title }}</b>
            <span :class="{
              changed: json(item.recordJson).stateChanged,
              failed: json(item.recordJson).outcome === 'error',
            }">{{ json(item.recordJson).outcome || "observed" }}</span>
          </div>
          <strong>{{ json(item.recordJson).label || json(item.recordJson).role || tr('页面控件', 'Page control') }}</strong>
          <p>
            {{ tr(`新增 ${json(item.recordJson).requestCount || 0} 个请求 · 观察并拦截 ${json(item.recordJson).blockedRequestCount || 0} 个写请求`, `${json(item.recordJson).requestCount || 0} new requests · ${json(item.recordJson).blockedRequestCount || 0} write requests observed and blocked`) }} ·
            {{ json(item.recordJson).durationMs || 0 }} ms
          </p>
          <code :title="json(item.recordJson).afterUrl">
            {{ json(item.recordJson).beforeUrl }}
            <template v-if="json(item.recordJson).afterUrl && json(item.recordJson).afterUrl !== json(item.recordJson).beforeUrl">
              → {{ json(item.recordJson).afterUrl }}
            </template>
          </code>
        </div>
      </article>
    </div>
    <div v-else class="empty-inline">
      {{ tr('当前是旧扫描记录或页面没有可触发控件；重新运行扫描后会生成轨迹。', 'This is an older scan record, or the page has no interactive controls. Run the scan again to generate a trace.') }}
    </div>
    <details v-if="observedMutationRows.length" class="runtime-mutation-details">
      <summary>{{ tr(`查看 ${observedMutationRows.length} 个被观察并中止的写请求`, `View ${observedMutationRows.length} observed and aborted write requests`) }}</summary>
      <div>
        <article v-for="item in observedMutationRows" :key="item.id">
          <b>{{ json(item.recordJson).method || "WRITE" }}</b>
          <code>{{ json(item.recordJson).url }}</code>
          <small>
            {{ tr(`参数：${text(json(item.recordJson).bodyKeys || json(item.recordJson).queryKeys) || "未识别"}`, `Parameters: ${text(json(item.recordJson).bodyKeys || json(item.recordJson).queryKeys) || "unidentified"}`) }}
            · {{ tr(`来源动作 ${json(item.recordJson).actionId || "—"}`, `Source action ${json(item.recordJson).actionId || "—"}`) }}
          </small>
          <pre v-if="json(item.recordJson).postData">{{ json(item.recordJson).postData }}</pre>
        </article>
      </div>
    </details>
  </section>
</template>
