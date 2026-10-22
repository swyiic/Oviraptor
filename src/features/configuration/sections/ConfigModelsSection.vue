<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { Eye, EyeOff, Plus, RefreshCw } from "@lucide/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
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
const useChat = ref(false), useScan = ref(true), testBusy = ref(false), testedSignature = ref("");
const testedMessage = ref(""), recoveryId = ref(""), oauth = ref<AgentOAuthStartResult>();
const networkMode = ref<AgentNetworkMode>("system"), networkProxy = ref(""), networkNoProxy = ref("");
const advancedBusy = ref(false), legacyId = ref<number>();
let client: HostSettingsClient | null = null, generation = 0, disposed = false;
onUnmounted(() => { disposed = true; generation++; });
type BoundProvider = NonNullable<AgentHostSettings["apiProvider"]> & { authType?: "api-key" | "none"; connectionRef?: ModelConnectionBinding | null };
const provider = computed(() => host.value?.apiProvider as BoundProvider | undefined);
const selected = computed(() => connections.value.find(connection => connection.key === selectedKey.value));
const selectingOAuth = computed(() => selectedKey.value === "grok-oauth");
const selectingUnboundHost = computed(() => selectedKey.value === "host-unbound");
const hostChatEnabled = computed(() => host.value?.runtime.id === "xai-api");
const oauthActive = computed(() => Boolean(hostChatEnabled.value && provider.value?.oauthConfigured));
const oauthSaved = computed(() => Boolean(provider.value?.oauthSaved || provider.value?.oauthConfigured));
const hasUnboundHost = computed(() => Boolean(provider.value?.configured && !provider.value.oauthConfigured && !provider.value.connectionRef));
const scanConnection = computed(() => connections.value.find(connection => category.value && sameCategoryReference(connection.reference, category.value.reference) && connection.profile.id === category.value.settings.activeModelProfileId));
const wasScan = computed(() => !!selected.value && selected.value.key === scanConnection.value?.key);
const wasChat = computed(() => selectingOAuth.value ? oauthActive.value : selectingUnboundHost.value ? hostChatEnabled.value && hasUnboundHost.value : Boolean(selected.value && hostChatEnabled.value && matchesModelBinding(provider.value?.connectionRef, selected.value)));
const legacySource = computed(() => props.profiles.find(profile => profile.id === legacyId.value) ?? props.profiles.find(profile => profile.id === category.value?.legacyProfileId) ?? props.profiles[0]);

