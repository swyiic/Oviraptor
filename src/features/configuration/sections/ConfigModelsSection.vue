<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { Eye, EyeOff, Plus, RefreshCw } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import { openAgentHostSettingsClient, type HostSettingsClient } from "../../agentHost/api";
import type { AgentHostSettings, AgentNetworkMode, AgentOAuthStartResult } from "../../agentHost/types";
import { configurationApi } from "../api";
import { createAgentModelProfile, type AgentModelProfile } from "../configProfileDefaults";
import { cloneConfigProfileSettings, configPolicyPreviewSettings, modelConnectionSignature, type ConfigPolicyReference } from "../configProfileSections";
import { acknowledgeCategoryWrite, categoryRecoveryId, categoryReferenceIsHead, prepareCategoryWrite, sameCategoryReference } from "../categoryState";
import type { CategoryPendingWrite, ConfigurationCategory } from "../categoryTypes";
import { directoryConnections, matchesModelBinding, modelSettingsForConnection, type ModelConnectionBinding, type ModelDirectoryConnection } from "../modelConnections";
import type { ConfigProfile } from "../types";
import { useI18n } from "../../../i18n";

const props = defineProps<{ profiles: ConfigProfile[] }>();
const emit = defineEmits<{ changed: []; notify: [type: "success" | "error" | "info", text: string] }>();
const { tr } = useI18n();
const loading = ref(true), busy = ref(false), error = ref(""), notice = ref(""), hostError = ref("");
const category = ref<ConfigurationCategory>(), connections = ref<ModelDirectoryConnection[]>([]);
const host = ref<AgentHostSettings>(), selectedKey = ref("new-api"), showKey = ref(false);
const form = reactive<AgentModelProfile>(createAgentModelProfile());
const makeDefault = ref(false), testBusy = ref(false), testedSignature = ref("");
const testedMessage = ref(""), recoveryId = ref(""), oauth = ref<AgentOAuthStartResult>();
const networkMode = ref<AgentNetworkMode>("system"), networkProxy = ref(""), networkNoProxy = ref("");
const advancedBusy = ref(false), legacyId = ref<number>();
const keychainEnabled = ref(false), keychainBusy = ref(false);
onMounted(async () => {
  try { keychainEnabled.value = await invoke<boolean>("get_credential_keychain_enabled"); } catch { /* default off */ }
});
async function toggleKeychain(next: boolean) {
  keychainBusy.value = true;
  try {
    keychainEnabled.value = await invoke<boolean>("set_credential_keychain_enabled", { enabled: next });
    emit("notify", "success", keychainEnabled.value
      ? tr("已开启：API Key 将保存到系统钥匙串（下次模型调用时自动迁移；数据库中仍保留以便回退）。", "Enabled: API keys will be stored in the system keychain (migrated on the next model call; a copy stays in the database as a fallback).")
      : tr("已关闭：仅使用数据库中保存的凭据。", "Disabled: credentials are used from the database only."));
  } catch (reason) {
    emit("notify", "error", String(reason));
  } finally { keychainBusy.value = false; }
}
let client: HostSettingsClient | null = null, generation = 0, disposed = false;
onUnmounted(() => { disposed = true; generation++; });
type BoundProvider = NonNullable<AgentHostSettings["apiProvider"]>;
const provider = computed(() => host.value?.apiProvider as BoundProvider | undefined);
const selected = computed(() => connections.value.find(connection => connection.key === selectedKey.value));
const selectingOAuth = computed(() => form.authType === "oauth");
const selectingUnboundHost = computed(() => selectedKey.value === "host-unbound");
const hostChatEnabled = computed(() => host.value?.runtime.id === "xai-api");
const oauthActive = computed(() => Boolean(hostChatEnabled.value && provider.value?.oauthConfigured));
const oauthSaved = computed(() => Boolean(provider.value?.oauthSaved || provider.value?.oauthConfigured));
const hasUnboundHost = computed(() => Boolean(provider.value?.configured && !provider.value.oauthConfigured && !provider.value.connectionRef));
const defaultConnection = computed(() => connections.value.find(connection => category.value && sameCategoryReference(connection.reference, category.value.reference) && connection.profile.id === category.value.settings.activeModelProfileId));
const isCurrentDefault = computed(() => Boolean(defaultConnection.value && defaultConnection.value.profile.id === form.id));
const hasOAuthConnection = computed(() => connections.value.some(connection => connection.profile.authType === "oauth"));
const visibleConnections = computed(() => {
  const preferred = connections.value.filter(connection => sameCategoryReference(connection.reference, category.value?.reference));
  const directory = new Map<string, ModelDirectoryConnection>();
  for (const connection of [...preferred, ...connections.value]) if (!directory.has(connection.profile.id)) directory.set(connection.profile.id, connection);
  return [...directory.values()];
});
const defaultSynchronized = computed(() => {
  const current = defaultConnection.value, saved = provider.value;
  if (!current || !hostChatEnabled.value || !saved?.configured) return false;
  const oauth = current.profile.authType === "oauth";
  const expectedModel = current.profile.llm.trim().replace(/^openai\//, "").trim();
  if (saved.model !== expectedModel) return false;
  if (Boolean(saved.oauthConfigured) !== oauth) return false;
  if (!oauth && saved.baseUrl?.replace(/\/$/, "") !== current.profile.apiBase.trim().replace(/\/$/, "")) return false;
  const bound = connections.value.find(connection => matchesModelBinding(saved.connectionRef, connection));
  return Boolean(bound && modelConnectionSignature(bound.profile) === modelConnectionSignature(current.profile));
});
const authChoice = computed({
  get: () => form.authType === "oauth" ? "oauth" : form.deployment,
  set: (value: string) => {
    form.authType = value === "oauth" ? "oauth" : value === "local" && !form.localApiKey ? "none" : "api-key";
    form.deployment = value === "local" ? "local" : "cloud";
    if (value === "oauth") { form.apiBase = "https://api.x.ai/v1"; form.apiKey = ""; form.localApiKey = ""; form.llm ||= "grok-4.7"; }
    testedSignature.value = ""; testedMessage.value = "";
  },
});
const legacySource = computed(() => props.profiles.find(profile => profile.id === legacyId.value) ?? props.profiles.find(profile => profile.id === category.value?.legacyProfileId) ?? props.profiles[0]);

type Workflow = {
  profile: AgentModelProfile; reference: ConfigPolicyReference | null;
  applyDefault: boolean; priorHostIdentity: string | null; defaultSuperseded: boolean;
  rustDone: boolean; hostDone: boolean; rustRequest?: CategoryPendingWrite;
};
const workflow = ref<Workflow>();
const locked = computed(() => busy.value || Boolean(workflow.value));
const steps = reactive({ rust: "", default: "" });
function isCurrent(request: number) { return !disposed && request === generation; }
function dispatchHostSettings(value: AgentHostSettings) {
  host.value = value;
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("oviraptor:agent-host-settings"));
}
function hostIdentity(value: AgentHostSettings | undefined) { const provider = value?.apiProvider; return JSON.stringify([value?.runtime.id, provider?.provider, provider?.baseUrl, provider?.model, provider?.oauthConfigured, provider?.connectionRef ?? null]); }
function assignDraft(profile: AgentModelProfile) {
  Object.assign(form, createAgentModelProfile(), cloneConfigProfileSettings(profile));
  form.authType = profile.authType ?? (profile.deployment === "local" && !profile.localApiKey ? "none" : "api-key");
  testedSignature.value = ""; testedMessage.value = ""; showKey.value = false;
}
function selectConnection(key: string, force = false) {
  if (locked.value && !force) return;
  selectedKey.value = key; error.value = ""; notice.value = "";
  const connection = connections.value.find(item => item.key === key);
  if (connection) assignDraft(connection.profile);
  else if (key === "host-unbound") assignDraft({ ...createAgentModelProfile(), name: provider.value?.model || "Host API", llm: provider.value?.model || "", apiBase: provider.value?.baseUrl || "", deployment: provider.value?.authType === "none" ? "local" : "cloud" });
  else if (key === "grok-oauth") assignDraft({ ...createAgentModelProfile("Grok"), llm: provider.value?.oauthConfigured ? provider.value?.model || "grok-4.7" : "grok-4.7", authType: "oauth", apiBase: "https://api.x.ai/v1" });
  else assignDraft(createAgentModelProfile(tr("新模型连接", "New model connection")));
  makeDefault.value = !defaultConnection.value || isCurrentDefault.value;
}
async function loadDirectory(preserveSelection = false, preferred?: ModelConnectionBinding) {
  const request = ++generation; loading.value = true; error.value = ""; hostError.value = "";
  try {
    const current = await configurationApi.getCategory("models");
    if (!isCurrent(request)) return;
    if (current.section !== "models") throw new Error("configuration_category_read_scope_mismatch");
    category.value = current;
    const references = current.entries.map(entry => entry.reference);
    if (current.reference && !references.some(reference => sameCategoryReference(reference, current.reference))) references.push(current.reference);
    const views = await Promise.allSettled(references.map(reference => configurationApi.getCategory("models", reference)));
    if (!isCurrent(request)) return;
    const list = current.reference ? [] as ModelDirectoryConnection[] : directoryConnections(current);
    for (let index = 0; index < views.length; index++) {
      const result = views[index];
      if (result.status === "rejected") throw result.reason;
      if (result.value.section !== "models" || !sameCategoryReference(result.value.reference, references[index])) throw new Error("configuration_category_read_scope_mismatch");
      list.push(...directoryConnections(result.value));
    }
    connections.value = [...new Map(list.map(connection => [connection.key, connection])).values()];
    legacyId.value = current.legacyProfileId ?? legacySource.value?.id;
    recoveryId.value = categoryRecoveryId(localStorage, "models") ?? "";
    try {
      if (!client) client = (await openAgentHostSettingsClient())?.client ?? null;
      if (client) {
        const value = await client.getSettings();
        if (!isCurrent(request)) return;
        dispatchHostSettings(value);
        networkMode.value = value.network?.mode ?? "system"; networkNoProxy.value = value.network?.noProxy ?? "";
      } else hostError.value = tr("模型服务暂未连接。仍可保存非默认连接；连接恢复后可重试默认模型同步。", "The model service is unavailable. Other connections can still be saved; retry default synchronization after reconnecting.");
    } catch (reason) { if (isCurrent(request)) hostError.value = String(reason); }
    if (!isCurrent(request)) return;
    const hostBinding = provider.value?.connectionRef;
    if (hostBinding && !connections.value.some(connection => matchesModelBinding(hostBinding, connection))) {
      const bound = await configurationApi.getCategory("models", { policyId: hostBinding.policyId, revision: hostBinding.revision, kind: "models" });
      if (!isCurrent(request)) return;
      connections.value.push(...directoryConnections(bound).filter(connection => !connections.value.some(existing => existing.key === connection.key)));
    }
    if (!preserveSelection || !connections.value.some(connection => connection.key === selectedKey.value)) {
      const chat = connections.value.find(connection => matchesModelBinding(provider.value?.connectionRef, connection));
      selectConnection(connections.value.find(connection => matchesModelBinding(preferred, connection))?.key ?? defaultConnection.value?.key ?? chat?.key ?? (oauthActive.value ? "grok-oauth" : connections.value[0]?.key) ?? "new-api", true);
    }
  } catch (reason) { if (isCurrent(request)) error.value = String(reason); }
  finally { if (isCurrent(request)) loading.value = false; }
}
function normalizedDraft(): AgentModelProfile {
  const profile = cloneConfigProfileSettings(form);
  profile.name = profile.name.trim(); profile.llm = profile.llm.trim(); profile.apiBase = profile.apiBase.trim();
  if (profile.authType === "oauth") { profile.deployment = "cloud"; profile.apiBase = "https://api.x.ai/v1"; profile.apiKey = ""; profile.localApiKey = ""; }
  else profile.authType = profile.deployment === "local" && !profile.localApiKey.trim() ? "none" : "api-key";
  return profile;
}
function validateConnectionFields() {
  if (!form.name.trim() || !form.llm.trim()) throw new Error(tr("请填写连接名称和模型 ID。", "Enter a connection name and model id."));
  if (selectingOAuth.value) {
    if (!oauthSaved.value) throw new Error(tr("请先完成 Grok 登录。", "Complete Grok sign-in first."));
    return;
  }
  if (!form.apiBase.trim()) throw new Error(tr("请填写 API 地址。", "Enter the API endpoint."));
  const url = new URL(form.apiBase.trim());
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw new Error(tr("API 地址必须是无内嵌凭据的 HTTP(S) 地址。", "Use an HTTP(S) endpoint without embedded credentials."));
  if (form.deployment === "cloud" && !form.apiKey.trim()) throw new Error(tr("请填写 API Key。", "Enter the API key."));
}
function validateDraft() {
  validateConnectionFields();
  if (selectingOAuth.value) return;
  const changed = !selected.value || modelConnectionSignature(form) !== modelConnectionSignature(selected.value.profile);
  if (changed && testedSignature.value !== modelConnectionSignature(form)) throw new Error(tr("连接已新增或修改，请先测试连接。测试会调用模型服务，可能计费。", "This connection is new or changed. Test it first; the model call may incur a charge."));
}
async function testConnection() {
  if (testBusy.value || locked.value) return;
  testBusy.value = true; testedMessage.value = ""; error.value = "";
  const signature = modelConnectionSignature(form);
  try {
    validateConnectionFields();
    const result = await configurationApi.testModelProfile({ llm: form.llm.trim(), deployment: form.deployment, apiBase: form.apiBase.trim(), apiKey: form.deployment === "local" ? form.localApiKey : form.apiKey });
    if (!result.ok) throw new Error(result.message);
    if (signature !== modelConnectionSignature(form)) throw new Error(tr("测试期间连接已改变，请重新测试。", "The connection changed during its test. Test again."));
    testedSignature.value = signature; testedMessage.value = result.message;
  } catch (reason) { error.value = String(reason); }
  finally { testBusy.value = false; }
}
async function sendRust(write: CategoryPendingWrite) {
  const exact = prepareCategoryWrite(localStorage, write); recoveryId.value = exact.input.requestId;
  const result = exact.command === "save" ? await configurationApi.saveCategory(exact.input) : await configurationApi.setCategoryDefault(exact.input);
  if (result.receipt.section !== "models" || result.receipt.requestId !== exact.input.requestId) throw new Error("configuration_category_receipt_scope_mismatch");
  acknowledgeCategoryWrite(localStorage, "models", exact.input.requestId); recoveryId.value = "";
  if (result.currentState === "modified") notice.value = tr("原写入已确认，但模型默认后来已变化；不会自动覆盖新的选择。", "The original write is acknowledged, but the model default changed later. The newer selection will not be overwritten automatically.");
  return result;
}
async function saveConnection() {
  if (locked.value || !category.value) return;
  error.value = ""; notice.value = ""; steps.rust = ""; steps.default = "";
  try {
    validateDraft();
    const current = category.value, profile = normalizedDraft();
    // Every saved connection joins the current directory. Advancing its revision
    // keeps the active model unchanged unless the user selects a new default.
    const settings = configPolicyPreviewSettings(modelSettingsForConnection(current.settings, profile, makeDefault.value), "models");
    const needsWrite = !current.reference || JSON.stringify(settings) !== JSON.stringify(configPolicyPreviewSettings(current.settings, "models"));
    const intent: Workflow = {
      profile, reference: current.reference, applyDefault: settings.activeModelProfileId === profile.id,
      priorHostIdentity: host.value ? hostIdentity(host.value) : null, defaultSuperseded: false, rustDone: !needsWrite, hostDone: false,
    };
    if (needsWrite) intent.rustRequest = { command: "save", input: {
      requestId: crypto.randomUUID(), section: "models", mode: !current.reference ? "create" : categoryReferenceIsHead(current) ? "revise" : "fork",
      name: current.entries.find(entry => entry.reference.policyId === current.reference?.policyId)?.name || tr("模型目录", "Model catalog"),
      settings, expectedSettings: cloneConfigProfileSettings(current.settings), expectedReference: current.reference,
      expectedDefault: current.defaultSelection, makeDefault: true,
    } };
    workflow.value = intent; await continueWorkflow();
  } catch (reason) { error.value = String(reason); }
}
async function continueWorkflow() {
  const intent = workflow.value;
  if (!intent || busy.value) return;
  const request = generation;
  busy.value = true; error.value = "";
  try {
    assertWorkflowActive(intent, request);
    if (!intent.rustDone && intent.rustRequest) {
      const result = await sendRust(intent.rustRequest); intent.reference = result.receipt.reference; intent.rustDone = true;
      intent.defaultSuperseded = result.currentState === "modified";
      steps.rust = tr("连接已保存，聊天、资产任务和 Nest 共用此目录。", "Connection saved in the shared catalog for chat, asset tasks and Nest.");
      assertWorkflowActive(intent, request);
    } else if (!steps.rust) steps.rust = tr("使用目录中已保存的连接。", "Using the saved catalog connection.");
    if (!intent.hostDone) await applyDefaultModel(intent, request);
    assertWorkflowActive(intent, request);
    intent.hostDone = true; workflow.value = undefined;
    notice.value ||= intent.defaultSuperseded
      ? tr("连接保存已确认；默认模型已在其他位置更新，请核对当前选择。", "The connection save is confirmed; the default changed elsewhere. Review the current selection.")
      : intent.applyDefault ? tr("已保存并设为默认模型。", "Saved and selected as the default model.") : tr("模型连接已保存，可随时设为默认。", "Connection saved and ready to select as the default.");
    await loadDirectory(false, intent.reference ? { ...intent.reference, kind: "models" as const, modelProfileId: intent.profile.id } : undefined); if (!disposed) emit("changed");
  } catch (reason) {
    if (!intent.rustDone && String(reason).startsWith("configuration_category_rejected:")) workflow.value = undefined;
    error.value = String(reason);
  } finally { busy.value = false; }
}
function assertWorkflowActive(intent: Workflow, request: number) {
  if (disposed || generation !== request || workflow.value !== intent) throw new Error(tr("页面已离开或重新读取。已确认的保存结果保留，请重新读取后核对默认模型。", "The page was left or reloaded. Confirmed saves are retained; reload to review the default model."));
}
async function applyDefaultModel(intent: Workflow, request: number) {
  assertWorkflowActive(intent, request);
  if (!intent.applyDefault || intent.defaultSuperseded) return;
  if (!client) throw new Error(tr("模型已保存；默认模型尚未同步。模型服务恢复后可重试，无需重新添加。", "The model is saved, but default synchronization is pending. Retry when the model service reconnects; do not add it again."));
  const current = await client.getSettings();
  assertWorkflowActive(intent, request);
  if (intent.priorHostIdentity !== null && hostIdentity(current) !== intent.priorHostIdentity) throw new Error(tr("当前模型已在其他位置切换，请重新读取后核对默认模型。", "The model changed elsewhere. Reload and review the default model."));
  if (!intent.reference) throw new Error("configuration_model_reference_unconfirmed");
  const profile = intent.profile;
  const connectionRef = { ...intent.reference, kind: "models" as const, modelProfileId: profile.id };
  const saved = profile.authType === "oauth"
    ? await client.selectOAuth({ model: profile.llm, connectionRef })
    : await client.setProvider({
      provider: "openai-compatible", model: profile.llm, baseUrl: profile.apiBase,
      apiKey: profile.deployment === "local" ? profile.localApiKey.trim() : profile.apiKey.trim(),
      authType: profile.deployment === "local" && !profile.localApiKey.trim() ? "none" : "api-key",
      clearApiKey: profile.deployment === "local" && !profile.localApiKey.trim(), connectionRef,
    });
  dispatchHostSettings(saved); intent.priorHostIdentity = hostIdentity(saved);
  assertWorkflowActive(intent, request);
  dispatchHostSettings(await client.setRuntime("xai-api"));
  intent.priorHostIdentity = hostIdentity(host.value);
  assertWorkflowActive(intent, request);
  steps.default = tr("默认模型已同步。", "Default model synchronized.");
}
async function retryDefault() {
  if (!workflow.value?.rustDone || busy.value) return;
  try { if (!client) client = (await openAgentHostSettingsClient())?.client ?? null; await continueWorkflow(); }
  catch (reason) { error.value = String(reason); }
}
async function recoverRust() {
  const id = recoveryId.value.trim(); if (!id || busy.value) return;
  const request = generation, intentAtStart = workflow.value;
  busy.value = true; error.value = "";
  try {
    const result = await configurationApi.getCategoryReceipt(id);
    if (!isCurrent(request) || workflow.value !== intentAtStart) return;
    if (!result) { notice.value = tr("尚未找到回执。编号保留，原请求可精确重试；不会另建连接。", "No receipt was found. Keep the id and retry the exact original request; no new connection is created."); return; }
    if (result.receipt.section !== "models" || result.receipt.requestId !== id) throw new Error("configuration_category_receipt_scope_mismatch");
    acknowledgeCategoryWrite(localStorage, "models", id); recoveryId.value = "";
    if (workflow.value) {
      const intent = workflow.value;
      if (intent.rustRequest) await sendRust(intent.rustRequest);
      if (!isCurrent(request) || workflow.value !== intent) return;
      intent.reference = result.receipt.reference; intent.rustDone = true;
      intent.defaultSuperseded = result.currentState === "modified";
    } else {
      notice.value = tr("原保存已确认，请核对当前模型与默认选择。", "The original save is confirmed. Review the current model and default selection.");
      await loadDirectory();
    }
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
async function authorizeOAuth() {
  if (!client || busy.value) return; busy.value = true; error.value = "";
  try { oauth.value = await client.startOAuth({ background: true, activate: false }); const uri = oauth.value.verificationUriComplete || oauth.value.oauth?.verificationUriComplete || oauth.value.verificationUri || oauth.value.oauth?.verificationUri; if (uri) await openUrl(uri); }
  catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
async function checkOAuth() {
  if (!client || busy.value) return; busy.value = true; error.value = "";
  try { oauth.value = await client.pollOAuth(); dispatchHostSettings(await client.getSettings()); }
  catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
async function logoutOAuth() { if (!client || busy.value) return; busy.value = true; try { dispatchHostSettings(await client.logoutOAuth()); } catch (reason) { error.value = String(reason); } finally { busy.value = false; } }
async function saveAdvanced() {
  if (!client || advancedBusy.value || locked.value) return; advancedBusy.value = true; error.value = "";
  try { dispatchHostSettings(await client.setNetwork({ mode: networkMode.value, noProxy: networkNoProxy.value, ...(networkProxy.value.trim() ? { proxyUrl: networkProxy.value.trim() } : {}) })); networkProxy.value = ""; notice.value = tr("聊天网络已保存，扫描业务网络保持独立。", "Chat networking saved; scan business networking remains independent."); }
  catch (reason) { error.value = String(reason); }
  finally { advancedBusy.value = false; }
}
function importLegacy() {
  if (locked.value || !legacySource.value || !category.value) return;
  const source = { ...category.value, reference: null, source: "legacy-fallback" as const, settings: legacySource.value.settings };
  const imported = directoryConnections(source);
  connections.value = [...connections.value.filter(connection => !connection.historical), ...imported];
  if (imported[0]) selectConnection(imported[0].key);
  notice.value = tr("仅载入历史模型连接；保存后才登记到目录，不复制其他分类或 OAuth 登录。", "Only historical model connections were loaded. Saving registers them in the catalog; no other category or OAuth login is copied.");
}
onMounted(() => { void loadDirectory(); });
</script>

<template>
  <section class="models-settings" data-testid="unified-models-settings">
    <header class="models-overview">
      <div><p class="models-intro">{{tr('一个模型目录，供聊天、资产任务和 Nest 共用。','One model catalog shared by chat, asset tasks and Nest.')}}</p><p class="helper">{{tr('添加连接后，设为默认即可用于后续对话与任务。','Save a connection and select it as the default for subsequent conversations and tasks.')}}</p></div>
      <button v-refresh-feedback="{busy:loading,error:error||hostError}" class="button ghost compact" :disabled="loading || locked" @click="loadDirectory()"><RefreshCw :size="14"/>{{tr('刷新','Refresh')}}</button>
    </header>
    <div v-if="defaultConnection" class="model-default-summary" data-testid="model-default-summary">
      <span>{{tr('默认模型','Default model')}}<strong>{{defaultConnection.name}}</strong></span>
      <span class="model-state" :class="{ready:defaultSynchronized}">{{defaultSynchronized?tr('已同步','Synchronized'):tr('待同步','Sync pending')}}</span>
      <button v-if="!defaultSynchronized" class="text-button" :disabled="locked || loading" @click="selectConnection(defaultConnection.key); makeDefault=true">{{tr('核对并同步','Review and sync')}}</button>
    </div>
    <div class="models-layout">
      <aside class="model-directory" :aria-label="tr('模型目录','Model catalog')">
        <div class="directory-heading"><span>{{tr('模型连接','Connections')}}</span><small>{{visibleConnections.length}}</small></div>
        <button class="button ghost compact add-model" :disabled="locked" @click="selectConnection('new-api')"><Plus :size="15"/>{{tr('添加模型','Add model')}}</button>
        <div class="model-directory-list">
          <button v-for="connection in visibleConnections" :key="connection.key" class="connection-row" :class="{active:selectedKey===connection.key}" :aria-pressed="selectedKey===connection.key" :disabled="locked" @click="selectConnection(connection.key)">
            <span class="connection-title"><strong>{{connection.name}}</strong><em v-if="connection.profile.id===defaultConnection?.profile.id">{{tr('默认','Default')}}</em></span>
            <small class="connection-model-id">{{connection.profile.llm}}</small><span class="connection-auth">{{connection.profile.authType==='oauth'?'OAuth':connection.profile.deployment==='local'?tr('本地 / 自建','Local / self-hosted'):'API Key'}}<span v-if="connection.historical"> · {{tr('历史来源','Historical source')}}</span></span>
          </button>
          <button v-if="!hasOAuthConnection" class="connection-row" :class="{active:selectedKey==='grok-oauth'}" :aria-pressed="selectedKey==='grok-oauth'" :disabled="locked" @click="selectConnection('grok-oauth')"><span class="connection-title"><strong>Grok</strong><em>OAuth</em></span><small>{{oauthSaved?tr('登录已保存，添加到模型目录','Sign-in saved · add to catalog'):tr('通过账号登录','Sign in with your account')}}</small></button>
          <button v-if="hasUnboundHost" class="connection-row" :class="{active:selectingUnboundHost}" :disabled="locked" @click="selectConnection('host-unbound')"><strong>{{provider?.model || 'Host API'}}</strong><small>{{tr('已有连接 · 待登记','Existing connection · not in catalog')}}</small></button>
        </div>
      </aside>
      <div class="model-editor">
        <header class="model-editor-heading"><div><h3>{{selected ? tr('编辑模型','Edit model') : selectingUnboundHost ? tr('登记已有模型','Register existing model') : tr('添加模型','Add model')}}</h3><p class="helper">{{tr('所有模型通用，仅连接与认证方式不同。','All models are shared; only connection and authentication differ.')}}</p></div><span v-if="selectingOAuth" class="model-auth-badge">OAuth</span></header>
        <p v-if="loading" class="helper" role="status">{{tr('正在读取模型目录…','Loading model catalog…')}}</p>
        <fieldset :disabled="locked || loading">
          <div class="model-fields">
            <label class="field"><span>{{tr('连接名称','Connection name')}}</span><input v-model="form.name" maxlength="120" :placeholder="tr('例如：DeepSeek','e.g. DeepSeek')"/></label>
            <label class="field"><span>{{tr('模型 ID','Model id')}}</span><input v-model="form.llm" :placeholder="selectingOAuth?'grok-4.7':'deepseek-chat / local model'" spellcheck="false"/></label>
            <label class="field model-full"><span>{{tr('连接方式','Connection type')}}</span><select v-model="authChoice"><option value="cloud">API Key</option><option value="local">{{tr('本地 / 自建（Key 可选）','Local / self-hosted (optional key)')}}</option><option value="oauth">Grok OAuth</option></select></label>
            <template v-if="!selectingOAuth">
              <label class="field model-full"><span>API URL</span><input v-model="form.apiBase" placeholder="https://api.example.com/v1" spellcheck="false"/></label>
              <label class="field model-full"><span>{{form.deployment==='local'?tr('API Key（可选）','API key (optional)'):'API Key'}}</span><div class="secret-input"><input v-if="form.deployment==='local'" v-model="form.localApiKey" :type="showKey?'text':'password'" autocomplete="new-password" spellcheck="false"/><input v-else v-model="form.apiKey" :type="showKey?'text':'password'" autocomplete="new-password" spellcheck="false"/><button class="button ghost compact model-key-toggle" type="button" :aria-label="showKey?tr('隐藏 Key','Hide key'):tr('显示 Key','Show key')" @click="showKey=!showKey"><EyeOff v-if="showKey" :size="16"/><Eye v-else :size="16"/></button></div></label>
            </template>
          </div>
          <div v-if="selectingOAuth" class="oauth-account">
            <div class="oauth-account-heading"><strong>{{tr('Grok 账号','Grok account')}}</strong><span class="model-state" :class="{ready:oauthSaved}">{{oauthSaved?tr('已登录','Signed in'):tr('未登录','Not signed in')}}</span></div>
            <p class="helper">{{tr('登录后，Grok 与其他模型一样可用于聊天、资产任务和 Nest。','After sign-in, Grok is available to chat, asset tasks and Nest like any other model.')}}</p>
            <div class="model-actions"><button class="button ghost compact" :disabled="!client || busy" @click="authorizeOAuth">{{oauthSaved?tr('重新登录','Sign in again'):tr('登录 Grok','Sign in to Grok')}}</button><button class="button ghost compact" :disabled="!client || busy" @click="checkOAuth">{{tr('检查登录','Check sign-in')}}</button><button v-if="oauthSaved" class="text-button" :disabled="!client || busy" @click="logoutOAuth">{{tr('退出登录','Sign out')}}</button></div>
            <p v-if="oauth?.userCode || oauth?.oauth?.userCode" class="helper">{{tr('登录码','Sign-in code')}}: <code>{{oauth.userCode || oauth.oauth?.userCode}}</code></p><p v-if="oauth?.status" class="helper">{{oauth.status}}</p>
          </div>
          <p v-if="selectingUnboundHost" class="helper model-inline-note">{{tr('请填写一次 Key，将已有连接登记到共用目录。','Enter the key once to register the existing connection in the shared catalog.')}}</p>
          <label class="model-default-toggle"><input v-model="makeDefault" type="checkbox" :disabled="isCurrentDefault"/><span>{{tr('设为默认模型','Set as default model')}}<small>{{tr('聊天、资产任务和 Nest 共用；已有任务保留启动时选择。','Shared by chat, asset tasks and Nest. Existing tasks keep their original selection.')}}</small></span></label>
        </fieldset>
        <footer class="model-editor-footer"><p class="helper">{{selectingOAuth?tr('登录凭据由本机模型服务管理。','Sign-in credentials are managed by the local model service.'):tr('新增或修改连接后先测试，测试会调用模型服务。','Test new or changed connections before saving. Testing calls the model service.')}}</p><div class="model-actions"><button v-if="!selectingOAuth" class="button ghost compact" :disabled="locked || loading || testBusy" @click="testConnection">{{testBusy?tr('测试中…','Testing…'):tr('测试连接','Test connection')}}</button><button class="button primary compact" :disabled="locked || loading || testBusy || selectingOAuth && !oauthSaved" @click="saveConnection">{{makeDefault?tr('保存并设为默认','Save as default'):tr('保存连接','Save connection')}}</button></div></footer>
        <p v-if="testedMessage" class="model-test-result" role="status">{{testedMessage}}</p>
        <div v-if="steps.rust || steps.default" class="connection-results" data-testid="model-write-results"><p v-if="steps.rust">{{steps.rust}}</p><p v-if="steps.default">{{steps.default}}</p></div>
        <div v-if="workflow" class="connection-recovery"><p class="helper">{{workflow.rustDone?tr('连接已保存，继续同步默认模型即可。','Connection saved. Continue default synchronization.'):tr('保存结果待确认，可重试原请求。','Save confirmation pending. Retry the original request.')}}</p><div class="model-actions"><button v-if="workflow.rustDone" class="button primary compact" :disabled="busy" @click="retryDefault">{{tr('重试默认同步','Retry synchronization')}}</button><button v-else class="button primary compact" :disabled="busy" @click="continueWorkflow()">{{tr('重试保存','Retry save')}}</button><button v-if="workflow.rustDone" class="button ghost compact" :disabled="busy" @click="workflow=undefined;loadDirectory()">{{tr('保留连接并重新读取','Keep connection and reload')}}</button></div></div>
      </div>
    </div>
    <p v-if="hostError" class="form-error" role="status">{{hostError}}</p><p v-if="notice" class="model-notice" role="status">{{notice}}</p><p v-if="error" class="form-error" role="alert">{{error}}</p>
    <details class="model-advanced"><summary>{{tr('凭据与网络','Credentials and networking')}}</summary><div class="model-advanced-content">
      <label class="model-default-toggle" data-testid="keychain-toggle"><input type="checkbox" :checked="keychainEnabled" :disabled="keychainBusy" @change="toggleKeychain(($event.target as HTMLInputElement).checked)"/><span>{{tr('将 API Key 保存到系统钥匙串','Store API keys in the system keychain')}}<small>{{tr('数据库中仍保留一份，以便回退。','A copy remains in the database for fallback.')}}</small></span></label>
      <p class="helper">{{tr('此处设置聊天与 OAuth 模型服务的网络；API 模型任务沿用业务网络设置。','Networking for chat and OAuth model service. API model tasks use business network settings.')}}</p><div class="model-fields"><label class="field"><span>{{tr('网络模式','Network mode')}}</span><select v-model="networkMode"><option value="system">{{tr('系统 / 环境','System / environment')}}</option><option value="direct">{{tr('直连','Direct')}}</option><option value="manual">{{tr('手动代理','Manual proxy')}}</option></select></label><label class="field"><span>{{tr('绕过代理','Proxy bypass')}}</span><input v-model="networkNoProxy"/></label><label v-if="networkMode==='manual'" class="field model-full"><span>{{tr('HTTP(S) 代理（留空保留原值）','HTTP(S) proxy (blank keeps saved value)')}}</span><input v-model="networkProxy" type="password" autocomplete="new-password"/></label></div><div class="model-actions"><button class="button ghost compact" :disabled="!client || advancedBusy || locked" @click="saveAdvanced">{{tr('保存网络','Save networking')}}</button></div>
      <div class="model-fields"><label v-if="host" class="field"><span>{{tr('聊天执行方式','Chat runtime')}}</span><select :value="host.runtime.id" :disabled="!client || locked" @change="client?.setRuntime(($event.target as HTMLSelectElement).value as 'none'|'xai-api'|'grok-acp').then(dispatchHostSettings).catch(reason=>error=String(reason))"><option value="none">{{tr('仅本地消息','Local messages only')}}</option><option value="xai-api">{{tr('使用默认模型','Use default model')}}</option><option value="grok-acp">Grok CLI / ACP</option></select></label><label v-if="host" class="field"><span>{{tr('回答模式','Answer mode')}}</span><select :value="host.answerMode || 'quality'" :disabled="!client || locked" @change="client?.setAnswerMode(($event.target as HTMLSelectElement).value === 'speed' ? 'speed' : 'quality').then(dispatchHostSettings).catch(reason=>error=String(reason))"><option value="quality">{{tr('质量','Quality')}}</option><option value="speed">{{tr('速度','Speed')}}</option></select></label></div>
    </div></details>
    <details class="model-advanced"><summary>{{tr('历史来源与保存恢复','Historical sources and recovery')}}</summary><div class="model-advanced-content"><div v-if="profiles.length" class="model-history-row"><label class="field"><span>{{tr('历史模型来源','Historical model source')}}</span><select :value="legacySource?.id" :disabled="locked" @change="legacyId=Number(($event.target as HTMLSelectElement).value)"><option v-for="source in profiles" :key="source.id" :value="source.id">{{source.name}}</option></select></label><button class="button ghost compact" :disabled="locked" @click="importLegacy">{{tr('载入模型','Load models')}}</button></div><div class="model-history-row"><label class="field"><span>{{tr('恢复请求编号','Recovery request id')}}</span><input v-model="recoveryId" maxlength="36" :readonly="Boolean(workflow)"/></label><button class="button ghost compact" :disabled="busy || !recoveryId.trim()" @click="recoverRust">{{tr('查询保存结果','Check saved result')}}</button></div></div></details>
  </section>
</template>

<style scoped>
.models-settings{min-width:0;container:models-settings / inline-size;font-size:var(--text-body);color:var(--ui-ink)}
.models-settings .helper{margin:0;color:var(--ui-muted);font-size:var(--text-meta);line-height:1.6;overflow-wrap:anywhere}
.models-overview{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;margin-bottom:16px}.models-overview>div{min-width:0}.models-intro{margin:0 0 4px;font-size:var(--text-body);font-weight:600}.models-overview>.button{flex-shrink:0}
.models-settings .button.compact{height:36px;min-height:36px;padding:0 12px;font-size:var(--text-meta);box-shadow:none;white-space:nowrap}.models-settings button:disabled{cursor:not-allowed}.models-settings button:focus-visible,.models-settings summary:focus-visible{outline:2px solid var(--blue);outline-offset:3px}
.model-default-summary{display:flex;align-items:center;gap:10px;padding:10px 12px;margin-bottom:16px;background:var(--ui-surface-raised);border:1px solid var(--ui-line);border-radius:var(--radius-md);font-size:var(--text-meta);flex-wrap:wrap}.model-default-summary>span:first-child{display:flex;align-items:center;gap:10px;min-width:0;color:var(--ui-muted)}.model-default-summary strong{font-weight:600;color:var(--ui-ink);overflow-wrap:anywhere}.model-state{font-size:var(--text-micro);color:var(--ui-muted);border-radius:var(--radius-xs);padding:3px 7px;background:var(--ui-surface);white-space:nowrap}.model-state.ready{color:var(--green);background:color-mix(in srgb,var(--green) 9%,var(--ui-surface))}
.models-layout{display:grid;grid-template-columns:220px minmax(0,1fr);gap:16px;align-items:start}.model-directory{min-width:0;display:flex;flex-direction:column;gap:10px}.directory-heading{display:flex;align-items:center;justify-content:space-between;font-size:var(--text-meta);font-weight:600;padding:0 2px}.directory-heading small{font-weight:400;color:var(--ui-muted)}.model-directory-list{display:flex;flex-direction:column;gap:8px}.connection-row{width:100%;min-width:0;display:flex;flex-direction:column;gap:5px;text-align:left;background:var(--ui-surface);border:1px solid var(--ui-line);padding:12px;border-radius:var(--radius-md);color:inherit;cursor:pointer}.connection-row:hover{background:var(--ui-surface-raised)}.connection-row.active{border-color:var(--blue);background:color-mix(in srgb,var(--blue) 6%,var(--ui-surface))}.connection-title{display:flex;align-items:flex-start;justify-content:space-between;gap:8px;width:100%;min-width:0}.connection-row strong{font-size:var(--text-body);font-weight:600;overflow-wrap:anywhere}.connection-row small{font-size:var(--text-meta);line-height:1.5;color:var(--ui-muted);overflow-wrap:anywhere}.connection-model-id{display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}.connection-auth{font-size:var(--text-micro);color:var(--ui-muted)}.connection-row em{font-size:var(--text-micro);font-style:normal;background:var(--ui-surface);padding:2px 6px;border-radius:var(--radius-xs);white-space:nowrap;flex-shrink:0}
.models-settings .model-editor{padding:16px;border:1px solid var(--ui-line);background:var(--ui-surface);border-radius:var(--radius-lg);min-width:0;box-shadow:none}.model-editor-heading{display:flex;align-items:flex-start;justify-content:space-between;gap:12px;margin-bottom:16px}.model-editor-heading h3{font-size:var(--text-heading);margin:0 0 4px;line-height:1.4}.model-auth-badge{font-size:var(--text-micro);padding:4px 7px;background:var(--ui-surface-raised);border:1px solid var(--ui-line);border-radius:var(--radius-xs)}.model-editor fieldset{border:0;padding:0;margin:0;min-width:0}.model-fields{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px 16px}.model-full{grid-column:1/-1}.models-settings .field{display:flex;flex-direction:column;gap:6px;min-width:0;margin:0}.models-settings .field>span{font-size:var(--text-meta);font-weight:500;color:var(--ui-muted);line-height:1.4}.models-settings .field input,.models-settings .field select{box-sizing:border-box;height:38px;min-height:38px;width:100%;max-width:100%;min-width:0;padding:0 10px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-ink);font-size:var(--text-body);box-shadow:none}.models-settings .field input:focus,.models-settings .field select:focus{border-color:var(--blue);outline:2px solid color-mix(in srgb,var(--blue) 18%,transparent);outline-offset:1px}.secret-input{display:flex;gap:6px;min-width:0}.secret-input input{flex:1;min-width:0}.models-settings .model-key-toggle{width:38px;height:38px;min-height:38px;flex:0 0 38px;padding:0}.model-default-toggle{display:flex;align-items:flex-start;gap:9px;margin-top:16px;padding:12px;background:var(--ui-surface-raised);border:1px solid var(--ui-line);border-radius:var(--radius-md);font-size:var(--text-meta)}.model-default-toggle input{width:15px;height:15px;flex:0 0 15px;margin:2px 0 0;accent-color:var(--blue)}.model-default-toggle>span{min-width:0;line-height:1.5;font-weight:500}.model-default-toggle small{display:block;color:var(--ui-muted);font-size:var(--text-micro);font-weight:400;margin-top:3px;line-height:1.5}.model-editor-footer{display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap;border-top:1px solid var(--ui-line);padding-top:14px;margin-top:16px}.model-editor-footer>.helper{flex:1 1 220px}.model-actions{display:flex;gap:8px;flex-wrap:wrap;align-items:center}.oauth-account{padding:12px;margin-top:14px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised)}.oauth-account-heading{display:flex;align-items:center;justify-content:space-between;gap:10px;margin-bottom:6px;font-size:var(--text-body)}.oauth-account .model-actions{margin-top:10px}.oauth-account .helper+.helper{margin-top:8px}.model-inline-note{margin-top:10px!important}
.model-test-result,.connection-results,.connection-recovery,.model-notice{padding:10px 12px;margin:12px 0 0;background:var(--ui-surface-raised);border:1px solid var(--ui-line);border-radius:var(--radius-md);font-size:var(--text-meta);line-height:1.6;overflow-wrap:anywhere}.model-test-result{color:var(--green)}.connection-results p{margin:2px 0}.connection-recovery .model-actions{margin-top:10px}.models-settings .form-error{margin:12px 0 0;font-size:var(--text-meta);line-height:1.6;overflow-wrap:anywhere}.model-advanced{margin-top:12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface);overflow:hidden}.model-advanced summary{cursor:pointer;font-size:var(--text-meta);font-weight:500;padding:12px 14px;line-height:1.5}.model-advanced-content{display:flex;flex-direction:column;gap:14px;padding:0 14px 14px}.model-advanced-content .model-default-toggle{margin:0}.model-history-row{display:flex;align-items:flex-end;gap:10px}.model-history-row>.field{flex:1;min-width:0}
@container models-settings (max-width:760px){.models-layout{grid-template-columns:1fr}.model-directory{display:grid;grid-template-columns:1fr auto;align-items:center}.model-directory-list{grid-column:1/-1;display:grid;grid-template-columns:repeat(2,minmax(0,1fr));max-height:280px;overflow:auto}.connection-row{height:100%}.model-editor-footer{align-items:flex-start}.models-overview{gap:10px}}
@container models-settings (max-width:520px){.models-settings .model-editor{padding:12px}.model-fields{grid-template-columns:1fr;gap:12px}.model-directory-list{grid-template-columns:1fr}.model-editor-footer{gap:12px}.model-editor-footer>.model-actions{width:100%}.model-editor-footer>.model-actions>.button{flex:1}.model-history-row{flex-direction:column;align-items:stretch}.model-default-summary>span:first-child{flex-wrap:wrap;gap:4px 10px}.model-default-toggle{padding:10px}}
</style>
