<script setup lang="ts">
import type { AgentTargetExecution } from "../types";
import { agentBackendLabel, agentModeLabel } from "../execution/presentation";
import AgentRequestUsage from "./AgentRequestUsage.vue";
import AgentRequestReview from "./AgentRequestReview.vue";
import { useI18n } from "../../../i18n";

const props = defineProps<{
  execution: AgentTargetExecution;
  scope?: { scanId: string; targetUrl: string } | null;
  scanId?: string;
  targetUrl: string;
}>();
const { tr } = useI18n();

// An unfinished family must never look like a completed negative result.
function gapLabel(gap: { status: string; reasonCode?: string }) {
  if (gap.status === "not_applicable") return tr("不适用", "Not applicable");
  if (gap.reasonCode === "tested_no_finding") return tr("已测，未发现", "Tested, nothing found");
  if (/seed_budget_exhausted|budget|预算|上限|限流/.test(String(gap.reasonCode || ""))) return tr("因预算未完成", "Incomplete due to budget");
  if (gap.status === "insufficient_evidence") return tr("证据不足", "Insufficient evidence");
  return tr("未覆盖", "Not covered");
}

function familyStatus(family: string) {
  const coverage = props.execution.coverage;
  if (!coverage) return "";
  if (coverage.covered.includes(family)) return tr("已覆盖", "Covered");
  const gap = (coverage.ledger?.uncoveredFamilies || []).find((row) => row.family === family);
  return gap ? gapLabel(gap) : tr("未覆盖", "Not covered");
}
</script>

