<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { RefreshCw } from "@lucide/vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";
import type { AgentStageDecision } from "../types";

// Agent intervention timeline: stage → outcome → agent decision → rationale,
// read-only from list_agent_stage_decisions. An empty list is the normal state
// until the gate records something, so it never surfaces as an error.
const props = defineProps<{ scanId: string; attempt?: number; refreshKey?: string }>();
const { tr } = useI18n();
const rows = ref<AgentStageDecision[]>([]);
const loaded = ref(false);
const loading = ref(false);
const failed = ref(false);
let generation = 0;
let disposed = false;

async function load() {
  const request = ++generation;
  const scanId = props.scanId;
  const current = () => !disposed && request === generation && scanId === props.scanId;
  if (!scanId) return;
  loading.value = true;
  try {
    const next = await sentinelApi.listAgentStageDecisions(scanId);
    if (!current()) return;
    rows.value = Array.isArray(next) ? next.filter((row) => row && row.scanId === scanId) : [];
    failed.value = false;
    loaded.value = true;
  } catch {
    if (current()) failed.value = true;
  } finally {
    if (current()) loading.value = false;
  }
}
watch(() => props.scanId, () => { rows.value = []; loaded.value = false; failed.value = false; void load(); }, { immediate: true });
watch(() => [props.attempt, props.refreshKey], () => { void load(); });
onUnmounted(() => { disposed = true; generation++; });

const STAGES: Record<string, [string, string]> = {
  root_frame_mapper: ["根框架映射", "Root frame mapping"],
  budget: ["预算", "Budget"],
  fetch: ["抓取", "Fetch"],
};
const OUTCOMES: Record<string, [string, string]> = {
  ok: ["正常", "OK"], degraded: ["降级", "Degraded"], blocked: ["受阻", "Blocked"], gap: ["有缺口", "Gap"],
};
const DECISIONS: Record<string, [string, string]> = {
  continue: ["继续", "Continue"], retry_adjusted: ["调整后重试", "Retry adjusted"], degrade: ["降级继续", "Degrade"],
  skip: ["跳过", "Skip"], needs_user: ["等你决定", "Needs you"],
};
const TIERS: Record<number, { tone: string; label: [string, string] }> = {
  1: { tone: "tier-1", label: ["自动吸收", "Absorbed"] },
  2: { tone: "tier-2", label: ["降级为 partial", "Downgraded to partial"] },
  3: { tone: "tier-3", label: ["需你决策", "Needs your decision"] },
};
const pick = (map: Record<string, [string, string]>, value: string) => map[value] ? tr(...map[value]) : value || "—";
const stageLabel = (stage: string) => pick(STAGES, stage);
const tierOf = (value: number) => TIERS[value] || { tone: "tier-0", label: ["未分档", "Untiered"] as [string, string] };
const outcomeTone = (value: string) => value === "ok" ? "ok" : value === "degraded" || value === "gap" ? "warn" : value === "blocked" ? "bad" : "";
function detail(row: AgentStageDecision) {
  if (!row.detailJson) return "";
  try {
    const value = JSON.parse(row.detailJson);
    if (value === null || (typeof value === "object" && !Object.keys(value).length)) return "";
    return JSON.stringify(value, null, 2);
  } catch { return row.detailJson; }
}

const groups = computed(() => {
  const byAttempt = new Map<number, AgentStageDecision[]>();
  for (const row of rows.value) {
    const list = byAttempt.get(row.attemptNumber) || [];
    list.push(row);
    byAttempt.set(row.attemptNumber, list);
  }
  return [...byAttempt.entries()].map(([attemptNumber, items]) => ({ attemptNumber, items }));
});
const tierCounts = computed(() => [1, 2, 3].map((tier) => ({
  tier, ...tierOf(tier), count: rows.value.filter((row) => row.gateTier === tier).length,
})).filter((item) => item.count));
const pendingUser = computed(() => rows.value.filter((row) =>
  row.gateTier === 3 && (!props.attempt || row.attemptNumber === props.attempt)).length);
</script>

