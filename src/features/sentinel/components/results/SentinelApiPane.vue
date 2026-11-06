<script setup lang="ts">
import {
  Activity,
  ChevronDown,
  ClipboardCopy,
  Code2,
  ExternalLink,
  Network,
  Shield,
  ShieldAlert,
} from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import SentinelJsFiles from "./SentinelJsFiles.vue";
import SentinelRequestHeaders from "./SentinelRequestHeaders.vue";
import SentinelRuntimeExploration from "./SentinelRuntimeExploration.vue";
import type { SentinelFinding } from "../../types";
import {
  createSentinelLabels,
  cryptoCategory,
  isHttpUrl,
  json,
  kindLabel,
  methodTone,
  safeSeverity,
  sensitiveType,
  text,
} from "../../presentation";

const { tr } = useI18n();
const { severityLabel } = createSentinelLabels(tr);

defineProps<{
apiRows: SentinelFinding[];
  cryptoRows: SentinelFinding[];
  declaredRequestHeaderRows: Array<Record<string, any>>;
  observedRequestHeaderRows: Array<Record<string, any>>;
  possibleRequestHeaderRows: Array<Record<string, any>>;
  expandedApiRows: number[];
  expandedSensitive: number[];
  jsRows: SentinelFinding[];
  observedMutationRows: SentinelFinding[];
  realtimeEndpointRows: SentinelFinding[];
  registrationRows: SentinelFinding[];
  requestHeaderIntelligence: Record<string, any>;
  routeRows: SentinelFinding[];
  runtimeActionRows: SentinelFinding[];
  runtimeFeatureRows: SentinelFinding[];
  runtimeRows: SentinelFinding[];
  sensitiveRows: SentinelFinding[];
  apiUrl: (item: SentinelFinding) => string;
  apiPath: (item: SentinelFinding) => string;
  apiQuery: (item: SentinelFinding) => string[];
  apiRecord: (item: SentinelFinding) => Record<string, any>;
  apiMethod: (item: SentinelFinding) => string;
  apiResponseSummary: (item: SentinelFinding) => string;
  apiSourceSummary: (item: SentinelFinding) => string;
  apiDescription: (item: SentinelFinding) => string;
  apiRequestPayload: (item: SentinelFinding) => any;
  apiResponseHeaders: (item: SentinelFinding) => any;
  apiIdentitySummary: (item: SentinelFinding) => string;
  registrationData: (item: SentinelFinding) => Record<string, any>;
  runtimeSignalUrl: (item: SentinelFinding) => string;
  headerDisplayValue: (row: Record<string, any>) => string;
  copyText: (value: string) => Promise<void>;
  openTargetUrl: (url: string) => Promise<void>;
  toggleApiRow: (id: number) => void;
  toggleSensitive: (id: number) => void;
}>();
</script>

