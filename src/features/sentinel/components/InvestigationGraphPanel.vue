<script setup lang="ts">
import { computed, ref } from "vue";
import {
  ArrowRight,
  Bot,
  Braces,
  CheckCircle2,
  CircleStop,
  GitCompareArrows,
  MousePointerClick,
  Network,
  ShieldCheck,
  Sparkles,
} from "@lucide/vue";
import type {
  InvestigationGraph,
  InvestigationHypothesis,
  InvestigationNode,
} from "../../../types";

const props = defineProps<{
  graph?: InvestigationGraph;
  busy?: boolean;
  updatingId?: number;
}>();
const emit = defineEmits<{
  status: [hypothesis: InvestigationHypothesis, status: string];
  approval: [hypothesis: InvestigationHypothesis, approved: boolean];
}>();

const metrics = computed(() => props.graph?.metrics);
const states = computed(() => props.graph?.nodes.filter((item) => item.nodeType === "page_state") || []);
const actions = computed(() => props.graph?.actions || []);
const apis = computed(() => props.graph?.apis || []);
const hypotheses = computed(() => props.graph?.hypotheses || []);
const readyHypotheses = computed(() => hypotheses.value.filter((item) => ["ready", "in_progress"].includes(item.status)));
const showAllHypotheses = ref(false);
const visibleHypotheses = computed(() => showAllHypotheses.value ? hypotheses.value : hypotheses.value.slice(0, 12));
const identities = computed(() => props.graph?.nodes.filter((item) => item.nodeType === "identity") || []);
const stopLabel = computed(() => ({
  confirmed_waf_or_challenge: "确认 WAF / 人机挑战，已立即停止",
  incremental_no_new_value: "与上次基线一致，没有新证据",
  no_high_value_hypothesis: "没有达到门禁的高价值假设",
  no_more_valuable_states: "页面状态和动作已探索完",
  identity_matrix_complete: "多身份差异矩阵已完成",
  evidence_collection_complete: "确定性证据收集完成",
}[metrics.value?.stopReason || ""] || metrics.value?.stopReason || "等待本地调查决策"));

function nodeFor(key: string): InvestigationNode | undefined {
  return props.graph?.nodes.find((item) => item.nodeKey === key);
}
function contractItems(hypothesis: InvestigationHypothesis, key: string): string[] {
  const value = hypothesis.contract?.[key];
  return Array.isArray(value) ? value.map(String) : [];
}
function shortIdentity(value: string) {
  if (value === "anonymous") return "匿名身份";
  const parts = value.split(":");
  return parts[parts.length - 1] || "登录身份";
}
function needsMutationApproval(hypothesis: InvestigationHypothesis) {
  const policy = String(hypothesis.contract?.mutationPolicy || "read_only");
  return !["read_only", "discovery_only_no_account_creation"].includes(policy);
}
function approvalScope(hypothesis: InvestigationHypothesis) {
  const scope = hypothesis.mutationApproval?.scope || {};
  return [scope.method, scope.endpoint].filter(Boolean).join(" ") || "等待具体端点";
}
</script>

