<script lang="ts">
// O0：按项目保留未保存的档案草稿。放在模块作用域，切换到其他视图（组件卸载）后在本次会话内仍保留；
// base 是草稿所基于的服务器档案版本，保存时作为 CAS 的 expectedVersion。
interface OwnershipProfileForm {
  legalName: string;
  jurisdiction: string;
  notes: string;
  jurisdictionsText: string;
  excludedJurisdictionsText: string;
  aliasesText: string;
  approvedDomainsText: string;
  sharedDomainsText: string;
  excludedNamesText: string;
  excludedDomainsText: string;
}
const ownershipProfileDrafts = new Map<number, { form: OwnershipProfileForm; base: import("../../../types").AssetOwnershipProfile }>();
</script>

<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { Check, ChevronLeft, ChevronRight, Link2, RefreshCw, Save, ShieldCheck, ShieldQuestion, X } from "@lucide/vue";
import { assetApi } from "../api";
import { assetConfirm, useI18n } from "../uiPorts";
import { ownershipAuthorizationApi } from "../ownershipDirectoryApi";
import type { AssetOwnershipDecisionExpectation } from "../api";
import type { Asset, AssetOwnershipProfile, AssetOwnershipSummary, AssetQuery } from "../types";
import type { Project } from "../../workspaces/types";
import "../../../asset-ownership.css";
import "../../../asset-ownership-fixes.css";
import OwnershipProjectSetup from "./OwnershipProjectSetup.vue";
import OwnershipReviewGroups from "./OwnershipReviewGroups.vue";

const props = defineProps<{ projects: Project[]; selectedProjectId?: number }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();
const emptySummary = (): AssetOwnershipSummary => ({ all: 0, unreviewed: 0, attributed: 0, confirmed: 0, related: 0, thirdParty: 0, excluded: 0, exposureEligible: 0 });
const FORM_KEYS: (keyof OwnershipProfileForm)[] = ["legalName", "jurisdiction", "notes", "jurisdictionsText", "excludedJurisdictionsText", "aliasesText", "approvedDomainsText", "sharedDomainsText", "excludedNamesText", "excludedDomainsText"];
const emptyForm = (): OwnershipProfileForm => ({ legalName: "", jurisdiction: "", notes: "", jurisdictionsText: "", excludedJurisdictionsText: "", aliasesText: "", approvedDomainsText: "", sharedDomainsText: "", excludedNamesText: "", excludedDomainsText: "" });
const formFromProfile = (value: AssetOwnershipProfile): OwnershipProfileForm => ({
  legalName: value.legalName, jurisdiction: value.jurisdiction, notes: value.notes,
  jurisdictionsText: value.jurisdictions.join("\n"), excludedJurisdictionsText: value.excludedJurisdictions.join("\n"),
  aliasesText: value.aliases.join("\n"), approvedDomainsText: value.approvedDomains.join("\n"), sharedDomainsText: value.sharedDomains.join("\n"),
  excludedNamesText: value.excludedNames.join("\n"), excludedDomainsText: value.excludedDomains.join("\n"),
});
const sameForm = (left: OwnershipProfileForm, right: OwnershipProfileForm) => FORM_KEYS.every(key => left[key] === right[key]);
// 尚未保存过的档案（updatedAt 为空）对应 expectedVersion=0；后端据此拒绝覆盖他人已保存的版本。
const expectedVersionOf = (value: AssetOwnershipProfile) => value.updatedAt ? value.policyVersion : 0;

const form = ref<OwnershipProfileForm>(emptyForm());
const baseProfile = ref<AssetOwnershipProfile | null>(null);
const serverProfile = ref<AssetOwnershipProfile | null>(null);
const profileProjectId = ref(0);
const queueProjectId = ref(0);
const profileLoading = ref(false);
const draftRestored = ref(false);
const saveConflict = ref(false);
// O1：档案保存或归属结论写入成功后递增，通知“项目归属设置”卡片刷新档案状态与本项目授权摘要。
const setupRefresh = ref(0);
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
// O3：学习规则的范围默认精确主机；“主机及其子域”必须由人明确选择（后端拒绝共享域名、公共后缀/托管平台根、IP）。
const ruleScope = ref<"exact_host" | "subdomains">("exact_host");
// 项目代次：切换项目、组件卸载时递增；所有异步响应回来后先核对项目与代次，过期结果直接丢弃。
let projectGeneration = 0;
let profileGeneration = 0;
let queueGeneration = 0;

