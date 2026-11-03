<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { FileText, Play, Plus, Search, Trash2, UploadCloud } from "@lucide/vue";
import { assetApi } from "../api";
import { defaultAssetExecutionProfile } from "../executionSource";
import type { Project } from "../../workspaces/types";
import type { AssetExecutionPlan, AssetExecutionProfile, Target } from "../types";
import { useI18n } from "../uiPorts";

const props = defineProps<{ projects: Project[]; profiles: AssetExecutionProfile[]; selectedProjectId?: number; allowCreateProject?: boolean }>();
const emit = defineEmits<{ started: [runId: number]; projectChange: [id: number]; createProject: []; notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();
const activeProjects = computed(() => props.projects.filter(project => project.status !== "archived"));
const projectId = ref(activeProjects.value.find(project => project.id === props.selectedProjectId)?.id ?? activeProjects.value[0]?.id);
const useLegacySource = ref(false);
const legacyProfileId = ref(defaultAssetExecutionProfile(props.profiles)?.id);
const currentSource = ref<AssetExecutionPlan>();
const sourceError = ref("");
const profileId = computed(() => useLegacySource.value ? legacyProfileId.value : 0);
const targetType = ref("auto"); const input = ref(""); const pipeline = ref("full");
const selectedProfile = computed(() => props.profiles.find(profile => profile.id === profileId.value));
// Match the backend boolean defaults; a legacy non-boolean does not disable probing.
const sourceReady = computed(() => useLegacySource.value ? !!selectedProfile.value : !!currentSource.value);
const probeEnabled = computed(() => useLegacySource.value ? selectedProfile.value?.settings.runProbe !== false : currentSource.value?.runProbe === true);
const refineEnabled = computed(() => useLegacySource.value ? selectedProfile.value?.settings.runRefine !== false || probeEnabled.value : currentSource.value?.runRefine === true);
const executionPlan = computed(() => [tr("采集", "Collect"), tr("候选入库", "Import candidates"),
  ...(refineEnabled.value ? [tr("分层", "Refine")] : []), ...(probeEnabled.value ? [tr("探测", "Probe")] : [])].join(" → "));
const jobName = ref(tr(`资产采集 ${new Date().toLocaleDateString()}`, `Asset collection ${new Date().toLocaleDateString()}`)); const targets = ref<Target[]>([]); const busy = ref(false);
const targetSearch = ref(""); const selectedIds = ref<number[]>([]);
const lines = computed(() => input.value.split(/\r?\n/).map(v => v.trim()).filter(v => v && !v.startsWith("#")));

async function refreshTargets() { targets.value = projectId.value ? await assetApi.listTargets(projectId.value) : []; selectedIds.value = []; }
watch(() => props.selectedProjectId, value => { projectId.value = activeProjects.value.find(project=>project.id===value)?.id ?? activeProjects.value[0]?.id; });
watch(() => props.projects, values => {
  const active=values.filter(project=>project.status!=="archived");
  if (!active.some(project => project.id === projectId.value)) projectId.value = active.find(project=>project.id===props.selectedProjectId)?.id ?? active[0]?.id;
}, { deep: true });
watch(() => props.profiles, values => {
  if (!values.some(profile => profile.id === legacyProfileId.value)) {
    legacyProfileId.value = defaultAssetExecutionProfile(values)?.id;
    useLegacySource.value = false;
  }
}, { deep: true });
watch(projectId, async value => {
  if (value) emit("projectChange", value);
  await refreshTargets();
});
async function refreshSource() {
  sourceError.value = "";
  try { currentSource.value = await assetApi.currentExecutionPlan(); }
  catch (error) { currentSource.value = undefined; sourceError.value = String(error); }
}
onMounted(() => Promise.allSettled([refreshTargets(), refreshSource()]));

async function addTargets() {
  if (!projectId.value || !lines.value.length) return;
  try {
    const count = await assetApi.importTargets(projectId.value, targetType.value, lines.value);
    input.value = ""; await refreshTargets(); emit("notify", "success", tr(`已新增 ${count} 个目标`, `${count} targets added`));
  } catch (error) { emit("notify", "error", String(error)); }
}
async function removeTarget(id: number) { await assetApi.removeTarget(id); await refreshTargets(); }
const filteredTargets = computed(() => {
  const q = targetSearch.value.trim().toLowerCase();
  return q ? targets.value.filter(t => `${t.value} ${t.targetType}`.toLowerCase().includes(q)) : targets.value;
});
const allFilteredSelected = computed(() => filteredTargets.value.length > 0 && filteredTargets.value.every(t => selectedIds.value.includes(t.id)));
function toggleTarget(id: number) { const i = selectedIds.value.indexOf(id); if (i >= 0) selectedIds.value.splice(i, 1); else selectedIds.value.push(id); }
function toggleAllFiltered() {
  if (allFilteredSelected.value) { const ids = new Set(filteredTargets.value.map(t => t.id)); selectedIds.value = selectedIds.value.filter(id => !ids.has(id)); }
  else { const have = new Set(selectedIds.value); for (const t of filteredTargets.value) if (!have.has(t.id)) selectedIds.value.push(t.id); }
}
async function removeSelected() {
  if (!selectedIds.value.length) return;
  const ids = [...selectedIds.value];
  busy.value = true;
  try { const n = await assetApi.removeTargets(ids); selectedIds.value = []; await refreshTargets(); emit("notify", "success", tr(`已删除 ${n} 个目标`, `${n} targets removed`)); }
  catch (error) { emit("notify", "error", String(error)); }
  finally { busy.value = false; }
}
async function loadFile(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]; if (!file) return;
  input.value = await file.text(); (event.target as HTMLInputElement).value = "";
}
const incremental = ref(false);
async function start() {
  if (!projectId.value) { emit("notify", "error", tr("请选择工作空间", "Select a workspace")); return; }
  if ((useLegacySource.value && !profileId.value) || !sourceReady.value) { emit("notify", "error", tr("请先在配置中心保存资产 API、采集与探测设置。", "Save asset API, collection and probing settings in Configuration Center first.")); return; }
  if (lines.value.length) await addTargets();
  if (!targets.value.length) { emit("notify", "error", tr("请先添加查询目标", "Add at least one target")); return; }
  busy.value = true;
  try {
    const runId = await assetApi.startJob(projectId.value, profileId.value ?? 0, jobName.value, pipeline.value, useLegacySource.value ? "legacy-profile" : "current-defaults", undefined, incremental.value);
    emit("started", runId); emit("notify", "success", tr(`任务 #${runId} 已启动`, `Job #${runId} started`));
  } catch (error) { emit("notify", "error", String(error)); }
  finally { busy.value = false; }
}
</script>

