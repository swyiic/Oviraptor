<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import { Bot, Play, Square, RefreshCw } from '@lucide/vue';
import { invoke } from '../uiPorts';
import HunterExpansionProgress, { type HunterExpansion } from './HunterExpansionProgress.vue';
const props = defineProps<{ projectId?: number; archived?: boolean }>();
const emit = defineEmits<{ changed: [] }>();
type Limits = { maxRounds: number; maxAssets: number; maxMinutes: number; hunterRequests: number; hunterEnabled: boolean };
type Step = { round: number; action: string; reason: string; state: string; result: unknown };
type Run = { id: string; status: string; roots: string[]; model: string; round: number; probed: number; hunterRequests: number; activeJobId?: number; summary: string; limits: Limits; steps: Step[] };
type Discovery = { expansions?:HunterExpansion[]; roots: string[]; settings: { keyConfigured: boolean; contractVerified: boolean } };
const opened = ref(false), goal = ref('根据当前项目已发现的证书、域名和 CNAME 线索，持续发现并探测确认范围内的资产，优先补充高价值线索，区分存活结果与归属证据。');
const limits = ref<Limits>({ maxRounds: 8, maxAssets: 100, maxMinutes: 15, hunterRequests: 0, hunterEnabled:true });
const run = ref<Run | null>(null), discovery = ref<Discovery>(), loading = ref(false), busy = ref(false), error = ref('');
let hunterPendingUntil=0;
let generation = 0, disposed = false, timer: ReturnType<typeof setTimeout> | undefined;
const running = computed(() => !!run.value && ['running', 'stopping'].includes(run.value.status));
const hunterAvailable = computed(() => !!discovery.value?.settings.keyConfigured && !!discovery.value?.settings.contractVerified);
const canStart = computed(() => !!props.projectId && !props.archived && !running.value && !busy.value && !loading.value && !!discovery.value?.roots.length && !!goal.value.trim());
const labels: Record<string, string> = { completed_step: '完成', running: '执行中', stopping: '正在停止', stopped: '已停止', completed: '本轮结束', interrupted: '已中断', failed: '失败' };
const actions: Record<string, string> = { inventory: '读取资产', harvest: '提取线索', ct: '查询 CT', dns: '查询 CNAME', hunter: 'Hunter 精确补查', probe: '探测现有资产', probeHosts: '探测域名线索', stop: '结束本轮' };
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
  if (result?.receipt && "spentPoints" in result.receipt) return `已核对消耗 ${(result.receipt as {spentPoints:unknown}).spentPoints} 积分，完整分页回执见上方。`;
  return result?.receipt ? [result.receipt.consumeQuota || '接口未报告消耗', result.receipt.restQuota || '接口未报告余额'].join(' · ') : step.state === 'failed' ? '没有成功回执，消耗未确认；未自动重试' : '';
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
  <summary><span class="agent-title"><Bot :size="18" />自主探测 Agent</span><span class="agent-badge" :data-status="run?.status">{{run ? labels[run.status] || run.status : '根据线索持续规划与执行'}}</span></summary>
  <div class="agent-content">
    <p class="agent-intro">Agent 根据已有证书、域名与 CNAME 线索规划下一步，执行后读取结果，再继续发现与探测。</p>
    <p v-if="!projectId">请先选择工作空间。</p>
    <template v-else>
      <div class="agent-scope" :class="{'scope-missing':!discovery?.roots.length}" role="status"><strong>{{discovery?.roots.length?'已确认的探测范围':'尚未设置探测范围'}}</strong><div v-if="discovery?.roots.length" class="agent-scope-domains"><span v-for="root in discovery.roots" :key="root">{{root}}</span></div><p v-else>请先在「新建查询 → 域名线索与补查」保存范围，再刷新此面板。未确认范围时不会启动探测。</p></div>
      <label class="field agent-goal"><span>本轮目标</span><textarea v-model="goal" rows="3" maxlength="2000" :disabled="running||busy" /></label>
      <section class="agent-run-limits"><h4>运行限制</h4><div class="agent-limits">
        <label class="field"><span>最多规划轮数</span><input v-model.number="limits.maxRounds" type="number" min="1" max="32" :disabled="running||busy" /></label>
        <label class="field"><span>最多探测资产</span><input v-model.number="limits.maxAssets" type="number" min="1" max="1000" :disabled="running||busy" /></label>
        <label class="field"><span>时长上限（分钟）</span><input v-model.number="limits.maxMinutes" type="number" min="1" max="60" :disabled="running||busy" /></label>
      </div></section>
      <section class="agent-hunter-permission"><label class="agent-hunter-toggle"><input type="checkbox" :checked="hunterAvailable&&limits.hunterEnabled" :disabled="running||busy||!hunterAvailable" @change="limits.hunterEnabled=($event.target as HTMLInputElement).checked"/><span><strong>允许 Hunter 精确关联补查</strong><small>按真实证书指纹、域名或 CNAME 自动翻页，去重合并关联资产。</small></span><span class="hunter-permission-state">{{!hunterAvailable?'未配置':limits.hunterEnabled?'已允许':'未启用'}}</span></label><p v-if="!hunterAvailable" class="hunter-unavailable">请先在「配置中心 → 资产 API」配置 Hunter API Key。</p><p class="agent-cost-note">可使用全部免费与权益积分，不设积分预算。每页与累计消耗会显示在进度中，可随时暂停。</p><details class="agent-execution-note"><summary>查询与数据使用说明</summary><p>只对有实际来源的精确线索补查；结果或扣费未知时暂停，不自动重发。CT / DNS 会向对应服务提交域名，模型会接收线索与探测摘要。发现和存活结果仍需核验归属。</p></details></section>
      <div class="agent-actions">
        <button class="button primary" data-agent-action="start" :disabled="!canStart" @click="start"><Play :size="15" />{{busy&&!running?'正在启动…':'开始自主探测'}}</button>
        <button v-if="running" class="button danger" data-agent-action="stop" :disabled="busy||run?.status==='stopping'" @click="stop"><Square :size="15" />{{run?.status==='stopping'?'正在停止…':'停止探测'}}</button>
        <button class="button ghost" :disabled="busy||loading" @click="load"><RefreshCw :size="15" />刷新状态与范围</button>
      </div>
      <p v-if="!running&&!loading&&!canStart" class="agent-start-hint">{{archived?'当前工作空间已归档，无法启动新任务。':!discovery?.roots.length?'保存并确认探测范围后，即可开始。':!goal.trim()?'请填写本轮目标。':''}}</p>
      <div v-if="run" class="agent-status" role="status">
        <strong>{{labels[run.status]||run.status}} · {{run.model}}</strong>
        <p>规划 {{run.round}} / {{run.limits.maxRounds}} 轮 · 探测 {{run.probed}} / {{run.limits.maxAssets}} 条 · Hunter 已规划 {{run.hunterRequests}} 个精确查询（分页 / 积分见下方）<span v-if="run.activeJobId"> · 原生任务 #{{run.activeJobId}}</span></p>
        <p>{{run.summary}}</p>
        <p class="helper">停止后等待当前请求退出；关闭应用后保留记录，未知请求不会自动重放。</p>
        <HunterExpansionProgress :project-id="projectId" :items="discovery?.expansions" @changed="refreshHunterProgress" /><ol v-if="run.steps.length" class="agent-steps"><li v-for="step in run.steps" :key="step.round"><div><strong>第 {{step.round}} 轮 · {{actions[step.action]||step.action}}</strong><span class="agent-step-state" :data-status="step.state">{{labels[step.state]||step.state}}</span></div><p>{{step.reason}}</p><p v-if="step.action==='hunter'&&step.state!=='running'" class="helper">{{hunterQuota(step)}}</p><details v-if="step.state!=='running'"><summary>执行结果</summary><pre>{{JSON.stringify(step.result,null,2)}}</pre></details></li></ol>
      </div>
    </template>
    <p v-if="loading" role="status">正在读取范围与运行记录…</p><p v-if="error" class="form-error" role="alert">{{error}}</p>
  </div>
