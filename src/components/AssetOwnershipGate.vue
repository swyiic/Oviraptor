<script setup lang="ts">
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import { Check, ChevronLeft, ChevronRight, Link2, RefreshCw, Save, ShieldCheck, ShieldQuestion, X } from "@lucide/vue";
import { api } from "../api";
import type { Asset, AssetOwnershipProfile, AssetOwnershipSummary, AssetQuery, AssetSelection, Project } from "../types";
import "../asset-ownership.css";
import "../asset-ownership-fixes.css";

const props = defineProps<{ projects: Project[]; selectedProjectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const emptySummary = (): AssetOwnershipSummary => ({ all: 0, unreviewed: 0, attributed: 0, confirmed: 0, related: 0, thirdParty: 0, excluded: 0, exposureEligible: 0 });
const emptyProfile = (projectId = 0): AssetOwnershipProfile => ({ projectId, legalName: "", jurisdiction: "", jurisdictions: [], excludedJurisdictions: [], aliases: [], approvedDomains: [], sharedDomains: [], excludedNames: [], excludedDomains: [], notes: "", policyVersion: 1, updatedAt: "" });
const profile = ref<AssetOwnershipProfile>(emptyProfile());
const summary = ref<AssetOwnershipSummary>(emptySummary());
const assets = shallowRef<Asset[]>([]);
const total = ref(0);
const loading = ref(false);
const saving = ref(false);
const assessing = ref(false);
const bulkBusy = ref(false);
const ownershipView = ref("review");
const search = ref("");
const page=ref(1),pageSize=ref(100);const pages=computed(()=>Math.max(1,Math.ceil(total.value/pageSize.value)));
const selected = ref<Map<string, Asset>>(new Map());
const learnDomainRule = ref(false);
const aliasesText = ref("");
const approvedDomainsText = ref("");
const sharedDomainsText=ref("");const jurisdictionsText=ref("");const excludedJurisdictionsText=ref("");
const excludedNamesText = ref("");
const excludedDomainsText = ref("");

const project = computed(() => props.projects.find(item => item.id === props.selectedProjectId));
const selectedRows = computed(() => [...selected.value.values()]);
const selectedAll = computed({
  get: () => assets.value.length > 0 && assets.value.every(asset => selected.value.has(`${asset.projectId}:${asset.id}`)),
  set: (checked: boolean) => {
    const next = new Map(selected.value);
    for (const asset of assets.value) checked ? next.set(`${asset.projectId}:${asset.id}`, asset) : next.delete(`${asset.projectId}:${asset.id}`);
    selected.value = next;
  },
});
const splitLines = (value: string) => value.split(/[\n,，]+/).map(item => item.trim()).filter(Boolean);
function syncProfileText() {
  aliasesText.value = profile.value.aliases.join("\n");
  approvedDomainsText.value = profile.value.approvedDomains.join("\n");
  sharedDomainsText.value=profile.value.sharedDomains.join("\n");jurisdictionsText.value=profile.value.jurisdictions.join("\n");excludedJurisdictionsText.value=profile.value.excludedJurisdictions.join("\n");
  excludedNamesText.value = profile.value.excludedNames.join("\n");
  excludedDomainsText.value = profile.value.excludedDomains.join("\n");
}
function selectionPayload(): AssetSelection[] { return selectedRows.value.map(asset => ({ projectId: asset.projectId, assetId: asset.id })); }
function toggle(asset: Asset, checked: boolean) {
  const next = new Map(selected.value); const key = `${asset.projectId}:${asset.id}`;
  checked ? next.set(key, asset) : next.delete(key); selected.value = next;
}
function ownershipLabel(value: string) {
  return ({ unreviewed: "未判断", attributed: "疑似归属", confirmed: "确认归属", related: "仅关联", third_party: "第三方", excluded: "已排除" } as Record<string,string>)[value] || value;
}
async function load() {
  if (!props.selectedProjectId) { profile.value = emptyProfile(); summary.value = emptySummary(); assets.value = []; total.value = 0; return; }
  loading.value = true;
  try {
    const query: AssetQuery = { projectId: props.selectedProjectId, search: search.value, conditions: [], page: page.value, pageSize: pageSize.value, includeDeleted: false, deletedView: "active", probeView: "browser_review", probeOutcomeView: "all", sentinelView: "all", decisionView: "ownership_scope", ownershipView: ownershipView.value, sortBy: "priority", sortDirection: "desc" };
    const [nextProfile, nextSummary, assetPage] = await Promise.all([api.getAssetOwnershipProfile(props.selectedProjectId), api.getAssetOwnershipSummary(props.selectedProjectId), api.listAssets(query)]);
    profile.value = nextProfile; summary.value = nextSummary; assets.value = assetPage.items; total.value = assetPage.total; selected.value = new Map(); syncProfileText();
  } catch (error) { emit("notify", "error", String(error)); }
  finally { loading.value = false; }
}
async function saveProfile() {
  if (!props.selectedProjectId || saving.value) return;
  saving.value = true;
  try {
    profile.value = await api.saveAssetOwnershipProfile({ ...profile.value, projectId: props.selectedProjectId, jurisdictions:splitLines(jurisdictionsText.value),excludedJurisdictions:splitLines(excludedJurisdictionsText.value), aliases: splitLines(aliasesText.value), approvedDomains: splitLines(approvedDomainsText.value),sharedDomains:splitLines(sharedDomainsText.value), excludedNames: splitLines(excludedNamesText.value), excludedDomains: splitLines(excludedDomainsText.value) });
    syncProfileText(); emit("notify", "success", "归属档案已保存；点击重新评估后应用新规则");
  } catch (error) { emit("notify", "error", String(error)); }
  finally { saving.value = false; }
}
async function assess() {
  if (!props.selectedProjectId || assessing.value) return;
  assessing.value = true;
  try {
    const result = await api.assessAssetOwnership(props.selectedProjectId);
    emit("notify", "success", `已评估 ${result.assessed.toLocaleString()} 条非人工锁定资产；只有人工确认归属的资产可进入暴露面`);
    await load();
  } catch (error) { emit("notify", "error", String(error)); }
  finally { assessing.value = false; }
}
async function decide(status: string) {
  if (!selectedRows.value.length || bulkBusy.value) return;
  bulkBusy.value = true;
  const notes: Record<string,string> = { confirmed: "人工确认：属于当前主体且在授权范围内", related: "人工确认：仅有关联，不属于当前授权资产", third_party: "人工确认：第三方服务，不作为自有资产", excluded: "人工确认：不属于当前主体", unreviewed: "撤销人工归属结论，重新进入待判断" };
  try {
    const changed = await api.updateAssetOwnership(selectionPayload(), status, notes[status] || "人工归属结论", learnDomainRule.value);
    emit("notify", "success", `已更新 ${changed.toLocaleString()} 条归属结论${learnDomainRule.value ? "，并保存域名规则" : ""}`);
    await load();
  } catch (error) { emit("notify", "error", String(error)); }
  finally { bulkBusy.value = false; }
}
function setView(view: string) { ownershipView.value = view;page.value=1; void load(); }
watch(() => props.selectedProjectId, () => {page.value=1;void load()});
onMounted(load);
</script>

<template>
  <div class="ownership-gate">
    <div class="section-toolbar ownership-heading">
      <div><span class="eyebrow">OWNERSHIP GATE</span><h2>资产归属与暴露面准入</h2><p>测绘结果可以保持高召回；只有“确认归属 + 授权允许”的资产才进入暴露面或自动调查。</p></div>
      <div><button class="button ghost" :disabled="!project||assessing" @click="assess"><RefreshCw :size="15" :class="{spinning:assessing}"/>{{assessing?'评估中':'按规则重新评估'}}</button><button class="button primary" :disabled="!project||saving" @click="saveProfile"><Save :size="15"/>保存归属档案</button></div>
    </div>
    <div v-if="!project" class="panel ownership-empty"><ShieldQuestion :size="28"/><strong>请先选择一个工作空间</strong><span>归属规则必须绑定具体公司，不能在“所有项目”范围混用。</span></div>
    <template v-else>
      <section class="panel ownership-profile">
        <header><div><strong>目标公司实体档案</strong><small>名称相似只能产生线索；确认域名提供技术控制证据，排除项优先级最高。</small></div><span>策略 v{{profile.policyVersion}}</span></header>
        <div class="ownership-form-grid">
          <label><span>法人或目标主体</span><input v-model="profile.legalName" placeholder="例如：香港移动通讯有限公司"/></label>
          <label><span>注册地（可留空，不作为自动归属条件）</span><input v-model="profile.jurisdiction" placeholder="例如：中国香港；未知时不要猜测"/></label>
          <label><span>经营或授权地域（可多项）</span><textarea v-model="jurisdictionsText" rows="4" placeholder="中国香港&#10;澳门&#10;跨境骨干网络"></textarea></label>
          <label><span>明确排除地域</span><textarea v-model="excludedJurisdictionsText" rows="4" placeholder="中国内地其他省公司&#10;台湾"></textarea></label>
          <label><span>允许名称与品牌</span><textarea v-model="aliasesText" rows="4" placeholder="CMHK&#10;China Mobile Hong Kong&#10;中國移動香港"></textarea></label>
          <label><span>确认根域名</span><textarea v-model="approvedDomainsText" rows="4" placeholder="cmhk.com&#10;hk.chinamobile.com"></textarea></label>
          <label><span>集团共享域名（只作为弱证据）</span><textarea v-model="sharedDomainsText" rows="4" placeholder="chinamobile.com&#10;集团多主体共用域名"></textarea></label>
          <label><span>排除主体或同名公司</span><textarea v-model="excludedNamesText" rows="4" placeholder="中国移动北京&#10;中国移动广东"></textarea></label>
          <label><span>排除域名</span><textarea v-model="excludedDomainsText" rows="4" placeholder="明确不属于当前范围的根域名"></textarea></label>
        </div>
      </section>

      <section class="ownership-summary">
        <button :class="{active:ownershipView==='review'}" @click="setView('review')"><span>待判断</span><strong>{{(summary.unreviewed+summary.attributed).toLocaleString()}}</strong><small>未判断 + 疑似归属</small></button>
        <button :class="{active:ownershipView==='confirmed'}" @click="setView('confirmed')"><span>确认归属</span><strong>{{summary.confirmed.toLocaleString()}}</strong><small>人工确认主体与授权</small></button>
        <button :class="{active:ownershipView==='related'}" @click="setView('related')"><span>仅关联</span><strong>{{summary.related.toLocaleString()}}</strong><small>集团或业务关联</small></button>
        <button :class="{active:ownershipView==='third_party'}" @click="setView('third_party')"><span>第三方</span><strong>{{summary.thirdParty.toLocaleString()}}</strong><small>供应商与外部依赖</small></button>
        <button :class="{active:ownershipView==='excluded'}" @click="setView('excluded')"><span>已排除</span><strong>{{summary.excluded.toLocaleString()}}</strong><small>不会再次进入准入</small></button>
        <button class="eligible" :class="{active:ownershipView==='eligible'}" @click="setView('eligible')"><span>暴露面可准入</span><strong>{{summary.exposureEligible.toLocaleString()}}</strong><small>唯一允许下游消费</small></button>
      </section>

      <section class="panel ownership-list">
        <header><div><strong>{{ownershipView==='eligible'?'暴露面准入预览':'归属复核队列'}}</strong><small>共 {{total.toLocaleString()}} 条资产中心有效 Web 候选；非 Web、已排除和不适用资产不计入。自动疑似归属不会直接放行。</small></div><label class="ownership-search"><input v-model="search" placeholder="搜索公司、域名、标题…" @keyup.enter="page=1;load()"/><button @click="page=1;load()"><RefreshCw :size="14"/></button></label></header>
        <div v-if="selectedRows.length" class="ownership-bulk">
          <strong>已选 {{selectedRows.length}}</strong>
          <button class="confirm" @click="decide('confirmed')"><Check :size="13"/>确认归属并授权</button>
          <button @click="decide('related')"><Link2 :size="13"/>仅集团/业务关联</button>
          <button @click="decide('third_party')">第三方服务</button>
          <button class="exclude" @click="decide('excluded')"><X :size="13"/>排除</button>
          <label><input v-model="learnDomainRule" type="checkbox"/>同时学习精确域名规则</label>
        </div>
        <div class="ownership-table ownership-table-wrap" tabindex="0" aria-label="资产归属复核列表，可上下滚动">
          <table>
          <colgroup>
            <col class="ownership-col-select"/>
            <col class="ownership-col-asset"/>
            <col class="ownership-col-company"/>
            <col class="ownership-col-status"/>
            <col class="ownership-col-reason"/>
            <col class="ownership-col-admission"/>
          </colgroup>
          <thead><tr><th><input v-model="selectedAll" type="checkbox"/></th><th>资产</th><th>公司 / 标题</th><th>归属状态</th><th>证据结论</th><th>暴露面准入</th></tr></thead>
          <tbody><tr v-for="asset in assets" :key="`${asset.projectId}:${asset.id}`">
            <td><input :checked="selected.has(`${asset.projectId}:${asset.id}`)" type="checkbox" @change="toggle(asset,($event.target as HTMLInputElement).checked)"/></td>
            <td><div class="ownership-cell-stack"><code class="ownership-primary ownership-host" :title="asset.domain||asset.host||asset.link||asset.ip">{{asset.domain||asset.host||asset.link||asset.ip}}</code><span class="ownership-secondary" :title="`${asset.ip||'无 IP'}${asset.port?`:${asset.port}`:''}`">{{asset.ip||'无 IP'}}<template v-if="asset.port"> : {{asset.port}}</template></span></div></td>
            <td><div class="ownership-cell-stack"><strong class="ownership-primary" :title="asset.company||'未提供公司'">{{asset.company||'未提供公司'}}</strong><span class="ownership-secondary ownership-title" :title="asset.title||'无标题'">{{asset.title||'无标题'}}</span></div></td>
            <td><div class="ownership-cell-stack ownership-status-stack"><span class="ownership-badge" :class="`ownership-${asset.ownershipStatus}`">{{ownershipLabel(asset.ownershipStatus)}}</span><span class="ownership-secondary">置信度 {{asset.ownershipConfidence}}% · {{asset.ownershipSource==='manual'?'人工结论':'规则判断'}}</span></div></td>
            <td><div class="ownership-reason">{{asset.ownershipReason||'尚未进行归属评估'}}</div></td>
            <td><span v-if="asset.exposureEligible" class="eligibility yes"><ShieldCheck :size="13"/>允许准入</span><span v-else class="eligibility no">不准入</span></td>
          </tr></tbody>
          </table>
          <div v-if="loading" class="ownership-loading">正在加载归属数据…</div><div v-else-if="!assets.length" class="ownership-loading">当前筛选没有资产</div>
        </div>
        <footer class="ownership-pagination"><span>共 {{total.toLocaleString()}} 条</span><select v-model="pageSize" @change="page=1;load()"><option :value="50">50 / 页</option><option :value="100">100 / 页</option><option :value="200">200 / 页</option></select><button :disabled="page<=1" @click="page--;load()"><ChevronLeft :size="15"/></button><strong>{{page}} / {{pages}}</strong><button :disabled="page>=pages" @click="page++;load()"><ChevronRight :size="15"/></button></footer>
      </section>
    </template>
  </div>
</template>
