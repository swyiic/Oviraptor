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
        <strong>{{ tr("本次实际生效策略", "Effective policy") }}</strong>
        <span>{{ tr("两个 Web 入口统一由后台解析，不再依赖页面默认值。", "Both Web entry points use the same backend policy resolver.") }}</span>
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
    <small>{{ tr("只读、原始 HTTP、有界竞争、按契约受控写入和攻击链能力自动准备；每任务 OAST 会检测目标到当前工作站的回连可达性。", "Read-only, raw HTTP, bounded race, contract-gated write, and attack-chain capabilities are prepared automatically. Per-task OAST probes whether the target can route back to this workstation.") }}</small>
  </section>
</template>
