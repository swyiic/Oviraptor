<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ExternalLink, RefreshCw, Search, Star, UploadCloud, X } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { hackerOneApi } from "../api";
import type { ConfigProfile } from "../../configuration/types";
import { hackerOneProfileOptions, hasHackerOneCredentials, initialHackerOneProfileId } from "../../configuration/profileSelection";
import type { HackerOneDetail, HackerOneEvent, HackerOneProgram } from "../types";
import type { Project } from "../../workspaces/types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ projects: Project[]; profiles: ConfigProfile[] }>();
const emit = defineEmits<{ notify: [type: "success" | "error" | "info", text: string]; configure: [profileId?: number] }>();
const { tr } = useI18n();
const programs = ref<HackerOneProgram[]>([]), detail = ref<HackerOneDetail>(), changes = ref<HackerOneEvent[]>([]);
const search = ref(""), busy = ref(false), readError = ref(""), selectedProject = ref<number>();
const selectedProfileId = ref(initialHackerOneProfileId(props.profiles));
const accountOptions = computed(() => hackerOneProfileOptions(props.profiles));
const selectedProfile = computed(() => props.profiles.find(profile => profile.id === selectedProfileId.value));
const accountConfigured = computed(() => hasHackerOneCredentials(selectedProfile.value));
let disposed = false, accountGeneration = 0, listGeneration = 0, detailGeneration = 0;

watch(accountOptions, options => {
  if (!options.some(option => option.id === selectedProfileId.value)) selectedProfileId.value = initialHackerOneProfileId(props.profiles);
}, { flush: "sync" });
// Changing a chosen account or its credentials invalidates pending UI results, never sends a request.
watch(() => [selectedProfileId.value, selectedProfile.value?.settings.hackerOneUsername, selectedProfile.value?.settings.hackerOneToken, selectedProfile.value?.settings.proxyUrl], () => {
  accountGeneration++; detailGeneration++; detail.value = undefined;
}, { flush: "sync" });

function configure() { emit("configure", selectedProfileId.value); }
async function load() {
  const request = ++listGeneration;
  readError.value = "";
  try {
    const [items, events] = await Promise.all([hackerOneApi.listHackerOnePrograms(search.value), hackerOneApi.listHackerOneEvents(50)]);
    if (disposed || request !== listGeneration) return;
    programs.value = items; changes.value = events;
  } catch (error) { if (!disposed && request === listGeneration) readError.value = String(error); }
}
async function sync(handle?: string) {
  if (busy.value) return;
  const profile = selectedProfile.value, generation = accountGeneration;
  if (!hasHackerOneCredentials(profile)) { emit("notify", "error", tr("请先为所选方案配置 HackerOne API identifier 和 token。", "Configure the HackerOne API identifier and token for the selected profile first.")); return; }
  // Capture the same explicitly selected profile for every official API synchronization.
  const profileId = profile!.id, shownHandle = detail.value?.program.handle;
  busy.value = true;
  const current = () => !disposed && generation === accountGeneration && selectedProfileId.value === profileId;
  try {
    await hackerOneApi.syncHackerOne(profileId, handle);
    if (!current()) return;
    await load();
    if (!current()) return;
    if (handle && shownHandle === handle && detail.value?.program.handle === handle) {
      const request = ++detailGeneration, value = await hackerOneApi.getHackerOneDetail(handle);
      if (current() && request === detailGeneration) detail.value = value;
    }
    if (current()) emit("notify", "success", tr("HackerOne 信息已更新", "HackerOne data updated"));
  } catch (error) { if (current()) emit("notify", "error", String(error)); }
  finally { if (!disposed) busy.value = false; }
}
async function show(program: HackerOneProgram) {
  if (busy.value) return;
  const request = ++detailGeneration;
  busy.value = true;
  try {
    // Details are cached reads. Network refresh always requires its explicit button and selected account.
    const value = await hackerOneApi.getHackerOneDetail(program.handle);
    if (disposed || request !== detailGeneration) return;
    detail.value = value; selectedProject.value = props.projects.find(project => project.status === "active")?.id;
  } catch (error) { if (!disposed && request === detailGeneration) emit("notify", "error", String(error)); }
  finally { if (!disposed) busy.value = false; }
}
function closeDetail() { detailGeneration++; detail.value = undefined; }
async function bookmark(program: HackerOneProgram) {
  try { await hackerOneApi.setHackerOneBookmark(program.handle, !program.bookmarked); if (!disposed) program.bookmarked = !program.bookmarked; }
  catch (error) { if (!disposed) emit("notify", "error", String(error)); }
}
async function send() {
  if (!detail.value || !selectedProject.value) return;
  const handle = detail.value.program.handle, projectId = selectedProject.value;
  try {
    const count = await hackerOneApi.addHackerOneScopesToProject(handle, projectId);
    if (!disposed) emit("notify", "success", tr(`已发送 ${count} 个可提交网络资产到项目`, `${count} network scopes added to project`));
  } catch (error) { if (!disposed) emit("notify", "error", String(error)); }
}
onMounted(() => void load());
onUnmounted(() => { disposed = true; accountGeneration++; listGeneration++; detailGeneration++; });
</script>