<template>
  <div class="query-layout">
    <section class="panel query-builder-panel">
      <div class="panel-heading"><div><h2>{{tr('创建资产查询','Create asset query')}}</h2><p>{{tr('每行一个公司名、域名、IP或CIDR。公司与域名命中的同一入口合并发现证据；是否进行存活探测由资产探测设置决定。来源标签和评分不是归属确认，人工结论和已看记录保持。','One company, domain, IP or CIDR per line. Shared endpoints merge discovery evidence; asset probing settings determine whether to probe. Discovery labels and scores are not ownership confirmation. Manual decisions and opened-entry records are preserved.')}}</p></div></div>
      <div class="form-grid two">
        <div class="project-field-with-action"><label class="field"><span>{{tr('归属工作空间','Workspace')}}</span><select v-model="projectId"><option v-for="p in projects.filter(item=>item.status!=='archived')" :key="p.id" :value="p.id">{{ p.name }}</option></select></label><button v-if="allowCreateProject!==false" class="button ghost" type="button" @click="$emit('createProject')"><Plus :size="14" />{{tr('新建','New')}}</button></div>
        <label class="field"><span>{{tr('任务名称','Job name')}}</span><input v-model="jobName" /></label>
      </div>
      <label class="checkbox-field"><input v-model="incremental" type="checkbox" data-query-incremental />{{tr('仅采新增（增量）：跳过本项目已采集完成的查询，只跑新增种子对应的查询；不勾为全量重采','Incremental: skip queries already completed for this project and only run those for newly added seeds; unchecked re-collects everything')}}</label>
      <div class="query-execution-plan" :data-query-profile="profileId" :data-probe-plan="!sourceReady ? 'unknown' : probeEnabled ? 'enabled' : 'disabled'" role="status">
        <strong>{{useLegacySource ? tr('按历史来源创建新任务','New task from a historical source') : tr('当前执行步骤','Current execution steps')}}</strong>
        <span>{{sourceReady ? executionPlan : tr('请先在配置中心保存资产设置','Save asset settings in Configuration Center first')}}</span>
        <small v-if="sourceReady && !probeEnabled">{{tr('本轮不进行存活探测：入库的是来源候选，不代表当前可访问。','This run does not probe availability. Imported source candidates do not prove current accessibility.')}}</small>
      </div>
      <p v-if="sourceError && !useLegacySource" class="plugin-read-error" role="alert">{{tr('读取当前设置失败：','Current settings could not be read: ')}}{{sourceError}} <button class="button ghost compact" @click="refreshSource">{{tr('重试','Retry')}}</button></p>
      <details class="query-legacy-source" data-query-legacy-source>
        <summary>{{tr('历史来源与兼容流程','Historical source & compatibility')}}</summary>
        <p>{{tr('普通新任务自动读取当前资产 API、采集、探测和内容分类设置。仅在需要兼容旧配置时启用历史来源；保留的历史附加参数可能仍会生效。断点继续仍使用原任务快照。','New tasks automatically read current asset API, collection, probing and content classification settings. Enable a historical source only for compatibility; retained legacy parameters may still apply. Checkpoint resume keeps the original task snapshot.')}}</p>
        <label class="checkbox-field"><input v-model="useLegacySource" type="checkbox" data-query-use-legacy />{{tr('为本次新任务使用历史来源','Use a historical source for this new task')}}</label>
        <label class="field"><span>{{tr('历史配置来源','Historical configuration source')}}</span><select v-model="legacyProfileId" :disabled="!useLegacySource" data-query-legacy-profile><option v-for="p in profiles" :key="p.id" :value="p.id">{{ p.name }}</option></select></label>
        <label class="field"><span>{{tr('兼容流程','Compatibility workflow')}}</span><select v-model="pipeline" data-query-pipeline><option value="full">{{tr('采集与处理（默认）','Collect and process (default)')}}</option><option value="collect">{{tr('兼容采集流程','Compatible collection workflow')}}</option></select></label>
        <small>{{tr('两个流程均以探测设置决定是否进行分层与存活探测。','Both workflows follow probing settings for refinement and availability checks.')}}</small>
      </details>
      <div class="input-toolbar">
        <div class="segmented">
          <button v-for="item in [['auto',tr('自动识别','Auto')],['company',tr('公司','Company')],['domain',tr('域名','Domain')],['ip','IP'],['cidr','CIDR']]" :key="item[0]" :class="{ active: targetType===item[0] }" @click="targetType=item[0]">{{ item[1] }}</button>
        </div>
        <label class="button ghost compact file-button"><UploadCloud :size="15" /> {{tr('读取 TXT','Import TXT')}}<input type="file" accept=".txt,.csv,text/plain" @change="loadFile" /></label>
      </div>
      <textarea v-model="input" class="target-input" rows="9" :placeholder="tr('中国移动通信有限公司\nexample.com\n203.0.113.0/24','Example Corporation Ltd.\nexample.com\n203.0.113.0/24')"></textarea>
      <div class="input-summary"><span><FileText :size="15" /> {{tr(`待添加 ${lines.length} 条`,`${lines.length} pending`)}}</span><span>{{tr('空行与 # 注释自动忽略','Blank lines and # comments are ignored')}}</span></div>
      <div class="query-actions">
        <button class="button secondary" :disabled="!lines.length" @click="addTargets"><Plus :size="16" /> {{tr('添加到项目','Add to project')}}</button>
        <button class="button primary large" :disabled="busy" @click="start"><Play :size="17" /> {{ busy ? tr('启动中…','Starting…') : tr('启动任务','Start job') }}</button>
      </div>
    </section>
    <aside class="panel target-list-panel">
      <div class="panel-heading compact"><div><h3>{{tr('项目目标','Project targets')}}</h3></div><span class="count-pill">{{ targets.length }}</span></div>
      <template v-if="targets.length">
        <label class="target-search"><Search :size="14" /><input v-model="targetSearch" :placeholder="tr('搜索域名 / IP / 公司…','Search domain / IP / company…')" /></label>
        <div class="target-toolbar">
          <label class="target-select-all"><input type="checkbox" :checked="allFilteredSelected" @change="toggleAllFiltered" />{{tr('全选','Select all')}}<span v-if="targetSearch.trim()" class="muted">（{{filteredTargets.length}}）</span></label>
          <button class="button danger compact" :disabled="busy || !selectedIds.length" @click="removeSelected"><Trash2 :size="13" />{{tr(`删除所选 ${selectedIds.length}`,`Delete ${selectedIds.length}`)}}</button>
        </div>
        <div v-if="filteredTargets.length" class="target-list">
          <label v-for="target in filteredTargets" :key="target.id" class="target-row" :class="{ selected: selectedIds.includes(target.id) }">
            <input type="checkbox" :checked="selectedIds.includes(target.id)" @change="toggleTarget(target.id)" />
            <span class="target-type">{{ target.targetType }}</span><span class="target-value">{{ target.value }}</span>
            <button class="icon-button subtle" @click.prevent="removeTarget(target.id)"><Trash2 :size="14" /></button>
          </label>
        </div>
        <div v-else class="empty-state small">{{tr('没有匹配的目标','No matching targets')}}</div>
      </template>
      <div v-else class="empty-state small">{{tr('还没有目标','No targets yet')}}<br />{{tr('从左侧输入或导入 TXT','Enter or import TXT on the left')}}</div>
    </aside>
  </div>
