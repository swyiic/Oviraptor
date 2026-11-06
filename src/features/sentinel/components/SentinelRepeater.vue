<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { CheckCircle2, Copy, RefreshCw, Send, ShieldAlert, UserRound, X } from "@lucide/vue";
import type { InvestigationApiModel, InvestigationHypothesis, InvestigationValidation, SentinelOpportunity } from "../types";
import { sentinelApi } from "../api";
import { buildRawHttpRequest, buildRawHttpResponse, parseRawHttpRequest, prettyHttpBody } from "../httpMessage";
import { useI18n } from "../../../i18n";

const props = defineProps<{
  scanId: string;
  targetUrl: string;
  opportunity: SentinelOpportunity;
  api?: InvestigationApiModel;
  hypothesis?: InvestigationHypothesis;
}>();
const emit = defineEmits<{ close: []; saved: [validation: InvestigationValidation] }>();
const { tr } = useI18n();

type ReplayResponse = { status: number; statusText: string; headers: Record<string, string>; body: string; decodedBody?: string; contentType?: string; contentEncoding?: string; elapsedMs: number; identityId?: string };
type IdentityChoice = { key: string; label: string; available: boolean; observation?: Record<string, any>; status?: number; replayed?: boolean };
const anonymousIdentity = (value: unknown) => String(value || "").trim().toLowerCase() === "anonymous";
const method = ref("GET");
const url = ref("");
const headers = ref<Record<string, string>>({});
const body = ref("");
const requestRaw = ref("");
const requestTab = ref<"message" | "structure">("message");
const responseTab = ref<"pretty" | "raw" | "headers">("pretty");
const wrapLines = ref(true);
const allowMutation = ref(false);
const busy = ref(false);
const error = ref("");
const response = ref<ReplayResponse>();
const verdict = ref("");
const severity = ref("medium");
const confidence = ref("medium");
const note = ref("");
const nextAction = ref("");
const saved = ref<InvestigationValidation>();
const history = ref<InvestigationValidation[]>([]);
const selectedIdentityKey = ref("");
const responseIdentityKey = ref("");

const opportunityRecord = computed<Record<string, any>>(() => props.opportunity.record || {});
const fallbackUrl = computed(() => String(
  opportunityRecord.value.url || opportunityRecord.value.endpoint || opportunityRecord.value.route ||
  props.opportunity.recommendedAction?.targetUrl || props.targetUrl || "",
));
const fallbackMethod = computed(() => String(
  opportunityRecord.value.method || opportunityRecord.value.httpMethod ||
  props.opportunity.recommendedAction?.method || "GET",
).toUpperCase());
const fallbackHeaders = computed<Record<string, string>>(() => {
  const record = opportunityRecord.value;
  const value = record.effectiveRequestHeaders || record.requestHeaders || record.headers ||
    props.opportunity.recommendedAction?.headers || {};
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, String(item)]));
});
const fallbackBody = computed(() => String(
  opportunityRecord.value.postData || opportunityRecord.value.requestBody || opportunityRecord.value.body ||
  props.opportunity.recommendedAction?.body || "",
));
const repeaterTitle = computed(() => props.api
  ? tr(`请求重放 · ${String(props.api.method || fallbackMethod.value).toUpperCase()} ${props.api.normalizedPath || "/"}`, `Request replay · ${String(props.api.method || fallbackMethod.value).toUpperCase()} ${props.api.normalizedPath || "/"}`)
  : props.opportunity.title);