type Workflow = {
  profile: AgentModelProfile; reference: ConfigPolicyReference | null; connection: ModelDirectoryConnection | undefined;
  bindScan: boolean; bindChat: boolean; removeScan: boolean; removeChat: boolean; priorHostIdentity: string;
  rustDone: boolean; scanDone: boolean; hostDone: boolean;
  rustRequest?: CategoryPendingWrite; scanRequest?: CategoryPendingWrite;
};
const workflow = ref<Workflow>();
const locked = computed(() => busy.value || Boolean(workflow.value));
const steps = reactive({ rust: "", scan: "", chat: "" });
function isCurrent(request: number) { return !disposed && request === generation; }
function dispatchHostSettings(value: AgentHostSettings) {
  host.value = value;
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("oviraptor:agent-host-settings"));
}
function hostIdentity(value: AgentHostSettings | undefined) { const provider = value?.apiProvider; return JSON.stringify([value?.runtime.id, provider?.provider, provider?.baseUrl, provider?.model, provider?.oauthConfigured, provider?.connectionRef ?? null]); }
function assignDraft(profile: AgentModelProfile) { Object.assign(form, cloneConfigProfileSettings(profile)); testedSignature.value = ""; testedMessage.value = ""; showKey.value = false; }
function selectConnection(key: string, force = false) {
  if (locked.value && !force) return;
  selectedKey.value = key; error.value = ""; notice.value = "";
  const connection = connections.value.find(item => item.key === key);
  if (connection) { assignDraft(connection.profile); useScan.value = connection.key === scanConnection.value?.key; useChat.value = Boolean(hostChatEnabled.value && matchesModelBinding(provider.value?.connectionRef, connection)); }
  else if (key === "host-unbound") { assignDraft({ ...createAgentModelProfile(), name: provider.value?.model || "Host API", llm: provider.value?.model || "", apiBase: provider.value?.baseUrl || "", deployment: provider.value?.authType === "none" ? "local" : "cloud" }); useChat.value = hostChatEnabled.value; useScan.value = false; }
  else if (key === "grok-oauth") { useChat.value = oauthActive.value; useScan.value = false; }
  else { assignDraft(createAgentModelProfile(tr("新模型连接", "New model connection"))); useChat.value = false; useScan.value = true; }
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
      } else hostError.value = tr("聊天 Host 未连接。模型目录与扫描配置仍可保存，聊天用途需连接后再应用。", "Chat Host is unavailable. The model catalog and scan settings remain available; apply chat usage after reconnecting.");
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
      selectConnection(connections.value.find(connection => matchesModelBinding(preferred, connection))?.key ?? chat?.key ?? scanConnection.value?.key ?? (oauthActive.value ? "grok-oauth" : connections.value[0]?.key) ?? "new-api", true);
    }
  } catch (reason) { if (isCurrent(request)) error.value = String(reason); }
  finally { if (isCurrent(request)) loading.value = false; }
}
function validateConnectionFields() {
  if (!form.name.trim() || !form.llm.trim() || !form.apiBase.trim()) throw new Error(tr("请填写连接名称、模型 ID 和明确的 API 地址。", "Enter a connection name, model id and explicit API endpoint."));
  const url = new URL(form.apiBase.trim());
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw new Error(tr("API 地址必须是无内嵌凭据的 HTTP(S) 地址。", "Use an HTTP(S) endpoint without embedded credentials."));
  if (form.deployment === "cloud" && !form.apiKey.trim()) throw new Error(tr("云端 API 连接需要明确填写 Key。Host 中已保存的 Key 不会被导出或暗借给扫描。", "Cloud API connections require an explicit key. Saved Host keys are not exported or borrowed for scans."));
}
function validateDraft() {
  validateConnectionFields();
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
  error.value = ""; notice.value = ""; steps.rust = ""; steps.scan = ""; steps.chat = "";
  try {
    validateDraft();
    const connection = selected.value, profile = cloneConfigProfileSettings(form);
    const base = connection?.reference ? connection.category : category.value;
    const sourceSettings = connection?.reference ? connection.category.settings : category.value.builtinSettings;
    const settings = configPolicyPreviewSettings(modelSettingsForConnection(sourceSettings, profile, useScan.value), "models");
    const needsWrite = !connection?.reference || JSON.stringify(settings) !== JSON.stringify(configPolicyPreviewSettings(connection.category.settings, "models"));
    const intent: Workflow = { profile, reference: connection?.reference ?? null, connection, bindScan: useScan.value, bindChat: useChat.value, removeScan: wasScan.value && !useScan.value, removeChat: wasChat.value && !useChat.value, priorHostIdentity: hostIdentity(host.value), rustDone: !needsWrite, scanDone: false, hostDone: false };
    if (needsWrite) intent.rustRequest = { command: "save", input: {
      requestId: crypto.randomUUID(), section: "models", mode: !connection?.reference ? "create" : categoryReferenceIsHead(connection.category) ? "revise" : "fork",
      name: connection?.reference && categoryReferenceIsHead(connection.category) ? connection.category.entries.find(entry => entry.reference.policyId === connection.reference?.policyId)?.name ?? "" : profile.name.trim(),
      settings, expectedSettings: cloneConfigProfileSettings(connection?.reference ? base.settings : category.value.settings), expectedReference: connection?.reference ?? null,
      expectedDefault: category.value.defaultSelection, makeDefault: useScan.value,
    } };
    workflow.value = intent; await continueWorkflow();
  } catch (reason) { error.value = String(reason); }
}
async function continueWorkflow(hostOnly = false) {
  const intent = workflow.value;
  if (!intent || busy.value) return;
  const request = generation;
  busy.value = true; error.value = "";
  try {
    assertWorkflowActive(intent, request);
    if (!hostOnly && !intent.rustDone && intent.rustRequest) {
      const result = await sendRust(intent.rustRequest); intent.reference = result.receipt.reference; intent.rustDone = true;
      steps.rust = tr("连接已保存到模型目录。", "Connection saved in the model catalog.");
      if (intent.bindScan) { intent.scanDone = true; steps.scan = tr("扫描默认变更已确认；当前状态以重新读取为准。", "Scan-default update acknowledged; the reloaded state is authoritative."); }
      assertWorkflowActive(intent, request);
    } else if (intent.rustDone && !steps.rust) steps.rust = tr("使用目录中已保存的连接，无需重复新建。", "Using a saved catalog connection; no duplicate creation.");
    if (!hostOnly && !intent.scanDone) {
      if ((intent.bindScan || intent.removeScan) && category.value) {
        intent.scanRequest ??= { command: "select", input: { requestId: crypto.randomUUID(), section: "models", reference: intent.removeScan ? null : intent.reference, expectedDefault: category.value.defaultSelection } };
        await sendRust(intent.scanRequest);
        intent.scanDone = true;
        steps.scan = intent.removeScan ? tr("扫描用途已解除，此类回到内置未配置状态。", "Scan usage removed; this category uses its unconfigured built-in state.") : tr("已明确选择扫描默认连接。", "Scan default connection selected explicitly.");
        assertWorkflowActive(intent, request);
      } else steps.scan = tr("其他扫描默认保持不变。", "The other scan default is unchanged.");
      intent.scanDone = true;
    }
    assertWorkflowActive(intent, request);
    await applyHostUsage(intent, request);
    assertWorkflowActive(intent, request);
    intent.hostDone = true; workflow.value = undefined;
    notice.value ||= tr("连接与用途已保存。聊天和扫描的实际状态分别显示在目录中。", "Connection and usage saved. The directory shows the actual chat and scan states separately.");
    await loadDirectory(false, intent.reference ? { ...intent.reference, kind: "models" as const, modelProfileId: intent.profile.id } : undefined); if (!disposed) emit("changed");
  } catch (reason) {
    if (!intent.rustDone && String(reason).startsWith("configuration_category_rejected:")) workflow.value = undefined;
    error.value = String(reason);
  } finally { busy.value = false; }
}
function assertWorkflowActive(intent: Workflow, request: number) {
  if (disposed || generation !== request || workflow.value !== intent) throw new Error(tr("模型页面已离开或重新读取，后续用途操作已停止。已确认的保存结果保留；请重新读取后明确应用用途。", "The model page was left or reloaded. Subsequent usage operations stopped; acknowledged saves are retained. Reload and explicitly apply usage."));
}
async function applyHostUsage(intent: Workflow, request: number) {
  assertWorkflowActive(intent, request);
  if (!intent.bindChat && !intent.removeChat) { steps.chat = tr("聊天选择保持不变。", "Chat selection unchanged."); return; }
  if (!client) throw new Error(tr("Rust 模型已保存；聊天 Host 不可用。连接后仅重试聊天同步，不会重复创建模型。", "The Rust model is saved; Chat Host is unavailable. Reconnect and retry chat synchronization only, without creating another model."));
  if (intent.bindChat) {
    const current = await client.getSettings();
    assertWorkflowActive(intent, request);
    if (hostIdentity(current) !== intent.priorHostIdentity) throw new Error(tr("聊天连接或运行方式已另行改变，未覆盖新的选择。请重新读取后明确应用用途。", "The chat connection or runtime changed elsewhere. The newer selection was not overwritten; reload and explicitly apply usage."));
    if (!intent.reference) throw new Error("configuration_model_reference_unconfirmed");
    const profile = intent.profile;
    const input = {
      provider: "openai-compatible" as const, model: profile.llm.trim(), baseUrl: profile.apiBase.trim(),
      apiKey: profile.deployment === "local" ? profile.localApiKey.trim() : profile.apiKey.trim(),
      authType: profile.deployment === "local" && !profile.localApiKey.trim() ? "none" as const : "api-key" as const,
      clearApiKey: profile.deployment === "local" && !profile.localApiKey.trim(),
      connectionRef: { ...intent.reference, kind: "models" as const, modelProfileId: profile.id },
    };
    const saved = await client.setProvider(input);
    dispatchHostSettings(saved); intent.priorHostIdentity = hostIdentity(saved);
    assertWorkflowActive(intent, request);
    dispatchHostSettings(await client.setRuntime("xai-api"));
    assertWorkflowActive(intent, request);
    steps.chat = tr("已将同一已保存连接明确用于聊天。", "The same saved connection is explicitly selected for chat.");
  } else {
    const current = await client.getSettings();
    assertWorkflowActive(intent, request);
    if (intent.connection ? !matchesModelBinding((current.apiProvider as BoundProvider | undefined)?.connectionRef, intent.connection) : hostIdentity(current) !== intent.priorHostIdentity) throw new Error(tr("聊天连接已另行切换，未停用新的连接。请重新读取后核对用途。", "Chat switched to another connection. The newer connection was not disabled; reload and review usage."));
    dispatchHostSettings(await client.setRuntime("none")); assertWorkflowActive(intent, request); steps.chat = tr("聊天用途已停用，凭据仍保留。", "Chat usage disabled; credentials retained.");
  }
}
async function retryChatOnly() {
  if (!workflow.value?.rustDone || !workflow.value.scanDone || busy.value) return;
  try { if (!client) client = (await openAgentHostSettingsClient())?.client ?? null; await continueWorkflow(true); }
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
      const original = intent.rustDone ? intent.scanRequest : intent.rustRequest;
      if (original) await sendRust(original);
      if (!isCurrent(request) || workflow.value !== intent) return;
      if (!intent.rustDone) { intent.reference = result.receipt.reference; intent.rustDone = true; if (intent.bindScan) intent.scanDone = true; }
      else intent.scanDone = true;
    } else {
      notice.value = tr("Rust 原操作已确认。聊天用途没有自动重放；请选择已保存连接并明确应用用途。", "The original Rust operation is acknowledged. Chat usage was not replayed automatically; select the saved connection and explicitly apply its usage.");
      await loadDirectory();
    }
  } catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
