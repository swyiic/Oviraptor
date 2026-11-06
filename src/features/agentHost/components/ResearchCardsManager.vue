<script setup lang="ts">
// D4 operator review UI (design §4d). The operator pastes an external write-up /
// method card; it is poison-stripped at ingest (Rust `ingest_research_card`) and
// stored `pending`. Here the operator reviews what methodology survived and what
// was stripped out, then approves or rejects. Only APPROVED cards are injected
// into the agent's reasoning prompt — advisory only, the same wall as memory:
// reference, never evidence, never authority; never feeds any scope/auth gate.
import { BookOpen, RefreshCw, Trash2, ShieldCheck, ShieldBan, ShieldAlert, Plus } from '@lucide/vue';
import { invoke } from '@tauri-apps/api/core';
import { onMounted, ref } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { useI18n } from '../../../i18n';

interface StrippedArtifact { kind: string; preview: string }
interface ResearchCard {
  id: string;
  title: string;
  source: string;
  methodology: string;
  stripped: StrippedArtifact[];
  status: 'pending' | 'approved' | 'rejected';
  createdAt: string;
}
type Filter = 'pending' | 'approved' | 'rejected' | 'all';

const { tr } = useI18n();
const cards = ref<ResearchCard[]>([]);
const filter = ref<Filter>('pending');
const loading = ref(false);
const busy = ref(false);
const error = ref('');

const showIngest = ref(false);
const form = ref({ title: '', source: '', markdown: '' });
const ingesting = ref(false);
const ingestError = ref('');

const filters: Filter[] = ['pending', 'approved', 'rejected', 'all'];
function filterLabel(value: Filter) {
  return value === 'approved' ? tr('已批准', 'Approved')
    : value === 'rejected' ? tr('已拒绝', 'Rejected')
      : value === 'all' ? tr('全部', 'All')
        : tr('待审', 'Pending');
}

