<script setup lang="ts">
// §F (scripts/artifacts the chat agent wrote into its own workspace — previously
// invisible to the operator) + §E (the MD report agent.md the deep-analysis
// guidance has it write at the end of a pass). Both just read the conversation's
// live workspace dir via the new host endpoints; nothing here grants any
// capability — it is a read-only operator view, same wall as memory/tool-library.
import { ChevronDown, FileText, FolderOpen, Wrench } from "@lucide/vue";
import { computed, defineComponent, h, onMounted, ref, watch, type VNode } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "../../../i18n";
import type { AgentHostChatClient, AgentWorkspaceFile } from "../chatClient";
import { parseMessageMarkdown, type MarkdownNode } from "../../conversations/markdown";
import ConversationCodeBlock from "../../conversations/ConversationCodeBlock.vue";

const props = defineProps<{ client: AgentHostChatClient; conversationId: string }>();
const { tr } = useI18n();

const files = ref<AgentWorkspaceFile[]>([]);
const loading = ref(false);
const error = ref("");
const openPath = ref("");
const content = ref("");
const contentTruncated = ref(false);
const previewLoading = ref(false);
const previewError = ref("");
const ingestBusy = ref(false);
const ingestNotice = ref("");

const REPORT_NAME = "report.md";
const reportFile = computed(() => files.value.find(item => item.path === REPORT_NAME));
const otherFiles = computed(() => files.value.filter(item => item.path !== REPORT_NAME));
const SCRIPT_LANGUAGES: Record<string, "node" | "python"> = { ".py": "python", ".js": "node", ".mjs": "node", ".cjs": "node", ".ts": "node", ".sh": "node" };
function extensionOf(path: string) { const at = path.lastIndexOf("."); return at < 0 ? "" : path.slice(at).toLowerCase(); }
function scriptLanguage(path: string): "node" | "python" | undefined { return SCRIPT_LANGUAGES[extensionOf(path)]; }
function codeLanguage(path: string) { const ext = extensionOf(path).replace(/^\./, ""); return ({ py: "python", js: "javascript", mjs: "javascript", cjs: "javascript", ts: "typescript", sh: "bash", json: "json", md: "markdown" } as Record<string, string>)[ext] || ""; }
function formatBytes(value: number) { return value < 1024 ? `${value} B` : value < 1024 * 1024 ? `${(value / 1024).toFixed(1)} KB` : `${(value / 1024 / 1024).toFixed(1)} MB`; }

async function load() {
  loading.value = true; error.value = "";
  try { files.value = await props.client.workspaceFiles(props.conversationId); }
  catch (e) { error.value = String((e as Error).message || e); }
  finally { loading.value = false; }
}
async function openFile(path: string) {
  if (openPath.value === path) { openPath.value = ""; return; }
  openPath.value = path; content.value = ""; contentTruncated.value = false; previewError.value = ""; previewLoading.value = true; ingestNotice.value = "";
  try {
    const result = await props.client.workspaceFile(props.conversationId, path);
    if (openPath.value !== path) return;
    content.value = result.content; contentTruncated.value = result.truncated;
  } catch (e) { if (openPath.value === path) previewError.value = String((e as Error).message || e); }
  finally { if (openPath.value === path) previewLoading.value = false; }
}
async function ingest(path: string) {
  const language = scriptLanguage(path);
  if (!language || openPath.value !== path || !content.value || ingestBusy.value) return;
  ingestBusy.value = true; ingestNotice.value = "";
  try {
    await invoke("ingest_agent_tool", {
      title: path, language, code: content.value,
      targetFingerprint: null, algoTags: JSON.stringify([]), sourceConversationId: props.conversationId,
    });
    ingestNotice.value = tr("已沉淀进工具库，待操作员审核。", "Sunk into the tool library, pending operator review.");
  } catch (e) { previewError.value = String((e as Error).message || e); }
  finally { ingestBusy.value = false; }
}