<template>
  <details class="agent-execution-details" :open="execution.backend === 'native'">
    <summary>
      {{ tr('执行计划', 'Execution plan') }} · {{ agentBackendLabel(String(execution.backend)) }} ·
      {{ agentModeLabel(String(execution.mode)) }} · {{ execution.targetStatusText }}
    </summary>
    <div class="agent-execution-grid">
      <section>
        <h4>{{ tr('覆盖收口', 'Coverage close-out') }}</h4>
        <ul>
          <li v-for="(label, index) in execution.coverage?.requiredLabels || []" :key="label">
            <span>{{ label }}</span>
            <em>{{ familyStatus((execution.coverage?.required || [])[index]) }}</em>
          </li>
        </ul>
        <p v-if="execution.coverage && !execution.coverage.ledgerReported">
          {{ tr('所选后端未输出覆盖账本；完成比例仅按已记录证据估算', 'The selected backend did not report a coverage ledger; completion is estimated from recorded evidence only') }}
        </p>
        <p v-else-if="execution.coverage">
          {{ tr(`完成比例 ${Math.round(execution.coverage.completedRatio * 100)}% · 确认问题 ${execution.coverage.confirmedFindings}`, `Completion ${Math.round(execution.coverage.completedRatio * 100)}% · Confirmed issues ${execution.coverage.confirmedFindings}`) }}
        </p>
        <ul v-if="execution.coverage?.ledger?.uncoveredFamilies?.length">
          <li v-for="gap in execution.coverage.ledger.uncoveredFamilies" :key="gap.family">
            <span>{{ tr(`${gap.label}（${gapLabel(gap)}）`, `${gap.label} (${gapLabel(gap)})`) }}</span>
            <em>{{ gap.reason }}</em>
          </li>
        </ul>
      </section>
      <section>
        <h4>{{ tr('预算与消耗', 'Budget and usage') }}</h4>
        <dl>
          <dt>{{ tr('软预算 · 已用', 'Soft budget · used') }}</dt>
          <dd>
            {{ tr(`${execution.budgets?.softUncachedTokens || 0} Token · ${execution.runtime?.tokenUsage?.inputTokens || 0} 输入 / ${execution.runtime?.tokenUsage?.outputTokens || 0} 输出`,
              `${execution.budgets?.softUncachedTokens || 0} tokens · ${execution.runtime?.tokenUsage?.inputTokens || 0} input / ${execution.runtime?.tokenUsage?.outputTokens || 0} output`) }}
          </dd>
          <dt>{{ tr('硬上限 · 已用', 'Hard limit · used') }}</dt>
          <dd>
            {{ tr(`${execution.hardLimits?.hardTotalTokens || 0} Token · ${execution.runtime?.tokenUsage?.totalTokens || 0} 已消耗`,
              `${execution.hardLimits?.hardTotalTokens || 0} tokens · ${execution.runtime?.tokenUsage?.totalTokens || 0} consumed`) }}
          </dd>
          <dt>{{ tr('模型调用', 'Model calls') }}</dt>
          <dd>
            {{ tr(`软 ${execution.budgets?.softModelRequests || 0} / 硬 ${execution.hardLimits?.hardModelRequests || 0} · 实际 ${execution.runtime?.tokenUsage?.modelRequests || 0}`,
              `Soft ${execution.budgets?.softModelRequests || 0} / hard ${execution.hardLimits?.hardModelRequests || 0} · actual ${execution.runtime?.tokenUsage?.modelRequests || 0}`) }}
          </dd>
          <dt>{{ tr('Agent 目标请求', 'Agent target requests') }}</dt>
          <dd>
            <AgentRequestUsage :accounting="execution.runtime?.requestAccounting" />
            <AgentRequestReview
              v-if="scope && scope.scanId === scanId && scope.targetUrl === targetUrl
                && execution.runtime?.requestAccounting?.attemptNumber"
              :scan-id="scope.scanId" :target-url="scope.targetUrl"
              :attempt-number="execution.runtime.requestAccounting.attemptNumber"
            />
          </dd>
          <dt>{{ tr('发现轮次', 'Discovery rounds') }}</dt>
          <dd>{{ execution.runtime?.budgetUsage?.discoveryRounds || 0 }}</dd>
          <dt>{{ tr('回合数', 'Turns') }}</dt>
          <dd>{{ execution.runtime?.turns || 0 }} / {{ execution.hardLimits?.maxTurns || 0 }}</dd>
          <dt>{{ tr('最近一次扩容原因', 'Last expansion reason') }}</dt>
          <dd>{{ execution.runtime?.lastExpansionReason || tr('未扩容', 'Not expanded') }}</dd>
        </dl>
      </section>
      <section>
        <h4>{{ tr('当前动作与终态', 'Current action and final state') }}</h4>
        <dl>
          <dt>{{ tr('当前动作', 'Current action') }}</dt>
          <dd>{{ execution.runtime?.currentAction || tr('尚未记录当前动作', 'No current action recorded yet') }}</dd>
          <dt>{{ tr('最近新证据签名', 'Latest new-evidence signature') }}</dt>
          <dd>{{ execution.runtime?.progressSignature || tr('尚无', 'None yet') }}</dd>
          <dt>{{ tr('无进展计数', 'No-progress streak') }}</dt>
          <dd>
            {{ execution.runtime?.noProgressStreak || 0 }} /
            {{ execution.hardLimits?.noProgressWindow || 0 }}
          </dd>
          <dt>{{ tr('最终停止原因', 'Final stop reason') }}</dt>
          <dd>{{ execution.runtime?.terminalReason || tr('尚未结束', 'Not finished yet') }}</dd>
        </dl>
      </section>
      <section
        v-if="execution.coverage?.ledger?.manualDeepDiveSuggestions?.length
          || execution.coverage?.ledger?.exclusions?.length"
        class="agent-followup-advice"
      >
        <h4>{{ tr('Agent 的后续建议', "Agent's recommended follow-up") }}</h4>
        <ul v-if="execution.coverage?.ledger?.manualDeepDiveSuggestions?.length">
          <li v-for="tip in execution.coverage.ledger.manualDeepDiveSuggestions" :key="`tip-${tip}`">
            {{ tr(`建议人工深入：${tip}`, `Manual deep-dive: ${tip}`) }}
          </li>
        </ul>
        <template v-if="execution.coverage?.ledger?.exclusions?.length">
          <p class="agent-followup-sub">{{ tr('本轮刻意排除', 'Deliberately excluded this run') }}</p>
          <ul>
            <li v-for="item in execution.coverage.ledger.exclusions" :key="`excl-${item}`">{{ item }}</li>
          </ul>
        </template>
        <p class="agent-followup-note">{{ tr('这些是 Agent 的建议,不是已执行结果。继续需另建一个经授权的新任务(不继承本次预算、作用域或凭据)。', 'These are the agent’s suggestions, not executed results. Continuing requires a separately authorized new task — no budget, scope or credentials are inherited.') }}</p>
      </section>
    </div>
  </details>
</template>

<style scoped src="./executionPresentation.css"></style>
<style scoped>
.agent-followup-note { font-size: 11px; color: var(--ui-muted, #777); margin-top: 8px; }
.agent-followup-sub { font-weight: 600; margin: 8px 0 2px; }
</style>