const repeaterSubtitle = computed(() => String(props.api?.url || fallbackUrl.value || props.targetUrl || ""));
const identityChoices = computed<IdentityChoice[]>(() => {
  const apiKeys = Array.isArray(props.api?.identityKeys) ? props.api!.identityKeys.map(String).filter(Boolean) : [];
  const observations = Array.isArray(props.api?.payload?.identityObservations)
    ? props.api!.payload.identityObservations.filter((item: any) => item && typeof item === "object")
    : [];
  const keys = [...new Set([...apiKeys, ...observations.map((item: any) => String(item.identityKey || "")).filter(Boolean)])];
  if (!keys.length) return [{ key: "captured", label: tr("原始采集", "Original capture"), available: true }];
  return keys.map((key, index) => {
    const observation = observations.find((item: any) => String(item.identityKey || "") === key);
    return {
      key,
      label: anonymousIdentity(key) ? tr("匿名访问", "Anonymous") : tr(`账号 ${String.fromCharCode(65 + keys.slice(0, index).filter((item) => !anonymousIdentity(item)).length)}`, `Account ${String.fromCharCode(65 + keys.slice(0, index).filter((item) => !anonymousIdentity(item)).length)}`),
      available: Boolean(observation?.observed !== false && (observation?.url || props.api?.url)),
      observation,
      status: observation?.status,
      replayed: Boolean(observation?.replayed),
    };
  });
});
const authenticatedIdentityChoices = computed(() => identityChoices.value.filter((item) => !anonymousIdentity(item.key) && item.key !== "captured"));
const showIdentitySelector = computed(() => authenticatedIdentityChoices.value.length > 0);
const selectedIdentity = computed(() => identityChoices.value.find((item) => item.key === selectedIdentityKey.value) || identityChoices.value[0]);
const responseIdentityLabel = computed(() => showIdentitySelector.value ? identityChoices.value.find((item) => item.key === responseIdentityKey.value)?.label || "" : "");
const sendButtonLabel = computed(() => busy.value ? tr("发送中…", "Sending…") : showIdentitySelector.value ? tr(`以 ${selectedIdentity.value?.label || "当前账号"} 发送`, `Send as ${selectedIdentity.value?.label || "current account"}`) : tr("发送请求", "Send request"));
const responseWaitingLabel = computed(() => showIdentitySelector.value ? tr(`等待 ${selectedIdentity.value?.label || "当前账号"} 的 HTTP 响应`, `Waiting for the HTTP response as ${selectedIdentity.value?.label || "current account"}`) : tr("等待 HTTP 响应", "Waiting for the HTTP response"));
const assessmentNotePlaceholder = computed(() => showIdentitySelector.value ? tr("记录账号差异、判断依据和缺失证据", "Note account differences, reasoning and missing evidence") : tr("记录判断依据、响应差异和缺失证据", "Note reasoning, response differences and missing evidence"));
const selectedObservation = computed<Record<string, any>>(() => selectedIdentity.value?.observation || {});
const initialHeaders = computed<Record<string, string>>(() => {
  const observation = selectedObservation.value;
  const value = observation.requestHeaders || observation.effectiveRequestHeaders || observation.headers ||
    props.api?.payload?.headers || props.api?.payload?.effectiveRequestHeaders || props.api?.payload?.requestHeaders || fallbackHeaders.value;
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, String(item)]));
});
const parsedPreview = computed(() => {
  try { return parseRawHttpRequest(requestRaw.value, url.value || fallbackUrl.value); } catch { return undefined; }
});
const mutation = computed(() => !["GET", "HEAD", "OPTIONS"].includes(String(parsedPreview.value?.method || requestRaw.value.trim().split(/\s+/)[0] || "GET").toUpperCase()));
const canSend = computed(() => Boolean(parsedPreview.value?.url) && (!mutation.value || allowMutation.value) && !busy.value);
const decodedBody = computed(() => prettyHttpBody(response.value?.decodedBody || response.value?.body || ""));
const rawResponse = computed(() => buildRawHttpResponse({
  status: response.value?.status ?? null,
  statusText: response.value?.statusText,
  headers: response.value?.headers,
  body: response.value?.decodedBody || response.value?.body || "",
}));
const responseHeaders = computed(() => {
  if (!response.value) return "";
  return Object.entries(response.value.headers || {}).map(([name, value]) => `${name}: ${value}`).join("\r\n");
});
const requestStats = computed(() => {
  const parsed = parsedPreview.value;
  return {
    method: parsed?.method || tr("无法解析", "Unparseable"),
    url: parsed?.url || tr("请检查请求行与 Host", "Check the request line and Host"),
    headerCount: parsed ? Object.keys(parsed.headers).length : 0,
    bodyBytes: parsed ? new TextEncoder().encode(parsed.body).length : 0,
  };
});
const aiAssessment = computed(() => {
  if (!response.value) return tr("发送请求后生成初步判断。", "A preliminary assessment appears after you send the request.");
  if (response.value.status >= 500) return tr("请求触发服务端错误，暂不能直接认定为安全问题，建议保留证据并继续核查。", "The request triggered a server error. This alone is not a security issue; keep the evidence and keep checking.");
  if ([401, 403].includes(response.value.status)) return showIdentitySelector.value ? tr("请求被身份或权限边界拦截；需要与另一账号使用同一请求进行对比。", "The request was blocked by an identity or permission boundary; compare the same request with another account.") : tr("匿名请求被身份或权限边界拦截；这是访问边界证据，不能单独认定为漏洞。", "The anonymous request was blocked by an identity or permission boundary. This is access-boundary evidence, not a vulnerability on its own.");
  if (response.value.status >= 400) return tr("请求返回客户端错误，建议检查参数契约和授权边界后再判断。", "The request returned a client error; check the parameter contract and authorization boundary before deciding.");
  const text = response.value.decodedBody || response.value.body || "";
  if (/password|token|secret|role|user_id|account_id|permission/i.test(text)) return showIdentitySelector.value ? tr("响应包含身份、权限或对象字段，建议进入账号间同请求对比。", "The response contains identity, permission or object fields; compare the same request across accounts.") : tr("匿名响应包含身份、权限或对象字段，建议先确认字段是否为公开数据，再决定是否补充登录态对照。", "The anonymous response contains identity, permission or object fields. Confirm whether they are public data before deciding to add a signed-in comparison.");
  return showIdentitySelector.value ? tr("收到正常响应，当前没有足够证据确认问题；可标记为正常，或继续补充账号间对比证据。", "Received a normal response with not enough evidence to confirm an issue. Mark it normal, or add more cross-account comparison evidence.") : tr("收到正常匿名响应，当前没有足够证据确认问题；可标记为正常，或继续核查响应内容与业务影响。", "Received a normal anonymous response with not enough evidence to confirm an issue. Mark it normal, or keep checking the response content and business impact.");
});

