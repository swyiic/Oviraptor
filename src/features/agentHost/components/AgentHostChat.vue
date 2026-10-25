<script setup lang="ts">
import { refreshFeedback as vRefreshFeedback } from '../../../shared/directives/refreshFeedback';
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { invoke } from '@tauri-apps/api/core';
import { Plus, Square, ArrowUp, ChevronDown, Copy, Check, Pencil, CornerDownRight, ShieldCheck, Search } from "@lucide/vue";
import { providerDisplayName, SESSION_MODEL_PRESETS } from "../providerPresets";
import { useI18n } from "../../../i18n";
import type { AgentHostSettings, AgentTask } from "../types";
import { describeAgentHostError } from "../errors";
import { agentRuntimeHeader, type AgentHostChatClient, type AgentHostConversation, type AgentHostMessage, type AgentModelUsage } from "../chatClient";
import ConversationMessage from "../../conversations/ConversationMessage.vue";
import AgentPermissionMode from "./AgentPermissionMode.vue";
import AgentTaskCollaboration from "./AgentTaskCollaboration.vue";
import DiscoveryAssistPanel from "./DiscoveryAssistPanel.vue";
import type { DiscoveryChatRequest } from "../../assets/discoveryAnalysis";
import ConversationFinder from '../../conversations/ConversationFinder.vue';
import type { AgentHostEvent } from "../chatClient";
import ConversationHistoryList from './ConversationHistoryList.vue';
import { sortConversationHistory } from '../conversationHistory';
import { useConversationFiles } from '../useConversationFiles';
import ConversationFiles from './ConversationFiles.vue';

const props = defineProps<{ client: AgentHostChatClient; settings: AgentHostSettings; projectName: string; projectId?: number; historySearchRequest?:number; draftRequest?:DiscoveryChatRequest }>();
const emit = defineEmits<{ draftApplied:[id:number]; settingsChanged: [settings: AgentHostSettings] }>();
const { tr } = useI18n();

const hostProjectId = ref<number>();
const conversations = ref<AgentHostConversation[]>([]);
const selectedId = ref<string>();
const historyQuery = ref("");
const finderOpen = ref(false);
const visibleConversations = computed(() => sortConversationHistory(conversations.value).filter(item => conversationLabel(item.title).toLocaleLowerCase().includes(historyQuery.value.trim().toLocaleLowerCase())));
const recentConversations=computed(()=>{
  const recent=visibleConversations.value.slice(0,20),selected=visibleConversations.value.find(item=>item.id===selectedId.value);
  return selected&&!recent.some(item=>item.id===selected.id)?[selected,...recent.slice(0,19)]:recent;
});
const finderConversations=computed(()=>sortConversationHistory(conversations.value).map(item=>({...item,title:conversationLabel(item.title)})));
function findConversation(id:string){if(conversations.value.some(item=>item.id===id)){selectedId.value=id;historyQuery.value='';}}
const permissionsOpen = ref(false);
const permissionsLoading = ref(false);
const permissionsNotice = ref("");
const savedFilePermissions = ref<string[]>([]);
const workspaceFullAccess=ref(false);
const savedAppPermissions = ref<Array<{ appId: string; labels: string[]; needsDisclosure: boolean }>>([]);
let permissionGeneration = 0;
const messages = ref<AgentHostMessage[]>([]);
const conversationFiles = useConversationFiles({ client: () => props.client, conversationId: () => selectedId.value, projectId: () => hostProjectId.value,
  refreshKey: () => `${messages.value[messages.value.length - 1]?.id || ''}:${messages.value[messages.value.length - 1]?.status || ''}` });
const steps = ref<string[]>([]);
const task = ref<AgentTask | null>(null);
const continuingTask = ref(false);
const cancellingChildren = ref<string[]>([]);
const taskLabel = (status: string) => ({ running: tr('执行中', 'Running'), interrupted: tr('已中断', 'Interrupted'), paused_budget: tr('请求额度暂停', 'Request budget paused'), awaiting_input: tr('等待继续或验收', 'Awaiting continuation or acceptance'), completed: tr('已记录完成', 'Completion recorded'), pending: tr('待处理', 'Pending'), in_progress: tr('进行中', 'In progress'), blocked: tr('受阻', 'Blocked'), failed: tr('失败', 'Failed') } as Record<string, string>)[status] || status;
const readOnlyAnswered = computed(() => {
  const view = task.value?.readOnlyAnswer, last = messages.value[messages.value.length - 1];
  return !running.value && view?.kind === "read_only_answer" && task.value?.conversationId === selectedId.value
    && task.value?.projectId === hostProjectId.value && last?.id === view.assistantMessageId
    && last.runId === view.runId && last.status === "completed" && last.stopReason === "end_turn"
    && !!last.content.trim() && messages.value.some(item => item.id === view.userMessageId && item.role === "user");
});
const taskDisplayGoal = computed(() => readOnlyAnswered.value
  ? messages.value.find(item => item.id === task.value?.readOnlyAnswer?.userMessageId)?.content || task.value?.goal
  : task.value?.goal);
type InnerView = { goal: string; phase: string; phaseLabel: string; facts: string[]; hypotheses: string[]; compressed: boolean; hold: string };
const inner = ref<InnerView | null>(null);
const activityRunId = ref<string>(), activityMessageId = ref<string>();
const activityOpen = ref(false);
function toggleActivity(event: Event) {
  const element = event.target as HTMLDetailsElement;
  activityOpen.value = element.open;
  if (element.open) void nextTick(() => {
    // Only an explicit user expansion reveals the details. Streaming updates
    // do not move the transcript or reopen a user-collapsed process.
    if (element.open && element.isConnected !== false) element.scrollIntoView?.({ block: 'nearest' });
  });
}
const activityMessage = computed(() => {
  if (activityMessageId.value || activityRunId.value) return messages.value.find(message => message.role === 'assistant'
    && (activityMessageId.value ? message.id === activityMessageId.value : message.runId === activityRunId.value));
  // SSE can reconnect after a tool wave or the App can reopen after completion.
  // Reconstruct only this reply's public activity from its persisted receipts.
  const latest = [...messages.value].reverse().find(message => message.role === 'assistant');
  return latest?.runId && task.value?.evidence.some(item => item.runId === latest.runId) ? latest : undefined;
});
const activityEvidence = computed(() => activityMessage.value?.runId
  ? task.value?.evidence.filter(item => item.runId === activityMessage.value?.runId).slice(-12) || [] : []);
