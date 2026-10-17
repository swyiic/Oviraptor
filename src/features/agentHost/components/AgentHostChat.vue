<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { Plus, Square, ArrowUp, Ellipsis } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { AgentHostSettings } from "../types";
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
  else if (event.type === "runtime.error") { error.value = String(event.code ?? "runtime_error"); }
});
onBeforeUnmount(stop);
watch(() => [props.projectName, props.projectId], () => void loadProject(), { immediate: true });
watch(selectedId, () => { steps.value = []; void loadMessages(); });
</script>

<template>
  <div class="conversation-main agent-host-chat" data-testid="agent-host-chat">
    <header class="conversation-header">
      <div class="conversation-heading">
        <h2>{{ selected?.title || tr('新对话', 'New chat') }}</h2>
        <span data-testid="agent-host-chat-runtime">{{ header }} · {{ projectName }}</span>
      </div>
      <div class="conversation-header-actions">
        <select v-if="conversations.length" v-model="selectedId" class="agent-host-chat-select" :aria-label="tr('选择对话', 'Select conversation')">
          <option v-for="item in conversations" :key="item.id" :value="item.id">{{ item.title }}</option>
        </select>
        <button class="chat-icon-button" :title="tr('新对话', 'New chat')" :aria-label="tr('新对话', 'New chat')" @click="newConversation"><Plus :size="18" /></button>
        <details class="agent-host-more">
          <summary class="chat-icon-button" :aria-label="tr('更多', 'More')"><Ellipsis :size="18" /></summary>
          <div class="agent-host-more-menu">
            <button type="button" @click="emit('useLegacy')">{{ tr('切换到旧版对话', 'Use legacy chat') }}</button>
          </div>
        </details>
      </div>
    </header>
    <div ref="transcript" class="agent-host-chat-transcript">
      <p v-if="!messages.length" class="conversation-muted">{{ tr('已接入 Grok。直接提问即可。', 'Connected to Grok. Ask anything.') }}</p>
      <article v-for="message in messages" :key="message.id" :class="['agent-host-chat-message', message.role]">
        <p>{{ message.content || (message.status === 'running' ? tr('思考中…', 'Thinking…') : '') }}</p>
        <small v-if="message.status === 'failed'" class="error">{{ message.failureCode || tr('失败', 'Failed') }}</small>
      </article>
      <ul v-if="running && steps.length" class="agent-host-chat-steps"><li v-for="step in steps" :key="step">{{ step }}</li></ul>
    </div>
    <p v-if="error" class="error" role="alert">{{ error }}</p>
    <form class="agent-host-chat-composer" @submit.prevent="send">
      <textarea v-model="draft" rows="1" :placeholder="tr('问点什么…', 'Ask anything…')" @keydown.enter.exact.prevent="send" />
      <button v-if="running" type="button" class="chat-icon-button" :aria-label="tr('停止', 'Stop')" @click="cancel"><Square :size="16" /></button>
      <button v-else type="submit" class="chat-icon-button" :disabled="!draft.trim()" :aria-label="tr('发送', 'Send')"><ArrowUp :size="16" /></button>
    </form>
  </div>
</template>

<style scoped>
.agent-host-chat { display: flex; flex-direction: column; flex: 1; min-width: 0; min-height: 0; width: 100%; height: 100%; }
.agent-host-chat-transcript { flex: 1; overflow: auto; padding: 12px 16px; display: flex; flex-direction: column; gap: 8px; }
.agent-host-chat-message { max-width: 78%; padding: 8px 12px; border-radius: 12px; white-space: pre-wrap; font-size: 13px; line-height: 1.5; }
.agent-host-chat-message.user { align-self: flex-end; background: var(--surface-strong, rgba(127,127,127,.15)); }
.agent-host-chat-message.assistant { align-self: flex-start; background: var(--surface, rgba(127,127,127,.07)); }
.agent-host-chat-message p { margin: 0; }
.agent-host-chat-steps { font-size: 11px; opacity: .7; margin: 0; padding-left: 18px; }
.agent-host-chat-composer {
  display: flex; gap: 8px; align-items: flex-end;
  padding: 10px 16px 14px;
  border-top: 1px solid var(--line, rgba(0,0,0,.06));
}
.agent-host-chat-composer textarea {
  flex: 1; resize: none; min-height: 40px; max-height: 120px;
  padding: 10px 12px; border: 1px solid var(--line); border-radius: 12px;
  background: var(--app-surface, #fff); font-size: 13px; line-height: 1.4;
}
.agent-host-chat-select { max-width: 180px; height: 32px; font-size: 12px; }
.agent-host-more { position: relative; }
.agent-host-more > summary { list-style: none; cursor: pointer; }
.agent-host-more > summary::-webkit-details-marker { display: none; }
.agent-host-more-menu {
  position: absolute; right: 0; top: 36px; z-index: 20;
  min-width: 160px; padding: 6px;
  border: 1px solid var(--line); border-radius: 10px;
  background: var(--app-surface, #fff); box-shadow: 0 10px 28px #0002;
}
.agent-host-more-menu button {
  display: block; width: 100%; text-align: left; border: 0; background: transparent;
  padding: 8px 10px; border-radius: 7px; font-size: 12px; color: var(--app-ink, #172033); cursor: pointer;
}
.agent-host-more-menu button:hover { background: color-mix(in srgb, var(--app-ink, #172033) 6%, transparent); }
.conversation-header { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 16px; min-height: 52px; border-bottom: 1px solid var(--line, rgba(0,0,0,.06)); }
.conversation-heading h2 { margin: 0; font-size: 14px; font-weight: 600; }
.conversation-heading span { display: block; margin-top: 2px; font-size: 11px; color: var(--muted); }
.conversation-header-actions { display: flex; align-items: center; gap: 6px; }
.conversation-muted { margin: 24px 0; font-size: 12px; color: var(--muted); }
.error { color: var(--red, #c44); font-size: 12px; margin: 0 16px 8px; }
</style>
