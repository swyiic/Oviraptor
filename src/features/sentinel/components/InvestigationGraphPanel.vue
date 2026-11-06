<script setup lang="ts">
import { computed, ref } from "vue";
import {
  Bot, Braces, CheckCircle2, ChevronDown, CircleStop, GitCompareArrows,
  MousePointerClick, Network, Send, ShieldAlert, ShieldCheck, Sparkles,
} from "@lucide/vue";
import type { InvestigationApiModel, InvestigationGraph, InvestigationHypothesis } from "../types";
import { buildRawHttpResponse, prettyHttpBody } from "../httpMessage";
import { useI18n } from "../../../i18n";

const props = defineProps<{ graph?: InvestigationGraph; busy?: boolean; updatingId?: number }>();
const emit = defineEmits<{
  status: [hypothesis: InvestigationHypothesis, status: string];
  replay: [api: InvestigationApiModel];
  "replay-hypothesis": [hypothesis: InvestigationHypothesis];
}>();
const { tr } = useI18n();

const metrics = computed(() => props.graph?.metrics);
const manualDeepDive = computed<any[]>(() =>
  Array.isArray(metrics.value?.decision?.manualDeepDive)
    ? metrics.value!.decision.manualDeepDive
    : [],
);
function manualPriorityLabel(value: string) {
  return ({
    critical: tr("最高优先", "Top priority"), high: tr("高优先", "High priority"),
    medium: tr("中优先", "Medium priority"), low: tr("低优先", "Low priority"),
  } as Record<string, string>)[value] || value || tr("待安排", "Unscheduled");
}
const standardInvestigationAllowed = computed(() => Boolean(metrics.value?.decision?.standardInvestigationAllowed));
const sourceGuidedInvestigationAllowed = computed(() => Boolean(metrics.value?.decision?.sourceGuidedInvestigationAllowed));
const investigationGateOpen = computed(() => Boolean(metrics.value?.tokenWorthy || standardInvestigationAllowed.value));
const investigationDecisionTitle = computed(() => {
  if (metrics.value?.tokenWorthy) return tr("风险证据已就绪：自动进入有界验证", "Risk evidence ready: entering bounded validation automatically");
  if (sourceGuidedInvestigationAllowed.value) return tr("源码映射已还原准确只读接口：进入有界目标调查", "Source maps recovered exact read-only endpoints: entering bounded target investigation");
  if (standardInvestigationAllowed.value) return tr("运行时接口已就绪：进入有界标准调查", "Runtime endpoints ready: entering bounded standard investigation");
  if (legacySourceMappedRead.value) return tr("已有可追溯的源码映射只读接口：继续当前任务即可进入新调查门禁", "Traceable source-mapped read-only endpoints found: continue the current task to reach the new investigation gate");
  return tr("本地收口：尚无可重放的高价值接口证据", "Closed locally: no replayable high-value endpoint evidence yet");
});
const apis = computed(() => props.graph?.apis || []);
const legacySourceMappedRead = computed(() => apis.value.some((api) =>
  ["GET", "HEAD"].includes(String(api.method || "").toUpperCase())
  && String(api.confidence || "").toLowerCase() === "high"
  && String(api.source || "").toLowerCase().includes(".js.map#")
  && !/[<>{}]/.test(String(api.url || api.normalizedPath || "")),
));
const relatedServices = computed(() => props.graph?.relatedServices || []);
const hypotheses = computed(() => props.graph?.hypotheses || []);
const isNoiseHypothesis = (item: InvestigationHypothesis) => {
  const contract = item.contract || {};
  const endpoint = String(contract.endpoint || "").toLowerCase();
  return String(contract.method || "").toUpperCase() === "OPTIONS" || /data_report_web|sentry|envelope|deviceprofile|telemetry/.test(endpoint);
};
const rawActionableHypotheses = computed(() => hypotheses.value.filter((item) => !isNoiseHypothesis(item)));
function normalizedEndpoint(value: unknown) {
  const text = String(value || "").split("#")[0].split("?")[0];
  return text.replace(/^https?:\/\/[^/]+/i, "").replace(/\/$/, "") || "/";
}
const actionableHypotheses = computed(() => {
  const grouped = new Map<string, InvestigationHypothesis>();
  for (const item of rawActionableHypotheses.value) {
    const contract = item.contract || {};
    const key = [String(contract.method || "GET").toUpperCase(), normalizedEndpoint(contract.endpoint), String(contract.kind || item.category)].join("|");
    const current = grouped.get(key);
    if (!current) {
      grouped.set(key, { ...item, decision: { ...(item.decision || {}), mergedCount: 1 } });
      continue;
    }
    const mergedCount = Number(current.decision?.mergedCount || 1) + 1;
    const preferred = item.score > current.score ? item : current;
    grouped.set(key, { ...preferred, decision: { ...(preferred.decision || {}), mergedCount } });
  }
  return [...grouped.values()].sort((left, right) => right.score - left.score);
});
const identities = computed(() => props.graph?.nodes.filter((item) =>
  item.nodeType === "identity" && String(item.payload?.identityKey || "").toLowerCase() !== "anonymous"
) || []);
const readyHypotheses = computed(() => actionableHypotheses.value.filter((item) => ["ready", "in_progress"].includes(item.status)));
const showAll = ref(false);
const contractFilter = ref("ready");
const apiQuery = ref("");
const selectedApiKey = ref("");
const expandedApiKeys = ref<string[]>([]);
const showAllRelatedServices = ref(false);
function toggleApiDetails(api: InvestigationApiModel) {
  const key = api.apiKey || `${api.method}|${api.normalizedPath || api.url}`;
  selectedApiKey.value = key;
  expandedApiKeys.value = expandedApiKeys.value.includes(key)
    ? expandedApiKeys.value.filter((value) => value !== key)
    : [...expandedApiKeys.value, key];
}
function apiExpanded(api: InvestigationApiModel) {
  const key = api.apiKey || `${api.method}|${api.normalizedPath || api.url}`;
  return expandedApiKeys.value.includes(key);
}
function apiJson(value: any) {
  return JSON.stringify(value ?? {}, null, 2);
}
function apiHeaders(api: InvestigationApiModel) {
  return api.requestHeaders || api.payload?.requestHeaders || {};
}
function apiResponseHeaders(api: InvestigationApiModel) {
  return api.responseHeaders || api.payload?.responseHeaders || {};
}
function apiSourceLabel(api: InvestigationApiModel) {
  return [api.source || "unknown", api.confidence || "unknown", api.captureStatus || "capture status unknown"].join(" · ");
}
function apiEvidenceKind(api: InvestigationApiModel) {
  const source = String(api.source || "").toLowerCase();
  return source.includes("runtime") ? tr(`CDP 观察 ${api.observedCount} 次`, `Seen ${api.observedCount}× via CDP`)
    : source.includes(".js.map#") ? tr("源码映射准确调用", "Exact call from source map") : tr("静态契约候选", "Static contract candidate");
}
function apiObservedResponse(api: InvestigationApiModel) {
  return api.decodedBody || api.responseBody || api.payload?.decodedBody || api.payload?.responseBody || tr("未采集响应正文", "Response body not captured");
}
const contractCounts = computed(() => ({
  actionable: actionableHypotheses.value.length,
  ready: actionableHypotheses.value.filter((item) => ["ready", "in_progress", "awaiting_authorization", "blocked_by_authorization"].includes(item.status)).length,
  completed: actionableHypotheses.value.filter((item) => ["validated", "confirmed", "rejected", "normal", "exhausted"].includes(item.status)).length,
}));
const filteredHypotheses = computed(() => actionableHypotheses.value.filter((item) => {
  if (contractFilter.value === "ready") return ["ready", "in_progress", "awaiting_authorization", "blocked_by_authorization"].includes(item.status);
  if (contractFilter.value === "completed") return ["validated", "confirmed", "rejected", "normal", "exhausted"].includes(item.status);
  return true;
}));
const visibleHypotheses = computed(() => showAll.value ? filteredHypotheses.value : filteredHypotheses.value.slice(0, 24));
const identityLabels = computed(() => identities.value.map((item, index) => ({
  item,
  key: String(item.payload?.identityKey || item.label || item.nodeKey || ""),
  label: String(item.payload?.identityLabel || item.payload?.displayName || (item.label?.startsWith("session:") ? tr(`账号 ${String.fromCharCode(65 + index)}`, `Account ${String.fromCharCode(65 + index)}`) : item.label) || tr(`账号 ${String.fromCharCode(65 + index)}`, `Account ${String.fromCharCode(65 + index)}`)),
  captureStatus: identityCaptureStatus(item),
})));
function identityCaptureStatus(item: any) {
  const payload = item?.payload || {};
  if (payload.captureStatus) return String(payload.captureStatus);
  if (payload.valid === false) return "failed";
  if (payload.runtimeStopReason || payload.observed === false) return "partial";
  if (payload.valid === true || payload.observed === true) return "complete";
  return "unknown";
}
function captureStatusLabel(value: string) {
  return ({
    complete: tr("采集完整", "Capture complete"), partial: tr("采集不完整", "Capture incomplete"),
    missing: tr("未采集", "Not captured"), failed: tr("采集失败", "Capture failed"), unknown: tr("状态未知", "Status unknown"),
  } as Record<string, string>)[value] || value;
}
function identityDisplay(key: string, index: number) {
  if (String(key || "").trim().toLowerCase() === "anonymous") return tr("匿名访问", "Anonymous");
  const match = identityLabels.value.find((entry) => entry.key === key || entry.item.nodeKey === key);
  return match?.label || tr(`账号 ${String.fromCharCode(65 + index)}`, `Account ${String.fromCharCode(65 + index)}`);
}
function relatedServiceLabel(value: string) {
  return ({
    monitoring_telemetry: tr("监控 / 遥测", "Monitoring / telemetry"),
    device_fingerprint: tr("设备指纹", "Device fingerprint"),
    page_bootstrap: tr("页面初始化", "Page bootstrap"),
    background_service: tr("后台关联服务", "Background related service"),
  } as Record<string, string>)[value] || value || tr("关联服务", "Related service");
}
function relatedIdentityLabels(keys: string[]) {
  return (keys || []).map((key, index) => identityDisplay(key, index)).join(" / ") || tr("未标注身份", "No identity labeled");
}
const visibleRelatedServices = computed(() => showAllRelatedServices.value ? relatedServices.value : relatedServices.value.slice(0, 6));
function identityCaptureForKey(key: string, value?: any) {
  const explicit = String(value?.captureStatus || "").toLowerCase();
  if (explicit) return explicit;
  return identityLabels.value.find((entry) => entry.key === key || entry.item.nodeKey === key)?.captureStatus || "unknown";
}
function diffIsComparable(diff: any) {
  const entries = matrixEntries(diff);
  return entries.length >= 2 && entries.every(([key, value]) => ["complete", "confirmed"].includes(identityCaptureForKey(key, value)));
}
function diffTypeLabel(value: string) {
  return ({
    reachability: tr("仅单侧观察到接口", "Endpoint seen on one side only"),
    status: tr("响应状态不同", "Response status differs"),
    response_schema: tr("响应字段不同", "Response fields differ"),
    cross_identity_replay_candidate: tr("对象权限待交叉验证", "Object access pending cross-check"),
    feature_surface: tr("功能入口仅单侧可见", "Feature entry visible on one side only"),
  } as Record<string, string>)[value] || value || tr("身份差异", "Identity difference");
}
function diffLabel(diff: any) { return diffIsComparable(diff) ? diffTypeLabel(diff.differenceType) : tr("采集链路不完整，暂不判定", "Capture incomplete; not judged yet"); }
const telemetry = (api: InvestigationApiModel) => /(?:data_report_web|sentry|envelope|deviceprofile|telemetry|\/pixel|\/beacon)/i.test(`${api.normalizedPath} ${api.url}`);
function cleanResponseKeys(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return [...new Set(value.map(String).map((item) => item.trim()).filter((item) =>
    item.length > 0 && item.length <= 160 && !/^[{[]/.test(item) && !/[\r\n]/.test(item) && !/^https?:\/\//i.test(item),
  ))].slice(0, 80);
}
function backgroundEndpoint(value: string) {
  const endpoint = value.toLowerCase().split("#")[0].split("?")[0];
  return /(?:data_report_web|sentry|\/envelope|deviceprofile|telemetry|\/pixel|\/beacon)/.test(endpoint)
    || /\.(?:avif|bmp|css|eot|gif|ico|jpe?g|map|mp[34]|pdf|png|svg|ttf|webp|woff2?)$/.test(endpoint)
    || /(?:\/categories|\/banner|\/feeds?|\/search\/found)$/.test(endpoint)
    || endpoint.includes("/welcome_page");
}
const filteredApis = computed(() => apis.value
  .filter((api) => !apiQuery.value.trim() || `${api.method} ${api.url} ${api.normalizedPath} ${api.identityKeys.join(" ")}`.toLowerCase().includes(apiQuery.value.trim().toLowerCase())));
const businessApis = computed(() => apis.value.filter((api) => !telemetry(api)));
const selectedApi = computed(() => filteredApis.value.find((api) => api.apiKey === selectedApiKey.value) || filteredApis.value[0]);
const stopLabel = computed(() => {
  const reason = metrics.value?.stopReason || "";
  if (reason === "identity_matrix_complete" && Number(metrics.value?.decision?.identityCount || 0) <= 1) {
    return tr("匿名页面运行时采集完成；该旧任务曾错误使用身份矩阵收口名称", "Anonymous page runtime capture complete; this older task mislabeled it as identity matrix completion");
  }
  return ({
    confirmed_waf_or_challenge: tr("确认 WAF / 人机挑战，已立即停止", "WAF / bot challenge confirmed; stopped immediately"),
    incremental_no_new_value: tr("与上次基线一致，没有新证据", "Same as the previous baseline; no new evidence"),
    no_high_value_hypothesis: tr("没有达到门禁的高价值假设", "No high-value hypothesis passed the gate"),
    no_more_valuable_states: tr("页面状态和动作已探索完", "All page states and actions explored"),
    identity_matrix_complete: tr("多身份矩阵已完成；后续验证不应再重复抓取同一请求", "Multi-identity matrix complete; later validation should not capture the same requests again"),
    anonymous_runtime_complete: tr("匿名页面运行时采集完成", "Anonymous page runtime capture complete"),
    source_mapped_readonly_contracts: tr("浏览器未自然触发业务请求，但源码映射保留了高置信度只读调用；系统将只验证这些准确端点", "The browser did not trigger business requests on its own, but source maps kept high-confidence read-only calls; only these exact endpoints will be validated"),
    evidence_collection_complete: tr("确定性证据收集完成", "Deterministic evidence collection complete"),
  } as Record<string, string>)[reason] || reason || tr("已完成本地调查决策", "Local investigation decision made");
});
function contractItems(h: InvestigationHypothesis, key: string): string[] { const value = h.contract?.[key]; return Array.isArray(value) ? value.map(String) : []; }
function responseSummary(api: InvestigationApiModel) { return Object.keys(api.responseSchema || {}).slice(0, 8).join(", ") || tr("未提取响应字段", "No response fields extracted"); }
function requestSummary(api: InvestigationApiModel) { return Object.keys(api.requestSchema || {}).slice(0, 8).join(", ") || api.parameters.slice(0, 8).join(", ") || tr("无参数", "No parameters"); }
function matrixEntries(diff: any) { return Object.entries(diff.matrix || {}) as Array<[string, any]>; }
function apiObservation(api: InvestigationApiModel, identityKey: string) {
  const observations = Array.isArray(api.payload?.identityObservations) ? api.payload.identityObservations : [];
  return observations.find((value: any) => String(value?.identityKey || "") === identityKey) ||
    (api.identityKeys.includes(identityKey) ? { observed: true, status: api.payload?.statusCode, responseKeys: Object.keys(api.responseSchema || {}) } : undefined);
}
const identityEvidence = computed(() => identityLabels.value.map((entry) => {
  const rows = apis.value.map((api) => ({ api, observation: apiObservation(api, entry.key) })).filter((row) => row.observation);
  const responseKeys = [...new Set(rows.flatMap((row) => cleanResponseKeys(row.observation?.responseKeys)))];
  const statuses = [...new Set(rows.map((row) => row.observation?.status).filter((value) => value != null).map(String))];
  return { ...entry, apiCount: rows.length, observedCount: rows.filter((row) => row.observation?.observed !== false).length, responseKeys, statuses };
}));
function identitySessionLabel(entry: any) {
  const payload = entry.item?.payload || {};
  if (payload.sessionValid === true) return tr("会话有效", "Session valid");
  if (payload.sessionValid === false) return tr("会话已失效", "Session expired");
  if (["failed", "partial", "unavailable"].includes(String(payload.captureStatus || "").toLowerCase())) return tr("采集异常", "Capture error");
  return tr("等待会话证据", "Waiting for session evidence");
}
function identitySessionTone(entry: any) {
  const payload = entry.item?.payload || {};
  if (payload.sessionValid === true && entry.captureStatus === "complete") return "ok";
  if (payload.sessionValid === false || entry.captureStatus === "failed") return "bad";
  return "warn";
}
function identitySessionDetail(entry: any) {
  const payload = entry.item?.payload || {};
  const parts = [
    payload.statusCode ? tr(`入口 HTTP ${payload.statusCode}`, `Entry HTTP ${payload.statusCode}`) : tr("无入口状态码", "No entry status code"),
    tr(`${Number(payload.stateCount || 0)} 个页面状态`, `${Number(payload.stateCount || 0)} page states`),
    tr(`${Number(payload.apiCount || 0)} 个自然触发接口`, `${Number(payload.apiCount || 0)} naturally triggered endpoints`),
  ];
  if (Number(payload.replayObservedCount || 0)) parts.push(tr(`${payload.replayObservedCount} 个交叉重放响应`, `${payload.replayObservedCount} cross-replay responses`));
  return parts.join(" · ");
}
function diagnosticScalar(value: unknown) {
  if (value == null || value === "") return "";
  const text = String(value).trim();
  return !text || text === "unknown" ? "" : text;
}
function identityRuntimeDiagnostics(entry: any) {
  const payload = entry?.item?.payload || {};
  const nested = payload.runtimeDiagnostics && typeof payload.runtimeDiagnostics === "object"
    ? payload.runtimeDiagnostics
    : {};
  const rows = [
    ["captureError", diagnosticScalar(payload.captureError || nested.captureError)],
    ["stopReason", diagnosticScalar(payload.runtimeStopReason || nested.runtimeStopReason)],
    ["failedStage", diagnosticScalar(payload.failedStage || nested.failedStage)],
    ["cdpTransport", diagnosticScalar(payload.cdpTransport || nested.cdpTransport)],
    ["browser", diagnosticScalar(payload.browserVersion || nested.browserVersion)],
    ["node", diagnosticScalar(payload.nodeVersion || nested.nodeVersion)],
    ["exitCode", diagnosticScalar(payload.browserExitCode ?? nested.browserExitCode)],
    ["signal", diagnosticScalar(payload.browserSignal ?? nested.browserSignal)],
  ].filter(([, value]) => value);
  const stderr = diagnosticScalar(payload.browserStderr || nested.browserStderr);
  return { rows, stderr };
}
function hasIdentityRuntimeDiagnostics(entry: any) {
  const { rows, stderr } = identityRuntimeDiagnostics(entry);
  return rows.length > 0 || Boolean(stderr);
}
function diffEndpoint(diff: any) {
  const parts = String(diff.apiKey || "").split("|");
  const method = String(parts[0] || "GET").toUpperCase();
  const path = normalizedEndpoint(parts.find((part: string) => part.startsWith("/")) || parts[2] || parts[1] || tr("未知接口", "Unknown endpoint"));
  const host = parts.length > 2 && !String(parts[1]).startsWith("/") ? parts[1] : "";
  return { method, path, host };
}
function diffValue(diff: any, side: string) {
  const key = side === "left" ? diff.leftIdentityKey : diff.rightIdentityKey;
  return diff.matrix?.[key] || diff.matrix?.[side] || { observed: false, status: null, responseKeys: [] };
}
function responseKeys(value: any): string[] { return cleanResponseKeys(value?.responseKeys); }
function responseFieldDiff(diff: any) {
  const left = responseKeys(diffValue(diff, "left"));
  const right = responseKeys(diffValue(diff, "right"));
  return {
    common: left.filter((key) => right.includes(key)),
    leftOnly: left.filter((key) => !right.includes(key)),
    rightOnly: right.filter((key) => !left.includes(key)),
  };
}
function responseHeaders(value: any): Record<string, string> {
  const headers = value?.responseHeaders;
  return headers && typeof headers === "object" && !Array.isArray(headers) ? headers : {};
}
function responseBody(value: any) {
  return String(value?.responseBody || "");
}
function matchingApiForDiff(diff: any) {
  const endpoint = diffEndpoint(diff);
  return apis.value.find((api) => String(api.method || "GET").toUpperCase() === endpoint.method && normalizedEndpoint(api.normalizedPath || api.url) === endpoint.path);
}
function sideResponseHeaders(diff: any, side: string) {
  const value = diffValue(diff, side);
  const own = responseHeaders(value);
  if (Object.keys(own).length || !value?.observed) return own;
  const api = matchingApiForDiff(diff);
  return api ? apiResponseHeaders(api) : {};
}
function sideResponseBody(diff: any, side: string) {
  const value = diffValue(diff, side);
  if (responseBody(value)) return responseBody(value);
  if (!value?.observed) return "";
  const api = matchingApiForDiff(diff);
  return String(api?.decodedBody || api?.responseBody || api?.payload?.decodedBody || api?.payload?.responseBody || api?.payload?.responsePreview || "");
}
function sideHttpResponse(diff: any, side: string) {
  const value = diffValue(diff, side);
  if (!value?.observed) return tr("该账户未捕获到同一接口响应。", "This account did not capture a response from the same endpoint.");
  return buildRawHttpResponse({
    status: value.status == null ? null : Number(value.status),
    headers: sideResponseHeaders(diff, side),
    body: sideResponseBody(diff, side) || (responseKeys(value).length ? JSON.stringify(Object.fromEntries(responseKeys(value).map((key) => [key, tr("<已采集字段，旧任务未保留正文>", "<field captured; older task did not keep the body>")])), null, 2) : tr("<未保留响应正文>", "<response body not kept>")),
  });
}
function sidePrettyBody(diff: any, side: string) {
  const value = diffValue(diff, side);
  if (!value?.observed) return tr("该账户未捕获到同一接口响应。", "This account did not capture a response from the same endpoint.");
  return prettyHttpBody(sideResponseBody(diff, side)) || (responseKeys(value).length ? responseKeys(value).join("\n") : tr("旧任务未保留响应正文", "Older task did not keep the response body"));
}
function comparisonRows(diff: any) {
  const left = diffValue(diff, "left");
  const right = diffValue(diff, "right");
  const leftKeys = responseKeys(left);
  const rightKeys = responseKeys(right);
  const rows = [
    { label: tr("采集结果", "Capture result"), left: left.observed ? (left.replayed ? tr("自动重放", "Auto-replayed") : tr("页面触发", "Page-triggered")) : tr("未捕获", "Not captured"), right: right.observed ? (right.replayed ? tr("自动重放", "Auto-replayed") : tr("页面触发", "Page-triggered")) : tr("未捕获", "Not captured") },
    { label: tr("HTTP 状态", "HTTP status"), left: left.status == null ? "—" : String(left.status), right: right.status == null ? "—" : String(right.status) },
    { label: tr("内容类型", "Content type"), left: String(left.contentType || sideResponseHeaders(diff, "left")["content-type"] || sideResponseHeaders(diff, "left")["Content-Type"] || "—"), right: String(right.contentType || sideResponseHeaders(diff, "right")["content-type"] || sideResponseHeaders(diff, "right")["Content-Type"] || "—") },
    { label: tr("响应大小", "Response size"), left: left.responseBytes ? `${left.responseBytes} B` : "—", right: right.responseBytes ? `${right.responseBytes} B` : "—" },
    { label: tr("响应字段", "Response fields"), left: leftKeys.length ? leftKeys.join(tr("、", ", ")) : "—", right: rightKeys.length ? rightKeys.join(tr("、", ", ")) : "—" },
  ];
  return rows.map((row) => ({ ...row, changed: row.left !== row.right }));
}
function diffConclusion(diff: any) {
  if (!diffIsComparable(diff)) return tr("缺少完整浏览器采集；不会交给 AI 判为权限问题", "Full browser capture is missing; this will not go to AI as an access issue");
  if (diffLikelyNormal(diff)) return tr("AI 初判为页面内容、个性化或功能开关差异；另一账号只读重放后若无数据越界将自动关闭", "AI's initial read: a page content, personalization, or feature-flag difference. Closes automatically if a read-only replay from the other account shows no data crossing boundaries");
  const left = diffValue(diff, "left");
  if (diff.differenceType === "reachability") {
    const side = left.observed ? identityDisplay(diff.leftIdentityKey, 0) : identityDisplay(diff.rightIdentityKey, 1);
    return tr(`仅 ${side} 自然触发；系统将用另一账号自动重放同一只读请求后再判断`, `Only ${side} triggered it naturally; the same read-only request will be replayed from the other account before judging`);
  }
  if (diff.differenceType === "status") return tr("同一请求返回不同状态码；AI 将先区分正常权限边界与异常放行", "The same request returned different status codes; AI will first separate normal access boundaries from improper access");
  if (diff.differenceType === "response_schema") return tr("两侧响应字段结构不同；先排除个性化数据和功能开关，再判断越权风险", "Response field structure differs between the two sides; personalized data and feature flags are ruled out before judging authorization risk");
  if (diff.differenceType === "cross_identity_replay_candidate") return tr("发现对象标识；需确认另一账号是否获得不属于自己的对象", "Object identifier found; confirm whether the other account can get objects it does not own");
  return tr("当前只是一条差异证据，不直接等于漏洞", "This is only one piece of difference evidence, not a vulnerability by itself");
}
function diffLikelyNormal(diff: any) {
  if (!diffIsComparable(diff)) return false;
  const endpoint = `${diffEndpoint(diff).host}${diffEndpoint(diff).path}`.toLowerCase();
  if (diff.differenceType === "reachability" && /(?:\/feeds?(?:\/|$)|banner|categories|welcome_page|search\/found)/.test(endpoint)) return true;
  if (diff.differenceType === "status") {
    const statuses = [diffValue(diff, "left").status, diffValue(diff, "right").status].map(Number);
    return statuses.some((status) => status === 401 || status === 403) && statuses.some((status) => status >= 200 && status < 400);
  }
  return false;
}
const identityDiffRows = computed(() => {
  const grouped = new Map<string, any>();
  for (const diff of props.graph?.identityDiffs || []) {
    if (diff.differenceType === "feature_surface" || String(diff.apiKey || "").toLowerCase().startsWith("feature:")) continue;
    const endpoint = diffEndpoint(diff);
    if (backgroundEndpoint(`${endpoint.host}${endpoint.path}`)) continue;
    const key = `${endpoint.method}|${endpoint.path}|${diff.differenceType}`;
    const current = grouped.get(key);
    if (!current || (diffIsComparable(diff) && !diffIsComparable(current)) || diff.riskScore > current.riskScore) grouped.set(key, diff);
  }
  return [...grouped.values()].sort((left, right) => Number(diffLikelyNormal(left)) - Number(diffLikelyNormal(right)) || Number(diffIsComparable(right)) - Number(diffIsComparable(left)) || right.riskScore - left.riskScore);
});
const identityPendingDiffs = computed(() => identityDiffRows.value.filter((item) => !diffLikelyNormal(item)));
const identityNormalDiffs = computed(() => identityDiffRows.value.filter(diffLikelyNormal));
type HypothesisIdentityRow = { label: string; state: string; detail: string; tone: "ok" | "warn" | "unknown" };
function hypothesisIdentityRows(item: InvestigationHypothesis): HypothesisIdentityRow[] {
  const contract = item.contract || {};
  const runs = Array.isArray(contract.identityRuns) ? contract.identityRuns : [];
  const keys = Array.isArray(contract.identityKeys) ? contract.identityKeys.map(String) : [];
  const api = apis.value.find((candidate) => {
    const endpoint = String(contract.endpoint || "");
    return endpoint && (candidate.url === endpoint || candidate.normalizedPath === endpoint || candidate.url.includes(endpoint) || endpoint.includes(candidate.normalizedPath));
  });
  const apiKeys = api?.identityKeys || [];
  const identityKeys = [...new Set([...runs.map((run: any) => String(run.identityKey || "")), ...keys, ...apiKeys]
    .filter((key) => key && key.trim().toLowerCase() !== "anonymous"))];
  if (!identityKeys.length) return [];
  return identityKeys.slice(0, 4).map((key, index) => {
    const run = runs.find((value: any) => String(value?.identityKey || "") === key) || {};
    const identityNode = identityLabels.value.find((entry) => entry.key === key || entry.item.nodeKey === key)?.item;
    const nodePayload = identityNode?.payload || {};
    // Historical hypothesis contracts may contain the one-sided opportunity
    // snapshot that originally created the card. The graph API node is newer
    // and already merges A/B observations by stable HTTP contract, so it must
    // win when deciding whether this identity observed the endpoint.
    const observation = api ? apiObservation(api, key) : undefined;
    const label = String(run.identityLabel || identityDisplay(key, index));
    const capture = String(run.captureStatus || identityCaptureForKey(key, run)).toLowerCase();
    const sessionValid = run.sessionValid ?? nodePayload.sessionValid;
    const statusValue = observation?.status ?? observation?.statusCode ?? run.statusCode ?? nodePayload.statusCode;
    const statusCode = statusValue != null ? `HTTP ${statusValue}` : tr("无状态码", "No status code");
    const observed = observation?.observed ?? run.observed;
    if (observed === false) {
      return { label, state: tr("未观察到", "Not observed"), detail: tr(`会话入口 ${statusCode} · 未触发同一接口`, `Session entry ${statusCode} · same endpoint not triggered`), tone: "unknown" };
    }
    if (sessionValid === false && run.validationReason && run.validationReason !== "runtime_probe_unavailable") {
      return { label, state: tr("明确失效", "Confirmed invalid"), detail: `${statusCode} · ${run.validationReason}`, tone: "warn" };
    }
    if (["failed", "partial", "unavailable"].includes(capture) || sessionValid == null || run.validationReason === "runtime_probe_unavailable") {
      return { label, state: tr("不可比较", "Not comparable"), detail: tr(`${statusCode} · runtime 未完成`, `${statusCode} · runtime incomplete`), tone: "unknown" };
    }
    const responseKeys = cleanResponseKeys(observation?.responseKeys).length;
    return { label, state: tr("已捕获", "Captured"), detail: tr(`${statusCode} · ${responseKeys} 响应字段`, `${statusCode} · ${responseKeys} response fields`), tone: "ok" };
  });
}
function contractStateLabel(item: InvestigationHypothesis) {
  return ({
    ready: tr("AI 已接管，等待执行", "Taken over by AI, waiting to run"), in_progress: tr("AI 正在验证", "AI validating"),
    validated: tr("AI 已取得有效证据", "AI obtained valid evidence"),
    rejected: tr("AI 判定不成立", "AI ruled it out"), normal: tr("正常行为", "Normal behavior"),
    exhausted: tr("已验证，无新增证据", "Validated, no new evidence"),
    candidate: tr("证据不足，暂不消耗模型", "Insufficient evidence; no model spend yet"), needs_more_evidence: tr("需要补充证据", "Needs more evidence"),
    awaiting_authorization: tr("已由自动授权策略接管", "Handled by the auto-authorization policy"),
    blocked_by_authorization: tr("已由自动授权策略恢复", "Resumed by the auto-authorization policy"),
  } as Record<string, string>)[item.status] || item.status;
}
function contractPurpose(item: InvestigationHypothesis) {
  return String(item.contract?.objective || item.decision?.reason || tr("AI 将按固定次数比较控制请求与测试请求，并直接给出成立、不成立或证据不足结论。", "AI compares the control and test requests a fixed number of times, then concludes directly: confirmed, ruled out, or insufficient evidence."));
}
function evidenceLabel(value: string) {
  return ({
    control_response: tr("基准账号响应", "Baseline account response"), cross_identity_response: tr("另一账号响应", "Other account response"),
    object_ownership_context: tr("对象归属", "Object ownership"),
    status_body_or_field_difference: tr("状态/正文/字段差异", "Status/body/field difference"), authenticated_request: tr("登录请求", "Authenticated request"),
    anonymous_control: tr("匿名对照", "Anonymous control"),
    redirect_or_response_difference: tr("跳转或响应差异", "Redirect or response difference"), session_validity_signal: tr("会话有效性", "Session validity"),
    control_request: tr("控制请求", "Control request"), test_request: tr("测试请求", "Test request"),
    status_timing_or_schema_difference: tr("状态/耗时/结构差异", "Status/timing/schema difference"),
    parameter_source: tr("参数来源", "Parameter source"), entry_source: tr("入口来源", "Entry source"), field_schema: tr("字段结构", "Field schema"),
    request_method: tr("请求方法", "Request method"), server_precondition: tr("服务端前置条件", "Server precondition"),
    source_evidence: tr("来源证据", "Source evidence"), control_result: tr("基准结果", "Baseline result"), test_result: tr("测试结果", "Test result"),
    impact_explanation: tr("影响说明", "Impact explanation"),
  } as Record<string, string>)[value] || value.replace(/_/g, " ");
}
function evidenceItems(item: InvestigationHypothesis) { return contractItems(item, "requiredEvidence").map(evidenceLabel); }
</script>

<template>
  <div class="investigation-graph-panel compact-investigation">
    <div v-if="busy" class="investigation-empty">{{ tr('正在装载调查图谱…', 'Loading investigation graph…') }}</div>
    <div v-else-if="!graph?.metrics" class="investigation-empty"><Network :size="22" /><strong>{{ tr('当前 URL 还没有调查图谱', 'No investigation graph for this URL yet') }}</strong><span>{{ tr('请先完成一次前端探测。', 'Run a front-end probe first.') }}</span></div>
    <template v-else>
      <section class="investigation-decision" :class="{ worthy: investigationGateOpen, stopped: !investigationGateOpen }">
        <div class="gain-score"><span>{{ metrics?.informationGain || 0 }}</span><small>{{ tr('信息增益', 'Information gain') }}</small></div>
        <div class="decision-copy"><span class="eyebrow"><Sparkles :size="14" />{{ tr('本地决策门禁', 'Local decision gate') }}</span><h3>{{ investigationDecisionTitle }}</h3><p>{{ stopLabel }}</p></div>
        <div class="decision-delta"><span><b>+{{ metrics?.addedCount || 0 }}</b> {{ tr('新增', 'New') }}</span><span><b>{{ metrics?.apiCount || 0 }}</b> {{ tr('全部 API', 'All APIs') }}</span><span><b>{{ identities.length }}</b> {{ tr('登录身份', 'Signed-in identities') }}</span></div>
      </section>

      <section class="investigation-kpis compact-kpis">
        <article><Network :size="16" /><span><b>{{ metrics?.stateCount }}</b>{{ tr('页面状态', 'Page states') }}</span></article>
        <article><MousePointerClick :size="16" /><span><b>{{ metrics?.actionCount }}</b>{{ tr('自动动作', 'Automated actions') }}</span></article>
        <article><Braces :size="16" /><span><b>{{ metrics?.apiCount }}</b>{{ tr('全部 API', 'All APIs') }}</span></article>
        <article><ShieldCheck :size="16" /><span><b>{{ metrics?.parameterCount }}</b>{{ tr('参数', 'Parameters') }}</span></article>
        <article><Bot :size="16" /><span><b>{{ readyHypotheses.length }}</b>{{ tr('待自动验证', 'Pending auto-validation') }}</span></article>
        <article><GitCompareArrows :size="16" /><span><b>{{ identityPendingDiffs.length }}</b>{{ tr('待复核差异', 'Differences to review') }}</span></article>
      </section>

      <section v-if="manualDeepDive.length" class="manual-deep-dive-section">
        <header>
          <div><strong>{{ tr('自动化未覆盖 · 人工深挖建议', 'Not covered by automation · manual deep-dive suggestions') }}</strong><small>{{ tr('依据当前目标真实接口、动作、身份和缺失证据确定性生成；这些是覆盖缺口，不是漏洞，也不计入风险数量。', 'Generated deterministically from this target’s real endpoints, actions, identities, and missing evidence. These are coverage gaps, not vulnerabilities, and do not count toward risk totals.') }}</small></div>
          <span>{{ tr(`${manualDeepDive.length} 条 · 按投入产出排序`, `${manualDeepDive.length} items · sorted by effort vs. payoff`) }}</span>
        </header>
        <div class="manual-deep-dive-list">
          <article v-for="item in manualDeepDive" :key="`${item.rank}-${item.category}`" :class="`priority-${item.priority}`">
            <div class="manual-rank"><b>{{ item.rank }}</b><small>{{ manualPriorityLabel(item.priority) }}</small></div>
            <div class="manual-lead-content">
              <header><strong>{{ item.title }}</strong><em>{{ tr('未测试 / 需人工', 'Untested / needs manual work') }}</em></header>
              <p>{{ item.reason }}</p>
              <div v-if="item.evidence?.length" class="manual-evidence"><span>{{ tr('当前证据', 'Current evidence') }}</span><code v-for="evidence in item.evidence" :key="evidence">{{ evidence }}</code></div>
              <div class="manual-missing"><b>{{ tr('还缺什么', 'Still missing') }}</b><span>{{ item.missingEvidence }}</span></div>
              <ol><li v-for="step in item.steps || []" :key="step">{{ step }}</li></ol>
              <small class="manual-stop"><CircleStop :size="12" />{{ tr(`停止条件：${item.stopCondition}`, `Stop condition: ${item.stopCondition}`) }}</small>
            </div>
          </article>
        </div>
      </section>

      <section v-if="identities.length" class="identity-matrix-section identity-focus">
        <header><div><strong>{{ identities.length > 1 ? tr('身份与权限差异', 'Identity and access differences') : tr('登录身份', 'Signed-in identities') }}</strong><small>{{ identities.length > 1 ? tr('固定按账号 A / B 对齐：先确认两侧会话与采集状态，再按同一接口比较响应；只读缺口会自动用另一账号重放。', 'Aligned as account A / B: first confirm both sessions and capture status, then compare responses for the same endpoint. Read-only gaps are replayed automatically from the other account.') : tr('当前只绑定一个登录账号；展示该账号的会话、采集和接口证据，但不生成虚假的 A/B 权限差异。', 'Only one signed-in account is bound. Its session, capture, and endpoint evidence are shown, but no artificial A/B access differences are generated.') }}</small></div><span class="identity-count">{{ tr(`${identities.length} 个登录身份`, `${identities.length} signed-in identities`) }}<template v-if="identities.length > 1"> · {{ tr(`${identityPendingDiffs.length} 个待自动复核 · ${identityNormalDiffs.length} 个预期正常`, `${identityPendingDiffs.length} pending auto-review · ${identityNormalDiffs.length} expected normal`) }}</template></span></header>
        <div class="identity-account-grid"><article v-for="entry in identityEvidence" :key="`identity-${entry.item.nodeKey}`" :class="`identity-account ${identitySessionTone(entry)}`"><header><div><b>{{ entry.label }}</b><span>{{ identitySessionLabel(entry) }}</span></div><code>{{ captureStatusLabel(entry.captureStatus) }}</code></header><p>{{ identitySessionDetail(entry) }}</p><div class="identity-kpi-row"><span><b>{{ entry.apiCount }}</b>{{ tr('关联接口', 'Related endpoints') }}</span><span><b>{{ entry.observedCount }}</b>{{ tr('有效响应', 'Valid responses') }}</span><span><b>{{ entry.responseKeys.length }}</b>{{ tr('响应字段', 'Response fields') }}</span></div><div v-if="hasIdentityRuntimeDiagnostics(entry)" class="identity-runtime-diagnostics"><strong>runtimeDiagnostics</strong><dl><div v-for="[label, value] in identityRuntimeDiagnostics(entry).rows" :key="`${entry.item.nodeKey}-${label}`"><dt>{{ label }}</dt><dd><code>{{ value }}</code></dd></div></dl><pre v-if="identityRuntimeDiagnostics(entry).stderr" class="identity-browser-stderr">{{ identityRuntimeDiagnostics(entry).stderr }}</pre></div><small v-if="entry.statuses.length">{{ tr(`已观察状态码：${entry.statuses.join(" / ")}`, `Observed status codes: ${entry.statuses.join(" / ")}`) }}</small><small v-else>{{ tr('尚未自然触发业务响应；系统会对另一侧独有的只读接口进行自动重放。', 'No business responses triggered naturally yet; read-only endpoints seen only on the other side will be replayed automatically.') }}</small></article></div>
        <div class="identity-diff-list">
          <article v-for="diff in identityDiffRows" :key="diff.id" class="identity-compare-row" :class="{ 'not-comparable': !diffIsComparable(diff), 'likely-normal': diffLikelyNormal(diff) }">
            <header><div class="diff-endpoint"><span class="api-method" :class="`method-${diffEndpoint(diff).method.toLowerCase()}`">{{ diffEndpoint(diff).method }}</span><div><strong>{{ diffEndpoint(diff).path }}</strong><small>{{ diffEndpoint(diff).host || tr('当前目标', 'Current target') }} · {{ diffLabel(diff) }}</small></div></div><span class="diff-risk">{{ !diffIsComparable(diff) ? tr('证据不完整', 'Incomplete evidence') : diffLikelyNormal(diff) ? tr('AI 初判：预期正常', 'AI initial read: expected normal') : tr(`待自动复核 · ${diff.riskScore}`, `Pending auto-review · ${diff.riskScore}`) }}</span></header>
            <p class="diff-conclusion"><ShieldAlert :size="14" />{{ diffConclusion(diff) }}</p>
            <div class="ab-difference-table">
              <div class="ab-difference-head"><span>{{ tr('对比项', 'Item') }}</span><b>{{ identityDisplay(diff.leftIdentityKey, 0) }}</b><b>{{ identityDisplay(diff.rightIdentityKey, 1) }}</b></div>
              <div v-for="row in comparisonRows(diff)" :key="`${diff.id}-${row.label}`" :class="{ changed: row.changed }"><span>{{ row.label }}</span><code>{{ row.left }}</code><code>{{ row.right }}</code></div>
            </div>
            <div v-if="responseFieldDiff(diff).leftOnly.length || responseFieldDiff(diff).rightOnly.length" class="field-difference"><span v-if="responseFieldDiff(diff).leftOnly.length">{{ tr(`仅 A：${responseFieldDiff(diff).leftOnly.slice(0, 10).join("、")}`, `A only: ${responseFieldDiff(diff).leftOnly.slice(0, 10).join(", ")}`) }}</span><span v-if="responseFieldDiff(diff).rightOnly.length">{{ tr(`仅 B：${responseFieldDiff(diff).rightOnly.slice(0, 10).join("、")}`, `B only: ${responseFieldDiff(diff).rightOnly.slice(0, 10).join(", ")}`) }}</span><span v-if="responseFieldDiff(diff).common.length">{{ tr(`共同字段 ${responseFieldDiff(diff).common.length} 个`, `${responseFieldDiff(diff).common.length} shared fields`) }}</span></div>
            <details class="ab-http-evidence">
              <summary>{{ tr('查看两侧完整 HTTP 响应', 'View full HTTP responses for both sides') }}</summary>
              <div class="ab-http-grid">
                <section><header><b>{{ identityDisplay(diff.leftIdentityKey, 0) }}</b><span>Pretty</span></header><pre>{{ sidePrettyBody(diff, 'left') }}</pre><details><summary>Raw HTTP</summary><pre>{{ sideHttpResponse(diff, 'left') }}</pre></details></section>
                <section><header><b>{{ identityDisplay(diff.rightIdentityKey, 1) }}</b><span>Pretty</span></header><pre>{{ sidePrettyBody(diff, 'right') }}</pre><details><summary>Raw HTTP</summary><pre>{{ sideHttpResponse(diff, 'right') }}</pre></details></section>
              </div>
            </details>
          </article>
          <p v-if="!identityDiffRows.length" class="identity-no-diff"><CheckCircle2 :size="15" />{{ identities.length > 1 ? tr('A/B 同接口比较没有发现需要进一步解释的差异。', 'Comparing the same endpoints across A/B found no differences that need further explanation.') : tr('当前只有一个登录账号，未执行跨账号权限比较。', 'Only one signed-in account; no cross-account access comparison was run.') }}</p>
        </div>
      </section>

      <section class="api-explorer-section">
        <header><div><strong>{{ tr('调查 API 接口', 'Investigation API endpoints') }}</strong><small>{{ tr('区分 CDP 真实观察与源码映射准确调用；后者只有 GET/HEAD、无动态占位符且具备调用位置时才可进入有界验证。监控、遥测和页面初始化请求保留在“关联服务”中。', 'Separates real CDP observations from exact calls recovered from source maps. Source-mapped calls enter bounded validation only when they are GET/HEAD, have no dynamic placeholders, and have a known call site. Monitoring, telemetry, and page bootstrap requests stay under “Related services”.') }}</small></div><div class="api-total"><b>{{ apis.length }}</b> {{ tr('接口契约', 'endpoint contracts') }} · <b>{{ businessApis.length }}</b> {{ tr('业务', 'business') }}</div></header>
        <div class="api-toolbar"><input v-model="apiQuery" :placeholder="tr('搜索方法、路径、身份或参数…', 'Search method, path, identity, or parameter…')" /></div>
        <div class="api-list"><article v-for="api in filteredApis" :key="api.id" class="api-row" :class="{ selected: selectedApi?.apiKey === api.apiKey, expanded: apiExpanded(api) }"><div class="api-row-head"><button class="api-row-toggle" type="button" @click="toggleApiDetails(api)"><span class="api-method" :class="`method-${api.method.toLowerCase()}`">{{ api.method }}</span><span class="api-path"><strong>{{ api.normalizedPath || api.url }}</strong><code :title="api.url">{{ api.url }}</code><small>{{ telemetry(api) ? tr('遥测/埋点：同类请求已归并', 'Telemetry/tracking: similar requests merged') : tr('业务接口', 'Business endpoint') }} · {{ api.confidence }} · {{ apiEvidenceKind(api) }}</small></span><span class="api-contract"><span>{{ tr(`请求：${requestSummary(api)}`, `Request: ${requestSummary(api)}`) }}</span><span>{{ tr(`响应：${responseSummary(api)}`, `Response: ${responseSummary(api)}`) }}</span><span>{{ tr(`身份：${api.identityKeys.map((key, i) => identityDisplay(key, i)).join(" / ") || "未标注"}`, `Identity: ${api.identityKeys.map((key, i) => identityDisplay(key, i)).join(" / ") || "unlabeled"}`) }}</span></span><ChevronDown :size="15" class="api-row-chevron" /></button><button class="replay-button" type="button" @click.stop="emit('replay', api)"><Send :size="14" />{{ tr('重放', 'Replay') }}</button></div><div v-if="apiExpanded(api)" class="api-inline-detail"><div class="api-detail-toolbar"><span>{{ tr('接口细节', 'Endpoint details') }} · {{ apiEvidenceKind(api) }} · {{ api.baselineStatus || tr('未建立基线', 'No baseline') }}</span><button class="button primary compact" type="button" @click.stop="emit('replay', api)"><Send :size="13" />{{ tr('打开请求重放', 'Open request replay') }}</button></div><div class="api-detail-grid contract-grid"><article><strong>{{ tr('规范化契约', 'Normalized contract') }}</strong><pre>{{ apiJson({ method: api.method, normalizedPath: api.normalizedPath, parameters: api.parameters, requestSchema: api.requestSchema }) }}</pre></article><article><strong>{{ tr('响应契约', 'Response contract') }}</strong><pre>{{ apiJson({ status: api.payload?.statusCode || api.payload?.status, responseSchema: api.responseSchema, responseBody: apiObservedResponse(api) }) }}</pre></article><article><strong>{{ tr('请求头 / 响应头', 'Request / response headers') }}</strong><pre>{{ apiJson({ requestHeaders: apiHeaders(api), responseHeaders: apiResponseHeaders(api) }) }}</pre></article><article><strong>{{ tr('身份 / 来源证据', 'Identity / source evidence') }}</strong><pre>{{ apiJson({ identities: api.identityKeys.map((key, i) => identityDisplay(key, i)), source: apiSourceLabel(api), stateKeys: api.stateKeys, actionKeys: api.actionKeys, payload: api.payload }) }}</pre></article></div></div></article><div v-if="!filteredApis.length" class="empty-inline">{{ tr('没有匹配接口', 'No matching endpoints') }}</div></div>
      </section>

      <section v-if="relatedServices.length" class="related-services-section">
        <header><div><strong>{{ tr('关联服务与降级请求', 'Related services and downgraded requests') }}</strong><small>{{ tr('来自 CDP 的真实网络观察，但不参与正式业务 API、身份权限差异或 AI 验证队列。这里保留域名、路径、身份和触发次数供人工追踪。', 'Real network observations from CDP that are kept out of business APIs, identity access differences, and the AI validation queue. Domains, paths, identities, and trigger counts are kept here for manual follow-up.') }}</small></div><div class="api-total"><b>{{ relatedServices.length }}</b> {{ tr('个服务域名/类型', 'service domains/types') }}</div></header>
        <div class="related-service-list">
          <article v-for="service in visibleRelatedServices" :key="`${service.host}-${service.classification}`">
            <div class="related-service-head"><Network :size="16" /><span><b>{{ service.host }}</b><small>{{ relatedServiceLabel(service.classification) }} · {{ service.relation === 'same_party' ? tr('同站关联域名', 'Same-site related domain') : tr('第三方服务', 'Third-party service') }} · {{ tr(`CDP 观察 ${service.requestCount} 次`, `Seen ${service.requestCount}× via CDP`) }}</small></span><em>{{ service.methods.join(' / ') }}</em></div>
            <div class="related-service-evidence"><span>{{ tr(`身份：${relatedIdentityLabels(service.identityKeys)}`, `Identity: ${relatedIdentityLabels(service.identityKeys)}`) }}</span><span>{{ tr(`传输：${service.resourceTypes.join(' / ') || '未知'}`, `Transport: ${service.resourceTypes.join(' / ') || 'unknown'}`) }}</span><span v-if="service.statuses.length">{{ tr(`状态：${service.statuses.join(' / ')}`, `Status: ${service.statuses.join(' / ')}`) }}</span></div>
            <details><summary>{{ tr('查看已观察路径与来源证据', 'View observed paths and source evidence') }}</summary><div class="related-service-paths"><code v-for="path in service.paths" :key="path">{{ path }}</code></div><p v-if="service.queryKeys.length">{{ tr(`查询字段：${service.queryKeys.join('、')}`, `Query fields: ${service.queryKeys.join(', ')}`) }}</p><p>{{ tr(`来源：${service.evidenceSource} · ${service.sources.join(' / ')}`, `Source: ${service.evidenceSource} · ${service.sources.join(' / ')}`) }}</p></details>
          </article>
        </div>
        <button v-if="relatedServices.length > 6" class="button ghost compact related-service-more" type="button" @click="showAllRelatedServices = !showAllRelatedServices">{{ showAllRelatedServices ? tr('收起关联服务', 'Collapse related services') : tr(`展开全部 ${relatedServices.length} 项`, `Show all ${relatedServices.length}`) }}</button>
      </section>


      <section class="hypothesis-contracts"><header><div><strong>{{ tr('AI 自动验证队列', 'AI auto-validation queue') }}</strong><small>{{ tr('AI 直接完成证据检查、同请求对照和成立/不成立判断。每条契约按准确端点、方法和最大尝试次数自动执行；无害状态变更自动清理，不再等待人工逐条授权。', 'AI checks the evidence, compares the same request, and decides whether it holds. Each contract runs automatically within its exact endpoint, method, and maximum attempts; harmless state changes are cleaned up automatically, with no per-item manual approval.') }}</small></div><div class="contract-toolbar"><select v-model="contractFilter"><option value="ready">{{ tr(`AI 待验证 ${contractCounts.ready}`, `AI pending ${contractCounts.ready}`) }}</option><option value="actionable">{{ tr(`全部验证分支 ${contractCounts.actionable}`, `All validation branches ${contractCounts.actionable}`) }}</option><option value="completed">{{ tr(`AI 已完成 ${contractCounts.completed}`, `AI completed ${contractCounts.completed}`) }}</option></select><button class="button ghost compact" type="button" @click="showAll = !showAll">{{ showAll ? tr('收起', 'Collapse') : tr(`展开全部 ${filteredHypotheses.length} 条`, `Show all ${filteredHypotheses.length}`) }}</button></div></header><div class="contract-grid human-contract-grid"><article v-for="item in visibleHypotheses" :key="item.id" :class="item.status"><div class="contract-title"><span class="contract-score">{{ item.score }}</span><div><strong>{{ item.title }}</strong><small>{{ contractStateLabel(item) }}<template v-if="Number(item.decision?.mergedCount || 1) > 1"> · {{ tr(`已合并 ${item.decision.mergedCount} 条重复证据`, `${item.decision.mergedCount} duplicate evidence items merged`) }}</template></small></div></div><p class="contract-purpose">{{ contractPurpose(item) }}</p><div class="contract-endpoint"><span class="api-method" :class="`method-${String(item.contract?.method || 'GET').toLowerCase()}`">{{ item.contract?.method || "GET" }}</span><code>{{ item.contract?.endpoint || tr('待解析具体接口', 'Endpoint not resolved yet') }}</code></div><div class="human-evidence-list"><span v-for="value in evidenceItems(item)" :key="value"><CheckCircle2 :size="12" />{{ value }}</span></div><div v-if="hypothesisIdentityRows(item).length" class="contract-identity-scope"><em v-for="row in hypothesisIdentityRows(item)" :key="`${item.id}-${row.label}`" :class="`identity-chip ${row.tone}`"><b>{{ row.label }} · {{ row.state }}</b><small>{{ row.detail }}</small></em></div><div class="contract-decision-line"><Bot :size="14" /><span>{{ tr('无需人工操作，AI 会按契约边界自动执行并在固定尝试次数内直接给出结论', 'No manual action needed. AI runs within the contract bounds and reaches a conclusion within a fixed number of attempts') }}</span></div><div class="contract-actions"><span v-if="['ready', 'in_progress', 'awaiting_authorization', 'blocked_by_authorization'].includes(item.status)" class="ai-auto-status"><Bot :size="13" />{{ item.status === 'in_progress' ? tr('正在自动验证', 'Auto-validating') : tr('已进入自动验证队列', 'Queued for auto-validation') }}</span><span v-if="['validated','confirmed'].includes(item.status)"><CheckCircle2 :size="14" />{{ tr('已取得可复核证据', 'Reviewable evidence obtained') }}</span><span v-else-if="['exhausted','rejected','normal'].includes(item.status)"><CircleStop :size="14" />{{ tr('AI 已结束该分支', 'AI closed this branch') }}</span></div><details class="technical-evidence"><summary>{{ tr('技术证据（调试）', 'Technical evidence (debug)') }}</summary><div class="technical-contract"><ul><li v-for="value in contractItems(item, 'requiredEvidence')" :key="value">{{ value }}</li></ul><pre>{{ JSON.stringify({ contract: item.contract, evidence: item.evidence, decision: item.decision }, null, 2) }}</pre></div></details></article></div></section>
    </template>
  </div>
</template>

<style scoped>
.compact-investigation{display:grid;gap:12px;min-width:0}.compact-kpis{grid-template-columns:repeat(6,minmax(0,1fr))}.identity-focus,.api-explorer-section,.related-services-section,.manual-deep-dive-section{border:1px solid var(--border);background:var(--panel);border-radius:var(--radius-xl);padding:16px}.identity-account-strip{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;margin-bottom:12px}.identity-account-strip article{display:grid;gap:4px;padding:12px;border:1px solid var(--border);border-radius:var(--radius-lg);background:var(--surface);min-width:0}.identity-account-strip b{color:var(--accent);font-size:var(--text-strong)}.identity-account-strip code,.api-path code{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.identity-account-strip small,.api-row small,.api-contract span{color:var(--muted);font-size:var(--text-micro)}.identity-diff-grid{display:grid;gap:8px}.identity-diff-card{display:flex;gap:10px;padding:10px;border:1px solid color-mix(in srgb,var(--warning) 40%,var(--border));border-radius:var(--radius-lg);background:var(--surface);min-width:0}.diff-main{display:grid;gap:4px;min-width:0}.diff-values{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:6px}.diff-values span{display:grid;gap:2px;padding:6px;border-radius:var(--radius-md);background:var(--panel)}.diff-values em{color:var(--muted);font-size:var(--text-micro);font-style:normal}.api-explorer-section>header,.identity-focus>header,.hypothesis-contracts>header,.related-services-section>header,.manual-deep-dive-section>header{display:flex;justify-content:space-between;gap:12px;align-items:center;margin-bottom:12px}.api-total{color:var(--muted);font-size:var(--text-meta)}.api-total b{color:var(--text)}.api-toolbar{display:flex;gap:8px;margin-bottom:8px}.api-toolbar input{flex:1;min-width:0}.api-list{display:grid;gap:6px;max-height:720px;overflow:auto}.api-row{display:grid;grid-template-columns:68px minmax(240px,1.3fr) minmax(180px,1fr) auto;gap:10px;align-items:center;padding:10px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--surface);cursor:pointer;min-width:0}.api-row:hover,.api-row.selected{border-color:var(--accent)}.api-method{font-weight:800;font-size:var(--text-meta);text-align:center}.api-path,.api-contract{display:grid;gap:3px;min-width:0}.api-path strong,.api-path code{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.api-contract span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.replay-button{display:inline-flex;align-items:center;gap:6px;border:1px solid var(--border);border-radius:var(--radius-md);padding:6px 8px;color:var(--accent);background:transparent;white-space:nowrap}.api-detail-card{margin-top:10px;padding:12px;border:1px solid color-mix(in srgb,var(--accent) 40%,var(--border));border-radius:var(--radius-lg);background:var(--surface)}.api-detail-card header{display:flex;justify-content:space-between;gap:10px;align-items:center}.api-detail-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}.api-detail-grid pre{max-height:260px;overflow:auto;padding:10px;border-radius:var(--radius-md);background:#091019;color:#b9c8da;font-size:var(--text-micro);white-space:pre-wrap;word-break:break-word}.contract-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.contract-grid>article{min-width:0}.contract-grid pre{max-height:220px;overflow:auto;white-space:pre-wrap;word-break:break-word;font-size:var(--text-micro)}.contract-meta{display:flex;gap:6px;flex-wrap:wrap}.contract-meta span{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;border:1px solid var(--border);border-radius:var(--radius-pill);padding:4px 8px;color:var(--muted);font-size:var(--text-micro)}.mutation-approval{display:flex;justify-content:space-between;gap:8px;align-items:center}.related-service-list{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}.related-service-list article{min-width:0;padding:12px;border:1px solid color-mix(in srgb, var(--blue) 24%, var(--ui-line));border-radius:var(--radius-lg);background:var(--ui-surface)}.related-service-head{display:grid;grid-template-columns:auto minmax(0,1fr) auto;gap:8px;align-items:center}.related-service-head span{min-width:0}.related-service-head b,.related-service-head small{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.related-service-head b{font-size:var(--text-body)}.related-service-head small{margin-top:2px;color:var(--muted);font-size:var(--text-micro)}.related-service-head em{padding:4px 6px;border-radius:var(--radius-pill);background:color-mix(in srgb, var(--blue) 12%, var(--ui-surface));color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font-size:var(--text-micro);font-style:normal}.related-service-evidence{display:flex;gap:8px;flex-wrap:wrap;margin-top:8px;color:var(--ui-muted);font-size:var(--text-micro)}.related-service-list details{margin-top:8px;border-top:1px solid var(--ui-line);padding-top:8px;color:var(--ui-muted);font-size:var(--text-micro)}.related-service-list summary{cursor:pointer;color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink))}.related-service-paths{display:grid;gap:3px;margin-top:6px}.related-service-paths code{overflow-wrap:anywhere;color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink))}.related-service-list p{margin:6px 0 0}.related-service-more{margin-top:10px}@media(max-width:1100px){.compact-kpis{grid-template-columns:repeat(3,1fr)}.api-row{grid-template-columns:60px minmax(0,1fr) auto}.api-contract{grid-column:2}.replay-button{grid-row:1/3;grid-column:3}.contract-grid,.api-detail-grid{grid-template-columns:1fr}}@media(max-width:700px){.identity-account-strip,.identity-symmetric-grid,.diff-values,.related-service-list{grid-template-columns:1fr}.compact-kpis{grid-template-columns:repeat(2,1fr)}.api-toolbar{flex-direction:column}.api-row{grid-template-columns:58px minmax(0,1fr)}.replay-button{grid-column:2;grid-row:auto;justify-self:start}}
.manual-deep-dive-section>header>div{min-width:0}.manual-deep-dive-section>header strong{display:block;color:var(--ui-ink);font-size:var(--text-strong)}.manual-deep-dive-section>header small{display:block;margin-top:4px;color:var(--ui-muted);font-size:var(--text-micro);line-height:1.5}.manual-deep-dive-section>header>span{flex:0 0 auto;color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink));font:700 var(--text-micro) var(--font-mono)}.manual-deep-dive-list{display:grid;gap:8px}.manual-deep-dive-list>article{display:grid;grid-template-columns:70px minmax(0,1fr);gap:12px;min-width:0;padding:12px;border:1px solid color-mix(in srgb, var(--amber) 36%, var(--ui-line));border-radius:var(--radius-lg);background:var(--ui-surface)}.manual-deep-dive-list>article.priority-critical{border-color:color-mix(in srgb, var(--red) 39%, var(--ui-line));background:var(--ui-surface)}.manual-deep-dive-list>article.priority-high{border-color:color-mix(in srgb, var(--amber) 47%, var(--ui-line))}.manual-rank{display:grid;align-content:start;justify-items:center;gap:6px;padding:8px;border-radius:var(--radius-md);background:color-mix(in srgb, var(--amber) 15%, var(--ui-surface));color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}.priority-critical .manual-rank{background:color-mix(in srgb, var(--red) 11%, var(--ui-surface));color:color-mix(in srgb, var(--red) 82%, var(--ui-ink))}.manual-rank b{font:800 var(--text-display)/1 var(--font-mono)}.manual-rank small{font-size:var(--text-micro);white-space:nowrap}.manual-lead-content{display:grid;gap:8px;min-width:0}.manual-lead-content>header{display:flex;justify-content:space-between;gap:8px;align-items:center}.manual-lead-content>header strong{color:var(--ui-ink);font-size:var(--text-body)}.manual-lead-content>header em{flex:0 0 auto;padding:3px 6px;border-radius:var(--radius-pill);background:color-mix(in srgb, var(--amber) 17%, var(--ui-surface));color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink));font-size:var(--text-micro);font-style:normal}.manual-lead-content>p{margin:0;color:var(--ui-muted);font-size:var(--text-micro);line-height:1.55}.manual-evidence{display:flex;align-items:center;gap:6px;flex-wrap:wrap}.manual-evidence>span{color:var(--ui-muted);font-size:var(--text-micro)}.manual-evidence code{max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;padding:4px 6px;border-radius:var(--radius-sm);background:var(--ui-surface-raised);color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font-size:var(--text-micro)}.manual-missing{display:grid;grid-template-columns:auto minmax(0,1fr);gap:8px;padding:8px 8px;border-radius:var(--radius-md);background:var(--ui-surface-raised);font-size:var(--text-micro)}.manual-missing b{color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}.manual-missing span{color:var(--ui-muted)}.manual-lead-content ol{display:grid;gap:4px;margin:0;padding-left:20px;color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font-size:var(--text-micro);line-height:1.45}.manual-stop{display:flex;align-items:flex-start;gap:6px;color:var(--ui-muted);font-size:var(--text-micro);line-height:1.45}.manual-stop svg{flex:0 0 auto;margin-top:1px}@media(max-width:700px){.manual-deep-dive-list>article{grid-template-columns:1fr}.manual-rank{grid-template-columns:auto auto;justify-content:start;align-items:center}.manual-lead-content>header{align-items:flex-start;flex-direction:column}}
.repeater-panel{border:1px solid color-mix(in srgb,var(--accent) 45%,var(--border));border-radius:var(--radius-xl);background:var(--panel);padding:16px}.repeater-panel>header{display:flex;justify-content:space-between;gap:12px;align-items:flex-start}.repeater-panel h3{margin:3px 0;font-size:var(--text-heading)}.repeater-panel small{color:var(--muted)}.repeater-grid{display:grid;grid-template-columns:140px minmax(0,1fr);gap:10px;margin-top:12px}.repeater-grid label{display:grid;gap:6px;color:var(--muted);font-size:var(--text-meta)}.repeater-grid .repeater-url{grid-column:2}.repeater-wide{grid-column:1/-1}.repeater-grid textarea,.repeater-grid input,.repeater-grid select{width:100%;box-sizing:border-box;font:var(--text-meta)/1.5 var(--font-mono)}.repeater-actions{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-top:10px}.repeater-actions label{color:var(--muted);font-size:var(--text-meta)}.replay-error{color:#ff9e9e;font-size:var(--text-meta)}.replay-result{margin-top:12px;border-top:1px solid var(--border);padding-top:12px}.replay-result header{display:flex;justify-content:space-between;color:var(--success)}

/* Repeater and investigation layout: keep every data block visible and readable. */
.compact-investigation { --panel:var(--ui-surface); --surface:var(--ui-surface); --border:var(--ui-line); --accent:var(--ui-accent); --text:var(--ui-ink); --muted:var(--ui-muted); --success:var(--green); --warning:var(--amber); color:var(--text); }
.investigation-decision { display:grid; grid-template-columns:110px minmax(0,1fr) auto; gap:16px; align-items:center; padding:20px; border:1px solid color-mix(in srgb, var(--blue) 23%, var(--ui-line)); border-radius:var(--radius-xl); background:linear-gradient(135deg,var(--ui-surface-raised),var(--ui-surface)); }
.investigation-decision.worthy { border-color:color-mix(in srgb, var(--green) 31%, var(--ui-line)); background:linear-gradient(135deg,var(--ui-surface-raised),var(--ui-surface)); }
.investigation-decision.stopped { border-color:color-mix(in srgb, var(--amber) 42%, var(--ui-line)); background:linear-gradient(135deg,color-mix(in srgb, var(--amber) 6%, var(--ui-surface)),var(--ui-surface)); }
.gain-score { display:grid; place-items:center; align-content:center; min-height:76px; border-radius:var(--radius-lg); background:color-mix(in srgb, var(--blue) 8%, var(--ui-surface)); color:#286dd8; }
.gain-score span { font:800 30px/1 var(--font-mono); }
.gain-score small,.decision-copy p,.decision-copy small { color:var(--ui-muted); font-size:var(--text-meta); }
.decision-copy { min-width:0; }
.decision-copy .eyebrow { display:flex; align-items:center; gap:6px; }
.decision-copy h3 { margin:6px 0 4px; font-size:var(--text-heading); line-height:1.35; }
.decision-copy p { margin:0; line-height:1.5; }
.decision-delta { display:flex; gap:8px; flex-wrap:wrap; justify-content:flex-end; }
.decision-delta span { display:grid; gap:2px; min-width:70px; padding:8px 10px; border:1px solid var(--ui-line); border-radius:var(--radius-md); background:var(--ui-surface); color:var(--ui-muted); font-size:var(--text-micro); text-align:center; }
.decision-delta b { color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font:700 var(--text-heading) var(--font-mono); }
.investigation-kpis { display:grid; gap:8px; }
.investigation-kpis article { display:flex; align-items:center; gap:8px; min-width:0; padding:12px; border:1px solid var(--ui-line); border-radius:var(--radius-lg); background:var(--ui-surface); color:var(--ui-muted); }
.investigation-kpis article svg { flex:0 0 auto; color:#4d83dc; }
.investigation-kpis article span { display:grid; gap:2px; min-width:0; font-size:var(--text-micro); }
.investigation-kpis article b { color:var(--ui-ink); font:700 var(--text-heading) var(--font-mono); }
.identity-matrix-section { color:var(--ui-ink); }
.identity-matrix-section header small,.api-explorer-section header small,.hypothesis-contracts header small { display:block; margin-top:4px; color:var(--ui-muted); font-size:var(--text-micro); line-height:1.5; }
.identity-symmetric-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px;margin-top:10px}.identity-symmetric-grid article{min-width:0;padding:12px;border:1px solid var(--ui-line);border-radius:var(--radius-lg);background:var(--ui-surface)}.identity-symmetric-grid header{display:flex;justify-content:space-between;gap:8px;align-items:center}.identity-symmetric-grid header span{padding:3px 6px;border-radius:var(--radius-pill);background:var(--ui-surface-raised);color:var(--ui-muted);font-size:var(--text-micro)}.identity-symmetric-grid header .capture-complete{background:color-mix(in srgb, var(--green) 9%, var(--ui-surface));color:color-mix(in srgb, var(--green) 82%, var(--ui-ink))}.identity-symmetric-grid header .capture-partial,.identity-symmetric-grid header .capture-failed{background:color-mix(in srgb, var(--amber) 12%, var(--ui-surface));color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink))}.identity-symmetric-grid>article>div{display:grid;grid-template-columns:repeat(3,auto);justify-content:start;gap:4px 12px;margin-top:10px}.identity-symmetric-grid>article>div b{font:700 var(--text-heading) var(--font-mono);color:var(--ui-ink)}.identity-symmetric-grid>article>div small{color:var(--ui-muted);font-size:var(--text-micro)}.identity-symmetric-grid p{margin:10px 0 0;overflow-wrap:anywhere;color:var(--ui-muted);font:var(--text-micro)/1.45 var(--font-mono)}.identity-symmetric-grid article>small{display:block;margin-top:8px;color:var(--ui-muted);font-size:var(--text-micro)}.identity-count { flex:0 0 auto; color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font:700 var(--text-micro) var(--font-mono); }
.diff-score { flex:0 0 34px; display:grid; place-items:center; width:34px; height:34px; border-radius:var(--radius-md); background:color-mix(in srgb, var(--amber) 15%, var(--ui-surface)); color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); font:700 var(--text-body) var(--font-mono); }
.identity-no-diff { display:flex; align-items:center; gap:6px; color:color-mix(in srgb, var(--green) 82%, var(--ui-ink)); font-size:var(--text-meta); }
.api-explorer-section,.hypothesis-contracts { min-width:0; }
@media (max-width:760px) { .investigation-decision { grid-template-columns:1fr; }.decision-delta { justify-content:flex-start; }.repeater-request-line,.repeater-edit-grid,.replay-response-grid { grid-template-columns:1fr; }.repeater-header { flex-direction:column; }.repeater-close { align-self:flex-start; } }
.diff-values span { min-width:0; overflow:hidden; }
.diff-values span b,.diff-values span em { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.diff-details { margin-top:2px; }
.diff-details summary { color:#4b78b6; cursor:pointer; font-size:var(--text-micro); }
.diff-details pre { max-height:180px; overflow:auto; margin:6px 0 0; padding:8px; border-radius:var(--radius-sm); background:#101a29; color:#dceafe; font:var(--text-micro)/1.45 var(--font-mono); white-space:pre-wrap; overflow-wrap:anywhere; }
.identity-diff-card { min-width:0; overflow:hidden; }


/* Each API contract owns its disclosure state; no second, detached detail pane. */
.api-row { display:block; padding:0; cursor:default; overflow:hidden; }
.api-row-head { display:grid; grid-template-columns:minmax(0,1fr) auto; align-items:stretch; min-width:0; }
.api-row-toggle { display:grid; grid-template-columns:68px minmax(180px,1.15fr) minmax(180px,.95fr) 18px; align-items:center; gap:10px; min-width:0; padding:10px; border:0; background:transparent; color:inherit; text-align:left; cursor:pointer; }
.api-row-toggle:hover { background:var(--ui-surface-raised); }
.api-row-toggle > * { min-width:0; }
.api-row-toggle .api-path strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.api-row-toggle .api-path code { display:block; margin-top:3px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.api-row-toggle .api-contract span { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.api-row-chevron { color:var(--ui-muted); transition:transform .18s ease; }
.api-row.expanded .api-row-chevron { transform:rotate(180deg); }
.api-row-head > .replay-button { align-self:center; margin-right:10px; }
.api-inline-detail { padding:0 12px 12px; border-top:1px solid var(--ui-line); background:var(--ui-surface); }
.api-detail-toolbar { display:flex; justify-content:space-between; align-items:center; gap:10px; padding:10px 0 4px; color:var(--ui-muted); font-size:var(--text-micro); }
.api-detail-grid article { min-width:0; }
.api-detail-grid article > strong { display:block; margin-bottom:6px; color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); }
.api-detail-grid pre { max-height:230px; margin:0; overflow:auto; white-space:pre-wrap; overflow-wrap:anywhere; word-break:break-word; }
.api-toolbar { align-items:center; padding:6px; border:1px solid var(--ui-line); border-radius:var(--radius-md); background:var(--ui-surface-raised); }
.api-toolbar input,.api-toolbar select,.contract-toolbar select { box-sizing:border-box; height:34px; border:1px solid color-mix(in srgb, var(--blue) 27%, var(--ui-line)); border-radius:var(--radius-md); background:var(--ui-surface); color:var(--ui-ink); font-size:var(--text-micro); }
.api-toolbar input { padding:0 10px; }
.api-toolbar select { flex:0 0 190px; padding:0 8px; }
.contract-toolbar { display:flex; align-items:center; gap:6px; flex:0 0 auto; }
.hypothesis-contracts>header { padding:10px 12px; border:1px solid var(--ui-line); border-radius:var(--radius-lg); background:var(--ui-surface-raised); }
@media (max-width:900px) { .api-row-toggle { grid-template-columns:58px minmax(0,1fr) 18px; }.api-row-toggle .api-contract { grid-column:2/4; }.api-row-head > .replay-button { grid-column:2; justify-self:start; margin:0 0 10px 10px; } }
@media (max-width:650px) { .api-row-toggle { grid-template-columns:54px minmax(0,1fr) 18px; gap:8px; }.api-row-toggle .api-contract span { white-space:normal; }.api-detail-toolbar { align-items:flex-start; flex-direction:column; } }

/* Human-facing A/B comparison: one account per column and one API per row. */
.identity-account-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:10px; margin-bottom:14px; }
.identity-account { min-width:0; padding:14px; border:1px solid var(--ui-line); border-radius:var(--radius-lg); background:var(--ui-surface); }
.identity-account.ok { border-color:color-mix(in srgb, var(--green) 28%, var(--ui-line)); background:var(--ui-surface-raised); }
.identity-account.warn { border-color:color-mix(in srgb, var(--amber) 45%, var(--ui-line)); background:var(--ui-surface); }
.identity-account.bad { border-color:color-mix(in srgb, var(--red) 39%, var(--ui-line)); background:var(--ui-surface); }
.identity-account>header { display:flex; justify-content:space-between; align-items:flex-start; gap:10px; margin:0; }
.identity-account>header div { display:grid; gap:3px; }
.identity-account>header b { color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-heading); }
.identity-account>header span { color:var(--ui-ink); font-weight:700; font-size:var(--text-meta); }
.identity-account>header code { padding:3px 8px; border-radius:var(--radius-pill); background:var(--ui-surface-sunken); color:var(--ui-muted); font-size:var(--text-micro); }
.identity-account>p { margin:10px 0; color:var(--ui-muted); font-size:var(--text-micro); line-height:1.5; }
.identity-account>small { display:block; margin-top:8px; color:var(--ui-muted); font-size:var(--text-micro); }
.identity-runtime-diagnostics { margin-top:10px; padding:10px 10px; border:1px dashed color-mix(in srgb, var(--blue) 29%, var(--ui-line)); border-radius:var(--radius-md); background:var(--ui-surface-raised); }
.identity-runtime-diagnostics>strong { display:block; margin-bottom:6px; color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); letter-spacing:0.02em; }
.identity-runtime-diagnostics dl { display:grid; gap:4px; margin:0; }
.identity-runtime-diagnostics dl>div { display:grid; grid-template-columns:92px minmax(0,1fr); gap:8px; align-items:start; }
.identity-runtime-diagnostics dt { margin:0; color:var(--ui-muted); font-size:var(--text-micro); }
.identity-runtime-diagnostics dd { margin:0; min-width:0; }
.identity-runtime-diagnostics code { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--ui-ink); font-size:var(--text-micro); }
.identity-browser-stderr { margin:8px 0 0; max-height:96px; overflow:auto; padding:8px; border-radius:var(--radius-md); background:#091019; color:#b9c8da; font-size:var(--text-micro); white-space:pre-wrap; word-break:break-word; }
.identity-kpi-row { display:grid; grid-template-columns:repeat(3,1fr); gap:6px; }
.identity-kpi-row span { display:grid; gap:2px; padding:8px; border-radius:var(--radius-md); background:var(--ui-surface-raised); color:var(--ui-muted); font-size:var(--text-micro); }
.identity-kpi-row b { color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font:800 var(--text-heading)/1 var(--font-mono); }
.identity-diff-list { display:grid; gap:10px; }
.identity-compare-row { min-width:0; padding:14px; border:1px solid var(--ui-line); border-radius:var(--radius-lg); background:var(--ui-surface); overflow:hidden; }
.identity-compare-row.not-comparable { border-color:color-mix(in srgb, var(--amber) 41%, var(--ui-line)); background:var(--ui-surface); }
.identity-compare-row.likely-normal { border-color:color-mix(in srgb, var(--green) 24%, var(--ui-line)); background:var(--ui-surface-raised); }
.identity-compare-row.likely-normal .diff-risk { background:color-mix(in srgb, var(--green) 11%, var(--ui-surface)); color:color-mix(in srgb, var(--green) 82%, var(--ui-ink)); }
.identity-compare-row>header { display:flex; justify-content:space-between; align-items:center; gap:12px; margin:0; }
.diff-endpoint { display:flex; align-items:center; gap:10px; min-width:0; }
.diff-endpoint>div { display:grid; gap:3px; min-width:0; }
.diff-endpoint strong { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--ui-ink); font-size:var(--text-body); }
.diff-endpoint small { color:var(--ui-muted); font-size:var(--text-micro); }
.diff-risk { flex:0 0 auto; padding:4px 8px; border-radius:var(--radius-pill); background:color-mix(in srgb, var(--amber) 17%, var(--ui-surface)); color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); font:700 var(--text-micro) var(--font-mono); }
.diff-conclusion { display:flex; align-items:center; gap:6px; margin:10px 0; padding:8px 10px; border-radius:var(--radius-md); background:var(--ui-surface-raised); color:var(--ui-muted); font-size:var(--text-micro); line-height:1.5; }
.identity-compare-columns { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:8px; }
.identity-compare-columns section { display:grid; grid-template-columns:auto 1fr auto; gap:6px 8px; align-items:center; min-width:0; padding:10px; border:1px solid var(--ui-line); border-radius:var(--radius-md); background:var(--ui-surface); }
.identity-compare-columns section>b { color:#286fda; font-size:var(--text-meta); }
.identity-compare-columns section>span { justify-self:start; padding:3px 6px; border-radius:var(--radius-pill); font-size:var(--text-micro); }
.identity-compare-columns section>span.observed { background:color-mix(in srgb, var(--green) 10%, var(--ui-surface)); color:color-mix(in srgb, var(--green) 82%, var(--ui-ink)); }
.identity-compare-columns section>span.missing { background:color-mix(in srgb, var(--amber) 15%, var(--ui-surface)); color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); }
.identity-compare-columns section>strong { color:var(--ui-ink); font:700 var(--text-meta) var(--font-mono); }
.identity-compare-columns section>small { grid-column:1/-1; overflow-wrap:anywhere; color:var(--ui-muted); font-size:var(--text-micro); line-height:1.45; }
.field-difference { display:flex; flex-wrap:wrap; gap:6px; margin-top:8px; }
.field-difference span { padding:6px 8px; border-radius:var(--radius-sm); background:color-mix(in srgb, var(--blue) 8%, var(--ui-surface)); color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); }
.ab-difference-table { overflow:hidden; border:1px solid var(--ui-line); border-radius:var(--radius-md); background:var(--ui-surface); }
.ab-difference-table>div { display:grid; grid-template-columns:120px repeat(2,minmax(0,1fr)); min-width:0; border-top:1px solid var(--ui-line); }
.ab-difference-table>div:first-child { border-top:0; }
.ab-difference-table span,.ab-difference-table b,.ab-difference-table code { min-width:0; padding:8px 10px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; border-left:1px solid var(--ui-line); font-size:var(--text-micro); }
.ab-difference-table span { border-left:0; color:var(--ui-muted); background:var(--ui-surface-raised); }
.ab-difference-table b { color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); background:var(--ui-surface-raised); font-size:var(--text-micro); }
.ab-difference-table code { color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); }
.ab-difference-table>div.changed code { background:color-mix(in srgb, var(--amber) 10%, var(--ui-surface)); color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); }
.ab-http-evidence { margin-top:10px; }
.ab-http-evidence>summary { cursor:pointer; color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); }
.ab-http-grid { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:8px; margin-top:8px; }
.ab-http-grid>section { min-width:0; overflow:hidden; border:1px solid var(--ui-line); border-radius:var(--radius-md); background:var(--ui-surface); }
.ab-http-grid>section>header { display:flex; justify-content:space-between; padding:8px 10px; border-bottom:1px solid var(--ui-line); background:var(--ui-surface-raised); }
.ab-http-grid>section>header b { color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); }
.ab-http-grid>section>header span,.ab-http-grid details>summary { color:var(--ui-muted); font-size:var(--text-micro); }
.ab-http-grid pre { min-height:130px; max-height:320px; margin:0; padding:10px; overflow:auto; background:var(--ui-surface); color:var(--ui-ink); font:var(--text-micro)/1.55 var(--font-mono); white-space:pre; }
.ab-http-grid details { border-top:1px solid var(--ui-line); }
.ab-http-grid details>summary { padding:6px 10px; cursor:pointer; background:var(--ui-surface-raised); }
.ab-http-grid details pre { background:#101a29; color:#dceafe; }
.technical-evidence { margin-top:8px; }
.technical-evidence>summary { color:var(--ui-muted); cursor:pointer; font-size:var(--text-micro); }
.technical-evidence pre { max-height:220px; overflow:auto; margin:8px 0 0; padding:10px; border-radius:var(--radius-md); background:#101a29; color:#dceafe; font:var(--text-micro)/1.45 var(--font-mono); white-space:pre-wrap; overflow-wrap:anywhere; }

/* Contracts are decisions first; raw execution JSON is a secondary disclosure. */
.human-contract-grid { grid-template-columns:1fr; }
.human-contract-grid>article { display:grid; grid-template-columns:minmax(0,1fr); gap:8px; padding:14px; border:1px solid var(--ui-line); border-radius:var(--radius-lg); background:var(--ui-surface); }
.human-contract-grid .contract-title,.human-contract-grid .contract-purpose,.human-contract-grid .contract-endpoint,.human-contract-grid .human-evidence-list,.human-contract-grid .contract-identity-scope,.human-contract-grid .contract-decision-line,.human-contract-grid .contract-actions,.human-contract-grid .mutation-approval,.human-contract-grid .technical-evidence { grid-column:1; }
.human-contract-grid .contract-title { display:flex; gap:10px; align-items:flex-start; }
.human-contract-grid .contract-title>div { display:grid; gap:3px; min-width:0; }
.human-contract-grid .contract-title small { color:var(--ui-muted); font-size:var(--text-micro); }
.contract-purpose { margin:0; color:var(--ui-muted); font-size:var(--text-micro); line-height:1.55; }
.contract-endpoint { display:flex; align-items:center; gap:8px; min-width:0; padding:8px; border-radius:var(--radius-md); background:var(--ui-surface-raised); }
.contract-endpoint code { min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); }
.human-evidence-list { display:flex; flex-wrap:wrap; gap:6px; }
.human-evidence-list span { display:inline-flex; align-items:center; gap:4px; padding:4px 8px; border:1px solid color-mix(in srgb, var(--blue) 23%, var(--ui-line)); border-radius:var(--radius-pill); color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); background:var(--ui-surface); font-size:var(--text-micro); }
.human-evidence-list svg { color:color-mix(in srgb, var(--green) 82%, var(--ui-ink)); }
.contract-identity-scope { display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:6px; }
.contract-identity-scope .identity-chip { display:grid; gap:3px; min-width:0; padding:8px; border-radius:var(--radius-md); font-style:normal; }
.contract-identity-scope .identity-chip b { font-size:var(--text-micro); }
.contract-identity-scope .identity-chip small { color:var(--ui-muted); font-size:var(--text-micro); }
.contract-decision-line { display:flex; align-items:flex-start; gap:6px; padding:8px; border-radius:var(--radius-md); background:color-mix(in srgb, var(--blue) 8%, var(--ui-surface)); color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); line-height:1.5; }
.contract-actions { display:flex; align-items:center; gap:8px; flex-wrap:wrap; color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink)); font-size:var(--text-micro); font-weight:700; }
.human-contract-grid .mutation-approval { margin-top:0; padding:10px 10px; border:1px solid color-mix(in srgb, var(--amber) 50%, var(--ui-line)); border-radius:var(--radius-md); background:color-mix(in srgb, var(--amber) 7%, var(--ui-surface)); }
.human-contract-grid .mutation-approval small { min-width:0; color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink)); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.technical-contract ul { margin:8px 0; padding-left:20px; color:var(--ui-muted); font-size:var(--text-micro); }
@media(max-width:900px) { .human-contract-grid>article { grid-template-columns:1fr; }.human-contract-grid .contract-title,.human-contract-grid .contract-purpose,.human-contract-grid .contract-endpoint,.human-contract-grid .human-evidence-list,.human-contract-grid .contract-identity-scope,.human-contract-grid .contract-decision-line,.human-contract-grid .contract-actions,.human-contract-grid .mutation-approval,.human-contract-grid .technical-evidence { grid-column:1; } }
@media(max-width:760px) { .identity-account-grid,.identity-compare-columns,.contract-identity-scope,.ab-http-grid { grid-template-columns:1fr; }.identity-compare-columns section { grid-template-columns:auto 1fr; }.identity-compare-columns section>strong { grid-column:1/-1; }.identity-kpi-row { grid-template-columns:1fr; }.ab-difference-table>div { grid-template-columns:80px repeat(2,minmax(0,1fr)); } }

</style>
