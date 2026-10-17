<script setup lang="ts">
import AgentRuntimeSettingsPanel from "../../agentHost/components/AgentRuntimeSettingsPanel.vue";
import { useI18n } from "../../../i18n";
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; openAdvanced: [] }>();
const { tr } = useI18n();
</script>
<template>
  <div class="config-section runtime-model" data-testid="config-section-runtime-model">
    <AgentRuntimeSettingsPanel @notify="(type, text) => emit('notify', type, text)" />
    <section class="panel config-section-note">
      <h3>{{ tr("扫描 / 旧版对话模型（高级）", "Scan / legacy chat models (advanced)") }}</h3>
      <p class="helper">
        {{ tr("问答主路径是上方的 xAI Grok（OAuth）。旧版「运行方案」里的 deepseek 等模型档案只用于扫描与旧版对话，请到「高级」分区编辑。", "Primary Q&A uses xAI Grok (OAuth) above. Legacy model profiles (e.g. deepseek) in Runtime profiles are for scans and legacy chat — edit them under Advanced.") }}
      </p>
      <button class="button ghost compact" type="button" data-testid="open-legacy-models" @click="emit('openAdvanced')">{{ tr("打开高级 · 运行方案", "Open Advanced · Runtime profiles") }}</button>
    </section>
  </div>
</template>
<style scoped>
.config-section-note { margin-top: 16px; padding: 16px 18px; }
.config-section-note h3 { margin: 0 0 6px; font-size: 14px; }
.config-section-note .helper { margin: 0 0 10px; font-size: 12px; color: var(--muted); line-height: 1.6; }
</style>
