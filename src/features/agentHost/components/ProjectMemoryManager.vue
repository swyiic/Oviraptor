<script setup lang="ts">
import { Brain, RefreshCw, Trash2 } from '@lucide/vue';
import { onMounted, ref } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { confirmInPlace } from '../../../shared/confirmInPlace';
import { useI18n } from '../../../i18n';
import { messageTimestamp } from '../../conversations/messageTime';
import { resolveAgentHostEndpoint } from '../api';
import { createAgentHostChatClient, type AgentHostChatClient, type AgentProjectMemoryEntry } from '../chatClient';

type Scope = 'project' | 'global';
const { tr } = useI18n();
const client = ref<AgentHostChatClient | null>(null);
const projects = ref<{ id: number; name: string }[]>([]);
const selected = ref<number | null>(null);
const scope = ref<Scope>('project');
const entries = ref<AgentProjectMemoryEntry[]>([]);
const loading = ref(false);
const busy = ref(false);
const error = ref('');
const unavailable = ref(false);

async function init() {
  const endpoint = await resolveAgentHostEndpoint();
  if (!endpoint) { unavailable.value = true; return; }
  client.value = createAgentHostChatClient(endpoint);
  try {
    projects.value = await client.value.projects();
    if (projects.value.length) selected.value = projects.value[0].id;
    await load();
  } catch (e) { error.value = String((e as Error).message || e); }
}

async function load() {
  if (!client.value) { entries.value = []; return; }
  if (scope.value === 'global') {
    loading.value = true; error.value = '';
    try { entries.value = await client.value.globalMemory(); }
    catch (e) { error.value = String((e as Error).message || e); }
    finally { loading.value = false; }
    return;
  }
  if (selected.value == null) { entries.value = []; return; }
  loading.value = true; error.value = '';
  try { entries.value = await client.value.projectMemory(selected.value); }
  catch (e) { error.value = String((e as Error).message || e); }
  finally { loading.value = false; }
}

async function setScope(value: Scope) {
  if (scope.value === value) return;
  scope.value = value;
  await load();
}

async function onProjectChange(event: Event) { selected.value = Number((event.target as HTMLSelectElement).value); await load(); }

async function forget(entryId: string) {
  if (!client.value || busy.value) return;
  busy.value = true; error.value = '';
  try {
    if (scope.value === 'global') await client.value.forgetGlobalMemory(entryId);
    else { if (selected.value == null) return; await client.value.forgetMemory(selected.value, entryId); }
    entries.value = entries.value.filter(entry => entry.entryId !== entryId);
  } catch (e) { error.value = String((e as Error).message || e); }
  finally { busy.value = false; }
}

async function clearAll() {
  if (!client.value || selected.value == null || busy.value || !entries.value.length) return;
  await confirmInPlace({
    title: tr('清空该项目的全部跨会话记忆？', 'Clear all cross-session memory for this project?'),
    detail: tr('此操作不可恢复。', 'This cannot be undone.'),
    confirmText: tr('清空', 'Clear'), busyText: tr('清空中…', 'Clearing…'), cancelText: tr('取消', 'Cancel'),
    run: async () => {
      if (!client.value || selected.value == null) return;
      busy.value = true; error.value = '';
      try { await client.value.clearMemory(selected.value); entries.value = []; }
      catch (e) { error.value = String((e as Error).message || e); }
      finally { busy.value = false; }
    },
  });
}

function when(value: string) { return messageTimestamp(value, tr('zh-CN', 'en-GB'))?.label || tr('时间未知', 'Time unknown'); }
function kindLabel(kind: string) { return kind === 'lesson' ? tr('经验', 'Lesson') : kind === 'pref' ? tr('偏好', 'Pref') : tr('事实', 'Fact'); }

