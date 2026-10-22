<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import type { AgentHostChatClient } from '../chatClient';
import type { DiscoveryFamily, DiscoveryView } from '../discoveryTypes';

const props = defineProps<{ client: AgentHostChatClient; conversationId: string; projectId: number; refreshKey?: number }>();
const data = ref<DiscoveryView>(), error = ref(''), notice = ref(''), busy = ref(false), reading = ref(false);
const selected = ref<string[]>([]), retryRead = ref(false), errorDismissed = ref(false);
let generation = 0, readSequence = 0, disposed = false, failures = 0;
let timer: ReturnType<typeof setTimeout> | undefined, flight: Promise<void> | undefined;
const uncheckedFamilies = new Set<string>();
const proposal = computed(() => data.value?.proposal);
function hasBoundEvidence(family: DiscoveryFamily) { return family.verification?.status === 'bound_local_evidence' && family.verification.sources?.some(s => !!s.kind?.trim() && !!s.host?.trim()); }
const families = computed(() => (proposal.value?.groups ?? []).filter(hasBoundEvidence));
const heldFamilies = computed(() => Math.max(proposal.value?.heldForVerification ?? 0, (proposal.value?.groups.length ?? 0) - families.value.length));
const hosts = computed(() => [...new Set(families.value.filter(g => selected.value.includes(g.root)).flatMap(g => g.scopes))]);
function sourceLabel(kind: string) { return ({ tls_subject: 'TLS 证书 Subject', icp: '备案单位', confirmed_directory: '已确认单位目录' } as Record<string, string>)[kind] ?? kind; }
const run = computed(() => data.value?.review?.run);
const counts = computed(() => data.value?.enrichment?.run?.counts ?? {});
const visible = computed(() => !!data.value?.runId);
const freeWorking = computed(() => ['running', 'waiting', 'pausing'].includes(data.value?.enrichment?.run?.status ?? ''));
const phaseLabel = computed(() => ({ preparing: '整理全项目证据', analyzing: 'AI 提炼中', awaiting_scope: '等待一次范围确认', ready: freeWorking.value ? '已确认范围自动补证' : 'AI 提炼已完成', paused: '已暂停', blocked: '需要处理错误', pausing: '正在保存进度' } as Record<string, string>)[data.value?.phase ?? ''] ?? 'AI 发现资产');
const compactNotice = computed(() => {
  if (visible.value || errorDismissed.value) return '';
  if (error.value) return retryRead.value ? 'AI 发现状态暂时读取失败，正在自动重试。' : 'AI 发现状态暂不可用，请检查资产读取权限或更新内置组件。';
  return data.value?.phase === 'empty' ? '这个项目还没有本地资产，请先到资产页采集或导入，再开始 AI 发现。' : '';
});
function pageVisible() { return typeof document === 'undefined' || !document.hidden; }
function clearTimer() { window.clearTimeout(timer); timer = undefined; }
function rememberChoices() {
  for (const family of families.value) {
    if (selected.value.includes(family.root)) uncheckedFamilies.delete(family.root);
    else uncheckedFamilies.add(family.root);
  }
}
function schedule(g: number) {
  clearTimer();
  if (disposed || g !== generation || !pageVisible()) return;
  if (retryRead.value || visible.value && (data.value?.enabled || freeWorking.value)) {
    const delay = retryRead.value ? Math.min(60000, 5000 * 2 ** Math.min(failures - 1, 4)) : 5000;
    timer = window.setTimeout(() => { timer = undefined; void load(); }, delay);
  }
}
function permanentReadFailure(message: string) {
  // A missing executor and a transient native reconnect share the old Host
  // error code. Retry that ambiguity finitely; unsupported routes never poll.
  if (message.includes('host_discovery_executor_unavailable')) return failures >= 3;
  return /host_(?:tool_permission_denied|tool_authority_changed|discovery_task_rejected|discovery_project_unmapped|discovery_project_binding_changed|discovery_result_too_large|app_unavailable|application_unavailable|route_unavailable|project_unavailable|conversation_unavailable)|agent_host_http_(?:401|403|404)/.test(message);
}
async function load(force = false) {
  if (disposed || !props.client.discoveryStatus) return;
  if (flight && force !== true) return flight;
  clearTimer();
  const g = generation, seq = ++readSequence, conversation = props.conversationId, project = props.projectId;
  reading.value = true;
  const current = () => !disposed && g === generation && seq === readSequence;
  const work = (async () => {
    try {
      const next = await props.client.discoveryStatus(conversation, project);
      if (!current()) return;
      if (next.proposal?.token !== data.value?.proposal?.token) {
        rememberChoices();
        selected.value = (next.proposal?.groups ?? []).filter(f => hasBoundEvidence(f) && !uncheckedFamilies.has(f.root)).map(f => f.root);
      }
      data.value = next; error.value = ''; retryRead.value = false; failures = 0; errorDismissed.value = false;
    } catch (e) {
      if (!current()) return;
      error.value = String((e as Error).message || e);
      failures++; retryRead.value = !permanentReadFailure(error.value);
      if (!retryRead.value) { rememberChoices(); data.value = undefined; }
    } finally { if (current()) { reading.value = false; schedule(g); } }
  })();
  flight = work;
  try { await work; } finally { if (flight === work) flight = undefined; }
}
async function control(action: 'start' | 'pause') {
  if (busy.value || !data.value) return;
  const g = generation, d = data.value; busy.value = true; error.value = '';
  try { await props.client.discoveryControl(props.conversationId, props.projectId, { ...d.confirmationWitness, action, ...(action === 'start' ? { organization: d.organization, includeIsolated: d.includeIsolated } : {}) }); if (g === generation && !disposed) await load(true); }
  catch (e) { if (g === generation && !disposed) { const message = String((e as Error).message || e); await load(true); if (g === generation && !disposed) error.value = message; } }
  finally { if (g === generation && !disposed) busy.value = false; }
}
async function confirm() {
  const p = proposal.value, d = data.value;
  if (busy.value || !p || !d?.runId || !hosts.value.length) return;
  const g = generation; busy.value = true; error.value = ''; notice.value = '';
  try {
    const result = await props.client.confirmDiscoveryScope(props.conversationId, props.projectId, { ...d.confirmationWitness, runId: d.runId, proposalToken: p.token, expectedRoots: p.roots, hosts: [...hosts.value], userConfirmed: true });
    if (g !== generation || disposed) return;
    notice.value = result.enrichmentStarted ? '范围已保存，后台自动补证已接续。' : `范围已保存，补证启动待恢复：${result.enrichmentError || '请查看任务状态'}；无需重复确认范围。`;
    // A read started before this write cannot establish the resulting state.
    await load(true);
  } catch (e) { if (g === generation && !disposed) { const message = String((e as Error).message || e); await load(true); if (g === generation && !disposed) error.value = message; } }
  finally { if (g === generation && !disposed) busy.value = false; }
}
function visibilityChanged() {
  if (!pageVisible()) clearTimer();
  else void load(true);
}
if (typeof document !== 'undefined') document.addEventListener('visibilitychange', visibilityChanged);
watch(() => [props.projectId, props.conversationId], () => { generation++; readSequence++; clearTimer(); flight = undefined; data.value = undefined; selected.value = []; uncheckedFamilies.clear(); error.value = ''; notice.value = ''; retryRead.value = false; failures = 0; errorDismissed.value = false; busy.value = false; reading.value = false; void load(); }, { immediate: true });
watch(() => props.refreshKey, () => { if (pageVisible()) void load(true); });
onBeforeUnmount(() => { disposed = true; generation++; readSequence++; clearTimer(); if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', visibilityChanged); });
</script>

<template>
  <div v-if="compactNotice" class="chat-discovery-note" role="status" data-testid="chat-discovery-notice"><span>{{ compactNotice }}<small v-if="error">{{ error }}</small></span><button class="button ghost compact" type="button" data-testid="chat-discovery-retry" :disabled="reading" @click="load(true)">重试读取</button><button class="button ghost compact" type="button" @click="errorDismissed = true">关闭</button></div>
  <section v-else-if="visible" class="chat-discovery" data-testid="chat-discovery">
    <header><div><strong>AI 发现资产</strong><span>{{ phaseLabel }}</span></div><div class="discovery-actions"><button v-refresh-feedback="{ scope: projectId + ':' + conversationId, busy: reading, error }" class="button ghost compact" type="button" :disabled="busy" @click="load(true)">刷新</button><button v-if="data?.enabled" class="button ghost compact" type="button" :disabled="busy" @click="control('pause')">暂停</button><button v-else class="button secondary compact" type="button" :disabled="busy" @click="control('start')">继续 AI 发现</button></div></header>
    <p>{{ data?.organization }} · {{ data?.reason }}</p>
    <div class="discovery-metrics"><span v-if="run">已整理 {{ run.processedAssets.toLocaleString() }} / {{ run.totalAssets.toLocaleString() }} 条</span><span v-if="run">模型 {{ run.requests }} 次 · {{ run.estimatedUsage ? '约 ' : '' }}{{ (run.inputTokens + run.outputTokens).toLocaleString() }} Token{{ run.estimatedUsage ? '（估算）' : '' }}</span><span>Hunter 0 积分</span><span v-if="data?.enrichment?.run">补证完成 {{ counts.done || 0 }} · 等待重试 {{ counts.retry || 0 }} · 失败 {{ counts.failed || 0 }}</span></div>
    <div v-if="families.length" class="discovery-consent">
      <strong>已核验的范围建议 · {{ families.length }} 组，集中确认一次</strong>
      <p>已核对本地证据与目标单位的绑定；确认后自动补证，真正冲突另行汇总。</p>
      <p v-if="proposal?.removedNoLiveEvidence">已省略 {{ proposal.removedNoLiveEvidence }} 项只有历史或已失效证据的范围，原记录保留。</p>
      <div class="discovery-families"><div v-for="family in families" :key="family.root" class="discovery-family"><label><input v-model="selected" type="checkbox" :value="family.root" :disabled="busy"><span><strong>{{ family.root }}</strong><small>{{ family.shared ? `共享平台，仅 ${family.scopes.length} 个具体主机` : `仅确认下列 ${family.scopes.length} 项范围` }} · {{ family.evidenceGroups }} 组证据</small></span></label><div class="discovery-verification"><small v-for="(source, index) in family.verification?.sources" :key="index">{{ sourceLabel(source.kind) }} · {{ source.host }} · {{ source.organization || '已确认的域名绑定' }}<template v-if="source.assetId"> · 资产 #{{ source.assetId }}</template></small></div><details><summary>查看本组将确认的 {{ family.scopes.length }} 项范围</summary><ul><li v-for="scope in family.scopes" :key="scope">{{ scope }}</li></ul><p v-if="family.reasons?.length">AI 推断（不作核验依据）：{{ family.reasons.join('；') }}</p></details></div></div>
      <button class="button primary compact" type="button" data-testid="chat-discovery-confirm" :disabled="busy || !hosts.length" @click="confirm">{{ busy ? '正在保存…' : `确认所选 ${selected.length} 组范围并自动补证` }}</button>
    </div>
    <p v-if="proposal">本次仅核验本地证据，未联网查证企业归属。</p>
    <p v-if="heldFamilies" class="discovery-held" role="status">{{ heldFamilies }} 个域名家族缺少逐域绑定依据，已暂存等待核验，不加入探测范围，无需逐项确认。<span v-if="proposal?.removedUnverified">本次拦截 {{ proposal.removedUnverified }} 项未核验范围。</span></p>
    <p v-if="notice" role="status">{{ notice }}</p><p v-if="error" class="discovery-error" role="alert">{{ error }}</p><p v-if="data?.enrichmentError" class="discovery-error">{{ data.enrichmentError }}</p>
  </section>
</template>

<style scoped>
.discovery-verification{margin:0 0 8px 23px;overflow-wrap:anywhere}.discovery-held{font-size:11px;border-top:1px solid var(--line);padding-top:9px}
.chat-discovery-note{display:flex;align-items:center;gap:8px;flex-wrap:wrap;border:1px solid var(--line);border-radius:8px;padding:8px 10px;font-size:11px;color:var(--app-muted,#72746f)}.chat-discovery-note>span{flex:1;min-width:160px}.chat-discovery-note small{display:block;overflow-wrap:anywhere}.chat-discovery{border:1px solid var(--line);border-radius:12px;padding:14px;font-size:12px;overflow-wrap:anywhere;background:var(--app-surface,#fff)}.chat-discovery header{display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap}.chat-discovery header>div:first-child{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.chat-discovery header span,.chat-discovery p,.discovery-metrics,.discovery-families small{color:var(--app-muted,#72746f)}.chat-discovery p{line-height:1.6;margin:9px 0}.discovery-actions,.discovery-metrics{display:flex;gap:8px;flex-wrap:wrap}.discovery-metrics{gap:7px 16px;font-size:11px}.discovery-consent{border-top:1px solid var(--line);padding-top:12px;margin-top:12px}.discovery-families{max-height:280px;overflow:auto;margin:8px 0 12px}.discovery-families label{display:flex;gap:9px;padding:9px 0;border-bottom:1px solid var(--line);align-items:flex-start}.discovery-family details{margin:0 0 8px 23px;font-size:11px}.discovery-family summary{cursor:pointer;color:var(--app-muted,#72746f)}.discovery-family ul{max-height:160px;overflow:auto;padding-left:18px;line-height:1.7}.discovery-families input{margin:3px 0 0;flex-shrink:0}.discovery-families small{display:block;margin-top:3px;line-height:1.5}.discovery-error{color:#ac3939!important}@media(max-width:600px){.chat-discovery{padding:11px}.discovery-consent>.button{width:100%}.discovery-actions{margin-left:auto}}
</style>
