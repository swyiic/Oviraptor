<script setup lang="ts">
import { useNativeProcessLog } from "../execution/useNativeProcessLog";
const props = defineProps<{ scanId: string; attempt: number; active: boolean }>();
const { replay, loading, readFailed, connectionUnavailable, refresh } = useNativeProcessLog(
  () => props.active && props.scanId ? { scanId: props.scanId, attempt: props.attempt } : undefined,
);
</script>

<template>
  <section class="native-process-log" aria-label="原生逐路日志">
    <header><div><span class="eyebrow">PROCESS STREAMS</span><strong>原生逐路日志 · #{{ replay.attempt || props.attempt || 0 }}</strong></div>
      <button class="button ghost compact" type="button" :disabled="loading" @click="refresh">刷新</button></header>
    <p v-if="connectionUnavailable" role="status">实时订阅暂不可用，正在补读已提交日志并重连。</p>
    <p v-if="readFailed" role="alert">此轮原生日志暂时不可读取，保留最后一次提交内容。</p>
    <p v-if="replay.hasGap" role="status">日志采集存在缺口；执行结果和证据请结合持久化记录核对。</p>
    <p v-if="replay.evicted" role="status">当前显示最近 300 条，已移出视图 {{ replay.evicted }} 条；完整记录仍可按游标读取。</p>
    <p v-if="replay.more || loading" role="status">正在恢复已提交的逐路日志…</p>
    <div v-if="replay.rows.length" class="native-log-rows" role="region" aria-label="已提交逐路日志内容" tabindex="0">
      <div v-for="row in replay.rows" :key="row.sequence" class="native-log-row" :class="row.stream">
        <time>{{ row.time }}</time><span class="native-log-channel">{{ row.branch }}/{{ row.stage }} · {{ row.executionId?.slice(0,8) ?? '归属待核验' }} · {{ row.stream }} #{{ row.streamSequence }}</span><code>{{ row.message }}</code>
      </div>
    </div>
    <p v-if="!loading && !readFailed && !replay.rows.length">此轮尚无已提交的原生逐路日志。</p>
  </section>
</template>

<style scoped src="./nativeLogPresentation.css"></style>