const activityCount = computed(() => steps.value.length || activityEvidence.value.length);
function resetActivity() {
  inner.value = null; steps.value = []; heartbeat.value = undefined;
  activityRunId.value = undefined; activityMessageId.value = undefined; activityOpen.value = false;
}
/** Match public execution updates to their reply; old runs cannot replace the active process. */
function bindActivity(event: AgentHostEvent) {
  if (event.conversationId !== selectedId.value) return false;
  const current = [...messages.value].reverse().find(message => message.role === 'assistant' && message.status === 'running');
  const target = event.messageId ? messages.value.find(message => message.id === event.messageId)
    : event.runId ? messages.value.find(message => message.role === 'assistant' && message.runId === event.runId) : current;
  if (target && target.role !== 'assistant') return false;
  if (current?.runId && event.runId && current.runId !== event.runId) return false;
  if (current && event.messageId && current.id !== event.messageId) return false;
  if (target && target.status !== 'running' && !(target.id === activityMessageId.value || target.runId && target.runId === activityRunId.value)) return false;
  if (activityRunId.value && event.runId && activityRunId.value !== event.runId) {
    if (!current || target?.id !== current.id) return false;
    resetActivity();
  }
  activityRunId.value = event.runId || target?.runId || activityRunId.value;
  activityMessageId.value = event.messageId || target?.id || activityMessageId.value;
  return !!(activityRunId.value || activityMessageId.value);
}
const draft = ref("");
const conversationDrafts=new Map<string,string>();
let appliedDraftRequest=0,applyingDraftRequest=false;
async function applyDiscoveryDraft(){
 const request=props.draftRequest;if(!request||request.id===appliedDraftRequest||request.projectId!==props.projectId||loadingProject.value||running.value||continuingTask.value||hostProjectId.value===undefined||applyingDraftRequest)return;
 applyingDraftRequest=true;const generation=projectGeneration;
 try{await newConversation();if(disposed||generation!==projectGeneration||props.draftRequest?.id!==request.id||request.projectId!==props.projectId)return;
 draft.value=request.prompt;appliedDraftRequest=request.id;emit("draftApplied",request.id);await nextTick();composerInput.value?.focus();composerInput.value?.setSelectionRange(0,0);if(composerInput.value)composerInput.value.scrollTop=0;
 }catch(e){error.value=String((e as Error).message||e);}finally{applyingDraftRequest=false;if(props.draftRequest&&props.draftRequest.id!==request.id)void nextTick(applyDiscoveryDraft);}
}
const running = ref(false);
const loadingProject = ref(false);
const connected = ref(true);
let projectGeneration = 0;
let messageGeneration = 0;
let disposed = false;
const error = ref("");
const transcript = ref<HTMLElement>();
const followingTail = ref(true), unreadBelow = ref(false), loadingMessages = ref(false), loadingEarlier = ref(false), hasEarlier = ref(false), discoveryPanelOpen = ref(false);
let earlierCursor: string | undefined, historyLoaded = false;
let messageLoad: {id:string;projectId:number;promise:Promise<void>} | undefined;
function observeTranscript(){const el=transcript.value;if(el){followingTail.value=el.scrollHeight-el.scrollTop-el.clientHeight<80;if(followingTail.value)unreadBelow.value=false;}}
function jumpToLatest(){followingTail.value=true;unreadBelow.value=false;void scroll(true);}
async function loadEarlierMessages(){
 const id=selectedId.value,pid=hostProjectId.value,cursor=earlierCursor,el=transcript.value;if(!id||pid===undefined||!cursor||loadingEarlier.value||!props.client.messagePage)return;
 const generation=messageGeneration,oldHeight=el?.scrollHeight??0,oldTop=el?.scrollTop??0;loadingEarlier.value=true;
 try{const page=await props.client.messagePage(id,pid,cursor);if(id!==selectedId.value||pid!==hostProjectId.value||generation!==messageGeneration)return;
 const ids=new Set(messages.value.map(m=>m.id));messages.value=[...page.items.filter(m=>!ids.has(m.id)),...messages.value];earlierCursor=page.nextCursor??undefined;hasEarlier.value=page.hasMore;historyLoaded=true;
 await nextTick();if(el&&id===selectedId.value){el.scrollTop=oldTop+el.scrollHeight-oldHeight;observeTranscript();}}
 catch(e){if(id===selectedId.value&&pid===hostProjectId.value)error.value=String((e as Error).message||e);}
 finally{if(id===selectedId.value&&pid===hostProjectId.value)loadingEarlier.value=false;}
}
const composerInput = ref<HTMLTextAreaElement>();
const heartbeat = ref<{ elapsedMs: number; stepTitle?: string }>();
type GrantOffer = { appId: string; revision: number; permissions: Array<{ permission: string; tools: string[] }> };
const grantPrompt = ref<{ conversationId: string; runId?: string; requestId?: string; projectId: number; identity: string; offers: GrantOffer[]; question: string } | null>(null);
type RuntimePermission = { id: string; conversationId: string; description: string; options: Array<{ id: string; label: string }> };
const runtimePermission = ref<RuntimePermission | null>(null);
const granting = ref(false);
const rememberWorkspace = ref(true);
const pendingApproval = ref<{ id: string; scopeHash: string; choice: string } | null>(null);
const grantIsWorkspace = computed(() => !!grantPrompt.value?.offers.some(offer => offer.appId === "workspace"));
function permissionLabel(id: string) {
  const known: Record<string, string> = {
    "inventory.read": "读取资产",
    "inventory.write": "修改资产",
    "inventory.exposure.read": "读取暴露面",
    "investigation.read": "读取调查",
    "investigation.draft.write": "创建调查草稿",
    "investigation.execute": "执行或恢复扫描（可能产生费用）",
    "investigation.control": "暂停扫描",
    "certificate.collect": "采集范围内的公开证书",
    "legacy.snapshot.read": "读取历史快照",
    "workspace.read": "读取工作区文件",
    "workspace.write": "写入工作区文件",
    "workspace.run": "在工作区运行脚本",
  };
  return known[id] || id;
}
const innerPhase = computed(() => (grantPrompt.value ? "wait" : inner.value?.phase || "model"));
const innerPhaseText = computed(() => {
  const message = activityMessage.value;
  if (message && message.status !== 'running') {
    if (message.status === 'failed') return tr('执行失败', 'Failed');
    if (message.status === 'cancelled') return tr('已停止', 'Stopped');
    if (message.status === 'interrupted') return tr('已中断', 'Interrupted');
    if (message.status === 'refused' || message.stopReason === 'refusal') return tr('已停止', 'Stopped');
    if (message.status === 'completed') return message.stopReason && message.stopReason !== 'end_turn'
      ? tr('本轮已结束', 'Turn ended') : tr('已完成', 'Completed');
  }
  const known: Record<string, [string, string]> = {
    model: ["思考中", "Thinking"],
    tool: ["正在执行工具", "Running a tool"],
    hold: ["正在核对结果", "Checking results"],
    compress: ["正在收起更早的上下文", "Folding older context"],
    wait: ["在等你授权", "Waiting for you"],
    done: ["正在整理回复", "Finishing reply"],
  };
  const pair = known[innerPhase.value];
  return pair ? tr(...pair) : (inner.value?.phaseLabel || "");
});
const heartbeatLabel = computed(() => {
  if (!heartbeat.value) return "";
  const seconds = Math.max(1, Math.round(heartbeat.value.elapsedMs / 1000));
  const step = heartbeat.value.stepTitle ? ` · ${heartbeat.value.stepTitle}` : "";
  return tr(`仍在运行 · 已用 ${seconds}s${step}`, `Still working · ${seconds}s${step}`);
});
const copiedId = ref<string>();
async function copyMessage(message: AgentHostMessage) {
  try { await navigator.clipboard.writeText(message.content); copiedId.value = message.id; setTimeout(() => { if (copiedId.value === message.id) copiedId.value = undefined; }, 1500); }
  catch { /* clipboard unavailable */ }
}
/** Edit & resend: host has no history truncation API, so the edited text is sent as a new message. */
async function editMessage(message: AgentHostMessage) {
  draft.value = message.content; await nextTick(); composerInput.value?.focus();
}
/** Insert a correction into the running turn. The host reads it before the next model step. */
async function steer(text: string) {
  if (!text || !selectedId.value) return;
  await props.client.steer(selectedId.value, text);
}
const liveSettings = ref<AgentHostSettings>(props.settings);
watch(() => props.settings, value => { liveSettings.value = value; });
const header = computed(() => agentRuntimeHeader(liveSettings.value));
const currentModel = computed(() => liveSettings.value?.apiProvider?.model || "");
const modelOptions = computed(() => {
  const oauth = Boolean(liveSettings.value?.apiProvider?.oauthConfigured);
  const list = SESSION_MODEL_PRESETS.map(item => {
    const name = item.provider === "xai"
      ? (oauth && currentModel.value === item.model ? "xAI Grok (OAuth)" : "xAI Grok (API key)")
      : providerDisplayName(item.provider, item.baseUrl);
    return { value: item.model, label: `${name} · ${item.model}` };
  });
  if (currentModel.value && !list.some(item => item.value === currentModel.value)) {
    list.unshift({ value: currentModel.value, label: header.value || currentModel.value });
  }
  return list;
});
const switchingModel = ref(false);
async function switchModel(event: Event) {
  const model = (event.target as HTMLSelectElement).value;
  if (!model || model === currentModel.value || switchingModel.value) return;
  switchingModel.value = true; error.value = "";
  try {
    liveSettings.value = await props.client.setModel(model);
    emit("settingsChanged", liveSettings.value);
  } catch (e) {
    error.value = String((e as Error).message || e);
    (event.target as HTMLSelectElement).value = currentModel.value;
  } finally { switchingModel.value = false; }
}
const errorView = computed(() => (error.value ? describeAgentHostError(error.value) : null));
const failure = (code?: string) => describeAgentHostError(code);