<template>
  <div class="investigation-graph-panel">
    <div v-if="busy" class="investigation-empty">正在装载调查图谱…</div>
    <div v-else-if="!graph?.metrics" class="investigation-empty">
      <Network :size="22" />
      <strong>当前 URL 还没有调查图谱</strong>
      <span>旧任务需要在结果同步后重新执行一次前端探测；不会因为空图谱调用模型。</span>
    </div>
    <template v-else>
      <section class="investigation-decision" :class="{ worthy: metrics?.tokenWorthy, stopped: !metrics?.tokenWorthy }">
        <div class="gain-score">
          <span>{{ metrics?.informationGain || 0 }}</span><small>信息增益</small>
        </div>
        <div class="decision-copy">
          <span class="eyebrow"><Sparkles :size="14" />本地决策门禁</span>
          <h3>{{ metrics?.tokenWorthy ? "证据足够：允许一次有界 AI 验证" : "本地停止：不为低价值情报消耗 Token" }}</h3>
          <p>{{ stopLabel }}</p>
        </div>
        <div class="decision-delta">
          <span><b>+{{ metrics?.addedCount || 0 }}</b> 新增</span>
          <span><b>~{{ metrics?.changedCount || 0 }}</b> 变化</span>
          <span><b>-{{ metrics?.removedCount || 0 }}</b> 消失</span>
          <span><b>{{ metrics?.duplicateCount || 0 }}</b> 去重</span>
        </div>
      </section>

      <section class="investigation-kpis">
        <article><Network :size="16" /><span><b>{{ metrics?.stateCount }}</b>页面状态</span></article>
        <article><MousePointerClick :size="16" /><span><b>{{ metrics?.actionCount }}</b>自动动作</span></article>
        <article><Braces :size="16" /><span><b>{{ metrics?.apiCount }}</b>API 模型</span></article>
        <article><ShieldCheck :size="16" /><span><b>{{ metrics?.parameterCount }}</b>参数</span></article>
        <article><Bot :size="16" /><span><b>{{ readyHypotheses.length }}</b>可验证假设</span></article>
        <article><GitCompareArrows :size="16" /><span><b>{{ graph.identityDiffs.length }}</b>身份差异</span></article>
      </section>

      <section class="causal-lane">
        <header><div><strong>页面 → 动作 → 请求 → 假设</strong><small>每个接口都能回溯到触发它的页面状态和自动动作。</small></div><span>{{ graph.edges.length }} 条证据关系</span></header>
        <div class="causal-columns">
          <div class="causal-column">
            <h4>页面状态</h4>
            <article v-for="state in states.slice(0, 8)" :key="state.nodeKey">
              <Network :size="14" /><span><strong>{{ state.label || state.nodeKey }}</strong><small>{{ state.status }} · {{ state.valueScore }} 分</small></span>
            </article>
            <p v-if="states.length > 8">另有 {{ states.length - 8 }} 个已去重状态</p>
          </div>
          <ArrowRight class="causal-arrow" :size="18" />
          <div class="causal-column">
            <h4>结构化动作</h4>
            <article v-for="action in actions.slice(0, 8)" :key="action.actionKey">
              <MousePointerClick :size="14" /><span><strong>{{ action.label || action.actionType }}</strong><small>{{ nodeFor(action.stateKey)?.label || action.stateKey }} · {{ action.outcome || "已执行" }}</small></span>
            </article>
            <p v-if="actions.length > 8">另有 {{ actions.length - 8 }} 个动作</p>
          </div>
          <ArrowRight class="causal-arrow" :size="18" />
          <div class="causal-column api-column">
            <h4>请求 / API</h4>
            <article v-for="api in apis.slice(0, 10)" :key="api.apiKey" :class="`baseline-${api.baselineStatus}`">
              <b>{{ api.method }}</b><span><strong>{{ api.normalizedPath }}</strong><small>{{ api.parameters.length }} 参数 · {{ api.baselineStatus }} · {{ api.source || "静态证据" }}</small></span>
            </article>
            <p v-if="apis.length > 10">另有 {{ apis.length - 10 }} 个接口模型</p>
          </div>
          <ArrowRight class="causal-arrow" :size="18" />
          <div class="causal-column hypothesis-column">
            <h4>安全假设</h4>
            <article v-for="item in hypotheses.slice(0, 8)" :key="item.hypothesisKey" :class="{ ready: item.status === 'ready' }">
              <ShieldCheck :size="14" /><span><strong>{{ item.title }}</strong><small>{{ item.score }} 分 · {{ item.status }} · {{ item.confidence }}</small></span>
            </article>
            <p v-if="hypotheses.length > 8">另有 {{ hypotheses.length - 8 }} 条假设</p>
          </div>
        </div>
      </section>

      <section v-if="identities.length" class="identity-matrix-section">
        <header><div><strong>身份与权限差异</strong><small>401/403 只作为权限边界，不会让整个会话熄灯；只有明确回到登录页才判定失效。</small></div><div class="identity-chips"><span v-for="identity in identities" :key="identity.nodeKey"><i></i>{{ shortIdentity(identity.label) }}</span></div></header>
        <div v-if="graph.identityDiffs.length" class="identity-diff-grid">
          <article v-for="diff in graph.identityDiffs" :key="diff.id">
            <span class="diff-score">{{ diff.riskScore }}</span><div><strong>{{ diff.differenceType }} · {{ diff.apiKey }}</strong><small>{{ shortIdentity(diff.leftIdentityKey) }} ↔ {{ shortIdentity(diff.rightIdentityKey) }} · 仅为待验证权限候选</small></div>
          </article>
        </div>
        <p v-else class="identity-no-diff"><CheckCircle2 :size="15" />当前身份观察中没有形成结构性差异；单身份任务不会伪造对比结论。</p>
      </section>

      <section class="hypothesis-contracts">
        <header><div><strong>验证契约队列</strong><small>Agent 只能按这里的证据要求、尝试上限和停止规则验证，不能开放式探索。</small></div><span>{{ readyHypotheses.length }} 条通过门禁</span></header>
        <div v-if="hypotheses.length" class="contract-grid">
          <article v-for="item in visibleHypotheses" :key="item.id" :class="[`status-${item.status}`, { ready: item.decision?.eligibleForModel }]">
            <div class="contract-title"><span class="contract-score">{{ item.score }}</span><div><strong>{{ item.title }}</strong><small>{{ item.category }} · {{ item.confidence || "未知置信度" }}</small></div></div>
            <p>{{ item.contract?.objective || "等待补全验证目标" }}</p>
            <div class="contract-meta"><span>最多 <b>{{ item.contract?.maxAttempts || 0 }}</b> 次</span><span>{{ item.contract?.mutationPolicy || "read_only" }}</span><span>{{ item.contract?.kind || "bounded" }}</span></div>
            <div v-if="needsMutationApproval(item)" class="mutation-approval" :class="{ active: item.mutationApproval?.active }">
              <div><strong>{{ item.mutationApproval?.active ? "状态变更授权有效" : "默认只捕获请求，不发送状态变更" }}</strong><small>{{ approvalScope(item) }}<template v-if="item.mutationApproval?.active"> · {{ item.mutationApproval.maxAttempts || 1 }} 次 · 到期 {{ item.mutationApproval.expiresAt }}</template></small></div>
              <button v-if="!item.mutationApproval?.active" class="button ghost compact" :disabled="updatingId === item.id" @click="emit('approval', item, true)">授权 1 次 / 30 分钟</button>
              <button v-else class="button ghost compact" :disabled="updatingId === item.id" @click="emit('approval', item, false)">立即撤销</button>
            </div>
            <details><summary>证据要求与停止规则</summary><div><strong>必须取得</strong><ul><li v-for="value in contractItems(item, 'requiredEvidence')" :key="value">{{ value }}</li></ul><strong>立即停止</strong><ul><li v-for="value in contractItems(item, 'stopRules')" :key="value">{{ value }}</li></ul></div></details>
            <div class="contract-actions">
              <button v-if="item.status === 'ready'" class="button primary compact" :disabled="updatingId === item.id" @click="emit('status', item, 'in_progress')">进入验证</button>
              <button v-if="!['validated','rejected','exhausted'].includes(item.status)" class="button ghost compact" :disabled="updatingId === item.id" @click="emit('status', item, 'rejected')">排除</button>
              <span v-if="item.status === 'validated'"><CheckCircle2 :size="14" />已验证</span>
              <span v-else-if="item.status === 'exhausted'"><CircleStop :size="14" />已耗尽</span>
            </div>
          </article>
        </div>
        <div v-else class="investigation-empty compact"><CircleStop :size="18" />没有证据支持的假设，任务应在本地结束。</div>
        <button
          v-if="hypotheses.length > 12"
          class="contract-expand"
          @click="showAllHypotheses = !showAllHypotheses"
        >
          {{ showAllHypotheses ? "收起，只看前 12 条" : `展开其余 ${hypotheses.length - 12} 条契约` }}
        </button>
      </section>
    </template>
  </div>