// Markdown rendering for report.md — same safe node-tree approach as chat
// messages (features/conversations/markdown.ts), trimmed to this read-only view:
// no local-file links (nothing here is a chat attachment), fenced code still
// goes through ConversationCodeBlock for consistent highlighting/copy.
const reportOpen = ref(false);
const reportMarkdown = ref("");
const reportLoading = ref(false);
const reportError = ref("");
async function openReport() {
  reportOpen.value = !reportOpen.value;
  if (!reportOpen.value || reportMarkdown.value || reportLoading.value) return;
  reportLoading.value = true; reportError.value = "";
  try { reportMarkdown.value = (await props.client.workspaceFile(props.conversationId, REPORT_NAME)).content; }
  catch (e) { reportError.value = String((e as Error).message || e); }
  finally { reportLoading.value = false; }
}
const reportNodes = computed(() => (reportMarkdown.value ? parseMessageMarkdown(reportMarkdown.value) : []));
function followReportLink(event: MouseEvent, href: string) {
  // Same rule as chat messages (ConversationMessage.vue): Tauri links leave the
  // webview through the system browser; markdown.ts already validated href.
  if (!("__TAURI_INTERNALS__" in globalThis)) return;
  event.preventDefault();
  void openUrl(href).catch(() => { reportError.value = tr("无法打开链接，请复制链接地址后重试", "Could not open the link. Copy its address and try in your browser."); });
}
function renderReportNode(node: MarkdownNode, key: number): VNode | string {
  if (node.kind === "text") return node.text;
  if (node.kind === "file") return node.text;
  if (node.kind === "code") return h(ConversationCodeBlock, { key, source: node.text, language: node.language });
  if (node.kind === "image") return h("span", { key, class: "ahc-workspace-report-image" }, `[${tr("图片", "Image")}${node.text ? `: ${node.text}` : ""}]`);
  const attributes: Record<string, unknown> = { key };
  if (node.tag === "a" && node.href) { attributes.href = node.href; attributes.target = "_blank"; attributes.rel = "noopener noreferrer"; attributes.referrerpolicy = "no-referrer"; attributes.onClick = (event: MouseEvent) => followReportLink(event, node.href!); }
  if (node.title) attributes.title = node.title;
  if (node.start) attributes.start = node.start;
  if (node.align) attributes.style = { textAlign: node.align };
  return h(node.tag, attributes, node.children.map(renderReportNode));
}
const ReportContent = defineComponent({ setup: () => () => h("div", { class: "ahc-workspace-report-body" }, reportNodes.value.map(renderReportNode)) });

watch(() => props.conversationId, () => { files.value = []; openPath.value = ""; content.value = ""; reportOpen.value = false; reportMarkdown.value = ""; void load(); }, { immediate: true });
onMounted(() => { if (!files.value.length) void load(); });
</script>

