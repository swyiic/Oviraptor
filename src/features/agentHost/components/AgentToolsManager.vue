<script setup lang="ts">
// Tool library panel (design §3 D3 / §7). The agent writes and verifies a
// script inside some workspace; once sunk into this global table it is
// recalled, advisory-only, for later targets — same wall as memory and
// technique cards: a suggestion to reuse, never auto-run, never feeding any
// auth/scope/settlement gate. Backend commands (list/decide/delete/sink) are
// the P2 contract in docs/AGENT_TOP_LEVEL_REFACTOR.md §3/§7; this panel
// degrades to a plain error state (like ResearchCardsManager) until they land.
import { Wrench, RefreshCw, Trash2, ShieldCheck, ShieldBan, Plus, Code2 } from '@lucide/vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { onMounted, ref } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { useI18n } from '../../../i18n';

interface AgentTool {
  id: string;
  title: string;
  language: 'node' | 'python';
  code: string;
  targetFingerprint: string;
  algoTags: string[];
  sourceConversationId: string | null;
  verified: boolean;
  status: 'pending' | 'approved' | 'rejected';
  createdAt: string;
}
type Filter = 'pending' | 'approved' | 'rejected' | 'all';

const { tr } = useI18n();
const tools = ref<AgentTool[]>([]);
const filter = ref<Filter>('pending');
const loading = ref(false);
const busy = ref(false);
const error = ref('');

const showSink = ref(false);
const sinkForm = ref({ title: '', conversationId: '', path: '' });
const sinking = ref(false);
const sinkError = ref('');

const filters: Filter[] = ['pending', 'approved', 'rejected', 'all'];
function filterLabel(value: Filter) {
  return value === 'approved' ? tr('已批准', 'Approved')
    : value === 'rejected' ? tr('已拒绝', 'Rejected')
      : value === 'all' ? tr('全部', 'All')
        : tr('待审', 'Pending');
}
function statusLabel(status: string) {
  return status === 'approved' ? tr('已批准', 'Approved')
    : status === 'rejected' ? tr('已拒绝', 'Rejected')
      : tr('待审', 'Pending');
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    tools.value = await invoke<AgentTool[]>('list_agent_tools', {
      status: filter.value === 'all' ? null : filter.value,
    });
  } catch (e) {
    error.value = String((e as Error).message || e);
  } finally {
    loading.value = false;
  }
}

async function setFilter(value: Filter) {
  if (filter.value === value) return;
  filter.value = value;
  await load();
}

async function pickPath() {
  const path = await open({ multiple: false, directory: false });
  if (path && !Array.isArray(path)) sinkForm.value.path = path;
}

async function sinkFromWorkspace() {
  if (sinking.value || !sinkForm.value.conversationId.trim() || !sinkForm.value.path.trim()) return;
  sinking.value = true;
  sinkError.value = '';
  try {
    await invoke('sink_agent_tool_from_workspace', {
      conversationId: sinkForm.value.conversationId.trim(),
      path: sinkForm.value.path.trim(),
      title: sinkForm.value.title.trim(),
    });
    sinkForm.value = { title: '', conversationId: '', path: '' };
    showSink.value = false;
    filter.value = 'pending';
    await load();
  } catch (e) {
    sinkError.value = String((e as Error).message || e);
  } finally {
    sinking.value = false;
  }
}

async function decide(id: string, approve: boolean) {
  if (busy.value) return;
  busy.value = true;
  error.value = '';
  try {
    await invoke('decide_agent_tool', { id, approve });
    await load();
  } catch (e) {
    error.value = String((e as Error).message || e);
  } finally {
    busy.value = false;
  }
}

async function remove(id: string) {
  if (busy.value) return;
  if (typeof window !== 'undefined'
    && !window.confirm(tr('删除这个脚本?此操作不可恢复。', 'Delete this script? This cannot be undone.'))) return;
  busy.value = true;
  error.value = '';
  try {
    await invoke('delete_agent_tool', { id });
    tools.value = tools.value.filter(item => item.id !== id);
  } catch (e) {
    error.value = String((e as Error).message || e);
  } finally {
    busy.value = false;
  }
}