</details>
</template>
<style scoped>
.autonomous-probe{margin:0 0 18px;overflow:hidden}.autonomous-probe>summary{padding:16px 20px;display:flex;align-items:center;justify-content:space-between;gap:12px;cursor:pointer;list-style:none}.autonomous-probe>summary::-webkit-details-marker{display:none}.autonomous-probe>summary::after{content:'展开';font-size:12px;color:var(--muted);flex-shrink:0}.autonomous-probe[open]>summary::after{content:'收起'}.agent-title{display:flex;gap:8px;align-items:center;font-weight:650;font-size:15px}.agent-badge{margin-left:auto;border-radius:6px;padding:4px 8px;font-size:11px;background:var(--app-sidebar,#f3f3f1);color:var(--muted)}.agent-badge[data-status=running]{color:#2463b4;background:#eaf2ff}.agent-badge[data-status=completed],.agent-step-state[data-status=completed]{background:#e6f6ee;color:#18734d}.agent-badge[data-status=interrupted],.agent-badge[data-status=stopping]{background:#fff3df;color:#9b5c08}
.agent-content{padding:20px;border-top:1px solid var(--line);font-size:13px;line-height:1.65}.agent-intro{margin:0 0 16px;color:var(--muted);font-size:12px}.agent-scope{padding:12px 14px;border:1px solid var(--line);border-radius:8px;background:var(--app-bg,#fafafa);margin-bottom:20px;overflow-wrap:anywhere}.agent-scope strong{font-size:12px;font-weight:600}.agent-scope p{margin:4px 0 0;font-size:12px;color:var(--muted)}.agent-scope.scope-missing{background:#fff9ef;border-color:#eedab5}.agent-scope.scope-missing strong{color:#906214}.agent-scope-domains{display:flex;flex-wrap:wrap;gap:6px;margin-top:8px}.agent-scope-domains span{padding:2px 8px;background:var(--app-surface,#fff);border:1px solid var(--line);border-radius:5px;font-size:12px}
.agent-content .field{display:flex;flex-direction:column;gap:8px;margin:0;min-width:0}.agent-content .field>span{font-size:12px;font-weight:550}.agent-goal textarea{display:block;width:100%;box-sizing:border-box;min-height:96px;resize:vertical;font-size:13px;line-height:1.7;padding:12px}.agent-run-limits{margin-top:22px}.agent-run-limits h4{font-size:12px;margin:0 0 12px;font-weight:600}.agent-limits{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}.agent-limits input[type=number]{width:100%;box-sizing:border-box;min-height:38px;font-size:13px}
.agent-hunter-permission{margin-top:22px;padding:16px;border:1px solid var(--line);border-radius:8px;background:var(--app-bg,#fafafa)}.agent-hunter-toggle{display:flex;align-items:flex-start;gap:10px;cursor:pointer}.agent-hunter-toggle>input[type=checkbox]{flex:0 0 17px;width:17px;height:17px;min-height:0;margin:3px 0 0;padding:0;accent-color:var(--blue)}.agent-hunter-toggle>span:first-of-type{min-width:0;flex:1}.agent-hunter-toggle strong{display:block;font-size:13px;font-weight:550}.agent-hunter-toggle small{display:block;font-size:12px;color:var(--muted);margin-top:3px;line-height:1.6}.hunter-permission-state{flex-shrink:0;font-size:11px;color:var(--muted);border:1px solid var(--line);border-radius:5px;padding:1px 7px}.agent-cost-note,.hunter-unavailable{margin:10px 0 0 27px;font-size:12px;color:var(--muted)}.hunter-unavailable{color:#94620f}.agent-execution-note{margin:10px 0 0 27px;font-size:12px;color:var(--muted)}.agent-execution-note summary{cursor:pointer}.agent-execution-note p{margin:8px 0 0;line-height:1.7}
.agent-actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin:22px 0 0;padding-top:18px;border-top:1px solid var(--line)}.agent-actions .button{min-height:38px;font-size:12px}.agent-start-hint{margin:8px 0 0;font-size:12px;color:var(--muted)}.agent-status{margin-top:20px;padding:16px;border:1px solid var(--line);border-radius:9px}.agent-status p{margin:7px 0}.agent-steps{padding-left:20px;max-height:420px;overflow:auto}.agent-steps li{padding:10px 4px;border-top:1px solid var(--line)}.agent-steps li>div{display:flex;gap:12px;justify-content:space-between}.agent-step-state{font-size:11px;padding:2px 6px;border-radius:4px;background:var(--app-bg,#f4f5f7)}.agent-step-state[data-status=failed]{color:#b54444;background:#fff1f1}pre{font-size:11px;white-space:pre-wrap;overflow-wrap:anywhere;max-height:240px;overflow:auto}
@media(max-width:650px){.autonomous-probe>summary{padding:14px 16px;flex-wrap:wrap}.agent-title{font-size:14px}.agent-badge{order:3;margin:0;width:100%;background:transparent;padding:0}.agent-content{padding:16px}.agent-limits{grid-template-columns:1fr;gap:12px}.agent-hunter-permission{padding:12px}.agent-cost-note,.agent-execution-note,.hunter-unavailable{margin-left:0}.agent-actions .button{flex:1 0 auto}.hunter-permission-state{max-width:70px;white-space:normal}}
</style>