const project = computed(() => props.projects.find(item => item.id === props.selectedProjectId));
const projectLabel = (projectId: number) => props.projects.find(item => item.id === projectId)?.name || `#${projectId}`;
const profileReady = computed(() => !!props.selectedProjectId && profileProjectId.value === props.selectedProjectId && !!baseProfile.value);
const queueReady = computed(() => !!props.selectedProjectId && queueProjectId.value === props.selectedProjectId);
const profileDirty = computed(() => !!baseProfile.value && !sameForm(form.value, formFromProfile(baseProfile.value)));
const serverAhead = computed(() => !!baseProfile.value && !!serverProfile.value && expectedVersionOf(serverProfile.value) !== expectedVersionOf(baseProfile.value));
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
const isCurrent = (projectId: number, generation: number) => generation === projectGeneration && projectId === props.selectedProjectId;
function toggle(asset: Asset, checked: boolean) {
  const next = new Map(selected.value); const key = `${asset.projectId}:${asset.id}`;
  checked ? next.set(key, asset) : next.delete(key); selected.value = next;
}
function ownershipLabel(value: string) {
  return ({ unreviewed: tr("未判断", "Unreviewed"), attributed: tr("疑似归属", "Attributed"), confirmed: tr("确认归属", "Confirmed"), related: tr("仅关联", "Related"), third_party: tr("第三方", "Third party"), excluded: tr("已排除", "Excluded") } as Record<string,string>)[value] || value;
}
function expectationOf(asset: Asset): AssetOwnershipDecisionExpectation {
  return { assetId: asset.id, status: asset.ownershipStatus, source: asset.ownershipSource, confidence: asset.ownershipConfidence, authorizationStatus: asset.authorizationStatus, exposureEligible: asset.exposureEligible, reason: asset.ownershipReason };
}
function profileInput(projectId: number, base: AssetOwnershipProfile, value: OwnershipProfileForm): AssetOwnershipProfile {
  return { ...base, projectId, legalName: value.legalName, jurisdiction: value.jurisdiction, notes: value.notes, jurisdictions: splitLines(value.jurisdictionsText), excludedJurisdictions: splitLines(value.excludedJurisdictionsText), aliases: splitLines(value.aliasesText), approvedDomains: splitLines(value.approvedDomainsText), sharedDomains: splitLines(value.sharedDomainsText), excludedNames: splitLines(value.excludedNamesText), excludedDomains: splitLines(value.excludedDomainsText) };
}
function stashDraft(projectId?: number) {
  if (!projectId || profileProjectId.value !== projectId || !baseProfile.value) return;
  if (profileDirty.value) ownershipProfileDrafts.set(projectId, { form: { ...form.value }, base: { ...baseProfile.value } });
  else ownershipProfileDrafts.delete(projectId);
}
function resetProjectState() {
  projectGeneration++;
  selected.value = new Map(); assets.value = []; total.value = 0; summary.value = emptySummary();
  form.value = emptyForm(); baseProfile.value = null; serverProfile.value = null;
  profileProjectId.value = 0; queueProjectId.value = 0; draftRestored.value = false; saveConflict.value = false;
  page.value = 1; learnDomainRule.value = false; ruleScope.value = "exact_host";
}
// 档案只在进入项目时加载一次；筛选、翻页、评估、批量结论都不会重新填表覆盖草稿。
async function loadProfile() {
  const projectId = props.selectedProjectId; if (!projectId) return;
  const generation = projectGeneration; const request = ++profileGeneration;
  profileLoading.value = true;
  try {
    const latest = await assetApi.getAssetOwnershipProfile(projectId);
    if (!isCurrent(projectId, generation) || request !== profileGeneration) return;
    serverProfile.value = latest;
    const draft = ownershipProfileDrafts.get(projectId);
    if (draft) { ownershipProfileDrafts.delete(projectId); baseProfile.value = draft.base; form.value = { ...draft.form }; draftRestored.value = true; }
    else { baseProfile.value = latest; form.value = formFromProfile(latest); draftRestored.value = false; }
    profileProjectId.value = projectId;
  } catch (error) { if (isCurrent(projectId, generation)) emit("notify", "error", String(error)); }
  finally { if (request === profileGeneration) profileLoading.value = false; }
}
// 只刷新“服务器最新版本”提示，不改动草稿。
async function refreshServerProfile() {
  const projectId = props.selectedProjectId; if (!projectId) return;
  const generation = projectGeneration;
  try {
    const latest = await assetApi.getAssetOwnershipProfile(projectId);
    if (isCurrent(projectId, generation) && profileProjectId.value === projectId) serverProfile.value = latest;
  } catch { /* 提示性刷新失败不影响草稿 */ }
}
async function discardDraft() {
  if (!profileReady.value || !serverProfile.value || !props.selectedProjectId) return;
  const projectId = props.selectedProjectId, generation = projectGeneration, latest = serverProfile.value;
  if (profileDirty.value && !await assetConfirm(tr("放弃当前未保存的草稿，并载入服务器上的最新归属档案？", "Discard the unsaved draft and load the latest ownership profile from the server?"))) return;
  if (!isCurrent(projectId, generation) || !profileReady.value || serverProfile.value !== latest) return;
  baseProfile.value = latest; form.value = formFromProfile(latest);
  draftRestored.value = false; saveConflict.value = false; ownershipProfileDrafts.delete(projectId);
}
// 冲突后的第二种出路：保留草稿内容，明确改为基于服务器最新版本；之后保存会用草稿整体替换该版本。
async function rebaseDraftOnLatest() {
  if (!profileReady.value || !serverProfile.value || !props.selectedProjectId) return;
  const latest = serverProfile.value, projectId = props.selectedProjectId, generation = projectGeneration;
  const latestVersion = latest.updatedAt ? `v${latest.policyVersion}` : tr("尚未保存", "not saved yet");
  if (!await assetConfirm(tr(`保留当前草稿，并改为基于服务器最新档案（${latestVersion}）？\n\n之后点击保存时，草稿会整体替换该版本中其他人或其他窗口的修改。建议先对照最新档案确认没有需要保留的内容。`, `Keep the current draft and rebase it on the latest server profile (${latestVersion})?\n\nWhen you save afterwards, the draft replaces that version entirely, including changes made by others or in other windows. Compare it with the latest profile first to make sure nothing needs to be kept.`))) return;
  if (!isCurrent(projectId, generation) || !profileReady.value || serverProfile.value !== latest) return;
  baseProfile.value = latest; saveConflict.value = false;
}
async function loadQueue() {
  const projectId = props.selectedProjectId; if (!projectId) return;
  const generation = projectGeneration; const request = ++queueGeneration;
  loading.value = true;
  selected.value = new Map();
  try {
    const query: AssetQuery = { projectId, search: search.value, conditions: [], page: page.value, pageSize: pageSize.value, includeDeleted: false, deletedView: "active", probeView: "browser_review", probeOutcomeView: "all", sentinelView: "all", decisionView: "ownership_scope", ownershipView: ownershipView.value, sortBy: "priority", sortDirection: "desc" };
    const [nextSummary, assetPage] = await Promise.all([assetApi.getAssetOwnershipSummary(projectId), assetApi.listAssets(query)]);
    if (!isCurrent(projectId, generation) || request !== queueGeneration) return;
    summary.value = nextSummary; assets.value = assetPage.items; total.value = assetPage.total; selected.value = new Map(); queueProjectId.value = projectId;
  } catch (error) { if (isCurrent(projectId, generation) && request === queueGeneration) emit("notify", "error", String(error)); return false; }
  finally { if (request === queueGeneration) loading.value = false; }
}
// 保存成功后，只把结果交给“仍基于本次提交所用版本”的那份草稿：可能是当前页面，也可能是切走后缓存的草稿
// （包括 A→B→A 切回后页面已从缓存恢复的情况）。基于其他版本的草稿一律不动，避免把旧响应写进新状态。
function adoptSaved(projectId: number, base: AssetOwnershipProfile, submitted: OwnershipProfileForm, saved: AssetOwnershipProfile): "page" | "draft" | "none" {
  const baseVersion = expectedVersionOf(base);
  if (props.selectedProjectId === projectId && profileProjectId.value === projectId && baseProfile.value && expectedVersionOf(baseProfile.value) === baseVersion) {
    const unchanged = sameForm(form.value, submitted);
    serverProfile.value = saved; baseProfile.value = saved; draftRestored.value = false; saveConflict.value = false;
    if (unchanged) form.value = formFromProfile(saved);
    return unchanged ? "page" : "draft";
  }
  const draft = ownershipProfileDrafts.get(projectId);
  if (draft && expectedVersionOf(draft.base) === baseVersion) {
    if (sameForm(draft.form, submitted)) ownershipProfileDrafts.delete(projectId); else draft.base = saved;
  }
  if (props.selectedProjectId === projectId && profileProjectId.value === projectId) serverProfile.value = saved;
  return "none";
}
async function saveProfile() {
  const projectId = props.selectedProjectId;
  if (!projectId || saving.value) return;
  if (!profileReady.value || !baseProfile.value) { emit("notify", "error", tr("归属档案尚未加载完成，未保存", "The ownership profile has not finished loading; nothing was saved")); return; }
  const generation = projectGeneration; const base = baseProfile.value; const submitted = { ...form.value };
  saving.value = true; saveConflict.value = false;
  try {
    const saved = await assetApi.saveAssetOwnershipProfile(profileInput(projectId, base, submitted), expectedVersionOf(base));
    const adopted = adoptSaved(projectId, base, submitted, saved);
    if (isCurrent(projectId, generation)) setupRefresh.value++;
    if (!isCurrent(projectId, generation)) {
      emit("notify", "success", tr(`「${projectLabel(projectId)}」归属档案已保存为 v${saved.policyVersion}`, `Ownership profile for “${projectLabel(projectId)}” saved as v${saved.policyVersion}`));
      return;
    }
    emit("notify", "success", tr(`归属档案已保存为 v${saved.policyVersion}；点击重新评估后应用新规则${adopted === "draft" ? "；保存期间的新改动仍为未保存草稿" : ""}`, `Ownership profile saved as v${saved.policyVersion}. Re-assess to apply the new rules${adopted === "draft" ? "; changes made while saving remain an unsaved draft" : ""}`));
  } catch (error) {
    const text = String(error);
    if (isCurrent(projectId, generation) && text.includes("OWNERSHIP_PROFILE_VERSION_CONFLICT")) { saveConflict.value = true; void refreshServerProfile(); }
    emit("notify", "error", isCurrent(projectId, generation) ? text : `「${projectLabel(projectId)}」${text}`);
  } finally { saving.value = false; }
}
async function assess() {
  const projectId = props.selectedProjectId;
  if (!projectId || assessing.value) return;
  const generation = projectGeneration;
  const unsavedProfile = !serverProfile.value?.updatedAt;
  const draftNote = unsavedProfile
    ? tr("（尚无已保存的归属档案：只按已绑定组织目录的名称与精确域名做线索匹配；项目标题不作为法人名称）", " (no saved ownership profile yet: only names and exact domains from the bound organization directory are matched as leads; the project title is not used as the legal name)")
    : profileDirty.value && serverProfile.value ? tr(`（按已保存档案 v${serverProfile.value.policyVersion} 评估，未包含当前未保存草稿）`, ` (assessed with saved profile v${serverProfile.value.policyVersion}; the current unsaved draft is not included)`) : "";
  assessing.value = true;
  try {
    const result = await assetApi.assessAssetOwnership(projectId);
    emit("notify", "success", tr(`「${projectLabel(projectId)}」已评估 ${result.assessed.toLocaleString()} 条非人工锁定资产${draftNote}；只有人工确认归属的资产可进入暴露面`, `${projectLabel(projectId)}: assessed ${result.assessed.toLocaleString()} assets that are not manually locked${draftNote}. Only assets with manually confirmed ownership can become exposure eligible`));
    if (isCurrent(projectId, generation)) await loadQueue();
  } catch (error) { emit("notify", "error", isCurrent(projectId, generation) ? String(error) : `「${projectLabel(projectId)}」${String(error)}`); }
  finally { assessing.value = false; }
}
const decisionNotes: Record<string,string> = { related: "人工确认：仅有关联，不属于当前授权资产", third_party: "人工确认：第三方服务，不作为自有资产", excluded: "人工确认：不属于当前主体", unreviewed: "撤销人工归属结论，重新进入待判断" }; // i18n-ignore: persisted by the backend as the decision note
function ruleText() {
  if (!learnDomainRule.value) return "";
  return ruleScope.value === "subdomains"
    ? tr("\n\n已选择学习“主机及其子域”规则：之后按规则评估时，这些主机的所有子域都会被标为疑似归属（仍不自动授权或准入）。共享域名、公共后缀/托管平台根和 IP 会被拒绝，整批不写入。", "\n\nA “host and subdomains” rule will be learned: in later rule-based assessments, every subdomain of these hosts is marked as attributed (still not authorized or made eligible automatically). Shared domains, public suffixes / hosting platform roots, and IPs are rejected, and then nothing in the batch is written.")
    : tr("\n\n已选择学习“精确主机”规则：只匹配与这些资产完全相同的主机，不包含子域、其他租户或路径。", "\n\nAn “exact host” rule will be learned: it matches only hosts identical to these assets, not subdomains, other tenants, or paths.");
}
// 逐条修正（相关/第三方/排除/撤销）：不涉及授权。确认归属与授权分别走 confirmOwnership / authorizeOwnership。
async function decide(status: string) {
  if (status === "confirmed") return confirmOwnership(true);
  const projectId = props.selectedProjectId;
  if (!projectId || !selectedRows.value.length || bulkBusy.value || loading.value) return;
  if (!queueReady.value) { emit("notify", "error", tr("复核队列尚未加载当前工作空间，未写入任何结论", "The review queue has not loaded the current workspace yet; no decisions were written")); return; }
  const generation = projectGeneration;
  const rows = selectedRows.value;
  if (rows.some(asset => asset.projectId !== projectId)) { selected.value = new Map(); emit("notify", "error", tr("选择中包含其他工作空间的资产，已清空选择，未写入任何结论", "The selection included assets from another workspace, so it was cleared; no decisions were written")); return; }
  bulkBusy.value = true;
  try {
    const changed = await assetApi.updateAssetOwnership(rows.map(asset => ({ projectId, assetId: asset.id })), status, decisionNotes[status] || "人工归属结论", learnDomainRule.value && status === "excluded", projectId, rows.map(expectationOf)); // i18n-ignore: persisted decision note
    emit("notify", "success", tr(`「${projectLabel(projectId)}」已更新 ${changed.toLocaleString()} 条归属结论${learnDomainRule.value && status === "excluded" ? "，并保存精确主机排除规则" : ""}`, `${projectLabel(projectId)}: updated ${changed.toLocaleString()} ownership decisions${learnDomainRule.value && status === "excluded" ? " and saved exact-host exclusion rules" : ""}`));
    if (isCurrent(projectId, generation)) { setupRefresh.value++; await loadQueue(); }
  } catch (error) {
    emit("notify", "error", isCurrent(projectId, generation) ? String(error) : `「${projectLabel(projectId)}」${String(error)}`);
    if (isCurrent(projectId, generation) && String(error).includes("OWNERSHIP_DECISION_PRECONDITION_FAILED")) await loadQueue();
  } finally { bulkBusy.value = false; }
}
function selectionGuard(): { projectId: number; generation: number; rows: Asset[] } | null {
  const projectId = props.selectedProjectId;
  if (!projectId || !selectedRows.value.length || bulkBusy.value || loading.value) return null;
  if (!queueReady.value) { emit("notify", "error", tr("复核队列尚未加载当前工作空间，未写入任何结论", "The review queue has not loaded the current workspace yet; no decisions were written")); return null; }
  const rows = selectedRows.value;
  if (rows.some(asset => asset.projectId !== projectId)) { selected.value = new Map(); emit("notify", "error", tr("选择中包含其他工作空间的资产，已清空选择，未写入任何结论", "The selection included assets from another workspace, so it was cleared; no decisions were written")); return null; }
  return { projectId, generation: projectGeneration, rows };
}
async function failWrite(projectId: number, generation: number, error: unknown) {
  emit("notify", "error", isCurrent(projectId, generation) ? String(error) : `「${projectLabel(projectId)}」${String(error)}`);
  if (isCurrent(projectId, generation) && /OWNERSHIP_(DECISION_PRECONDITION_FAILED|AUTHORIZATION_REQUIRES_CONFIRMED|RULE_SCOPE_NOT_ALLOWED)/.test(String(error))) await loadQueue();
}
// 第 1 步“确认归属”：只写归属事实，授权与暴露面准入保持各资产现状；归属结论本身不创建任何执行授权。
// withAuthorization=true 为向导：确认框同时列出两步影响，第 2 步用第 1 步写入后的结论版本走既有授权合同。
async function confirmOwnership(withAuthorization = false) {
  const guard = selectionGuard(); if (!guard) return;
  const { projectId, generation, rows } = guard;
  const count = rows.length.toLocaleString();
  const message = withAuthorization
    ? tr(`向导：将对「${projectLabel(projectId)}」的 ${count} 条资产依次执行两步。\n\n第 1 步 确认归属：\n· 归属结论 = 确认归属（人工锁定，重新评估不会覆盖）\n\n第 2 步 授权（既有受控授权合同）：\n· 授权状态 = 允许\n· 暴露面准入 = 是（被动暴露面采集与自动调查可以消费这些资产的 origin）\n\n授权不是主动扫描授权：主动扫描仍按扫描任务自己的授权流程。第 2 步使用第 1 步写入后的结论版本；两步之间任何一条被修改，第 2 步整批拒绝，不授权。${ruleText()}\n\n确定继续吗？`, `Wizard: this runs two steps in order on ${count} assets in “${projectLabel(projectId)}”.\n\nStep 1, confirm ownership:\n· Ownership decision = Confirmed (manually locked; re-assessment will not override it)\n\nStep 2, authorize (existing controlled authorization contract):\n· Authorization status = Allowed\n· Exposure eligibility = Yes (passive exposure collection and automatic investigation may use these assets' origins)\n\nThis is not active-scan authorization: active scans still follow each scan task's own authorization flow. Step 2 uses the decision version written by step 1; if any asset changes between the two steps, step 2 rejects the whole batch and nothing is authorized.${ruleText()}\n\nContinue?`)
    : tr(`将对「${projectLabel(projectId)}」的 ${count} 条资产执行“确认归属”。\n\n只写入归属事实：\n· 归属结论 = 确认归属（人工锁定，重新评估不会覆盖）\n· 授权状态、暴露面准入 = 保持各资产现状（不授权）\n\n确认归属本身不创建任何执行授权；需要授权时请再点“授权”。${ruleText()}\n\n确定继续吗？`, `This confirms ownership of ${count} assets in “${projectLabel(projectId)}”.\n\nOnly the ownership fact is written:\n· Ownership decision = Confirmed (manually locked; re-assessment will not override it)\n· Authorization status, exposure eligibility = unchanged for each asset (not authorized)\n\nConfirming ownership does not create any execution authorization; click “Authorize” afterwards if you need it.${ruleText()}\n\nContinue?`);
  if (!await assetConfirm(message)) return;
  if (!isCurrent(projectId, generation) || !queueReady.value) { emit("notify", "error", tr("确认期间工作空间或复核队列已变化，未写入任何结论", "The workspace or review queue changed during confirmation; no decisions were written")); return; }
  bulkBusy.value = true;
  const selections = rows.map(asset => ({ projectId, assetId: asset.id }));
  try {
    const result = await ownershipAuthorizationApi.confirmAssetOwnership({ selections, note: "", domainRuleScope: learnDomainRule.value ? ruleScope.value : "none" }, projectId, rows.map(expectationOf));
    const learned = result.learnedRules.length ? tr(`；已学习规则 ${result.learnedRules.join("、")}`, `; learned rules: ${result.learnedRules.join(", ")}`) : "";
    emit("notify", "success", tr(`「${projectLabel(projectId)}」已确认归属 ${result.changed.toLocaleString()} 条（未授权，授权与准入保持原值）${learned}`, `${projectLabel(projectId)}: confirmed ownership of ${result.changed.toLocaleString()} assets (not authorized; authorization and eligibility unchanged)${learned}`));
    if (withAuthorization) {
      if (!isCurrent(projectId, generation)) { emit("notify", "error", tr(`「${projectLabel(projectId)}」已切换工作空间：第 2 步授权未执行`, `${projectLabel(projectId)}: the workspace was switched, so step 2 (authorization) did not run`)); return; }
      const authorized = await ownershipAuthorizationApi.authorizeAssetOwnership({ selections, note: "" }, projectId, result.expectations);
      emit("notify", "success", tr(`「${projectLabel(projectId)}」已授权 ${authorized.toLocaleString()} 条：授权 = 允许，暴露面准入 = 是`, `${projectLabel(projectId)}: authorized ${authorized.toLocaleString()} assets (authorization = Allowed, exposure eligibility = Yes)`));
    }
    if (isCurrent(projectId, generation)) { setupRefresh.value++; await loadQueue(); }
  } catch (error) { await failWrite(projectId, generation, error); }
  finally { bulkBusy.value = false; }
}
// 第 2 步“授权”：只接受已人工确认归属的资产，走既有受控授权合同（逐行 CAS）。
async function authorizeOwnership() {
  const guard = selectionGuard(); if (!guard) return;
  const { projectId, generation, rows } = guard;
  const unconfirmed = rows.filter(asset => asset.ownershipStatus !== "confirmed" || asset.ownershipSource !== "manual").length;
  if (unconfirmed) { emit("notify", "error", tr(`${unconfirmed} 条资产尚未人工确认归属，未授权；请先“确认归属”，或使用“确认归属后授权”向导`, `${unconfirmed} assets do not have manually confirmed ownership yet, so nothing was authorized. Use “Confirm ownership” first, or the “Confirm, then authorize” wizard`)); return; }
  if (!await assetConfirm(tr(`将对「${projectLabel(projectId)}」的 ${rows.length.toLocaleString()} 条已确认归属资产执行“授权”。\n\n· 授权状态 = 允许\n· 暴露面准入 = 是（被动暴露面采集与自动调查可以消费这些资产的 origin；批准某个主机不覆盖它的兄弟子域）\n\n这不是主动扫描授权：主动扫描仍按扫描任务自己的授权流程。\n\n确定继续吗？`, `This authorizes ${rows.length.toLocaleString()} assets with confirmed ownership in “${projectLabel(projectId)}”.\n\n· Authorization status = Allowed\n· Exposure eligibility = Yes (passive exposure collection and automatic investigation may use these assets' origins; approving one host does not cover its sibling subdomains)\n\nThis is not active-scan authorization: active scans still follow each scan task's own authorization flow.\n\nContinue?`))) return;
  if (!isCurrent(projectId, generation) || !queueReady.value) { emit("notify", "error", tr("确认期间工作空间或复核队列已变化，未授权", "The workspace or review queue changed during confirmation; nothing was authorized")); return; }
  bulkBusy.value = true;
  try {
    const authorized = await ownershipAuthorizationApi.authorizeAssetOwnership({ selections: rows.map(asset => ({ projectId, assetId: asset.id })), note: "" }, projectId, rows.map(expectationOf));
    emit("notify", "success", tr(`「${projectLabel(projectId)}」已授权 ${authorized.toLocaleString()} 条：授权 = 允许，暴露面准入 = 是`, `${projectLabel(projectId)}: authorized ${authorized.toLocaleString()} assets (authorization = Allowed, exposure eligibility = Yes)`));
    if (isCurrent(projectId, generation)) { setupRefresh.value++; await loadQueue(); }
  } catch (error) { await failWrite(projectId, generation, error); }
  finally { bulkBusy.value = false; }
}
// O1：目录/绑定保存后，只有“尚未保存档案且表单没有草稿改动”时才用目录预填刷新表单；否则只刷新服务器版本提示。
async function onDirectorySaved(projectId: number) {
  const generation = projectGeneration;
  if (!isCurrent(projectId, generation) || profileProjectId.value !== projectId) return;
  try {
    const latest = await assetApi.getAssetOwnershipProfile(projectId);
    if (!isCurrent(projectId, generation) || profileProjectId.value !== projectId) return;
    serverProfile.value = latest;
    if (!latest.updatedAt && baseProfile.value && !baseProfile.value.updatedAt && !profileDirty.value) { baseProfile.value = latest; form.value = formFromProfile(latest); }
  } catch { /* 提示性刷新失败不影响草稿 */ }
}
// O2：组批量操作写入后刷新项目设置摘要与资产明细；组队列自身已重新读取快照。
function onGroupApplied(projectId: number) {
  if (projectId !== props.selectedProjectId) return;
  setupRefresh.value++; void loadQueue();
}
function setView(view: string) { ownershipView.value = view;page.value=1; void loadQueue(); }
watch(() => props.selectedProjectId, (next, previous) => {
  stashDraft(previous);
  resetProjectState();
  if (next) { void loadProfile(); void loadQueue(); }
});
onMounted(() => { void loadProfile(); void loadQueue(); });
onBeforeUnmount(() => { stashDraft(props.selectedProjectId); projectGeneration++; });
</script>