<template>
  <div class="ahc-workspace-panel">
    <p v-if="error" role="alert" class="ahc-workspace-error">{{ error }}</p>
    <section v-if="reportFile" class="ahc-workspace-report">
      <button type="button" class="ahc-workspace-report-toggle" :aria-expanded="reportOpen" @click="openReport">
        <FileText :size="14" />{{ tr("报告 · report.md", "Report · report.md") }}<ChevronDown :size="13" class="ahc-workspace-chevron" />
      </button>
      <div v-if="reportOpen" class="ahc-workspace-report-pane">
        <p v-if="reportLoading" class="ahc-workspace-muted">{{ tr("正在读取…", "Reading…") }}</p>
        <p v-else-if="reportError" role="alert" class="ahc-workspace-error">{{ reportError }}</p>
        <ReportContent v-else />
      </div>
    </section>
    <div class="ahc-workspace-head">
      <strong><FolderOpen :size="14" />{{ tr("产物与脚本", "Artifacts and scripts") }}</strong>
      <small>{{ tr("当前对话工作区里的文件；可复用的脚本能一键沉淀进工具库，由操作员审核。", "Files in this conversation's workspace. Reusable scripts can be sunk into the tool library for operator review in one click.") }}</small>
    </div>
    <p v-if="loading" class="ahc-workspace-muted">{{ tr("正在加载…", "Loading…") }}</p>
    <p v-else-if="!otherFiles.length" class="ahc-workspace-muted">{{ tr("工作区暂无文件。", "No files in the workspace yet.") }}</p>
    <ul v-else class="ahc-workspace-list">
      <li v-for="file in otherFiles" :key="file.path">
        <button type="button" class="ahc-workspace-row" :aria-expanded="openPath === file.path" @click="openFile(file.path)">
          <span class="ahc-workspace-path">{{ file.path }}</span><small>{{ formatBytes(file.bytes) }} · {{ file.modifiedAt }}</small>
        </button>
        <div v-if="openPath === file.path" class="ahc-workspace-preview">
          <p v-if="previewLoading" class="ahc-workspace-muted">{{ tr("正在读取…", "Reading…") }}</p>
          <p v-else-if="previewError" role="alert" class="ahc-workspace-error">{{ previewError }}</p>
          <template v-else>
            <ConversationCodeBlock :source="content" :language="codeLanguage(file.path)" />
            <p v-if="contentTruncated" class="ahc-workspace-muted">{{ tr("内容过大，已截断显示。", "Content is large; this view is truncated.") }}</p>
            <div v-if="scriptLanguage(file.path)" class="ahc-workspace-ingest">
              <button type="button" class="button ghost compact" :disabled="ingestBusy" @click="ingest(file.path)"><Wrench :size="13" />{{ ingestBusy ? tr("正在沉淀…", "Sinking…") : tr("沉淀进工具库", "Sink into tool library") }}</button>
              <span v-if="ingestNotice" class="ahc-workspace-muted">{{ ingestNotice }}</span>
            </div>
          </template>
        </div>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.ahc-workspace-panel { display: flex; flex-direction: column; gap: 10px; }
.ahc-workspace-error { margin: 0; color: var(--red, #b63f3f); font-size: 12px; overflow-wrap: anywhere; }
.ahc-workspace-muted { margin: 0; color: var(--app-muted, #687078); font-size: 12px; }
.ahc-workspace-head { display: flex; flex-direction: column; gap: 2px; }
.ahc-workspace-head > strong { display: flex; align-items: center; gap: 6px; font-size: 13px; }
.ahc-workspace-head > small { color: var(--app-muted, #687078); font-size: 11px; line-height: 1.5; }
.ahc-workspace-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.ahc-workspace-row { display: flex; align-items: baseline; justify-content: space-between; gap: 10px; width: 100%; padding: 7px 9px; border: 1px solid var(--line, #e1e3e7); border-radius: 8px; background: transparent; text-align: left; cursor: pointer; }
.ahc-workspace-row:hover { background: color-mix(in srgb, var(--app-ink, #1c2024) 4%, transparent); }
.ahc-workspace-path { font: 12px var(--font-mono, monospace); overflow-wrap: anywhere; }
.ahc-workspace-row > small { flex: none; color: var(--app-muted, #687078); font-size: 11px; }
.ahc-workspace-preview { margin: 4px 0 2px 9px; padding-left: 10px; border-left: 2px solid var(--line, #e1e3e7); display: flex; flex-direction: column; gap: 6px; }
.ahc-workspace-ingest { display: flex; align-items: center; gap: 8px; }
.ahc-workspace-report { border: 1px solid var(--line, #e1e3e7); border-radius: 10px; overflow: hidden; }
.ahc-workspace-report-toggle { display: flex; align-items: center; gap: 7px; width: 100%; padding: 9px 12px; border: 0; background: transparent; text-align: left; font-size: 13px; font-weight: 550; cursor: pointer; }
.ahc-workspace-chevron { margin-left: auto; }
.ahc-workspace-report-toggle[aria-expanded="true"] .ahc-workspace-chevron { transform: rotate(180deg); }
.ahc-workspace-report-pane { padding: 0 14px 14px; border-top: 1px solid var(--line, #e1e3e7); }
.ahc-workspace-report-body :deep(h1), .ahc-workspace-report-body :deep(h2), .ahc-workspace-report-body :deep(h3) { margin: 14px 0 6px; }
.ahc-workspace-report-body :deep(p) { margin: 6px 0; font-size: 13px; line-height: 1.7; }
.ahc-workspace-report-image { color: var(--app-muted, #687078); font-size: 12px; }
</style>
