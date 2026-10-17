<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { Plus, Square, ArrowUp, ChevronDown } from "@lucide/vue";
import { SESSION_MODEL_PRESETS } from "../providerPresets";
import { useI18n } from "../../../i18n";
import type { AgentHostSettings } from "../types";
import { describeAgentHostError } from "../errors";
import { agentRuntimeHeader, type AgentHostChatClient, type AgentHostConversation, type AgentHostMessage } from "../chatClient";

const props = defineProps<{ client: AgentHostChatClient; settings: AgentHostSettings; projectName: string; projectId?: number }>();
const emit = defineEmits<{ useLegacy: []; settingsChanged: [settings: AgentHostSettings] }>();
const { tr } = useI18n();

const hostProjectId = ref<number>();
const conversations = ref<AgentHostConversation[]>([]);
const selectedId = ref<string>();
const messages = ref<AgentHostMessage[]>([]);
const steps = ref<string[]>([]);
const draft = ref("");
const running = ref(false);
const error = ref("");
const transcript = ref<HTMLElement>();
const liveSettings = ref<AgentHostSettings>(props.settings);
watch(() => props.settings, value => { liveSettings.value = value; });
const header = computed(() => agentRuntimeHeader(liveSettings.value));
const currentModel = computed(() => liveSettings.value?.apiProvider?.model || "");
const modelOptions = computed(() => {
  const list = SESSION_MODEL_PRESETS.map(item => ({ value: item.model, label: item.label }));
  if (currentModel.value && !list.some(item => item.value === currentModel.value)) list.unshift({ value: currentModel.value, label: currentModel.value });
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
const selected = computed(() => conversations.value.find(item => item.id === selectedId.value));

async function scroll() { await nextTick(); transcript.value?.scrollTo({ top: transcript.value.scrollHeight }); }
async function loadMessages() {
  if (!selectedId.value || hostProjectId.value === undefined) { messages.value = []; return; }
  messages.value = await props.client.messages(selectedId.value, hostProjectId.value);
  running.value = messages.value.some(item => item.status === "running");
  await scroll();
}
async function loadProject() {
  error.value = "";
  try {
    hostProjectId.value = await props.client.ensureProject(props.projectName, props.projectId);
    conversations.value = await props.client.conversations(hostProjectId.value);
    selectedId.value = conversations.value[conversations.value.length - 1]?.id;
    await loadMessages();
  } catch (e) { error.value = String((e as Error).message || e); }
}
async function newConversation() {
  if (hostProjectId.value === undefined) return;
  const created = await props.client.createConversation(hostProjectId.value, tr("新对话", "New conversation"));
  conversations.value = [...conversations.value, created];
  selectedId.value = created.id; messages.value = []; steps.value = [];
}
async function send() {
  const text = draft.value.trim();
  if (!text || running.value || hostProjectId.value === undefined) return;
  error.value = "";
  if (!selectedId.value) await newConversation();
  draft.value = ""; steps.value = []; running.value = true;
  try { await props.client.send(selectedId.value!, hostProjectId.value, text); await loadMessages(); }
  catch (e) { running.value = false; error.value = String((e as Error).message || e); }
}
async function cancel() { if (selectedId.value) await props.client.cancel(selectedId.value).catch(() => {}); }

const stop = props.client.events(event => {
  if (!selectedId.value || event.conversationId !== selectedId.value) return;
  if (event.type === "message.delta" && event.messageId) {
    const target = messages.value.find(item => item.id === event.messageId);
    if (target) { target.content += String(event.delta ?? ""); void scroll(); } else void loadMessages();
  } else if (event.type === "runtime.step" && event.title) {
    const title = String(event.title); if (!steps.value.includes(title)) steps.value = [...steps.value, title].slice(-8);
  } else if (event.type === "message.completed") { running.value = false; void loadMessages(); }
  // runtime.error is shown on the failed assistant bubble (reloaded on completion); no duplicate banner.
  else if (event.type === "runtime.error") { void loadMessages(); }
});
onBeforeUnmount(stop);
watch(() => [props.projectName, props.projectId], () => void loadProject(), { immediate: true });
watch(selectedId, () => { steps.value = []; void loadMessages(); });
</script>

<template>
  <div class="conversation-main agent-host-chat" data-testid="agent-host-chat">
    <header class="ahc-toolbar">
      <div class="ahc-toolbar-left">
        <div class="ahc-title">
          <h2 :title="selected?.title">{{ selected?.title || tr('新对话', 'New conversation') }}</h2>
          <span class="ahc-subtitle">{{ projectName }}</span>
        </div>
        <label v-if="conversations.length > 1" class="ahc-select-wrap">
          <select v-model="selectedId" class="ahc-select" :aria-label="tr('切换对话', 'Switch conversation')" data-testid="agent-host-session-switcher">
            <option v-for="item in conversations" :key="item.id" :value="item.id">{{ item.title }}</option>
          </select>
          <ChevronDown :size="14" class="ahc-select-caret" />
        </label>
        <button class="ahc-icon-button" type="button" :title="tr('新对话', 'New chat')" :aria-label="tr('新对话', 'New chat')" @click="newConversation"><Plus :size="16" /></button>
      </div>
      <div class="ahc-toolbar-right">
        <label class="ahc-select-wrap ahc-model" :title="header" data-testid="agent-host-chat-runtime">
          <select class="ahc-select" data-testid="agent-host-model-picker" :value="currentModel" :disabled="switchingModel || running" :aria-label="tr('会话模型', 'Session model')" @change="switchModel">
            <option v-for="item in modelOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
          </select>
          <ChevronDown :size="14" class="ahc-select-caret" />
        </label>
        <button class="ahc-legacy-link" type="button" @click="emit('useLegacy')">{{ tr('旧版对话', 'Legacy chat') }}</button>
      </div>
    </header>
    <div ref="transcript" class="agent-host-chat-transcript">
      <p v-if="!messages.length" class="ahc-empty">{{ tr(`当前模型：${header}。提问会经授权只读工具核实。`, `Model: ${header}. Answers use permission-scoped read-only tools.`) }}</p>
      <article v-for="message in messages" :key="message.id" :class="['agent-host-chat-message', message.role]">
        <p>{{ message.content || (message.status === 'running' ? tr('思考中…', 'Thinking…') : '') }}</p>
        <div v-if="message.status === 'failed'" class="agent-host-chat-failure" role="note">
          <strong>{{ tr(...failure(message.failureCode).title) }}</strong>
          <span>{{ tr(...failure(message.failureCode).hint) }}</span>
          <code :title="tr('错误码', 'Error code')">{{ failure(message.failureCode).code }}</code>
        </div>
      </article>
      <ul v-if="running && steps.length" class="agent-host-chat-steps"><li v-for="step in steps" :key="step">{{ step }}</li></ul>
    </div>
    <div v-if="errorView" class="agent-host-chat-alert" role="alert">
      <strong>{{ tr(...errorView.title) }}</strong> <span>{{ tr(...errorView.hint) }}</span>
      <code>{{ errorView.code }}</code>
    </div>
    <form class="ahc-composer" @submit.prevent="send">
      <textarea v-model="draft" rows="1" :placeholder="tr('问点什么…（Enter 发送，Shift+Enter 换行）', 'Ask anything… (Enter to send)')" @keydown.enter.exact.prevent="send" />
      <button v-if="running" type="button" class="ahc-send" :aria-label="tr('停止', 'Stop')" @click="cancel"><Square :size="14" /></button>
      <button v-else type="submit" class="ahc-send" :disabled="!draft.trim()" :aria-label="tr('发送', 'Send')"><ArrowUp :size="16" /></button>
    </form>
  </div>
</template>

<style scoped>
.agent-host-chat { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; width: 100%; height: 100%; }
.agent-host-chat-transcript { flex: 1; overflow: auto; padding: 16px; display: flex; flex-direction: column; gap: 10px; }
.agent-host-chat-message { max-width: 80%; padding: 10px 12px; border-radius: 12px; white-space: pre-wrap; }
.agent-host-chat-message.user { align-self: flex-end; background: var(--surface-strong, rgba(127,127,127,.15)); }
.agent-host-chat-message.assistant { align-self: flex-start; background: var(--surface, rgba(127,127,127,.07)); }
.agent-host-chat-message p { margin: 0; }
.agent-host-chat-steps { font-size: 12px; opacity: .7; margin: 0; padding-left: 18px; }
.agent-host-chat-failure { display: flex; flex-direction: column; gap: 2px; margin-top: 6px; padding: 8px 10px; border-radius: 8px; font-size: 12px; white-space: normal; background: color-mix(in srgb, #d14343 9%, transparent); color: var(--app-ink, inherit); }
.agent-host-chat-failure strong, .agent-host-chat-alert strong { color: #c03535; font-weight: 600; }
.agent-host-chat-failure code, .agent-host-chat-alert code { font-size: 10px; opacity: .55; }
.agent-host-chat-alert { margin: 0 16px; padding: 8px 12px; border-radius: 8px; font-size: 12px; background: color-mix(in srgb, #d14343 9%, transparent); }
.ahc-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-shrink: 0; min-height: 48px; padding: 8px 20px; border-bottom: 1px solid color-mix(in srgb, var(--app-ink, #172033) 8%, transparent); }
.ahc-toolbar-left, .ahc-toolbar-right { display: flex; align-items: center; gap: 6px; min-width: 0; }
.ahc-toolbar-right { flex-shrink: 0; }
.ahc-title { min-width: 0; margin-right: 6px; }
.ahc-title h2 { margin: 0; font-size: 14px; font-weight: 600; line-height: 1.3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 280px; }
.ahc-subtitle { display: block; font-size: 11px; color: var(--app-muted, var(--muted, #6b7280)); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 280px; }
.ahc-select-wrap { position: relative; display: inline-flex; align-items: center; }
.ahc-select { appearance: none; -webkit-appearance: none; height: 30px; max-width: 180px; padding: 0 26px 0 10px; font: inherit; font-size: 12px; color: inherit; border: 1px solid color-mix(in srgb, var(--app-ink, #172033) 14%, transparent); border-radius: 8px; background: var(--app-surface, transparent); cursor: pointer; text-overflow: ellipsis; }
.ahc-select:hover { border-color: color-mix(in srgb, var(--app-ink, #172033) 28%, transparent); }
.ahc-select:disabled { opacity: .6; cursor: default; }
.ahc-select-caret { position: absolute; right: 8px; pointer-events: none; opacity: .6; }
.ahc-model .ahc-select { max-width: 200px; font-weight: 500; }
.ahc-icon-button { width: 30px; height: 30px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 8px; background: transparent; color: var(--app-muted, var(--muted, #6b7280)); cursor: pointer; }
.ahc-icon-button:hover { background: color-mix(in srgb, var(--app-ink, #172033) 7%, transparent); }
.ahc-legacy-link { border: 0; background: none; padding: 0 4px; font: inherit; font-size: 11px; color: var(--app-muted, var(--muted, #6b7280)); opacity: .75; cursor: pointer; text-decoration: underline; text-underline-offset: 2px; }
.ahc-legacy-link:hover { opacity: 1; }
.ahc-empty { margin: auto 0; text-align: center; font-size: 13px; color: var(--app-muted, var(--muted, #6b7280)); }
.ahc-composer { display: flex; align-items: flex-end; gap: 8px; margin: 8px 20px 16px; padding: 10px 10px 10px 14px; border: 1px solid color-mix(in srgb, var(--app-ink, #172033) 14%, transparent); border-radius: 16px; background: var(--app-surface, rgba(127,127,127,.04)); box-shadow: 0 1px 3px rgba(0,0,0,.05); }
.ahc-composer:focus-within { border-color: color-mix(in srgb, var(--app-accent, #3b82f6) 55%, transparent); }
.ahc-composer textarea { flex: 1; min-height: 24px; max-height: 200px; padding: 4px 0; border: 0; outline: none; resize: none; background: transparent; color: inherit; font: inherit; font-size: 14px; line-height: 1.5; field-sizing: content; }
.ahc-send { flex-shrink: 0; width: 32px; height: 32px; display: grid; place-items: center; padding: 0; border: 0; border-radius: 50%; background: var(--app-ink, #172033); color: var(--app-bg, #fff); cursor: pointer; }
.ahc-send:disabled { opacity: .3; cursor: default; }
</style>
