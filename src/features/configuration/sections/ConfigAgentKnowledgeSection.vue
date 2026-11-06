<script setup lang="ts">
// Config-center section "记忆与技法": the agent's two kinds of advisory material —
// its own cross-session project memory, and operator-ingested technique cards.
// Both are reference only; they shape how the agent reasons, never what it may
// act on, and never feed any auth/scope/settlement/capability gate.
import ProjectMemoryManager from "../../agentHost/components/ProjectMemoryManager.vue";
import ResearchCardsManager from "../../agentHost/components/ResearchCardsManager.vue";
import { useI18n } from "../../../i18n";

const { tr } = useI18n();
</script>
<template>
  <section class="panel config-domain-panel agent-knowledge">
    <p class="config-domain-description">{{ tr('agent 的两类参考素材:它在各项目自己记下的跨会话记忆,以及你审入的外部技法卡。两者都是参考、不是授权——只影响 agent 怎么想,不改变它能对谁做什么。', 'The agent\'s two kinds of advisory material: the cross-session memory it keeps per project, and the technique cards you ingest. Both are reference, not authorization — they shape how it thinks, never what it may act on.') }}</p>
    <ProjectMemoryManager />
    <ResearchCardsManager />
  </section>
</template>
<style scoped>
.agent-knowledge{display:flex;flex-direction:column}
.agent-knowledge .config-domain-description{margin-bottom:2px}
</style>
