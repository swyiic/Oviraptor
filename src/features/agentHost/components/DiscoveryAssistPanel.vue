<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import type { AgentHostChatClient } from '../chatClient';
import type { DiscoveryFamily, DiscoveryView, DiscoveryWitness } from '../discoveryTypes';
import DiscoveryPipelineProgress from '../../assets/components/DiscoveryPipelineProgress.vue';

const props = defineProps<{ client: AgentHostChatClient; conversationId: string; projectId: number; refreshKey?: number }>();
const data = ref<DiscoveryView>(), error = ref(''), notice = ref(''), busy = ref(false), reading = ref(false);
const selected = ref<string[]>([]), retryRead = ref(false), errorDismissed = ref(false), statusFresh = ref(false), expanded = ref(false), adjustScopes = ref(false);
let generation = 0, readSequence = 0, disposed = false, failures = 0;
let timer: ReturnType<typeof setTimeout> | undefined, flight: Promise<void> | undefined;
const uncheckedFamilies = new Set<string>();
const proposal = computed(() => data.value?.proposal);
function hasBoundEvidence(family: DiscoveryFamily) { return family.verification?.status === 'bound_local_evidence' && family.verification.sources?.some(s => !!s.kind?.trim() && !!s.host?.trim()); }
const families = computed(() => (proposal.value?.groups ?? []).filter(hasBoundEvidence));
const heldFamilies = computed(() => Math.max(proposal.value?.heldForVerification ?? 0, (proposal.value?.groups.length ?? 0) - families.value.length));
const hosts = computed(() => [...new Set(families.value.filter(g => selected.value.includes(g.root)).flatMap(g => g.scopes))]);
type ScopeConfirmation = { projectId: number; conversationId: string; generation: number; runId: string; token: string; roots: string[]; hosts: string[]; selected: string[]; witness: DiscoveryWitness };
const scopeConfirmations = computed<ScopeConfirmation[]>(() => data.value?.runId && proposal.value ? [{ projectId: props.projectId, conversationId: props.conversationId, generation,
  runId: data.value.runId, token: proposal.value.token, roots: [...proposal.value.roots], hosts: [...hosts.value], selected: [...selected.value], witness: { ...data.value.confirmationWitness } }] : []);
function sourceLabel(kind: string) { return ({ tls_subject: 'TLS 证书 Subject', icp: '备案单位', confirmed_directory: '已确认单位目录', public_ct_subject: '公开 CT 叶证书 Subject' } as Record<string, string>)[kind] ?? kind; }
const run = computed(() => data.value?.review?.run);
const counts = computed(() => data.value?.enrichment?.run?.counts ?? {});
const visible = computed(() => !!data.value?.runId || !!data.value?.pipeline);
const freeWorking = computed(() => ['running', 'waiting', 'waiting_storage', 'pausing'].includes(data.value?.enrichment?.run?.status ?? ''));
const pipelineStopped = computed(() => ['blocked','paused'].includes(data.value?.pipeline?.phase??''));
const phase = computed(() => pipelineStopped.value ? data.value!.pipeline!.phase : data.value?.phase ?? '');
const stopping = computed(() => phase.value === 'pausing' || data.value?.enrichment?.run?.status === 'pausing');
const storageWaiting = computed(() => data.value?.pipeline?.phase === 'waiting_storage' || data.value?.enrichment?.run?.status === 'waiting_storage');
const working = computed(() => Boolean(data.value?.enabled && !pipelineStopped.value || freeWorking.value));
const costUnknown = computed(() => data.value?.pipeline?.hunter?.costUnknown === true);
const hunterUnavailable = computed(() => Boolean(data.value?.pipeline?.allowHunter && data.value?.readiness
  && (!data.value.readiness.hunter.keyConfigured || !data.value.readiness.hunter.contractVerified)));
