<script setup lang="ts">
import { computed, onUnmounted, reactive, ref, watch } from "vue";
import HunterAccountSettings from "../components/HunterAccountSettings.vue";
import ProfileSectionEditor from "../components/ProfileSectionEditor.vue";
import { configurationApi } from "../api";
import { CONFIG_PROFILE_SECTIONS, cloneConfigProfileSettings, type ConfigProfileSection } from "../configProfileSections";
import { acknowledgeCategoryWrite, categoryRecoveryId, categoryReferenceIsHead, categoryReferenceKey, legacyCategoryDraft, prepareCategoryWrite, sameCategoryReference } from "../categoryState";
import type { CategoryPendingWrite, CategoryWriteResult, ConfigurationCategory } from "../categoryTypes";
import type { ConfigProfile } from "../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ profiles: ConfigProfile[]; section: ConfigProfileSection; revision?: number }>();
const emit = defineEmits<{ changed: [section: ConfigProfileSection]; manageHistory: [] }>();
const { tr } = useI18n();
const view = ref<ConfigurationCategory>();
const defaultView = ref<ConfigurationCategory>();
const loading = ref(false), error = ref(""), notice = ref(""), contextError = ref("");
const modelDeployment = ref<"local" | "cloud">(), proxyUrlContext = ref<string>();
const initialSettings = ref<Record<string, unknown>>();
const editorRevision = ref(0), legacyId = ref<number>();
const recoveryId = ref("");
const pendingWrites = reactive(new Map<ConfigProfileSection, CategoryPendingWrite>());
const writing = reactive(new Set<ConfigProfileSection>());
const pending = computed(() => pendingWrites.get(props.section));
const busy = computed(() => loading.value || writing.has(props.section));
const forkSettings = ref<Record<string, unknown>>();
const forkName = ref(""), forkDefault = ref(false);
let generation = 0, disposed = false;
onUnmounted(() => { disposed = true; generation++; });
const categoryName = computed(() => { const label = CONFIG_PROFILE_SECTIONS.find(item => item.key === props.section)!.label; return tr(label[0], label[1]); });
const selectedEntry = computed(() => view.value?.entries.find(entry => entry.reference.policyId === view.value?.reference?.policyId));
const selectedIsDefault = computed(() => view.value && defaultView.value && sameCategoryReference(view.value.reference, defaultView.value.reference) && (Boolean(view.value.reference) || view.value.source === defaultView.value.source && !initialSettings.value));
const editingHead = computed(() => !view.value || categoryReferenceIsHead(view.value));
const selectedKey = computed(() => categoryReferenceKey(view.value?.reference));
const legacySource = computed(() => props.profiles.find(profile => profile.id === legacyId.value) ?? props.profiles.find(profile => profile.id === view.value?.legacyProfileId) ?? props.profiles[0]);
const saveLabel = computed(() => !view.value?.reference ? tr('保存并设为此类默认', 'Save as this category’s default') : tr('保存当前分类', 'Save this category'));
function active(request: number, section: ConfigProfileSection) { return !disposed && generation === request && props.section === section; }
function receiptNotice(result: CategoryWriteResult) {
  return result.currentState === "modified"
    ? tr("原操作已完成；此分类默认后来有变化，已重新读取目录与当前默认。回执仅记录原操作。", "The original operation completed; this category’s default changed later. The catalog and current default have been reloaded. The receipt records the original operation only.")
    : tr("操作已确认，已重新读取目录和默认配置。回执版本不代表此配置的最新版本。", "The operation is acknowledged and the catalog and default have been reloaded. Its receipt revision is not a claim about the latest revision.");
}
function validateReceipt(result: CategoryWriteResult, section: ConfigProfileSection, requestId: string) {
  if (result.receipt.requestId !== requestId || result.receipt.section !== section) throw new Error("configuration_category_receipt_scope_mismatch");
}
async function loadCategory(recover = true, preferredOwner?: string) {
  const section = props.section, request = ++generation;
  loading.value = true; error.value = ""; contextError.value = "";
  forkSettings.value = undefined; initialSettings.value = undefined;
  try {
    recoveryId.value = categoryRecoveryId(localStorage, section) ?? "";
    if (recover && recoveryId.value) {
      const id = recoveryId.value;
      const result = await configurationApi.getCategoryReceipt(id);
      if (!active(request, section)) return false;
      if (result) {
        validateReceipt(result, section, id); acknowledgeCategoryWrite(localStorage, section, id);
        pendingWrites.delete(section); recoveryId.value = ""; notice.value = receiptNotice(result);
      }
    }
    const contextSection = section === "agent" ? "models" : section === "assetApis" ? "network" : undefined;
    const results = await Promise.allSettled([configurationApi.getCategory(section), contextSection ? configurationApi.getCategory(contextSection) : Promise.resolve(undefined)]);
    if (!active(request, section)) return false;
    if (results[0].status === "rejected") throw results[0].reason;
    const current = results[0].value!;
    if (current.section !== section) throw new Error("configuration_category_read_scope_mismatch");
    defaultView.value = cloneConfigProfileSettings(current);
    let edited = current;
    if (preferredOwner) {
      const entry = current.entries.find(item => item.reference.policyId === preferredOwner);
      if (entry) edited = await configurationApi.getCategory(section, entry.reference);
      if (!active(request, section)) return false;
    }
    view.value = cloneConfigProfileSettings(edited);
    modelDeployment.value = undefined; proxyUrlContext.value = undefined;
    if (results[1].status === "fulfilled" && results[1].value) {
      const context = results[1].value.settings;
      if (section === "agent") modelDeployment.value = context.modelDeployment === "local" ? "local" : "cloud";
      if (section === "assetApis") proxyUrlContext.value = String(context.proxyUrl ?? "");
    } else if (contextSection) contextError.value = tr("相关默认配置读取失败。可重新加载；FOFA 连通性测试暂不执行，运行时仍会独立核对实际配置。", "A related default configuration could not be loaded. Reload to retry. FOFA connectivity tests are unavailable; runtime still checks actual settings independently.");
    legacyId.value = current.legacyProfileId ?? legacySource.value?.id;
    editorRevision.value++;
    return true;
  } catch (reason) { if (active(request, section)) error.value = String(reason); return false; }
  finally { if (active(request, section)) loading.value = false; }
}
async function chooseConfiguration(key: string) {
  if (!view.value || !defaultView.value || busy.value || pending.value) return;
  const section = props.section, request = ++generation;
  loading.value = true; error.value = ""; initialSettings.value = undefined; forkSettings.value = undefined;
  try {
    const currentDefault = await configurationApi.getCategory(section);
    if (!active(request, section)) return;
    if (currentDefault.section !== section) throw new Error("configuration_category_read_scope_mismatch");
    defaultView.value = cloneConfigProfileSettings(currentDefault);
    if (key === "builtin") view.value = { ...cloneConfigProfileSettings(currentDefault), reference: null, source: "builtin", settings: cloneConfigProfileSettings(currentDefault.builtinSettings) };
    else {
      const reference = currentDefault.entries.flatMap(entry => [entry.reference, ...entry.versions]).find(item => categoryReferenceKey(item) === key);
      if (!reference) throw new Error("configuration_category_reference_unavailable");
      const selected = await configurationApi.getCategory(section, reference);
      if (!active(request, section)) return;
      if (selected.section !== section || !sameCategoryReference(selected.reference, reference)) throw new Error("configuration_category_read_scope_mismatch");
      view.value = cloneConfigProfileSettings(selected);
    }
    if (active(request, section)) editorRevision.value++;
  } catch (reason) { if (active(request, section)) error.value = String(reason); }
  finally { if (active(request, section)) loading.value = false; }
}
async function perform(proposed: CategoryPendingWrite) {
  const section = proposed.input.section, request = generation;
  if (writing.has(section)) return;
  let acknowledged = false;
  const write = pendingWrites.get(section) ?? prepareCategoryWrite(localStorage, proposed);
  pendingWrites.set(section, write); writing.add(section);
  if (props.section === section) { recoveryId.value = write.input.requestId; error.value = ""; }
  try {
    const result = write.command === "save" ? await configurationApi.saveCategory(write.input) : await configurationApi.setCategoryDefault(write.input);
    validateReceipt(result, section, write.input.requestId);
    acknowledgeCategoryWrite(localStorage, section, write.input.requestId);
    acknowledged = true; pendingWrites.delete(section);
    if (active(request, section)) {
      recoveryId.value = ""; notice.value = receiptNotice(result);
      const preferOwner = write.command === "save" && !write.input.makeDefault ? result.receipt.reference?.policyId : undefined;
      const loaded = await loadCategory(false, preferOwner);
      if (!loaded) { view.value = undefined; defaultView.value = undefined; error.value = tr("保存已确认，但重新读取失败。请重新加载；不要重复创建。", "Saving is acknowledged, but reloading failed. Reload the category; do not create it again."); }
      emit("changed", section);
    }
  } catch (reason) {
    if (!acknowledged && String(reason).startsWith("configuration_category_rejected:")) pendingWrites.delete(section);
    if (!disposed && props.section === section) error.value = String(reason);
    throw reason;
  } finally { writing.delete(section); }
}
async function persist(settings: Record<string, unknown>) {
  const current = view.value, base = defaultView.value;
  if (!current || !base) throw new Error("configuration_category_not_loaded");
  await perform({ command: "save", input: {
    requestId: crypto.randomUUID(), section: props.section, mode: current.reference ? "revise" : "create",
    name: selectedEntry.value?.name ?? `${categoryName.value} ${tr('配置', 'configuration')}`,
    settings, expectedSettings: cloneConfigProfileSettings(current.reference ? current.settings : base.settings),
    expectedReference: current.reference, expectedDefault: base.defaultSelection,
    makeDefault: !current.reference || Boolean(selectedIsDefault.value),
  } });
}
function prepareFork(settings: Record<string, unknown>) {
  if (busy.value || pending.value) return;
  forkSettings.value = cloneConfigProfileSettings(settings);
  forkName.value = `${selectedEntry.value?.name ?? categoryName.value} ${tr('副本', 'copy')}`; forkDefault.value = false;
}
async function saveFork() {
  const current = view.value, base = defaultView.value;
  if (!current || !base || !forkSettings.value || !forkName.value.trim() || busy.value) return;
  try { await perform({ command: "save", input: {
    requestId: crypto.randomUUID(), section: props.section, mode: current.reference ? "fork" : "create",
    name: forkName.value.trim(), settings: cloneConfigProfileSettings(forkSettings.value),
    expectedSettings: cloneConfigProfileSettings(current.reference ? current.settings : base.settings),
    expectedReference: current.reference, expectedDefault: base.defaultSelection, makeDefault: forkDefault.value,
  } }); } catch { /* Error and the exact pending request stay visible. */ }
}
async function useAsDefault() {
  if (!view.value || !defaultView.value || busy.value || pending.value) return;
  try { await perform({ command: "select", input: {requestId: crypto.randomUUID(), section: props.section, reference: view.value.reference, expectedDefault: defaultView.value.defaultSelection} }); }
  catch { /* Keep the rejected selection and CAS context visible. */ }
}
function importLegacy() {
  if (!view.value || !defaultView.value || !legacySource.value || busy.value || pending.value) return;
  initialSettings.value = legacyCategoryDraft(defaultView.value, legacySource.value.settings);
  view.value = { ...cloneConfigProfileSettings(defaultView.value), reference: null };
  forkSettings.value = undefined; editorRevision.value++;
  notice.value = tr("已载入这一个分类的历史草稿，尚未保存。保存会只创建并切换此类默认，历史来源和其他分类保持原值。", "This category’s historical draft is loaded but not saved. Saving creates and selects only this category’s default; its source and other categories retain their values.");
}
async function recoverReceipt() {
  const section = props.section, id = recoveryId.value.trim(), request = generation;
  if (!id || busy.value) return;
  writing.add(section); error.value = "";
  try {
    const result = await configurationApi.getCategoryReceipt(id);
    if (!active(request, section)) return;
    if (!result) { error.value = tr("尚未查到已提交回执，编号保留。可重试原请求；重新打开应用后需核对并填写原操作参数，仍使用同一编号。", "No committed receipt was found. Keep the id and retry the original request. After reopening, re-enter its original parameters using the same id."); return; }
    validateReceipt(result, section, id); acknowledgeCategoryWrite(localStorage, section, id);
    pendingWrites.delete(section); recoveryId.value = ""; notice.value = receiptNotice(result);
    await loadCategory(false); emit("changed", section);
  } catch (reason) { if (!disposed && props.section === section) error.value = String(reason); }
  finally { writing.delete(section); }
}
async function retryPending() { if (pending.value && !busy.value) try { await perform(pending.value); } catch { /* Already visible. */ } }
watch(() => props.section, () => { view.value = undefined; defaultView.value = undefined; notice.value = ""; void loadCategory(); }, { immediate: true });
</script>

