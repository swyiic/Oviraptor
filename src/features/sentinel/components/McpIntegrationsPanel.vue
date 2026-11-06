<script setup lang="ts">
import { onMounted, ref } from "vue";
import { sentinelApi } from "../api";
import { useI18n } from "../../../i18n";

const { tr } = useI18n();
const servers = ref<Array<Record<string, unknown>>>([]);
const allowlist = ref("");
const loading = ref(false);
const busy = ref("");
const error = ref("");
type McpToolPreview = { name?: string };
const toolsPreview = ref<Record<string, unknown[]>>({});
const catalogPreview = ref<Record<string, { resources: unknown[]; prompts: unknown[] }>>({});

const draft = ref({
  serverId: "",
  label: "",
  transport: "stdio" as "stdio" | "http" | "sse",
  command: "",
  args: "",
  headers: "",
  enabled: false,
});

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const [rows, rules] = await Promise.all([
      sentinelApi.listAgentMcpServers(),
      sentinelApi.getAgentMcpToolAllowlist(),
    ]);
    servers.value = rows as Array<Record<string, unknown>>;
    allowlist.value = rules.map((r) => r.pattern).join("\n");
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function saveServer() {
  const id = draft.value.serverId.trim();
  if (!id || !draft.value.command.trim()) return;
  busy.value = "save";
  try {
    const env = draft.value.headers
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const idx = line.indexOf(":");
        if (idx <= 0) return null;
        return { name: line.slice(0, idx).trim(), value: line.slice(idx + 1).trim() };
      })
      .filter(Boolean) as Array<{ name: string; value: string }>;
    await sentinelApi.upsertAgentMcpServer({
      serverId: id,
      label: draft.value.label.trim() || id,
      transport: draft.value.transport,
      command: draft.value.command.trim(),
      args: draft.value.transport === "stdio" ? draft.value.args.split(/\s+/).filter(Boolean) : [],
      env,
      enabled: draft.value.enabled,
    });
    draft.value = { serverId: "", label: "", transport: "stdio", command: "", args: "", headers: "", enabled: false };
    await load();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function saveAllowlist() {
  busy.value = "allow";
  try {
    const rules = allowlist.value
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .map((pattern) => ({ pattern, decision: "allow" as const }));
    await sentinelApi.setAgentMcpToolAllowlist(rules);
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function listCatalog(serverId: string) {
  busy.value = `catalog-${serverId}`;
  try {
    const catalog = await sentinelApi.listAgentMcpServerCatalog(serverId);
    toolsPreview.value[serverId] = catalog.tools as unknown[];
    catalogPreview.value[serverId] = { resources: catalog.resources, prompts: catalog.prompts };
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function listTools(serverId: string) {
  busy.value = `tools-${serverId}`;
  try {
    toolsPreview.value[serverId] = await sentinelApi.listAgentMcpServerTools(serverId);
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

async function remove(serverId: string) {
  busy.value = `del-${serverId}`;
  try {
    await sentinelApi.deleteAgentMcpServer(serverId);
    await load();
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = "";
  }
}

onMounted(load);
</script>

<template>
  <section class="mcp-panel panel">
    <header>
      <strong>{{ tr("MCP 插件", "MCP plugins") }}</strong>
      <small>{{ tr("支持 stdio / HTTP / SSE 远程端点。默认全部拒绝；启用并在 allowlist 写 server/tool（如 nuclei/*）后 Agent 才可见 call_mcp_tool。", "stdio, HTTP, and SSE remote endpoints. Default deny; enable server + allowlist (e.g. nuclei/*) before call_mcp_tool appears.") }}</small>
    </header>
    <label>{{ tr("Allowlist（每行一条 glob，如 myserver/*）", "Allowlist (one glob per line, e.g. myserver/*)") }}
      <textarea v-model="allowlist" rows="3" />
    </label>
    <button type="button" class="button secondary compact" :disabled="busy === 'allow'" @click="saveAllowlist">{{ tr("保存 allowlist", "Save allowlist") }}</button>
    <form class="mcp-form" @submit.prevent="saveServer">
      <input v-model="draft.serverId" :placeholder="tr('server_id', 'server_id')" maxlength="64" />
      <input v-model="draft.label" :placeholder="tr('显示名', 'Label')" />
      <select v-model="draft.transport">
        <option value="stdio">stdio</option>
        <option value="http">HTTP</option>
        <option value="sse">SSE / streamable HTTP</option>
      </select>
      <input
        v-model="draft.command"
        :placeholder="draft.transport === 'stdio' ? tr('可执行文件路径', 'Command path') : tr('https://mcp 端点 URL', 'https://mcp endpoint URL')"
      />
      <input v-if="draft.transport === 'stdio'" v-model="draft.args" :placeholder="tr('参数（空格分隔）', 'Args (space-separated)')" />
      <textarea
        v-else
        v-model="draft.headers"
        rows="2"
        :placeholder="tr('HTTP 头（每行 Name: value，如 Authorization: Bearer …）', 'HTTP headers (one Name: value per line)')"
      />
      <label><input v-model="draft.enabled" type="checkbox" />{{ tr("启用", "Enabled") }}</label>
      <button type="submit" class="button primary compact" :disabled="busy === 'save'">{{ tr("保存服务器", "Save server") }}</button>
    </form>
    <ul v-if="servers.length" class="mcp-list">
      <li v-for="row in servers" :key="String(row.serverId)">
        <strong>{{ row.label || row.serverId }}</strong>
        <em>{{ row.transport || "stdio" }}</em>
        <code>{{ row.command }}</code>
        <span>{{ row.enabled ? tr("已启用", "enabled") : tr("未启用", "disabled") }}</span>
        <button type="button" class="button ghost compact" :disabled="!!busy" @click="listTools(String(row.serverId))">{{ tr("列出工具", "List tools") }}</button>
        <button type="button" class="button ghost compact" :disabled="!!busy" @click="listCatalog(String(row.serverId))">{{ tr("资源 / 提示", "Resources / prompts") }}</button>
        <button type="button" class="button ghost compact" :disabled="!!busy" @click="remove(String(row.serverId))">{{ tr("删除", "Delete") }}</button>
        <small v-if="catalogPreview[String(row.serverId)]">
          resources {{ catalogPreview[String(row.serverId)].resources.length }} · prompts {{ catalogPreview[String(row.serverId)].prompts.length }} · sampling disabled
        </small>
        <ul v-if="toolsPreview[String(row.serverId)]?.length">
          <li v-for="tool in toolsPreview[String(row.serverId)]" :key="String((tool as McpToolPreview).name)">{{ (tool as McpToolPreview).name }}</li>
        </ul>
      </li>
    </ul>
    <p v-if="loading" class="muted">{{ tr("加载中…", "Loading…") }}</p>
    <p v-if="error" class="error">{{ error }}</p>
  </section>
</template>

<style scoped>
.mcp-panel { display: flex; flex-direction: column; gap: 10px; padding: 14px; margin: 12px 0; }
.mcp-panel header { display: flex; flex-direction: column; gap: 4px; }
.mcp-panel small { color: var(--muted, #666); line-height: 1.5; }
.mcp-form { display: grid; gap: 8px; }
.mcp-form input { font: inherit; padding: 6px 8px; border: 1px solid var(--border, #ddd); border-radius: 6px; }
.mcp-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
.mcp-list li { border: 1px solid var(--border, #e5e5e5); border-radius: 8px; padding: 10px; display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
.error { color: #b4332f; }
.muted { color: var(--muted, #777); }
</style>