function restoreCapturedRequest() {
  const observation = selectedObservation.value;
  method.value = String(observation.method || props.api?.method || fallbackMethod.value || "GET").toUpperCase();
  url.value = String(observation.url || props.api?.url || fallbackUrl.value || props.targetUrl || "");
  headers.value = { ...initialHeaders.value };
  body.value = String(observation.requestBody || observation.postData || observation.body || props.api?.payload?.postData || props.api?.payload?.requestBody || props.api?.payload?.body || fallbackBody.value || "");
  requestRaw.value = buildRawHttpRequest({ method: method.value, url: url.value, headers: headers.value, body: body.value });
  error.value = "";
}
function resetFromContext() {
  selectedIdentityKey.value = identityChoices.value.find((item) => item.available)?.key || identityChoices.value[0]?.key || "";
  restoreCapturedRequest();
  response.value = undefined;
  responseIdentityKey.value = "";
  verdict.value = "";
  saved.value = undefined;
  allowMutation.value = false;
  responseTab.value = "pretty";
  requestTab.value = "message";
  void loadHistory();
}
function selectIdentity(key: string) {
  const choice = identityChoices.value.find((item) => item.key === key);
  if (!choice?.available || choice.key === selectedIdentityKey.value) return;
  selectedIdentityKey.value = choice.key;
  restoreCapturedRequest();
  response.value = undefined;
  responseIdentityKey.value = "";
  saved.value = undefined;
}
async function loadHistory() {
  try {
    const rows = await sentinelApi.listInvestigationValidations(props.scanId, props.opportunity.id > 0 ? props.opportunity.id : undefined);
    history.value = props.opportunity.id > 0 ? rows : rows.filter((row) => !props.api?.apiKey || row.apiKey === props.api.apiKey);
  } catch { history.value = []; }
}
watch(() => [props.api, props.opportunity, props.scanId], resetFromContext, { immediate: true });