</template>

<style scoped>
.investigation-graph-panel{--border:var(--line);--panel:var(--app-surface,#fff);--text:var(--app-ink,#172033);--accent:var(--blue);--success:var(--green);--danger:var(--red);--warning:var(--amber);--surface:color-mix(in srgb,var(--app-ink,#172033) 4%,var(--app-surface,#fff));display:grid;gap:16px}.investigation-empty{min-height:220px;border:1px dashed var(--border);border-radius:16px;display:grid;place-content:center;justify-items:center;gap:8px;color:var(--muted);text-align:center;padding:24px}.investigation-empty strong{color:var(--text)}.investigation-empty.compact{min-height:100px}.investigation-decision{display:grid;grid-template-columns:auto 1fr auto;gap:18px;align-items:center;border:1px solid color-mix(in srgb,var(--danger) 30%,var(--border));background:linear-gradient(120deg,color-mix(in srgb,var(--danger) 8%,var(--panel)),var(--panel));border-radius:18px;padding:18px}.investigation-decision.worthy{border-color:color-mix(in srgb,var(--success) 35%,var(--border));background:linear-gradient(120deg,color-mix(in srgb,var(--success) 9%,var(--panel)),var(--panel))}.gain-score{width:86px;height:86px;border-radius:50%;display:grid;place-content:center;text-align:center;border:7px solid color-mix(in srgb,var(--danger) 55%,var(--border));background:var(--panel)}.worthy .gain-score{border-color:color-mix(in srgb,var(--success) 60%,var(--border))}.gain-score span{font-size:28px;font-weight:800;line-height:1}.gain-score small,.decision-copy p,.causal-lane small,.identity-matrix-section small,.hypothesis-contracts small{color:var(--muted)}.decision-copy .eyebrow{display:flex;align-items:center;gap:6px;font-size:12px;color:var(--accent);font-weight:700}.decision-copy h3{margin:5px 0;font-size:18px}.decision-copy p{margin:0}.decision-delta{display:grid;grid-template-columns:repeat(2,minmax(72px,1fr));gap:8px}.decision-delta span{border:1px solid var(--border);border-radius:10px;padding:8px 10px;font-size:12px}.decision-delta b{display:block;font-size:17px;color:var(--text)}.investigation-kpis{display:grid;grid-template-columns:repeat(6,1fr);gap:8px}.investigation-kpis article{display:flex;align-items:center;gap:8px;border:1px solid var(--border);background:var(--panel);border-radius:12px;padding:11px}.investigation-kpis b{font-size:18px;margin-right:4px}.causal-lane,.identity-matrix-section,.hypothesis-contracts{border:1px solid var(--border);background:var(--panel);border-radius:16px;padding:16px}.causal-lane>header,.identity-matrix-section>header,.hypothesis-contracts>header{display:flex;justify-content:space-between;align-items:center;margin-bottom:14px}.causal-lane header div,.identity-matrix-section header div,.hypothesis-contracts header div{display:grid;gap:3px}.causal-columns{display:grid;grid-template-columns:minmax(150px,1fr) auto minmax(160px,1fr) auto minmax(210px,1.25fr) auto minmax(180px,1fr);align-items:start;gap:8px}.causal-arrow{align-self:center;color:var(--muted)}.causal-column{display:grid;gap:7px;min-width:0}.causal-column h4{margin:0 0 3px;font-size:12px;text-transform:uppercase;color:var(--muted)}.causal-column article{display:flex;align-items:flex-start;gap:7px;border:1px solid var(--border);border-radius:10px;padding:8px;background:var(--surface);min-width:0}.causal-column article>span{display:grid;min-width:0}.causal-column article strong{font-size:12px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.causal-column article small{font-size:10px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.causal-column p{margin:2px 0;font-size:11px;color:var(--muted)}.api-column article>b{font-size:10px;color:var(--accent);min-width:35px}.api-column article.baseline-new{border-color:color-mix(in srgb,var(--success) 35%,var(--border))}.api-column article.baseline-changed{border-color:color-mix(in srgb,var(--warning) 40%,var(--border))}.hypothesis-column article.ready{border-color:color-mix(in srgb,var(--accent) 40%,var(--border))}.identity-chips{display:flex!important;flex-direction:row;flex-wrap:wrap;gap:6px}.identity-chips span{display:flex;align-items:center;gap:5px;border:1px solid var(--border);border-radius:999px;padding:5px 9px;font-size:11px}.identity-chips i{width:7px;height:7px;border-radius:50%;background:var(--success)}.identity-diff-grid{display:grid;grid-template-columns:repeat(2,1fr);gap:8px}.identity-diff-grid article{display:flex;gap:10px;align-items:center;border:1px solid color-mix(in srgb,var(--warning) 35%,var(--border));border-radius:12px;padding:10px}.diff-score{width:36px;height:36px;border-radius:50%;display:grid;place-content:center;background:color-mix(in srgb,var(--warning) 15%,transparent);font-weight:800}.identity-diff-grid article div{display:grid;gap:3px}.identity-no-diff{display:flex;gap:7px;align-items:center;color:var(--muted);margin:0}.contract-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.contract-grid>article{border:1px solid var(--border);border-radius:14px;padding:13px;background:var(--surface);display:grid;gap:10px}.contract-grid>article.ready{border-color:color-mix(in srgb,var(--accent) 40%,var(--border))}.contract-title{display:flex;align-items:center;gap:10px}.contract-title>div{display:grid}.contract-score{width:40px;height:40px;border-radius:11px;display:grid;place-content:center;background:color-mix(in srgb,var(--accent) 14%,transparent);font-weight:800}.contract-grid p{margin:0;color:var(--muted);font-size:12px}.contract-meta{display:flex;gap:6px;flex-wrap:wrap}.contract-meta span{border:1px solid var(--border);border-radius:999px;padding:4px 8px;font-size:10px}.mutation-approval{display:flex;align-items:center;justify-content:space-between;gap:10px;border:1px solid color-mix(in srgb,var(--warning) 40%,var(--border));border-radius:10px;padding:9px;background:color-mix(in srgb,var(--warning) 7%,transparent)}.mutation-approval.active{border-color:color-mix(in srgb,var(--success) 45%,var(--border));background:color-mix(in srgb,var(--success) 7%,transparent)}.mutation-approval>div{display:grid;gap:2px;min-width:0}.mutation-approval strong{font-size:11px}.mutation-approval small{font-size:10px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.mutation-approval button{flex:none}.contract-grid details{font-size:11px}.contract-grid details summary{cursor:pointer;color:var(--accent)}.contract-grid ul{margin:4px 0 8px;padding-left:18px}.contract-actions{display:flex;align-items:center;gap:7px}.contract-actions>span{display:flex;align-items:center;gap:5px;color:var(--muted);font-size:12px}@media(max-width:1180px){.investigation-kpis{grid-template-columns:repeat(3,1fr)}.causal-columns{grid-template-columns:1fr 1fr}.causal-arrow{display:none}.contract-grid,.identity-diff-grid{grid-template-columns:1fr}}@media(max-width:720px){.investigation-decision{grid-template-columns:1fr}.investigation-kpis{grid-template-columns:repeat(2,1fr)}.causal-columns{grid-template-columns:1fr}.mutation-approval{align-items:flex-start;flex-direction:column}}
</style>