</template>

<style scoped>
.query-execution-plan { margin:12px 20px; display:grid; gap:4px; padding:10px 12px; border:1px solid var(--line); border-radius:var(--radius-md); font-size:var(--text-meta); line-height:1.6; }
.query-execution-plan strong { font-size:var(--text-micro); }.query-execution-plan span,.query-execution-plan small { color:var(--muted); overflow-wrap:anywhere; }
.query-legacy-source { margin:12px 20px; color:var(--muted); font-size:var(--text-meta); line-height:1.6; }
.query-legacy-source summary { cursor:pointer; }.query-legacy-source .field { margin-top:10px; }.query-legacy-source p,.query-legacy-source small { overflow-wrap:anywhere; }
.checkbox-field { display:flex; align-items:flex-start; gap:8px; margin:0 20px 10px; font-size:var(--text-meta); line-height:1.5; color:var(--muted); font-weight:400; }
.checkbox-field input[type="checkbox"] { margin-top:3px; flex:none; }
.target-search { display:flex; align-items:center; gap:6px; margin:8px 14px; padding:5px 9px; border:1px solid var(--line); border-radius:var(--radius-sm); color:var(--muted); }
.target-search input { border:none; outline:none; width:100%; background:transparent; font:inherit; color:var(--ink,inherit); }
.target-toolbar { display:flex; align-items:center; justify-content:space-between; gap:8px; margin:0 14px 8px; font-size:var(--text-micro); }
.target-select-all { display:flex; align-items:center; gap:6px; color:var(--muted); cursor:pointer; }
.target-toolbar .button.compact { display:inline-flex; align-items:center; gap:5px; }
.target-row { display:flex; align-items:center; gap:8px; cursor:pointer; }
.target-row.selected { background:color-mix(in srgb, var(--blue) 8%, transparent); }
.target-row input[type="checkbox"] { flex:none; margin:0; }
.target-row .target-value { flex:1; min-width:0; overflow-wrap:anywhere; }
</style>
