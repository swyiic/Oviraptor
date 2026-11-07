<script setup lang="ts">
// Tool library panel (design §3 D3 / §7, contract finalized 2026-11-06). The
// agent writes and verifies a script inside some workspace; once ingested
// into this global table it is recalled, advisory-only, for later targets —
// same wall as memory and technique cards: a suggestion to reuse, never
// auto-run, never feeding any auth/scope/settlement gate. Unlike technique
// cards, this is the agent's own code, not outside material, so it is never
// poison-stripped (stripping would wrongly scrub the target URLs/IPs it is
// built around). Backend commands (list/decide/delete/ingest) are the P2
// contract; this panel degrades to a plain error state (like
// ResearchCardsManager) until they land.
import { Wrench, RefreshCw, Trash2, ShieldCheck, ShieldBan, Plus, Code2, Download } from '@lucide/vue';
import { invoke } from '@tauri-apps/api/core';
import { onMounted, ref } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { confirmInPlace } from '../../../shared/confirmInPlace';
import { useI18n } from '../../../i18n';
import { resolveAgentHostEndpoint } from '../api';
import { createAgentHostChatClient, type AgentHostChatClient, type AgentConversationFile } from '../chatClient';

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

const showForm = ref(false);
const form = ref({ title: '', language: 'node' as 'node' | 'python', code: '', targetFingerprint: '', algoTags: '', sourceConversationId: '' });
const submitting = ref(false);
const submitError = ref('');

// "从工作区入库" convenience: pull a script's content from a conversation's host
// workspace via the agent-host HTTP client (resolved lazily — this panel is
// otherwise pure Tauri invoke), then it is still ingested as plain code.
let hostClient: AgentHostChatClient | undefined;
const fetchProjectId = ref('');
const fetchPath = ref('');
const fetchBusy = ref(false);
const fetchError = ref('');
const workspaceFiles = ref<AgentConversationFile[]>([]);
const filesBusy = ref(false);
const filesError = ref('');

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

async function ensureHostClient(): Promise<AgentHostChatClient> {
  if (hostClient) return hostClient;
  const endpoint = await resolveAgentHostEndpoint();
  if (!endpoint) throw Error(tr('agent-host 未运行', 'agent-host is not running'));
  hostClient = createAgentHostChatClient(endpoint);
  return hostClient;
}

async function listWorkspaceFiles() {
  const projectId = Number(fetchProjectId.value);
  const conversationId = form.value.sourceConversationId.trim();
  if (!conversationId || !Number.isFinite(projectId) || projectId <= 0) return;
  filesBusy.value = true;
  filesError.value = '';
  try {
    const client = await ensureHostClient();
    const result = await client.conversationFiles(conversationId, projectId);
    workspaceFiles.value = result.items;
  } catch (e) {
    filesError.value = String((e as Error).message || e);
  } finally {
    filesBusy.value = false;
  }
}

async function fetchFileContent(path?: string) {
  const projectId = Number(fetchProjectId.value);
  const conversationId = form.value.sourceConversationId.trim();
  const targetPath = (path ?? fetchPath.value).trim();
  if (!conversationId || !targetPath || !Number.isFinite(projectId) || projectId <= 0) return;
  fetchBusy.value = true;
  fetchError.value = '';
  try {
    const client = await ensureHostClient();
    const preview = await client.previewConversationFile(conversationId, projectId, targetPath);
    form.value.code = preview.content;
    if (!form.value.title.trim()) form.value.title = preview.name;
    form.value.language = preview.language === 'python' ? 'python' : 'node';
    fetchPath.value = targetPath;
  } catch (e) {
    fetchError.value = String((e as Error).message || e);
  } finally {
    fetchBusy.value = false;
  }
}

