<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../../../i18n";

const { tr } = useI18n();
interface PathSignal { path: string; status?: number[]; bodyRegex?: string }
type Condition =
  | { type: "all"; children: Condition[] }
  | { type: "any"; children: Condition[] }
  | { type: "body"; pattern: string }
  | { type: "cookie"; pattern: string }
  | { type: "header"; name: string; pattern: string };
interface Fingerprint { id: string; name: string; headerRegex?: Record<string, string>; setCookieRegex?: string[]; bodyRegex?: string[]; pathSignals?: PathSignal[]; condition?: Condition }
interface ExposureCheck { id: string; title: string; severity: string; path: string; appliesTo?: string[] }
interface CatalogEntry { fingerprint: Fingerprint; source: "builtin" | "custom" }
interface Library {
  fingerprints: { fingerprints?: Fingerprint[] };
  exposures: { checks?: ExposureCheck[] };
  user?: { path?: string; booleanPath?: string; fingerprints?: Fingerprint[]; error?: string };
  effective?: { fingerprints?: Fingerprint[]; error?: string };
  booleanRuntime?: string;
}

const loading = ref(true);
const error = ref("");
const fingerprints = ref<Fingerprint[]>([]);
const exposures = ref<ExposureCheck[]>([]);
const userFingerprints = ref<Fingerprint[]>([]);
const effectiveFingerprints = ref<Fingerprint[] | null>(null);
const effectiveError = ref("");
const userPath = ref("");
const booleanPath = ref("");
const userError = ref("");
const booleanRuntime = ref("");
const booleanSupported = computed(() => booleanRuntime.value === "condition-tree-v1");
const mode = ref<"" | "add" | "import" | "export">("");
const editorText = ref("");
const actionError = ref("");
const busy = ref(false);
const query = ref("");
const sourceFilter = ref<"all" | "builtin" | "custom">("all");
const page = ref(1);
const pageSize = 25;
const catalogError = computed(() => error.value || effectiveError.value || userError.value);
const catalogReady = computed(() => !catalogError.value);
const catalogEntries = computed<CatalogEntry[]>(() => {
  if (!catalogReady.value) return [];
  const customIds = new Set(userFingerprints.value.map(fp => fp.id));
  const rules = effectiveFingerprints.value ?? [...fingerprints.value, ...userFingerprints.value];
  const byId = new Map<string, CatalogEntry>();
  // Exact IDs match the runtime merge: custom overrides builtin once. An
  // explicitly empty effective list is authoritative, not a fallback trigger.
  for (const fingerprint of rules) byId.set(fingerprint.id, {
    fingerprint, source: customIds.has(fingerprint.id) ? "custom" : "builtin",
  });
  return [...byId.values()];
});
const effectiveCount = computed(() => catalogEntries.value.length);
const builtinCount = computed(() => catalogEntries.value.filter(entry => entry.source === "builtin").length);
const customCount = computed(() => catalogEntries.value.filter(entry => entry.source === "custom").length);
function normalizeSearch(value: string) { return value.normalize("NFKC").trim().toLowerCase(); }
function conditionText(condition: Condition, depth = 0): string {
  const indent = "  ".repeat(Math.min(depth, 20));
  if (depth > 20) return `${indent}${tr("条件层级过深", "Condition depth exceeded")}`;
  if (condition.type === "all" || condition.type === "any") {
    const operator = condition.type === "all" ? "AND" : "OR";
    const label = condition.type === "all" ? "ALL" : "ANY";
    return `${indent}${label} (\n${condition.children.map(child => conditionText(child, depth + 1)).join(`\n${indent}  ${operator}\n`)}\n${indent})`;
  }
  if (condition.type === "header") return `${indent}Header ${condition.name} ~ ${condition.pattern}`;
  if (condition.type === "cookie") return `${indent}Set-Cookie ~ ${condition.pattern}`;
  return `${indent}Body ~ ${condition.pattern}`;
}
function signals(fp: Fingerprint): string[] {
  const out: string[] = [];
  for (const [name, pattern] of Object.entries(fp.headerRegex ?? {})) out.push(`Header ${name} ~ ${pattern}`);
  for (const pattern of fp.setCookieRegex ?? []) out.push(`Set-Cookie ~ ${pattern}`);
  for (const pattern of fp.bodyRegex ?? []) out.push(`Body ~ ${pattern}`);
  for (const signal of fp.pathSignals ?? []) out.push(`${signal.path}${signal.status?.length ? ` [${signal.status.join(",")}]` : ""}${signal.bodyRegex ? ` AND Body ~ ${signal.bodyRegex}` : ""}`);
  return out;
}
const filteredEntries = computed(() => {
  const term = normalizeSearch(query.value);
  return catalogEntries.value.filter(entry => {
    if (sourceFilter.value !== "all" && entry.source !== sourceFilter.value) return false;
    if (!term) return true;
    const fp = entry.fingerprint;
    return normalizeSearch([fp.name, fp.id, ...signals(fp), fp.condition ? conditionText(fp.condition) : ""].join("\n")).includes(term);
  });
});
const totalPages = computed(() => Math.max(1, Math.ceil(filteredEntries.value.length / pageSize)));
const currentPage = computed(() => Math.min(Math.max(1, page.value), totalPages.value));
const pagedEntries = computed(() => filteredEntries.value.slice((currentPage.value - 1) * pageSize, currentPage.value * pageSize));
watch([query, sourceFilter], () => { page.value = 1; });
watch(totalPages, count => { page.value = Math.min(Math.max(1, page.value), count); });
function clearSearch() { query.value = ""; page.value = 1; }
function previousPage() { page.value = Math.max(1, currentPage.value - 1); }
function nextPage() { page.value = Math.min(totalPages.value, currentPage.value + 1); }

