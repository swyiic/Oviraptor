<script setup lang="ts">
import { computed } from "vue";
import { Cpu, Wrench } from "@lucide/vue";
import type { AgentTraceDetail } from "../../types";
import { formatNumber } from "../../presentation";
import RootDecisionSummary from "../RootDecisionSummary.vue";
import { isRootModelRound } from "../../traces/rootDecisionContract";
import { useI18n } from "../../../../i18n";

const props = defineProps<{
  detail?: AgentTraceDetail;
  busy: boolean;
  targetUrl: string;
  taskStatus: string;
}>();
const { tr } = useI18n();
// These are recorded events, not a heartbeat or proof of a currently running tool.
const visible = computed(() => props.detail || props.busy || ["scanning", "pausing"].includes(props.taskStatus));
function normalizedTarget(value: string) {
  try {
    const parsed = new URL(value);
    return `${parsed.protocol}//${parsed.host}${parsed.pathname.replace(/\/+$/, "")}`;
  } catch {
    return value.trim().replace(/\/+$/, "").toLowerCase();
  }
}
const recentEvents = computed(() => (props.detail?.events || [])
  .filter(event => props.targetUrl === "*" || !event.targetUrl
    || normalizedTarget(event.targetUrl) === normalizedTarget(props.targetUrl))
  .slice(-30).reverse());
const latestEvent = computed(() => recentEvents.value[0]);
const sourceLabel = computed(() => props.detail?.summary.sourceAuthority === "historical_external"
  ? tr("历史外部记录 · 只读", "Historical external record · read-only")
  : props.detail ? tr("Native 账本记录", "Native ledger record") : tr("等待轨迹记录", "Waiting for trace records"));
function eventLabel(value: string) {
  return ({ function_call: tr("工具调用", "Tool call"), function_call_output: tr("工具结果", "Tool result"),
    reasoning: tr("分析阶段", "Analysis"), message: tr("Agent 消息", "Agent message"),
    model_round_completed: tr("模型轮次记录", "Model round record") } as Record<string, string>)[value] || value;
}
function eventTitle(event: AgentTraceDetail["events"][number]) {
  if (!event.name) return eventLabel(event.eventType);
  if (event.eventType === "function_call_output") return tr(`${event.name} 返回结果`, `${event.name} returned`);
  if (event.eventType === "function_call") return tr(`调用 ${event.name}`, `Call ${event.name}`);
  return event.name;
}
const sessionLabel = (value: string) => value ? value.slice(0, 8) : "root";
</script>

<template>
  <section v-if="visible" class="result-block agent-live-chain">
    <div class="block-title">
      <Cpu :size="16" />
      <div>
        <strong>{{ tr('运行轨迹 · 事件记录', 'Run trace · event records') }}</strong>
        <small>{{ tr(`${sourceLabel}；记录不代表智能体当前仍在执行相同步骤。`, `${sourceLabel}. A record does not mean the agent is still running the same step.`) }}</small>
      </div>
      <span v-if="busy" class="live-chain-state">{{ tr('刷新中', 'Refreshing') }}</span>
    </div>
    <div class="live-chain-current">
      <span>{{ tr('最近记录', 'Latest record') }}</span>
      <strong>{{ latestEvent ? eventTitle(latestEvent) : tr('尚无可展示的结构化事件', 'No structured events to show yet') }}</strong>
      <small v-if="latestEvent">
        Agent {{ sessionLabel(latestEvent.sessionId) }}
        <template v-if="latestEvent.targetUrl"> · {{ latestEvent.targetUrl }}</template>
        <template v-if="latestEvent.callId"> · {{ tr(`调用 ${latestEvent.callId.slice(0, 12)}`, `Call ${latestEvent.callId.slice(0, 12)}`) }}</template>
        · {{ latestEvent.createdAt }}
      </small>
    </div>
    <template v-if="detail">
      <p class="live-chain-note">{{ tr('任务级累计统计，不随下方目标筛选变化。', 'Task-level cumulative totals; they do not change with the target filter below.') }}</p>
      <div class="live-chain-metrics">
        <article><span>{{ tr('模型请求', 'Model requests') }}</span><strong>{{ detail.summary.llmRequests }}</strong></article>
        <article><span>{{ tr('工具调用 / 返回', 'Tool calls / results') }}</span><strong>{{ detail.summary.toolCallCount }} / {{ detail.summary.toolResultCount }}</strong></article>
        <article><span>Agent</span><strong>{{ detail.summary.agentCount }}</strong></article>
        <article><span>{{ tr('总 Token', 'Total tokens') }}</span><strong>{{ formatNumber(detail.summary.totalTokens) }}</strong></article>
      </div>
      <div v-if="detail.summary.tools.length" class="live-chain-tools">
        <span v-for="tool in detail.summary.tools" :key="tool.name">
          <Wrench :size="11" /><b>{{ tool.name }}</b><em>{{ tr(`${tool.calls} 调用 / ${tool.results} 返回`, `${tool.calls} calls / ${tool.results} results`) }}</em>
        </span>
      </div>
    </template>
    <p class="live-chain-note">{{ tr('按接口返回顺序展示当前筛选最近 30 条，最新记录在前；包含未标注目标的任务级事件。', 'Shows the 30 most recent records for the current filter in API order, newest first, including task-level events without a target.') }}</p>
    <div v-if="recentEvents.length" class="live-chain-events">
      <article v-for="(event, index) in recentEvents" :key="event.id" :class="event.eventType">
        <header>
          <span>{{ eventLabel(event.eventType) }}</span><strong>{{ eventTitle(event) }}</strong>
          <em>Agent {{ sessionLabel(event.sessionId) }} · {{ isRootModelRound(event) ? tr('已记录', 'Recorded') : event.status || event.role || "recorded" }}</em>
          <time>{{ event.createdAt }}</time>
        </header>
        <RootDecisionSummary v-if="isRootModelRound(event)" :event="event" :source-authority="detail?.summary.sourceAuthority || ''" />
        <details v-else-if="event.detail" :open="index === 0">
          <summary>
            {{ event.eventType === "function_call" ? tr('查看调用参数', 'View call arguments')
              : event.eventType === "function_call_output" ? tr('查看返回摘要', 'View result summary') : tr('查看阶段摘要', 'View step summary') }}
            <span v-if="event.detailTruncated">· {{ tr('限长预览', 'Truncated preview') }}</span>
          </summary>
          <pre>{{ event.detail }}</pre>
        </details>
      </article>
    </div>
    <div v-else class="empty-inline live-chain-empty">
      {{ busy ? tr('正在读取 Agent 结构化事件…', 'Loading Agent structured events…') : tr('当前筛选尚无可展示的结构化事件；不能据此判断任务进展或完成情况。', 'No structured events to show for the current filter. This does not indicate task progress or completion.') }}
    </div>
    <p class="live-chain-note">{{ tr('详情为后端过滤后的展示预览，不是原始模型请求或私有推理；未标记的自由文本仍需谨慎处理。', 'Details are display previews filtered by the backend, not raw model requests or private reasoning. Treat unlabeled free text with care.') }}</p>
  </section>
</template>

<style src="../sentinelTracePresentation.css"></style>