<template>
  <section class="h1-board">
    <div class="section-toolbar"><div><span class="eyebrow">HACKERONE</span><h2>{{tr('厂商资讯与 Scope','Programs and scopes')}}</h2><p>{{tr('官方 API 同步使用所选方案；项目与 Scope 显示本机共享缓存。','Official API sync uses the selected profile; programs and scopes are read from this device’s shared cache.')}}</p></div><button class="button primary" :disabled="busy || !accountConfigured" @click="sync()"><RefreshCw :size="16" :class="{spinning:busy}"/> {{tr('同步项目','Sync programs')}}</button></div>
    <section class="panel h1-account-settings" data-testid="hackerone-account-settings">
      <label class="field"><span>{{ tr('HackerOne 账号方案', 'HackerOne account profile') }}</span><select v-model.number="selectedProfileId" :disabled="busy || !accountOptions.length" data-testid="hackerone-profile-choice"><option v-if="!accountOptions.length" :value="undefined">{{ tr('尚无业务方案', 'No business profiles') }}</option><option v-for="account in accountOptions" :key="account.id" :value="account.id">{{ account.name }}{{ account.isDefault ? tr('（默认）', ' (default)') : '' }} · {{ account.configured ? tr('已配置', 'Configured') : tr('待配置', 'Setup needed') }}</option></select></label>
      <button type="button" class="button ghost compact" :disabled="busy" data-testid="hackerone-configure" @click="configure">{{ accountConfigured ? tr('管理此账号', 'Manage this account') : tr('配置 HackerOne 账号', 'Configure HackerOne account') }}</button>
      <p v-if="!accountConfigured" class="helper">{{ tr('所选方案缺少凭据。可以浏览本机缓存；配置此方案后即可同步。', 'This profile needs credentials. Cached records remain available; configure this profile to synchronize.') }}</p>
      <p v-else class="helper">{{ tr('同步项目和详情刷新都会使用上方选中的方案；切换方案不会自动同步或清空共享缓存。', 'Program sync and detail refresh both use the selected profile. Switching profiles does not synchronize or clear the shared cache.') }}</p>
    </section>
    <p v-if="readError" class="form-error" role="alert">{{ readError }} <button class="text-button" type="button" @click="load">{{ tr('重试读取缓存', 'Retry cached reads') }}</button></p>
    <div class="h1-search panel"><Search :size="16"/><input v-model="search" :placeholder="tr('搜索厂商或 handle','Search name or handle')" @keyup.enter="load"/><button class="button ghost compact" @click="load">{{tr('查询','Search')}}</button></div>
    <section v-if="changes.length" class="panel h1-changes"><strong>{{tr('最近 Scope / Policy 变化','Recent scope / policy changes')}}</strong><div v-for="item in changes.slice(0,8)" :key="item.id"><em>{{item.eventType}}</em><span>@{{item.programHandle}} · {{item.summary}}</span><time>{{item.createdAt}}</time></div></section>
    <div v-if="programs.length" class="h1-grid"><article v-for="program in programs" :key="program.handle" class="h1-card panel">
      <header><img v-if="program.iconUrl" :src="program.iconUrl"/><span v-else>{{program.name.slice(0,1).toUpperCase()}}</span><button class="icon-button subtle" @click="bookmark(program)"><Star :size="16" :fill="program.bookmarked?'currentColor':'none'"/></button></header>
      <h3>{{program.name}}</h3><small>@{{program.handle}} · {{program.programState}}</small>
      <div class="h1-tags"><em v-if="program.submissionState==='open'">{{tr('接受提交','Open')}}</em><em v-if="program.offersBounties">{{tr('有赏金','Bounty')}}</em><em v-if="program.openScope">Open Scope</em><em v-if="program.safeHarbor">Safe Harbor</em><em v-if="program.fastPayments">Fast Pay</em></div>
      <footer><span>{{program.scopeCount}} Scope</span><button class="text-button" @click="show(program)">{{tr('查看详情','Details')}}</button></footer>
    </article></div>
    <div v-else class="empty-state panel">{{tr('尚无本机缓存。选择已配置的账号方案，再点击同步项目。','No cached programs. Select a configured account profile and synchronize when ready.')}}</div>

    <div v-if="detail" class="h1-detail-overlay"><section class="h1-detail panel"><button class="h1-close icon-button subtle" @click="closeDetail"><X :size="18"/></button>
      <div class="h1-detail-head"><img v-if="detail.program.iconUrl" :src="detail.program.iconUrl"/><div><h2>{{detail.program.name}}</h2><span>@{{detail.program.handle}} · {{detail.program.lastSyncedAt}}</span></div><button class="button ghost compact" :disabled="busy || !accountConfigured" @click="sync(detail.program.handle)"><RefreshCw :size="14"/>{{tr('实时刷新','Refresh')}}</button><button class="button ghost compact" @click="openUrl(`https://hackerone.com/${detail.program.handle}`)"><ExternalLink :size="14"/>{{tr('官网','Official')}}</button></div>
      <div class="h1-send"><select v-model="selectedProject"><option v-for="p in projects" :key="p.id" :value="p.id">{{p.name}}</option></select><button class="button secondary" @click="send"><UploadCloud :size="15"/>{{tr('发送可提交网络 Scope 到项目','Send network scopes to project')}}</button></div>
      <h3>Scope</h3><div class="h1-scope-table"><div class="h1-scope-head"><span>Type</span><span>Asset</span><span>{{tr('提交','Submit')}}</span><span>{{tr('赏金','Bounty')}}</span><span>Max</span></div><div v-for="scope in detail.scopes" :key="scope.id"><span>{{scope.assetType}}</span><code>{{scope.assetIdentifier}}</code><span>{{scope.eligibleForSubmission?'✓':'—'}}</span><span>{{scope.eligibleForBounty?'✓':'—'}}</span><span>{{scope.maxSeverity||'—'}}</span><small v-if="scope.instruction">{{scope.instruction}}</small></div></div>
      <template v-if="detail.exclusions.length"><h3>Scope Exclusions</h3><div class="h1-exclusions"><article v-for="item in detail.exclusions" :key="item.id"><strong>{{item.category}}</strong><p>{{item.details}}</p></article></div></template>
      <h3>Policy</h3><pre class="h1-policy">{{detail.program.policy||tr('暂无 Policy','No policy')}}</pre>
    </section></div>
  </section>
</template>
