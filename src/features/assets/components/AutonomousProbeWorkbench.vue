<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, onUnmounted, ref, watch } from 'vue';
import { Bot, Play, Square, RefreshCw } from '@lucide/vue';
import { invoke, useI18n } from '../uiPorts';
import HunterExpansionProgress, { type HunterExpansion } from './HunterExpansionProgress.vue';
const props = defineProps<{ projectId?: number; archived?: boolean }>();
const emit = defineEmits<{ changed: [] }>();
const { tr } = useI18n();
type Limits = { maxRounds: number; maxAssets: number; maxMinutes: number; hunterRequests: number; hunterEnabled: boolean };
type Step = { round: number; action: string; reason: string; state: string; result: unknown };
type Run = { id: string; status: string; roots: string[]; model: string; round: number; probed: number; hunterRequests: number; activeJobId?: number; summary: string; limits: Limits; steps: Step[] };
type Discovery = { expansions?:HunterExpansion[]; roots: string[]; settings: { keyConfigured: boolean; contractVerified: boolean } };
const opened = ref(false), goal = ref(tr('根据当前项目已发现的证书、域名和 CNAME 线索，持续发现并探测确认范围内的资产，优先补充高价值线索，区分存活结果与归属证据。', 'Using the certificate, domain and CNAME leads already found in this workspace, keep discovering and probing assets within the confirmed scope. Prioritize high-value leads and keep liveness results separate from ownership evidence.'));
const limits = ref<Limits>({ maxRounds: 8, maxAssets: 100, maxMinutes: 15, hunterRequests: 0, hunterEnabled:true });
const run = ref<Run | null>(null), discovery = ref<Discovery>(), loading = ref(false), busy = ref(false), error = ref('');
let hunterPendingUntil=0;
let generation = 0, disposed = false, timer: ReturnType<typeof setTimeout> | undefined;
const running = computed(() => !!run.value && ['running', 'stopping'].includes(run.value.status));
const hunterAvailable = computed(() => !!discovery.value?.settings.keyConfigured && !!discovery.value?.settings.contractVerified);
const canStart = computed(() => !!props.projectId && !props.archived && !running.value && !busy.value && !loading.value && !!discovery.value?.roots.length && !!goal.value.trim());
const labels = computed<Record<string, string>>(() => ({ limited:tr('达到本轮限制 · 尚未完成', 'Run limit reached · not finished'),needs_review:tr('仍有未读取项 · 待核对', 'Unread items remain · needs review'), completed_step: tr('完成', 'Done'), running: tr('执行中', 'Running'), stopping: tr('正在停止', 'Stopping'), stopped: tr('已停止', 'Stopped'), completed: tr('本轮结束', 'Run finished'), interrupted: tr('已中断', 'Interrupted'), failed: tr('失败', 'Failed') }));
const actions = computed<Record<string, string>>(() => ({ inventory: tr('读取资产', 'Read assets'), harvest: tr('提取线索', 'Extract leads'), ct: tr('查询 CT', 'Query CT'), dns: tr('查询 CNAME', 'Query CNAME'), hunter: tr('Hunter 精确补查', 'Hunter exact lookup'), probe: tr('探测现有资产', 'Probe existing assets'), probeHosts: tr('探测域名线索', 'Probe domain leads'), stop: tr('结束本轮', 'End run') }));
function schedule(g: number) { clearTimeout(timer); if (!disposed && g === generation && (running.value||Date.now()<hunterPendingUntil||discovery.value?.expansions?.some(i=>["running","stopping"].includes(i.status)))) timer = setTimeout(() => void poll(g), 2000); }
async function poll(g = generation) {
  const projectId = props.projectId;
  if (!projectId || disposed || g !== generation) return;
  try {
    const [next,scope] = await Promise.all([invoke<Run | null>('autonomous_probe_status', { projectId }),invoke<Discovery>('get_asset_discovery',{projectId})]);
    if (disposed || g !== generation) return;
    const wasRunning = running.value; run.value = next; discovery.value=scope; error.value = '';
    if (wasRunning && !running.value) emit('changed');
  } catch (e) { if (!disposed && g === generation) error.value = String(e); }
  finally { schedule(g); }
}
async function load() {
  const projectId = props.projectId, g = ++generation;
  clearTimeout(timer); run.value = null; discovery.value = undefined; error.value = ''; busy.value = false;
  if (!projectId) { loading.value = false; return; }
  loading.value = true;
  try {
    const [scope, status] = await Promise.all([invoke<Discovery>('get_asset_discovery', { projectId }), invoke<Run | null>('autonomous_probe_status', { projectId })]);
    if (disposed || g !== generation) return;
    discovery.value = scope; run.value = status;
    if (status && ['running', 'stopping'].includes(status.status)) opened.value = true;
  } catch (e) { if (!disposed && g === generation) error.value = String(e); }
  finally { if (!disposed && g === generation) { loading.value = false; schedule(g); } }
}
function refreshHunterProgress(){hunterPendingUntil=Date.now()+5000;void poll();}
function hunterQuota(step: Step) {
  const result = step.result as { receipt?: { consumeQuota?: string; restQuota?: string } };
  if (result?.receipt && "spentPoints" in result.receipt) { const spent = (result.receipt as {spentPoints:unknown}).spentPoints; return tr(`已核对消耗 ${spent} 积分，完整分页回执见上方。`, `Verified spend: ${spent} points. Full per-page receipts are shown above.`); }
  return result?.receipt ? [result.receipt.consumeQuota || tr('接口未报告消耗', 'API did not report usage'), result.receipt.restQuota || tr('接口未报告余额', 'API did not report balance')].join(' · ') : step.state === 'failed' ? tr('没有成功回执，消耗未确认；未自动重试', 'No successful receipt; spend unconfirmed. Not retried automatically.') : '';
}
function hunterPermission(limits: Limits) {
  const permission = limits.hunterEnabled ? tr('已允许精确补查', 'exact lookups allowed') : tr('未启用', 'off');
  const cap = limits.hunterEnabled && limits.hunterRequests === 0 ? tr(' · 不设查询数和积分预算', ' · no query or points budget') : limits.hunterRequests > 0 ? tr(` · 最多 ${limits.hunterRequests} 个查询`, ` · up to ${limits.hunterRequests} queries`) : '';
  return tr(`本轮启动时的 Hunter 权限：${permission}${cap}。上面的复选框用于下一轮。`, `Hunter permission when this run started: ${permission}${cap}. The checkbox above applies to the next run.`);
}
async function start() {
  if (!canStart.value || !props.projectId) return;
  const g = generation, projectId = props.projectId;
  busy.value = true; error.value = ''; opened.value = true;
  try {
    await invoke<string>('start_autonomous_probe', { input: { projectId, goal: goal.value.trim(), limits: { ...limits.value, hunterRequests:0,hunterEnabled:hunterAvailable.value&&limits.value.hunterEnabled } } });
    if (!disposed && g === generation) await poll(g);
  } catch (e) { if (!disposed && g === generation) error.value = String(e); }
  finally { if (!disposed && g === generation) busy.value = false; }
}
async function stop() {
  if (!running.value || !run.value || !props.projectId || busy.value) return;
  const g = generation, projectId = props.projectId, runId = run.value.id;
  busy.value = true; error.value = '';
  try {
    await invoke<void>('stop_autonomous_probe', { projectId, runId });
    if (!disposed && g === generation) await poll(g);
  } catch (e) { if (!disposed && g === generation) error.value = String(e); }
  finally { if (!disposed && g === generation) busy.value = false; }
}
watch(() => props.projectId, load, { immediate: true });
onUnmounted(() => { disposed = true; generation++; clearTimeout(timer); });
</script>
<template>
<details class="panel autonomous-probe" :open="opened" @toggle="opened=($event.target as HTMLDetailsElement).open" data-testid="autonomous-probe">
  <summary :data-expand="tr('展开', 'Expand')" :data-collapse="tr('收起', 'Collapse')"><span class="agent-title"><Bot :size="18" />{{ tr('自主探测 Agent', 'Autonomous probe Agent') }}</span><span class="agent-badge" :data-status="run?.status">{{run ? labels[run.status] || run.status : tr('根据线索持续规划与执行', 'Plans and acts continuously from leads')}}</span></summary>
  <div class="agent-content">
    <p class="agent-intro">{{ tr('Agent 根据已有证书、域名与 CNAME 线索规划下一步，执行后读取结果，再继续发现与探测。', 'The Agent plans its next step from existing certificate, domain and CNAME leads, reads the results after each step, then keeps discovering and probing.') }}</p>
    <p v-if="!projectId">{{ tr('请先选择工作空间。', 'Select a workspace first.') }}</p>
    <template v-else>
      <div class="agent-scope" :class="{'scope-missing':!discovery?.roots.length}" role="status"><strong>{{discovery?.roots.length?tr('已确认的探测范围', 'Confirmed probe scope'):tr('尚未设置探测范围', 'Probe scope not set')}}</strong><div v-if="discovery?.roots.length" class="agent-scope-domains"><span v-for="root in discovery.roots" :key="root">{{root}}</span></div><p v-else>{{ tr('请先在「发现资产 → 更多操作 → 管理确认范围」保存范围，再刷新此面板。未确认范围时不会启动探测。', 'Save a scope under "Discover assets → More actions → Manage confirmed scope", then refresh this panel. Probing does not start without a confirmed scope.') }}</p></div>
      <label class="field agent-goal"><span>{{ tr('本轮目标', 'Run goal') }}</span><textarea v-model="goal" rows="3" maxlength="2000" :disabled="running||busy" /></label>
      <section class="agent-run-limits"><h4>{{ tr('本轮主动探测的停止条件', 'Stop conditions for active probing in this run') }}</h4><p class="helper">{{ tr('只限制访问目标的主动任务，不是 Hunter 积分预算，也不限制本地证书分析。达到限制表示本轮暂停推进，不代表全项目已查完。', 'These limits apply only to active tasks that contact targets. They are not a Hunter points budget and do not limit local certificate analysis. Reaching a limit pauses this run; it does not mean the whole workspace has been covered.') }}</p><details><summary>{{ tr(`当前：${limits.maxAssets} 条主动探测 · ${limits.maxMinutes} 分钟 · ${limits.maxRounds} 次 AI 决策（点击调整）`, `Current: ${limits.maxAssets} active probes · ${limits.maxMinutes} min · ${limits.maxRounds} AI decisions (click to adjust)`) }}</summary><div class="agent-limits">
        <label class="field"><span>{{ tr('AI 决策次数上限', 'Max AI decisions') }}</span><input v-model.number="limits.maxRounds" type="number" min="1" max="32" :disabled="running||busy" /></label>
        <label class="field"><span>{{ tr('主动访问的资产条数上限', 'Max assets to contact') }}</span><input v-model.number="limits.maxAssets" type="number" min="1" max="1000" :disabled="running||busy" /></label>
        <label class="field"><span>{{ tr('本轮主动任务时长（分钟）', 'Active task time limit (minutes)') }}</span><input v-model.number="limits.maxMinutes" type="number" min="1" max="60" :disabled="running||busy" /></label>
      </div><p class="helper">{{ tr('一次模型规划/读取工具结果算一轮；资产条数统计实际提交的主动探测目标。资产条数不会截断 Hunter 的返回条数。本轮时长覆盖规划与等待，可能暂停 Hunter 分页；页码已独立保存，可在 Hunter 进度面板继续。', 'Each model planning step or tool-result read counts as one round. The asset count tracks probe targets actually submitted and does not truncate Hunter results. The time limit covers planning and waiting and may pause Hunter paging; the page position is saved separately and can be resumed from the Hunter progress panel.') }}</p></details></section>
      <section class="agent-hunter-permission"><label class="agent-hunter-toggle"><input type="checkbox" :checked="hunterAvailable&&limits.hunterEnabled" :disabled="running||busy||!hunterAvailable" @change="limits.hunterEnabled=($event.target as HTMLInputElement).checked"/><span><strong>{{ tr('允许 Hunter 精确关联补查', 'Allow Hunter exact related lookups') }}</strong><small>{{ tr('按真实证书指纹、域名或 CNAME 自动翻页，去重合并关联资产。', 'Pages through results by real certificate fingerprint, domain or CNAME, and merges related assets without duplicates.') }}</small></span><span class="hunter-permission-state">{{!hunterAvailable?tr('未配置', 'Not configured'):limits.hunterEnabled?tr('已允许', 'Allowed'):tr('未启用', 'Off')}}</span></label><p v-if="!hunterAvailable" class="hunter-unavailable">{{ tr('请先在「配置中心 → 资产 API」配置 Hunter API Key。', 'Configure a Hunter API key in "Settings → Asset APIs" first.') }}</p><p class="agent-cost-note">{{ tr('可使用全部免费与权益积分，不设积分预算。每页与累计消耗会显示在进度中，可随时暂停。', 'May use all free and plan points; no points budget is set. Per-page and total spend appear in the progress view, and you can pause at any time.') }}</p><details class="agent-execution-note"><summary>{{ tr('查询与数据使用说明', 'How queries and data are used') }}</summary><p>{{ tr('只对有实际来源的精确线索补查；结果或扣费未知时暂停，不自动重发。CT / DNS 会向对应服务提交域名，模型会接收线索与探测摘要。发现和存活结果仍需核验归属。', 'Lookups run only for exact leads with a real source. If a result or charge is unknown, the run pauses and does not resend automatically. CT / DNS lookups send domains to those services, and the model receives leads and probe summaries. Discovered and live results still need ownership verification.') }}</p></details></section>
      <div class="agent-actions">
        <button class="button primary" data-agent-action="start" :disabled="!canStart" @click="start"><Play :size="15" />{{busy&&!running?tr('正在启动…', 'Starting…'):tr('开始自主探测', 'Start autonomous probe')}}</button>
        <button v-if="running" class="button danger" data-agent-action="stop" :disabled="busy||run?.status==='stopping'" @click="stop"><Square :size="15" />{{run?.status==='stopping'?tr('正在停止…', 'Stopping…'):tr('停止探测', 'Stop probing')}}</button>
        <button v-refresh-feedback="{scope:projectId,busy:loading,error}" class="button ghost" :disabled="busy||loading" @click="load"><RefreshCw :size="15" />{{ tr('刷新状态与范围', 'Refresh status and scope') }}</button>
      </div>
      <p v-if="!running&&!loading&&!canStart" class="agent-start-hint">{{archived?tr('当前工作空间已归档，无法启动新任务。', 'This workspace is archived; new runs cannot be started.'):!discovery?.roots.length?tr('保存并确认探测范围后，即可开始。', 'Save and confirm a probe scope to start.'):!goal.trim()?tr('请填写本轮目标。', 'Enter a goal for this run.'):''}}</p>
      <div v-if="run" class="agent-status" role="status">
        <strong>{{labels[run.status]||run.status}} · {{run.model}}</strong>
        <p>{{ tr(`规划 ${run.round} / ${run.limits.maxRounds} 轮 · 探测 ${run.probed} / ${run.limits.maxAssets} 条 · Hunter 已规划 ${run.hunterRequests} 个精确查询（分页 / 积分见下方）`, `Planned ${run.round} / ${run.limits.maxRounds} rounds · Probed ${run.probed} / ${run.limits.maxAssets} · Hunter: ${run.hunterRequests} exact queries planned (pages / points below)`) }}<span v-if="run.activeJobId">{{ tr(` · 原生任务 #${run.activeJobId}`, ` · Native job #${run.activeJobId}`) }}</span></p>
        <p class="helper">{{ hunterPermission(run.limits) }}</p><p>{{run.summary}}</p>
        <p class="helper">{{ tr('停止后等待当前请求退出；关闭应用后保留记录，未知请求不会自动重放。', 'After you stop, the current request is allowed to finish. Records are kept after the app closes, and requests with unknown outcomes are not replayed automatically.') }}</p>
        <HunterExpansionProgress :project-id="projectId" :items="discovery?.expansions" @changed="refreshHunterProgress" /><ol v-if="run.steps.length" class="agent-steps"><li v-for="step in run.steps" :key="step.round"><div><strong>{{ tr(`第 ${step.round} 轮 · ${actions[step.action]||step.action}`, `Round ${step.round} · ${actions[step.action]||step.action}`) }}</strong><span class="agent-step-state" :data-status="step.state">{{step.state==='completed'?tr('完成', 'Done'):labels[step.state]||step.state}}</span></div><p>{{step.reason}}</p><p v-if="step.action==='hunter'&&step.state!=='running'" class="helper">{{hunterQuota(step)}}</p><details v-if="step.state!=='running'"><summary>{{ tr('执行结果', 'Result') }}</summary><pre>{{JSON.stringify(step.result,null,2)}}</pre></details></li></ol>
      </div>
    </template>
    <p v-if="loading" role="status">{{ tr('正在读取范围与运行记录…', 'Loading scope and run history…') }}</p><p v-if="error" class="form-error" role="alert">{{error}}</p>
  </div>
