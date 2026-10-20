<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
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
async function toggleApp(appId: string, enabled: boolean) {
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
    const result = await client.value.ensureBundled();
    const failed = result.items?.find(item => item.error);
    if (failed?.error) throw Error(`${failed.id}: ${failed.error}`);
    await refresh();
    emit("notify", "success", tr("内置组件已更新", "Built-in components updated"));
  } catch (e) {
    emit("notify", "error", String((e as Error).message || e));
  } finally { busy.value = false; }
}
onMounted(() => void refresh());
watch(() => props.restoringBuiltinApps, (v, prev) => { if (prev && !v) void refresh(); });
</script>
<template>
  <div class="config-section apps settings-page" data-testid="config-section-apps">
    <section class="settings-block">
      <header class="settings-block-head">
        <div>
          <h3>{{ tr("侧栏", "Sidebar") }}</h3>
          <p>{{ tr("关掉只是藏起入口。已经开始的任务会做完。", "Turning one off only hides its entry. Work already started finishes.") }}</p>
        </div>
        <button class="button ghost compact" type="button" data-testid="restore-builtin-apps" :disabled="restoringBuiltinApps" @click="emit('restoreBuiltinApps')">
          <RefreshCw :size="14" :class="{ spinning: restoringBuiltinApps }" />
          {{ restoringBuiltinApps ? tr("恢复中…", "Restoring…") : tr("恢复", "Restore") }}
        </button>
      </header>
      <p v-if="pluginPreferenceError" class="host-action-error" role="alert">{{ pluginPreferenceError }}</p>
      <article v-for="plugin in BUILTIN_PLUGINS" :key="plugin.id" class="settings-row">
        <div>
          <strong>{{ tr(...plugin.label) }}</strong>
          <small>{{ tr(...plugin.description) }}</small>
        </div>
        <label class="settings-switch">
          <input type="checkbox" :checked="pluginEnabled[plugin.id]" :aria-label="tr(...plugin.label)" @change="emit('togglePlugin', plugin.id, $event)" />
        </label>
      </article>
    </section>
    <section class="settings-block">
      <header class="settings-block-head">
        <div>
          <h3>{{ tr("问答可以调用的应用", "Apps chat can call") }}</h3>
          <p>{{ tr("更新内置组件会通过独立数据副本升级，保留原版本。读取数据仍需按工作空间授权。", "Grant read access per workspace under Data permissions.") }}</p>
        </div>
        <div class="settings-block-actions">
          <button class="button ghost compact" type="button" :disabled="busy" @click="refresh"><RefreshCw :size="14" />{{ tr("刷新", "Refresh") }}</button>
          <button class="button ghost compact" type="button" :disabled="busy || !client" @click="ensureBundled">{{ tr("更新内置组件", "Update bundled components") }}</button>
        </div>
      </header>
      <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>
      <p v-else-if="!client" class="settings-note">{{ tr("问答服务还没连上。", "Chat service is not connected.") }}</p>
      <article v-for="app in apps" :key="app.id" class="settings-row">
        <div>
          <strong>{{ app.id.includes('assets') ? tr('资产', 'Assets') : app.id.includes('scanning') ? tr('扫描', 'Scanning') : (app.manifest?.name || app.id) }}</strong>
          <small>{{ app.id }}</small>
        </div>
        <label class="settings-switch">
          <input type="checkbox" :checked="app.enabled" :disabled="busy" :aria-label="app.id" @change="toggleApp(app.id, ($event.target as HTMLInputElement).checked)" />
        </label>
      </article>
    </section>
  </div>
</template>
