<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { RefreshCw } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import { resolveAgentHostEndpoint } from "../../agentHost/api";
import { createAgentHostChatClient, type AgentHostChatClient } from "../../agentHost/chatClient";
import { BUILTIN_PLUGINS } from "../../../plugins";

const props = defineProps<{
  pluginEnabled: Record<string, boolean>;
  pluginPreferenceError?: string;
  restoringBuiltinApps?: boolean;
  hackerOneEnabled?: boolean;
}>();
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  togglePlugin: [id: string, event: Event];
  restoreBuiltinApps: [];
}>();
const { tr } = useI18n();
const client = ref<AgentHostChatClient>();
const apps = ref<Array<{ id: string; enabled: boolean; version?: string; manifest?: { name?: string; permissions?: Array<{ id: string }> } }>>([]);
const busy = ref(false);
const error = ref("");

const APP_LABELS: Record<string, [string, string]> = {
  "org.oviraptor.assets": ["资产", "Assets"],
  "org.oviraptor.scanning": ["扫描", "Scanning"],
};

function appTitle(app: { id: string; manifest?: { name?: string } }) {
  const mapped = APP_LABELS[app.id];
  if (mapped) return tr(...mapped);
  return app.manifest?.name || app.id.replace(/^org\.oviraptor\./, "");
}

async function refresh() {
  error.value = "";
  try {
    const endpoint = await resolveAgentHostEndpoint();
    if (!endpoint) { client.value = undefined; apps.value = []; return; }
    client.value = createAgentHostChatClient(endpoint);
    const listed = await client.value.listApps();
    apps.value = listed.items ?? [];
  } catch (e) {
    error.value = String((e as Error).message || e);
  }
}
async function toggleApp(appId: string, event: Event) {
  const enabled = Boolean((event.target as HTMLInputElement | null)?.checked);
  if (!client.value || busy.value) return;
  busy.value = true;
  try {
    await client.value.setAppEnabled(appId, enabled);
    await refresh();
    emit("notify", "success", enabled ? tr("应用已启用", "App enabled") : tr("应用已停用", "App disabled"));
  } catch (e) {
    emit("notify", "error", String((e as Error).message || e));
  } finally { busy.value = false; }
}
async function ensureBundled() {
  if (!client.value || busy.value) return;
  busy.value = true;
  try {
    await client.value.ensureBundled();
    await refresh();
    emit("notify", "success", tr("已恢复内置应用", "Built-in apps restored"));
  } catch (e) {
    emit("notify", "error", String((e as Error).message || e));
  } finally { busy.value = false; }
}
function restoreAll() {
  emit("restoreBuiltinApps");
  void ensureBundled();
}
onMounted(() => void refresh());
watch(() => props.restoringBuiltinApps, (v, prev) => { if (prev && !v) void refresh(); });

const restoring = computed(() => Boolean(props.restoringBuiltinApps || busy.value));
</script>
<template>
  <div class="config-section apps panel" data-testid="config-section-apps">
    <div class="config-section-head">
      <h3>{{ tr("已装应用", "Installed apps") }}</h3>
      <div class="actions">
        <button class="button ghost compact" type="button" :disabled="restoring" @click="refresh">
          <RefreshCw :size="13" />{{ tr("刷新", "Refresh") }}
        </button>
        <button class="button secondary compact" type="button" data-testid="restore-builtin-apps" :disabled="restoring" @click="restoreAll">
          <RefreshCw :size="13" :class="{ spinning: restoring }" />
          {{ restoring ? tr("恢复中…", "Restoring…") : tr("恢复内置", "Restore") }}
        </button>
      </div>
    </div>

    <p v-if="pluginPreferenceError" class="host-action-error" role="alert">{{ pluginPreferenceError }}</p>
    <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>

    <div class="group-label">{{ tr("侧栏", "Sidebar") }}</div>
    <ul class="config-dense-list">
      <li v-for="plugin in BUILTIN_PLUGINS" :key="plugin.id" class="config-dense-row">
        <div>
          <p class="title">{{ tr(...plugin.label) }}</p>
          <span class="meta">{{ plugin.id }}<template v-if="plugin.id === 'hackerone' && !hackerOneEnabled"> · {{ tr("需配置 API", "API required") }}</template></span>
        </div>
        <label class="switch">
          <input type="checkbox" :checked="pluginEnabled[plugin.id]" :aria-label="tr(...plugin.label)" @change="emit('togglePlugin', plugin.id, $event)" />
          {{ pluginEnabled[plugin.id] ? tr("开", "On") : tr("关", "Off") }}
        </label>
      </li>
    </ul>

    <div class="group-label">{{ tr("Agent 工具", "Agent tools") }}</div>
    <p v-if="!client" class="config-muted-line">{{ tr("Agent-host 不可用", "Agent-host unavailable") }}</p>
    <ul v-else class="config-dense-list">
      <li v-for="app in apps" :key="app.id" class="config-dense-row">
        <div>
          <p class="title">{{ appTitle(app) }}</p>
          <span class="meta">{{ app.id }}<template v-if="app.version"> · v{{ app.version }}</template></span>
        </div>
        <label class="switch">
          <input type="checkbox" :checked="app.enabled" :disabled="busy" @change="toggleApp(app.id, $event)" />
          {{ app.enabled ? tr("开", "On") : tr("关", "Off") }}
        </label>
      </li>
      <li v-if="!apps.length" class="config-muted-line" style="padding:8px 0;border:0">{{ tr("暂无 Agent 应用，点「恢复内置」", "No agent apps yet — Restore") }}</li>
    </ul>
  </div>
</template>
<style scoped>
.group-label {
  margin: 14px 0 4px;
  font-size: 10px;
  font-weight: 650;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--muted);
}
.group-label:first-of-type { margin-top: 4px; }
</style>
