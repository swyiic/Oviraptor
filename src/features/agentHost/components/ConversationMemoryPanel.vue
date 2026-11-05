<script setup lang="ts">
import { Brain, RefreshCw, Trash2 } from '@lucide/vue';
import { ref, watch } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { useI18n } from '../../../i18n';
import { messageTimestamp } from '../../conversations/messageTime';
import type { AgentHostChatClient, AgentProjectMemoryEntry } from '../chatClient';

const props = defineProps<{ client: AgentHostChatClient; projectId?: number; running?: boolean }>();
const { tr } = useI18n();
const entries = ref<AgentProjectMemoryEntry[]>([]);
const loading = ref(false);
const busy = ref(false);
const error = ref('');

async function load() {
  const projectId = props.projectId;
  if (projectId === undefined) { entries.value = []; return; }
  loading.value = true; error.value = '';
  try { entries.value = await props.client.projectMemory(projectId); }
  catch (e) { error.value = String((e as Error).message || e); }
  finally { loading.value = false; }
}

async function forget(entryId: string) {
  const projectId = props.projectId;
  if (projectId === undefined || busy.value) return;
  busy.value = true; error.value = '';
  try { await props.client.forgetMemory(projectId, entryId); entries.value = entries.value.filter(entry => entry.entryId !== entryId); }
  catch (e) { error.value = String((e as Error).message || e); }
  finally { busy.value = false; }
}

async function clearAll() {
  const projectId = props.projectId;
  if (projectId === undefined || busy.value || !entries.value.length) return;
  if (typeof window !== 'undefined' && !window.confirm(tr('清空本项目的全部跨会话记忆?此操作不可恢复。', 'Clear all cross-session memory for this project? This cannot be undone.'))) return;
  busy.value = true; error.value = '';
  try { await props.client.clearMemory(projectId); entries.value = []; }
  catch (e) { error.value = String((e as Error).message || e); }
  finally { busy.value = false; }
}

function when(value: string) { return messageTimestamp(value, tr('zh-CN', 'en-GB'))?.label || tr('时间未知', 'Time unknown'); }
function kindLabel(kind: string) { return kind === 'lesson' ? tr('经验', 'Lesson') : kind === 'pref' ? tr('偏好', 'Pref') : tr('事实', 'Fact'); }

watch(() => props.projectId, () => void load(), { immediate: true });
// After a run ends the agent may have used remember/forget — reload so the panel stays current.
watch(() => props.running, (now, previous) => { if (previous && !now) void load(); });
</script>
<template>
  <section v-if="projectId !== undefined && (entries.length || error)" class="chat-memory" :aria-label="tr('跨会话记忆', 'Cross-session memory')" data-testid="agent-memory-panel">
    <header>
      <strong><Brain :size="14" /> {{ tr('跨会话记忆', 'Cross-session memory') }} <span>{{ entries.length }}</span></strong>
      <div class="mem-actions">
        <button v-refresh-feedback class="mem-icon" type="button" :disabled="loading" :title="tr('刷新', 'Refresh')" :aria-label="tr('刷新记忆', 'Refresh memory')" @click="load()"><RefreshCw :size="14" :class="{ spinning: loading }" /></button>
        <button v-if="entries.length" class="mem-icon" type="button" :disabled="busy" :title="tr('清空全部', 'Clear all')" :aria-label="tr('清空全部记忆', 'Clear all memory')" @click="clearAll()"><Trash2 :size="14" /></button>
      </div>
    </header>
    <p class="mem-caption">{{ tr('这些是 agent 自己判断值得跨会话记住的参考笔记——是参考,不是证据,也不授予任何权限;每个目标的访问与测试授权仍需单独确认。你可以随时删除。', 'Notes the agent chose to keep across sessions — reference only, not evidence, granting no authority; every target still needs its own authorization. You can delete any of them.') }}</p>
    <ul class="mem-list">
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
    <p v-if="error" class="mem-error" role="alert">{{ error }} <button type="button" @click="load()">{{ tr('重试', 'Retry') }}</button></p>
  </section>
</template>
<style scoped>
.chat-memory{border:1px solid var(--line,#e1e3e7);border-radius:12px;padding:14px 16px;margin:18px 0 22px;min-width:0}
.chat-memory header{display:flex;justify-content:space-between;align-items:center;gap:12px}
.chat-memory header strong{display:inline-flex;align-items:center;gap:6px;font-size:12px}
.chat-memory header strong span{font-weight:400;color:var(--app-muted,#687078)}
.mem-actions{display:inline-flex;gap:6px}
.mem-caption{font-size:11px;color:var(--app-muted,#687078);margin:7px 0 10px;line-height:1.6}
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
@media(max-width:600px){.chat-memory{padding:12px}}
</style>