async function scroll(force=false) { await nextTick(); if(force||followingTail.value){transcript.value?.scrollTo({ top: transcript.value.scrollHeight });unreadBelow.value=false;}else unreadBelow.value=true; }
async function loadMessages(refreshAfterPending=false) {
 const id=selectedId.value,projectId=hostProjectId.value;if(!id||projectId===undefined)return;
 if(messageLoad?.id===id&&messageLoad.projectId===projectId){const pending=messageLoad.promise;await pending;if(refreshAfterPending&&id===selectedId.value&&projectId===hostProjectId.value)return loadMessages();return;}
 const promise=readMessages();messageLoad={id,projectId,promise};
 try{await promise;}finally{if(messageLoad?.promise===promise)messageLoad=undefined;}
}
async function readMessages() {
  const id = selectedId.value, projectId = hostProjectId.value, generation = ++messageGeneration;
  if (!id || projectId === undefined) { messages.value = []; task.value = null; return; }
  let loaded: AgentHostMessage[], savedTask: AgentTask | null;
  loadingMessages.value=true;
  let page:{items:AgentHostMessage[];hasMore:boolean;nextCursor:string|null}|undefined;
  try {
    [loaded, savedTask] = await Promise.all([(props.client.messagePage ? props.client.messagePage(id,projectId).then(result=>{page=result;return result.items;}) : props.client.messages(id, projectId)), props.client.task ? props.client.task(id, projectId).catch(() => null) : Promise.resolve(null)]);
  } catch (e) {
    if (!disposed && generation === messageGeneration && id === selectedId.value && projectId === hostProjectId.value) error.value = String((e as Error).message || e);
    if(generation===messageGeneration)loadingMessages.value=false;
    return;
  }
  if (disposed || generation !== messageGeneration || id !== selectedId.value || projectId !== hostProjectId.value) return;
  loadingMessages.value=false;
  if(historyLoaded){const ids=new Set(loaded.map(m=>m.id));messages.value=[...messages.value.filter(m=>!ids.has(m.id)),...loaded];}else{messages.value=loaded;hasEarlier.value=page?.hasMore??false;earlierCursor=page?.nextCursor??undefined;}
  // Read the actual saved message time, including after a completed reply;
  // merely opening a chat must not make it look recently active.
  const lastActivityAt = loaded.reduce((latest, item) => Number.isFinite(Date.parse(item.createdAt)) && (!latest || Date.parse(item.createdAt) > Date.parse(latest)) ? item.createdAt : latest, '');
  if (lastActivityAt) conversations.value = conversations.value.map(item => item.id === id && (!item.lastActivityAt || Date.parse(lastActivityAt) > Date.parse(item.lastActivityAt)) ? { ...item, lastActivityAt } : item);
  if (savedTask && !savedTask.parentTaskId && savedTask.conversationId === id && savedTask.projectId === projectId && (!task.value || (savedTask.id === task.value.id ? savedTask.revision >= task.value.revision : savedTask.createdAt >= task.value.createdAt))) task.value = savedTask;
  running.value = messages.value.some(item => item.status === "running");
  await scroll();
}
async function loadProject() {
  const generation = ++projectGeneration;
  ++messageGeneration;
  const { projectName, projectId } = props;
  hostProjectId.value = undefined; selectedId.value = undefined; messages.value = []; task.value = null; conversations.value = [];
  grantPrompt.value = null; runtimePermission.value = null; running.value = false; draft.value = "";
  permissionsOpen.value = false; ++permissionGeneration; savedFilePermissions.value = []; workspaceFullAccess.value=false; savedAppPermissions.value = []; historyQuery.value = ""; finderOpen.value=false;
  loadingProject.value = true;
  error.value = "";
  try {
    const boundId = await props.client.ensureProject(projectName, projectId);
    if (disposed || generation !== projectGeneration) return;
    const listed = await props.client.conversations(boundId);
    if (disposed || generation !== projectGeneration) return;
    hostProjectId.value = boundId;
    conversations.value = listed;
    selectedId.value = conversations.value[conversations.value.length - 1]?.id;
  } catch (e) { if (generation === projectGeneration) error.value = String((e as Error).message || e); }
  finally { if (generation === projectGeneration) loadingProject.value = false; }
}
function declineGrant() {
  const prompt = grantPrompt.value;
  grantPrompt.value = null;
  if (prompt) void props.client.resolveAuthorization(prompt.conversationId, false, { requestId: prompt.requestId, runId: prompt.runId }).catch(() => {});
}
async function acceptGrant() {
  const prompt = grantPrompt.value;
  if (!prompt || granting.value || hostProjectId.value === undefined) return;
  granting.value = true;
  const generation = projectGeneration;
  const valid = () => generation === projectGeneration && !disposed && prompt.conversationId === selectedId.value && prompt.projectId === hostProjectId.value && grantPrompt.value?.conversationId === prompt.conversationId && grantPrompt.value?.requestId === prompt.requestId && grantPrompt.value?.runId === prompt.runId;
  try {
    const listed = prompt.offers.some(offer => offer.appId !== "workspace") ? await props.client.listApps() : { items: [] };
    if (!valid()) return;
    for (const offer of prompt.offers) {
      if (!valid()) return;
      if (offer.appId === 'workspace') continue;
      const app = listed.items.find(item => item.id === offer.appId);
      const existing = app?.grants?.find(item => item.projectId === prompt.projectId)?.permissions ?? [];
      const permissions = [...new Set([...existing, ...offer.permissions.map(item => item.permission)])];
      await props.client.setGrants(offer.appId, {
        projectId: prompt.projectId,
        permissions,
        expectedRevision: offer.revision,
        disclosureAccepted: true,
        expectedRuntimeIdentity: prompt.identity,
      });
    }
    if (!valid()) return;
    if (grantPrompt.value?.requestId === prompt.requestId) grantPrompt.value = null;
    const result = await props.client.resolveAuthorization(prompt.conversationId, true, { requestId: prompt.requestId, runId: prompt.runId, ...(prompt.offers.some(offer => offer.appId === "workspace") ? { remember: rememberWorkspace.value } : {}) });
    if ((result as { accepted?: boolean })?.accepted === false) throw Error("host_authorization_expired");
  } catch (e) { error.value = String((e as Error).message || e); }

  finally { granting.value = false; }
}
async function loadPermissions() {
  const conversationId = selectedId.value, projectId = hostProjectId.value, generation = ++permissionGeneration;
  if (!conversationId || projectId === undefined) return;
  permissionsLoading.value = true; permissionsNotice.value = "";
  try {
    const [apps, files, policy] = await Promise.all([props.client.listApps(), props.client.workspaceGrants(conversationId, projectId),props.client.accessPolicy(projectId)]);
    if (disposed || generation !== permissionGeneration || conversationId !== selectedId.value || projectId !== hostProjectId.value) return;
    workspaceFullAccess.value=policy.mode==='full';
    savedFilePermissions.value = policy.mode==='full'?policy.workspacePermissions:files.permissions;
    savedAppPermissions.value = apps.items.flatMap(app => {
      const grant = app.grants?.find(item => item.projectId === projectId);
      const automatic=policy.mode==='full'?policy.apps[app.id]:undefined;
      const permissions=automatic?[...new Set([...automatic,...(grant?.disclosureRuntime===policy.runtimeIdentity?grant.permissions:[])])]:grant?.permissions||[];
      return permissions.length ? [{ appId:app.id,labels:permissions.map(permissionLabel),needsDisclosure:!automatic&&grant?.disclosureRuntime!==liveSettings.value.runtime?.identity }] : [];
    });
  } catch (e) { if (generation === permissionGeneration) permissionsNotice.value = tr("权限状态读取失败，请重试", "Could not load permissions. Retry."); }
  finally { if (generation === permissionGeneration) permissionsLoading.value = false; }
}
async function togglePermissions() {
  permissionsOpen.value = !permissionsOpen.value;
  if (permissionsOpen.value) await loadPermissions();
}
async function clearWorkspaceGrants() {
  if (!selectedId.value || hostProjectId.value === undefined || running.value) return;
  try { await props.client.clearWorkspaceGrants(selectedId.value, hostProjectId.value); await loadPermissions(); }
  catch (e) { error.value = String((e as Error).message || e); }
}
async function updateHistoryConversation(id: string, patch: { title?: string; pinned?: boolean }) {
  const projectId = hostProjectId.value, generation = projectGeneration;
  if (projectId === undefined || loadingProject.value || !conversations.value.some(item => item.id === id)) throw Error('host_conversation_not_found');
  const updated = await props.client.updateConversation(id, projectId, patch);
  if (disposed || generation !== projectGeneration || projectId !== hostProjectId.value) return;
  if (updated.id !== id || updated.projectId !== projectId) throw Error('host_conversation_scope_mismatch');
  conversations.value = conversations.value.map(item => item.id === id && (updated.metadataRevision??0)>=(item.metadataRevision??0) ? { ...item, ...updated,
    ...(item.lastActivityAt && (!updated.lastActivityAt || item.lastActivityAt > updated.lastActivityAt) ? { lastActivityAt: item.lastActivityAt } : {}),
  } : item);
}
async function revealHistoryConversation(id: string) {
  const projectId = hostProjectId.value, generation = projectGeneration;
  if (projectId === undefined || loadingProject.value || !conversations.value.some(item => item.id === id)) throw Error('host_conversation_not_found');
  const workspace = await props.client.conversationWorkspace(id, projectId);
  if (disposed || generation !== projectGeneration || projectId !== hostProjectId.value) return '';
  if (workspace.conversationId !== id || workspace.projectId !== projectId || workspace.shared !== true) throw Error('host_conversation_scope_mismatch');
  if (!("__TAURI_INTERNALS__" in globalThis)) throw Error(tr('请在桌面 App 中使用 Finder', 'Use Finder from the desktop App'));
  await invoke('agent_host_chat_file_action', { projectId, conversationId: id, action: 'reveal' });
  return workspace.path;
}
async function removeConversation(id = selectedId.value): Promise<boolean> {
  if (!id || hostProjectId.value === undefined || running.value || continuingTask.value || !conversations.value.some(item => item.id === id)) return false;
  const generation = projectGeneration;
  try {
    await props.client.deleteConversation(id, hostProjectId.value);
    if (generation !== projectGeneration || disposed) return false;
    conversations.value = conversations.value.filter(item => item.id !== id);
    if (selectedId.value === id) selectedId.value = conversations.value[conversations.value.length - 1]?.id;
    if (!selectedId.value) { messages.value = []; task.value = null; steps.value = []; }
    return true;
  } catch (e) { if (generation === projectGeneration && !disposed) error.value = String((e as Error).message || e); return false; }
}
function conversationLabel(title?: string) {
  if (!title || title === "New conversation" || title === "新对话" || title === "新對話") return tr("新对话", "New conversation");
  return title;
}
async function newConversation() {
  if (hostProjectId.value === undefined) return;
  const projectId = hostProjectId.value, generation = projectGeneration;
  const created = await props.client.createConversation(projectId);
  if (disposed || generation !== projectGeneration || projectId !== hostProjectId.value) return;
  conversations.value = [...conversations.value, created];
  selectedId.value = created.id; messages.value = []; task.value = null; resetActivity();
}
async function chooseIntent(choice: string, message: AgentHostMessage) {
  const url = [...messages.value].reverse().find(item => item.role === "user" && item.createdAt <= message.createdAt)?.content.trim();
  if (!url || !selectedId.value || hostProjectId.value === undefined || running.value) return;
  try {
    const prepared = await props.client.prepareTask(selectedId.value, hostProjectId.value, choice, url);
    pendingApproval.value = { id: prepared.id, scopeHash: prepared.scopeHash, choice };
  } catch (e) { error.value = String((e as Error).message || e); }
}
async function confirmIntent() {
  const approval = pendingApproval.value;
  if (!approval || !selectedId.value || hostProjectId.value === undefined) return;
  try {
    const confirmed = await props.client.confirmTask(selectedId.value, hostProjectId.value, approval.id, approval.scopeHash);
    if (confirmed.startsScan) error.value = "确认不应启动扫描";
    pendingApproval.value = null;
  } catch (e) { error.value = String((e as Error).message || e); }
}
async function send() {
  const text = draft.value.trim();
  if (!text || loadingProject.value || continuingTask.value || hostProjectId.value === undefined) return;
  const generation = projectGeneration, projectId = hostProjectId.value;
  error.value = "";
  if (running.value) {
    const conversationId = selectedId.value;
    draft.value = "";
    try { await steer(text); if (generation === projectGeneration && conversationId === selectedId.value) await loadMessages(); }
    catch (e) { if (generation === projectGeneration && conversationId === selectedId.value) { draft.value ||= text; error.value = String((e as Error).message || e); } }
    return;
  }
  if (!selectedId.value) await newConversation();
  if (generation !== projectGeneration || projectId !== hostProjectId.value || !selectedId.value) return;
  const conversationId = selectedId.value;
  draft.value = ""; resetActivity(); running.value = true;
  try {
    const binding = task.value?.status !== "completed" ? task.value?.id : undefined;
    if (binding) await props.client.send(conversationId, projectId, text, binding); else await props.client.send(conversationId, projectId, text);
    if (generation === projectGeneration && conversationId === selectedId.value) await loadMessages();
  }
  catch (e) { if (generation === projectGeneration && conversationId === selectedId.value) { running.value = false; draft.value ||= text; error.value = String((e as Error).message || e); } }
}
async function cancel() { if (selectedId.value) await props.client.cancel(selectedId.value).catch(() => {}); }
async function continueTask() {
  const current = task.value, conversationId = selectedId.value, projectId = hostProjectId.value, generation = projectGeneration;
  if (!current || !conversationId || projectId === undefined || running.value || continuingTask.value || current.conversationId !== conversationId || current.projectId !== projectId) return;
  continuingTask.value = true; error.value = ""; resetActivity();
  try {
    await props.client.continueTask(conversationId, projectId, current.id, draft.value.trim() || undefined);
    if (generation === projectGeneration && conversationId === selectedId.value) { draft.value = ""; await loadMessages(); }
  } catch (e) { if (generation === projectGeneration && conversationId === selectedId.value) error.value = String((e as Error).message || e); }
  finally { continuingTask.value = false; }
}