const scopeAvailable = computed(() => statusFresh.value && !data.value?.readOnly && !['paused','blocked','pausing'].includes(phase.value) && families.value.length > 0);
const phaseLabel = computed(() => data.value?.readOnly ? '已归档 · 只读' : !statusFresh.value ? '状态待更新' : stopping.value ? '正在保存进度' : storageWaiting.value ? '等待本地存储，自动重试' : ({ preparing: '整理全项目证据', analyzing: 'AI 提炼中', awaiting_scope: '等待一次范围确认', ready: freeWorking.value ? '已确认范围自动补证' : 'AI 提炼已完成', paused: '已暂停', blocked: '需要处理错误', collecting: '正在采集初始资产' } as Record<string, string>)[phase.value] ?? 'AI 发现资产');
const summary = computed(() => {
  if (data.value?.readOnly) return '此项目已归档，现有进度与证据可查看。';
  if (!statusFresh.value) return '状态读取失败，旧进度已保留。重新读取成功后再继续或确认范围。';
  if (stopping.value) return '正在保存当前结果并暂停，请稍候。';
  if (storageWaiting.value) return '本地存储繁忙，系统会沿原任务自动重试。';
  if (costUnknown.value) return 'Hunter 上次请求或扣费待核对。可先继续免费发现，原请求与回执保留，不会重发。';
  if (hunterUnavailable.value) return 'Hunter 账户尚未准备好，可先继续免费发现；付费补查前请到配置中心保存账户。';
  if (phase.value === 'blocked') return data.value?.pipeline?.reason || data.value?.reason || '流程暂时无法继续，已完成的结果保留。';
  if (phase.value === 'paused') return '已暂停。继续时沿用原任务和已保存结果。';
  if (scopeAvailable.value) return `已汇总 ${families.value.length} 组新增范围，确认一次后自动继续。`;
  if (working.value) return data.value?.pipeline?.reason || data.value?.reason || '后台正在自动整理、补证和发现新资产。';
  return '当前结果已保存，继续时只处理新增或变化的证据。';
});
const mainAction = computed(() => {
  if (!statusFresh.value) return 'refresh';
  if (data.value?.readOnly || stopping.value || storageWaiting.value) return '';
  if (scopeAvailable.value) return 'confirm';
  if (pipelineStopped.value && (costUnknown.value || hunterUnavailable.value)) return 'free';
  if (working.value) return 'pause';
  if (costUnknown.value || hunterUnavailable.value) return 'free';
  return 'start';
});
const knownPoints = computed(() => data.value?.pipeline?.hunter?.spentPoints);
const showHunterReceipt = computed(() => costUnknown.value || (data.value?.pipeline?.hunter?.queries ?? 0) > 0 || (knownPoints.value ?? 0) > 0);
const conflictCount = computed(() => data.value?.pipeline?.counts?.conflicts ?? run.value?.queueCounts?.conflict ?? 0);
const compactNotice = computed(() => {
  if (visible.value || errorDismissed.value) return '';
  if (error.value) return retryRead.value ? 'AI 发现状态暂时读取失败，正在自动重试。' : 'AI 发现状态暂不可用，请检查资产读取权限或更新内置组件。';
  return data.value?.phase === 'empty' ? '这个项目还没有本地资产。到资产页填写目标并开始 AI 发现，系统会先采集 FOFA，再自动接续。' : '';
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
  if (disposed || g !== generation || !pageVisible() || data.value?.readOnly) return;
  if (retryRead.value || visible.value && (data.value?.enabled || freeWorking.value)) {
    const delay = retryRead.value ? Math.min(60000, 5000 * 2 ** Math.min(failures - 1, 4)) : 5000;
    timer = window.setTimeout(() => { timer = undefined; void load(); }, delay);
  }
}
function permanentReadFailure(message: string) {
  // A missing executor and a transient native reconnect share the old Host
  // error code. Retry that ambiguity finitely; unsupported routes never poll.
  if (/discovery_(?:scope_mismatch|status_invalid)/.test(message)) return true;
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
      if (!next || typeof next.phase !== 'string' || typeof next.organization !== 'string') throw Error('discovery_status_invalid');
      const responseProject = (next as DiscoveryView & { projectId?: number }).projectId;
      const witness = next?.confirmationWitness;
      if (responseProject !== undefined && responseProject !== project) throw Error('discovery_scope_mismatch');
      if (!witness || !['appRevision','grantRevision','policyRevision'].every(key => Number.isSafeInteger(witness[key as keyof typeof witness]) && Number(witness[key as keyof typeof witness]) >= 0)
        || typeof witness.runtimeIdentity !== 'string' || !witness.runtimeIdentity) throw Error('discovery_status_invalid');
      if (next.proposal?.token !== data.value?.proposal?.token) {
        rememberChoices();
        selected.value = (next.proposal?.groups ?? []).filter(f => hasBoundEvidence(f) && !uncheckedFamilies.has(f.root)).map(f => f.root);
      }
      data.value = next; statusFresh.value = true; error.value = ''; retryRead.value = false; failures = 0; errorDismissed.value = false;
    } catch (e) {
      if (!current()) return;
      statusFresh.value = false; error.value = String((e as Error).message || e);
      failures++; retryRead.value = !permanentReadFailure(error.value);
      if (!retryRead.value) { rememberChoices(); data.value = undefined; }
    } finally { if (current()) { reading.value = false; schedule(g); } }
  })();
  flight = work;
  try { await work; } finally { if (flight === work) flight = undefined; }
}
async function control(action: 'start' | 'pause', allowHunter?: false) {
  if (busy.value || disposed || !data.value || data.value.readOnly) return;
  if (action === 'start' && (!statusFresh.value || storageWaiting.value || stopping.value)) return;
  const d = data.value, paid = allowHunter === false ? false : d.pipeline?.allowHunter;
  if (action === 'start' && paid && costUnknown.value) { error.value = 'Hunter 请求或扣费结果未知，已保留回执等待核对；可以选择继续免费发现，不会重新发送该请求。'; return; }
  if (action === 'start' && paid && hunterUnavailable.value) { error.value = '请先在配置中心保存 Hunter 账户，或选择继续免费发现。'; return; }
  const g = generation; busy.value = true; error.value = ''; notice.value = ''; readSequence++;
  try { await props.client.discoveryControl(props.conversationId, props.projectId, { ...d.confirmationWitness, action, ...(action === 'start' ? { organization: d.organization, includeIsolated: d.includeIsolated, ...(paid !== undefined ? { allowHunter: paid } : {}) } : {}) }); if (g === generation && !disposed) { await load(true); if (g === generation && allowHunter === false) notice.value = '已按本次选择继续免费发现，Hunter 原查询与费用回执保留。'; } }
  catch (e) { if (g === generation && !disposed) { const message = String((e as Error).message || e); await load(true); if (g === generation && !disposed) error.value = message; } }
  finally { if (g === generation && !disposed) busy.value = false; }
}
async function confirm(userClicked = false, displayed = scopeConfirmations.value[0]) {
  const p = proposal.value, d = data.value;
  if (!userClicked || !displayed || busy.value || disposed || !scopeAvailable.value || !statusFresh.value || d?.readOnly || !p || !d?.runId || !hosts.value.length || hosts.value.length > 500) return;
  if (displayed.projectId !== props.projectId || displayed.conversationId !== props.conversationId || displayed.generation !== generation
    || displayed.runId !== d.runId || displayed.token !== p.token || JSON.stringify(displayed.roots) !== JSON.stringify(p.roots)
    || JSON.stringify(displayed.hosts) !== JSON.stringify(hosts.value) || JSON.stringify(displayed.selected) !== JSON.stringify(selected.value)
    || !(['appRevision','grantRevision','policyRevision','runtimeIdentity'] as const).every(key => displayed.witness[key] === d.confirmationWitness[key])) {
    notice.value = '建议、选择或授权状态已更新，请按当前显示的范围确认。'; return;
  }
  const g = generation; busy.value = true; error.value = ''; notice.value = ''; readSequence++;
  try {
    const result = await props.client.confirmDiscoveryScope(displayed.conversationId, displayed.projectId, { ...displayed.witness, runId: displayed.runId, proposalToken: displayed.token, expectedRoots: [...displayed.roots], hosts: [...displayed.hosts], userConfirmed: true });
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
watch(() => [props.projectId, props.conversationId], () => { generation++; readSequence++; clearTimer(); flight = undefined; data.value = undefined; selected.value = []; uncheckedFamilies.clear(); error.value = ''; notice.value = ''; retryRead.value = false; statusFresh.value = false; expanded.value = false; adjustScopes.value = false; failures = 0; errorDismissed.value = false; busy.value = false; reading.value = false; void load(); }, { immediate: true });
watch(() => props.refreshKey, () => { if (pageVisible()) void load(true); });
onBeforeUnmount(() => { disposed = true; generation++; readSequence++; clearTimer(); if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', visibilityChanged); });
</script>

<template>
  <div v-if="compactNotice" class="chat-discovery-note" role="status" data-testid="chat-discovery-notice"><span>{{ compactNotice }}<small v-if="error">{{ error }}</small></span><button class="button ghost compact" type="button" data-testid="chat-discovery-retry" :disabled="reading" @click="load(true)">重试读取</button><button class="button ghost compact" type="button" @click="errorDismissed = true">关闭</button></div>
  <section v-else-if="visible" class="chat-discovery" data-testid="chat-discovery">
    <header><div><strong>资产发现</strong><span class="discovery-phase" :class="{attention:costUnknown||!statusFresh||phase==='blocked'}">{{ phaseLabel }}</span></div><div class="discovery-actions">
      <button v-if="mainAction!=='refresh'" v-refresh-feedback="{ scope: projectId + ':' + conversationId, busy: reading, error }" class="button ghost compact" type="button" data-testid="chat-discovery-refresh" :disabled="busy||reading" @click="load(true)">刷新</button>
      <button v-if="working&&!data?.readOnly&&['confirm','free'].includes(mainAction)" class="button ghost compact" type="button" data-testid="chat-discovery-pause-secondary" :disabled="busy" @click="control('pause')">暂停</button>
      <button v-if="mainAction==='refresh'" class="button secondary compact" type="button" data-testid="chat-discovery-retry" :disabled="busy||reading" @click="load(true)">{{reading?'正在读取…':'重试读取'}}</button>
      <button v-else-if="mainAction==='pause'" class="button secondary compact" type="button" data-testid="chat-discovery-pause" :disabled="busy" @click="control('pause')">暂停发现</button>
      <button v-else-if="mainAction==='free'" class="button primary compact" type="button" data-testid="chat-discovery-free" :disabled="busy" @click="control('start',false)">{{busy?'正在接续…':'继续免费发现'}}</button>
      <button v-else-if="mainAction==='start'" class="button primary compact" type="button" data-testid="chat-discovery-continue" :disabled="busy" @click="control('start')">{{busy?'正在接续…':'继续 AI 发现'}}</button>
    </div></header>
    <p class="discovery-target">{{ data?.organization }}</p><p class="discovery-summary" role="status" data-testid="chat-discovery-summary">{{ summary }}</p>
    <div class="discovery-overview" aria-label="发现结果摘要"><span v-if="run">项目资产 <b>{{run.totalAssets.toLocaleString()}}</b></span><span v-else-if="data?.pipeline?.counts.collectedAssets!=null">项目资产 <b>{{data.pipeline.counts.collectedAssets.toLocaleString()}}</b></span><span v-if="data?.pipeline?.counts.roundAdded!=null">本轮新增 <b>{{data.pipeline.counts.roundAdded.toLocaleString()}}</b></span><span>需要你决定 <b>{{conflictCount}}</b></span></div>
    <p class="discovery-cost" :class="{attention:costUnknown}" role="status"><template v-if="showHunterReceipt"><span v-if="knownPoints!=null">Hunter 已知消耗 {{knownPoints.toLocaleString()}} 积分</span><span v-else>Hunter 积分待回执</span><span v-if="costUnknown"> · 另有扣费待核对</span><span v-if="!data?.pipeline?.allowHunter"> · 当前仅免费发现</span></template><template v-else>{{data?.pipeline?.allowHunter?'Hunter 已允许，按实际回执显示消耗':'Hunter 0 积分 · 未启用付费补查'}}</template></p>
    <div v-if="scopeAvailable" class="discovery-consent">
      <strong>已核验的范围建议 · {{ families.length }} 组，集中确认一次</strong>
      <p>已核对本地证据与目标单位的绑定；确认后自动补证，真正冲突另行汇总。</p>
      <p class="discovery-scope-preview">{{families.slice(0,3).flatMap(family=>family.scopes.slice(0,2)).join('、')}}{{families.length>3?' 等':''}}</p>
      <details class="discovery-adjust" :open="adjustScopes" @toggle="adjustScopes=($event.target as HTMLDetailsElement).open"><summary>查看或调整范围 · 已选 {{selected.length}} / {{families.length}} 组</summary>
        <p v-if="proposal?.removedNoLiveEvidence">已省略 {{ proposal.removedNoLiveEvidence }} 项只有历史或已失效证据的范围，原记录保留。</p>
        <div class="discovery-families"><div v-for="family in families" :key="family.root" class="discovery-family"><label><input v-model="selected" type="checkbox" :value="family.root" :disabled="busy"><span><strong>{{ family.root }}</strong><small>{{ family.shared ? `共享平台，仅 ${family.scopes.length} 个具体主机` : `仅确认下列 ${family.scopes.length} 项范围` }} · {{ family.evidenceGroups }} 组证据</small></span></label><div class="discovery-verification"><small v-for="(source, index) in family.verification?.sources" :key="index">{{ sourceLabel(source.kind) }} · {{ source.host }} · {{ source.organization || '已确认的域名绑定' }}<template v-if="source.assetId"> · 资产 #{{ source.assetId }}</template><template v-if="source.validUntil"> · 证书有效至 {{source.validUntil}}</template><span v-if="source.sourceUrl" class="verification-source-url">{{source.sourceUrl}}</span></small></div><details><summary>查看本组将确认的 {{ family.scopes.length }} 项范围</summary><ul><li v-for="scope in family.scopes" :key="scope">{{ scope }}</li></ul><p v-if="family.reasons?.length">AI 推断（不作核验依据）：{{ family.reasons.join('；') }}</p></details></div></div>
      </details>
      <button v-for="confirmation in scopeConfirmations" :key="confirmation.token" class="button primary compact" type="button" data-testid="chat-discovery-confirm" :disabled="busy||!statusFresh||!hosts.length||hosts.length>500" @click="confirm(true,confirmation)">{{ busy ? '正在保存…' : `确认所选 ${selected.length} 组范围并自动继续` }}</button>
      <p v-if="hosts.length>500" class="discovery-error" role="alert">所选范围超过 500 项。展开调整范围，取消部分域名家族后再确认。</p>
    </div>
    <p v-if="notice" class="discovery-notice" role="status">{{ notice }}</p>
    <p v-if="!statusFresh&&error" class="discovery-error" role="alert">最近一次读取失败，重新读取成功后可继续操作。</p>
    <details class="discovery-details" data-testid="chat-discovery-details" :open="expanded" @toggle="expanded=($event.target as HTMLDetailsElement).open"><summary>过程、费用与详细记录</summary>
      <DiscoveryPipelineProgress v-if="data?.pipeline" :pipeline="data.pipeline" compact/>
      <div class="discovery-metrics"><span v-if="run">已整理 {{ run.processedAssets.toLocaleString() }} / {{ run.totalAssets.toLocaleString() }} 条</span><span v-if="run">模型 {{ run.requests }} 次 · {{ run.estimatedUsage ? '约 ' : '' }}{{ (run.inputTokens + run.outputTokens).toLocaleString() }} Token{{ run.estimatedUsage ? '（估算）' : '' }}</span><span v-if="data?.enrichment?.run">补证完成 {{ counts.done || 0 }} · 等待重试 {{ counts.retry || 0 }} · 失败 {{ counts.failed || 0 }}</span></div>
      <p v-if="proposal">{{proposal.networkVerified?'本次包含公开 CT 叶证书核验；证书证据不代表主机当前存活，新增探测范围仍需确认。':'本次仅核验本地证据，未联网查证企业归属。'}}</p>
      <p v-if="heldFamilies" class="discovery-held">{{ heldFamilies }} 个域名家族缺少逐域绑定依据，已暂存等待核验，不加入探测范围，无需逐项确认。<span v-if="proposal?.removedUnverified">本次拦截 {{ proposal.removedUnverified }} 项未核验范围。</span></p>
      <p v-if="hunterUnavailable">Hunter {{data?.readiness?.hunter.keyConfigured?'账户已保存，接口配置仍需更新':'账户尚未保存'}}；请到「配置中心 → 资产 API」完善设置。已保存账户不代表网络已连通。</p>
      <p v-if="error" class="discovery-error" role="alert">{{ error }}</p><p v-if="data?.enrichmentError" class="discovery-error">{{ data.enrichmentError }}</p>
    </details>
  </section>
</template>

<style scoped>
.chat-discovery-note{display:flex;align-items:center;gap:8px;flex-wrap:wrap;border:1px solid var(--line);border-radius:8px;padding:8px 10px;font-size:11px;color:var(--app-muted,#72746f)}.chat-discovery-note>span{flex:1;min-width:160px}.chat-discovery-note small{display:block;overflow-wrap:anywhere}.chat-discovery{border:1px solid var(--line);border-radius:10px;padding:14px;font-size:12px;overflow-wrap:anywhere;background:var(--app-surface,#fff);min-width:0}.chat-discovery header{display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap}.chat-discovery header>div:first-child{display:flex;align-items:center;gap:10px;flex-wrap:wrap}.discovery-phase{font-size:10px;padding:2px 6px;border:1px solid var(--line);border-radius:4px;color:var(--app-muted,#72746f)}.chat-discovery p{line-height:1.65;margin:8px 0;color:var(--app-muted,#72746f)}.chat-discovery .discovery-target{font-weight:550;color:var(--text,#30343b);margin:11px 0 4px}.chat-discovery .discovery-summary{margin:0 0 10px}.discovery-actions{display:flex;gap:6px;flex-wrap:wrap}.discovery-overview{display:flex;gap:8px 20px;flex-wrap:wrap;padding-top:3px;font-size:11px;color:var(--app-muted,#72746f)}.discovery-overview b{margin-left:5px;font-weight:600;color:var(--text,#30343b)}.chat-discovery .discovery-cost{font-size:11px;margin-top:8px}.chat-discovery .attention{color:#95641e}.discovery-consent{border-top:1px solid var(--line);padding-top:12px;margin-top:12px}.chat-discovery .discovery-scope-preview{font-family:ui-monospace,SFMono-Regular,monospace;color:var(--text,#30343b);font-size:11px;line-height:1.8}.discovery-adjust{margin:9px 0 12px}.discovery-adjust>summary,.discovery-details>summary{cursor:pointer;color:var(--app-muted,#72746f);font-size:11px}.discovery-families{max-height:280px;overflow:auto;margin:8px 0 12px;overscroll-behavior:contain}.discovery-families label{display:flex;gap:9px;padding:9px 0;border-bottom:1px solid var(--line);align-items:flex-start}.discovery-families label>span{min-width:0}.discovery-family details{margin:0 0 8px 23px;font-size:11px}.discovery-family summary{cursor:pointer;color:var(--app-muted,#72746f)}.discovery-family ul{max-height:160px;overflow:auto;padding-left:18px;line-height:1.7}.discovery-families input{margin:3px 0 0;flex-shrink:0}.discovery-families small{display:block;margin-top:3px;line-height:1.5}.discovery-verification{margin:0 0 8px 23px;overflow-wrap:anywhere}.verification-source-url{display:block;overflow-wrap:anywhere}.discovery-details{border-top:1px solid var(--line);padding-top:9px;margin-top:11px}.discovery-metrics{display:flex;gap:7px 16px;flex-wrap:wrap;font-size:11px;color:var(--app-muted,#72746f);margin-top:9px}.discovery-held{font-size:11px}.chat-discovery .discovery-notice{font-size:11px;color:#246c4e}.chat-discovery .discovery-error{color:#ac3939}.chat-discovery .button:disabled{cursor:default}@media(max-width:600px){.chat-discovery{padding:11px}.discovery-consent>.button{width:100%}.discovery-actions{margin-left:auto}.discovery-actions>.primary{flex:1}.discovery-overview{gap:7px 14px}}
</style>