</details>
</template>
<style scoped>
.autonomous-probe{margin:0 0 20px;overflow:hidden}.autonomous-probe>summary{padding:16px 20px;display:flex;align-items:center;justify-content:space-between;gap:12px;cursor:pointer;list-style:none}.autonomous-probe>summary::-webkit-details-marker{display:none}.autonomous-probe>summary::after{content:attr(data-expand);font-size:var(--text-body);color:var(--muted);flex-shrink:0}.autonomous-probe[open]>summary::after{content:attr(data-collapse)}.agent-title{display:flex;gap:8px;align-items:center;font-weight:650;font-size:var(--text-heading)}.agent-badge{margin-left:auto;border-radius:var(--radius-sm);padding:4px 8px;font-size:var(--text-meta);background:var(--app-sidebar,var(--ui-surface-sunken));color:var(--muted)}.agent-badge[data-status=running]{color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));background:color-mix(in srgb, var(--blue) 10%, var(--ui-surface))}.agent-badge[data-status=completed],.agent-step-state[data-status=completed]{background:color-mix(in srgb, var(--green) 11%, var(--ui-surface));color:color-mix(in srgb, var(--green) 82%, var(--ui-ink))}.agent-badge[data-status=limited],.agent-badge[data-status=needs_review],.agent-badge[data-status=interrupted],.agent-badge[data-status=stopping]{background:color-mix(in srgb, var(--amber) 14%, var(--ui-surface));color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}
.agent-content{padding:20px;border-top:1px solid var(--line);font-size:var(--text-strong);line-height:1.65}.agent-intro{margin:0 0 16px;color:var(--muted);font-size:var(--text-body)}.agent-scope{padding:12px 14px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--app-bg,var(--ui-surface-raised));margin-bottom:20px;overflow-wrap:anywhere}.agent-scope strong{font-size:var(--text-body);font-weight:600}.agent-scope p{margin:4px 0 0;font-size:var(--text-body);color:var(--muted)}.agent-scope.scope-missing{background:color-mix(in srgb, var(--amber) 7%, var(--ui-surface));border-color:color-mix(in srgb, var(--amber) 40%, var(--ui-line))}.agent-scope.scope-missing strong{color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}.agent-scope-domains{display:flex;flex-wrap:wrap;gap:6px;margin-top:8px}.agent-scope-domains span{padding:2px 8px;background:var(--app-surface,var(--ui-surface));border:1px solid var(--line);border-radius:var(--radius-sm);font-size:var(--text-body)}
.agent-content .field{display:flex;flex-direction:column;gap:8px;margin:0;min-width:0}.agent-content .field>span{font-size:var(--text-body);font-weight:550}.agent-goal textarea{display:block;width:100%;box-sizing:border-box;min-height:96px;resize:vertical;font-size:var(--text-strong);line-height:1.7;padding:12px}.agent-run-limits{margin-top:24px}.agent-run-limits h4{font-size:var(--text-body);margin:0 0 12px;font-weight:600}.agent-run-limits details>summary{cursor:pointer;font-size:var(--text-body);color:var(--muted);margin-bottom:12px}.agent-limits{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}.agent-limits input[type=number]{width:100%;box-sizing:border-box;min-height:38px;font-size:var(--text-strong)}
.agent-hunter-permission{margin-top:24px;padding:16px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--app-bg,var(--ui-surface-raised))}.agent-hunter-toggle{display:flex;align-items:flex-start;gap:10px;cursor:pointer}.agent-hunter-toggle>input[type=checkbox]{flex:0 0 17px;width:17px;height:17px;min-height:0;margin:3px 0 0;padding:0;accent-color:var(--blue)}.agent-hunter-toggle>span:first-of-type{min-width:0;flex:1}.agent-hunter-toggle strong{display:block;font-size:var(--text-strong);font-weight:550}.agent-hunter-toggle small{display:block;font-size:var(--text-body);color:var(--muted);margin-top:3px;line-height:1.6}.hunter-permission-state{flex-shrink:0;font-size:var(--text-meta);color:var(--muted);border:1px solid var(--line);border-radius:var(--radius-sm);padding:1px 8px}.agent-cost-note,.hunter-unavailable{margin:10px 0 0 28px;font-size:var(--text-body);color:var(--muted)}.hunter-unavailable{color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}.agent-execution-note{margin:10px 0 0 28px;font-size:var(--text-body);color:var(--muted)}.agent-execution-note summary{cursor:pointer}.agent-execution-note p{margin:8px 0 0;line-height:1.7}
.agent-actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin:24px 0 0;padding-top:20px;border-top:1px solid var(--line)}.agent-actions .button{min-height:38px;font-size:var(--text-body)}.agent-start-hint{margin:8px 0 0;font-size:var(--text-body);color:var(--muted)}.agent-status{margin-top:20px;padding:16px;border:1px solid var(--line);border-radius:var(--radius-md)}.agent-status p{margin:8px 0}.agent-steps{padding-left:20px;max-height:420px;overflow:auto}.agent-steps li{padding:10px 4px;border-top:1px solid var(--line)}.agent-steps li>div{display:flex;gap:12px;justify-content:space-between}.agent-step-state{font-size:var(--text-meta);padding:2px 6px;border-radius:var(--radius-xs);background:var(--app-bg,var(--ui-surface-raised))}.agent-step-state[data-status=failed]{color:color-mix(in srgb, var(--red) 82%, var(--ui-ink));background:color-mix(in srgb, var(--red) 7%, var(--ui-surface))}pre{font-size:var(--text-meta);white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto}
@media(max-width:650px){.autonomous-probe>summary{padding:14px 16px;flex-wrap:wrap}.agent-title{font-size:var(--text-heading)}.agent-badge{order:3;margin:0;width:100%;background:transparent;padding:0}.agent-content{padding:16px}.agent-limits{grid-template-columns:1fr;gap:12px}.agent-hunter-permission{padding:12px}.agent-cost-note,.agent-execution-note,.hunter-unavailable{margin-left:0}.agent-actions .button{flex:1 0 auto}.hunter-permission-state{max-width:70px;white-space:normal}}
</style>