async function cancelChild(childTaskId: string, parentRunId: string) {
  const current = task.value, conversationId = selectedId.value, projectId = hostProjectId.value, generation = projectGeneration;
  const child = current?.children?.find(item => item.id === childTaskId && item.parentRunId === parentRunId);
  if (!current || !child || child.status !== "running" || !conversationId || projectId === undefined || cancellingChildren.value.includes(childTaskId)) return;
  cancellingChildren.value = [...cancellingChildren.value, childTaskId];
  const isCurrent = () => !disposed && generation === projectGeneration && conversationId === selectedId.value && projectId === hostProjectId.value && task.value?.id === current.id;
  try {
    const result = await props.client.cancelChild(conversationId, projectId, current.id, childTaskId, parentRunId);
    if (isCurrent() && result.task.id === current.id && result.task.conversationId === conversationId && result.task.projectId === projectId && result.task.revision >= task.value!.revision) task.value = result.task;
  } catch (reason) { if (isCurrent()) error.value = String((reason as Error).message || reason); }
  finally { if (isCurrent()) cancellingChildren.value = cancellingChildren.value.filter(id => id !== childTaskId); }
}

function onComposerKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter" || event.shiftKey || event.ctrlKey || event.altKey || event.metaKey || event.isComposing || event.keyCode === 229) return;
  event.preventDefault(); void send();
}
async function decidePermission(optionId: string) {
  const permission = runtimePermission.value;
  if (!permission) return;
  try { await props.client.decidePermission(permission.id, optionId); if (runtimePermission.value?.id === permission.id) runtimePermission.value = null; }
  catch (e) { error.value = String((e as Error).message || e); }
}
async function restoreStatus() {
  const id = selectedId.value, generation = projectGeneration;
  if (!id) return;
  try { const snapshot = await props.client.status(); if (!disposed && generation === projectGeneration && id === selectedId.value) handleEvent({ ...snapshot, type: "host.snapshot", restoreOnly:true }); }
  catch { connected.value = false; }
}

let needsReconnectRefresh=false;
async function refreshConversationMetadata(){const pid=hostProjectId.value,g=projectGeneration;if(pid===undefined)return;try{const rows=await props.client.conversations(pid);if(!disposed&&g===projectGeneration&&pid===hostProjectId.value){conversations.value=rows.map(row=>{const current=conversations.value.find(c=>c.id===row.id);return current&&(current.metadataRevision??0)>(row.metadataRevision??0)?current:row;});if(selectedId.value&&!rows.some(c=>c.id===selectedId.value))selectedId.value=visibleConversations.value[0]?.id;}}catch{/* Reconnect metadata is retried by the next event. */}}
function handleEvent(event: AgentHostEvent) {
  if(event.type==='access.policy.changed'&&typeof window!=='undefined'&&typeof CustomEvent!=='undefined')window.dispatchEvent(new CustomEvent('oviraptor:access-policy-changed',{detail:{projectId:event.projectId}}));
  if (event.type === "connection.status") { if(event.connected!==true)needsReconnectRefresh=true;connected.value = event.connected === true; return; }
  if (event.type === "host.snapshot") {
    connected.value = true;
    const authorizations = Array.isArray(event.authorizations) ? event.authorizations as AgentHostEvent[] : [];
    grantPrompt.value = null;
    for (const pending of authorizations) if (pending.conversationId === selectedId.value) handleEvent({ ...pending, type: "authorization.requested" });
    const permissions = Array.isArray(event.permissions) ? event.permissions as AgentHostEvent[] : [];
    runtimePermission.value = null;
    for (const pending of permissions) if (pending.conversationId === selectedId.value) handleEvent({ ...pending, type: "permission.request" });
    if(!event.restoreOnly&&needsReconnectRefresh){needsReconnectRefresh=false;void refreshConversationMetadata();if(selectedId.value)void loadMessages(true);}
    return;
  }
  if (event.type === "permission.request" && typeof event.conversationId === "string" && event.conversationId === selectedId.value && typeof event.id === "string" && Array.isArray(event.options)) {
    runtimePermission.value = { id: event.id, conversationId: event.conversationId, description: String(event.description || event.title || ""), options: event.options as RuntimePermission["options"] }; return;
  }
  if ((event.type === "authorization.cancelled" || event.type === "authorization.resolved") && event.requestId === grantPrompt.value?.requestId) { grantPrompt.value = null; return; }
  if (event.type === "runtime.inner" && event.conversationId === selectedId.value) {
    if (!bindActivity(event)) return;
    inner.value = {
      goal: String(event.goal ?? ""),
      phase: String(event.phase ?? "model"),
      phaseLabel: String(event.phaseLabel ?? ""),
      facts: Array.isArray(event.facts) ? event.facts.map(item => String(item)).slice(-3) : [],
      hypotheses: Array.isArray(event.hypotheses) ? event.hypotheses.map(item => String(item)).slice(-2) : [],
      compressed: Boolean(event.compressed),
      hold: String(event.hold ?? ""),
    };
    return;
  }
  if (event.type === "authorization.requested" && typeof event.conversationId === "string" && event.conversationId === selectedId.value && Array.isArray(event.offers) && event.offers.length) {
    grantPrompt.value = {
      conversationId: event.conversationId,
      requestId: typeof event.requestId === "string" ? event.requestId : undefined,
      runId: event.runId,
      projectId: Number(event.projectId) || hostProjectId.value || 0,
      identity: String(event.identity ?? ""),
      offers: event.offers as GrantOffer[],
      question: String(event.question ?? ""),
    };
    return;
  }
  if(event.type==='conversation.deleted'&&event.conversationId){conversations.value=conversations.value.filter(item=>item.id!==event.conversationId);if(selectedId.value===event.conversationId)selectedId.value=visibleConversations.value[0]?.id;return;}
  if (event.type === "conversation.updated" && event.conversationId) {
    conversations.value = conversations.value.map(item => (item.id === event.conversationId && (typeof event.metadataRevision!=='number'||event.metadataRevision>=(item.metadataRevision??0)) ? { ...item,
      ...(typeof event.metadataRevision==='number'?{metadataRevision:event.metadataRevision}:{}),
      ...(typeof event.title === 'string' ? { title: event.title } : {}),
      ...(typeof event.pinned === 'boolean' ? { pinned: event.pinned } : {}),
      ...(typeof event.updatedAt === 'string' ? { updatedAt: event.updatedAt } : {}),
      ...(typeof event.lastActivityAt === 'string' ? { lastActivityAt: event.lastActivityAt } : {}),
    } : item));
    return;
  }
  if(event.type==='message.completed'&&event.conversationId!==selectedId.value)void refreshConversationMetadata();
  if (!selectedId.value || event.conversationId !== selectedId.value) return;
  if (event.type === "task.child.updated") {
    if (task.value && event.parentTaskId === task.value.id) void loadMessages();
  } else if (event.type === "task.updated" && event.task && typeof event.task === "object") {
    const next = event.task as AgentTask;
    if (next.parentTaskId) return;
    if (next.conversationId === selectedId.value && next.projectId === hostProjectId.value && (!task.value || (next.id === task.value.id ? next.revision >= task.value.revision : next.createdAt >= task.value.createdAt))) task.value = next;
  } else if (event.type === "message.delta" && event.messageId) {
    const target = messages.value.find(item => item.id === event.messageId);
    if (target) { target.content += String(event.delta ?? ""); void scroll(); } else void loadMessages();
  } else if (event.type === "runtime.step" && event.title) {
    if (!bindActivity(event)) return;
    const title = String(event.title); if (!steps.value.includes(title)) steps.value = [...steps.value, title].slice(-8);
  } else if (event.type === "runtime.heartbeat") {
    if (!bindActivity(event)) return;
    heartbeat.value = { elapsedMs: Number(event.elapsedMs) || 0, stepTitle: typeof event.stepTitle === "string" ? event.stepTitle : undefined };
  } else if (event.type === "message.steer") { void loadMessages(); }
  else if (event.type === "runtime.usage" && event.usage && typeof event.usage === 'object') {
    const message = messages.value.find(item => item.role === 'assistant' && item.runId === event.runId);
    if (message) message.usage = event.usage as AgentModelUsage;
  }
  else if (event.type === "message.completed") {
    const active = [...messages.value].reverse().find(message => message.role === 'assistant' && message.status === 'running');
    if (active && (event.runId && active.runId && event.runId !== active.runId || event.messageId && event.messageId !== active.id)) return;
    running.value = false; heartbeat.value = undefined; grantPrompt.value = null; runtimePermission.value = null; void loadMessages(true); }
  else if (event.type === "runtime.permission_resolved") runtimePermission.value = null;
  // runtime.error is shown on the failed assistant bubble (reloaded on completion); no duplicate banner.
  else if (event.type === "runtime.error") { void loadMessages(); }
}
const stop = props.client.events(handleEvent);
onBeforeUnmount(() => { ++permissionGeneration; disposed = true; ++projectGeneration; ++messageGeneration; stop(); });
watch(() => [props.projectName, props.projectId], () => void loadProject(), { immediate: true, flush: "sync" });
watch(running, value => { if (!value) heartbeat.value = undefined; });
watch(selectedId, (id,previous) => { if(previous)conversationDrafts.set(previous,draft.value);++messageGeneration;messageLoad=undefined;messages.value=[];loadingMessages.value=false;loadingEarlier.value=false;historyLoaded=false;earlierCursor=undefined;hasEarlier.value=false;followingTail.value=true;unreadBelow.value=false;error.value='';discoveryPanelOpen.value=!!props.draftRequest?.analysis;draft.value=id?conversationDrafts.get(id)||"":""; permissionsOpen.value = false; ++permissionGeneration; resetActivity(); task.value = null; cancellingChildren.value = []; grantPrompt.value = null; runtimePermission.value = null; running.value = false; void loadMessages(); void restoreStatus(); }, { flush: "sync" });
watch([()=>props.draftRequest,loadingProject,running,continuingTask,hostProjectId],()=>void applyDiscoveryDraft(),{immediate:true});
watch(()=>props.historySearchRequest,value=>{if(value)finderOpen.value=true;},{immediate:true});
</script>