<template>
  <div class="result-section-stack">
    <SentinelRuntimeExploration
      :runtime-feature-rows="runtimeFeatureRows"
      :runtime-action-rows="runtimeActionRows"
      :observed-mutation-rows="observedMutationRows"
    />
    <section
      v-if="registrationRows.length"
      class="result-block registration-alert"
    >
      <div class="block-title">
        <ShieldAlert :size="17" />
        <div>
          <strong
            >{{ tr(`发现 ${registrationRows.length} 个注册 / 创建账户入口`, `${registrationRows.length} sign-up / account creation entry points found`) }}</strong
          ><small
            >{{ tr('这是前端无明显漏洞时应优先人工验证的高价值入口；Oviraptor 不会自动提交注册或创建账户。', 'High-value entry points to validate manually first when the frontend shows no obvious vulnerability. Oviraptor never submits a sign-up or creates an account on its own.') }}</small
          >
        </div>
      </div>
      <div class="registration-entry-list">
        <article
          v-for="item in registrationRows"
          :key="item.id"
        >
          <header>
            <span>{{ registrationData(item).title || tr("注册入口", "Sign-up entry") }}</span>
            <em>{{ registrationData(item).confidence || "unknown" }}</em>
            <b>{{ registrationData(item).sourceType || "candidate" }}</b>
          </header>
          <div class="long-value-cell">
            <code
              class="scroll-value"
              :title="registrationData(item).url"
              >{{ registrationData(item).url }}</code
            >
            <button
              class="icon-button compact"
              :title="tr('复制入口', 'Copy entry URL')"
              @click="copyText(registrationData(item).url)"
            >
              <ClipboardCopy :size="13" />
            </button>
          </div>
          <p>
            {{ registrationData(item).note }}
            <span v-if="registrationData(item).matchedTerms?.length">
              {{ tr(`· 命中：${registrationData(item).matchedTerms.join("、")}`, `· Matched: ${registrationData(item).matchedTerms.join(", ")}`) }}
            </span>
          </p>
        </article>
      </div>
    </section>
    <SentinelRequestHeaders
      :observed-request-header-rows="observedRequestHeaderRows"
      :declared-request-header-rows="declaredRequestHeaderRows"
      :possible-request-header-rows="possibleRequestHeaderRows"
      :request-header-intelligence="requestHeaderIntelligence"
      :header-display-value="headerDisplayValue"
    />
    <section v-if="realtimeEndpointRows.length" class="result-block realtime-endpoint-block">
      <div class="block-title">
        <Activity :size="17" />
        <div><strong>{{ tr('实时通信接口', 'Realtime endpoints') }}</strong><small>{{ tr('WebSocket / EventSource 握手及其生效请求头。', 'WebSocket / EventSource handshakes and their effective request headers.') }}</small></div>
      </div>
      <div class="realtime-endpoint-list">
        <article v-for="item in realtimeEndpointRows" :key="item.id">
          <b>{{ json(item.recordJson).transport || 'Realtime' }}</b>
          <code>{{ json(item.recordJson).url }}</code>
          <span>{{ tr(`HTTP ${json(item.recordJson).statusCode || '—'} · 动作 ${json(item.recordJson).actionId || 'initial'}`, `HTTP ${json(item.recordJson).statusCode || '—'} · Action ${json(item.recordJson).actionId || 'initial'}`) }}</span>
          <small>{{ tr(`请求头：${text(Object.keys(json(item.recordJson).requestHeaders || {})) || '未捕获'}`, `Request headers: ${text(Object.keys(json(item.recordJson).requestHeaders || {})) || 'not captured'}`) }}</small>
        </article>
      </div>
    </section>
    <section class="result-block kind-api api-explorer-block">
      <div class="block-title api-explorer-heading">
        <Code2 :size="16" />
        <div>
          <strong>API Explorer</strong>
          <small>{{ tr(`共 ${apiRows.length} 个接口 · 路径规范化展示；完整 URL、参数、响应和来源按条目展开。`, `${apiRows.length} APIs · paths shown normalized; expand an entry for the full URL, parameters, response and source.`) }}</small>
        </div>
        <div class="api-explorer-stats"><b>{{ apiRows.length }}</b><span>{{ tr('接口', 'APIs') }}</span><b>{{ new Set(apiRows.map((item) => String(json(item.recordJson).method || "GET").toUpperCase())).size }}</b><span>{{ tr('方法', 'Methods') }}</span></div>
      </div>
      <div v-if="apiRows.length" class="api-explorer">
        <div class="api-explorer-list">
          <article v-for="item in apiRows" :key="item.id" class="api-explorer-row" :class="{ expanded: expandedApiRows.includes(item.id) }">
            <button class="api-row-main" type="button" @click="toggleApiRow(item.id)">
              <b class="method-badge" :class="methodTone(json(item.recordJson).method)">{{ json(item.recordJson).method || "UNKNOWN" }}</b>
              <span class="api-path" :title="apiUrl(item)">{{ apiPath(item) }}</span>
              <span class="api-row-meta">{{ tr(`${apiQuery(item).length} 参数 · ${json(item.recordJson).statusCode || json(item.recordJson).status || "—"}`, `${apiQuery(item).length} params · ${json(item.recordJson).statusCode || json(item.recordJson).status || "—"}`) }}</span>
              <ChevronDown :size="15" class="api-row-chevron" />
            </button>
            <div class="api-row-actions">
              <button class="icon-button compact" :title="tr('复制完整 URL', 'Copy full URL')" @click.stop="copyText(apiUrl(item))"><ClipboardCopy :size="13" /></button>
            </div>
            <div v-if="expandedApiRows.includes(item.id)" class="api-row-detail">
              <div class="api-detail-grid">
                <div><span>{{ tr('完整 URL', 'Full URL') }}</span><code class="api-long-value">{{ apiUrl(item) }}</code></div>
                <div><span>{{ tr('方法 / 来源 / 置信度', 'Method / source / confidence') }}</span><p>{{ apiMethod(item) }} · {{ apiSourceSummary(item) }} · {{ apiRecord(item).confidence || "unknown" }}</p></div>
                <div><span>{{ tr('接口说明', 'Description') }}</span><p>{{ apiDescription(item) }}</p></div>
                <div><span>{{ tr('身份上下文', 'Identity context') }}</span><p>{{ apiIdentitySummary(item) }}</p></div>
                <div><span>{{ tr('请求参数', 'Request parameters') }}</span><p>{{ apiQuery(item).join(tr("、", ", ")) || tr("无", "None") }}</p></div>
                <div><span>{{ tr('响应字段', 'Response fields') }}</span><p>{{ apiResponseSummary(item) }}</p></div>
                <div><span>{{ tr('响应状态', 'Response status') }}</span><p>{{ tr(`HTTP ${apiRecord(item).statusCode || apiRecord(item).status || "未记录"} · ${apiRecord(item).captureStatus || "采集状态未记录"}`, `HTTP ${apiRecord(item).statusCode || apiRecord(item).status || "not recorded"} · ${apiRecord(item).captureStatus || "capture status not recorded"}`) }}</p></div>
              </div>
              <details><summary>{{ tr('请求头 / 请求体 / 发起位置', 'Request headers / body / initiator') }}</summary><pre>{{ JSON.stringify({ headers: apiRecord(item).requestHeaders || {}, body: apiRequestPayload(item), initiator: apiRecord(item).initiator || null }, null, 2) }}</pre></details>
              <details><summary>{{ tr('响应头 / 响应体', 'Response headers / body') }}</summary><pre>{{ JSON.stringify({ headers: apiResponseHeaders(item), body: apiRecord(item).decodedBody || apiRecord(item).responseBody || apiRecord(item).responsePreview || null }, null, 2) }}</pre></details>
              <details><summary>{{ tr('原始证据', 'Raw evidence') }}</summary><pre>{{ JSON.stringify(apiRecord(item), null, 2) }}</pre></details>
            </div>
          </article>
        </div>
      </div>
      <div v-else class="empty-inline">{{ tr('没有发现可信 API；运行期 Hook 建议会显示在请求头和实时通信区域。', 'No trusted APIs found. Runtime Hook suggestions appear in the request header and realtime sections.') }}</div>
    </section>
    <SentinelJsFiles :js-rows="jsRows" :copy-text="copyText" />
    <section class="result-block">
      <div class="block-title">
        <Activity :size="16" />
        <div>
          <strong>{{ tr('运行期信号', 'Runtime signals') }}</strong
          ><small
            >{{ tr('只记录静态证据提示；是否启动浏览器 Hook 由调查过程针对单个候选决定。', 'Static evidence hints only. Whether to start a browser Hook is decided per candidate during the investigation.') }}</small
          >
        </div>
      </div>
      <div class="runtime-signal-grid">
        <article v-for="item in runtimeRows" :key="item.id">
          <dl>
            <div>
              <dt>Single runtime Hook recommendation</dt>
              <dd>
                <strong>{{
                  json(item.recordJson).label ||
                  kindLabel(json(item.recordJson).type)
                }}</strong>
              </dd>
            </div>
            <div>
              <dt>URL</dt>
              <dd>
                <button
                  v-if="isHttpUrl(runtimeSignalUrl(item))"
                  class="runtime-url-link scroll-value"
                  :title="runtimeSignalUrl(item)"
                  @click="openTargetUrl(runtimeSignalUrl(item))"
                >
                  {{ runtimeSignalUrl(item) }}
                  <ExternalLink :size="12" />
                </button>
                <code
                  v-else
                  class="scroll-value"
                  :title="runtimeSignalUrl(item)"
                  >{{ runtimeSignalUrl(item) || "—" }}</code
                >
              </dd>
            </div>
            <div>
              <dt>{{ tr('展示信息', 'Details') }}</dt>
              <dd>
                <pre class="scroll-value scroll-value-pre">{{
                  json(item.recordJson).context ||
                  json(item.recordJson).evidence ||
                  json(item.recordJson).reason ||
                  "—"
                }}</pre>
              </dd>
            </div>
          </dl>
        </article>
        <div v-if="!runtimeRows.length" class="empty-inline">
          {{ tr('没有需要运行期采样的信号', 'No signals need runtime sampling') }}
        </div>
      </div>
    </section>
    <section class="result-block kind-route">
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>{{ tr('前端路由', 'Frontend routes') }}</strong
          ><small
            >{{ tr('只保留具有路由结构证据的有效路径；旧任务中的 SVG 属性和单字符结果会自动隐藏。', 'Only valid paths with route-structure evidence are kept; SVG attributes and single-character results from older tasks are hidden automatically.') }}</small
          >
        </div>
      </div>
      <div class="route-chip-list">
        <span
          v-for="item in routeRows"
          :key="item.id"
          class="route-record"
          ><code
            class="scroll-value"
            :title="json(item.recordJson).path"
            >{{ json(item.recordJson).path }}</code
          ><em>{{ json(item.recordJson).type || "route" }}</em
          ><small
            class="scroll-value"
            :title="json(item.recordJson).source"
            >{{ json(item.recordJson).source || "—" }}</small
          ></span
        >
        <div v-if="!routeRows.length" class="empty-inline">
          {{ tr('没有可信路由记录', 'No trusted routes recorded') }}
        </div>
      </div>
    </section>
    <section class="result-block kind-crypto-signal">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>{{ tr('加密方式', 'Cryptography') }}</strong
          ><small
            >{{ tr('由本地静态分析分类，仅用于展示，不会发送给原生 Agent。', 'Classified by local static analysis for display only; never sent to the native Agent.') }}</small
          >
        </div>
      </div>
      <div class="crypto-table">
        <div class="table-head">
          <span>{{ tr('类别', 'Category') }}</span><span>{{ tr('算法', 'Algorithm') }}</span><span>{{ tr('操作', 'Operation') }}</span
          ><span>{{ tr('来源', 'Source') }}</span><span>{{ tr('证据', 'Evidence') }}</span>
        </div>
        <div v-for="item in cryptoRows" :key="item.id">
          <span>{{
            cryptoCategory(json(item.recordJson).category)
          }}</span
          ><strong>{{ json(item.recordJson).algorithm }}</strong
          ><span>{{ json(item.recordJson).operation }}</span
          ><code
            class="scroll-value"
            :title="json(item.recordJson).source"
            >{{ json(item.recordJson).source || "—" }}</code
          ><code
            class="scroll-value"
            :title="
              json(item.recordJson).context ||
              json(item.recordJson).evidence
            "
            >{{
              json(item.recordJson).evidence ||
              json(item.recordJson).context ||
              "—"
            }}</code
          >
        </div>
        <div v-if="!cryptoRows.length" class="empty-inline">
          {{ tr('没有识别到本地加密算法调用', 'No local cryptographic calls detected') }}
        </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>{{ tr('敏感信息线索', 'Sensitive data leads') }}</strong
          ><small
            >{{ tr('仅匹配凭据或个人信息；普通 href、图片、CSS 和普通 URL 不再进入此处。点击查看原始值与 200 字符上下文。', 'Matches credentials and personal data only; plain hrefs, images, CSS and ordinary URLs are excluded. Click a row to see the raw value with 200 characters of context.') }}</small
          >
        </div>
      </div>
      <div class="sensitive-table">
        <div class="table-head">
          <span>{{ tr('等级', 'Severity') }}</span><span>{{ tr('类型', 'Type') }}</span><span>{{ tr('完整值', 'Full value') }}</span
          ><span>{{ tr('来源文件', 'Source file') }}</span><span>SHA-256</span>
        </div>
        <button
          v-for="item in sensitiveRows"
          :key="item.id"
          type="button"
          :class="{ expanded: expandedSensitive.includes(item.id) }"
          @click="toggleSensitive(item.id)"
        >
          <b
            :class="`severity-badge ${safeSeverity(item.severity)}`"
            >{{ severityLabel(item.severity) }}</b
          ><strong>{{
            sensitiveType(json(item.recordJson).type)
          }}</strong
          ><code>{{
            json(item.recordJson).value ||
            json(item.recordJson).maskedValue
          }}</code
          ><code>{{ json(item.recordJson).source }}</code
          ><code>{{ json(item.recordJson).sha256 }}</code>
          <div
            v-if="expandedSensitive.includes(item.id)"
            class="sensitive-context"
          >
            <span>{{ tr('原始上下文（最多 200 字符）', 'Raw context (up to 200 characters)') }}</span>
            <pre>{{
              json(item.recordJson).context ||
              tr("旧结果没有上下文，请重新扫描该 URL。", "Older results have no context; rescan this URL.")
            }}</pre>
            <small v-if="json(item.recordJson).scope"
              >{{ tr(`IP 类型：${json(item.recordJson).scope === "private" ? "内网" : "公网"}`, `IP type: ${json(item.recordJson).scope === "private" ? "private" : "public"}`) }}</small
            >
          </div>
        </button>
        <div v-if="!sensitiveRows.length" class="empty-inline">
          {{ tr('没有发现敏感信息线索', 'No sensitive data leads found') }}
        </div>
      </div>
    </section>
  </div>
</template>