async function load() {
  error.value = "";
  try {
    const data = await invoke<Library>("scan_rule_library");
    fingerprints.value = data?.fingerprints?.fingerprints ?? [];
    exposures.value = data?.exposures?.checks ?? [];
    userFingerprints.value = data?.user?.fingerprints ?? [];
    userPath.value = data?.user?.path ?? "";
    booleanPath.value = data?.user?.booleanPath ?? "";
    userError.value = data?.user?.error ?? "";
    effectiveFingerprints.value = data?.effective?.fingerprints ?? null;
    effectiveError.value = data?.effective?.error ?? "";
    booleanRuntime.value = data?.booleanRuntime ?? "";
    page.value = 1;
  } catch (e) { error.value = String(e); }
  finally { loading.value = false; }
}
onMounted(load);
async function reloadLibrary() {
  if (busy.value) return;
  busy.value = true;
  try { await load(); } finally { busy.value = false; }
}
function cancel() { mode.value = ""; editorText.value = ""; actionError.value = ""; }
function guardConditionRuntime(document: unknown) {
  const object = document && typeof document === "object" ? document as Record<string, unknown> : null;
  const rules = Array.isArray(document) ? document : Array.isArray(object?.fingerprints) ? object.fingerprints : [document];
  const incompatibleMode = rules.some(rule => !!rule && typeof rule === "object" && "matchMode" in rule && (rule as Record<string, unknown>).matchMode !== "any");
  if (incompatibleMode) throw new Error(tr("不支持平面 matchMode 设置；AND 或混合条件请使用 condition 条件树，不能按旧规则 OR 导入。", "Flat matchMode settings are unsupported. Use a condition tree for AND or mixed conditions; they must not be imported as legacy OR rules."));
  const hasTree = rules.some(rule => !!rule && typeof rule === "object" && (rule as Record<string, unknown>).condition != null);
  if (hasTree && !booleanSupported.value) throw new Error(tr("当前后端不支持条件树，请使用包含 AND/OR 匹配器的新版应用。", "The current backend does not support condition trees. Use the app with the AND/OR matcher."));
}
async function startAdd() {
  if (busy.value) return;
  actionError.value = "";
  try {
    const template = await invoke<Record<string, unknown>>("scan_fingerprint_template");
    editorText.value = JSON.stringify(template, null, 2);
    mode.value = "add";
  } catch (e) { actionError.value = String(e); }
}
function startBooleanAdd() {
  if (busy.value || !booleanSupported.value) return;
  actionError.value = "";
  editorText.value = JSON.stringify({
    id: "my-boolean-product", name: tr("我的条件指纹", "My conditional fingerprint"),
    condition: { type: "all", children: [
      { type: "body", pattern: "MyProduct" },
      { type: "any", children: [
        { type: "header", name: "server", pattern: "MyProductServer" },
        { type: "body", pattern: "MyProductConsole" },
      ] },
    ] },
  }, null, 2);
  mode.value = "add";
}
function startImport() { if (!busy.value) { actionError.value = ""; editorText.value = ""; mode.value = "import"; } }
async function startExport() {
  if (busy.value) return;
  actionError.value = "";
  busy.value = true;
  try { editorText.value = await invoke<string>("scan_export_user_fingerprints"); mode.value = "export"; }
  catch (e) { actionError.value = String(e); }
  finally { busy.value = false; }
}
async function saveAdd() {
  if (busy.value) return;
  busy.value = true; actionError.value = "";
  try {
    let fp: unknown;
    try { fp = JSON.parse(editorText.value); } catch { throw new Error(tr("JSON 格式错误，请检查", "Invalid JSON")); }
    guardConditionRuntime(fp);
    userFingerprints.value = await invoke<Fingerprint[]>("scan_add_user_fingerprint", { fingerprint: fp });
    await load();
    cancel();
  } catch (e) { actionError.value = String(e); }
  finally { busy.value = false; }
}
async function doImport() {
  if (busy.value) return;
  busy.value = true; actionError.value = "";
  try {
    let document: unknown;
    try { document = JSON.parse(editorText.value); } catch { throw new Error(tr("JSON 格式错误，请检查", "Invalid JSON")); }
    guardConditionRuntime(document);
    userFingerprints.value = await invoke<Fingerprint[]>("scan_import_user_fingerprints", { text: editorText.value });
    await load();
    cancel();
  } catch (e) { actionError.value = String(e); }
  finally { busy.value = false; }
}
async function remove(id: string) {
  if (busy.value) return;
  busy.value = true; actionError.value = "";
  try { userFingerprints.value = await invoke<Fingerprint[]>("scan_delete_user_fingerprint", { id }); await load(); }
  catch (e) { actionError.value = String(e); }
  finally { busy.value = false; }
}
async function copyEditor() { try { await navigator.clipboard.writeText(editorText.value); } catch { /* clipboard may be unavailable */ } }
</script>
<template>
  <div class="fingerprint-library" :aria-busy="busy || loading">
    <p v-if="loading" class="fp-muted" role="status">{{ tr("加载中…", "Loading…") }}</p>
    <template v-else>
      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <p v-if="actionError && !mode" class="form-error" role="alert">{{ actionError }}</p>
      <header class="fp-group-head">
        <div class="fp-heading"><h3 id="fp-catalog-title">{{ tr("规则目录", "Rule catalog") }}</h3><span class="fp-count">{{ tr("产品与框架识别", "Product & framework identification") }}</span></div>
        <div class="fp-actions">
          <button type="button" class="button primary compact" :disabled="busy || !!mode || !catalogReady" @click="startAdd">{{ tr("添加指纹", "Add fingerprint") }}</button>
          <button type="button" class="button ghost compact" :disabled="busy || !!mode || !catalogReady || !booleanSupported" @click="startBooleanAdd">{{ tr("AND / OR 模板", "AND / OR template") }}</button>
          <button type="button" class="button ghost compact" :disabled="busy || !!mode || !catalogReady" @click="startImport">{{ tr("导入", "Import") }}</button>
          <button type="button" class="button ghost compact" :disabled="busy || !!mode || !catalogReady" @click="startExport">{{ tr("导出", "Export") }}</button>
        </div>
      </header>
      <p v-if="!booleanSupported" class="fp-runtime-note" role="status">{{ tr("当前后端不支持条件树；AND / OR 添加与导入不可用。", "This backend does not support condition trees. Adding or importing AND / OR rules is unavailable.") }}</p>
      <div v-if="mode" class="fp-editor">
        <label class="fp-editor-label" for="fp-rule-editor">{{ mode === 'add' ? tr("新指纹（单条 JSON）", "New fingerprint (one JSON object)") : mode === 'import' ? tr("粘贴规则 JSON（合并并保留已有 ID）", "Paste rules JSON (merge, preserving existing IDs)") : tr("全部自定义规则（整份 JSON）", "All custom rules (whole JSON)") }}</label>
        <textarea id="fp-rule-editor" v-model="editorText" class="fp-textarea" spellcheck="false" :readonly="mode === 'export'" :disabled="busy" rows="12"></textarea>
        <p v-if="actionError" class="form-error" role="alert">{{ actionError }}</p>
        <div class="fp-editor-actions">
          <button v-if="mode === 'add'" type="button" class="button primary compact" :disabled="busy" @click="saveAdd">{{ tr("保存", "Save") }}</button>
          <button v-else-if="mode === 'import'" type="button" class="button primary compact" :disabled="busy" @click="doImport">{{ tr("合并导入", "Merge import") }}</button>
          <button v-else type="button" class="button ghost compact" :disabled="busy" @click="copyEditor">{{ tr("复制", "Copy") }}</button>
          <button type="button" class="button ghost compact" :disabled="busy" @click="cancel">{{ tr("关闭", "Close") }}</button>
        </div>
      </div>
      <p v-if="userError" class="form-error" role="alert">{{ tr("规则文件校验失败，原文件已保留", "Rule validation failed; the original file is retained") }}：{{ userError }}</p>
      <section class="fp-group fp-catalog" aria-labelledby="fp-catalog-title">
        <p v-if="!catalogReady" class="form-error" role="alert">{{ tr("当前统计不可用，请修复读取错误后重新加载。", "Current counts are unavailable. Fix the loading error and reload.") }} {{ effectiveError }}</p>
        <dl v-if="catalogReady" class="fp-stats" :aria-label="tr('指纹数量', 'Fingerprint counts')">
          <div class="fp-stat-total"><dt>{{ tr("去重总数", "Unique total") }}</dt><dd>{{ effectiveCount }}</dd></div>
          <div><dt>{{ tr("内置生效", "Effective builtin") }}</dt><dd>{{ builtinCount }}</dd></div>
          <div><dt>{{ tr("自定义生效", "Effective custom") }}</dt><dd>{{ customCount }}</dd></div>
        </dl>
        <div class="fp-search-toolbar" role="search" :aria-label="tr('搜索指纹', 'Search fingerprints')">
          <label class="fp-search-field" for="fp-catalog-query"><span class="fp-sr-only">{{ tr("框架名称、ID 或特征", "Framework name, ID or feature") }}</span><svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 4.5 4.5"/></svg><input id="fp-catalog-query" v-model="query" type="search" :disabled="!catalogReady" :placeholder="tr('搜索框架名称、ID 或特征…', 'Search framework name, ID or feature…')" autocomplete="off" spellcheck="false" /></label>
          <label class="fp-source-field" for="fp-catalog-source"><span class="fp-sr-only">{{ tr("来源", "Source") }}</span><select id="fp-catalog-source" v-model="sourceFilter" :disabled="!catalogReady"><option value="all">{{ tr("全部来源", "All sources") }}</option><option value="builtin">{{ tr("内置", "Builtin") }}</option><option value="custom">{{ tr("自定义", "Custom") }}</option></select></label>
          <button type="button" class="button ghost compact fp-clear" :disabled="!query" @click="clearSearch">{{ tr("清空", "Clear") }}</button>
          <button type="button" class="button ghost compact fp-reload" :disabled="busy || !!mode" @click="reloadLibrary">{{ tr("刷新", "Refresh") }}</button>
        </div>
        <template v-if="catalogReady">
          <p class="fp-result-count fp-muted" role="status" aria-live="polite">{{ tr(`匹配 ${filteredEntries.length} / ${effectiveCount} 条`, `${filteredEntries.length} of ${effectiveCount} rules match`) }}</p>
          <ul class="fp-list">
            <li v-for="entry in pagedEntries" :key="entry.fingerprint.id" class="fp-catalog-item">
              <details class="fp-details">
                <summary class="fp-result-summary"><strong :title="entry.fingerprint.name">{{ entry.fingerprint.name }}</strong><code class="fp-id" :title="entry.fingerprint.id">{{ entry.fingerprint.id }}</code><span class="fp-tag" :class="{ 'fp-tag-builtin': entry.source === 'builtin' }">{{ entry.source === 'custom' ? tr("自定义", "Custom") : tr("内置", "Builtin") }}</span><span class="fp-match-mode">{{ entry.fingerprint.condition ? "AND / OR" : tr("任一信号", "Any signal") }}</span></summary>
                <div class="fp-detail-body">
                  <div class="fp-detail-identity"><strong>{{ entry.fingerprint.name }}</strong><code>{{ entry.fingerprint.id }}</code></div>
                  <pre v-if="entry.fingerprint.condition" class="fp-condition-tree">{{ conditionText(entry.fingerprint.condition) }}</pre>
                  <template v-else>
                    <p class="fp-muted">{{ tr("以下任一信号命中即可识别；每个路径内部仍按状态码与正文条件组合判断。", "Any signal below identifies the product. Each path retains its status and body conditions.") }}</p>
                    <ul class="fp-signals"><li v-for="(signal, index) in signals(entry.fingerprint)" :key="index"><code>{{ signal }}</code></li></ul>
                  </template>
                  <button v-if="entry.source === 'custom'" type="button" class="button ghost compact fp-delete" :disabled="busy || !!mode" @click="remove(entry.fingerprint.id)">{{ tr("删除这条自定义规则", "Delete this custom rule") }}</button>
                </div>
              </details>
            </li>
            <li v-if="!filteredEntries.length" class="fp-empty">{{ effectiveCount ? tr("没有匹配结果，请调整名称、特征或来源筛选。", "No matches. Change the name, feature or source filter.") : tr("暂无可用指纹，可添加或导入规则。", "No fingerprints are available. Add or import rules.") }}</li>
          </ul>
          <nav v-if="totalPages > 1" class="fp-pagination" :aria-label="tr('指纹分页', 'Fingerprint pages')"><span class="fp-count">{{ tr(`第 ${currentPage} / ${totalPages} 页，每页 ${pageSize} 条`, `Page ${currentPage} of ${totalPages}, ${pageSize} per page`) }}</span><button type="button" class="button ghost compact" :disabled="currentPage <= 1" @click="previousPage">{{ tr("上一页", "Previous") }}</button><button type="button" class="button ghost compact" :disabled="currentPage >= totalPages" @click="nextPage">{{ tr("下一页", "Next") }}</button></nav>
        </template>
      </section>
      <div class="fp-support">
        <details class="fp-file-details">
          <summary>{{ tr("规则说明与文件位置", "Rule behavior & file locations") }}</summary>
          <div class="fp-support-body">
            <p>{{ tr("按精确产品 ID 去重，自定义同 ID 规则覆盖内置。名称相同但 ID 不同的规则分别保留。", "Exact product IDs determine uniqueness. Custom rules override builtin rules with the same ID; identical names with different IDs stay separate.") }}</p>
            <p>{{ tr("旧规则按任一信号命中；条件树保留 AND / OR 层级。导入按 ID 合并，保留已有规则。", "Legacy rules match any signal; condition trees retain AND / OR nesting. Import merges by ID and keeps existing rules.") }}</p>
            <p v-if="booleanSupported">{{ tr("当前支持混合条件；条件树独立保存，旧规则文件保持原格式。", "Mixed conditions are supported and stored separately; the legacy file retains its format.") }}</p>
            <p v-if="userPath" class="fp-path">{{ tr("旧规则", "Legacy rules") }}：{{ userPath }}</p>
            <p v-if="booleanPath" class="fp-path">{{ tr("AND / OR 条件树", "AND / OR condition trees") }}：{{ booleanPath }}</p>
            <p>{{ tr("保存后下一次扫描读取；导入或删除后，列表和统计会重新加载。", "The next scan reads saved files. Importing or deleting reloads the catalog and counts.") }}</p>
          </div>
        </details>
        <details class="fp-exposure-group">
          <summary>{{ tr("主动暴露检查", "Active exposure checks") }}<span class="fp-count">{{ exposures.length }}</span></summary>
          <div class="fp-support-body">
            <p class="fp-muted">{{ tr("以下检查独立于产品指纹，不计入指纹总数，也不随指纹导出。", "These checks are separate from product fingerprints and excluded from their count and export.") }}</p>
            <ul class="fp-list"><li v-for="check in exposures" :key="check.id" class="fp-item fp-exposure"><div class="fp-item-head"><strong>{{ check.title }}</strong><span class="fp-sev" :class="`sev-${check.severity}`">{{ check.severity }}</span></div><div class="fp-signals"><code>{{ check.path }}</code><code class="fp-id">{{ check.id }}</code></div></li><li v-if="!exposures.length" class="fp-empty">{{ tr("暂无检查项", "No checks") }}</li></ul>
          </div>
        </details>
      </div>
    </template>
  </div>