<template>
  <Teleport defer to="#conversation-history">
    <nav class="ahc-side-list" :aria-label="tr('聊天记录', 'Chat history')" data-testid="agent-host-history">
      <button type="button" class="ahc-side-new" :disabled="loadingProject" @click="newConversation"><Plus :size="16" />{{ tr('新对话', 'New chat') }}</button>
      <label class="ahc-history-search"><Search :size="14" /><input v-model="historyQuery" type="search" :placeholder="tr('搜索对话', 'Search chats')" :aria-label="tr('搜索对话', 'Search chats')" /><button type="button" :aria-label="tr('查找全部会话','Find all conversations')" title="⌘K / Ctrl+K" @click="finderOpen=true">⌘K</button></label>
      <div class="ahc-history-heading"><span>{{ tr('对话', 'Chats') }}</span><button type="button" @click="finderOpen=true">{{tr(`全部 ${conversations.length}`,`All ${conversations.length}`)}}</button></div>
      <ConversationHistoryList :items="recentConversations" :selected-id="selectedId" :project-id="hostProjectId" :loading="loadingProject" :deleting-disabled="running||continuingTask" :empty-label="historyQuery?tr('没有匹配的对话','No matching chats'):tr('新对话会显示在这里','Your chats appear here')" :update-conversation="updateHistoryConversation" :reveal-conversation="revealHistoryConversation" :remove-conversation="removeConversation" @select="selectedId=$event"/>
    </nav>
  </Teleport>
  <ConversationFinder v-model:open="finderOpen" v-model:query="historyQuery" :items="finderConversations" :selected-id="selectedId" :project-name="projectName" :loading="loadingProject" @select="findConversation"/>
  <div class="conversation-main agent-host-chat" data-testid="agent-host-chat">
    <details v-if="task" class="ahc-task" data-testid="agent-host-task" :open="!readOnlyAnswered && task.status === 'interrupted'">
      <summary><strong>{{ readOnlyAnswered ? tr('本轮只读核对已答复', 'Read-only answer delivered') : `${tr('任务', 'Task')} · ${taskLabel(task.status)}` }}</strong><span>{{ taskDisplayGoal }}</span><small v-if="readOnlyAnswered && task.readOnlyAnswer?.historicalWorkPending">{{ tr('历史任务仍需处理', 'Historical work still needs attention') }}</small></summary>
      <p v-if="readOnlyAnswered">{{ tr('本轮只读取已有数据并作答。下方保留历史任务原记录，不代表历史执行已完成或通过验收。', 'This turn read saved data and answered. The original task history below is retained; this does not complete or accept earlier execution.') }}</p>
      <p v-if="task.outcomeUnknown" class="ahc-task-warning">{{ tr('上次操作结果未确认。继续时会先核实，不自动重放写操作。', 'An earlier operation has an unknown outcome. Continue checks its effects before retrying any write.') }}</p>
      <p v-if="task.summary">{{ task.summary }}</p>
      <ol v-if="task.steps.length"><li v-for="step in task.steps" :key="step.id"><strong>{{ step.title }}</strong> · {{ taskLabel(step.status) }} <small>{{ step.verified ? tr('有工具证据引用', 'Tool evidence referenced') : tr('计划状态，尚未验收', 'Plan status; unverified') }}</small><span v-if="step.dependsOn.length"> · {{ tr('依赖', 'Depends on') }} {{ step.dependsOn.join(', ') }}</span></li></ol>
      <p v-else>{{ readOnlyAnswered ? tr('只读核对无需创建执行计划。', 'A read-only check needs no execution plan.') : tr('尚未记录步骤；Agent 可通过任务工具保存计划。', 'No steps recorded yet. The agent can save a plan using task tools.') }}</p>
      <details v-if="task.evidence.length"><summary>{{ tr('工具证据', 'Tool evidence') }} · {{ task.evidence.length }}</summary><ul><li v-for="item in task.evidence.slice(-12)" :key="item.id"><code>{{ item.id }}</code> · {{ item.toolName }} · {{ taskLabel(item.status) }}<p v-if="item.summary">{{ item.summary }}</p></li></ul></details>
      <ul v-if="task.artifacts.length"><li v-for="artifact in task.artifacts" :key="artifact.id">{{ artifact.label }} · <code>{{ artifact.location }}</code> · {{ tr('Agent 登记的产物位置', 'Agent-reported artifact location') }} · {{ artifact.evidenceRefs.length }} {{ tr('条证据引用', 'evidence references') }}</li></ul>
      <AgentTaskCollaboration :task="task" :cancelling="cancellingChildren" @cancel-child="cancelChild" />
      <button v-if="task.status !== 'completed' && !running && (!readOnlyAnswered || task.readOnlyAnswer?.historicalWorkPending)" class="button secondary compact" type="button" :disabled="continuingTask || loadingProject" data-testid="agent-task-continue" @click="continueTask">{{ continuingTask ? tr('接续中…', 'Continuing…') : readOnlyAnswered ? tr('继续历史任务', 'Continue historical task') : tr('继续原任务', 'Continue task') }}</button>
    </details>
    <div ref="transcript" class="agent-host-chat-transcript" @scroll="observeTranscript">
      <div class="ahc-column">
      <details v-if="selectedId&&hostProjectId" class="ahc-discovery-details" :open="discoveryPanelOpen" @toggle="discoveryPanelOpen=($event.target as HTMLDetailsElement).open"><summary>{{tr('项目资产发现','Project asset discovery')}}</summary><DiscoveryAssistPanel v-if="discoveryPanelOpen" :client="client" :conversation-id="selectedId" :project-id="hostProjectId" :refresh-key="task?.revision" /></details>
      <button v-if="hasEarlier" type="button" class="text-button" :disabled="loadingEarlier" @click="loadEarlierMessages">{{loadingEarlier?tr('读取中…','Loading…'):tr('加载更早消息','Load earlier messages')}}</button>
      <p v-if="loadingMessages&&!messages.length" role="status">{{tr('正在读取此会话…','Loading this conversation…')}}</p>
      <p v-if="!messages.length" class="ahc-empty">{{ tr(`当前模型：${header || '尚未选择'}。权限不足时会说明用途；已保存的授权会在原范围内复用。`, `Model: ${header || 'not selected'}. Missing permissions are explained; saved grants are reused within their scope.`) }}</p>
      <p v-if="!messages.length" class="ahc-empty ahc-empty-note">{{ tr('执行过程显示在回复下方，点击箭头可查看工具记录与证据。', 'Expand the activity below a reply to view tool records and evidence.') }}</p>
      <article v-for="message in messages" :key="message.id" :class="['agent-host-chat-message', message.role]">
        <ConversationMessage v-if="message.content" :message="message" :show-actions="false" :files="conversationFiles.references.value" @open-file="conversationFiles.showPreview($event)" />
        <p v-else-if="message.status === 'running' && activityMessage?.id !== message.id" class="ahc-reply-pending">{{ tr('正在准备回复…', 'Preparing reply…') }}</p>
        <small v-if="message.role === 'user' && message.stopReason?.startsWith('steer_')" class="ahc-steer-state">{{ message.stopReason === 'steer_delivered' ? tr('补充要求已交给 Agent', 'Correction delivered to Agent') : message.stopReason === 'steer_interrupted' ? tr('补充要求已保存，将在你继续时交付', 'Correction saved for your next continuation') : tr('补充要求已保存，等待 Agent 接收', 'Correction saved, awaiting delivery') }}</small>
        <small v-if="message.role === 'assistant' && message.usage?.requests" class="ahc-model-usage">{{tr('模型请求', 'Model requests')}} {{message.usage.requests}} · {{tr('输入', 'Input')}} {{message.usage.inputTokens?.toLocaleString() ?? tr('未知','unknown')}} / {{tr('输出','Output')}} {{message.usage.outputTokens?.toLocaleString() ?? tr('未知','unknown')}} Token<span v-if="message.usage.unknownRequests"> · {{message.usage.unknownRequests}} {{tr('次用量未返回，未计入上述数量','requests did not report usage and are excluded from these totals')}}</span></small>
        <div v-if="message.content && message.status !== 'running'" class="ahc-msg-actions">
          <button type="button" data-testid="agent-host-copy" :title="tr('复制', 'Copy')" :aria-label="tr('复制', 'Copy')" @click="copyMessage(message)"><Check v-if="copiedId === message.id" :size="13" /><Copy v-else :size="13" /></button>
          <button v-if="message.role === 'user'" type="button" data-testid="agent-host-edit" :title="tr('编辑后重发', 'Edit & resend')" :aria-label="tr('编辑后重发', 'Edit & resend')" @click="editMessage(message)"><Pencil :size="13" /></button>
        </div>
        <p v-if="message.role === 'assistant' && (message.status === 'refused' || message.stopReason === 'refusal')" class="ahc-refusal">{{ tr('模型拒绝继续。这一轮没有改写问题，也没有再调用工具。', 'The model refused. This turn did not rephrase the request or call more tools.') }}</p>
        <div v-if="message.role === 'assistant' && message.stopReason === 'intent_preview'" class="ahc-intent" data-testid="agent-host-intent">
          <button type="button" @click="chooseIntent('view_link', message)">{{ tr('查看链接', 'View link') }}</button>
          <button type="button" @click="chooseIntent('import_asset', message)">{{ tr('查询或导入资产', 'Look up or import assets') }}</button>
          <button type="button" @click="chooseIntent('prepare_authorized_scan', message)">{{ tr('准备授权扫描', 'Prepare an authorized scan') }}</button>
          <button v-if="pendingApproval" type="button" data-testid="agent-host-confirm-intent" @click="confirmIntent">{{ tr('确认这个选择', 'Confirm this choice') }}</button>
        </div>
        <div v-if="message.status === 'failed'" class="agent-host-chat-failure" role="note">
          <strong>{{ tr(...failure(message.failureCode).title) }}</strong>
          <span>{{ tr(...failure(message.failureCode).hint) }}</span>
          <code :title="tr('错误码', 'Error code')">{{ failure(message.failureCode).code }}</code>
        </div>
        <details v-if="message.role === 'assistant' && activityMessage?.id === message.id && (inner || steps.length || heartbeat || activityEvidence.length)" class="ahc-inner" data-testid="agent-host-inner" :open="activityOpen" @toggle="toggleActivity">
          <summary :title="tr('查看执行过程', 'View activity')">
            <span v-if="message.status === 'running'" class="ahc-pulse" />
            <span class="ahc-inner-phase">{{ innerPhaseText }}</span>
            <span v-if="activityCount" class="ahc-inner-meta">{{ tr(`${activityCount} 项操作`, `${activityCount} actions`) }}</span>
            <span v-if="message.status === 'running' && heartbeat" class="ahc-inner-meta">{{ Math.max(1, Math.round(heartbeat.elapsedMs / 1000)) }}s</span>
            <ChevronDown :size="13" class="ahc-inner-chevron" aria-hidden="true" />
          </summary>
          <div class="ahc-inner-body">
            <div v-if="inner?.goal"><strong>{{ tr('目标', 'Goal') }}</strong><p>{{ inner.goal }}</p></div>
            <div v-if="inner?.hold"><strong>{{ tr('检查', 'Check') }}</strong><p>{{ inner.hold }}</p></div>
            <div v-if="inner">
              <strong>{{ tr('已确认', 'Confirmed') }}</strong>
              <ul v-if="inner.facts.length"><li v-for="fact in inner.facts" :key="fact">{{ fact }}</li></ul>
              <p v-else>{{ tr('还没有。模型说的数字要等工具结果回来。', 'None yet. A number waits for a tool result.') }}</p>
            </div>
            <div v-if="inner?.hypotheses.length">
              <strong>{{ tr('待验证', 'Unverified') }}</strong>
              <ul v-if="inner.hypotheses.length"><li v-for="item in inner.hypotheses" :key="item">{{ item }}</li></ul>
              <p v-else>{{ tr('没有。这里的内容不是事实。', 'None. Items here are not facts.') }}</p>
            </div>
            <p v-if="inner?.compressed">{{ tr('更早的原文已收成一条。最早的要求还留着。', 'Older text was folded. The first request is still kept.') }}</p>
            <p v-if="running && heartbeat" class="ahc-heartbeat" data-testid="agent-host-heartbeat" aria-live="polite"><span class="ahc-pulse" />{{ heartbeatLabel }}</p>
            <ol v-if="steps.length" class="agent-host-chat-steps"><li v-for="step in steps" :key="step">{{ step }}</li></ol>
            <ol v-else-if="activityEvidence.length" class="agent-host-chat-steps"><li v-for="item in activityEvidence" :key="item.id"><strong>{{ item.toolName }} · {{ taskLabel(item.status) }}</strong><p v-if="item.summary">{{ item.summary }}</p></li></ol>
          </div>
        </details>
      </article>
      <ConversationFiles :state="conversationFiles" />
      </div>
    </div>
      <button v-if="unreadBelow" class="button secondary compact ahc-jump-latest" type="button" @click="jumpToLatest">{{tr('有新消息 · 回到底部','New messages · Jump to latest')}}</button>
    <div v-if="errorView" class="agent-host-chat-alert" role="alert">
      <strong>{{ tr(...errorView.title) }}</strong> <span>{{ tr(...errorView.hint) }}</span>
      <code>{{ errorView.code }}</code>
    </div>
    <div v-if="grantPrompt" class="ahc-grant" role="dialog" aria-modal="true" :aria-label="tr('授权', 'Grant access')">
      <strong>{{ grantIsWorkspace ? tr('需要授权才能继续', 'Permission needed to continue') : tr('需要授权才能查', 'Permission needed') }}</strong>
      <p>{{ tr('这次操作要用到还没打开的权限。授权只作用于当前工作空间，也可以先不授。', 'This step needs a permission that is still off. Granting applies only to this workspace.') }}</p>
      <p v-if="grantPrompt.offers.some(offer => offer.appId === 'workspace')">{{ tr('此请求只授予列出的文件权限。脚本执行将在隔离运行环境接通后开放。', 'This request grants only the listed file permissions. Script execution requires an isolated runtime.') }}</p>
      <ul>
        <li v-for="offer in grantPrompt.offers" :key="offer.appId">{{ offer.permissions.map(item => permissionLabel(item.permission)).join('、') }}</li>
      </ul>
      <label v-if="grantPrompt.offers.some(offer => offer.appId === 'workspace')" class="ahc-remember"><input v-model="rememberWorkspace" type="checkbox" />{{ tr('记住本对话的这些文件权限；切换模型后需重新确认', 'Remember these file permissions for this conversation; changing models requires confirmation') }}</label>
      <footer>
        <button type="button" class="button ghost compact" :disabled="granting" @click="declineGrant">{{ tr('暂不', 'Not now') }}</button>
        <button type="button" class="button primary compact" :disabled="granting" @click="acceptGrant">{{ granting ? tr('授权中…', 'Granting…') : (grantIsWorkspace ? tr('授权并继续', 'Grant and continue') : tr('授权并继续', 'Grant and continue')) }}</button>
      </footer>
    </div>
    <div v-if="runtimePermission" class="ahc-grant" role="dialog" :aria-label="tr('运行时授权','Runtime permission')"><strong>{{tr('请审核运行时操作','Review runtime operation')}}</strong><p>{{runtimePermission.description}}</p><footer><button v-for="option in runtimePermission.options" :key="option.id" type="button" class="button secondary compact" @click="decidePermission(option.id)">{{option.label}}</button></footer></div>
    <p v-if="!connected" class="ahc-connection" role="status">{{tr('连接中断，正在重连。已运行的任务状态会重新核实。','Connection lost. Reconnecting and checking the running task.')}}</p>
    <div class="ahc-compose-dock">
    <form class="ahc-composer" @submit.prevent="send">
      <textarea ref="composerInput" v-model="draft" rows="1" :disabled="loadingProject" :placeholder="running ? tr('输入插话：会在下一步带入当前回复', 'Steer: injected before the next step') : tr('问点什么…（Enter 发送，Shift+Enter 换行）', 'Ask anything… (Enter to send)')" @keydown="onComposerKeydown" />
      <button v-if="running && draft.trim()" type="submit" class="ahc-send ahc-steer" data-testid="agent-host-steer" :title="tr('纠偏', 'Steer')" :aria-label="tr('纠偏', 'Steer')"><CornerDownRight :size="15" /></button>
      <button v-else-if="running" type="button" class="ahc-send" :aria-label="tr('停止', 'Stop')" @click="cancel"><Square :size="14" /></button>
      <button v-else type="submit" class="ahc-send" :disabled="!draft.trim()" :aria-label="tr('发送', 'Send')"><ArrowUp :size="16" /></button>
    </form>
      <div class="ahc-composer-controls" data-testid="agent-host-composer-controls">
        <label class="ahc-select-wrap ahc-model" :title="header" data-testid="agent-host-chat-runtime">
          <select class="ahc-select" data-testid="agent-host-model-picker" :value="currentModel" :disabled="switchingModel || running" :aria-label="tr('会话模型', 'Session model')" @change="switchModel">
            <option v-for="item in modelOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
          </select><ChevronDown :size="13" class="ahc-select-caret" />
        </label>
        <AgentPermissionMode :client="client" :project-id="hostProjectId" :runtime-identity="liveSettings.runtime?.identity" :disabled="loadingProject||continuingTask" compact @changed="permissionsOpen&&loadPermissions()" />
        <button type="button" class="ahc-permissions-toggle" :aria-expanded="permissionsOpen" aria-controls="agent-chat-permissions" :disabled="!selectedId" @click="togglePermissions"><ShieldCheck :size="14" />{{ tr('权限详情', 'Permission details') }}</button>
        <span class="ahc-composer-hint">{{ running ? tr('可随时停止或补充要求', 'Stop or steer at any time') : tr('Enter 发送 · Shift+Enter 换行', 'Enter to send · Shift+Enter for a new line') }}</span>
      </div>
      <section v-if="permissionsOpen" id="agent-chat-permissions" class="ahc-permissions-panel" :aria-label="tr('当前权限', 'Current permissions')">
        <header><strong>{{ tr('当前权限', 'Current permissions') }}</strong><button v-refresh-feedback="{scope:projectId,busy:permissionsLoading,error:permissionsNotice}" type="button" @click="loadPermissions">{{ tr('刷新', 'Refresh') }}</button></header>
        <p v-if="permissionsLoading" role="status">{{ tr('正在读取权限…', 'Loading permissions…') }}</p>
        <p v-else-if="permissionsNotice" role="status">{{ permissionsNotice }}</p>
        <template v-else>
          <p><strong>{{ tr('文件访问', 'File access') }}</strong> · {{ savedFilePermissions.length ? savedFilePermissions.map(permissionLabel).join('、') : tr('尚未授权', 'No saved grants') }}</p>
          <button v-if="savedFilePermissions.length&&!workspaceFullAccess" type="button" :disabled="running" @click="clearWorkspaceGrants">{{ tr('清除本对话文件权限', 'Clear conversation file grants') }}</button>
          <p><strong>{{ tr('当前项目数据', 'Project data') }}</strong> · {{ savedAppPermissions.length ? '' : tr('尚未授权', 'No saved grants') }}</p>
          <ul v-if="savedAppPermissions.length"><li v-for="app in savedAppPermissions" :key="app.appId">{{ app.labels.join('、') }}<span v-if="app.needsDisclosure"> · {{ tr('切换模型后待重新确认', 'Requires confirmation for this model') }}</span></li></ul>
          <p class="ahc-permissions-help">{{ tr('项目权限可在配置中心 → 数据权限调整。新操作需要的权限会在执行前说明。', 'Manage project grants in Configuration Center → Data permissions. New permissions are explained before use.') }}</p>
        </template>
      </section>
    </div>
  </div>
