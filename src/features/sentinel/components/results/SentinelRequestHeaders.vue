<script setup lang="ts">
import { Network } from "@lucide/vue";
import { text } from "../../presentation";
import { useI18n } from "../../../../i18n";

defineProps<{
  declaredRequestHeaderRows: Array<Record<string, any>>;
  observedRequestHeaderRows: Array<Record<string, any>>;
  possibleRequestHeaderRows: Array<Record<string, any>>;
  requestHeaderIntelligence: Record<string, any>;
  headerDisplayValue: (row: Record<string, any>) => string;
}>();

const { tr } = useI18n();
</script>

<template>
  <section class="result-block request-header-intelligence">
    <div class="block-title">
      <Network :size="17" />
      <div>
        <strong>{{ tr('请求头情报', 'Request header intelligence') }}</strong>
        <small>{{ tr('运行时生效值、JS 声明值和浏览器可能管理但尚未观察到的 Header 分层展示。', 'Headers grouped into values in effect at runtime, values declared in JS, and browser-managed headers that may exist but were not observed.') }}</small>
      </div>
      <div class="runtime-exploration-counts">
        <span>{{ tr(`已观察 ${observedRequestHeaderRows.length}`, `Observed ${observedRequestHeaderRows.length}`) }}</span>
        <span>{{ tr(`仅声明 ${declaredRequestHeaderRows.length}`, `Declared only ${declaredRequestHeaderRows.length}`) }}</span>
        <span>ExtraInfo {{ requestHeaderIntelligence.summary?.extraInfoHeaderCount || 0 }}</span>
      </div>
    </div>
    <div
      v-if="observedRequestHeaderRows.length || declaredRequestHeaderRows.length || possibleRequestHeaderRows.length"
      class="request-header-grid"
    >
      <article>
        <header><b>{{ tr('运行时真实生效', 'In effect at runtime') }}</b><span>{{ tr('可以作为复现依据', 'Usable for reproduction') }}</span></header>
        <div v-for="row in observedRequestHeaderRows" :key="`observed-${row.name}`" class="request-header-row">
          <div><code>{{ row.name }}</code><em v-if="row.sources?.includes('browser-extra-info')">{{ tr('隐藏补全', 'Hidden, recovered') }}</em></div>
          <p :title="headerDisplayValue(row)">{{ headerDisplayValue(row) }}</p>
          <small>{{ tr(`${row.occurrences || 1} 次`, `${row.occurrences || 1}×`) }} · {{ text(row.sources) }}</small>
        </div>
        <div v-if="!observedRequestHeaderRows.length" class="empty-inline">{{ tr('没有捕获到 XHR/Fetch/WebSocket 请求头', 'No XHR/Fetch/WebSocket request headers captured') }}</div>
      </article>
      <article>
        <header><b>{{ tr('JS 明确声明', 'Declared in JS') }}</b><span>{{ tr('需要运行时确认', 'Needs runtime confirmation') }}</span></header>
        <div v-for="row in declaredRequestHeaderRows" :key="`declared-${row.name}`" class="request-header-row declared">
          <div><code>{{ row.name }}</code><em>{{ tr('待确认', 'Unconfirmed') }}</em></div>
          <p :title="headerDisplayValue(row)">{{ headerDisplayValue(row) }}</p>
          <small>{{ text(row.sources) }}</small>
        </div>
        <div v-if="!declaredRequestHeaderRows.length" class="empty-inline">{{ tr('JS 中没有发现额外 Header 声明', 'No extra header declarations found in JS') }}</div>
      </article>
      <article>
        <header><b>{{ tr('浏览器管理头', 'Browser-managed headers') }}</b><span>{{ tr('可能存在，不算证据', 'May be present; not evidence') }}</span></header>
        <div v-for="row in possibleRequestHeaderRows" :key="`possible-${row.name}`" class="request-header-row possible">
          <div><code>{{ row.name }}</code><em>{{ tr('可能', 'Possible') }}</em></div>
          <p>{{ row.reason }}</p>
        </div>
      </article>
    </div>
    <div v-else class="empty-inline">
      {{ tr('当前是旧扫描记录；重新运行扫描后会从 CDP ExtraInfo 和业务 JS 生成请求头证据。', 'This is an older scan record. Run the scan again to build request header evidence from CDP ExtraInfo and the app’s JS.') }}
    </div>
  </section>
</template>