async function authorizeOAuth() {
  if (!client || busy.value) return; busy.value = true; error.value = "";
  try { oauth.value = await client.startOAuth({ background: true }); const uri = oauth.value.verificationUriComplete || oauth.value.oauth?.verificationUriComplete || oauth.value.verificationUri || oauth.value.oauth?.verificationUri; if (uri) await openUrl(uri); }
  catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
async function checkOAuth() {
  if (!client || busy.value) return; busy.value = true; error.value = "";
  try { oauth.value = await client.pollOAuth(); dispatchHostSettings(await client.getSettings()); }
  catch (reason) { error.value = String(reason); }
  finally { busy.value = false; }
}
async function applyOAuth() {
  if (!client || busy.value) return; busy.value = true; error.value = "";
  try {
    if (useChat.value) { if (!oauthSaved.value) throw new Error(tr("请先完成 Grok 登录。", "Complete Grok sign-in first.")); dispatchHostSettings(await client.selectOAuth()); dispatchHostSettings(await client.setRuntime("xai-api")); }
    else if (oauthActive.value) dispatchHostSettings(await client.setRuntime("none"));
    notice.value = tr("OAuth 聊天用途已更新。扫描配置未修改，登录令牌未复制。", "OAuth chat usage updated. Scan settings were not changed and login tokens were not copied."); emit("changed");
  } catch (reason) { error.value = String(reason); }
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
    <p class="helper">{{tr('连接只填写一次，再选择用于聊天、扫描或两者。API 连接保存到模型目录；OAuth 保留真实登录类型与能力限制。','Enter a connection once, then choose chat, scanning, or both. API connections are saved in the model catalog; OAuth retains its actual authentication type and capabilities.')}}</p>
    <div class="models-layout">
      <aside class="model-directory" aria-label="Model connections">
        <button class="button ghost compact add-model" :disabled="locked" @click="selectConnection('new-api')"><Plus :size="15"/>{{tr('添加模型连接','Add model connection')}}</button>
        <button v-for="connection in connections" :key="connection.key" class="connection-row" :class="{active:selectedKey===connection.key}" :disabled="locked" @click="selectConnection(connection.key)"><strong>{{connection.name}}</strong><small>{{connection.profile.llm}} · {{connection.profile.deployment==='local'?tr('本地 / 自建','Local / self-hosted'):'API Key'}}</small><span class="connection-uses"><em v-if="matchesModelBinding(provider?.connectionRef,connection) && hostChatEnabled">{{tr('聊天','Chat')}}</em><em v-if="connection.key===scanConnection?.key">{{tr('扫描默认','Scan default')}}</em><em v-if="connection.historical">{{tr('历史来源','Historical source')}}</em></span></button>
        <button class="connection-row" :class="{active:selectingOAuth}" :disabled="locked" @click="selectConnection('grok-oauth')"><strong>Grok OAuth</strong><small>{{oauthSaved?tr('已保留登录','Sign-in saved'):tr('登录连接','Sign-in connection')}}</small><span class="connection-uses"><em>{{tr('仅聊天','Chat only')}}</em><em v-if="oauthActive">{{tr('当前使用','Active')}}</em></span></button>
        <button v-if="hasUnboundHost" class="connection-row" :class="{active:selectingUnboundHost}" :disabled="locked" @click="selectConnection('host-unbound')"><strong>{{provider?.model || 'Host API'}}</strong><small>{{tr('Host 旧连接，尚未登记目录','Legacy Host connection, not yet registered')}}</small><em v-if="hostChatEnabled">{{tr('聊天','Chat')}}</em></button>
        <button v-refresh-feedback="{busy:loading,error:error||hostError}" class="button ghost compact" :disabled="loading || locked" @click="loadDirectory()"><RefreshCw :size="14"/>{{tr('刷新目录','Refresh catalog')}}</button>
      </aside>
      <div class="model-editor panel">
        <p v-if="loading" class="helper" role="status">{{tr('正在读取模型目录…','Loading model catalog…')}}</p>
        <template v-if="selectingOAuth">
          <h3>Grok OAuth</h3><p class="helper">{{tr('登录由聊天 Host 管理。当前 Rust 扫描需要可直接调用的 API / 本地模型，因此此登录不能勾选扫描用途；令牌不会复制为 API Key。','Chat Host owns this sign-in. Rust scanning currently requires a directly callable API or local model, so this login cannot be selected for scanning. Its token is not copied as an API key.')}}</p>
          <label class="check-field"><input v-model="useChat" type="checkbox" :disabled="busy"/>{{tr('用于聊天','Use for chat')}}</label><label class="check-field"><input type="checkbox" disabled/>{{tr('用于扫描（当前不支持）','Use for scanning (unsupported)')}}</label>
          <div class="hero-actions"><button class="button primary compact" :disabled="!client || busy" @click="authorizeOAuth">{{oauthSaved?tr('重新登录','Sign in again'):tr('登录 Grok','Sign in to Grok')}}</button><button class="button ghost compact" :disabled="!client || busy" @click="checkOAuth">{{tr('检查登录结果','Check sign-in result')}}</button><button class="button primary compact" :disabled="!client || busy || useChat && !oauthSaved" @click="applyOAuth">{{tr('应用聊天用途','Apply chat usage')}}</button><button v-if="oauthSaved" class="button ghost compact" :disabled="busy" @click="logoutOAuth">{{tr('移除保存的登录','Remove saved sign-in')}}</button></div>
          <p v-if="oauth?.userCode || oauth?.oauth?.userCode" class="helper">{{tr('登录码','Sign-in code')}}: <code>{{oauth.userCode || oauth.oauth?.userCode}}</code></p><p v-if="oauth?.status" class="helper">{{oauth.status}}</p>
        </template>
        <template v-else>
          <h3>{{selected ? tr('编辑模型连接','Edit model connection') : selectingUnboundHost ? tr('登记 Host 连接','Register Host connection') : tr('添加模型连接','Add model connection')}}</h3>
          <fieldset :disabled="locked">
            <div class="form-grid two"><label class="field"><span>{{tr('连接名称','Connection name')}}</span><input v-model="form.name" maxlength="120"/></label><label class="field"><span>{{tr('模型 ID','Model id')}}</span><input v-model="form.llm" placeholder="grok-4.7 / deepseek-chat / local model"/></label><label class="field span-two"><span>API URL</span><input v-model="form.apiBase" placeholder="https://api.example.com/v1"/></label><label class="field"><span>{{tr('认证类型','Authentication type')}}</span><select v-model="form.deployment"><option value="cloud">{{tr('云端 API Key','Cloud API key')}}</option><option value="local">{{tr('本地 / 自建，Key 可选','Local / self-hosted, optional key')}}</option></select></label><label class="field"><span>{{form.deployment==='local'?tr('本地 Key（可留空）','Local key (optional)'):'API Key'}}</span><div class="secret-input"><input v-if="form.deployment==='local'" v-model="form.localApiKey" :type="showKey?'text':'password'" autocomplete="new-password"/><input v-else v-model="form.apiKey" :type="showKey?'text':'password'" autocomplete="new-password"/><button class="button ghost compact" type="button" :aria-label="showKey?tr('隐藏 Key','Hide key'):tr('显示 Key','Show key')" @click="showKey=!showKey"><EyeOff v-if="showKey" :size="16"/><Eye v-else :size="16"/></button></div></label></div>
            <p v-if="selectingUnboundHost" class="helper">{{tr('Host 已保存的 Key 不会读取或导出。登记为目录连接时请明确填写一次 Key；本地无鉴权连接会显式清除旧鉴权。','Saved Host keys are not read or exported. Enter a key explicitly to register this catalog connection. An unauthenticated local connection explicitly removes old authentication.')}}</p>
            <div class="model-usage"><strong>{{tr('用途','Usage')}}</strong><label class="check-field"><input v-model="useChat" type="checkbox"/>{{tr('聊天','Chat')}}</label><label class="check-field"><input v-model="useScan" type="checkbox"/>{{tr('扫描默认','Scan default')}}</label></div>
          </fieldset>
          <p class="helper">{{tr('只保存模型连接与用途，不复制其他分类。新的或已修改连接需测试；测试会请求模型服务，可能产生费用。','Saves only the connection and its usage, without copying other categories. New or changed connections require a model call to test them; the test may incur a charge.')}}</p>
          <div class="hero-actions"><button class="button ghost compact" :disabled="locked || testBusy" @click="testConnection">{{testBusy?tr('测试中…','Testing…'):tr('测试连接','Test connection')}}</button><button class="button primary compact" :disabled="locked || loading || testBusy" @click="saveConnection">{{tr('保存连接与用途','Save connection and usage')}}</button></div>
          <p v-if="testedMessage" class="helper">{{testedMessage}}</p>
        </template>
        <div v-if="steps.rust || steps.scan || steps.chat" class="connection-results" data-testid="model-write-results"><p v-if="steps.rust">{{steps.rust}}</p><p v-if="steps.scan">{{steps.scan}}</p><p v-if="steps.chat">{{steps.chat}}</p></div>
        <div v-if="workflow" class="connection-recovery"><p class="helper">{{tr('按步骤确认实际结果，不会因聊天同步失败再创建一次 Rust 连接。','Results are acknowledged step by step. A chat synchronization failure does not create another Rust connection.')}}</p><button v-if="workflow.rustDone && workflow.scanDone" class="button primary compact" :disabled="busy" @click="retryChatOnly">{{tr('仅重试聊天同步','Retry chat synchronization only')}}</button><button v-else class="button primary compact" :disabled="busy" @click="continueWorkflow()">{{tr('重试未完成的原步骤','Retry the original incomplete step')}}</button><button v-if="workflow.rustDone" class="button ghost compact" :disabled="busy" @click="workflow=undefined;loadDirectory()">{{tr('保留连接，重新核对未完成用途','Keep connection; review incomplete usage')}}</button></div>
      </div>
    </div>
    <p v-if="hostError" class="form-error">{{hostError}}</p>
    <p v-if="notice" class="helper model-notice" role="status">{{notice}}</p><p v-if="error" class="form-error" role="alert">{{error}}</p>
    <details class="model-advanced"><summary>{{tr('聊天执行与网络（高级）','Chat execution and networking (advanced)')}}</summary><p class="helper">{{tr('聊天网络独立于扫描业务网络，不改变模型目录中的认证类型。','Chat networking is independent of scan business networking and does not change catalog authentication types.')}}</p><div class="form-grid two"><label class="field"><span>{{tr('聊天网络模式','Chat network mode')}}</span><select v-model="networkMode"><option value="system">{{tr('系统 / 环境','System / environment')}}</option><option value="direct">{{tr('直连','Direct')}}</option><option value="manual">{{tr('手动代理','Manual proxy')}}</option></select></label><label class="field"><span>{{tr('绕过代理','Proxy bypass')}}</span><input v-model="networkNoProxy"/></label><label v-if="networkMode==='manual'" class="field span-two"><span>{{tr('手动 HTTP(S) 代理（留空保留已保存值）','Manual HTTP(S) proxy (blank keeps saved value)')}}</span><input v-model="networkProxy" type="password" autocomplete="new-password"/></label></div><button class="button ghost compact" :disabled="!client || advancedBusy || locked" @click="saveAdvanced">{{tr('保存聊天网络','Save chat networking')}}</button><label v-if="host" class="field"><span>{{tr('聊天执行方式（高级）','Chat runtime (advanced)')}}</span><select :value="host.runtime.id" :disabled="!client || locked" @change="client?.setRuntime(($event.target as HTMLSelectElement).value as 'none'|'xai-api'|'grok-acp').then(dispatchHostSettings).catch(reason=>error=String(reason))"><option value="none">{{tr('仅本地消息','Local messages only')}}</option><option value="xai-api">{{tr('使用选定模型连接','Use selected model connection')}}</option><option value="grok-acp">Grok CLI / ACP</option></select></label><label v-if="host" class="field"><span>{{tr('回答模式','Answer mode')}}</span><select :value="host.answerMode || 'quality'" :disabled="!client || locked" @change="client?.setAnswerMode(($event.target as HTMLSelectElement).value === 'speed' ? 'speed' : 'quality').then(dispatchHostSettings).catch(reason=>error=String(reason))"><option value="quality">{{tr('质量','Quality')}}</option><option value="speed">{{tr('速度','Speed')}}</option></select></label></details>
    <details class="model-advanced"><summary>{{tr('历史来源与模型回执','Historical sources and model receipts')}}</summary><label v-if="profiles.length" class="field"><span>{{tr('显式载入历史模型','Explicitly load historical models')}}</span><select :value="legacySource?.id" :disabled="locked" @change="legacyId=Number(($event.target as HTMLSelectElement).value)"><option v-for="source in profiles" :key="source.id" :value="source.id">{{source.name}}</option></select></label><button v-if="profiles.length" class="button ghost compact" :disabled="locked" @click="importLegacy">{{tr('仅载入模型连接','Load model connections only')}}</button><label class="field"><span>{{tr('恢复请求编号（不含 Key）','Recovery request id (no key)')}}</span><input v-model="recoveryId" maxlength="36" :readonly="Boolean(workflow)"/></label><button class="button ghost compact" :disabled="busy || !recoveryId.trim()" @click="recoverRust">{{tr('查询 Rust 原写入结果','Look up original Rust write')}}</button></details>
  </section>
</template>

<style scoped>
.models-settings{font-size:13px;min-width:0}.models-settings .helper{font-size:12px;line-height:1.7}.models-layout{display:grid;grid-template-columns:minmax(180px,240px) minmax(0,1fr);gap:18px;margin-top:18px}.model-directory{display:flex;flex-direction:column;gap:8px}.connection-row{display:flex;flex-direction:column;align-items:flex-start;gap:5px;text-align:left;background:var(--app-bg,#f7f9fc);border:1px solid var(--border,#dce2eb);padding:12px;border-radius:8px;color:inherit}.connection-row.active{border-color:var(--blue,#2878ff);background:var(--app-selected,#edf4ff)}.connection-row strong{font-size:13px}.connection-row small{font-size:11px;color:var(--muted,#64748b);overflow-wrap:anywhere}.connection-uses{display:flex;gap:6px;flex-wrap:wrap}.connection-row em{font-size:10px;font-style:normal;background:var(--app-bg,#f1f5f9);padding:2px 6px;border-radius:4px}.model-editor{padding:20px;min-width:0}.model-editor h3{font-size:15px;margin:0 0 16px}.model-editor fieldset{border:0;padding:0;margin:0;min-width:0}.secret-input{display:flex;gap:6px}.secret-input input{flex:1;min-width:0}.model-usage{display:flex;gap:18px;align-items:center;margin:18px 0}.model-usage strong{font-size:12px}.model-advanced{margin-top:20px;border-top:1px solid var(--border,#dce2eb);padding-top:16px}.model-advanced summary{cursor:pointer;font-size:12px;margin-bottom:14px}.model-advanced>.field{margin:14px 0}.connection-results,.connection-recovery,.model-notice{padding:12px;margin-top:16px;background:var(--app-bg,#f5f7fa);border-radius:8px}.connection-results p{font-size:12px;margin:4px 0}.connection-recovery button+button{margin-left:8px}@media(max-width:850px){.models-layout{grid-template-columns:1fr}.model-directory{display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr))}.model-usage{flex-wrap:wrap}}
</style>
