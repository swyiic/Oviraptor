<script setup lang="ts">
import { computed, ref, watch } from "vue";
import SourceReviewEvidence from "../SourceReviewEvidence.vue";
import {
  Activity,
  Bug,
  ClipboardCheck,
  Code2,
  Download,
  FileJson,
  Layers3,
  Network,
  Pause,
  Play,
  RefreshCw,
} from "@lucide/vue";
import { useI18n } from "../../../../i18n";
import type { AppSecScanResult, SentinelFinding, SentinelScan, SentinelScanAttempt, SentinelValidation } from "../../types";
import {
  attemptEndReason,
  attemptStageLabel,
  attemptTime,
  createSentinelLabels,
  formatNumber,
  scanSummary,
  scanTokenTotal,
  text,
  uncachedInput,
} from "../../presentation";

const { tr } = useI18n();
const {
  retryActionLabel,
  scanTypeLabel,
  severityLabel,
  statusLabel,
  verdictLabel,
} = createSentinelLabels(tr);
const emit = defineEmits<{
  "toggle-attempt-history": [];
  "select-finding": [id: number];
}>();
const props = defineProps<{
  selected: SentinelScan;
  detailBusy: boolean;
  scanControlBusy: string;
  scanAttempts: SentinelScanAttempt[];
  visibleScanAttempts: SentinelScanAttempt[];
  attemptModeLabel: (mode: string, attemptNumber: number) => string;
  showAttemptHistory: boolean;
  resultTab: string;
  selectedFindingId?: number;
  isGreyboxScan: boolean;
  isCicdScan: boolean;
  appsecResult?: AppSecScanResult;
  appsecVulnerabilities: any[];
  appsecSourceCounts: Record<string, any>;
  cicdBlockingFindings: any[];
  greyboxCorrelated: Record<string, any>;
  focusedSourceFindingRows: any[];
  sourceFindingRows: any[];
  sourceDependencies: any[];
  sourceFrameworks: any[];
  sourceInventory: Record<string, any>;
  sourceInventoryFinding?: SentinelFinding;
  sourceIssueGroups: any[];
  sourceLanguageRows: any[];
  sourceManifests: any[];
  sourceSeverityCounts: Record<string, any>;
  sourceStats: Record<string, any>;
  appsecSourcesFor: (item: any) => any[];
  authTypeLabel: (value: string) => string;
  ciProviderLabel: (value: string) => string;
  correlationParts: (item: any) => any;
  editValidation: (item: SentinelFinding, verdict?: string) => void;
  effectiveSeverity: (item: any) => string;
  exportProject: (...args: any[]) => any;
  gateStatusLabel: (value: string) => string;
  pauseScan: (...args: any[]) => any;
  rescan: (...args: any[]) => any;
  resumeScan: (...args: any[]) => any;
  sourceLocations: (item: any) => any[];
  /** Memoized record_json parse (keyed by finding id) — use instead of recordOf(item) to avoid reparsing per field per render. */
  recordOf: (item: any) => any;
  sourceTypeLabel: (value: string) => string;
  validationFor: (item: any) => SentinelValidation | undefined;
}>();
const preferredReviewAttempt = ref<number>();
const reviewAttempts = computed(() => [...new Set(props.scanAttempts
  .filter(row => row.scanId === props.selected.id && Number.isSafeInteger(row.attemptNumber) && row.attemptNumber > 0)
  .map(row => row.attemptNumber))].sort((a, b) => b - a));
const reviewAttempt = computed({
  get: () => preferredReviewAttempt.value !== undefined && reviewAttempts.value.includes(preferredReviewAttempt.value)
    ? preferredReviewAttempt.value : reviewAttempts.value[0],
  set: (value: number | undefined) => { preferredReviewAttempt.value = value; },
});
watch(() => props.selected.id, () => { preferredReviewAttempt.value = undefined; }, { flush: "sync" });

// Read the saved payload, not the parent's legacy zero-coerced projection.
// Unknown historical measurements must not look like measured empty files.
function savedCount(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}
function savedCountLabel(value: unknown): string {
  const count = savedCount(value);
  return count === undefined ? tr("未记录", "Not recorded") : formatNumber(count);
}
const savedLineStats = computed(() => {
  const raw = props.sourceInventory.lineStats;
  return raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
});
const incompleteLineStats = computed(() => ["physical", "code", "comments", "blank"]
  .some(key => savedCount(savedLineStats.value[key]) === undefined));
const skippedLargeFiles = computed(() => savedCount(savedLineStats.value.skippedLargeFiles));
</script>