</template>

<style scoped>
.ahc-side-list{display:flex;flex-direction:column;height:100%;min-height:0;gap:10px;padding:4px 0 12px;overflow:hidden}
.ahc-side-new{display:flex;align-items:center;gap:8px;flex-shrink:0;width:100%;padding:10px;border:1px solid var(--line,#e5e6e1);border-radius:9px;background:var(--app-surface,#fff);color:var(--app-ink,#252624);font:inherit;font-size:12px;cursor:pointer}
.ahc-history-search{display:flex;align-items:center;gap:7px;flex-shrink:0;padding:0 8px;color:var(--app-muted,#72746f)}
.ahc-history-search input{min-width:0;width:100%;height:28px;padding:0;border:0!important;background:transparent!important;box-shadow:none!important;font:inherit;font-size:11px;color:inherit}
.ahc-history-search button{flex-shrink:0;padding:3px 5px;border:1px solid var(--line);border-radius:5px;background:transparent;color:inherit;font:inherit;font-size:10px;cursor:pointer}.ahc-history-heading button{border:0;padding:0;background:transparent;color:inherit;font:inherit;cursor:pointer}.ahc-history-heading button:hover{color:var(--blue)}
.ahc-history-heading{display:flex;justify-content:space-between;padding:2px 10px 0;font-size:10px;color:var(--app-muted,#72746f)}

.agent-host-chat { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; width: 100%; height: 100%; background: var(--app-bg, transparent); }
.agent-host-chat-transcript { flex: 1; min-height: 0; overflow: auto; padding: 18px 28px 8px; }
.ahc-jump-latest{align-self:center;margin:4px 0 8px;flex-shrink:0}.ahc-discovery-details{border-bottom:1px solid var(--line,#e1e3e7);padding:4px 0 10px;color:var(--app-muted,#687078);font-size:12px}.ahc-discovery-details>summary{cursor:pointer}.ahc-discovery-details[open]>summary{margin-bottom:10px}.text-button{align-self:center;border:1px solid var(--line,#e1e3e7);border-radius:7px;background:transparent;color:inherit;padding:6px 12px;font-size:12px;cursor:pointer}
.ahc-column { width: min(760px, 100%); margin: 0 auto; min-height: 100%; display: flex; flex-direction: column; gap: 14px; }
.agent-host-chat-message { position: relative; width: 100%; min-width: 0; padding: 0; white-space: normal; font-size: 14px; line-height: 1.65; overflow-wrap: anywhere; }
.agent-host-chat-message :deep(.conversation-message) { margin-bottom: 0; }
.agent-host-chat-message.user { align-self: stretch; background: transparent; }
.agent-host-chat-message.assistant { align-self: flex-start; padding-left: 0; background: transparent; }
.agent-host-chat-message p { margin: 0; }
.agent-host-chat-steps { margin: 0; padding: 0 0 0 18px; border: 0; color: var(--app-muted, #72746f); font-size: 12px; line-height: 1.6; }
.ahc-empty-note { margin-top: -8px; font-size: 12px; }
.ahc-task { flex-shrink: 0; max-height: 34vh; overflow: auto; margin: 10px 20px 0; padding: 9px 12px; border: 1px solid var(--line,#e5e6e1); border-radius: 10px; font-size: 12px; overflow-wrap: anywhere; }
.ahc-task>summary { display: flex; gap: 10px; cursor: pointer; }
.ahc-task>summary span { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ahc-task p { margin: 6px 0; white-space: pre-wrap; }
.ahc-task ul,.ahc-task ol { padding-left: 20px; }
.ahc-task small { color: var(--app-muted,#72746f); }
.ahc-task-warning { color: #a35d12; }
.ahc-inner { width: 100%; min-width: 0; margin: 6px 0 0; color: var(--app-muted, #72746f); font-size: 12px; }
.ahc-inner>summary { display: flex; align-items: center; gap: 8px; width: fit-content; max-width: 100%; min-height: 28px; padding: 2px 4px; border-radius: 6px; cursor: pointer; list-style: none; user-select: none; }
.ahc-inner>summary:hover { background: color-mix(in srgb, var(--app-ink, #252624) 5%, transparent); color: var(--app-ink, #252624); }
.ahc-inner>summary:focus-visible { outline: 2px solid var(--app-accent, #3b82f6); outline-offset: 2px; }
.ahc-inner>summary::-webkit-details-marker { display: none; }
.ahc-inner-phase { flex-shrink: 0; font-weight: 500; }
.ahc-inner-meta { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ahc-inner-chevron { flex: 0 0 auto; transition: transform .15s ease; }
.ahc-inner[open]>summary .ahc-inner-chevron { transform: rotate(180deg); }
.ahc-inner-body { display: grid; gap: 10px; max-height: min(360px, 45vh); overflow: auto; margin: 4px 0 0 4px; padding: 10px 12px; border-left: 2px solid var(--line, #e5e6e1); overflow-wrap: anywhere; }
.ahc-inner-body strong { display: block; color: var(--app-ink, #252624); font-weight: 500; }
.ahc-inner-body p, .ahc-inner-body li { margin: 2px 0 0; line-height: 1.55; white-space: pre-wrap; }
.ahc-inner-body ul { margin: 2px 0 0; padding-left: 18px; }
.ahc-reply-pending { color: var(--app-muted, #72746f); font-size: 12px; }
@media(prefers-reduced-motion:reduce){.ahc-inner-chevron{transition:none}.ahc-inner .ahc-pulse{animation:none}}
.agent-host-chat-failure { display: flex; flex-direction: column; gap: 2px; margin-top: 6px; padding: 8px 10px; border-radius: 8px; font-size: 12px; white-space: normal; background: color-mix(in srgb, #d14343 9%, transparent); color: var(--app-ink, inherit); }
.agent-host-chat-failure strong, .agent-host-chat-alert strong { color: #c03535; font-weight: 600; }
.agent-host-chat-failure code, .agent-host-chat-alert code { font-size: 10px; opacity: .55; }
.ahc-refusal { margin: 6px 0 0; color: var(--app-muted, #72746f); font-size: 12px; line-height: 1.5; }
.ahc-intent { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
.ahc-intent button { border: 1px solid var(--line, #e5e6e1); border-radius: 999px; background: transparent; color: var(--app-ink, #252624); font: inherit; font-size: 12px; padding: 4px 10px; cursor: pointer; }
.agent-host-chat-alert { margin: 0 16px; padding: 8px 12px; border-radius: 8px; font-size: 12px; background: color-mix(in srgb, #d14343 9%, transparent); }
.ahc-select-wrap { position: relative; display: inline-flex; align-items: center; }
.ahc-select { appearance: none; -webkit-appearance: none; height: 30px; max-width: 180px; padding: 0 26px 0 10px; font: inherit; font-size: 12px; color: inherit; border: 1px solid color-mix(in srgb, var(--app-ink, #172033) 14%, transparent); border-radius: 8px; background: var(--app-surface, #fff); box-shadow: none !important; cursor: pointer; text-overflow: ellipsis; }
.ahc-select:hover { border-color: color-mix(in srgb, var(--app-ink, #172033) 28%, transparent); }
.ahc-select:disabled { opacity: .6; cursor: default; }
.ahc-select-caret { position: absolute; right: 8px; pointer-events: none; opacity: .6; }
.ahc-model .ahc-select { max-width: 280px; font-weight: 550; }
.ahc-empty { margin: auto; text-align: center; font-size: 14px; line-height: 1.7; color: var(--app-muted, var(--muted, #6b7280)); max-width: 420px; }
.ahc-grant{width:min(480px,calc(100% - 40px));margin:0 auto 10px;padding:14px 16px;border:1px solid var(--line,#e5e6e1);border-radius:14px;background:var(--app-surface,#fff);box-shadow:0 10px 28px #00000014}
.ahc-grant strong{display:block;font-size:14px}
.ahc-grant p,.ahc-grant li{margin:6px 0 0;color:var(--app-muted,#72746f);font-size:12px;line-height:1.6}
.ahc-grant ul{margin:8px 0 0;padding-left:18px}
.ahc-grant footer{display:flex;justify-content:flex-end;gap:8px;margin-top:12px}
.ahc-composer { display: flex; align-items: flex-end; gap: 8px; width: 100%; margin: 0; padding: 12px 12px 12px 16px; border: 1px solid color-mix(in srgb, var(--app-ink, #172033) 14%, transparent); border-radius: 22px; background: var(--app-surface, #fff); box-shadow: 0 8px 28px #0000000a; }
.ahc-composer:focus-within { border-color: color-mix(in srgb, var(--app-ink, #172033) 32%, transparent); }
.ahc-composer textarea { flex: 1; width: 100%; min-height: 44px; max-height: 160px; padding: 4px 0; border: 0 !important; outline: none; resize: none; background: transparent !important; box-shadow: none !important; color: inherit; font: inherit; font-size: 14px; line-height: 1.6; field-sizing: content; }
.ahc-send { flex-shrink: 0; width: 32px; height: 32px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 50%; background: var(--app-ink, #172033); color: var(--app-bg, #fff); cursor: pointer; }
.ahc-heartbeat { display: flex; align-items: center; gap: 6px; margin: 0; font-size: 12px; color: var(--app-muted, var(--muted, #6b7280)); }
.ahc-pulse { width: 7px; height: 7px; border-radius: 50%; background: #22a06b; animation: ahc-pulse 1.4s ease-in-out infinite; }
@keyframes ahc-pulse { 0%, 100% { opacity: .35; } 50% { opacity: 1; } }
.ahc-steer { background: var(--app-accent, #3b82f6); }
.ahc-steer-state,.ahc-model-usage { display: block; color: var(--app-text-muted, #737373); font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
.ahc-msg-actions { display: flex; gap: 2px; margin-top: 4px; opacity: 0; transition: opacity .15s; }
.agent-host-chat-message:hover .ahc-msg-actions, .ahc-msg-actions:focus-within { opacity: 1; }
.agent-host-chat-message.user .ahc-msg-actions { justify-content: flex-end; }
.ahc-msg-actions button { width: 24px; height: 24px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 6px; background: transparent; color: var(--app-muted, var(--muted, #6b7280)); cursor: pointer; }
.ahc-msg-actions button:hover { background: color-mix(in srgb, var(--app-ink, #172033) 8%, transparent); }
.ahc-send:disabled { opacity: .3; cursor: default; }
.ahc-remember { display:flex; align-items:flex-start; gap:8px; margin-top:12px; font-size:12px; line-height:1.6; }
.ahc-remember input { flex:0 0 auto; width:16px; height:16px; margin:2px 0 0; }
@media(max-width:900px){.agent-host-chat-transcript{padding:16px}.ahc-model .ahc-select{max-width:240px}}

.ahc-compose-dock{flex-shrink:0;position:relative;width:min(800px,calc(100% - 40px));margin:8px auto 12px}
.ahc-composer-controls{display:flex;align-items:center;gap:8px;min-height:34px;padding:6px 8px 0;color:var(--app-muted,#72746f)}
.ahc-composer-controls .ahc-select{max-width:280px;height:26px;padding-left:0;border-color:transparent;background:transparent;font-size:11px;font-weight:500}
.ahc-permissions-toggle{display:flex;align-items:center;gap:5px;padding:5px 7px;border:0;border-radius:6px;background:transparent;color:inherit;font:inherit;font-size:11px;cursor:pointer}
.ahc-permissions-toggle:hover,.ahc-permissions-toggle[aria-expanded="true"]{background:color-mix(in srgb,var(--app-ink,#252624) 6%,transparent);color:var(--app-ink,#252624)}
.ahc-composer-hint{margin-left:auto;font-size:10px;white-space:nowrap}
.ahc-permissions-panel{position:absolute;bottom:38px;left:0;z-index:25;width:min(420px,100%);max-height:44vh;overflow:auto;padding:14px 16px;border:1px solid var(--line,#e5e6e1);border-radius:12px;background:var(--app-surface,#fff);box-shadow:0 8px 28px #00000014;font-size:12px;line-height:1.6}
.ahc-permissions-panel header{display:flex;justify-content:space-between;align-items:center}.ahc-permissions-panel p{margin:8px 0}.ahc-permissions-panel ul{padding-left:18px;margin:6px 0}.ahc-permissions-panel button{padding:4px 0;border:0;background:transparent;color:var(--blue,#386fe5);font:inherit;font-size:11px;cursor:pointer}.ahc-permissions-help{color:var(--app-muted,#72746f);font-size:11px}
@media(max-width:650px){.ahc-compose-dock{width:calc(100% - 24px)}.ahc-composer-controls .ahc-select{max-width:210px}.ahc-composer-hint{display:none}.ahc-task{margin:8px 12px 0}}
</style>