onMounted(load);
</script>
<template>
  <section class="atm-mgr" data-testid="agent-tools-manager">
    <header>
      <strong><Wrench :size="15" /> {{ tr('工具库', 'Tool library') }}</strong>
      <div class="atm-head-actions">
        <button class="atm-icon" type="button" :disabled="busy" :title="tr('从工作区入库', 'Sink from workspace')" :aria-label="tr('从工作区入库', 'Sink from workspace')" @click="showSink = !showSink"><Plus :size="14" /></button>
        <button v-refresh-feedback class="atm-icon" type="button" :disabled="loading" :title="tr('刷新', 'Refresh')" :aria-label="tr('刷新工具库', 'Refresh tool library')" @click="load()"><RefreshCw :size="14" :class="{ spinning: loading }" /></button>
      </div>
    </header>
    <p class="atm-desc">{{ tr('agent 自己写、在某个工作空间验证过的脚本,沉淀在这里跨工作空间复用。批准后它才会作为"可复用这个脚本"的建议注入新对话——是参考,不是证据;复用前 agent 仍须重读一遍、在新目标沙箱里重新验证。', 'Scripts the agent wrote and verified in some workspace, kept here for reuse across workspaces. Once approved it is suggested — "you could reuse this script" — to new conversations: reference, not evidence; the agent must still re-read and re-verify it against the new target before use.') }}</p>

    <form v-if="showSink" class="atm-sink" @submit.prevent="sinkFromWorkspace">
      <div class="atm-sink-row">
        <label class="atm-field">{{ tr('标题', 'Title') }}<input v-model="sinkForm.title" :placeholder="tr('例如:批量探测 IDOR 的重放脚本', 'e.g. IDOR batch replay script')" maxlength="200" /></label>
        <label class="atm-field">{{ tr('对话 ID', 'Conversation ID') }}<input v-model="sinkForm.conversationId" :placeholder="tr('脚本产出于哪个对话', 'Conversation the script came from')" /></label>
      </div>
      <label class="atm-field atm-full">{{ tr('工作区脚本路径', 'Workspace script path') }}
        <span class="atm-path-row"><input v-model="sinkForm.path" :placeholder="tr('选择或粘贴路径', 'Pick or paste a path')" /><button type="button" class="button ghost compact" @click="pickPath"><Code2 :size="13" />{{ tr('选择文件', 'Browse') }}</button></span>
      </label>
      <div class="atm-sink-actions">
        <button class="button primary compact" type="submit" :disabled="sinking || !sinkForm.conversationId.trim() || !sinkForm.path.trim()">{{ sinking ? tr('入库中…', 'Sinking…') : tr('入库', 'Sink') }}</button>
        <button class="button ghost compact" type="button" :disabled="sinking" @click="showSink = false">{{ tr('取消', 'Cancel') }}</button>
      </div>
      <p v-if="sinkError" class="atm-error" role="alert">{{ sinkError }}</p>
    </form>

    <div class="atm-filters" role="tablist">
      <button v-for="f in filters" :key="f" type="button" class="atm-tab" :class="{ active: filter === f }" role="tab" :aria-selected="filter === f" @click="setFilter(f)">{{ filterLabel(f) }}</button>
    </div>

    <ul v-if="tools.length" class="atm-list">
      <li v-for="tool in tools" :key="tool.id" class="atm-card">
        <div class="atm-card-head">
          <span class="atm-status" :data-status="tool.status">{{ statusLabel(tool.status) }}</span>
          <strong v-if="tool.title" class="atm-title">{{ tool.title }}</strong>
          <code class="atm-lang">{{ tool.language }}</code>
          <ShieldCheck v-if="tool.verified" :size="13" :title="tr('已在沙箱验证过', 'Verified in sandbox')" class="atm-verified" />
        </div>
        <div v-if="tool.algoTags.length" class="atm-tags"><span v-for="tag in tool.algoTags" :key="tag" class="atm-tag">{{ tag }}</span></div>
        <small v-if="tool.targetFingerprint" class="atm-fingerprint">{{ tr('目标指纹', 'Target fingerprint') }}: {{ tool.targetFingerprint }}</small>
        <small v-if="tool.sourceConversationId" class="atm-source">{{ tr('来源对话', 'Source conversation') }}: {{ tool.sourceConversationId }}</small>

        <details class="atm-code">
          <summary><Code2 :size="13" /> {{ tr('查看脚本正文', 'View script') }}</summary>
          <pre>{{ tool.code }}</pre>
        </details>

        <div class="atm-actions">
          <template v-if="tool.status === 'pending'">
            <button class="button primary compact" type="button" :disabled="busy" @click="decide(tool.id, true)"><ShieldCheck :size="14" /> {{ tr('批准', 'Approve') }}</button>
            <button class="button ghost compact" type="button" :disabled="busy" @click="decide(tool.id, false)"><ShieldBan :size="14" /> {{ tr('拒绝', 'Reject') }}</button>
          </template>
          <button class="atm-icon atm-del" type="button" :disabled="busy" :title="tr('删除', 'Delete')" :aria-label="tr('删除这个脚本', 'Delete this script')" @click="remove(tool.id)"><Trash2 :size="15" /></button>
        </div>
      </li>
    </ul>
    <p v-else-if="!loading && !error" class="atm-muted" role="status">{{ tr('这里还没有入库的脚本。点右上角 + 从某个对话的工作区挑一个脚本。', 'No scripts yet. Use + to sink one from a conversation’s workspace.') }}</p>
    <p v-if="error" class="atm-error" role="alert">{{ error }} <button type="button" @click="load()">{{ tr('重试', 'Retry') }}</button></p>
  </section>