async function submit() {
  if (submitting.value || !form.value.title.trim() || !form.value.code.trim()) return;
  submitting.value = true;
  submitError.value = '';
  try {
    // Rust's ingest_agent_tool takes algo_tags as a JSON-encoded string (it stores
    // and re-parses it), not a native array — the IPC layer rejects an array here.
    const tags = form.value.algoTags.split(',').map(tag => tag.trim()).filter(Boolean);
    await invoke('ingest_agent_tool', {
      title: form.value.title.trim(),
      language: form.value.language,
      code: form.value.code,
      targetFingerprint: form.value.targetFingerprint.trim() || null,
      algoTags: JSON.stringify(tags),
      sourceConversationId: form.value.sourceConversationId.trim() || null,
    });
    form.value = { title: '', language: 'node', code: '', targetFingerprint: '', algoTags: '', sourceConversationId: '' };
    fetchProjectId.value = ''; fetchPath.value = ''; workspaceFiles.value = [];
    showForm.value = false;
    filter.value = 'pending';
    await load();
  } catch (e) {
    submitError.value = String((e as Error).message || e);
  } finally {
    submitting.value = false;
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
  await confirmInPlace({
    title: tr('删除这个脚本？', 'Delete this script?'),
    detail: tr('此操作不可恢复。', 'This cannot be undone.'),
    confirmText: tr('删除', 'Delete'), busyText: tr('删除中…', 'Deleting…'), cancelText: tr('取消', 'Cancel'),
    run: async () => {
      if (busy.value) return;
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
    },
  });
}

onMounted(load);
</script>
<template>
  <section class="atm-mgr" data-testid="agent-tools-manager">
    <header>
      <strong><Wrench :size="15" /> {{ tr('工具库', 'Tool library') }}</strong>
      <div class="atm-head-actions">
        <button class="atm-icon" type="button" :disabled="busy" :title="tr('新增脚本', 'Add a script')" :aria-label="tr('新增脚本', 'Add a script')" @click="showForm = !showForm"><Plus :size="14" /></button>
        <button v-refresh-feedback class="atm-icon" type="button" :disabled="loading" :title="tr('刷新', 'Refresh')" :aria-label="tr('刷新工具库', 'Refresh tool library')" @click="load()"><RefreshCw :size="14" :class="{ spinning: loading }" /></button>
      </div>
    </header>
    <p class="atm-desc">{{ tr('agent 自己写、在某个工作空间验证过的脚本,沉淀在这里跨工作空间复用。批准后它才会作为"可复用这个脚本"的建议注入新对话——是参考,不是证据;复用前 agent 仍须重读一遍、在新目标沙箱里重新验证。这里存的是 agent 自写代码,不是外来材料,不会被剥毒处理。', 'Scripts the agent wrote and verified in some workspace, kept here for reuse across workspaces. Once approved it is suggested — "you could reuse this script" — to new conversations: reference, not evidence; the agent must still re-read and re-verify it against the new target before use. This is the agent’s own code, not outside material, so it is never poison-stripped.') }}</p>

    <form v-if="showForm" class="atm-sink" @submit.prevent="submit">
      <div class="atm-sink-row">
        <label class="atm-field">{{ tr('标题', 'Title') }}<input v-model="form.title" :placeholder="tr('例如:批量探测 IDOR 的重放脚本', 'e.g. IDOR batch replay script')" maxlength="200" /></label>
        <label class="atm-field">{{ tr('语言', 'Language') }}<select v-model="form.language"><option value="node">node</option><option value="python">python</option></select></label>
      </div>
      <div class="atm-sink-row">
        <label class="atm-field">{{ tr('目标指纹（可选）', 'Target fingerprint (optional)') }}<input v-model="form.targetFingerprint" :placeholder="tr('例如:nginx+php-fpm / jwt-none-alg', 'e.g. nginx+php-fpm / jwt-none-alg')" /></label>
        <label class="atm-field">{{ tr('算法标签（逗号分隔，可选）', 'Algo tags (comma separated, optional)') }}<input v-model="form.algoTags" :placeholder="tr('idor, batch-replay', 'idor, batch-replay')" /></label>
      </div>
      <label class="atm-field atm-full">{{ tr('脚本正文', 'Script') }}
        <textarea v-model="form.code" rows="8" :placeholder="tr('粘贴脚本,或用下面的「从工作区读取」拉取', 'Paste the script, or pull it with “Read from workspace” below')"></textarea>
      </label>

      <details class="atm-fetch">
        <summary><Download :size="13" /> {{ tr('从工作区读取（可选）', 'Read from workspace (optional)') }}</summary>
        <div class="atm-sink-row">
          <label class="atm-field">{{ tr('来源对话 ID', 'Source conversation ID') }}<input v-model="form.sourceConversationId" :placeholder="tr('脚本产出于哪个对话', 'Conversation the script came from')" /></label>
          <label class="atm-field">{{ tr('宿主项目 ID', 'Host project ID') }}<input v-model="fetchProjectId" type="number" min="1" :placeholder="tr('该对话所在的 agent-host 项目', 'The agent-host project that conversation lives in')" /></label>
        </div>
        <div class="atm-sink-row">
          <button type="button" class="button ghost compact" :disabled="filesBusy" @click="listWorkspaceFiles"><Code2 :size="13" />{{ filesBusy ? tr('列出中…', 'Listing…') : tr('列出工作区文件', 'List workspace files') }}</button>
          <span class="atm-path-row"><input v-model="fetchPath" :placeholder="tr('或直接填路径', 'or paste a path directly')" /><button type="button" class="button ghost compact" :disabled="fetchBusy" @click="fetchFileContent()">{{ fetchBusy ? tr('读取中…', 'Reading…') : tr('读取内容', 'Read content') }}</button></span>
        </div>
        <ul v-if="workspaceFiles.length" class="atm-file-list">
          <li v-for="file in workspaceFiles" :key="file.path"><button type="button" class="text-button" @click="fetchFileContent(file.path)">{{ file.path }}</button></li>
        </ul>
        <p v-if="filesError" class="atm-error" role="alert">{{ filesError }}</p>
        <p v-if="fetchError" class="atm-error" role="alert">{{ fetchError }}</p>
      </details>

      <div class="atm-sink-actions">
        <button class="button primary compact" type="submit" :disabled="submitting || !form.title.trim() || !form.code.trim()">{{ submitting ? tr('入库中…', 'Saving…') : tr('入库', 'Save') }}</button>
        <button class="button ghost compact" type="button" :disabled="submitting" @click="showForm = false">{{ tr('取消', 'Cancel') }}</button>
      </div>
      <p v-if="submitError" class="atm-error" role="alert">{{ submitError }}</p>
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
    <p v-else-if="!loading && !error" class="atm-muted" role="status">{{ tr('这里还没有入库的脚本。点右上角 + 粘贴一段验证过的脚本,或从某个对话的工作区读取。', 'No scripts yet. Use + to paste a verified script, or read one from a conversation’s workspace.') }}</p>
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
.atm-field input,.atm-field select,.atm-field textarea{font-size:12px;padding:6px 9px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:var(--app-surface,#fff);color:inherit;width:100%;box-sizing:border-box;font-family:inherit}
.atm-field textarea{resize:vertical;line-height:1.5;font-family:ui-monospace,monospace}
.atm-path-row{display:flex;gap:8px;align-items:center;flex:1;min-width:220px}
.atm-path-row input{flex:1}
.atm-fetch{font-size:11px;border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:9px}
.atm-fetch summary{cursor:pointer;color:var(--app-muted,#687078);display:inline-flex;align-items:center;gap:5px;margin-bottom:8px}
.atm-fetch[open] summary{margin-bottom:9px}
.atm-file-list{list-style:none;margin:8px 0 0;padding:0;display:flex;flex-direction:column;gap:3px;max-height:140px;overflow-y:auto}
.atm-file-list .text-button{font-size:11px;text-align:left;overflow-wrap:anywhere}
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
