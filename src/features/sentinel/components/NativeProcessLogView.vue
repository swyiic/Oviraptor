<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { useNativeProcessLog } from "../execution/useNativeProcessLog";
import { useI18n } from "../../../i18n";
const props = defineProps<{ scanId: string; attempt: number; active: boolean }>();
const { tr } = useI18n();
const { replay, loading, readFailed, connectionUnavailable, refresh } = useNativeProcessLog(
  () => props.active && props.scanId ? { scanId: props.scanId, attempt: props.attempt } : undefined,
);
</script>

<template>
  <section class="native-process-log" :aria-label="tr('原生逐路日志', 'Native per-branch log')">
    <header><div><strong>{{ tr(`原生逐路日志 · #${replay.attempt || props.attempt || 0}`, `Native per-branch log · #${replay.attempt || props.attempt || 0}`) }}</strong></div>
      <button v-refresh-feedback="{scope:scanId+':'+attempt,busy:loading,error:readFailed}" class="button ghost compact" type="button" :disabled="loading" @click="refresh">{{ tr('刷新', 'Refresh') }}</button></header>
    <p v-if="connectionUnavailable" role="status">{{ tr('实时订阅暂不可用，正在补读已提交日志并重连。', 'Live subscription unavailable; catching up on committed logs and reconnecting.') }}</p>
    <p v-if="readFailed" role="alert">{{ tr('此轮原生日志暂时不可读取，保留最后一次提交内容。', "This attempt's native log can't be read right now; the last committed content is kept.") }}</p>
    <p v-if="replay.hasGap" role="status">{{ tr('日志采集存在缺口；执行结果和证据请结合持久化记录核对。', 'Log capture has gaps; check run results and evidence against the stored records.') }}</p>
    <p v-if="replay.evicted" role="status">{{ tr(`当前显示最近 300 条，已移出视图 ${replay.evicted} 条；完整记录仍可按游标读取。`, `Showing the latest 300 entries; ${replay.evicted} have left the view. The full record can still be read by cursor.`) }}</p>
    <p v-if="replay.more || loading" role="status">{{ tr('正在恢复已提交的逐路日志…', 'Restoring committed branch logs…') }}</p>
    <div v-if="replay.rows.length" class="native-log-rows" role="region" :aria-label="tr('已提交逐路日志内容', 'Committed branch log content')" tabindex="0">
      <div v-for="row in replay.rows" :key="row.sequence" class="native-log-row" :class="row.stream">
        <time>{{ row.time }}</time><span class="native-log-channel">{{ row.branch }}/{{ row.stage }} · {{ row.executionId?.slice(0,8) ?? tr('归属待核验', 'Owner unverified') }} · {{ row.stream }} #{{ row.streamSequence }}</span><code>{{ row.message }}</code>
      </div>
    </div>
    <p v-if="!loading && !readFailed && !replay.rows.length">{{ tr('此轮尚无已提交的原生逐路日志。', 'No committed native branch logs for this attempt yet.') }}</p>
  </section>
</template>

<style scoped src="./nativeLogPresentation.css"></style>
