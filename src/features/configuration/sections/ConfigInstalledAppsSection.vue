<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import { Puzzle, RefreshCw } from "@lucide/vue";
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
    await client.value.ensureBundled();
    await refresh();
    emit("notify", "success", tr("已确保内置应用", "Built-in apps ensured"));
  } catch (e) {
    emit("notify", "error", String((e as Error).message || e));
  } finally { busy.value = false; }
}
onMounted(() => void refresh());
watch(() => props.restoringBuiltinApps, (v, prev) => { if (prev && !v) void refresh(); });
</script>
<template>
  <div class="config-section apps" data-testid="config-section-apps">
    <section class="panel builtin-plugin-manager">
      <header>
        <div>
          <h3>{{ tr("侧栏内置插件", "Sidebar built-in plugins") }}</h3>
          <p>{{ tr("插件随应用提供。停用只隐藏入口并阻止新界面操作；已接受的操作与后台任务继续完成，数据与配置保留。", "Plugins are bundled with this app. Disabling hides entry points and blocks new UI actions; accepted work continues.") }}</p>
        </div>
        <button class="button secondary compact" type="button" data-testid="restore-builtin-apps" :disabled="restoringBuiltinApps" @click="emit('restoreBuiltinApps')">
          <RefreshCw :size="14" :class="{ spinning: restoringBuiltinApps }" />
          {{ restoringBuiltinApps ? tr("恢复中…", "Restoring…") : tr("恢复内置应用", "Restore built-in apps") }}
        </button>
      </header>
      <p class="helper" style="margin:0 0 12px;font-size:12px;color:var(--muted)">
        {{ tr("若侧栏缺少「资产」「扫描」，点上方恢复。", "If Assets / Scanning are missing from the sidebar, restore above.") }}
      </p>
      <p v-if="pluginPreferenceError" class="host-action-error" role="alert">{{ pluginPreferenceError }}</p>
      <article v-for="plugin in BUILTIN_PLUGINS" :key="plugin.id">
        <div>
          <h4>{{ tr(...plugin.label) }}</h4>
          <p>{{ tr(...plugin.description) }}</p>
          <small v-if="plugin.id === 'hackerone' && !hackerOneEnabled">{{ tr("还需要配置 HackerOne API 凭据", "HackerOne API credentials are also required") }}</small>
        </div>
        <label>
          <input type="checkbox" :checked="pluginEnabled[plugin.id]" :aria-label="tr(...plugin.label)" @change="emit('togglePlugin', plugin.id, $event)" />
          {{ pluginEnabled[plugin.id] ? tr("已启用", "Enabled") : tr("已停用", "Disabled") }}
        </label>
      </article>
    </section>

    <section class="panel agent-host-apps" style="margin-top:16px;padding:22px">
      <header style="display:flex;justify-content:space-between;gap:12px;align-items:flex-start;margin-bottom:12px">
        <div>
          <h3><Puzzle :size="16" style="vertical-align:-2px;margin-right:6px" />{{ tr("Agent-host 已装应用", "Agent-host installed apps") }}</h3>
          <p class="helper" style="margin:4px 0 0;font-size:12px;color:var(--muted)">{{ tr("供问答 Agent 调用的 MCP 应用（assets / scanning）。权限在「数据权限」分区按项目授予。", "MCP apps for the Q&A agent (assets / scanning). Grant permissions per project under Data permissions.") }}</p>
        </div>
        <div style="display:flex;gap:8px">
          <button class="button ghost compact" type="button" :disabled="busy" @click="refresh"><RefreshCw :size="14" />{{ tr("刷新", "Refresh") }}</button>
          <button class="button secondary compact" type="button" :disabled="busy || !client" @click="ensureBundled">{{ tr("确保内置包", "Ensure bundled") }}</button>
        </div>
      </header>
      <p v-if="error" class="host-action-error" role="alert">{{ error }}</p>
      <p v-else-if="!client" class="helper">{{ tr("Agent-host sidecar 不可用。", "Agent-host sidecar unavailable.") }}</p>
      <article v-for="app in apps" :key="app.id" style="display:flex;justify-content:space-between;gap:16px;padding:14px 0;border-top:1px solid var(--line)">
        <div>
          <h4 style="margin:0">{{ app.manifest?.name || app.id }}</h4>
          <p style="margin:4px 0 0;font-size:12px;color:var(--muted)">{{ app.id }} · v{{ app.version || "?" }}</p>
        </div>
        <label style="display:flex;gap:8px;align-items:center;white-space:nowrap;font-size:12px">
          <input type="checkbox" :checked="app.enabled" :disabled="busy" @change="toggleApp(app.id, ($event.target as HTMLInputElement).checked)" />
          {{ app.enabled ? tr("已启用", "Enabled") : tr("已停用", "Disabled") }}
        </label>
      </article>
      <p v-if="client && !apps.length" class="helper">{{ tr("尚无已装应用。点「确保内置包」或上方「恢复内置应用」。", "No installed apps yet. Ensure bundled packages or restore built-in apps above.") }}</p>
    </section>
  </div>
</template>
