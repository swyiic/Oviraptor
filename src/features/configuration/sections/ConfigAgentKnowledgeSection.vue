<script setup lang="ts">
// Legacy landing spot for old deep links ("agent-knowledge" / "memory" /
// "techniques"). The managers themselves moved to the top-level "库" (Library)
// view (see docs/AGENT_TOP_LEVEL_REFACTOR.md §6) so they are reused from one
// place instead of being mounted here too.
import { useI18n } from "../../../i18n";

const emit = defineEmits<{ "open-library": [] }>();
const { tr } = useI18n();
</script>
<template>
  <section class="panel config-domain-panel agent-knowledge-moved">
    <p class="config-domain-description">{{ tr('记忆库、技法卡和工具库已经搬到顶层的「库」区,不再属于配置中心——它们是跨工作空间共享的参考素材,不是某一项设置。', 'Memory, technique cards and the tool library moved to the top-level “Library” area — they are advisory material shared across workspaces, not a setting.') }}</p>
    <button class="button secondary compact" type="button" @click="emit('open-library')">{{ tr('前往库', 'Open Library') }}</button>
  </section>
</template>
<style scoped>
.agent-knowledge-moved{display:flex;flex-direction:column;gap:12px;align-items:flex-start}
</style>
