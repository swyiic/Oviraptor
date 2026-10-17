<script setup lang="ts">
import { computed } from "vue";
import { SlidersHorizontal, ShieldAlert, Server, TerminalSquare, KeyRound } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { AdvancedSubSection } from "../configCenter";

defineProps<{ sub: AdvancedSubSection }>();
const emit = defineEmits<{ "update:sub": [value: AdvancedSubSection] }>();
const { tr } = useI18n();

const items = computed(() => [
  { id: "profiles" as const, icon: SlidersHorizontal, title: tr("运行方案 / 旧版模型", "Runtime profiles / legacy models"), hint: tr("扫描参数与旧版对话模型档案", "Scan policy and legacy chat model profiles") },
  { id: "rules" as const, icon: ShieldAlert, title: tr("页面与划词规则", "Page and text rules"), hint: tr("项目隔离规则", "Project exclusions") },
  { id: "workers" as const, icon: Server, title: tr("Worker 节点", "Worker nodes"), hint: tr("远程执行", "Remote execution") },
  { id: "runtime" as const, icon: TerminalSquare, title: tr("运行环境", "Runtime environment"), hint: tr("依赖检测与安装", "Dependency check and install") },
  { id: "legacy-models" as const, icon: KeyRound, title: tr("账号与代理提示", "Accounts & proxy tip"), hint: tr("FOFA / HackerOne / Clash 在运行方案编辑器中", "FOFA / HackerOne / Clash live in the profile editor") },
]);
</script>
<template>
  <div class="config-section advanced" data-testid="config-section-advanced">
    <nav class="advanced-subnav" data-testid="advanced-subnav">
      <button
        v-for="item in items"
        :key="item.id"
        type="button"
        :class="{ active: sub === item.id }"
        @click="emit('update:sub', item.id)"
      >
        <component :is="item.icon" :size="16" />
        <span><b>{{ item.title }}</b><small>{{ item.hint }}</small></span>
      </button>
    </nav>
    <div class="advanced-body">
      <slot :name="sub" />
      <slot />
    </div>
  </div>
</template>
<style scoped>
.advanced-subnav {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 16px;
}
.advanced-subnav button {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  text-align: left;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface, transparent);
  cursor: pointer;
  min-width: 180px;
  flex: 1 1 180px;
}
.advanced-subnav button.active {
  border-color: var(--accent, #2878ff);
  box-shadow: 0 0 0 1px var(--accent, #2878ff) inset;
}
.advanced-subnav b { display: block; font-size: 13px; }
.advanced-subnav small { display: block; font-size: 11px; color: var(--muted); margin-top: 2px; }
</style>
