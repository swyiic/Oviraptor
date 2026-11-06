<script setup lang="ts">
// Top-level "库" (Library): the agent's cross-workspace advisory material —
// memory, technique cards and the tool library — promoted out of the config
// center (see docs/AGENT_TOP_LEVEL_REFACTOR.md §6). All three are reference
// only: they shape how the agent reasons, never what it may act on, and never
// feed any auth/scope/settlement gate.
import { BrainCircuit, BookOpen, Wrench } from "@lucide/vue";
import { ref, type Component } from "vue";
import ProjectMemoryManager from "../agentHost/components/ProjectMemoryManager.vue";
import ResearchCardsManager from "../agentHost/components/ResearchCardsManager.vue";
import AgentToolsManager from "../agentHost/components/AgentToolsManager.vue";
import { useI18n } from "../../i18n";

type LibraryTab = "memory" | "techniques" | "tools";
const { tr } = useI18n();
const tab = ref<LibraryTab>("memory");
const tabs: Array<{ id: LibraryTab; icon: Component; label: [string, string] }> = [
  { id: "memory", icon: BrainCircuit, label: ["记忆库", "Memory"] },
  { id: "techniques", icon: BookOpen, label: ["技法卡", "Techniques"] },
  { id: "tools", icon: Wrench, label: ["工具库", "Tools"] },
];
</script>
<template>
  <section class="library-workspace">
    <p class="library-description">{{ tr('agent 跨工作空间共享的参考素材:它自己记下的跨会话记忆、你审入的外部技法卡、以及它自己写过并验证过的可复用脚本。三者都是参考,不是授权——只影响 agent 怎么想,不改变它能对谁做什么;每个目标仍需单独确认。', 'Advisory material the agent shares across workspaces: the cross-session memory it keeps, the technique cards you ingest, and the scripts it has written and verified for reuse. All three are reference, not authorization — they shape how it thinks, never what it may act on; every target still needs its own authorization.') }}</p>
    <div class="library-tabs" role="tablist">
      <button v-for="item in tabs" :key="item.id" type="button" class="library-tab" role="tab" :aria-selected="tab === item.id" :class="{ active: tab === item.id }" @click="tab = item.id">
        <component :is="item.icon" :size="15" />{{ tr(item.label[0], item.label[1]) }}
      </button>
    </div>
    <ProjectMemoryManager v-if="tab === 'memory'" />
    <ResearchCardsManager v-else-if="tab === 'techniques'" />
    <AgentToolsManager v-else-if="tab === 'tools'" />
  </section>
</template>
<style scoped>
.library-workspace { display: flex; flex-direction: column; max-width: 920px; }
.library-description { margin: 0 0 14px; font-size: 12px; line-height: 1.6; color: var(--app-muted, #687078); }
.library-tabs { display: flex; gap: 6px; flex-wrap: wrap; }
.library-tab { display: inline-flex; align-items: center; gap: 6px; font-size: 13px; padding: 7px 14px; border: 1px solid var(--line, #e1e3e7); border-radius: 9px; background: transparent; color: var(--app-muted, #687078); cursor: pointer; }
.library-tab.active { color: var(--app-ink, #1c2024); border-color: var(--blue, #416b9c); background: var(--app-surface, #fff); font-weight: 550; }
</style>