<template>
  <section class="config-domain-editor" data-testid="config-domain-settings" :data-section="section">
    <p v-if="loading" class="helper" role="status">{{tr('正在读取此分类配置…','Loading category settings…')}}</p>
    <button v-if="!view && !loading" class="button ghost compact" @click="loadCategory()">{{tr('重新加载此分类','Reload category')}}</button>
    <template v-if="view">
      <header class="domain-owner">
        <div><strong>{{selectedEntry?.name || (view.source==='legacy-fallback' && !initialSettings ? tr('当前历史兼容来源','Current historical fallback') : tr('内置参数 / 新配置','Built-in parameters / new configuration'))}}</strong><span v-if="view.reference">{{tr(`第 ${view.reference.revision} 版`,`Revision ${view.reference.revision}`)}}</span><span v-if="selectedIsDefault" class="default-pill">{{tr('此类当前默认','Current category default')}}</span></div>
        <button class="button ghost compact" :disabled="busy || Boolean(pending)" @click="loadCategory()">{{tr('重新加载','Reload')}}</button>
      </header>
      <p v-if="view.source==='legacy-fallback' && !view.reference && !initialSettings" class="helper">{{tr('此类尚未选择独立默认，当前真实使用历史兼容值。首次保存只建立此类默认，不修改原来源或其他分类。','No independent default is selected for this category; runtime currently uses these historical values. Its first save creates only this category’s default, preserving its source and other categories.')}}</p>
      <div class="category-catalog">
        <label class="field"><span>{{tr('编辑此类配置','Configuration to edit')}}</span><select :value="selectedKey" :disabled="busy || Boolean(pending) || Boolean(forkSettings)" @change="chooseConfiguration(($event.target as HTMLSelectElement).value)"><option value="builtin">{{tr('内置参数 / 新建此类配置','Built-in parameters / create this category')}}</option><optgroup v-for="entry in view.entries" :key="entry.reference.policyId" :label="entry.name"><option v-for="version in [...entry.versions].sort((a,b)=>b.revision-a.revision)" :key="categoryReferenceKey(version)" :value="categoryReferenceKey(version)">{{tr(`第 ${version.revision} 版`,`Revision ${version.revision}`)}}{{sameCategoryReference(version,defaultView?.reference)?tr('（当前默认）',' (current default)'):''}}{{sameCategoryReference(version,entry.reference)?tr(' · 最新',' · latest'):''}}</option></optgroup></select></label>
        <button class="button secondary compact" :disabled="busy || Boolean(pending) || Boolean(forkSettings) || Boolean(selectedIsDefault)" @click="useAsDefault">{{view.reference?tr('仅设为此类默认','Use as this category’s default'):tr('仅将此类恢复为内置参数','Use built-in parameters for this category')}}</button>
      </div>
      <p class="helper">{{tr('选择编辑对象不会切换默认。保存和默认切换只作用于这一类；已有任务保留其启动配置与恢复约束。','Selecting an editor does not switch defaults. Saving or switching defaults affects only this category. Existing tasks retain their starting configuration and recovery constraints.')}}</p>
      <p v-if="!editingHead" class="helper">{{tr('正在查看旧版本，不能覆盖历史。可编辑后另存为新配置，或明确将此版本设为默认。','This is a historical version and cannot be overwritten. Edit and save a new configuration, or explicitly select this version as the default.')}}</p>
      <p v-if="contextError" class="form-error">{{contextError}}</p>
      <ProfileSectionEditor :key="`${section}:${editorRevision}`" :section="section" :category="view" :initial-settings="initialSettings" :model-deployment="modelDeployment" :proxy-url-context="proxyUrlContext" :persist="persist" :save-label="saveLabel" :read-only="!editingHead" :locked="busy || Boolean(pending) || Boolean(forkSettings)" @fork-draft="prepareFork" @reload="loadCategory()" />
      <HunterAccountSettings v-if="section==='assetApis'" />
      <section v-if="forkSettings" class="category-fork panel" data-testid="category-fork">
        <h3>{{tr('仅另存此分类','Save only this category as new')}}</h3>
        <label class="field"><span>{{tr('新配置名称','New configuration name')}}</span><input v-model="forkName" maxlength="120" :disabled="busy || Boolean(pending)" /></label>
        <label class="check-field"><input v-model="forkDefault" type="checkbox" :disabled="busy || Boolean(pending)" />{{tr('保存后仅设为此类默认','Use as this category’s default after saving')}}</label>
        <div class="hero-actions"><button class="button primary compact" :disabled="busy || Boolean(pending) || !forkName.trim()" @click="saveFork">{{tr('创建这一类的新配置','Create this category’s new configuration')}}</button><button class="button ghost compact" :disabled="busy || Boolean(pending)" @click="forkSettings=undefined">{{tr('取消另存','Cancel save as')}}</button></div>
      </section>
      <details class="domain-scope">
        <summary>{{tr('历史来源与显式导入（仅此分类）','Historical sources and explicit import (this category only)')}}</summary>
        <template v-if="profiles.length"><label class="field"><span>{{tr('历史来源','Historical source')}}</span><select :value="legacySource?.id" :disabled="busy || Boolean(pending)" @change="legacyId=Number(($event.target as HTMLSelectElement).value)"><option v-for="source in profiles" :key="source.id" :value="source.id">{{source.name}}</option></select></label><button class="button ghost compact" :disabled="busy || Boolean(pending) || Boolean(forkSettings)" @click="importLegacy">{{tr('仅载入此分类草稿','Load only this category’s draft')}}</button></template>
        <p class="helper">{{tr('导入不会复制其他七类、未知字段或停用凭据。草稿确认保存后才创建此类独立配置，原历史来源保持不变。','Import does not copy the other seven categories, unknown fields or disabled credentials. Only saving the reviewed draft creates an independent configuration; its historical source remains unchanged.')}}</p>
        <button class="button ghost compact" @click="$emit('manageHistory')">{{tr('管理历史来源名称','Manage historical source names')}}</button>
      </details>
    </template>
    <details class="category-recovery" :open="Boolean(pending) || Boolean(recoveryId)">
      <summary>{{tr('操作回执与恢复','Operation receipts and recovery')}}</summary>
      <label class="field"><span>{{tr('只保存不含凭据的请求编号','Only the non-secret request id is stored')}}</span><input v-model="recoveryId" :readonly="Boolean(pending)" maxlength="36" /></label>
      <div class="hero-actions"><button class="button ghost compact" :disabled="busy || !recoveryId.trim()" @click="recoverReceipt">{{tr('查询原操作结果','Look up original result')}}</button><button v-if="pending" class="button primary compact" :disabled="busy" @click="retryPending">{{tr('原参数精确重试','Retry exact original request')}}</button></div>
      <p v-if="pending" class="helper">{{tr('原操作结果尚未确认，当前私有请求已冻结；重试沿用同一编号和参数，不会自动另建配置。','The original outcome is unconfirmed. Its private request is frozen; retry uses the same id and parameters without automatically creating another configuration.')}}</p>
    </details>
    <p v-if="notice" class="helper category-notice" role="status">{{notice}}</p>
    <p v-if="error" class="form-error" role="alert">{{error}}</p>
  </section>
