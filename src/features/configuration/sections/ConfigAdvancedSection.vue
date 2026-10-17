<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "../../../i18n";
import type { AdvancedSubSection } from "../configCenter";

defineProps<{ sub: AdvancedSubSection }>();
const emit = defineEmits<{ "update:sub": [value: AdvancedSubSection] }>();
const { tr } = useI18n();

const items = computed(() => [
  { id: "profiles" as const, title: tr("运行方案", "Profiles") },
  { id: "rules" as const, title: tr("页面规则", "Page rules") },
  { id: "workers" as const, title: tr("Worker", "Workers") },
  { id: "runtime" as const, title: tr("运行环境", "Environment") },
  { id: "legacy-models" as const, title: tr("账号与代理", "Accounts & proxy") },
]);
</script>
<template>
  <div class="config-section advanced advanced-layout" data-testid="config-section-advanced">
    <nav class="advanced-subnav" data-testid="advanced-subnav" :aria-label="tr('高级分区','Advanced sections')">
      <button
        v-for="item in items"
        :key="item.id"
        type="button"
        :class="{ active: sub === item.id }"
        @click="emit('update:sub', item.id)"
      >{{ item.title }}</button>
    </nav>
    <div class="advanced-body">
      <slot :name="sub" />
      <slot />
    </div>
  </div>
</template>
<style scoped>
.advanced-layout {
  display: grid;
  grid-template-columns: 148px minmax(0, 1fr);
  gap: 16px;
  align-items: start;
  min-height: 0;
}
.advanced-subnav {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: color-mix(in srgb, var(--app-ink, #172033) 3%, transparent);
}
.advanced-subnav button {
  display: block;
  width: 100%;
  height: 32px;
  padding: 0 10px;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: var(--muted);
  font-size: 12px;
  font-weight: 550;
  text-align: left;
  cursor: pointer;
}
.advanced-subnav button:hover {
  color: var(--app-ink, #172033);
  background: color-mix(in srgb, var(--app-ink, #172033) 5%, transparent);
}
.advanced-subnav button.active {
  color: var(--app-ink, #172033);
  background: var(--app-surface, #fff);
  box-shadow: 0 1px 2px #00000012;
}
.advanced-body { min-width: 0; }
@media (max-width: 820px) {
  .advanced-layout { grid-template-columns: 1fr; }
  .advanced-subnav { flex-direction: row; flex-wrap: wrap; }
  .advanced-subnav button { width: auto; }
}
</style>