<template>
  <div class="ownership-gate">
    <div class="section-toolbar ownership-heading">
      <div><p>{{tr('测绘结果可以保持高召回；只有“确认归属 + 授权允许”的资产才进入暴露面或自动调查。','Mapping results can stay high-recall; only assets with “ownership confirmed + authorization allowed” enter exposure or automatic investigation.')}}</p></div>
      <div><button class="button ghost" :disabled="!project||assessing" @click="assess"><RefreshCw :size="15" :class="{spinning:assessing}"/>{{assessing?tr('评估中','Assessing…'):tr('按规则重新评估','Re-assess by rules')}}</button><button class="button primary" :disabled="!project||saving||!profileReady" @click="saveProfile"><Save :size="15"/>{{saving?tr('保存中','Saving…'):tr('保存归属档案','Save ownership profile')}}</button></div>
    </div>
    <div v-if="!project" class="panel ownership-empty"><ShieldQuestion :size="28"/><strong>{{tr('请先选择一个工作空间','Select a workspace first')}}</strong><span>{{tr('归属规则必须绑定具体公司，不能在“所有项目”范围混用。','Ownership rules must be bound to a specific company and cannot be mixed across “All projects”.')}}</span></div>
    <template v-else>
      <OwnershipProjectSetup :project-id="project.id" :project-name="project.name" :refresh-token="setupRefresh" @notify="(type, text) => emit('notify', type, text)" @directory-saved="onDirectorySaved"/>
      <details class="ownership-advanced"><summary>{{tr('高级：主体档案、别名、域名、排除项','Advanced: entity profile, aliases, domains, exclusions')}}<template v-if="profileDirty"> · {{tr('有未保存草稿','Unsaved draft')}}</template><template v-if="saveConflict||serverAhead"> · {{tr('保存冲突待处理','Save conflict to resolve')}}</template></summary>
      <section class="panel ownership-profile">
        <header><div><strong>{{tr('目标公司实体档案','Target company entity profile')}}</strong><small>{{tr('名称相似只能产生线索；确认域名提供技术控制证据，排除项优先级最高。','Similar names only produce leads; confirmed domains provide evidence of technical control, and exclusions take top priority.')}}</small></div><span v-if="!profileReady">{{profileLoading?tr('档案加载中…','Loading profile…'):tr('档案未加载','Profile not loaded')}}</span><span v-else><template v-if="baseProfile?.updatedAt">{{tr(`策略 v${baseProfile?.policyVersion}`,`Policy v${baseProfile?.policyVersion}`)}}</template><template v-else>{{tr('尚未保存（主体名称来自已确认组织目录，未绑定时为空；项目标题不作为法人名称）','Not saved yet (the entity name comes from the confirmed organization directory and is empty when none is bound; the project title is not used as the legal name)')}}</template><template v-if="profileDirty"> · {{tr('有未保存草稿','Unsaved draft')}}</template><template v-if="draftRestored"> · {{tr('已恢复切换前的草稿','Restored the draft from before switching')}}</template></span></header>
        <div v-if="profileReady&&(saveConflict||serverAhead)" class="ownership-bulk"><strong>{{tr(`服务器档案已更新为 ${serverProfile?.updatedAt?'v'+serverProfile?.policyVersion:'未保存'}，当前草稿基于 ${baseProfile?.updatedAt?'v'+baseProfile?.policyVersion:'未保存的预填'}；直接保存会被拒绝，草稿不会被覆盖。`,`The server profile is now ${serverProfile?.updatedAt?'v'+serverProfile?.policyVersion:'unsaved'}, but the current draft is based on ${baseProfile?.updatedAt?'v'+baseProfile?.policyVersion:'the unsaved prefill'}. Saving directly will be rejected; the draft will not be overwritten.`)}}</strong><button @click="discardDraft">{{tr('放弃草稿并载入最新','Discard draft and load latest')}}</button><button @click="rebaseDraftOnLatest">{{tr('保留草稿，改为基于最新版本','Keep draft and rebase on latest')}}</button></div>
        <div class="ownership-form-grid">
          <label><span>{{tr('法人或目标主体','Legal entity or target entity')}}</span><input v-model="form.legalName" :disabled="!profileReady" :placeholder="tr('例如：香港移动通讯有限公司','e.g. China Mobile Hong Kong Company Limited')"/></label>
          <label><span>{{tr('注册地（可留空，不作为自动归属条件）','Place of registration (optional; not used for automatic ownership)')}}</span><input v-model="form.jurisdiction" :disabled="!profileReady" :placeholder="tr('例如：中国香港；未知时不要猜测','e.g. Hong Kong, China; do not guess if unknown')"/></label>
          <label><span>{{tr('经营或授权地域（可多项）','Operating or authorized regions (multiple allowed)')}}</span><textarea v-model="form.jurisdictionsText" :disabled="!profileReady" rows="4" :placeholder="tr('中国香港\n澳门\n跨境骨干网络','Hong Kong, China\nMacau\nCross-border backbone network')"></textarea></label>
          <label><span>{{tr('明确排除地域','Explicitly excluded regions')}}</span><textarea v-model="form.excludedJurisdictionsText" :disabled="!profileReady" rows="4" :placeholder="tr('中国内地其他省公司\n台湾','Other mainland provincial companies\nTaiwan')"></textarea></label>
          <label><span>{{tr('允许名称与品牌','Allowed names and brands')}}</span><textarea v-model="form.aliasesText" :disabled="!profileReady" rows="4" :placeholder="tr('CMHK\nChina Mobile Hong Kong\n中國移動香港','CMHK\nChina Mobile Hong Kong\nChina Mobile HK')"></textarea></label>
          <label><span>{{tr('确认根域名','Confirmed root domains')}}</span><textarea v-model="form.approvedDomainsText" :disabled="!profileReady" rows="4" placeholder="cmhk.com&#10;hk.chinamobile.com"></textarea></label>
          <label><span>{{tr('集团共享域名（只作为弱证据）','Group shared domains (weak evidence only)')}}</span><textarea v-model="form.sharedDomainsText" :disabled="!profileReady" rows="4" :placeholder="tr('chinamobile.com\n集团多主体共用域名','chinamobile.com\nDomains shared by several group entities')"></textarea></label>
          <label><span>{{tr('排除主体或同名公司','Excluded entities or same-name companies')}}</span><textarea v-model="form.excludedNamesText" :disabled="!profileReady" rows="4" :placeholder="tr('中国移动北京\n中国移动广东','China Mobile Beijing\nChina Mobile Guangdong')"></textarea></label>
          <label><span>{{tr('排除域名','Excluded domains')}}</span><textarea v-model="form.excludedDomainsText" :disabled="!profileReady" rows="4" :placeholder="tr('明确不属于当前范围的根域名','Root domains clearly outside the current scope')"></textarea></label>
        </div>
      </section>
      </details>

      <section class="ownership-summary">
        <button :class="{active:ownershipView==='review'}" @click="setView('review')"><span>{{tr('待判断','To review')}}</span><strong>{{(summary.unreviewed+summary.attributed).toLocaleString()}}</strong><small>{{tr('未判断 + 疑似归属','Unreviewed + attributed')}}</small></button>
        <button :class="{active:ownershipView==='confirmed'}" @click="setView('confirmed')"><span>{{tr('确认归属','Confirmed')}}</span><strong>{{summary.confirmed.toLocaleString()}}</strong><small>{{tr('人工确认归属（组确认不含授权）','Ownership confirmed manually (group confirmation excludes authorization)')}}</small></button>
        <button :class="{active:ownershipView==='related'}" @click="setView('related')"><span>{{tr('仅关联','Related')}}</span><strong>{{summary.related.toLocaleString()}}</strong><small>{{tr('集团或业务关联','Group or business relation')}}</small></button>
        <button :class="{active:ownershipView==='third_party'}" @click="setView('third_party')"><span>{{tr('第三方','Third party')}}</span><strong>{{summary.thirdParty.toLocaleString()}}</strong><small>{{tr('供应商与外部依赖','Vendors and external dependencies')}}</small></button>
        <button :class="{active:ownershipView==='excluded'}" @click="setView('excluded')"><span>{{tr('已排除','Excluded')}}</span><strong>{{summary.excluded.toLocaleString()}}</strong><small>{{tr('不会再次进入准入','Will not become eligible again')}}</small></button>
        <button class="eligible" :class="{active:ownershipView==='eligible'}" @click="setView('eligible')"><span>{{tr('暴露面可准入','Exposure eligible')}}</span><strong>{{summary.exposureEligible.toLocaleString()}}</strong><small>{{tr('唯一允许下游消费','The only set downstream may use')}}</small></button>
      </section>

      <OwnershipReviewGroups :project-id="project.id" :refresh-token="setupRefresh" @notify="(type, text) => emit('notify', type, text)" @applied="onGroupApplied"/>

      <section class="panel ownership-list">
        <header><div><strong>{{ownershipView==='eligible'?tr('暴露面准入预览','Exposure eligibility preview'):tr('资产明细（原 URL 列表：组内详情与逐条修正）','Asset details (formerly the URL list: group details and per-item corrections)')}}</strong><small>{{tr(`共 ${total.toLocaleString()} 条资产中心有效 Web 候选；非 Web、已排除和不适用资产不计入。自动疑似归属不会直接放行。`,`${total.toLocaleString()} valid web candidates in the asset center; non-web, excluded, and not-applicable assets are not counted. Automatic attribution never lets an asset through on its own.`)}}</small></div><label class="ownership-search"><input v-model="search" :placeholder="tr('搜索公司、域名、标题…','Search company, domain, title…')" @keyup.enter="page=1;loadQueue()"/><button v-refresh-feedback="{scope:selectedProjectId,busy:loading}" :aria-label="tr('刷新归属资产','Refresh ownership assets')" @click="() => {page=1;return loadQueue()}"><RefreshCw :size="14"/></button></label></header>
        <div v-if="selectedRows.length" class="ownership-bulk">
          <strong>{{tr(`已选 ${selectedRows.length}`,`${selectedRows.length} selected`)}}</strong>
          <button class="confirm" :disabled="bulkBusy||loading" :title="tr('只写入：确认归属（人工锁定）；授权与暴露面准入保持现状','Writes only: ownership confirmed (manually locked); authorization and exposure eligibility stay as they are')" @click="confirmOwnership(false)"><Check :size="13"/>{{tr('确认归属','Confirm ownership')}}</button>
          <button class="confirm" :disabled="bulkBusy||loading" :title="tr('只对已人工确认归属的资产：授权=允许 + 暴露面准入=是（不是主动扫描授权）','Only for assets with manually confirmed ownership: authorization = Allowed + exposure eligibility = Yes (not active-scan authorization)')" @click="authorizeOwnership"><ShieldCheck :size="13"/>{{tr('授权','Authorize')}}</button>
          <button :disabled="bulkBusy||loading" :title="tr('向导：先确认归属，再授权；确认框列出两步各自的影响','Wizard: confirm ownership, then authorize; the confirmation lists the effect of each step')" @click="confirmOwnership(true)">{{tr('确认归属后授权（向导）','Confirm, then authorize (wizard)')}}</button>
          <button :disabled="bulkBusy||loading" @click="decide('related')"><Link2 :size="13"/>{{tr('仅集团/业务关联','Group / business related only')}}</button>
          <button :disabled="bulkBusy||loading" @click="decide('third_party')">{{tr('第三方服务','Third-party service')}}</button>
          <button class="exclude" :disabled="bulkBusy||loading" @click="decide('excluded')"><X :size="13"/>{{tr('排除','Exclude')}}</button>
          <button :disabled="bulkBusy||loading" :title="tr('撤销所选人工归属结论和暴露面授权，恢复自动判断；保留资产及采集历史','Revoke the selected manual ownership decisions and exposure authorization and return to automatic assessment; assets and collection history are kept')" @click="decide('unreviewed')">{{tr('撤销人工结论','Revoke manual decision')}}</button>
          <label><input v-model="learnDomainRule" type="checkbox"/>{{tr('同时学习域名规则','Also learn a domain rule')}}</label>
          <select v-if="learnDomainRule" v-model="ruleScope" :aria-label="tr('规则范围','Rule scope')"><option value="exact_host">{{tr('精确主机（默认）','Exact host (default)')}}</option><option value="subdomains">{{tr('主机及其子域（需明确选择）','Host and subdomains (explicit choice)')}}</option></select>
          <small>{{tr('“确认归属”只记录归属事实，不授权；“授权”只对已人工确认归属的资产授予暴露面准入，不是主动扫描授权。路径级范围不可用（下游授权以 origin 为粒度）。','“Confirm ownership” only records the ownership fact and does not authorize. “Authorize” grants exposure eligibility only to assets with manually confirmed ownership and is not active-scan authorization. Path-level scope is unavailable (downstream authorization works per origin).')}}</small>
        </div>
        <div class="ownership-table ownership-table-wrap" tabindex="0" :aria-label="tr('资产归属复核列表，可上下滚动','Asset ownership review list, scrollable')">
          <table>
          <colgroup>
            <col class="ownership-col-select"/>
            <col class="ownership-col-asset"/>
            <col class="ownership-col-company"/>
            <col class="ownership-col-status"/>
            <col class="ownership-col-reason"/>
            <col class="ownership-col-admission"/>
          </colgroup>
          <thead><tr><th><input v-model="selectedAll" type="checkbox"/></th><th>{{tr('资产','Asset')}}</th><th>{{tr('公司 / 标题','Company / title')}}</th><th>{{tr('归属状态','Ownership status')}}</th><th>{{tr('证据结论','Evidence conclusion')}}</th><th>{{tr('暴露面准入','Exposure eligibility')}}</th></tr></thead>
          <tbody><tr v-for="asset in assets" :key="`${asset.projectId}:${asset.id}`">
            <td><input :checked="selected.has(`${asset.projectId}:${asset.id}`)" type="checkbox" @change="toggle(asset,($event.target as HTMLInputElement).checked)"/></td>
            <td><div class="ownership-cell-stack"><code class="ownership-primary ownership-host" :title="asset.link||asset.host||asset.domain||asset.ip">{{asset.link||asset.host||asset.domain||asset.ip}}</code><span class="ownership-secondary" :title="`${asset.ip||tr('无 IP','No IP')}${asset.port?`:${asset.port}`:''}`">{{asset.ip||tr('无 IP','No IP')}}<template v-if="asset.port"> : {{asset.port}}</template></span></div></td>
            <td><div class="ownership-cell-stack"><strong class="ownership-primary" :title="asset.company||tr('未提供公司','No company provided')">{{asset.company||tr('未提供公司','No company provided')}}</strong><span class="ownership-secondary ownership-title" :title="asset.title||tr('无标题','No title')">{{asset.title||tr('无标题','No title')}}</span></div></td>
            <td><div class="ownership-cell-stack ownership-status-stack"><span class="ownership-badge" :class="`ownership-${asset.ownershipStatus}`">{{ownershipLabel(asset.ownershipStatus)}}</span><span class="ownership-secondary">{{tr(`置信度 ${asset.ownershipConfidence}%`,`Confidence ${asset.ownershipConfidence}%`)}} · {{asset.ownershipSource==='manual'?tr('人工结论','Manual decision'):tr('规则判断','Rule-based')}}</span></div></td>
            <td><div class="ownership-reason">{{asset.ownershipReason||tr('尚未进行归属评估','Ownership not assessed yet')}}</div></td>
            <td><span v-if="asset.exposureEligible" class="eligibility yes"><ShieldCheck :size="13"/>{{tr('允许准入','Eligible')}}</span><span v-else class="eligibility no">{{tr('不准入','Not eligible')}}</span></td>
          </tr></tbody>
          </table>
          <div v-if="loading" class="ownership-loading">{{tr('正在加载归属数据…','Loading ownership data…')}}</div><div v-else-if="!assets.length" class="ownership-loading">{{tr('当前筛选没有资产','No assets match the current filter')}}</div>
        </div>
        <footer class="ownership-pagination"><span>{{tr(`共 ${total.toLocaleString()} 条`,`${total.toLocaleString()} total`)}}</span><select v-model="pageSize" @change="page=1;loadQueue()"><option :value="50">{{tr('50 / 页','50 / page')}}</option><option :value="100">{{tr('100 / 页','100 / page')}}</option><option :value="200">{{tr('200 / 页','200 / page')}}</option></select><button :disabled="page<=1||loading" @click="page--;loadQueue()"><ChevronLeft :size="15"/></button><strong>{{page}} / {{pages}}</strong><button :disabled="page>=pages||loading" @click="page++;loadQueue()"><ChevronRight :size="15"/></button></footer>
      </section>
    </template>
  </div>
</template>