</template>
<style scoped>
.atm-mgr{border:1px solid var(--line,#e1e3e7);border-radius:12px;padding:16px 18px;margin:18px 0 0;min-width:0}
.atm-mgr header{display:flex;justify-content:space-between;align-items:center;gap:12px}
.atm-mgr header strong{display:inline-flex;align-items:center;gap:7px;font-size:var(--text-body)}
.atm-head-actions{display:flex;gap:6px}
.atm-desc{font-size:11px;color:var(--app-muted,#687078);margin:8px 0 12px;line-height:1.6}
.atm-muted{font-size:12px;color:var(--app-muted,#687078);margin:8px 0}
.atm-sink{border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:12px;margin-bottom:12px;display:flex;flex-direction:column;gap:9px}
.atm-sink-row{display:flex;gap:10px;flex-wrap:wrap}
.atm-field{display:flex;flex-direction:column;gap:5px;font-size:11px;color:var(--app-muted,#687078);flex:1;min-width:140px}
.atm-field.atm-full{flex-basis:100%}
.atm-field input{font-size:12px;padding:6px 9px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:var(--app-surface,#fff);color:inherit;width:100%;box-sizing:border-box;font-family:inherit}
.atm-path-row{display:flex;gap:8px;align-items:center}
.atm-path-row input{flex:1}
.atm-sink-actions{display:flex;gap:8px}
.atm-filters{display:flex;gap:6px;margin-bottom:10px;flex-wrap:wrap}
.atm-tab{font-size:12px;padding:5px 11px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;color:var(--app-muted,#687078);cursor:pointer}
.atm-tab.active{color:var(--app-ink,#1c2024);border-color:var(--blue,#416b9c);background:var(--app-surface,#fff)}
.atm-list{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:10px}
.atm-card{border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:12px;min-width:0;display:flex;flex-direction:column;gap:8px}
.atm-card-head{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.atm-status{font-size:var(--text-micro);font-weight:600;border:1px solid var(--line,#e1e3e7);border-radius:6px;padding:1px 7px;color:var(--app-muted,#687078)}
.atm-status[data-status="approved"]{color:var(--green,#3f8f5a);border-color:var(--green,#3f8f5a)}
.atm-status[data-status="rejected"]{color:var(--red,#b63f3f);border-color:var(--red,#b63f3f)}
.atm-title{font-size:12px;overflow-wrap:anywhere}
.atm-lang{font-size:var(--text-micro);color:var(--app-muted,#687078);border:1px solid var(--line,#e1e3e7);border-radius:5px;padding:0 5px}
.atm-verified{color:var(--green,#3f8f5a);flex-shrink:0}
.atm-tags{display:flex;gap:5px;flex-wrap:wrap}
.atm-tag{font-size:var(--text-micro);color:var(--blue,#416b9c);border:1px solid var(--line,#e1e3e7);border-radius:5px;padding:1px 6px}
.atm-fingerprint,.atm-source{font-size:var(--text-micro);color:var(--app-muted,#687078);overflow-wrap:anywhere}
.atm-code{font-size:11px}
.atm-code summary{cursor:pointer;color:var(--app-muted,#687078);display:inline-flex;align-items:center;gap:5px}
.atm-code pre{margin:7px 0 0;padding:9px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:color-mix(in srgb,var(--app-ink,#172033) 3%,transparent);font-size:11px;line-height:1.6;white-space:pre-wrap;overflow-wrap:anywhere;max-height:320px;overflow-y:auto}
.atm-actions{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.atm-icon{display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;padding:6px;color:inherit;flex-shrink:0;cursor:pointer}
.atm-del{margin-left:auto}
.atm-del:hover:not(:disabled){color:var(--red,#b63f3f);border-color:var(--red,#b63f3f)}
.atm-error{color:var(--red,#b63f3f);font-size:12px;overflow-wrap:anywhere}
.spinning{animation:atm-spin .8s linear infinite}
@keyframes atm-spin{to{transform:rotate(360deg)}}
</style>
