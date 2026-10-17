<script setup lang="ts">
import AgentRuntimeSettingsPanel from "../../agentHost/components/AgentRuntimeSettingsPanel.vue";
import { useI18n } from "../../../i18n";
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; openAdvanced: [] }>();
const { tr } = useI18n();
</script>
<template>
  <div class="config-section runtime-model" data-testid="config-section-runtime-model">
    <AgentRuntimeSettingsPanel @notify="(type, text) => emit('notify', type, text)" />
    <div class="legacy-link-row">
      <span>{{ tr("扫描 / 旧版对话模型", "Scan / legacy chat models") }}</span>
      <button class="text-button" type="button" data-testid="open-legacy-models" @click="emit('openAdvanced')">{{ tr("在高级中编辑", "Edit in Advanced") }}</button>
    </div>
  </div>
</template>
<style scoped>
.legacy-link-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-radius: 9px;
  font-size: 12px;
  color: var(--muted);
}
.text-button {
  border: 0;
  background: transparent;
  color: var(--blue, #2878ff);
  font-size: 12px;
  cursor: pointer;
  padding: 0;
}
.text-button:hover { text-decoration: underline; }
</style>
