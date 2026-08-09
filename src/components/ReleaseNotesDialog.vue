<script setup lang="ts">
import { useI18n } from "../i18n";
import ModalShell from "./ModalShell.vue";

defineProps<{ version: string }>();
defineEmits<{ close: [] }>();
const { tr } = useI18n();
</script>

<template>
  <ModalShell :title="tr(`Oviraptor v${version} 更新说明`, `Oviraptor v${version} release notes`)" @close="$emit('close')">
    <template #eyebrow><span class="eyebrow">RELEASE · 2026-08-09</span></template>
    <div class="release-notes">
      <article>
        <strong>{{ tr("修复健康检查误触发无进展熔断", "Health checks no longer trigger the no-progress fuse") }}</strong>
        <p>{{ tr("模型启动时的 OK 检查继续计入 Token 成本，但不再算作正式扫描轮次或失败。Strix 第一轮真实工具调用可以正常执行。", "The startup OK check remains in token accounting but no longer counts as a scan turn or failure, allowing Strix's first real tool call to run.") }}</p>
      </article>
      <article>
        <strong>{{ tr("调查图谱跟随内容区自适应", "The investigation graph follows its actual container") }}</strong>
        <p>{{ tr("因果链按实际结果区宽度切换四列、两列或单列，长路径不再裁掉右侧假设；验证契约默认只展示最高价值的 12 条。", "The causal chain switches between four, two, and one columns based on result-area width. Long paths no longer clip hypotheses, and only the top 12 contracts render by default.") }}</p>
      </article>
      <article>
        <strong>{{ tr("任务与成本取消常驻双栏", "Task and cost no longer reserves two permanent columns") }}</strong>
        <p>{{ tr("任务列表使用完整内容宽度，选中任务后在下方展开紧凑详情；成本卡、目标和操作按钮都减少了无效占位。", "The task list uses the full content width and expands a compact detail panel below the selection, reducing wasted space across cost cards, targets, and actions.") }}</p>
      </article>
      <article>
        <strong>{{ tr("维护请求与扫描请求可解释", "Maintenance and scan requests are now distinguishable") }}</strong>
        <p>{{ tr("运行轨迹分别标注模型健康检查、上下文压缩和正式 Agent 请求，累计 Token 不丢失，扫描轮次也不会再虚增。", "Runtime traces label health checks, context compaction, and real agent calls separately without losing token totals or inflating scan turns.") }}</p>
      </article>
      <article>
        <strong>{{ tr("现有数据库无需迁移", "No database migration is required") }}</strong>
        <p>{{ tr("本次只修正请求分类、熔断判断与界面布局；原任务、尝试账本、证据和累计成本保持不变。", "This release only adjusts request classification, fuse decisions, and layout. Existing tasks, attempt ledgers, evidence, and cumulative costs remain unchanged.") }}</p>
      </article>
    </div>
  </ModalShell>
</template>
