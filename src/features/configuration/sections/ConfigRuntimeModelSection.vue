<script setup lang="ts">
import AgentRuntimeSettingsPanel from "../../agentHost/components/AgentRuntimeSettingsPanel.vue";
import { useI18n } from "../../../i18n";
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; openAdvanced: [] }>();
const { tr } = useI18n();
</script>
<template>
  <div class="config-section runtime-model" data-testid="config-section-runtime-model">
    <AgentRuntimeSettingsPanel @notify="(type, text) => emit('notify', type, text)" />
    <p class="legacy-line">
      <button type="button" class="text-button" data-testid="open-legacy-models" @click="emit('openAdvanced')">{{ tr("扫描 / 旧版模型 → 高级", "Scan / legacy models → Advanced") }}</button>
    </p>
  </div>
</template>
<style scoped>
.legacy-line{margin:10px 0 0;text-align:right}
.text-button{border:0;background:transparent;color:var(--blue,#2878ff);font-size:12px;cursor:pointer;padding:0}
.text-button:hover{text-decoration:underline}
</style>
