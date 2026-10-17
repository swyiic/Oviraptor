<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { Plus, Square, ArrowUp } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { AgentHostSettings } from "../types";
import { describeAgentHostError } from "../errors";
import { agentRuntimeHeader, type AgentHostChatClient, type AgentHostConversation, type AgentHostMessage } from "../chatClient";

const props = defineProps<{ client: AgentHostChatClient; settings: AgentHostSettings; projectName: string; projectId?: number }>();
const emit = defineEmits<{ useLegacy: [] }>();
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
const header = computed(() => agentRuntimeHeader(props.settings));
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
    <header class="conversation-header">
      <div class="conversation-heading">
        <h2>{{ selected?.title || tr('新想法，新对话', 'A fresh conversation') }}</h2>
        <span data-testid="agent-host-chat-runtime">{{ header }} · {{ projectName }}</span>
      </div>
      <div class="conversation-header-actions">
        <select v-if="conversations.length" v-model="selectedId" class="agent-host-chat-select" :aria-label="tr('选择对话', 'Select conversation')">
          <option v-for="item in conversations" :key="item.id" :value="item.id">{{ item.title }}</option>
        </select>
        <button class="chat-icon-button" :title="tr('新对话', 'New chat')" :aria-label="tr('新对话', 'New chat')" @click="newConversation"><Plus :size="18" /></button>
        <button class="button ghost compact" type="button" @click="emit('useLegacy')">{{ tr('旧版对话', 'Legacy chat') }}</button>
      </div>
    </header>
    <div ref="transcript" class="agent-host-chat-transcript">
      <p v-if="!messages.length" class="conversation-muted">{{ tr('已接入 xAI Grok（Agent runtime）。提问会经授权只读工具核实。', 'Connected to xAI Grok agent runtime. Answers use permission-scoped read-only tools.') }}</p>
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
    <form class="agent-host-chat-composer" @submit.prevent="send">
      <textarea v-model="draft" rows="3" :placeholder="tr('问点什么…（Enter 发送，Shift+Enter 换行）', 'Ask anything… (Enter to send)')" @keydown.enter.exact.prevent="send" />
      <button v-if="running" type="button" class="chat-icon-button" :aria-label="tr('停止', 'Stop')" @click="cancel"><Square :size="16" /></button>
      <button v-else type="submit" class="chat-icon-button" :disabled="!draft.trim()" :aria-label="tr('发送', 'Send')"><ArrowUp :size="16" /></button>
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
.agent-host-chat-composer { display: flex; gap: 8px; padding: 12px 16px; align-items: flex-end; }
.agent-host-chat-composer textarea { flex: 1; resize: vertical; }
.agent-host-chat-failure { display: flex; flex-direction: column; gap: 2px; margin-top: 6px; padding: 8px 10px; border-radius: 8px; font-size: 12px; white-space: normal; background: color-mix(in srgb, #d14343 9%, transparent); color: var(--app-ink, inherit); }
.agent-host-chat-failure strong, .agent-host-chat-alert strong { color: #c03535; font-weight: 600; }
.agent-host-chat-failure code, .agent-host-chat-alert code { font-size: 10px; opacity: .55; }
.agent-host-chat-alert { margin: 0 16px; padding: 8px 12px; border-radius: 8px; font-size: 12px; background: color-mix(in srgb, #d14343 9%, transparent); }
.agent-host-chat-select { max-width: 200px; }
</style>