</template>
<style scoped>
.fingerprint-library{display:grid;gap:16px;min-width:0;padding:0;color:var(--ui-ink)}
.fingerprint-library p{margin:0}
.fingerprint-library .button{min-height:36px;padding:8px 12px;font:600 12px/1.4 var(--font-sans);border-radius:8px;white-space:nowrap}
.fp-muted{color:var(--ui-muted);font-size:12px;line-height:1.6}
.fp-runtime-note{padding:10px 12px;border:1px solid var(--ui-line);border-radius:8px;background:var(--ui-surface-raised);color:var(--ui-copy);font-size:12px;line-height:1.6}
.fp-group{display:grid;gap:12px;min-width:0}
.fp-group-head{display:flex;align-items:center;justify-content:space-between;flex-wrap:wrap;gap:12px}
.fp-heading{display:flex;flex-wrap:wrap;align-items:baseline;gap:6px 12px;min-width:0}
.fp-group-head h3{margin:0;color:var(--ui-ink);font-size:14px;font-weight:600}
.fp-count{color:var(--ui-muted);font-size:12px;font-variant-numeric:tabular-nums}
.fp-actions{display:flex;flex-wrap:wrap;gap:6px}
.fp-stats{display:flex;align-items:center;flex-wrap:wrap;gap:12px 24px;margin:0;padding:0 0 4px}
.fp-stats div{display:flex;align-items:baseline;gap:8px}
.fp-stats dt{color:var(--ui-muted);font-size:12px}
.fp-stats dd{margin:0;color:var(--ui-ink);font:600 16px/1.5 var(--font-sans);font-variant-numeric:tabular-nums}
.fp-stat-total dd{font-size:20px}
.fp-search-toolbar{display:flex;align-items:center;gap:8px;min-width:0}
.fp-search-field{display:flex;align-items:center;gap:9px;flex:1;min-width:140px;height:38px;padding:0 11px;border:1px solid var(--ui-line);border-radius:8px;background:var(--ui-surface);color:var(--ui-muted);transition:border-color .15s,box-shadow .15s}
.fp-search-field svg{flex-shrink:0}
.fp-search-field:focus-within{border-color:var(--ui-accent);box-shadow:0 0 0 3px color-mix(in srgb,var(--ui-accent) 12%,transparent)}
.fp-search-field input{width:100%;min-width:0;height:100%;padding:0;border:0;outline:0;border-radius:0;background:transparent;color:var(--ui-ink);font:12px/1.5 var(--font-sans);box-shadow:none;appearance:none;-webkit-appearance:none}
.fp-search-field input:focus{outline:0;border:0;box-shadow:none;background:transparent}
.fp-search-field input::placeholder{color:var(--ui-muted)}
.fp-source-field{flex:0 0 132px;min-width:0}
.fp-source-field select{width:100%;height:38px;padding:0 10px;border:1px solid var(--ui-line);border-radius:8px;background:var(--ui-surface);color:var(--ui-ink);font:12px/1.5 var(--font-sans)}
.fp-source-field select:focus-visible,.fp-textarea:focus-visible{outline:2px solid var(--ui-accent);outline-offset:2px}
.fp-search-toolbar .button{height:38px;min-height:38px}
.fp-result-count{font-variant-numeric:tabular-nums}
.fp-list{display:grid;margin:0;padding:0;list-style:none;min-width:0}
.fp-catalog-item{min-width:0;border-top:1px solid var(--ui-line)}
.fp-catalog-item:last-child{border-bottom:1px solid var(--ui-line)}
.fp-result-summary{display:grid;grid-template-columns:10px minmax(130px,1.3fr) minmax(90px,1fr) 60px 90px;align-items:center;column-gap:12px;min-height:44px;padding:9px 8px;cursor:pointer;color:var(--ui-ink);list-style:none;border-radius:6px;transition:background-color .15s}
.fp-result-summary::-webkit-details-marker{display:none}
.fp-result-summary::before{content:"";width:6px;height:6px;border-right:1.5px solid var(--ui-muted);border-bottom:1.5px solid var(--ui-muted);transform:rotate(-45deg);transition:transform .15s}
.fp-result-summary:hover{background:var(--ui-surface-raised)}
.fp-result-summary:focus-visible,.fp-support summary:focus-visible{outline:2px solid var(--ui-accent);outline-offset:2px}
.fp-details[open]>.fp-result-summary::before{transform:rotate(45deg)}
.fp-result-summary strong{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:13px;font-weight:600}
.fp-id{color:var(--ui-muted);font:12px/1.5 var(--font-mono);overflow-wrap:anywhere}
.fp-result-summary>.fp-id{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.fp-tag{justify-self:start;padding:2px 7px;border-radius:5px;background:color-mix(in srgb,var(--ui-accent) 8%,var(--ui-surface));color:color-mix(in srgb,var(--ui-accent) 82%,var(--ui-ink));font:500 11px/1.5 var(--font-sans);white-space:nowrap}
.fp-tag-builtin{background:var(--ui-surface-raised);color:var(--ui-muted)}
.fp-match-mode{justify-self:end;color:var(--ui-muted);font-size:11px;white-space:nowrap}
.fp-detail-body{display:grid;gap:10px;min-width:0;padding:6px 12px 16px 30px}
.fp-detail-identity{display:grid;gap:3px;font-size:12px;overflow-wrap:anywhere}
.fp-detail-identity strong{font-weight:600}.fp-detail-identity code{color:var(--ui-muted);font:11px/1.6 var(--font-mono)}
.fp-condition-tree{max-width:100%;margin:0;padding:12px;border:1px solid var(--ui-line);border-radius:8px;background:var(--ui-surface-raised);color:var(--ui-copy);font:12px/1.7 var(--font-mono);white-space:pre-wrap;overflow-wrap:anywhere}
.fp-signals{display:grid;gap:6px;min-width:0;margin:0;padding:0;list-style:none}
.fp-signals code{display:inline-block;max-width:100%;padding:3px 8px;border:1px solid var(--ui-line);border-radius:6px;background:var(--ui-surface-raised);color:var(--ui-copy);font:12px/1.6 var(--font-mono);white-space:pre-wrap;overflow-wrap:anywhere}
.fp-delete{justify-self:start}
.fp-pagination{display:flex;align-items:center;flex-wrap:wrap;gap:8px;padding-top:2px}
.fp-pagination>.fp-count{margin-right:auto}
.fp-empty{padding:20px 8px;border-top:1px solid var(--ui-line);color:var(--ui-muted);font-size:12px;line-height:1.7}
.fp-editor{display:grid;gap:10px;padding:14px;border:1px solid var(--ui-line);border-radius:8px;background:var(--ui-surface-raised)}
.fp-editor-label{color:var(--ui-ink);font-size:12px;font-weight:600}
.fp-textarea{width:100%;box-sizing:border-box;resize:vertical;padding:10px;border:1px solid var(--ui-line);border-radius:6px;background:var(--ui-surface);color:var(--ui-ink);font:12px/1.6 var(--font-mono);tab-size:2}
.fp-editor-actions{display:flex;flex-wrap:wrap;gap:8px}
.fp-support{display:grid;border-top:1px solid var(--ui-line)}
.fp-support>details{min-width:0;border-bottom:1px solid var(--ui-line)}
.fp-support summary{padding:12px 4px;color:var(--ui-muted);font-size:12px;line-height:1.5;cursor:pointer}
.fp-support summary .fp-count{margin-left:8px}
.fp-support-body{display:grid;gap:8px;padding:0 4px 14px 16px;color:var(--ui-muted);font-size:12px;line-height:1.6;min-width:0}
.fp-path{color:var(--ui-muted);font:11px/1.6 var(--font-mono);overflow-wrap:anywhere}
.fp-item{display:grid;gap:8px;min-width:0;padding:12px 0;border-top:1px solid var(--ui-line)}
.fp-item-head{display:flex;align-items:center;flex-wrap:wrap;gap:6px 10px;min-width:0}
.fp-item-head strong{color:var(--ui-ink);font-size:13px;font-weight:600;overflow-wrap:anywhere}
.fp-sev{--sev:var(--ui-muted);margin-left:auto;padding:2px 7px;border-radius:5px;background:color-mix(in srgb,var(--sev) 9%,var(--ui-surface));color:color-mix(in srgb,var(--sev) 82%,var(--ui-ink));font:500 11px/1.5 var(--font-sans);text-transform:uppercase}
.sev-critical,.sev-high{--sev:var(--red)}.sev-medium{--sev:var(--amber)}.sev-low{--sev:var(--ui-accent)}
.fp-sr-only{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap;border:0}
@media(max-width:760px){.fingerprint-library{padding:0}.fp-group-head{align-items:flex-start}.fp-actions{width:100%}.fp-stats{gap:8px 18px}.fp-result-summary{grid-template-columns:10px minmax(0,1fr) auto auto;gap:4px 10px}.fp-result-summary>.fp-id{grid-column:2;grid-row:2;font-size:11px}.fp-result-summary>.fp-tag{grid-column:3;grid-row:1}.fp-match-mode{grid-column:4;grid-row:1}.fp-detail-body{padding-left:26px}.fp-search-toolbar{flex-wrap:wrap}.fp-search-field{flex-basis:100%}.fp-source-field{flex:1 1 140px}.fp-heading>.fp-count{font-size:11px}}
@media(max-width:420px){.fp-stats{gap:10px}.fp-stats div{gap:5px}.fp-stats dt{font-size:11px}.fp-stats dd{font-size:15px}.fp-actions .button{padding-inline:9px}.fp-match-mode{font-size:10px}.fp-result-summary{gap:4px 8px}}
</style>
