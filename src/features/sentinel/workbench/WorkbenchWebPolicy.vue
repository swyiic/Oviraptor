<script setup lang="ts">
import { Check } from "@lucide/vue";
import { useI18n } from "../../../i18n";

defineProps<{
  scanMode: "quick" | "standard" | "deep";
  summary: { contracts: number; discovery: number; verifiers: number; chain: boolean };
  skillNames: string[];
}>();
const { tr } = useI18n();
</script>

<template>
  <section class="effective-policy-card span-two">
    <header>
      <div>
        <strong>{{ tr("模式策略预览", "Mode policy preview") }}</strong>
        <span>{{ tr("展示该模式的默认策略；实际执行以后台解析、授权与准入结果为准。", "Default policy for this mode; actual execution depends on backend resolution, authorization and admission.") }}</span>
      </div>
      <em>{{ scanMode.toUpperCase() }}</em>
    </header>
    <dl>
      <div><dt>{{ tr("验证契约", "Contracts") }}</dt><dd>≤ {{ summary.contracts }}</dd></div>
      <div><dt>{{ tr("定向发现", "Discovery") }}</dt><dd>{{ summary.discovery }} {{ tr("轮", "passes") }}</dd></div>
      <div><dt>{{ tr("验证器", "Verifiers") }}</dt><dd>≤ {{ summary.verifiers }}</dd></div>
      <div><dt>{{ tr("攻击链关联", "Attack chains") }}</dt><dd>{{ summary.chain ? tr("启用", "On") : tr("仅单点证据", "Single findings") }}</dd></div>
    </dl>
    <p><Check :size="13" />{{ skillNames.join(" + ") }}</p>
    <small>{{ tr("可用能力由后台检查；实际启用受范围、授权与运行环境约束。最终执行计划见任务详情。", "The backend checks available capabilities against scope, authorization and environment. See task details for the final execution plan.") }}</small>
  </section>
</template>