</template>

<style scoped>
.config-domain-editor{min-width:0}.domain-owner{display:flex;align-items:center;justify-content:space-between;gap:12px;margin:0 0 14px}.domain-owner>div{display:flex;align-items:center;flex-wrap:wrap;gap:10px}.domain-owner strong{font-size:13px}.domain-owner span{font-size:12px;color:var(--muted,#64748b)}.category-catalog{display:flex;align-items:flex-end;gap:12px;margin:16px 0}.category-catalog .field{flex:1;margin:0}.category-catalog>button{min-height:36px}.domain-scope,.category-recovery,.category-fork{margin:18px 0;padding:14px;background:var(--app-bg,#f5f7fa);border:1px solid var(--border,#dce2eb);border-radius:8px}.domain-scope summary,.category-recovery summary{font-size:12px;cursor:pointer}.domain-scope[open] summary,.category-recovery[open] summary{margin-bottom:12px}.config-domain-editor .helper{font-size:12px;line-height:1.7}.category-fork h3{font-size:14px;margin:0 0 12px}.category-fork .field{margin-bottom:12px}.category-fork .hero-actions,.category-recovery .hero-actions{margin-top:12px}.category-notice{padding:12px;background:var(--app-bg,#f5f7fa);border-radius:8px}@media(max-width:760px){.category-catalog{align-items:stretch;flex-direction:column}}
</style>
