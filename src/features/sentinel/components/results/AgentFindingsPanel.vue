<script setup lang="ts">
// Agent-recorded findings (docs/AGENT_SCAN_RESULTS_REDESIGN.md §D/§G): the chat
// agent's own `record_finding` calls, held in the host's `agent_findings` table —
// distinct from the native Tauri `sentinel_findings` SentinelVulnerabilitiesPane
// renders above. No scan_id on these rows (the agent writes them from whatever
// conversation it is analysing in), so this panel scopes by project, not by the
// task currently open in Results. Unifying the two is future work; for now this
// is deliberately its own block, styled to match (reuses the global .vuln-card /
// .severity-badge / .vuln-columns rules from src/styles/sentinel-base.css so it
// sits visually next to the native pane without new global CSS).
import { ArrowDownLeft, ArrowUpRight, Bug, ChevronDown, Copy, ShieldBan, ShieldCheck, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "../../../../i18n";
import { resolveAgentHostEndpoint } from "../../../agentHost/api";
import { createAgentHostChatClient, type AgentHostChatClient, type AgentFinding, type AgentFindingRecord } from "../../../agentHost/chatClient";
import { createSentinelLabels, safeSeverity, text } from "../../presentation";

const props = defineProps<{ projectId?: number }>();
const { tr } = useI18n();
const { severityLabel } = createSentinelLabels(tr);

type Filter = "unverified" | "confirmed" | "rejected" | "all";
const FILTERS: Filter[] = ["unverified", "confirmed", "rejected", "all"];
function filterLabel(value: Filter) {
  return value === "confirmed" ? tr("已确认", "Confirmed")
    : value === "rejected" ? tr("已拒绝", "Rejected")
      : value === "all" ? tr("全部", "All")
        : tr("待复核", "Unverified");
}

const filter = ref<Filter>("unverified");
const items = ref<AgentFinding[]>([]);
const loading = ref(false);
const error = ref("");
const busyId = ref("");
const expandedId = ref("");

let hostClient: AgentHostChatClient | undefined;
async function ensureClient(): Promise<AgentHostChatClient> {
  if (hostClient) return hostClient;
  const endpoint = await resolveAgentHostEndpoint();
  if (!endpoint) throw Error(tr("agent-host 未运行", "agent-host is not running"));
  hostClient = createAgentHostChatClient(endpoint);
  return hostClient;
}

// agent_findings carries projectId per row but the GET /api/findings endpoint
// only filters by conversationId/status — so scope to this project client-side.
const visible = computed(() => items.value.filter(item => props.projectId === undefined || item.projectId === props.projectId));

async function load() {
  loading.value = true; error.value = "";
  try {
    const client = await ensureClient();
    items.value = await client.listFindings(filter.value === "all" ? undefined : filter.value);
  } catch (e) { error.value = String((e as Error).message || e); }
  finally { loading.value = false; }
}
async function setFilter(value: Filter) { if (filter.value === value || loading.value) return; filter.value = value; await load(); }
async function decide(item: AgentFinding, confirm: boolean) {
  if (busyId.value) return;
  busyId.value = item.id; error.value = "";
  try {
    const client = await ensureClient();
    const updated = await client.decideFinding(item.id, confirm);
    items.value = items.value.map(row => (row.id === updated.id ? updated : row));
  } catch (e) { error.value = String((e as Error).message || e); }
  finally { busyId.value = ""; }
}
async function remove(item: AgentFinding) {
  if (busyId.value) return;
  busyId.value = item.id; error.value = "";
  try {
    const client = await ensureClient();
    await client.deleteFinding(item.id);
    items.value = items.value.filter(row => row.id !== item.id);
  } catch (e) { error.value = String((e as Error).message || e); }
  finally { busyId.value = ""; }
}
async function copyText(value: string) { try { await navigator.clipboard?.writeText(value); } catch { /* clipboard may be unavailable */ } }

const SENSITIVE_HEADERS = new Set(["authorization", "cookie", "set-cookie", "proxy-authorization", "x-api-key", "x-auth-token", "x-csrf-token"]);
const REDACTED = () => tr("••••••••（已脱敏）", "•••••••• (redacted)");
function headerRows(headers?: Record<string, string>) {
  if (!headers || typeof headers !== "object") return [];
  return Object.entries(headers).map(([key, value]) => ({ key, value: SENSITIVE_HEADERS.has(key.toLowerCase()) ? REDACTED() : text(value) }));
}
function statusLabel(item: AgentFinding) {
  return item.status === "confirmed" ? tr("已确认", "Confirmed") : item.status === "rejected" ? tr("已拒绝", "Rejected") : tr("待复核", "Unverified");
}
// workspace_ws captures (increment 4): sorted by `at`, no header-style secrets to
// redact here — the existing evidence redaction above covers HTTP headers only.
type WsFrame = NonNullable<AgentFindingRecord["ws_frames"]>[number];
function sortedFrames(frames?: WsFrame[]) {
  return frames ? [...frames].sort((a, b) => (a.at ?? 0) - (b.at ?? 0)) : [];
}
function frameTruncated(frame: WsFrame) {
  const shown = frame.text?.length ?? frame.base64?.length ?? 0;
  return typeof frame.bytes === "number" && frame.bytes > shown;
}

onMounted(load);
</script>

<template>
  <section class="result-block agent-findings-panel">
    <div class="block-title">
      <Bug :size="16" />
      <div>
        <strong>{{ tr("Agent 发现", "Agent findings") }}</strong>
        <small>{{ tr("chat agent 用 record_finding 自记的发现，操作员审一道再入库确认；跟上面原生漏洞各自独立展示。", "Findings the chat agent recorded with record_finding, pending an operator decide before they count as confirmed — kept separate from the native pane above.") }}</small>
      </div>
    </div>
    <div class="agent-findings-filters">
      <button v-for="value in FILTERS" :key="value" type="button" :class="{ active: filter === value }" :disabled="loading" @click="setFilter(value)">{{ filterLabel(value) }}</button>
    </div>
    <p v-if="error" role="alert" class="agent-findings-error">{{ error }}</p>
    <div v-if="loading" class="empty-inline">{{ tr("正在加载…", "Loading…") }}</div>
    <div v-else-if="!visible.length" class="empty-inline">{{ tr("当前筛选下没有记录", "No findings under this filter") }}</div>
    <div v-else class="vulnerability-list">
      <article v-for="item in visible" :key="item.id" :class="`vuln-card severity-border-${safeSeverity(item.record.severity || item.severity)}`">
        <header>
          <span :class="`severity-badge ${safeSeverity(item.record.severity || item.severity)}`">{{ severityLabel(safeSeverity(item.record.severity || item.severity)) }}</span>
          <div>
            <strong>{{ item.record.title || item.title }}</strong>
            <small>{{ [item.record.cvss ? `CVSS ${item.record.cvss}` : "", item.record.confidence ? tr(`置信度 ${item.record.confidence}`, `Confidence ${item.record.confidence}`) : "", [item.record.cwe, item.record.cve].filter(Boolean).join(" · ")].filter(Boolean).join(" · ") }}</small>
          </div>
          <span :class="`validation-chip ${item.status === 'confirmed' ? 'true_positive' : item.status === 'rejected' ? 'false_positive' : ''}`">{{ statusLabel(item) }}</span>
        </header>
        <div class="vuln-columns">
          <div v-if="item.record.description"><span>{{ tr("漏洞描述", "Description") }}</span><p>{{ item.record.description }}</p></div>
          <div v-if="item.record.evidence"><span>{{ tr("证据", "Evidence") }}</span><pre>{{ text(item.record.evidence) }}</pre></div>
          <div v-if="item.record.chain_context" class="vuln-chain-context"><span>{{ tr("组合攻击链", "Attack chain") }}</span><p>{{ item.record.chain_context }}</p></div>
          <div v-if="item.record.business_impact" class="vuln-business-impact"><span>{{ tr("业务危害", "Business impact") }}</span><p>{{ item.record.business_impact }}</p></div>
          <div v-if="item.record.recommendation"><span>{{ tr("修复建议", "Remediation") }}</span><p>{{ item.record.recommendation }}</p></div>
        </div>
        <section v-if="item.record.poc_script || item.record.poc_request || item.record.poc_output" class="agent-finding-poc">
          <div v-if="item.record.poc_request" class="deep-field"><span>{{ tr("PoC 请求", "PoC request") }}<button class="button ghost compact" @click="copyText(String(item.record.poc_request))"><Copy :size="12" />{{ tr("复制", "Copy") }}</button></span><pre>{{ item.record.poc_request }}</pre></div>
          <div v-if="item.record.poc_script" class="deep-field"><span>{{ tr("PoC 脚本", "PoC script") }}<button class="button ghost compact" @click="copyText(String(item.record.poc_script))"><Copy :size="12" />{{ tr("复制", "Copy") }}</button></span><pre>{{ item.record.poc_script }}</pre></div>
          <div v-if="item.record.poc_output" class="deep-field"><span>{{ tr("PoC 输出", "PoC output") }}</span><pre>{{ item.record.poc_output }}</pre></div>
        </section>
        <details v-if="item.record.http_request || item.record.http_response || item.record.ws_frames?.length" class="agent-finding-http" :open="expandedId === item.id" @toggle="expandedId = ($event.target as HTMLDetailsElement).open ? item.id : ''">
          <summary><ChevronDown :size="13" />{{ tr("HTTP/WS 证据（已脱敏敏感请求头）", "HTTP/WS evidence (sensitive headers redacted)") }}</summary>
          <div v-if="item.record.http_request" class="agent-finding-http-block">
            <strong>{{ tr("请求", "Request") }}</strong>
            <code class="agent-finding-http-line">{{ item.record.http_request.method || "GET" }} {{ item.record.http_request.url || "" }}</code>
            <ul v-if="headerRows(item.record.http_request.headers).length" class="agent-finding-headers">
              <li v-for="row in headerRows(item.record.http_request.headers)" :key="row.key"><b>{{ row.key }}</b>{{ row.value }}</li>
            </ul>
            <pre v-if="item.record.http_request.body">{{ text(item.record.http_request.body) }}</pre>
          </div>
          <div v-if="item.record.http_response" class="agent-finding-http-block">
            <strong>{{ tr("响应", "Response") }}</strong>
            <code class="agent-finding-http-line">{{ item.record.http_response.status ?? "—" }}</code>
            <ul v-if="headerRows(item.record.http_response.headers).length" class="agent-finding-headers">
              <li v-for="row in headerRows(item.record.http_response.headers)" :key="row.key"><b>{{ row.key }}</b>{{ row.value }}</li>
            </ul>
            <pre v-if="item.record.http_response.body">{{ text(item.record.http_response.body) }}</pre>
          </div>
          <div v-if="item.record.ws_frames?.length" class="agent-finding-http-block">
            <strong>{{ tr(`WebSocket 帧（${item.record.ws_frames.length}）`, `WebSocket frames (${item.record.ws_frames.length})`) }}</strong>
            <ol class="agent-finding-ws-frames">
              <li v-for="(frame, index) in sortedFrames(item.record.ws_frames)" :key="index" :class="`ws-frame ws-frame-${frame.dir === 'send' ? 'send' : 'recv'}`">
                <span class="ws-frame-meta">
                  <ArrowUpRight v-if="frame.dir === 'send'" :size="12" /><ArrowDownLeft v-else :size="12" />
                  {{ frame.dir === 'send' ? tr('发', 'Sent') : tr('收', 'Recv') }} · {{ frame.type || 'text' }}
                  <template v-if="typeof frame.at === 'number'"> · {{ tr(`+${frame.at}ms`, `+${frame.at}ms`) }}</template>
                  <template v-if="typeof frame.bytes === 'number'"> · {{ frame.bytes }}B</template>
                  <em v-if="frameTruncated(frame)" class="ws-frame-truncated">{{ tr('已截断', 'Truncated') }}</em>
                </span>
                <pre v-if="frame.type !== 'binary' && frame.text">{{ frame.text }}</pre>
                <details v-else-if="frame.base64" class="ws-frame-binary"><summary>{{ tr('二进制（base64）', 'Binary (base64)') }}</summary><pre>{{ frame.base64 }}</pre></details>
              </li>
            </ol>
          </div>
        </details>
        <footer v-if="item.status === 'unverified'">
          <button class="button primary compact" :disabled="busyId === item.id" @click="decide(item, true)"><ShieldCheck :size="13" />{{ tr("确认", "Confirm") }}</button>
          <button class="button ghost compact" :disabled="busyId === item.id" @click="decide(item, false)"><ShieldBan :size="13" />{{ tr("拒绝", "Reject") }}</button>
          <button class="button danger compact" :disabled="busyId === item.id" @click="remove(item)"><Trash2 :size="13" />{{ tr("删除", "Delete") }}</button>
        </footer>
        <footer v-else>
          <button class="button danger compact" :disabled="busyId === item.id" @click="remove(item)"><Trash2 :size="13" />{{ tr("删除", "Delete") }}</button>
        </footer>
      </article>
    </div>
  </section>
</template>

<style scoped>
.agent-findings-filters { display: flex; gap: 6px; flex-wrap: wrap; padding: 0 14px 10px; }
.agent-findings-filters button { padding: 5px 11px; border: 1px solid var(--ui-line, #ddd); border-radius: 999px; background: transparent; color: var(--ui-muted, #777); font-size: 12px; cursor: pointer; }
.agent-findings-filters button.active { background: color-mix(in srgb, var(--blue, #2878ff) 10%, transparent); border-color: color-mix(in srgb, var(--blue, #2878ff) 40%, var(--ui-line, #ddd)); color: color-mix(in srgb, var(--blue, #2878ff) 80%, var(--ui-ink, #1a1a1a)); font-weight: 600; }
.agent-findings-filters button:disabled { opacity: .5; cursor: default; }
.agent-findings-error { margin: 0 14px 10px; color: var(--red, #b63f3f); font-size: 12px; overflow-wrap: anywhere; }
.agent-finding-poc { margin: 0 14px 12px; display: flex; flex-direction: column; gap: 8px; }
.deep-field { display: flex; flex-direction: column; gap: 4px; }
.deep-field > span { display: flex; align-items: center; gap: 8px; font-size: 12px; font-weight: 600; color: var(--ui-muted, #777); }
.deep-field > pre { margin: 0; padding: 8px 10px; border: 1px solid var(--ui-line, #ddd); border-radius: 8px; background: var(--ui-surface-raised, #f7f7f7); overflow-x: auto; white-space: pre-wrap; word-break: break-word; font-size: 12px; line-height: 1.5; }
.agent-finding-http { margin: 0 14px 12px; border: 1px solid var(--ui-line, #ddd); border-radius: 10px; overflow: hidden; }
.agent-finding-http > summary { display: flex; align-items: center; gap: 6px; padding: 9px 12px; cursor: pointer; font-size: 12px; font-weight: 600; color: var(--ui-muted, #777); list-style: none; }
.agent-finding-http > summary::-webkit-details-marker { display: none; }
.agent-finding-http[open] > summary { border-bottom: 1px solid var(--ui-line, #ddd); }
.agent-finding-http-block { padding: 10px 12px; display: flex; flex-direction: column; gap: 6px; border-bottom: 1px solid var(--ui-line, #ddd); }
.agent-finding-http-block:last-child { border-bottom: 0; }
.agent-finding-http-block > strong { font-size: 11px; text-transform: uppercase; letter-spacing: .04em; color: var(--ui-muted, #777); }
.agent-finding-http-line { font: 12px/1.5 var(--font-mono, monospace); word-break: break-all; }
.agent-finding-headers { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; font: 11px/1.6 var(--font-mono, monospace); }
.agent-finding-headers li { display: flex; gap: 6px; overflow-wrap: anywhere; }
.agent-finding-headers b { flex: none; color: var(--ui-muted, #777); font-weight: 600; }
.agent-finding-http-block > pre { margin: 0; max-height: 220px; overflow: auto; padding: 8px; background: var(--ui-surface-raised, #f7f7f7); border-radius: 8px; font: 11px/1.55 var(--font-mono, monospace); white-space: pre-wrap; word-break: break-all; }
.agent-finding-ws-frames { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; max-height: 320px; overflow: auto; }
.ws-frame { display: flex; flex-direction: column; gap: 4px; padding: 6px 8px; border-radius: 8px; border: 1px solid var(--ui-line, #ddd); max-width: 90%; }
.ws-frame-send { align-self: flex-end; background: color-mix(in srgb, var(--blue, #2878ff) 6%, var(--ui-surface, #fff)); }
.ws-frame-recv { align-self: flex-start; background: var(--ui-surface-raised, #f7f7f7); }
.ws-frame-meta { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; font-size: 11px; color: var(--ui-muted, #777); }
.ws-frame-truncated { font-style: normal; color: #b07b32; }
.ws-frame > pre { margin: 0; max-height: 160px; overflow: auto; padding: 6px 8px; background: var(--ui-surface-raised, #f7f7f7); border-radius: 6px; font: 11px/1.5 var(--font-mono, monospace); white-space: pre-wrap; word-break: break-all; }
.ws-frame-binary > summary { cursor: pointer; font-size: 11px; color: var(--ui-muted, #777); }
.ws-frame-binary > pre { margin: 4px 0 0; max-height: 160px; overflow: auto; padding: 6px 8px; background: var(--ui-surface-raised, #f7f7f7); border-radius: 6px; font: 10px/1.5 var(--font-mono, monospace); white-space: pre-wrap; word-break: break-all; }
.vuln-business-impact { border-left: 3px solid color-mix(in srgb, #b4332f 55%, transparent); padding-left: 10px; }
.vuln-business-impact > span { color: color-mix(in srgb, #b4332f 80%, var(--ui-ink, #1a1a1a)); font-weight: 600; }
.vuln-chain-context { border-left: 3px solid color-mix(in srgb, #7a5bbf 55%, transparent); padding-left: 10px; }
.vuln-chain-context > span { color: color-mix(in srgb, #7a5bbf 80%, var(--ui-ink, #1a1a1a)); font-weight: 600; }
</style>
