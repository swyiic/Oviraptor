<script setup lang="ts">
import { computed } from 'vue';
import { Activity, BrainCircuit, Braces, Database, FileClock, Globe2, LayoutDashboard, Plus, ShieldAlert, ShieldCheck, TerminalSquare } from '@lucide/vue';
import { useI18n } from '../i18n';
import { workspaceNavigation, type ModuleKey, type WorkspaceNavItem } from './moduleCatalog';
import type { ViewKey } from './types';
const props=defineProps<{module:ModuleKey;view:ViewKey;sentinelMenu:string;runningJobs?:number;blockedCount?:number;fuse?:number}>();
const emit=defineEmits<{navigate:[item:WorkspaceNavItem]}>();
const {t,tr}=useI18n();
const pages=computed(()=>workspaceNavigation(props.module));
const current=computed(()=>pages.value.find(item=>props.view===item.view&&(props.module!=='sentinel'||item.view==='logs'||props.sentinelMenu===item.key))?.key||'');
// Every section is a direct button (the bar wraps when it runs out of room);
// a CTA (e.g. 发现资产) is pushed to the right edge.
const sectionPages=computed(()=>pages.value.filter(item=>!item.cta));
const ctaPages=computed(()=>pages.value.filter(item=>item.cta));
const icons={chat:BrainCircuit,assets:Database,seeds:FileClock,scan:ShieldCheck,hackerone:ShieldCheck,dashboard:LayoutDashboard,tasks:Activity,ownership:ShieldCheck,exposure:Globe2,changes:FileClock,quarantine:ShieldAlert,query:Plus,dialog:BrainCircuit,results:Globe2,skills:Braces};
const pageNames=computed<Record<string,string>>(()=>props.module==='asset'?t.value.pages.asset:props.module==='sentinel'?t.value.pages.sentinel:{});
const label=(item:WorkspaceNavItem)=>pageNames.value[item.key]??tr(...item.label);
function badgeText(item:WorkspaceNavItem){
  const value=item.badge?props[item.badge]:undefined;
  return typeof value==='number'&&Number.isFinite(value)&&value>=0?value.toLocaleString(tr('zh-CN','en-US')):'';
}
function choose(event:Event){const item=pages.value.find(item=>item.key===(event.target as HTMLSelectElement).value);if(item)emit('navigate',item);}
function pick(item:WorkspaceNavItem){emit('navigate',item);}
</script>
<template>
  <nav v-if="pages.length" class="workspace-section-navigation" :aria-label="tr('当前模块栏目','Workspace sections')">
    <select class="workspace-section-select" :aria-label="tr('切换栏目','Choose section')" :value="current" @change="choose"><option v-if="!current" disabled value="">{{tr('选择栏目','Choose section')}}</option><option v-for="item in pages" :key="item.key" :value="item.key">{{label(item)}}{{badgeText(item)?` · ${badgeText(item)}`:''}}</option></select>
    <button v-for="item in sectionPages" :key="item.key" type="button" class="workspace-section-button" :class="{active:current===item.key}" :aria-current="current===item.key?'page':undefined" @click="pick(item)"><TerminalSquare v-if="item.key==='logs'" :size="15"/><component :is="icons[item.icon]" v-else :size="15"/><span>{{label(item)}}</span><small v-if="badgeText(item)" :title="badgeText(item)">{{badgeText(item)}}</small></button>
    <button v-for="item in ctaPages" :key="item.key" type="button" class="workspace-section-button is-action" :class="{active:current===item.key}" :aria-current="current===item.key?'page':undefined" @click="pick(item)"><component :is="icons[item.icon]" :size="15"/><span>{{label(item)}}</span></button>
  </nav>
</template>
<style scoped>
.workspace-section-navigation{display:flex;flex-wrap:wrap;align-items:center;gap:4px;position:sticky;top:64px;z-index:18;min-height:52px;padding:6px calc(var(--page-gutter,28px) - 10px);border-bottom:1px solid var(--line);background:var(--app-bg,#fcfcfb);color:var(--app-ink,#252624)}
.workspace-section-button{display:flex;align-items:center;justify-content:center;gap:6px;height:36px;padding:0 10px;border:0;border-radius:var(--radius-md);background:transparent;color:var(--app-muted,var(--muted));font:inherit;font-size:var(--text-body);white-space:nowrap;cursor:pointer}
.workspace-section-button:hover{color:var(--app-ink);background:color-mix(in srgb,var(--app-ink) 5%,transparent)}.workspace-section-button.active{background:color-mix(in srgb,var(--blue) 10%,transparent);color:var(--blue);font-weight:600}.workspace-section-button small{flex-shrink:0;font-variant-numeric:tabular-nums;font-size:var(--text-micro);padding:1px 5px;border-radius:var(--radius-xs);background:color-mix(in srgb,var(--app-ink) 7%,transparent)}
.workspace-section-button.is-action{margin-left:auto;background:var(--blue);color:#fff}.workspace-section-button.is-action:hover{filter:brightness(.95)}.workspace-section-select{display:none}
@media(max-width:1100px){.workspace-section-button{gap:5px}}
@media(max-width:760px){.workspace-section-button{display:none}.workspace-section-select{display:block;min-width:0;width:100%;height:36px;padding:0 12px;background:var(--app-surface);color:var(--app-ink);border:1px solid var(--line);border-radius:8px;font-size:13px}}
</style>