<template>
  <header class="url-intelligence-head source-intelligence-head">
    <div>
      <span class="eyebrow">{{ scanTypeLabel(selected.scanType) }}</span>
      <h3>{{ selected.taskName || selected.projectName }}</h3>
      <p>
        {{ selected.sourcePath || tr("未提供源码路径", "No source path provided") }} ·
        {{ statusLabel(selected.status) }} · {{ selected.id }}
      </p>
      <small
        v-if="scanSummary(selected)"
        class="live-checkpoint"
        ><Activity :size="12" />
        {{ scanSummary(selected) }}</small
      >
    </div>
    <div class="hero-actions">
      <button
        v-if="
          selected.status === 'scanning' ||
          selected.status === 'pausing'
        "
        class="button warning compact"
        :disabled="scanControlBusy === selected.id"
        @click="pauseScan(selected)"
      >
        <Pause :size="14" />{{
          selected.status === "pausing"
            ? tr("正在停止", "Stopping")
            : tr("停止并保留", "Stop and keep")
        }}</button
      ><button
        v-else-if="selected.status === 'paused'"
        class="button secondary compact"
        :disabled="scanControlBusy === selected.id"
        @click="resumeScan(selected)"
      >
        <Play :size="14" />{{ tr("继续扫描", "Resume scan") }}</button
      ><button
        v-else-if="
          selected.status !== 'draft' &&
          selected.status !== 'pausing'
        "
        class="button ghost compact"
        :disabled="scanControlBusy === selected.id"
        @click="rescan(selected)"
      >
        <RefreshCw :size="14" />{{ retryActionLabel(selected) }}</button
      ><button class="button ghost compact" @click="exportProject">
        <Download :size="14" />{{ tr("导出项目包", "Export project package") }}
      </button>
    </div>
  </header>
  <div v-if="detailBusy" class="empty-state">{{ tr("正在解析源码结果…", "Parsing source results…") }}</div>
  <div v-else class="source-result-stack">
    <section class="result-block" :aria-label="tr('正式源码审查', 'Formal source review')">
      <label v-if="reviewAttempts.length">{{ tr("审查执行轮次", "Review attempt") }}
        <select v-model="reviewAttempt" :aria-label="tr('源码结果审查轮次', 'Source review attempt')">
          <option v-for="attempt in reviewAttempts" :key="attempt" :value="attempt">{{ tr(`第 ${attempt} 次执行`, `Attempt ${attempt}`) }}</option>
        </select>
      </label>
      <SourceReviewEvidence v-if="reviewAttempt !== undefined" :key="selected.id" :scan-id="selected.id" :attempt-number="reviewAttempt" />
      <p v-else>{{ tr("尚无可选择的执行轮次；下方历史资料不代表独立审查已确认。", "No attempt to select yet; the historical material below has not been confirmed by an independent review.") }}</p>
    </section>
    <details class="source-historical-results">
      <summary>{{ tr("历史 / 分析器参考资料（不代表所选轮次独立确认）", "Historical / analyzer reference (not an independent confirmation of the selected attempt)") }}</summary>
      <p>{{ tr("以下保留原有仓库概况、分析器记录及人工标记，可能跨执行轮次。它们与上方正式裁决分别展示；历史状态、人工复核或旧门禁字段不等于本轮独立 Reviewer 确认。项目包仍是历史资料导出；正式裁决请使用上方“导出本轮审查 JSON”。", "Below are the earlier repository overview, analyzer records, and manual marks, which may span attempts. They are shown separately from the formal verdict above; historical status, manual review, or old gate fields are not an independent Reviewer confirmation for this attempt. The project package is still a historical export; for the formal verdict, use “Export review JSON” above.") }}</p>
    <section class="source-scan-meta">
      <div>
        <span>{{ tr("扫描类型", "Scan type") }}</span
        ><strong>{{ scanTypeLabel(selected.scanType) }}</strong>
      </div>
      <div>
        <span>{{ tr("扫描状态", "Scan status") }}</span
        ><strong>{{ statusLabel(selected.status) }}</strong>
      </div>
      <div>
        <span>{{ tr("源码路径", "Source path") }}</span
        ><code>{{ selected.sourcePath || "—" }}</code>
      </div>
      <div>
        <span>{{ tr("扫描器 / 技能", "Scanner / skills") }}</span
        ><code>{{ selected.skillNames || tr("默认规则集", "Default rule set") }}</code>
      </div>
      <div v-if="isGreyboxScan">
        <span>{{ tr("联测范围", "Combined test scope") }}</span><strong>{{ tr("源码 + 运行期证据", "Source + runtime evidence") }}</strong>
      </div>
      <div v-if="isCicdScan">
        <span>{{ tr("质量门禁", "Quality gate") }}</span><strong>{{ tr("变更范围 / 阻断发现", "Changed scope / blocking findings") }}</strong>
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-architecture-block">
      <div class="block-title">
        <Layers3 :size="16" />
        <div>
          <strong>{{ tr("项目架构与技术栈", "Project architecture and tech stack") }}</strong>
          <small
            >{{ tr("只根据源码扩展名和仓库清单识别，不使用漏洞记录里的 ecosystem 字段推测语言。", "Identified only from source file extensions and repository manifests; the ecosystem field in vulnerability records is not used to infer languages.") }}</small
          >
        </div>
      </div>
      <template v-if="sourceInventoryFinding">
        <div class="source-architecture-grid">
          <article class="architecture-primary">
            <span>{{ tr("应用架构", "App architecture") }}</span>
            <strong>{{
              sourceInventory.architecture || "Source repository"
            }}</strong>
            <small
              >{{ tr(`${sourceStats.totalFiles} 个仓库文件 · ${sourceStats.codeFiles} 个可识别代码文件`, `${sourceStats.totalFiles} repository files · ${sourceStats.codeFiles} recognized code files`) }}</small
            >
          </article>
          <article
            v-for="framework in sourceFrameworks"
            :key="`${framework.layer}-${framework.name}`"
          >
            <span>{{ framework.layer }}</span>
            <strong>{{ framework.name }}</strong>
            <small>{{ tr(`清单特征：${framework.evidence}`, `Manifest signal: ${framework.evidence}`) }}</small>
          </article>
        </div>
        <div class="source-loc-grid">
          <article>
            <span>{{ tr("物理行", "Physical lines") }}</span>
            <strong>{{
              savedCountLabel(savedLineStats.physical)
            }}</strong>
          </article>
          <article class="loc-code">
            <span>{{ tr("有效代码", "Code lines") }}</span>
            <strong>{{
              savedCountLabel(savedLineStats.code)
            }}</strong>
          </article>
          <article>
            <span>{{ tr("注释行", "Comment lines") }}</span>
            <strong>{{
              savedCountLabel(savedLineStats.comments)
            }}</strong>
          </article>
          <article>
            <span>{{ tr("空行", "Blank lines") }}</span>
            <strong>{{
              savedCountLabel(savedLineStats.blank)
            }}</strong>
          </article>
        </div>
        <p class="source-loc-rule">
          {{ tr("显示保存的行数统计；查看历史记录不会重新读取当前源码目录。", "Showing saved line counts; viewing history does not re-read the current source directory.") }}
          <span v-if="incompleteLineStats">
            {{ tr("部分统计未记录或格式无效，不按零计算。如需更新，请创建新的授权扫描任务。", "Some counts were not recorded or are invalid, so they are not counted as zero. To update them, create a new authorized scan task.") }}
          </span>
          <span v-if="skippedLargeFiles !== undefined">
            {{ tr(`记录中跳过 ${skippedLargeFiles} 个超大源码文件。`, `The record skipped ${skippedLargeFiles} oversized source files.`) }}
          </span>
        </p>
        <div
          v-if="sourceManifests.length"
          class="source-manifest-list"
        >
          <span>{{ tr("识别证据", "Detection evidence") }}</span>
          <code
            v-for="manifest in sourceManifests"
            :key="manifest"
            >{{ manifest }}</code
          >
        </div>
        <div
          v-if="sourceLanguageRows.length"
          class="source-language-table"
        >
          <div class="table-head">
            <span>{{ tr("语言", "Language") }}</span><span>{{ tr("文件数", "Files") }}</span><span>{{ tr("有效代码", "Code lines") }}</span
            ><span>{{ tr("注释", "Comments") }}</span><span>{{ tr("空行", "Blank") }}</span><span>{{ tr("物理行", "Physical") }}</span
            ><span>{{ tr("代码体积", "Code size") }}</span><span>{{ tr("文件占比", "File share") }}</span>
          </div>
          <div
            v-for="language in sourceLanguageRows"
            :key="language.name"
          >
            <strong>{{ language.name }}</strong>
            <span>{{ formatNumber(language.files) }}</span>
            <span>{{ savedCountLabel(language.codeLines) }}</span>
            <span>{{ savedCountLabel(language.commentLines) }}</span>
            <span>{{ savedCountLabel(language.blankLines) }}</span>
            <span>{{ savedCountLabel(language.lines) }}</span>
            <span>{{ formatNumber(language.bytes) }} B</span>
            <span
              >{{ Number(language.percent || 0).toFixed(1) }}%</span
            >
          </div>
        </div>
      </template>
      <div v-else class="source-inventory-warning">
        {{ tr("没有可用的源码清单，通常表示原源码路径已不可读取。恢复该目录后重新打开任务即可本地补建，不会调用模型；也可以再次扫描生成新结果。", "No source inventory is available, which usually means the original source path can no longer be read. Restore that directory and reopen the task to rebuild it locally without calling the model, or scan again to produce new results.") }}
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-token-block">
      <div class="block-title">
        <Activity :size="16" />
        <div>
          <strong>{{ tr("模型与 Token 消耗", "Model and token usage") }}</strong
          ><small
            >{{ tr("缓存输入属于输入总计；新增输入和模型输出分别展示。", "Cached input is part of total input; new input and model output are shown separately.") }}</small
          >
        </div>
      </div>
      <div class="source-token-grid">
        <article>
          <span>{{ tr("模型请求", "Model requests") }}</span
          ><strong>{{ formatNumber(selected.llmRequests) }}</strong>
        </article>
        <article>
          <span>{{ tr("输入 Token", "Input tokens") }}</span
          ><strong>{{ formatNumber(selected.inputTokens) }}</strong>
        </article>
        <article>
          <span>{{ tr("输出 Token", "Output tokens") }}</span
          ><strong>{{
            formatNumber(selected.outputTokens)
          }}</strong>
        </article>
        <article>
          <span>{{ tr("缓存 Token", "Cached tokens") }}</span
          ><strong>{{
            formatNumber(selected.cachedTokens)
          }}</strong>
        </article>
        <article>
          <span>{{ tr("新增输入 Token", "New input tokens") }}</span
          ><strong>{{
            formatNumber(uncachedInput(selected))
          }}</strong>
        </article>
        <article>
          <span>{{ tr("输入 + 输出总计", "Input + output total") }}</span
          ><strong>{{ formatNumber(scanTokenTotal(selected)) }}</strong>
        </article>
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities' && scanAttempts.length" class="result-block attempt-ledger-block">
      <div class="block-title">
        <RefreshCw :size="16" />
        <div><strong>{{ tr("本次执行结果与增量成本", "Current attempt results and incremental cost") }}</strong><small>{{ tr("默认只显示最新一次最终状态；旧轮次不会再混入当前结果，需要审计时再展开。", "Only the latest final state is shown by default; older attempts are no longer mixed into the current results. Expand them when you need to audit.") }}</small></div>
        <button v-if="scanAttempts.length > 1" class="attempt-history-toggle" @click="emit('toggle-attempt-history')">{{showAttemptHistory ? tr('收起历史', 'Hide history') : tr(`查看历史 ${scanAttempts.length - 1} 次`, `Show history (${scanAttempts.length - 1})`)}}</button>
      </div>
      <div class="attempt-ledger">
        <article v-for="attempt in visibleScanAttempts" :key="attempt.attemptNumber" :class="[`attempt-${attempt.status}`, { current: attempt.attemptNumber === selected.attemptCount }]">
          <header><span>{{tr(`第 ${attempt.attemptNumber} 次 · ${attemptModeLabel(attempt.executionMode, attempt.attemptNumber)}`, `Attempt ${attempt.attemptNumber} · ${attemptModeLabel(attempt.executionMode, attempt.attemptNumber)}`)}}</span><b>{{attemptStageLabel(attempt.stage)}}</b><em class="status-chip" :class="attempt.status">{{statusLabel(attempt.status)}}</em></header>
          <p>{{attempt.checkpoint || tr('尚无阶段详情', 'No stage details yet')}}</p>
          <div class="attempt-cost"><span>{{tr('请求', 'Requests')}} <b>{{formatNumber(attempt.llmRequests)}}</b></span><span>{{tr('输入', 'Input')}} <b>{{formatNumber(attempt.inputTokens)}}</b></span><span>{{tr('缓存', 'Cached')}} <b>{{formatNumber(attempt.cachedTokens)}}</b></span><span>{{tr('输出', 'Output')}} <b>{{formatNumber(attempt.outputTokens)}}</b></span><span>{{tr('本次总计', 'Attempt total')}} <b>{{formatNumber(attempt.totalTokens)}}</b></span></div>
          <small>{{attemptTime(attempt)}}</small><code v-if="attempt.workDir" :title="attempt.workDir">{{attempt.workDir}}</code><mark v-if="attemptEndReason(attempt)">{{tr(`结束说明：${attemptEndReason(attempt)}`, `End reason: ${attemptEndReason(attempt)}`)}}</mark>
        </article>
      </div>
    </section>
    <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-summary-block">
      <div class="block-title">
        <Code2 :size="16" />
        <div>
          <strong>{{ tr("代码扫描概况", "Code scan overview") }}</strong
          ><small
            >{{ tr("参考 SAST、SCA、质量门禁和数据流分析结果标准化展示。", "Standardized view modeled on SAST, SCA, quality gate, and data flow analysis results.") }}</small
          >
        </div>
      </div>
      <div class="source-metric-grid">
        <article>
          <span>{{ tr("历史 / 分析器条目", "Historical / analyzer entries") }}</span
          ><strong>{{ sourceStats.findings }}</strong>
        </article>
        <article>
          <span>{{ tr("受影响文件", "Affected files") }}</span
          ><strong>{{ sourceStats.files }}</strong>
        </article>
        <article>
          <span>{{ tr("代码位置", "Code locations") }}</span
          ><strong>{{ sourceStats.locations }}</strong>
        </article>
        <article>
          <span>{{ tr("命中规则", "Matched rules") }}</span
          ><strong>{{ sourceStats.rules }}</strong>
        </article>
        <article>
          <span>{{ tr("仓库代码文件", "Repository code files") }}</span
          ><strong>{{ sourceStats.codeFiles }}</strong>
        </article>
      </div>
      <div class="source-severity-grid">
        <article
          v-for="item in sourceSeverityCounts"
          :key="item.severity"
          :class="`source-severity ${item.severity}`"
        >
          <span>{{ severityLabel(item.severity) }}</span
          ><strong>{{ item.count }}</strong>
        </article>
      </div>
    </section>
    <section class="result-block source-finding-index-block">
      <div class="block-title">
        <FileJson :size="16" />
        <div>
          <strong>{{ tr("发现索引", "Finding index") }}</strong
          ><small
            >{{ tr("先在索引选择一条发现，下方只展示当前发现的完整证据。", "Pick a finding in the index; only that finding's full evidence is shown below.") }}</small
          >
        </div>
      </div>
      <div class="source-finding-index">
        <div class="table-head">
          <span>{{ tr("等级", "Severity") }}</span><span>{{ tr("标题", "Title") }}</span
          ><span>{{ tr("引擎 / Rule ID", "Engine / Rule ID") }}</span><span>CWE / CVE</span
          ><span>{{ tr("文件", "File") }}</span><span>{{ tr("行号", "Line") }}</span>
        </div>
        <div
          v-for="item in sourceFindingRows"
          :key="`index-${item.id}`"
          :class="{ active: selectedFindingId === item.id }"
          role="button"
          tabindex="0"
          @click="emit('select-finding', item.id)"
          @keydown.enter="emit('select-finding', item.id)"
        >
          <span
            :class="`severity-badge ${effectiveSeverity(item)}`"
            >{{ severityLabel(effectiveSeverity(item)) }}</span
          >
          <strong>{{
            item.title ||
            recordOf(item).title ||
            item.recordKey
          }}</strong>
          <code
            >{{
              recordOf(item).engine ||
              recordOf(item).source ||
              "Agent"
            }}
            ·
            {{
              recordOf(item).rule_id ||
              recordOf(item).ruleId ||
              item.recordKey
            }}</code
          >
          <span
            >{{ recordOf(item).cwe || "—" }} /
            {{ recordOf(item).cve || "—" }}</span
          >
          <code>{{ sourceLocations(item)[0]?.file || "—" }}</code>
          <span>{{
            sourceLocations(item)[0]?.start_line ||
            sourceLocations(item)[0]?.startLine ||
            "—"
          }}</span>
        </div>
        <div v-if="!sourceFindingRows.length" class="empty-inline">
          {{ tr("没有可索引的安全发现。", "No security findings to index.") }}
        </div>
      </div>
    </section>
    <section class="result-block source-findings-block">
      <div class="block-title">
        <Bug :size="16" />
        <div>
          <strong>{{ tr("代码发现与审计证据", "Code findings and audit evidence") }}</strong
          ><small
            >{{ tr("仅展示结构化安全问题与漏洞证据；扫描总结和质量门禁摘要不作为问题加载。", "Only structured security issues and vulnerability evidence are shown; scan summaries and quality gate digests are not loaded as issues.") }}</small
          >
        </div>
      </div>
      <div
        v-if="sourceIssueGroups.length"
        class="source-issue-groups"
      >
        <article
          v-for="group in sourceIssueGroups"
          :key="group.name"
        >
          <span>{{ group.name }}</span>
          <strong>{{ group.count }}</strong>
          <small v-if="group.high">{{ tr(`${group.high} 个高风险`, `${group.high} high risk`) }}</small>
          <small v-else>{{ tr("无高风险项", "No high-risk items") }}</small>
        </article>
      </div>
      <div class="source-finding-list">
        <article
          v-for="item in focusedSourceFindingRows"
          :key="item.id"
          :class="`source-finding-card severity-border-${effectiveSeverity(item)}`"
        >
          <header>
            <span
              :class="`severity-badge ${effectiveSeverity(item)}`"
              >{{ severityLabel(effectiveSeverity(item)) }}</span
            >
            <div>
              <strong>{{
                item.title ||
                recordOf(item).title ||
                item.recordKey
              }}</strong
              ><small
                >{{
                  recordOf(item).rule_id ||
                  recordOf(item).ruleId ||
                  tr("未标注规则", "No rule")
                }}
                · {{ recordOf(item).cwe || tr("无 CWE", "No CWE") }} ·
                {{ recordOf(item).cve || tr("无 CVE", "No CVE") }} · {{ tr("置信度", "Confidence") }}
                {{ recordOf(item).confidence || "—" }}</small
              >
            </div>
            <span
              v-if="recordOf(item).status"
              class="finding-state"
              >{{ recordOf(item).status }}</span
            >
          </header>
          <div class="source-finding-grid">
            <div>
              <span>{{ tr("描述", "Description") }}</span>
              <p>
                {{
                  recordOf(item).description ||
                  recordOf(item).message ||
                  "—"
                }}
              </p>
            </div>
            <div>
              <span>{{ tr("技术分析 / 影响", "Technical analysis / impact") }}</span>
              <p>
                {{
                  recordOf(item).technical_analysis ||
                  recordOf(item).impact ||
                  recordOf(item).detail ||
                  "—"
                }}
              </p>
            </div>
            <div v-if="sourceLocations(item).length">
              <span>{{ tr("文件与行号", "File and lines") }}</span>
              <div
                v-for="location in sourceLocations(item)"
                :key="`${item.id}-${location.file}-${location.start_line}`"
                class="source-location"
              >
                <code>{{ location.file || "—" }}</code
                ><b
                  >L{{
                    location.start_line ||
                    location.startLine ||
                    "?"
                  }}<span
                    v-if="location.end_line || location.endLine"
                    >-{{
                      location.end_line || location.endLine
                    }}</span
                  ></b
                >
                <pre v-if="location.snippet">{{
                  location.snippet
                }}</pre>
              </div>
            </div>
            <div
              v-if="
                recordOf(item).data_flow ||
                recordOf(item).taint_flow ||
                recordOf(item).call_chain
              "
            >
              <span>{{ tr("数据流 / 调用链", "Data flow / call chain") }}</span>
              <pre>{{
                text(
                  recordOf(item).data_flow ||
                    recordOf(item).taint_flow ||
                    recordOf(item).call_chain,
                )
              }}</pre>
            </div>
            <div>
              <span>{{ tr("修复建议", "Remediation") }}</span>
              <p>
                {{
                  recordOf(item).recommendation ||
                  recordOf(item).remediation_steps ||
                  "—"
                }}
              </p>
            </div>
            <div
              v-if="
                recordOf(item).fix_before ||
                recordOf(item).fix_after
              "
            >
              <span>{{ tr("修复前 / 修复后", "Before / after fix") }}</span>
              <pre
                >{{
                  recordOf(item).fix_before || "—"
                }}\n\n→\n\n{{
                  recordOf(item).fix_after || "—"
                }}</pre>
            </div>
            <div
              v-if="
                recordOf(item).dependency_metadata ||
                recordOf(item).package
              "
            >
              <span>{{ tr("依赖信息", "Dependency info") }}</span>
              <pre>{{
                JSON.stringify(
                  recordOf(item).dependency_metadata ||
                    recordOf(item).package,
                  null,
                  2,
                )
              }}</pre>
            </div>
            <div
              v-if="
                recordOf(item).evidence ||
                recordOf(item).pocRequest
              "
            >
              <span>{{ tr("证据 / 验证", "Evidence / validation") }}</span>
              <pre>{{
                text(
                  recordOf(item).evidence ||
                    recordOf(item).pocRequest,
                )
              }}</pre>
            </div>
            <div v-if="recordOf(item).assumptions">
              <span>{{ tr("前提与限制", "Assumptions and limitations") }}</span>
              <p>{{ text(recordOf(item).assumptions) }}</p>
            </div>
          </div>
          <footer>
            <button
              class="button primary compact"
              @click="editValidation(item)"
            >
              <ClipboardCheck :size="13" />{{
                validationFor(item)
                  ? tr("修改验证结论", "Edit validation verdict")
                  : tr("开始人工验证", "Start manual validation")
              }}</button
            ><span
              v-if="validationFor(item)"
              class="validation-saved-note"
              >{{
                verdictLabel(validationFor(item)?.verdict || "")
              }}</span
            >
          </footer>
        </article>
        <div v-if="!sourceFindingRows.length" class="empty-inline">
          {{ tr("当前源码任务没有结构化发现。", "This source task has no structured findings.") }}
        </div>
      </div>
    </section>
    <section v-if="sourceDependencies.length" class="result-block">
      <div class="block-title">
        <Layers3 :size="16" />
        <div>
          <strong>{{ tr("依赖与供应链风险", "Dependencies and supply chain risk") }}</strong
          ><small
            >{{ tr("展示包生态、已安装版本、修复版本和 CVE/CVSS。", "Shows package ecosystem, installed version, fixed version, and CVE/CVSS.") }}</small
          >
        </div>
      </div>
      <div class="source-dependency-list">
        <article
          v-for="item in sourceDependencies"
          :key="`dep-${item.id}`"
        >
          <strong>{{
            recordOf(item).package_name ||
            recordOf(item).package ||
            recordOf(item).dependency_metadata?.name ||
            item.title
          }}</strong
          ><span
            >{{
              recordOf(item).dependency_metadata
                ?.ecosystem ||
              recordOf(item).package_ecosystem ||
              "—"
            }}
            ·
            {{
              recordOf(item).dependency_metadata
                ?.installed_version ||
              recordOf(item).installed_version ||
              "—"
            }}</span
          ><b
            >{{ recordOf(item).cve || tr("无 CVE", "No CVE") }} · CVSS
            {{ recordOf(item).cvss ?? "—" }}</b
          ><em
            >{{ tr("修复版本", "Fixed version") }}
            {{
              recordOf(item).dependency_metadata
                ?.fixed_version ||
              recordOf(item).fixed_version ||
              "—"
            }}</em
          >
        </article>
      </div>
    </section>
    <section
      v-if="isGreyboxScan"
      class="result-block source-runtime-block"
    >
      <div class="block-title">
        <Network :size="16" />
        <div>
          <strong>{{ tr("灰盒运行期关联", "Greybox runtime correlation") }}</strong
          ><small
            >{{ tr("统一漏洞关联源码位置、运行期端点、参数和验证来源。", "Unified vulnerabilities linked to source locations, runtime endpoints, parameters, and validation sources.") }}</small
          >
        </div>
      </div>
      <div class="greybox-overview">
        <article>
          <span>{{ tr("测试环境", "Test environment") }}</span>
          <strong>{{
            appsecResult?.context?.environment || tr("未记录", "Not recorded")
          }}</strong>
        </article>
        <article>
          <span>{{ tr("认证上下文", "Auth context") }}</span>
          <strong>{{
            authTypeLabel(appsecResult?.context?.authType || "none")
          }}</strong>
          <small>{{
            appsecResult?.context?.authenticated
              ? appsecResult?.context?.authProfileName ||
                tr("本次临时会话", "Temporary session for this run")
              : tr("未提供认证会话", "No auth session provided")
          }}</small>
        </article>
        <article>
          <span>{{ tr("统一漏洞", "Unified vulnerabilities") }}</span>
          <strong>{{ appsecVulnerabilities.length }}</strong>
        </article>
        <article class="correlated-count">
          <span>{{ tr("SAST + DAST 已关联", "SAST + DAST correlated") }}</span>
          <strong>{{ greyboxCorrelated.length }}</strong>
        </article>
        <article>
          <span>{{ tr("来源记录", "Source records") }}</span>
          <strong>{{ appsecResult?.sources.length }}</strong>
          <small
            >SAST {{ appsecSourceCounts.sast || 0 }} · DAST
            {{ appsecSourceCounts.dast || 0 }} · IAST
            {{ appsecSourceCounts.iast || 0 }} · SCA
            {{ appsecSourceCounts.sca || 0 }}</small
          >
        </article>
      </div>
      <div class="appsec-correlation-list">
        <article
          v-for="vulnerability in appsecVulnerabilities"
          :key="`appsec-${vulnerability.id}`"
          :class="`appsec-correlation-card severity-border-${vulnerability.severity}`"
        >
          <header>
            <span
              :class="`severity-badge ${vulnerability.severity}`"
              >{{ severityLabel(vulnerability.severity) }}</span
            >
            <div>
              <strong>{{ vulnerability.title }}</strong>
              <small
                >{{
                  vulnerability.vulnerabilityType || tr("未分类漏洞", "Uncategorized vulnerability")
                }}
                · {{ vulnerability.status || "open" }}</small
              >
            </div>
            <span class="correlation-score"
              >{{ tr(`${vulnerability.correlationScore}% 关联度`, `${vulnerability.correlationScore}% correlation`) }}</span
            >
          </header>
          <div class="appsec-link-grid">
            <div>
              <span>{{ tr("运行期端点", "Runtime endpoint") }}</span>
              <code
                >{{ vulnerability.httpMethod || "—" }}
                {{ vulnerability.url || "—" }}</code
              >
              <small
                >{{ tr(`参数：${vulnerability.parameter || "—"}`, `Parameter: ${vulnerability.parameter || "—"}`) }}</small
              >
            </div>
            <div>
              <span>{{ tr("代码位置", "Code location") }}</span>
              <code
                >{{ vulnerability.file || "—"
                }}<template v-if="vulnerability.startLine"
                  >:{{ vulnerability.startLine }}</template
                ></code
              >
              <small>{{ tr(`符号：${vulnerability.symbol || "—"}`, `Symbol: ${vulnerability.symbol || "—"}`) }}</small>
            </div>
          </div>
          <div class="appsec-source-list">
            <span
              v-for="source in appsecSourcesFor(vulnerability)"
              :key="source.id"
            >
              <b>{{ sourceTypeLabel(source.sourceType) }}</b>
              {{ source.engine || source.sourceKey }}
            </span>
          </div>
          <div
            v-if="vulnerability.correlation?.embeddedEvidence"
            class="embedded-correlation-note"
          >
            {{ tr("同一条已验证记录同时包含源码位置与运行期请求证据。", "The same validated record contains both the source location and runtime request evidence.") }}
          </div>
          <div v-else class="correlation-breakdown">
            <span
              v-for="part in correlationParts(vulnerability)"
              :key="part.key"
              :class="{ matched: part.matched }"
            >
              {{ part.label }}
              <b>{{
                part.matched ? `+${part.weight}` : tr("未匹配", "No match")
              }}</b>
            </span>
          </div>
        </article>
        <div
          v-if="!appsecVulnerabilities.length"
          class="empty-inline"
        >
          {{ tr("当前任务没有可统一的结构化漏洞记录。", "This task has no structured vulnerability records to unify.") }}
        </div>
      </div>
    </section>
    <section
      v-if="isCicdScan"
      class="result-block source-runtime-block"
    >
      <div class="block-title">
        <ClipboardCheck :size="16" />
        <div>
          <strong>{{ tr("CI/CD 质量门禁", "CI/CD quality gate") }}</strong
          ><small
            >{{ tr("流水线只负责触发与门禁，问题来源仍保留 SAST、SCA 和验证证据。", "The pipeline only triggers and gates; issues still keep their SAST, SCA, and validation evidence sources.") }}</small
          >
        </div>
      </div>
      <div class="cicd-context-grid">
        <div>
          <span>Provider</span
          ><strong>{{
            ciProviderLabel(appsecResult?.context?.ciProvider || "")
          }}</strong>
        </div>
        <div>
          <span>{{ tr("仓库", "Repository") }}</span
          ><code>{{
            appsecResult?.context?.repositoryUrl || tr("未记录", "Not recorded")
          }}</code>
        </div>
        <div>
          <span>{{ tr("分支", "Branch") }}</span
          ><code>{{
            appsecResult?.context?.branch || tr("未记录", "Not recorded")
          }}</code>
        </div>
        <div>
          <span>Commit</span
          ><code>{{
            appsecResult?.context?.commitSha || tr("未记录", "Not recorded")
          }}</code>
        </div>
        <div>
          <span>Build / Pipeline</span
          ><code>{{
            appsecResult?.context?.buildId || tr("未记录", "Not recorded")
          }}</code>
        </div>
        <div>
          <span>{{ tr("环境", "Environment") }}</span
          ><strong>{{
            appsecResult?.context?.environment || tr("未记录", "Not recorded")
          }}</strong>
        </div>
      </div>
      <div
        :class="`gate-status ${appsecResult?.context?.gateStatus || 'not_evaluated'}`"
      >
        <div>
          <span>{{ tr("发布门禁", "Release gate") }}</span>
          <strong>{{
            gateStatusLabel(
              appsecResult?.context?.gateStatus || "not_evaluated",
            )
          }}</strong>
          <small>{{
            appsecResult?.context?.gateReason ||
            tr("历史任务没有门禁上下文", "Historical task has no gate context")
          }}</small>
        </div>
        <dl>
          <div>
            <dt>Critical</dt>
            <dd>
              {{
                appsecVulnerabilities.filter(
                  (item) => item.severity === "critical",
                ).length
              }}
              / {{ appsecResult?.context?.policy?.maxCritical ?? 0 }}
            </dd>
          </div>
          <div>
            <dt>High</dt>
            <dd>
              {{
                appsecVulnerabilities.filter(
                  (item) => item.severity === "high",
                ).length
              }}
              / {{ appsecResult?.context?.policy?.maxHigh ?? 5 }}
            </dd>
          </div>
          <div>
            <dt>{{ tr("策略", "Policy") }}</dt>
            <dd>
              {{
                appsecResult?.context?.policy?.blockRelease
                  ? tr("超限阻断", "Block when exceeded")
                  : tr("仅告警", "Warn only")
              }}
            </dd>
          </div>
        </dl>
      </div>
      <div class="cicd-blocking-table">
        <div class="table-head">
          <span>{{ tr("等级", "Severity") }}</span><span>{{ tr("问题", "Issue") }}</span><span>{{ tr("位置 / 端点", "Location / endpoint") }}</span
          ><span>{{ tr("状态", "Status") }}</span><span>{{ tr("生命周期", "Lifecycle") }}</span><span>{{ tr("负责人", "Owner") }}</span>
        </div>
        <div
          v-for="vulnerability in cicdBlockingFindings"
          :key="`gate-${vulnerability.id}`"
        >
          <span
            :class="`severity-badge ${vulnerability.severity}`"
            >{{ severityLabel(vulnerability.severity) }}</span
          >
          <strong>{{ vulnerability.title }}</strong>
          <code>{{
            vulnerability.file
              ? `${vulnerability.file}${vulnerability.startLine ? `:${vulnerability.startLine}` : ""}`
              : vulnerability.url || "—"
          }}</code>
          <span>{{ vulnerability.status || "open" }}</span>
          <small
            >{{ vulnerability.firstSeen }}<br />{{
              vulnerability.lastSeen
            }}</small
          >
          <span>{{ vulnerability.owner || tr("未分配", "Unassigned") }}</span>
        </div>
        <div
          v-if="!cicdBlockingFindings.length"
          class="empty-inline"
        >
          {{ tr("此历史汇总中没有导致门禁超限的 Critical / High 条目；不代表所选轮次 CI 已通过。", "This historical summary has no Critical / High entries that exceed the gate; this does not mean CI passed for the selected attempt.") }}
        </div>
      </div>
    </section>
    </details>
  </div>
</template>

<style scoped>
.source-historical-results { min-width: 0; }
.source-historical-results > summary { cursor: pointer; padding: 12px 0; }
.source-historical-results > p { overflow-wrap: anywhere; }
</style>
