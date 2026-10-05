<script setup lang="ts">
import { Network } from "@lucide/vue";
import { text } from "../../presentation";

defineProps<{
  declaredRequestHeaderRows: Array<Record<string, any>>;
  observedRequestHeaderRows: Array<Record<string, any>>;
  possibleRequestHeaderRows: Array<Record<string, any>>;
  requestHeaderIntelligence: Record<string, any>;
  headerDisplayValue: (row: Record<string, any>) => string;
}>();
</script>

<template>
  <section class="result-block request-header-intelligence">
    <div class="block-title">
      <Network :size="17" />
      <div>
        <strong>请求头情报</strong>
        <small>运行时生效值、JS 声明值和浏览器可能管理但尚未观察到的 Header 分层展示。</small>
      </div>
      <div class="runtime-exploration-counts">
        <span>已观察 {{ observedRequestHeaderRows.length }}</span>
        <span>仅声明 {{ declaredRequestHeaderRows.length }}</span>
        <span>ExtraInfo {{ requestHeaderIntelligence.summary?.extraInfoHeaderCount || 0 }}</span>
      </div>
    </div>
    <div
      v-if="observedRequestHeaderRows.length || declaredRequestHeaderRows.length || possibleRequestHeaderRows.length"
      class="request-header-grid"
    >
      <article>
        <header><b>运行时真实生效</b><span>可以作为复现依据</span></header>
        <div v-for="row in observedRequestHeaderRows" :key="`observed-${row.name}`" class="request-header-row">
          <div><code>{{ row.name }}</code><em v-if="row.sources?.includes('browser-extra-info')">隐藏补全</em></div>
          <p :title="headerDisplayValue(row)">{{ headerDisplayValue(row) }}</p>
          <small>{{ row.occurrences || 1 }} 次 · {{ text(row.sources) }}</small>
        </div>
        <div v-if="!observedRequestHeaderRows.length" class="empty-inline">没有捕获到 XHR/Fetch/WebSocket 请求头</div>
      </article>
      <article>
        <header><b>JS 明确声明</b><span>需要运行时确认</span></header>
        <div v-for="row in declaredRequestHeaderRows" :key="`declared-${row.name}`" class="request-header-row declared">
          <div><code>{{ row.name }}</code><em>待确认</em></div>
          <p :title="headerDisplayValue(row)">{{ headerDisplayValue(row) }}</p>
          <small>{{ text(row.sources) }}</small>
        </div>
        <div v-if="!declaredRequestHeaderRows.length" class="empty-inline">JS 中没有发现额外 Header 声明</div>
      </article>
      <article>
        <header><b>浏览器管理头</b><span>可能存在，不算证据</span></header>
        <div v-for="row in possibleRequestHeaderRows" :key="`possible-${row.name}`" class="request-header-row possible">
          <div><code>{{ row.name }}</code><em>可能</em></div>
          <p>{{ row.reason }}</p>
        </div>
      </article>
    </div>
    <div v-else class="empty-inline">
      当前是旧扫描记录；重新运行扫描后会从 CDP ExtraInfo 和业务 JS 生成请求头证据。
    </div>
  </section>
</template>