<template>
  <section class="stage-timeline" :aria-label="tr('Agent 介入时间线', 'Agent intervention timeline')">
    <header class="stage-timeline-head">
      <div>
        <h3>{{ tr("Agent 介入时间线", "Agent intervention timeline") }}</h3>
      </div>
      <div class="stage-timeline-tools">
        <span v-for="item in tierCounts" :key="item.tier" class="stage-tier" :class="item.tone">{{ tr(...item.label) }} {{ item.count }}</span>
        <button type="button" class="button ghost compact" :disabled="loading" :aria-label="tr('刷新 Agent 介入记录', 'Refresh agent decisions')" @click="load"><RefreshCw :size="13" /></button>
      </div>
    </header>
    <p v-if="pendingUser" class="stage-timeline-alert" role="status">
      {{ tr(`本轮有 ${pendingUser} 处需要你决定，Agent 不会自动越过。`, `${pendingUser} decision(s) in this attempt need you; the agent will not proceed past them on its own.`) }}
    </p>
    <p v-if="failed && !rows.length" class="stage-timeline-empty">{{ tr("Agent 介入记录暂时读不到，可点右上角刷新重试。", "Agent decisions could not be read; use refresh to retry.") }}</p>
    <p v-else-if="!loaded" class="stage-timeline-empty">{{ tr("正在读取 Agent 介入记录…", "Reading agent decisions…") }}</p>
    <p v-else-if="!rows.length" class="stage-timeline-empty">{{ tr("尚无 Agent 介入：各阶段按预期结束，或还没有需要裁决的停止原因。", "No agent intervention yet: stages ended as expected, or no stop reason needed a decision.") }}</p>
    <div v-else class="stage-timeline-body">
      <section v-for="group in groups" :key="group.attemptNumber" class="stage-attempt">
        <h4 v-if="groups.length > 1 || (attempt && group.attemptNumber !== attempt)">
          {{ tr(`第 ${group.attemptNumber} 次执行`, `Attempt ${group.attemptNumber}`) }}<small v-if="attempt && group.attemptNumber === attempt">{{ tr("本轮", "current") }}</small>
        </h4>
        <ol class="stage-list">
          <li v-for="row in group.items" :key="row.id" class="stage-item" :class="tierOf(row.gateTier).tone">
            <div class="stage-flow">
              <span class="stage-name" :title="row.stage">{{ stageLabel(row.stage) }}<code v-if="stageLabel(row.stage) !== row.stage">{{ row.stage }}</code></span>
              <span class="stage-arrow" aria-hidden="true">→</span>
              <span class="stage-outcome" :class="outcomeTone(row.outcome)">{{ pick(OUTCOMES, row.outcome) }}</span>
              <span class="stage-arrow" aria-hidden="true">→</span>
              <strong class="stage-decision">{{ pick(DECISIONS, row.decision) }}</strong>
              <span class="stage-tier" :class="tierOf(row.gateTier).tone">{{ tr(...tierOf(row.gateTier).label) }}</span>
              <time>{{ row.createdAt }}</time>
            </div>
            <p v-if="row.rationale" class="stage-rationale">{{ row.rationale }}</p>
            <details v-if="detail(row)" class="stage-detail">
              <summary>{{ tr("原始细节", "Raw detail") }}</summary>
              <pre>{{ detail(row) }}</pre>
            </details>
          </li>
        </ol>
      </section>
    </div>
  </section>
</template>

