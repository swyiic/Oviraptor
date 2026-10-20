<script setup lang="ts">
import { computed } from "vue";
import type { AgentTask, AgentTaskCriterion } from "../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ task: AgentTask; cancelling: string[] }>();
defineEmits<{ cancelChild: [childTaskId: string, parentRunId: string] }>();
const { tr } = useI18n();
const children = computed(() => props.task.children ?? []);
const criteria = computed(() => props.task.criteria ?? []);
const receipt = computed(() => props.task.acceptance);
const validReceipt = computed(() => props.task.acceptanceValid === true && receipt.value?.provenance === "host-finite-checks" && receipt.value.ok === true);
const childStatus = (value: string) => ({ running: tr("执行中", "Running"), reported: tr("报告已返回", "Report received"), interrupted: tr("已中断", "Interrupted"), cancelled: tr("已取消", "Cancelled") } as Record<string, string>)[value] ?? value;
function criterionLabel(criterion: AgentTaskCriterion): string {
  if (criterion.kind === "file_exists") return tr(`读取产物 ${criterion.artifactId}`, `Read artifact ${criterion.artifactId}`);
  if (criterion.kind === "file_contains") return tr(`产物 ${criterion.artifactId} 包含指定文字`, `Artifact ${criterion.artifactId} contains the declared text`);
  const target = `${criterion.toolName} · ${criterion.path.join(".") || tr("返回值", "result")}`;
  return criterion.kind === "tool_read_equals" ? tr(`重新读取并检查 ${target}`, `Read again and check ${target}`) : tr(`检查本轮工具结果 ${target}`, `Check current-run tool result ${target}`);
}
</script>

<template>
  <section v-if="children.length" class="task-children" data-testid="agent-task-children">
    <h4>{{ tr('子任务', 'Child tasks') }} · {{ children.length }}</h4>
    <p class="task-limits">{{ tr('子任务报告供父任务参考；父任务仍需核实，报告不能代替工具证据或测试结果。', 'Child reports advise the parent task. The parent must verify them; reports do not replace tool evidence or test results.') }}</p>
    <p v-if="children.length > 8" class="task-limits">{{ tr('显示最近 8 个子任务。', 'Showing the latest 8 child tasks.') }}</p>
    <article v-for="child in children.slice(-8)" :key="`${child.id}:${child.runId}`" class="task-child" :data-child-task="child.id">
      <header><strong>{{ child.kind === 'review' ? tr('审查', 'Review') : tr('调查', 'Research') }} · {{ childStatus(child.status) }}</strong><button v-if="child.status === 'running'" type="button" class="button ghost compact" :disabled="cancelling.includes(child.id)" @click="$emit('cancelChild', child.id, child.parentRunId)">{{ cancelling.includes(child.id) ? tr('停止中…', 'Stopping…') : tr('停止子任务', 'Stop child task') }}</button></header>
      <p>{{ child.goal }}</p>
      <details v-if="child.report"><summary>{{ tr('查看参考报告', 'View advisory report') }}</summary><pre>{{ child.report.text.slice(0, 16000) }}</pre><small v-if="child.report.text.length > 16000">{{ tr('报告较长，当前显示前 16000 个字符。', 'Showing the first 16000 characters of this report.') }}</small><code>{{ child.resultRef }}</code></details>
    </article>
  </section>
  <section v-if="criteria.length || receipt" class="task-acceptance" data-testid="agent-task-acceptance">
    <h4>{{ tr('验收条件与检查', 'Acceptance criteria and checks') }}</h4>
    <p class="task-limits">{{ tr('条件由 Agent 提出。检查只证明列出的有限断言，不等于已运行测试或功能整体通过。', 'The agent declares these criteria. Checks prove only the listed finite assertions; they do not establish that tests ran or the full feature works.') }}</p>
    <ul v-if="criteria.length"><li v-for="criterion in criteria" :key="criterion.id"><strong>{{ criterionLabel(criterion) }}</strong><details><summary>{{ tr('条件详情', 'Criterion details') }}</summary><pre>{{ JSON.stringify(criterion, null, 2) }}</pre></details></li></ul>
    <p v-if="validReceipt" class="task-check-pass">{{ tr('有限检查通过，当前任务版本有效。', 'Finite checks passed for the current task revision.') }}</p>
    <p v-else-if="receipt?.ok" class="task-limits">{{ tr('保留了一份历史检查记录；当前任务需要重新检查。', 'A previous successful check is retained. The current task needs a new check.') }}</p>
    <p v-else-if="receipt">{{ tr('检查未通过，任务仍需处理。', 'Checks did not pass; work remains.') }}</p>
    <p v-else class="task-limits">{{ tr('尚无检查回执。', 'No check receipt yet.') }}</p>
    <ul v-if="receipt"><li v-for="check in receipt.checks" :key="check.criterionId">{{ check.criterionId }} · {{ check.ok ? tr('断言通过', 'Assertion passed') : tr('断言未通过', 'Assertion failed') }}<small v-if="check.evidenceId"> · {{ check.evidenceId }}</small></li></ul>
  </section>
</template>

<style scoped>
.task-children,.task-acceptance{margin-top:12px;padding-top:10px;border-top:1px solid var(--line,#e5e6e1)}h4{margin:0 0 6px;font-size:12px}.task-limits{color:var(--app-muted,#72746f);line-height:1.55}.task-child{margin:8px 0;padding:8px 10px;background:var(--app-bg,#f5f6f2);border-radius:7px}.task-child header{display:flex;align-items:center;justify-content:space-between;gap:10px}.task-child p{margin:6px 0;white-space:pre-wrap}.task-child details,.task-acceptance details{margin:6px 0}.task-child summary,.task-acceptance summary{cursor:pointer;color:var(--app-muted,#72746f)}pre{max-height:190px;overflow:auto;white-space:pre-wrap;overflow-wrap:anywhere;margin:7px 0;padding:8px;background:var(--app-surface,#fff);border-radius:6px;font-size:11px}code{display:block;font-size:10px;overflow-wrap:anywhere}.task-check-pass{color:#247141}ul{padding-left:20px}li{margin:6px 0}
</style>