function copy(value: string) { void navigator.clipboard?.writeText(value); }
function setVerdict(value: string) {
  verdict.value = value;
  if (value === "confirmed_issue") severity.value = severity.value === "none" ? "medium" : severity.value;
  if (value === "normal" || value === "not_applicable") severity.value = "none";
  if (value === "needs_more_evidence") nextAction.value = showIdentitySelector.value ? tr("继续同请求账号对比并补齐缺失响应", "Keep comparing the same request across accounts and fill in missing responses") : tr("补充请求参数、响应内容或登录态对照证据", "Add request parameters, response content or signed-in comparison evidence");
  if (value === "confirmed_issue") nextAction.value = tr("创建行动项并进入漏洞结论复核", "Create an action item and send the vulnerability verdict to review");
}
function historyIdentityLabel(identityId?: string) {
  if (!showIdentitySelector.value || anonymousIdentity(identityId)) return tr("匿名请求", "Anonymous request");
  return identityChoices.value.find((choice) => choice.key === identityId)?.label || identityId || tr("未标注账号", "Unlabeled account");
}
function applyRawRequest() {
  const parsed = parseRawHttpRequest(requestRaw.value, url.value || fallbackUrl.value);
  method.value = parsed.method;
  url.value = parsed.url;
  headers.value = parsed.headers;
  body.value = parsed.body;
  return parsed;
}
async function send() {
  error.value = "";
  let parsed;
  try { parsed = applyRawRequest(); } catch (e) { error.value = String(e); return; }
  if (!canSend.value) { error.value = mutation.value ? tr("状态变更请求必须确认授权后才能发送", "State-changing requests need your authorization before sending") : tr("请求报文缺少有效 URL", "The request message has no valid URL"); return; }
  busy.value = true;
  response.value = undefined;
  saved.value = undefined;
  try {
    response.value = await sentinelApi.replayInvestigationRequest({ ...parsed, allowMutation: allowMutation.value, timeoutMs: 120000, identityId: selectedIdentity.value?.key || "" });
    responseIdentityKey.value = response.value.identityId || selectedIdentity.value?.key || "";
    verdict.value = "";
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
function responseForCopy() {
  if (!response.value) return "";
  if (responseTab.value === "headers") return responseHeaders.value;
  return responseTab.value === "pretty" ? decodedBody.value : rawResponse.value;
}
async function save() {
  if (!response.value || !verdict.value) { error.value = tr("请先发送请求，并选择验证结论", "Send the request and choose a verdict first"); return; }
  try { applyRawRequest(); } catch (e) { error.value = String(e); return; }
  busy.value = true;
  error.value = "";
  try {
    const result = await sentinelApi.saveInvestigationValidation({
      scanId: props.scanId, targetUrl: props.targetUrl, opportunityId: props.opportunity.id > 0 ? props.opportunity.id : undefined, hypothesisId: props.hypothesis?.id,
      apiKey: props.api?.apiKey, identityId: responseIdentityKey.value || selectedIdentity.value?.key, method: method.value, requestUrl: url.value, requestHeaders: headers.value, requestBody: body.value,
      responseStatus: response.value.status, responseStatusText: response.value.statusText, responseHeaders: response.value.headers,
      responseBody: response.value.body, decodedBody: decodedBody.value, verdict: verdict.value, severity: severity.value,
      confidence: confidence.value, aiAssessment: aiAssessment.value, note: note.value, nextAction: nextAction.value,
      evidenceRefs: [props.api?.apiKey, `opportunity:${props.opportunity.id}`].filter(Boolean) as string[],
    });
    saved.value = result;
    history.value = [result, ...history.value.filter((row) => row.id !== result.id)];
    emit("saved", result);
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
</script>

<template>
  <div class="repeater-backdrop" @click.self="emit('close')">
    <section class="sentinel-repeater" role="dialog" aria-modal="true">
      <header class="repeater-topbar">
        <div class="repeater-heading-copy"><span class="eyebrow">{{ tr(`HTTP 重放 · ${opportunity.category}`, `HTTP replay · ${opportunity.category}`) }}</span><h2 :title="repeaterTitle">{{ repeaterTitle }}</h2><p :title="repeaterSubtitle">{{ repeaterSubtitle }}</p></div>
        <button class="icon-button" type="button" :title="tr('关闭', 'Close')" @click="emit('close')"><X :size="18" /></button>
      </header>
      <div class="repeater-context"><span>{{ tr(`任务 ${scanId}`, `Task ${scanId}`) }}</span><span v-if="api">{{ api.method }} {{ api.normalizedPath }}</span><span v-if="hypothesis">{{ tr(`契约 ${hypothesis.status}`, `Contract ${hypothesis.status}`) }}</span><span>{{ mutation ? tr("状态变更：发送前授权", "State-changing: authorize before sending") : tr("只读：可直接发送", "Read-only: send directly") }}</span></div>
      <div v-if="showIdentitySelector" class="identity-replay-bar">
        <div class="identity-replay-title"><UserRound :size="16" /><span><b>{{ tr('选择重放身份', 'Choose replay identity') }}</b><small>{{ tr('切换后载入该账户实际采集的 URL、Cookie、请求头与请求体，不混用 A/B 会话。', 'Switching loads the URL, cookies, headers and body actually captured for that account. A/B sessions are never mixed.') }}</small></span></div>
        <div class="identity-replay-options">
          <button v-for="choice in identityChoices" :key="choice.key" type="button" :disabled="!choice.available" :class="{ selected: choice.key === selectedIdentity?.key }" @click="selectIdentity(choice.key)">
            <i></i><span><b>{{ choice.label }}</b><small>{{ choice.available ? `${choice.replayed ? tr('交叉重放采集', 'Cross-replay capture') : tr('浏览器自然采集', 'Natural browser capture')}${choice.status ? ` · HTTP ${choice.status}` : ''}` : tr('该身份没有完整请求', 'No complete request for this identity') }}</small></span>
          </button>
        </div>
        <strong class="identity-replay-current">{{ tr(`本次将以 ${selectedIdentity?.label || "原始采集"} 发送`, `Will send as ${selectedIdentity?.label || "original capture"}`) }}</strong>
      </div>
      <main class="repeater-main">
        <section class="repeater-pane request-pane">
          <header class="message-heading"><div><b>Request</b><small>{{ tr('完整 HTTP 请求报文', 'Full HTTP request message') }}</small></div><div><button class="text-button" type="button" @click="restoreCapturedRequest"><RefreshCw :size="13" />{{ tr('恢复采集', 'Restore capture') }}</button><button class="text-button" type="button" @click="copy(requestRaw)"><Copy :size="13" />{{ tr('复制', 'Copy') }}</button></div></header>
          <nav class="message-tabs"><button :class="{active:requestTab==='message'}" @click="requestTab='message'">{{ tr('报文', 'Message') }}</button><button :class="{active:requestTab==='structure'}" @click="requestTab='structure'">{{ tr('结构', 'Structure') }}</button><button :class="{active:wrapLines}" @click="wrapLines=!wrapLines">{{ wrapLines ? tr('自动换行', 'Wrap lines') : tr('单行滚动', 'No wrap') }}</button></nav>
          <textarea v-if="requestTab==='message'" v-model="requestRaw" class="http-editor" :class="{nowrap:!wrapLines}" spellcheck="false" />
          <div v-else class="message-structure"><dl><div><dt>{{ tr('方法', 'Method') }}</dt><dd>{{ requestStats.method }}</dd></div><div><dt>{{ tr('完整 URL', 'Full URL') }}</dt><dd><code>{{ requestStats.url }}</code></dd></div><div><dt>{{ tr('请求头', 'Headers') }}</dt><dd>{{ tr(`${requestStats.headerCount} 个`, `${requestStats.headerCount}`) }}</dd></div><div><dt>{{ tr('请求体', 'Body') }}</dt><dd>{{ requestStats.bodyBytes }} bytes</dd></div></dl><p>{{ tr('结构视图只用于快速确认；切回“报文”可直接修改请求行、请求头和正文。', 'The structure view is for a quick check only. Switch back to "Message" to edit the request line, headers and body.') }}</p></div>
          <div class="repeater-sendbar"><label v-if="mutation" class="mutation-warning"><input v-model="allowMutation" type="checkbox" />{{ tr('授权发送这一次状态变更请求', 'Authorize sending this state-changing request once') }}</label><span v-else class="readonly-badge">{{ tr('GET / HEAD / OPTIONS 无需人工放行', 'GET / HEAD / OPTIONS need no manual approval') }}</span><button class="button primary" :disabled="!canSend" type="button" @click="send"><Send :size="14" />{{ sendButtonLabel }}</button></div>
        </section>
        <section class="repeater-pane response-pane">
          <header class="message-heading"><div><b>Response <em v-if="responseIdentityLabel">{{ responseIdentityLabel }}</em></b><small v-if="response">{{ response.status }} {{ response.statusText }} · {{ response.elapsedMs }} ms · {{ response.contentType || "unknown" }}</small><small v-else>{{ responseWaitingLabel }}</small></div><button v-if="response" class="text-button" type="button" @click="copy(responseForCopy())"><Copy :size="13" />{{ tr('复制', 'Copy') }}</button></header>
          <nav class="message-tabs"><button :class="{active:responseTab==='pretty'}" @click="responseTab='pretty'">Pretty</button><button :class="{active:responseTab==='raw'}" @click="responseTab='raw'">Raw</button><button :class="{active:responseTab==='headers'}" @click="responseTab='headers'">Headers</button></nav>
          <pre v-if="response" class="http-viewer" :class="{nowrap:!wrapLines}">{{ responseTab === 'pretty' ? decodedBody : responseTab === 'raw' ? rawResponse : responseHeaders }}</pre><div v-else class="response-empty"><Send :size="22" /><span>{{ tr('发送后在这里显示完整 HTTP 响应', 'The full HTTP response appears here after sending') }}</span></div>
          <details v-if="response" class="assessment-drawer"><summary><ShieldAlert :size="15" /><b>{{ tr('AI 判断与验证结论', 'AI assessment and verdict') }}</b><span>{{ aiAssessment }}</span></summary><div class="assessment-card"><div class="verdict-buttons"><button :class="{selected:verdict==='confirmed_issue'}" @click="setVerdict('confirmed_issue')">{{ tr('确认存在问题', 'Confirmed issue') }}</button><button :class="{selected:verdict==='normal'}" @click="setVerdict('normal')">{{ tr('正常 / 可忽略', 'Normal / ignorable') }}</button><button :class="{selected:verdict==='needs_more_evidence'}" @click="setVerdict('needs_more_evidence')">{{ tr('需要更多证据', 'Needs more evidence') }}</button><button :class="{selected:verdict==='unauthorized_stop'}" @click="setVerdict('unauthorized_stop')">{{ tr('权限边界 / 停止', 'Permission boundary / stop') }}</button></div><div class="assessment-fields"><select v-model="severity"><option value="none">{{ tr('无', 'None') }}</option><option value="low">{{ tr('低', 'Low') }}</option><option value="medium">{{ tr('中', 'Medium') }}</option><option value="high">{{ tr('高', 'High') }}</option><option value="critical">{{ tr('严重', 'Critical') }}</option></select><select v-model="confidence"><option>low</option><option>medium</option><option>high</option></select><input v-model="nextAction" :placeholder="tr('下一步动作', 'Next action')" /></div><textarea v-model="note" :placeholder="assessmentNotePlaceholder"></textarea><button class="button primary save-verdict" :disabled="busy || !verdict" @click="save"><CheckCircle2 :size="14" />{{ saved ? tr('已保存并同步', 'Saved and synced') : tr('保存验证结论', 'Save verdict') }}</button></div></details>
          <details v-if="history.length" class="validation-history"><summary>{{ tr(`验证历史（${history.length} 次）`, `Validation history (${history.length})`) }}</summary><article v-for="item in history" :key="item.id"><header><b>{{ historyIdentityLabel(item.identityId) }} · {{ item.verdict }}</b><span>{{ item.responseStatus }} · {{ item.severity }} · {{ item.confidence }}</span><time>{{ item.updatedAt }}</time></header><p>{{ item.aiAssessment || item.note || tr("未填写说明", "No notes") }}</p><code>{{ item.method }} {{ item.requestUrl }}</code></article></details>
        </section>
      </main>
      <p v-if="error" class="repeater-error">{{ error }}</p>
    </section>
  </div>
</template>

<style scoped>
.repeater-backdrop{position:fixed;inset:0;z-index:80;display:grid;place-items:center;padding:14px;background:rgba(11,20,36,.58);backdrop-filter:blur(4px)}
.sentinel-repeater{width:min(1680px,98vw);height:min(980px,96vh);display:flex;flex-direction:column;overflow:hidden;border:1px solid color-mix(in srgb, var(--blue) 36%, var(--ui-line));border-radius:var(--radius-lg);background:var(--ui-surface-raised);box-shadow:0 28px 90px rgba(0,0,0,.28);color:var(--ui-ink)}
.repeater-topbar{display:flex;justify-content:space-between;gap:20px;padding:16px 20px 12px;border-bottom:1px solid color-mix(in srgb, var(--blue) 23%, var(--ui-line));background:var(--ui-surface)}.repeater-heading-copy{min-width:0;flex:1}.repeater-topbar h2{margin:3px 0 2px;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-title)}.repeater-topbar p{margin:0;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ui-muted);font:var(--text-micro) var(--font-mono)}.icon-button{display:grid;place-items:center;width:32px;height:32px;flex:0 0 32px;border:1px solid color-mix(in srgb, var(--blue) 28%, var(--ui-line));border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-muted)}
.repeater-context{display:flex;gap:6px;flex-wrap:wrap;padding:8px 20px;color:var(--ui-muted);font:var(--text-micro) var(--font-mono);background:var(--ui-surface);border-bottom:1px solid var(--ui-line)}.repeater-context span{padding:3px 8px;border:1px solid color-mix(in srgb, var(--blue) 23%, var(--ui-line));border-radius:var(--radius-pill);background:var(--ui-surface)}
.identity-replay-bar{display:grid;grid-template-columns:minmax(220px,.8fr) minmax(360px,1.4fr) auto;align-items:center;gap:12px;padding:10px 20px;border-bottom:1px solid color-mix(in srgb, var(--blue) 24%, var(--ui-line));background:color-mix(in srgb, var(--blue) 8%, var(--ui-surface))}.identity-replay-title{display:flex;align-items:center;gap:8px;color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink))}.identity-replay-title span{min-width:0}.identity-replay-title b,.identity-replay-title small{display:block}.identity-replay-title b{font-size:var(--text-meta)}.identity-replay-title small{margin-top:2px;color:var(--ui-muted);font-size:var(--text-micro)}.identity-replay-options{display:flex;gap:6px;min-width:0;overflow-x:auto}.identity-replay-options button{display:flex;align-items:center;gap:8px;min-width:128px;padding:6px 10px;border:1px solid color-mix(in srgb, var(--blue) 35%, var(--ui-line));border-radius:var(--radius-md);background:var(--ui-surface);color:var(--ui-muted);text-align:left}.identity-replay-options button.selected{border-color:#327be0;background:color-mix(in srgb, var(--blue) 13%, var(--ui-surface));color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));box-shadow:0 0 0 1px rgba(50,123,224,.14)}.identity-replay-options button:disabled{opacity:.48}.identity-replay-options i{width:8px;height:8px;flex:0 0 auto;border-radius:50%;background:#9aa9ba}.identity-replay-options button.selected i{background:#20a46b;box-shadow:0 0 0 3px rgba(32,164,107,.14)}.identity-replay-options b,.identity-replay-options small{display:block}.identity-replay-options b{font-size:var(--text-micro)}.identity-replay-options small{margin-top:1px;white-space:nowrap;font-size:var(--text-micro);color:var(--ui-muted)}.identity-replay-current{padding:6px 10px;border-radius:var(--radius-sm);background:#1f6ed4;color:#fff;white-space:nowrap;font-size:var(--text-micro)}.message-heading em{display:inline-block;margin-left:6px;padding:2px 6px;border-radius:var(--radius-pill);background:color-mix(in srgb, var(--blue) 12%, var(--ui-surface));color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font-size:var(--text-micro);font-style:normal}
.repeater-main{display:grid;grid-template-columns:minmax(0,1.08fr) minmax(0,.92fr);gap:1px;min-height:0;flex:1;background:color-mix(in srgb, var(--blue) 28%, var(--ui-surface))}.repeater-pane{display:flex;flex-direction:column;min-width:0;min-height:0;padding:0;background:var(--ui-surface)}.message-heading{display:flex;justify-content:space-between;gap:10px;align-items:center;padding:12px 14px 8px}.message-heading>div:last-child{display:flex;gap:10px}.message-heading b{display:block;font-size:var(--text-heading)}.message-heading small{display:block;margin-top:2px;color:var(--ui-muted);font-size:var(--text-micro)}.text-button{display:inline-flex;align-items:center;gap:4px;border:0;background:transparent;color:#4c79b5;font-size:var(--text-micro)}
.message-tabs{display:flex;align-items:center;gap:1px;padding:0 10px;border-bottom:1px solid var(--ui-line)}.message-tabs button{border:0;border-bottom:2px solid transparent;padding:8px 10px;background:transparent;color:var(--ui-muted);font-size:var(--text-micro)}.message-tabs button.active{border-bottom-color:#f06d55;color:var(--ui-ink);font-weight:800}.message-tabs button:last-child{margin-left:auto;color:var(--ui-muted)}
.http-editor,.http-viewer{box-sizing:border-box;width:100%;min-height:0;flex:1;margin:0;border:0;outline:0;padding:12px 14px;background:var(--ui-surface);color:var(--ui-ink);font:var(--text-body)/1.55 var(--font-mono);tab-size:2;white-space:pre-wrap;overflow:auto;overflow-wrap:anywhere}.http-editor{resize:none}.http-editor.nowrap,.http-viewer.nowrap{white-space:pre;overflow-wrap:normal}.http-viewer{border-bottom:1px solid var(--ui-line)}.message-structure{display:grid;align-content:start;gap:16px;min-height:0;flex:1;padding:20px;overflow:auto}.message-structure dl{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;margin:0}.message-structure dl>div{min-width:0;padding:12px;border:1px solid var(--ui-line);border-radius:var(--radius-md);background:var(--ui-surface-raised)}.message-structure dt{color:var(--ui-muted);font-size:var(--text-micro)}.message-structure dd{margin:6px 0 0;color:var(--ui-ink);font-size:var(--text-meta)}.message-structure code{overflow-wrap:anywhere}.message-structure p{margin:0;color:var(--ui-muted);font-size:var(--text-micro)}
.repeater-sendbar{display:flex;align-items:center;gap:10px;padding:10px 12px;border-top:1px solid var(--ui-line);background:var(--ui-surface-raised)}.mutation-warning{display:flex;align-items:center;gap:6px;flex:1;color:color-mix(in srgb, var(--amber) 82%, var(--ui-ink));font-size:var(--text-micro)}.readonly-badge{flex:1;color:color-mix(in srgb, var(--green) 82%, var(--ui-ink));font-size:var(--text-micro)}.repeater-sendbar .button{min-width:86px;justify-content:center}.response-empty{display:grid;place-items:center;gap:8px;flex:1;color:var(--ui-muted);font-size:var(--text-micro);background:var(--ui-surface)}
.assessment-drawer,.validation-history{flex:0 0 auto;border-top:1px solid color-mix(in srgb, var(--blue) 25%, var(--ui-line));background:var(--ui-surface)}.assessment-drawer>summary,.validation-history>summary{display:grid;grid-template-columns:auto auto minmax(0,1fr);align-items:center;gap:8px;padding:10px 12px;cursor:pointer;color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font-size:var(--text-micro)}.assessment-drawer>summary span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ui-muted);font-weight:400}.assessment-card{display:grid;gap:8px;padding:0 12px 12px}.verdict-buttons{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:6px}.verdict-buttons button{padding:8px;border:1px solid color-mix(in srgb, var(--blue) 29%, var(--ui-line));border-radius:var(--radius-sm);background:var(--ui-surface);color:var(--ui-muted);font-size:var(--text-micro)}.verdict-buttons button.selected{border-color:#4e88db;background:color-mix(in srgb, var(--blue) 10%, var(--ui-surface));color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font-weight:700}.assessment-fields{display:grid;grid-template-columns:90px 90px minmax(0,1fr);gap:6px}.assessment-fields select,.assessment-fields input,.assessment-card textarea{min-width:0;border:1px solid color-mix(in srgb, var(--blue) 30%, var(--ui-line));border-radius:var(--radius-sm);background:var(--ui-surface);color:var(--ui-ink);padding:8px;font-size:var(--text-micro)}.assessment-card textarea{min-height:48px;resize:vertical}.save-verdict{justify-content:center}.validation-history article{display:grid;gap:3px;padding:8px 12px;border-top:1px solid var(--ui-line)}.validation-history article header{display:flex;gap:8px;align-items:center}.validation-history article span,.validation-history article time,.validation-history article p{margin:0;color:var(--ui-muted);font-size:var(--text-micro)}.validation-history article code{overflow-wrap:anywhere;color:color-mix(in srgb, var(--blue) 82%, var(--ui-ink));font:var(--text-micro) var(--font-mono)}.repeater-error{margin:0;padding:8px 14px;color:#c63f4c;font-size:var(--text-micro);background:color-mix(in srgb, var(--red) 7%, var(--ui-surface))}
@media(max-width:900px){.repeater-main{grid-template-columns:1fr;overflow:auto}.sentinel-repeater{height:97vh}.repeater-pane{min-height:520px}.verdict-buttons{grid-template-columns:repeat(2,1fr)}}
@media(max-width:560px){.repeater-backdrop{padding:6px}.repeater-topbar{padding:12px}.repeater-context{padding:8px 12px}.assessment-fields,.message-structure dl{grid-template-columns:1fr}.assessment-drawer>summary{grid-template-columns:auto 1fr}.assessment-drawer>summary span{grid-column:1/-1}}
</style>
