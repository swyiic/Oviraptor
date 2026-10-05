<script setup lang="ts">
import type { NativeBudgetDimension } from "../../../types";
import { useI18n } from "../../../i18n";
defineProps<{ dimensions: NativeBudgetDimension[] }>();
const { tr } = useI18n();
const labels: Record<string, [string, string]> = {
  model_input_tokens: ['输入 Token', 'Input tokens'], model_cached_tokens: ['缓存 Token', 'Cached tokens'],
  model_output_tokens: ['输出 Token', 'Output tokens'], model_requests: ['模型请求', 'Model requests'],
  target_requests: ['目标请求', 'Target requests'], browser_actions: ['浏览器动作', 'Browser actions'],
  controlled_writes: ['受控写入', 'Controlled writes'], upload_bytes: ['上传字节', 'Upload bytes'],
  concurrency_batches: ['并发槽位', 'Concurrency slots'], wall_time_ms: ['根任务耗时（毫秒）', 'Root elapsed (ms)'],
};
const coverage: Record<NativeBudgetDimension['coverage'], [string, string]> = {
  assignment_settlement: ['联合子任务逐轮回执；单智能体及根自身未覆盖', 'Per-call joint child receipts; single mode and root work uncovered'],
  multi_agent_broker: ['多智能体请求；单智能体未接入', 'Multi-agent requests; single mode not integrated'],
  multi_agent_lane: ['多智能体槽位', 'Multi-agent slots'],
  admission_samples: ['联合任务采样/限时；单智能体及启动侦察未覆盖', 'Multi-agent samples/deadlines; single mode and preflight uncovered'],
  denied_by_contract: ['未获执行授权', 'Execution not granted'],
};
</script>

<template>
  <div class="native-budget-vector">
    <p>{{ tr('追加账目记录已接入的费用路径；仍有覆盖缺口，不代表十维完整验收。未决成本不会因取消或人工声明退款。',
      'Entries describe integrated cost paths with remaining gaps; they do not prove complete acceptance. Cancellation or operator statements never refund unknown costs.') }}</p>
    <table>
      <thead><tr><th>{{ tr('维度', 'Dimension') }}</th><th>{{ tr('上限', 'Limit') }}</th><th>{{ tr('已用', 'Consumed') }}</th><th>{{ tr('预留', 'Reserved') }}</th><th>{{ tr('未决', 'Unknown') }}</th><th>{{ tr('记录范围', 'Coverage') }}</th></tr></thead>
      <tbody><tr v-for="row in dimensions" :key="row.dimension">
        <th scope="row">{{ labels[row.dimension] ? tr(...labels[row.dimension]) : row.dimension }}</th>
        <td>{{ row.hardLimit ?? tr('不限额', 'Unlimited') }}</td><td>{{ row.consumed }}</td><td>{{ row.reserved }}</td>
        <td :class="{ unresolved: row.indeterminate > 0 }">{{ row.indeterminate }}</td><td>{{ tr(...coverage[row.coverage]) }}</td>
      </tr></tbody>
    </table>
  </div>
</template>

<style scoped>
.native-budget-vector { overflow-x: auto; }
table { width: 100%; border-collapse: collapse; }
th, td { text-align: left; padding: 5px; border-bottom: 1px solid #d7e3f3; }
.unresolved { color: #a63812; font-weight: 700; }
</style>