<style scoped>
.stage-timeline { --tier: var(--ui-faint); display: grid; gap: 10px; min-width: 0; padding: 16px; border: 1px solid var(--ui-line); border-radius: var(--radius-lg); background: var(--ui-surface); color: var(--ui-ink); font-family: var(--font-sans); }
.stage-timeline-head { display: flex; flex-wrap: wrap; align-items: flex-end; justify-content: space-between; gap: 8px 16px; }
.stage-timeline-head h3 { margin: 2px 0 0; color: var(--ui-ink); font-size: var(--text-heading); font-weight: 600; }
.stage-timeline-tools { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
.stage-timeline-tools .button { display: inline-flex; align-items: center; justify-content: center; min-width: 28px; min-height: 28px; padding: 0 6px; }
.stage-timeline-empty { margin: 0; color: var(--ui-faint); font-size: var(--text-body); line-height: 1.6; }
.stage-timeline-alert { margin: 0; padding: 8px 12px; border-radius: var(--radius-md); background: color-mix(in srgb, var(--red) 8%, var(--ui-surface)); color: color-mix(in srgb, var(--red) 85%, var(--ui-ink)); font-size: var(--text-body); font-weight: 600; line-height: 1.6; }
.stage-timeline-body { display: grid; gap: 12px; }
.stage-attempt { display: grid; gap: 6px; min-width: 0; }
.stage-attempt h4 { display: flex; align-items: baseline; gap: 6px; margin: 0; color: var(--ui-muted); font-size: var(--text-meta); font-weight: 600; }
.stage-attempt h4 small { color: var(--ui-accent); font-size: var(--text-micro); }
.stage-list { display: grid; margin: 0; padding: 0; list-style: none; }
/* A rail with one dot per decision; the dot carries the gate tier colour. */
.stage-item { position: relative; display: grid; gap: 4px; min-width: 0; padding: 8px 0 10px 22px; }
.stage-item::before { content: ""; position: absolute; top: 0; bottom: 0; left: 6px; width: 1px; background: var(--ui-line); }
.stage-item:first-child::before { top: 14px; }
.stage-item:last-child::before { bottom: calc(100% - 14px); }
.stage-item::after { content: ""; position: absolute; top: 10px; left: 2px; width: 9px; height: 9px; border-radius: 50%; background: var(--tier); box-shadow: 0 0 0 3px var(--ui-surface); }
.tier-1 { --tier: var(--green); }
.tier-2 { --tier: var(--amber); }
.tier-3 { --tier: var(--red); }
.stage-flow { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 8px; min-width: 0; font-size: var(--text-body); line-height: 1.6; }
.stage-name { display: inline-flex; align-items: baseline; gap: 6px; min-width: 0; color: var(--ui-ink); font-weight: 600; }
.stage-name code { color: var(--ui-muted); font: var(--text-micro)/1.5 var(--font-mono); font-weight: 400; }
.stage-arrow { color: var(--ui-faint); }
.stage-outcome { color: var(--ui-muted); }
.stage-outcome.ok { color: color-mix(in srgb, var(--green) 82%, var(--ui-ink)); }
.stage-outcome.warn { color: color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); }
.stage-outcome.bad { color: color-mix(in srgb, var(--red) 82%, var(--ui-ink)); }
.stage-decision { color: var(--ui-ink); font-weight: 600; }
.stage-tier { padding: 1px 8px; border-radius: var(--radius-pill); background: color-mix(in srgb, var(--tier) 12%, var(--ui-surface)); color: color-mix(in srgb, var(--tier) 82%, var(--ui-ink)); font: 600 var(--text-micro)/1.6 var(--font-sans); white-space: nowrap; }
.stage-tier.tier-0 { --tier: var(--ui-muted); }
.stage-flow time { margin-left: auto; color: var(--ui-muted); font-size: var(--text-meta); font-variant-numeric: tabular-nums; white-space: nowrap; }
.stage-rationale { margin: 0; color: var(--ui-copy); font-size: var(--text-body); line-height: 1.65; overflow-wrap: anywhere; }
.stage-item.tier-3 .stage-rationale { color: var(--ui-ink); }
.stage-detail summary { width: fit-content; color: var(--ui-muted); font-size: var(--text-meta); }
.stage-detail pre { max-height: 220px; margin: 6px 0 0; padding: 10px; overflow: auto; border: 1px solid var(--ui-line); border-radius: var(--radius-md); background: var(--ui-surface-raised); color: var(--ui-copy); font: var(--text-meta)/1.6 var(--font-mono); white-space: pre-wrap; overflow-wrap: anywhere; }
@media (max-width: 600px) { .stage-flow time { margin-left: 0; flex-basis: 100%; } }
</style>