onMounted(init);
</script>
<template>
  <section class="mem-mgr" data-testid="project-memory-manager">
    <header>
      <strong><Brain :size="15" /> {{ tr('跨会话记忆', 'Cross-session memory') }}</strong>
      <button v-if="!unavailable" v-refresh-feedback class="mem-icon" type="button" :disabled="loading || !client" :title="tr('刷新', 'Refresh')" :aria-label="tr('刷新记忆', 'Refresh memory')" @click="load()"><RefreshCw :size="14" :class="{ spinning: loading }" /></button>
    </header>
    <p class="mem-desc">{{ tr('agent 自己记下、下次运行会自动看到的参考笔记——按项目记,或标成全局跨所有工作空间通用。是参考,不是证据,也不授予任何权限;每个目标的访问与测试授权仍需单独确认。你可以在这里查看并删除。', 'Reference notes the agent keeps and sees on its next run — scoped to one project, or marked global to apply across every workspace. Reference only — not evidence, granting no authority; every target still needs its own authorization. Review and delete them here.') }}</p>
    <p v-if="unavailable" class="mem-muted" role="status">{{ tr('agent 运行时未连接,暂时无法读取记忆。', 'Agent runtime not connected; memory is unavailable right now.') }}</p>
    <template v-else>
      <div class="mem-scope-tabs" role="tablist">
        <button type="button" class="mem-tab" :class="{ active: scope === 'project' }" role="tab" :aria-selected="scope === 'project'" @click="setScope('project')">{{ tr('按项目', 'Per project') }}</button>
        <button type="button" class="mem-tab" :class="{ active: scope === 'global' }" role="tab" :aria-selected="scope === 'global'" @click="setScope('global')">{{ tr('全局', 'Global') }}</button>
      </div>
      <div class="mem-row">
        <label v-if="scope === 'project'" class="mem-field">{{ tr('项目', 'Project') }}
          <select :value="selected ?? ''" :disabled="!projects.length" @change="onProjectChange">
            <option v-for="project in projects" :key="project.id" :value="project.id">{{ project.name }}</option>
          </select>
        </label>
        <small v-else class="mem-muted">{{ tr('跨所有工作空间通用的记忆。', 'Memory shared across every workspace.') }}</small>
        <button v-if="scope === 'project' && entries.length" class="mem-icon" type="button" :disabled="busy" :title="tr('清空全部', 'Clear all')" :aria-label="tr('清空全部记忆', 'Clear all memory')" @click="clearAll()"><Trash2 :size="14" /></button>
      </div>
      <ul v-if="entries.length" class="mem-list">
        <li v-for="entry in entries" :key="entry.entryId" class="mem-card">
          <div class="mem-body">
            <span class="mem-kind" :data-kind="entry.kind">{{ kindLabel(entry.kind) }}</span>
            <p class="mem-note">{{ entry.note }}</p>
            <div v-if="entry.tags.length" class="mem-tags"><span v-for="tag in entry.tags" :key="tag">#{{ tag }}</span></div>
            <small>{{ when(entry.updatedAt) }}</small>
          </div>
          <button class="mem-icon mem-del" type="button" :disabled="busy" :title="tr('删除这条', 'Delete')" :aria-label="tr('删除这条记忆', 'Delete this memory')" @click="forget(entry.entryId)"><Trash2 :size="15" /></button>
        </li>
      </ul>
      <p v-else-if="!loading && !error" class="mem-muted" role="status">{{ scope === 'global' ? tr('还没有全局记忆。', 'No global memory yet.') : (projects.length ? tr('该项目还没有记忆。', 'No memory for this project yet.') : tr('还没有项目。', 'No projects yet.')) }}</p>
      <p v-if="error" class="mem-error" role="alert">{{ error }} <button type="button" @click="load()">{{ tr('重试', 'Retry') }}</button></p>
    </template>
  </section>
</template>
<style scoped>
.mem-mgr{border:1px solid var(--line,#e1e3e7);border-radius:12px;padding:16px 18px;margin:18px 0 0;min-width:0}
.mem-mgr header{display:flex;justify-content:space-between;align-items:center;gap:12px}
.mem-mgr header strong{display:inline-flex;align-items:center;gap:7px;font-size:var(--text-body)}
.mem-desc{font-size:11px;color:var(--app-muted,#687078);margin:8px 0 12px;line-height:1.6}
.mem-muted{font-size:12px;color:var(--app-muted,#687078);margin:8px 0}
.mem-scope-tabs{display:flex;gap:6px;margin-bottom:10px}
.mem-tab{font-size:12px;padding:5px 11px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;color:var(--app-muted,#687078);cursor:pointer}
.mem-tab.active{color:var(--app-ink,#1c2024);border-color:var(--blue,#416b9c);background:var(--app-surface,#fff)}
.mem-row{display:flex;justify-content:space-between;align-items:flex-end;gap:12px;margin-bottom:10px}
.mem-field{display:flex;flex-direction:column;gap:5px;font-size:11px;color:var(--app-muted,#687078)}
.mem-field select{font-size:12px;padding:6px 9px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:var(--app-surface,#fff);color:inherit;min-width:180px}
.mem-list{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:9px}
.mem-card{display:flex;align-items:flex-start;gap:10px;border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:11px;min-width:0}
.mem-body{flex:1;min-width:0;display:flex;flex-direction:column;gap:5px}
.mem-kind{align-self:flex-start;font-size:var(--text-micro);font-weight:600;color:var(--blue,#416b9c);border:1px solid var(--line,#e1e3e7);border-radius:6px;padding:1px 7px}
.mem-kind[data-kind="lesson"]{color:var(--green,#3f8f5a)}
.mem-kind[data-kind="pref"]{color:var(--purple,#7a5bbf)}
.mem-note{margin:0;font-size:12px;line-height:1.6;overflow-wrap:anywhere;white-space:pre-wrap}
.mem-tags{display:flex;flex-wrap:wrap;gap:5px}
.mem-tags span{font-size:var(--text-micro);color:var(--app-muted,#687078)}
.mem-body small{font-size:var(--text-micro);color:var(--app-muted,#687078)}
.mem-icon{display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;padding:6px;color:inherit;flex-shrink:0;cursor:pointer}
.mem-del:hover:not(:disabled){color:var(--red,#b63f3f);border-color:var(--red,#b63f3f)}
.mem-error{color:var(--red,#b63f3f);font-size:12px;overflow-wrap:anywhere}
.spinning{animation:mem-spin .8s linear infinite}
@keyframes mem-spin{to{transform:rotate(360deg)}}
</style>
