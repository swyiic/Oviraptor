<script setup lang="ts">
import { computed } from "vue";
import { Database, MessageSquare, Search, ShieldCheck } from "@lucide/vue";
import { useI18n } from "../i18n";
import type { PluginEnabledState } from "../plugins/types";
import type { ViewKey } from "./types";
import { visibleModules, type ModuleKey } from "./moduleCatalog";
const props = defineProps<{ collapsed?: boolean; module: ModuleKey; view: ViewKey; sentinelMenu: string; hackerOneEnabled: boolean; enabledPlugins?: PluginEnabledState; runningJobs?: number; blockedCount?: number; fuse?: number }>();
const emit = defineEmits<{ moduleChange: [module: ModuleKey]; searchChat: [] }>();
const { t, tr } = useI18n();
const modules = computed(() => visibleModules(props.hackerOneEnabled,props.enabledPlugins));
const icons = { chat: MessageSquare, assets: Database, scan: ShieldCheck, hackerone: ShieldCheck };
</script>
<template>
  <div class="workspace-navigation" :class="{collapsed}">
    <nav class="workspace-modules workspace-primary-modules" :class="{collapsed}" :aria-label="tr('主要功能','Main navigation')">
      <button v-for="item in modules" :key="item.id" :title="t.modules[item.id]??tr(...item.label)" :aria-label="t.modules[item.id]??tr(...item.label)" :class="{active:module===item.id&&!['settings','projects','skills'].includes(view)}" :aria-current="module===item.id&&!['settings','projects','skills'].includes(view)?'page':undefined" @click="emit('moduleChange',item.id)">
        <component :is="icons[item.id==='chat'?'chat':item.id==='asset'?'assets':'scan']" :size="18"/><span>{{t.modules[item.id]??tr(...item.label)}}</span>
      </button>
    </nav>
    <button class="workspace-find-chat" :title="`${t.shell.find} · ⌘K / Ctrl+K`" :aria-label="t.shell.find" @click="emit('searchChat')"><Search :size="18"/><span>{{t.shell.find}}</span></button>
    <div v-show="!collapsed&&view==='chat'" class="workspace-chat-history"><slot name="history"/></div>
    <div v-show="!collapsed" class="workspace-selector"><slot name="workspace"/></div>
  </div>
</template>
<style scoped>
.workspace-navigation{display:flex;flex-direction:column;flex:1;min-height:0;min-width:0}
.workspace-chat-history{flex:1;min-height:0;overflow:hidden}
.workspace-selector{flex-shrink:0;margin-top:auto}

.workspace-modules{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:4px;margin:2px 0 16px;flex-shrink:0}
.workspace-modules button{display:flex;flex-direction:column;justify-content:center;align-items:center;gap:5px;width:100%;min-height:50px;padding:6px;border:0;border-radius:9px;background:transparent;color:var(--app-muted,var(--muted));font-size:11px;font-weight:550}
.workspace-modules button:hover{background:color-mix(in srgb,var(--app-ink,#172033) 7%,transparent);color:var(--app-ink,#172033)}
.workspace-modules button.active{background:color-mix(in srgb,var(--blue) 12%,transparent);color:var(--blue)}
.workspace-find-chat{display:flex;align-items:center;justify-content:center;gap:6px;min-height:42px;border:0;border-radius:8px;background:transparent;color:var(--app-muted,var(--muted));font-size:var(--text-micro);flex-shrink:0}
.workspace-find-chat:hover{color:var(--app-ink);background:color-mix(in srgb,var(--app-ink) 7%,transparent)}
.workspace-modules.collapsed{display:flex;flex-direction:column;align-items:center}.workspace-modules.collapsed button{width:40px;padding:0}.workspace-modules.collapsed button span,.collapsed .workspace-find-chat span{display:none}
</style>
