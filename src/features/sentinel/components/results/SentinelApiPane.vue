<script setup lang="ts">
import {
  Activity,
  ChevronDown,
  ClipboardCopy,
  Code2,
  ExternalLink,
  FileJson,
  Network,
  Shield,
  ShieldAlert,
} from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { SentinelFinding } from "../../../../types";
import {
  createSentinelLabels,
  cryptoCategory,
  formatNumber,
  isHttpUrl,
  json,
  kindLabel,
  methodTone,
  safeSeverity,
  scriptTone,
  sensitiveType,
  text,
} from "../../presentation";

const { severityLabel } = createSentinelLabels(useI18n().tr);

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
    <section class="result-block runtime-exploration-block">
      <div class="block-title">
        <Activity :size="17" />
        <div>
          <strong>自动探索轨迹</strong>
          <small
            >保留每个页面状态、触发动作及其新增请求；写操作只观察并中止，不自动提交。</small
          >
        </div>
        <div class="runtime-exploration-counts">
          <span>{{ runtimeFeatureRows.length }} 个状态</span>
          <span>{{ runtimeActionRows.length }} 次动作</span>
          <span>{{ observedMutationRows.length }} 个写请求</span>
        </div>
      </div>
      <div
        v-if="runtimeFeatureRows.length || runtimeActionRows.length"
        class="runtime-exploration-grid"
      >
        <article class="runtime-state-column">
          <header>页面与功能状态</header>
          <div
            v-for="item in runtimeFeatureRows"
            :key="item.id"
            class="runtime-trace-card"
          >
            <div>
              <b>{{ json(item.recordJson).stateId || item.title }}</b>
              <span>深度 {{ json(item.recordJson).depth ?? 0 }}</span>
            </div>
            <strong>{{ json(item.recordJson).title || "未命名页面" }}</strong>
            <code :title="json(item.recordJson).url">{{
              json(item.recordJson).url
            }}</code>
            <p v-if="json(item.recordJson).highValueLabels?.length">
              高价值功能：{{
                json(item.recordJson).highValueLabels.join("、")
              }}
            </p>
            <small>
              {{ json(item.recordJson).interactiveCount || 0 }} 个可触发控件 ·
              {{ json(item.recordJson).formCount || 0 }} 个表单
              <template v-if="json(item.recordJson).fieldNames?.length">
                · 字段 {{ json(item.recordJson).fieldNames.join("、") }}
              </template>
            </small>
          </div>
        </article>
        <article class="runtime-action-column">
          <header>触发动作与请求增量</header>
          <div
            v-for="item in runtimeActionRows"
            :key="item.id"
            class="runtime-trace-card"
          >
            <div>
              <b>{{ json(item.recordJson).id || item.title }}</b>
              <span
                :class="{
                  changed: json(item.recordJson).stateChanged,
                  failed: json(item.recordJson).outcome === 'error',
                }"
                >{{ json(item.recordJson).outcome || "observed" }}</span
              >
            </div>
            <strong>
              {{ json(item.recordJson).label || json(item.recordJson).role || "页面控件" }}
            </strong>
            <p>
              新增 {{ json(item.recordJson).requestCount || 0 }} 个请求 ·
              观察并拦截 {{ json(item.recordJson).blockedRequestCount || 0 }} 个写请求 ·
              {{ json(item.recordJson).durationMs || 0 }} ms
            </p>
            <code :title="json(item.recordJson).afterUrl">
              {{ json(item.recordJson).beforeUrl }}
              <template
                v-if="
                  json(item.recordJson).afterUrl &&
                  json(item.recordJson).afterUrl !== json(item.recordJson).beforeUrl
                "
              >
                → {{ json(item.recordJson).afterUrl }}
              </template>
            </code>
          </div>
        </article>
      </div>
      <div v-else class="empty-inline">
        当前是旧扫描记录或页面没有可触发控件；重新运行扫描后会生成轨迹。
      </div>
      <details
        v-if="observedMutationRows.length"
        class="runtime-mutation-details"
      >
        <summary>
          查看 {{ observedMutationRows.length }} 个被观察并中止的写请求
        </summary>
        <div>
          <article
            v-for="item in observedMutationRows"
            :key="item.id"
          >
            <b>{{ json(item.recordJson).method || "WRITE" }}</b>
            <code>{{ json(item.recordJson).url }}</code>
            <small>
              参数：{{
                text(
                  json(item.recordJson).bodyKeys ||
                    json(item.recordJson).queryKeys,
                ) || "未识别"
              }}
              · 来源动作 {{ json(item.recordJson).actionId || "—" }}
            </small>
            <pre v-if="json(item.recordJson).postData">{{
              json(item.recordJson).postData
            }}</pre>
          </article>
        </div>
      </details>
    </section>
    <section
      v-if="registrationRows.length"
      class="result-block registration-alert"
    >
      <div class="block-title">
        <ShieldAlert :size="17" />
        <div>
          <strong
            >发现 {{ registrationRows.length }} 个注册 / 创建账户入口</strong
          ><small
            >这是前端无明显漏洞时应优先人工验证的高价值入口；Oviraptor
            不会自动提交注册或创建账户。</small
          >
        </div>
      </div>
      <div class="registration-entry-list">
        <article
          v-for="item in registrationRows"
          :key="item.id"
        >
          <header>
            <span>{{ registrationData(item).title || "注册入口" }}</span>
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
              title="复制入口"
              @click="copyText(registrationData(item).url)"
            >
              <ClipboardCopy :size="13" />
            </button>
          </div>
          <p>
            {{ registrationData(item).note }}
            <span v-if="registrationData(item).matchedTerms?.length">
              · 命中：{{ registrationData(item).matchedTerms.join("、") }}
            </span>
          </p>
        </article>
      </div>
    </section>
    <section class="result-block request-header-intelligence">
      <div class="block-title">
        <Network :size="17" />
        <div>
          <strong>请求头情报</strong>
          <small>运行时生效值、JS 声明值和浏览器可能管理但尚未观察到的 Header 分层展示。</small>
        </div>
        <div class="runtime-exploration-counts">
          <span>已观察 {{ observedRequestHeaderRows.length }}</span>
          <span>仅声明 {{ declaredRequestHeaderRows.length }}</span>
          <span>ExtraInfo {{ requestHeaderIntelligence.summary?.extraInfoHeaderCount || 0 }}</span>
        </div>
      </div>
      <div
        v-if="observedRequestHeaderRows.length || declaredRequestHeaderRows.length || possibleRequestHeaderRows.length"
        class="request-header-grid"
      >
        <article>
          <header><b>运行时真实生效</b><span>可以作为复现依据</span></header>
          <div v-for="row in observedRequestHeaderRows" :key="`observed-${row.name}`" class="request-header-row">
            <div><code>{{ row.name }}</code><em v-if="row.sources?.includes('browser-extra-info')">隐藏补全</em></div>
            <p :title="headerDisplayValue(row)">{{ headerDisplayValue(row) }}</p>
            <small>{{ row.occurrences || 1 }} 次 · {{ text(row.sources) }}</small>
          </div>
          <div v-if="!observedRequestHeaderRows.length" class="empty-inline">没有捕获到 XHR/Fetch/WebSocket 请求头</div>
        </article>
        <article>
          <header><b>JS 明确声明</b><span>需要运行时确认</span></header>
          <div v-for="row in declaredRequestHeaderRows" :key="`declared-${row.name}`" class="request-header-row declared">
            <div><code>{{ row.name }}</code><em>待确认</em></div>
            <p :title="headerDisplayValue(row)">{{ headerDisplayValue(row) }}</p>
            <small>{{ text(row.sources) }}</small>
          </div>
          <div v-if="!declaredRequestHeaderRows.length" class="empty-inline">JS 中没有发现额外 Header 声明</div>
        </article>
        <article>
          <header><b>浏览器管理头</b><span>可能存在，不算证据</span></header>
          <div v-for="row in possibleRequestHeaderRows" :key="`possible-${row.name}`" class="request-header-row possible">
            <div><code>{{ row.name }}</code><em>可能</em></div>
            <p>{{ row.reason }}</p>
          </div>
        </article>
      </div>
      <div v-else class="empty-inline">
        当前是旧扫描记录；重新运行扫描后会从 CDP ExtraInfo 和业务 JS 生成请求头证据。
      </div>
    </section>
    <section v-if="realtimeEndpointRows.length" class="result-block realtime-endpoint-block">
      <div class="block-title">
        <Activity :size="17" />
        <div><strong>实时通信接口</strong><small>WebSocket / EventSource 握手及其生效请求头。</small></div>
      </div>
      <div class="realtime-endpoint-list">
        <article v-for="item in realtimeEndpointRows" :key="item.id">
          <b>{{ json(item.recordJson).transport || 'Realtime' }}</b>
          <code>{{ json(item.recordJson).url }}</code>
          <span>HTTP {{ json(item.recordJson).statusCode || '—' }} · 动作 {{ json(item.recordJson).actionId || 'initial' }}</span>
          <small>请求头：{{ text(Object.keys(json(item.recordJson).requestHeaders || {})) || '未捕获' }}</small>
        </article>
      </div>
    </section>
    <section class="result-block kind-api api-explorer-block">
      <div class="block-title api-explorer-heading">
        <Code2 :size="16" />
        <div>
          <strong>API Explorer</strong>
          <small>共 {{ apiRows.length }} 个接口 · 路径规范化展示；完整 URL、参数、响应和来源按条目展开。</small>
        </div>
        <div class="api-explorer-stats"><b>{{ apiRows.length }}</b><span>接口</span><b>{{ new Set(apiRows.map((item) => String(json(item.recordJson).method || "GET").toUpperCase())).size }}</b><span>方法</span></div>
      </div>
      <div v-if="apiRows.length" class="api-explorer">
        <div class="api-explorer-list">
          <article v-for="item in apiRows" :key="item.id" class="api-explorer-row" :class="{ expanded: expandedApiRows.includes(item.id) }">
            <button class="api-row-main" type="button" @click="toggleApiRow(item.id)">
              <b class="method-badge" :class="methodTone(json(item.recordJson).method)">{{ json(item.recordJson).method || "UNKNOWN" }}</b>
              <span class="api-path" :title="apiUrl(item)">{{ apiPath(item) }}</span>
              <span class="api-row-meta">{{ apiQuery(item).length }} 参数 · {{ json(item.recordJson).statusCode || json(item.recordJson).status || "—" }}</span>
              <ChevronDown :size="15" class="api-row-chevron" />
            </button>
            <div class="api-row-actions">
              <button class="icon-button compact" title="复制完整 URL" @click.stop="copyText(apiUrl(item))"><ClipboardCopy :size="13" /></button>
            </div>
            <div v-if="expandedApiRows.includes(item.id)" class="api-row-detail">
              <div class="api-detail-grid">
                <div><span>完整 URL</span><code class="api-long-value">{{ apiUrl(item) }}</code></div>
                <div><span>方法 / 来源 / 置信度</span><p>{{ apiMethod(item) }} · {{ apiSourceSummary(item) }} · {{ apiRecord(item).confidence || "unknown" }}</p></div>
                <div><span>接口说明</span><p>{{ apiDescription(item) }}</p></div>
                <div><span>身份上下文</span><p>{{ apiIdentitySummary(item) }}</p></div>
                <div><span>请求参数</span><p>{{ apiQuery(item).join("、") || "无" }}</p></div>
                <div><span>响应字段</span><p>{{ apiResponseSummary(item) }}</p></div>
                <div><span>响应状态</span><p>HTTP {{ apiRecord(item).statusCode || apiRecord(item).status || "未记录" }} · {{ apiRecord(item).captureStatus || "采集状态未记录" }}</p></div>
              </div>
              <details><summary>请求头 / 请求体 / 发起位置</summary><pre>{{ JSON.stringify({ headers: apiRecord(item).requestHeaders || {}, body: apiRequestPayload(item), initiator: apiRecord(item).initiator || null }, null, 2) }}</pre></details>
              <details><summary>响应头 / 响应体</summary><pre>{{ JSON.stringify({ headers: apiResponseHeaders(item), body: apiRecord(item).decodedBody || apiRecord(item).responseBody || apiRecord(item).responsePreview || null }, null, 2) }}</pre></details>
              <details><summary>原始证据</summary><pre>{{ JSON.stringify(apiRecord(item), null, 2) }}</pre></details>
            </div>
          </article>
        </div>
      </div>
      <div v-else class="empty-inline">没有发现可信 API；运行期 Hook 建议会显示在请求头和实时通信区域。</div>
    </section>
    <section class="result-block kind-js-file">
      <div class="block-title">
        <FileJson :size="16" />
        <div>
          <strong>JS 文件</strong
          ><small
            >业务包深度分析；runtime 只发现分包；vendor
            与公共依赖不进入 Strix。</small
          >
        </div>
      </div>
      <div class="js-file-list">
        <article
          v-for="item in jsRows"
          :key="item.id"
          :class="scriptTone(json(item.recordJson).type)"
        >
          <header>
            <span>{{ json(item.recordJson).type || "script" }}</span
            ><b>{{
              json(item.recordJson).statusCode ||
              json(item.recordJson).priority ||
              "info"
            }}</b>
          </header>
          <div class="long-value-cell">
            <code
              class="scroll-value"
              :title="json(item.recordJson).url"
              >{{ json(item.recordJson).url }}</code
            ><button
              class="icon-button compact"
              title="复制 JS 地址"
              @click="copyText(json(item.recordJson).url)"
            >
              <ClipboardCopy :size="13" />
            </button>
          </div>
          <p>
            {{ formatNumber(json(item.recordJson).size || 0) }}
            bytes ·
            {{
              json(item.recordJson).isMinified
                ? "已压缩"
                : "未压缩"
            }}<template v-if="json(item.recordJson).discoveredFrom">
              · 来源
              {{
                json(item.recordJson).discoveredFrom === "html"
                  ? "HTML"
                  : json(item.recordJson).discoveredFrom
              }}</template
            >
          </p>
          <div
            v-if="json(item.recordJson).analysis"
            class="js-analysis-tags"
          >
            <span
              :class="{
                active: json(item.recordJson).analysis
                  .sourceMapReference,
              }"
              >Source Map
              {{
                json(item.recordJson).analysis.sourceMapReference
                  ? "存在"
                  : "未发现"
              }}</span
            ><span
              :class="{
                active: json(item.recordJson).analysis.module,
              }"
              >ES Module
              {{
                json(item.recordJson).analysis.module ? "是" : "否"
              }}</span
            ><span
              v-if="json(item.recordJson).analysis.moduleCount"
              class="active"
              >模块
              {{ json(item.recordJson).analysis.moduleCount }}</span
            ><span
              v-if="json(item.recordJson).analysis.businessScore"
              class="active"
              >业务信号
              {{
                json(item.recordJson).analysis.businessScore
              }}</span
            ><span>{{
              json(item.recordJson).analysis.extractionEngine ||
              "inventory"
            }}</span>
          </div>
          <p v-if="json(item.recordJson).error" class="form-error">
            {{ json(item.recordJson).error }}
          </p>
        </article>
        <div v-if="!jsRows.length" class="empty-inline">
          没有 JS 分析记录
        </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Activity :size="16" />
        <div>
          <strong>运行期信号</strong
          ><small
            >只记录静态证据提示；是否启动浏览器 Hook 由调查过程
            针对单个候选决定。</small
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
              <dt>展示信息</dt>
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
          没有需要运行期采样的信号
        </div>
      </div>
    </section>
    <section class="result-block kind-route">
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>前端路由</strong
          ><small
            >只保留具有路由结构证据的有效路径；旧任务中的 SVG
            属性和单字符结果会自动隐藏。</small
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
          没有可信路由记录
        </div>
      </div>
    </section>
    <section class="result-block kind-crypto-signal">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>加密方式</strong
          ><small
            >由本地静态分析分类，仅用于展示，不会发送给
            Strix。</small
          >
        </div>
      </div>
      <div class="crypto-table">
        <div class="table-head">
          <span>类别</span><span>算法</span><span>操作</span
          ><span>来源</span><span>证据</span>
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
          没有识别到本地加密算法调用
        </div>
      </div>
    </section>
    <section class="result-block">
      <div class="block-title">
        <Shield :size="16" />
        <div>
          <strong>敏感信息线索</strong
          ><small
            >仅匹配凭据或个人信息；普通 href、图片、CSS 和普通 URL
            不再进入此处。点击查看原始值与 200 字符上下文。</small
          >
        </div>
      </div>
      <div class="sensitive-table">
        <div class="table-head">
          <span>等级</span><span>类型</span><span>完整值</span
          ><span>来源文件</span><span>SHA-256</span>
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
            <span>原始上下文（最多 200 字符）</span>
            <pre>{{
              json(item.recordJson).context ||
              "旧结果没有上下文，请重新扫描该 URL。"
            }}</pre>
            <small v-if="json(item.recordJson).scope"
              >IP 类型：{{
                json(item.recordJson).scope === "private"
                  ? "内网"
                  : "公网"
              }}</small
            >
          </div>
        </button>
        <div v-if="!sensitiveRows.length" class="empty-inline">
          没有发现敏感信息线索
        </div>
      </div>
    </section>
  </div>
</template>