async function load() {
  loading.value = true;
  error.value = '';
  try {
    cards.value = await invoke<ResearchCard[]>('list_research_cards', {
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

async function ingest() {
  if (ingesting.value || !form.value.markdown.trim()) return;
  ingesting.value = true;
  ingestError.value = '';
  try {
    await invoke<ResearchCard>('ingest_research_card', {
      title: form.value.title.trim(),
      source: form.value.source.trim(),
      markdown: form.value.markdown,
    });
    form.value = { title: '', source: '', markdown: '' };
    showIngest.value = false;
    filter.value = 'pending';
    await load();
  } catch (e) {
    ingestError.value = String((e as Error).message || e);
  } finally {
    ingesting.value = false;
  }
}

async function decide(id: string, approve: boolean) {
  if (busy.value) return;
  busy.value = true;
  error.value = '';
  try {
    await invoke('decide_research_card', { id, approve });
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
    && !window.confirm(tr('删除这张技法卡?此操作不可恢复。', 'Delete this card? This cannot be undone.'))) return;
  busy.value = true;
  error.value = '';
  try {
    await invoke('delete_research_card', { id });
    cards.value = cards.value.filter(card => card.id !== id);
  } catch (e) {
    error.value = String((e as Error).message || e);
  } finally {
    busy.value = false;
  }
}

function statusLabel(status: string) {
  return status === 'approved' ? tr('已批准', 'Approved')
    : status === 'rejected' ? tr('已拒绝', 'Rejected')
      : tr('待审', 'Pending');
}

onMounted(load);
</script>
<template>
  <section class="rc-mgr" data-testid="research-cards-manager">
    <header>
      <strong><BookOpen :size="15" /> {{ tr('技法卡审阅', 'Technique cards') }}</strong>
      <div class="rc-head-actions">
        <button class="rc-icon" type="button" :disabled="busy" :title="tr('摄入新材料', 'Ingest material')" :aria-label="tr('摄入新材料', 'Ingest material')" @click="showIngest = !showIngest"><Plus :size="14" /></button>
        <button v-refresh-feedback class="rc-icon" type="button" :disabled="loading" :title="tr('刷新', 'Refresh')" :aria-label="tr('刷新技法卡', 'Refresh cards')" @click="load()"><RefreshCw :size="14" :class="{ spinning: loading }" /></button>
      </div>
    </header>
    <p class="rc-desc">{{ tr('把外部复现文章 / 方法论粘进来——摄入时自动剥离回连地址、密钥、取-执行等制品,只留"怎么想"的思路。批准后它才会作为参考素材注入 agent 推理:是参考,不是证据,也不授予任何权限,每个目标仍需单独确认。', 'Paste an external write-up / methodology. Callback addresses, keys and fetch-exec artifacts are stripped at ingest — only the reasoning survives. Only after you approve it does a card inform the agent, as reference: not evidence, granting no authority; every target still needs its own authorization.') }}</p>

    <form v-if="showIngest" class="rc-ingest" @submit.prevent="ingest">
      <div class="rc-ingest-row">
        <label class="rc-field">{{ tr('标题', 'Title') }}<input v-model="form.title" :placeholder="tr('例如:越权从单点到批量', 'e.g. IDOR: single to bulk')" maxlength="200" /></label>
        <label class="rc-field">{{ tr('来源', 'Source') }}<input v-model="form.source" :placeholder="tr('文章/出处(可空)', 'article / origin (optional)')" maxlength="300" /></label>
      </div>
      <label class="rc-field rc-full">{{ tr('原文(不可信——会在摄入时剥毒)', 'Material (untrusted — stripped at ingest)') }}
        <textarea v-model="form.markdown" rows="6" :placeholder="tr('粘贴 Markdown / 文字…', 'Paste markdown / text…')"></textarea>
      </label>
      <div class="rc-ingest-actions">
        <button class="button primary compact" type="submit" :disabled="ingesting || !form.markdown.trim()">{{ ingesting ? tr('摄入中…', 'Ingesting…') : tr('摄入并剥毒', 'Ingest & strip') }}</button>
        <button class="button ghost compact" type="button" :disabled="ingesting" @click="showIngest = false">{{ tr('取消', 'Cancel') }}</button>
      </div>
      <p v-if="ingestError" class="rc-error" role="alert">{{ ingestError }}</p>
    </form>

    <div class="rc-filters" role="tablist">
      <button v-for="f in filters" :key="f" type="button" class="rc-tab" :class="{ active: filter === f }" role="tab" :aria-selected="filter === f" @click="setFilter(f)">{{ filterLabel(f) }}</button>
    </div>

    <ul v-if="cards.length" class="rc-list">
      <li v-for="card in cards" :key="card.id" class="rc-card">
        <div class="rc-card-head">
          <span class="rc-status" :data-status="card.status">{{ statusLabel(card.status) }}</span>
          <strong v-if="card.title" class="rc-title">{{ card.title }}</strong>
          <small v-if="card.source" class="rc-source">{{ card.source }}</small>
        </div>

        <div class="rc-section-label">{{ tr('注入 agent 的方法论(剥毒后)', 'Methodology injected into the agent (after stripping)') }}</div>
        <p class="rc-method">{{ card.methodology || tr('(剥毒后无可用方法论)', '(nothing usable left after stripping)') }}</p>

        <details v-if="card.stripped.length" class="rc-stripped">
          <summary><ShieldAlert :size="13" /> {{ tr('已剥离', 'Stripped') }} ({{ card.stripped.length }}) — {{ tr('不进库、不注入', 'never stored or injected') }}</summary>
          <ul class="rc-stripped-list">
            <li v-for="(item, index) in card.stripped" :key="index"><code class="rc-kind">{{ item.kind }}</code> <span class="rc-preview">{{ item.preview }}</span></li>
          </ul>
        </details>

        <div class="rc-actions">
          <template v-if="card.status === 'pending'">
            <button class="button primary compact" type="button" :disabled="busy" @click="decide(card.id, true)"><ShieldCheck :size="14" /> {{ tr('批准', 'Approve') }}</button>
            <button class="button ghost compact" type="button" :disabled="busy" @click="decide(card.id, false)"><ShieldBan :size="14" /> {{ tr('拒绝', 'Reject') }}</button>
          </template>
          <button class="rc-icon rc-del" type="button" :disabled="busy" :title="tr('删除', 'Delete')" :aria-label="tr('删除这张技法卡', 'Delete this card')" @click="remove(card.id)"><Trash2 :size="15" /></button>
        </div>
      </li>
    </ul>
    <p v-else-if="!loading && !error" class="rc-muted" role="status">{{ tr('这里还没有技法卡。点右上角 + 粘贴一篇材料。', 'No cards yet. Use + to paste some material.') }}</p>
    <p v-if="error" class="rc-error" role="alert">{{ error }} <button type="button" @click="load()">{{ tr('重试', 'Retry') }}</button></p>
  </section>
</template>
<style scoped>
.rc-mgr{border:1px solid var(--line,#e1e3e7);border-radius:12px;padding:16px 18px;margin:18px 0 0;min-width:0}
.rc-mgr header{display:flex;justify-content:space-between;align-items:center;gap:12px}
.rc-mgr header strong{display:inline-flex;align-items:center;gap:7px;font-size:var(--text-body)}
.rc-head-actions{display:flex;gap:6px}
.rc-desc{font-size:11px;color:var(--app-muted,#687078);margin:8px 0 12px;line-height:1.6}
.rc-muted{font-size:12px;color:var(--app-muted,#687078);margin:8px 0}
.rc-ingest{border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:12px;margin-bottom:12px;display:flex;flex-direction:column;gap:9px}
.rc-ingest-row{display:flex;gap:10px;flex-wrap:wrap}
.rc-field{display:flex;flex-direction:column;gap:5px;font-size:11px;color:var(--app-muted,#687078);flex:1;min-width:140px}
.rc-field.rc-full{flex-basis:100%}
.rc-field input,.rc-field textarea{font-size:12px;padding:6px 9px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:var(--app-surface,#fff);color:inherit;width:100%;box-sizing:border-box;font-family:inherit}
.rc-field textarea{resize:vertical;line-height:1.5}
.rc-ingest-actions{display:flex;gap:8px}
.rc-filters{display:flex;gap:6px;margin-bottom:10px;flex-wrap:wrap}
.rc-tab{font-size:12px;padding:5px 11px;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;color:var(--app-muted,#687078);cursor:pointer}
.rc-tab.active{color:var(--app-ink,#1c2024);border-color:var(--blue,#416b9c);background:var(--app-surface,#fff)}
.rc-list{list-style:none;margin:0;padding:0;display:flex;flex-direction:column;gap:10px}
.rc-card{border:1px solid var(--line,#e1e3e7);border-radius:9px;padding:12px;min-width:0;display:flex;flex-direction:column;gap:8px}
.rc-card-head{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.rc-status{font-size:var(--text-micro);font-weight:600;border:1px solid var(--line,#e1e3e7);border-radius:6px;padding:1px 7px;color:var(--app-muted,#687078)}
.rc-status[data-status="approved"]{color:var(--green,#3f8f5a);border-color:var(--green,#3f8f5a)}
.rc-status[data-status="rejected"]{color:var(--red,#b63f3f);border-color:var(--red,#b63f3f)}
.rc-title{font-size:12px;overflow-wrap:anywhere}
.rc-source{font-size:var(--text-micro);color:var(--app-muted,#687078);overflow-wrap:anywhere}
.rc-section-label{font-size:var(--text-micro);color:var(--app-muted,#687078);font-weight:600}
.rc-method{margin:0;font-size:12px;line-height:1.6;white-space:pre-wrap;overflow-wrap:anywhere}
.rc-stripped{font-size:11px}
.rc-stripped summary{cursor:pointer;color:var(--amber,#9a6b2f);display:inline-flex;align-items:center;gap:5px}
.rc-stripped-list{list-style:none;margin:7px 0 0;padding:0;display:flex;flex-direction:column;gap:4px}
.rc-stripped-list li{display:flex;gap:7px;align-items:baseline;overflow-wrap:anywhere}
.rc-kind{font-size:var(--text-micro);color:var(--red,#b63f3f);border:1px solid var(--line,#e1e3e7);border-radius:5px;padding:0 5px;flex-shrink:0}
.rc-preview{font-size:11px;color:var(--app-muted,#687078);overflow-wrap:anywhere;word-break:break-all}
.rc-actions{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.rc-icon{display:inline-flex;align-items:center;justify-content:center;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;padding:6px;color:inherit;flex-shrink:0;cursor:pointer}
.rc-del{margin-left:auto}
.rc-del:hover:not(:disabled){color:var(--red,#b63f3f);border-color:var(--red,#b63f3f)}
.rc-error{color:var(--red,#b63f3f);font-size:12px;overflow-wrap:anywhere}
.spinning{animation:rc-spin .8s linear infinite}
@keyframes rc-spin{to{transform:rotate(360deg)}}
</style>
