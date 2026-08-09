<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref, shallowRef, watch } from "vue";
import {
  Activity,
  Archive,
  Bug,
  CheckCircle2,
  ClipboardCheck,
  ClipboardCopy,
  Code2,
  Cpu,
  Download,
  ExternalLink,
  Eye,
  FileJson,
  Fingerprint,
  Globe2,
  HelpCircle,
  Layers3,
  Network,
  Pause,
  Play,
  RefreshCw,
  Save,
  Server,
  Shield,
  ShieldAlert,
  Trash2,
  Wrench,
  X,
} from "@lucide/vue";
import { api } from "../api";
import { useI18n } from "../i18n";
import { openUrl } from "@tauri-apps/plugin-opener";
import InlineConfirm from "./InlineConfirm.vue";
import StrixWorkbench from "./StrixWorkbench.vue";
import StrixTraceHub from "./StrixTraceHub.vue";
import SentinelValidationWorkbench from "../features/sentinel/components/SentinelValidationWorkbench.vue";
import SentinelTaskCenter from "../features/sentinel/components/SentinelTaskCenter.vue";
import SentinelAuthRecoveryPanel from "../features/sentinel/components/SentinelAuthRecoveryPanel.vue";
import InvestigationGraphPanel from "../features/sentinel/components/InvestigationGraphPanel.vue";
import {
  attemptEndReason,
  attemptStageLabel,
  attemptTime,
  createSentinelLabels,
  cryptoCategory,
  displayName,
  displayVersion,
  endpointUrl,
  formatCompactNumber,
  formatNumber,
  fuseVerdictLabel,
  isHttpUrl,
  json,
  kindLabel,
  kindTone,
  methodTone,
  routeModeLabel,
  safeSeverity,
  scanSummary,
  scanTokenTotal,
  scanTitle,
  scriptTone,
  sensitiveType,
  statusTone,
  text,
  uncachedInput,
  validRouteRecord,
  validSensitiveRecord,
  llmDeploymentClass,
} from "../features/sentinel/presentation";
import type {
  AppSecScanResult,
  AppSecVulnerability,
  BrowserAuthSession,
  InvestigationGraph,
  InvestigationHypothesis,
  InvestigationOverview,
  Project,
  SentinelCheckpoint,
  SentinelFinding,
  SentinelFuseEntry,
  SentinelOverviewStats,
  SentinelOpportunity,
  SentinelScan,
  SentinelScanAttempt,
  SentinelTarget,
  SentinelValidation,
  SentinelValidationWorkItem,
  StrixTraceDetail,
} from "../types";

const props = withDefaults(
  defineProps<{
    projects: Project[];
    projectId?: number;
    section?: Tab;
    resultView?: ResultTab;
    search?: string;
    workbenchMode?: WorkbenchMode;
    active?: boolean;
  }>(),
  {
    section: "overview",
    resultView: "summary",
    search: "",
    workbenchMode: "code",
    active: true,
  },
);
const emit = defineEmits<{
  notify: [type: "success" | "error" | "info", text: string];
  "section-change": [section: Tab];
  "projects-change": [];
  "create-project": [];
  "alerts-change": [alerts: { fuse: number; vulnerabilities: number }];
}>();
const { tr } = useI18n();
type Tab =
  | "overview"
  | "queue"
  | "results"
  | "fuse"
  | "validations"
  | "workbench"
  | "help";
type WorkbenchMode = "web" | "code" | "greybox" | "cicd" | "skills" | "traces";
type ResultTab =
  | "summary"
  | "investigation"
  | "opportunities"
  | "fingerprint"
  | "api"
  | "endpoints"
  | "vulnerabilities";

const tab = ref<Tab>(props.section);
const resultTab = ref<ResultTab>(props.resultView);
// These records are replaced as immutable snapshots. Shallow refs avoid
// creating thousands of nested Vue proxies for checkpoint/finding payloads.
const scans = shallowRef<SentinelScan[]>([]);
const vulnerabilityScanIds = shallowRef<string[]>([]);
const targets = shallowRef<SentinelTarget[]>([]);
const opportunities = shallowRef<SentinelOpportunity[]>([]);
const detailOpportunities = shallowRef<SentinelOpportunity[]>([]);
const stats = ref<SentinelOverviewStats>({
  taskCount: 0,
  urlCount: 0,
  fingerprintCount: 0,
  apiCount: 0,
  endpointCount: 0,
  vulnerabilityCount: 0,
  highRiskCount: 0,
  validatedCount: 0,
  pendingVulnerabilityCount: 0,
  vulnerableUrlCount: 0,
  activeFuseCount: 0,
  opportunityCount: 0,
  readyOpportunityCount: 0,
});
const investigationStats = ref<InvestigationOverview>({
  targetCount: 0,
  nodeCount: 0,
  edgeCount: 0,
  apiCount: 0,
  parameterCount: 0,
  hypothesisCount: 0,
  readyHypothesisCount: 0,
  identityDiffCount: 0,
  tokenWorthyCount: 0,
  averageInformationGain: 0,
  factCount: 0,
  promotedStrategyCount: 0,
});
const opportunityView = ref<"ready" | "all" | "history">("ready");
const opportunityBusy = ref(0);
const loading = ref(false);
const backgroundSyncing = ref(false);
const detailBusy = ref(false);
// Strix follows the single project scope selected in App.vue. Keeping a second
// selector here previously allowed every page to silently drift into a
// different project and made counts, fuse entries and validations disagree.
const projectFilter = ref<number | undefined>(props.projectId);
const selected = ref<SentinelScan>();
const scanAttempts = shallowRef<SentinelScanAttempt[]>([]);
const selectedUrl = ref("");
const previewScan = ref<SentinelScan>();
const previewUrls = ref<string[]>([]);
const checkpoints = shallowRef<SentinelCheckpoint[]>([]);
const findings = shallowRef<SentinelFinding[]>([]);
const validations = shallowRef<SentinelValidation[]>([]);
const previousFindings = shallowRef<SentinelFinding[]>([]);
const appsecResult = ref<AppSecScanResult>({
  vulnerabilities: [],
  sources: [],
});
const investigationGraph = shallowRef<InvestigationGraph>();
const investigationBusy = ref(false);
const investigationUpdatingId = ref<number>();
const validationWorkItems = shallowRef<SentinelValidationWorkItem[]>([]);
const validationWorkEditor = ref<SentinelValidationWorkItem>();
const fuseEntries = shallowRef<SentinelFuseEntry[]>([]);
const fuseFilter = ref("active");
const fuseCategoryFilter = ref("all");
const fuseEditor = ref<SentinelFuseEntry>();
const pendingFuseRemoval = ref<SentinelFuseEntry>();
const fuseBusy = ref(false);
const fuseForm = reactive({
  verdict: "pending",
  note: "",
  evidence: "",
  archived: false,
});
const validationFilter = ref("pending");
const validationEditor = ref<SentinelFinding>();
const selectedFindingId = ref<number>();
const validationForm = reactive({
  verdict: "needs_more",
  severity: "",
  note: "",
  evidence: "",
});
const validationWorkForm = reactive({
  verdict: "needs_more",
  severity: "medium",
  note: "",
  evidence: "",
});
const pendingDelete = ref<SentinelScan>();
const deleting = ref(false);
const scanControlBusy = ref("");
const authRecoveryScan = ref<SentinelScan>();
const authRecoverySessions = ref<BrowserAuthSession[]>([]);
const authRecoveryBusy = ref("");
const matchedScanIds = ref<string[]>([]);
const expandedSensitive = ref<number[]>([]);
const liveTrace = ref<StrixTraceDetail>();
const liveTraceBusy = ref(false);
type FuseDetailTab =
  "summary" | "fingerprint" | "assets" | "endpoints" | "proof";
type FuseDetailState = {
  open: boolean;
  loading: boolean;
  loaded: boolean;
  tab: FuseDetailTab;
  findings: SentinelFinding[];
  validations: SentinelValidation[];
};
const fuseDetailTabs: [FuseDetailTab, string][] = [
  ["summary", "概要"],
  ["fingerprint", "指纹配置"],
  ["assets", "JS / API"],
  ["endpoints", "端点验证"],
  ["proof", "漏洞证明"],
];
const fuseDetails = reactive<Record<number, FuseDetailState>>({});
const emptyFuseDetail: FuseDetailState = {
  open: false,
  loading: false,
  loaded: false,
  tab: "summary",
  findings: [],
  validations: [],
};

const {
  statusLabel,
  retryActionLabel,
  verdictLabel,
  severityLabel,
  scanTypeLabel,
  llmDeploymentLabel,
} =
  createSentinelLabels(tr);
function findingKey(item: SentinelFinding) {
  return `${item.stage}:${item.kind}:${item.recordKey}`;
}
const validationByFinding = computed(() => {
  const map=new Map<string,SentinelValidation>();
  for(const item of validations.value) map.set(`${item.url}|${item.findingKey}`,item);
  return map;
});
function validationFor(item: SentinelFinding) {
  return validationByFinding.value.get(`${item.targetUrl}|${findingKey(item)}`);
}
function effectiveSeverity(item: SentinelFinding) {
  const validation = validationFor(item);
  if (validation?.verdict === "false_positive") return "none";
  if (validation && validation.verdict !== "pending" && validation.severity)
    return safeSeverity(validation.severity);
  return safeSeverity(item.severity);
}
async function openTargetUrl(url: string) {
  if (!/^https?:\/\//i.test(url)) return;
  try {
    await openUrl(url);
  } catch (e) {
    emit("notify", "error", `无法打开浏览器：${String(e)}`);
  }
}
function apiUrl(item: SentinelFinding) {
  const data = json(item.recordJson);
  return data.url || endpointUrl(selectedUrl.value, data.path);
}
function registrationData(item: SentinelFinding) {
  const data = json(item.recordJson);
  return data.registration || data;
}
function isRegistrationApi(item: SentinelFinding) {
  return Boolean(json(item.recordJson).registration?.detected);
}
function runtimeSignalUrl(item: SentinelFinding) {
  const data = json(item.recordJson);
  return data.url || data.source || selectedUrl.value || "";
}
async function copyText(value: string) {
  try {
    await navigator.clipboard.writeText(value);
    emit("notify", "success", "已复制到剪贴板");
  } catch {
    emit("notify", "error", "复制失败");
  }
}
function toggleSensitive(id: number) {
  expandedSensitive.value = expandedSensitive.value.includes(id)
    ? expandedSensitive.value.filter((item) => item !== id)
    : [...expandedSensitive.value, id];
}
const tokenScope = ref<"all" | "cloud" | "local">("all");
const tokenScans = computed(() =>
  tokenScope.value === "all"
    ? scans.value
    : scans.value.filter((scan) => scan.llmDeployment === tokenScope.value),
);
function fuseReasonParts(item: SentinelFuseEntry) {
  const raw = fuseTarget(item)?.routingReason || item.reason || "";
  return String(raw)
    .split("；")
    .map((text) => text.trim())
    .filter(Boolean)
    .map((text) => {
      let tone = "general",
        label = "扫描信号";
      if (/HTTP|入口可访问|入口受限|入口响应/.test(text)) {
        tone = "access";
        label = "入口";
      } else if (/SourceMap/i.test(text)) {
        tone = "sourcemap";
        label = "SourceMap";
      } else if (/业务脚本|应用分包|JS/i.test(text)) {
        tone = "javascript";
        label = "JS";
      } else if (/识别到/.test(text)) {
        tone = "framework";
        label = "框架";
      } else if (/API/.test(text)) {
        tone = "api";
        label = "API";
      } else if (/路由/.test(text)) {
        tone = "route";
        label = "路由";
      } else if (/敏感|鉴权|管理|上传|业务入口/.test(text)) {
        tone = "sensitive";
        label = "敏感业务";
      } else if (/熔断|模型调用|Token|无进展|计划\/待办/.test(text)) {
        tone = "fuse";
        label = "熔断";
      }
      return { text, tone, label };
    });
}
function fuseReasonCategory(item: SentinelFuseEntry) {
  const reason = `${item.reason} ${fuseTarget(item)?.routingReason || ""}`.toLowerCase();
  if (/token|预算|budget|无进展|no.?progress|模型调用|计划\/待办/.test(reason))
    return "budget";
  if (/401|403|鉴权|登录|认证|unauthor|forbidden|access denied/.test(reason))
    return "access";
  if (/waf|拦截|验证码|captcha|限流|rate.?limit|封禁|anti.?bot/.test(reason))
    return "blocked";
  if (/timeout|超时|连接|network|dns|tls|证书|异常|error|failed/.test(reason))
    return "failure";
  return "low_value";
}
function fuseCategoryLabel(category: string) {
  return ({
    budget: "成本 / 无进展",
    access: "缺少访问条件",
    blocked: "遭到拦截",
    failure: "网络 / 执行异常",
    low_value: "价值不足",
  } as Record<string, string>)[category] || category;
}
function fuseRecommendedAction(item: SentinelFuseEntry) {
  return ({
    budget: "先看已保存情报；有明确接口或参数再恢复，避免继续空烧 Token。",
    access: "补充 Cookie、Token 或登录态后恢复重试。",
    blocked: "保持停止；确认访问策略或降低频率后再恢复。",
    failure: "确认网络、DNS 或证书状态，修复环境后直接重试。",
    low_value: "快速人工复核现有 JS/API 证据；无新增价值即可归档。",
  } as Record<string, string>)[fuseReasonCategory(item)];
}

const normalizedSearch = computed(() => props.search.trim().toLowerCase());
const searchableScanIds = computed(
  () =>
    new Set(
      targets.value
        .filter((target) =>
          `${target.company} ${target.url}`
            .toLowerCase()
            .includes(normalizedSearch.value),
        )
        .map((target) => target.scanId)
        .filter(Boolean),
    ),
);
const visibleScans = computed(() =>
  scans.value.filter(
    (scan) =>
      (!projectFilter.value || scan.projectId === projectFilter.value) &&
      (!normalizedSearch.value ||
        matchedScanIds.value.includes(scan.id) ||
        `${scan.projectName} ${scan.id}`
          .toLowerCase()
          .includes(normalizedSearch.value) ||
        searchableScanIds.value.has(scan.id)),
  ),
);
const taskGroups = computed(() => {
  const dates = new Map<
    string,
    { date: string; types: { type: string; scans: SentinelScan[] }[] }
  >();
  for (const scan of visibleScans.value) {
    const raw = scan.createdAt || scan.updatedAt || "";
    const date = raw ? raw.slice(0, 10) : "未标注日期";
    let group = dates.get(date);
    if (!group) {
      group = { date, types: [] };
      dates.set(date, group);
    }
    const type = scan.scanType || "web";
    let bucket = group.types.find((item) => item.type === type);
    if (!bucket) {
      bucket = { type, scans: [] };
      group.types.push(bucket);
    }
    bucket.scans.push(scan);
  }
  const order = ["web", "code", "greybox", "cicd"];
  return [...dates.values()]
    .sort((a, b) => b.date.localeCompare(a.date))
    .map((group) => ({
      ...group,
      types: group.types.sort(
        (a, b) => order.indexOf(a.type) - order.indexOf(b.type),
      ),
    }));
});
const queueScans = computed(() =>
  [...visibleScans.value].sort(
    (a, b) =>
      (({ draft: 0, queued: 1, scanning: 2, failed: 3, completed: 4 })[
        a.status
      ] ?? 5) -
      ({ draft: 0, queued: 1, scanning: 2, failed: 3, completed: 4 }[
        b.status
      ] ?? 5),
  ),
);
const resultTaskScans = computed(() =>
  resultTab.value === "vulnerabilities"
    ? visibleScans.value.filter((scan) => vulnerabilityScanIds.value.includes(scan.id))
    : visibleScans.value,
);
const scanTargets = computed(() =>
  selected.value
    ? targets.value.filter(
        (t) =>
          t.scanId === selected.value?.id &&
          (selected.value?.scanType !== "web" || isHttpUrl(t.url)),
      )
    : [],
);
const scanTargetByUrl = computed(() => new Map(scanTargets.value.map(item=>[item.url,item])));
const findingsByUrl = computed(() => {
  const map=new Map<string,SentinelFinding[]>();
  for(const finding of findings.value){
    const list=map.get(finding.targetUrl);
    if(list) list.push(finding); else map.set(finding.targetUrl,[finding]);
  }
  return map;
});
const targetUrls = computed(() => {
  const urls = [
    ...scanTargets.value.map((t) => t.url),
    ...findings.value.map((f) => f.targetUrl),
  ].filter(
    (v) =>
      v &&
      v !== "*" &&
      (selected.value?.scanType !== "web" || isHttpUrl(v)),
  );
  const unique = [...new Set(urls)];
  if (findings.value.some((f) => f.targetUrl === "*")) unique.push("*");
  return unique;
});
function companyForUrl(url: string) {
  return (
    scanTargetByUrl.value.get(url)?.company?.trim() ||
    "未提供公司"
  );
}
function targetForUrl(url: string) {
  return scanTargetByUrl.value.get(url);
}
const currentRows = computed(() => findingsByUrl.value.get(selectedUrl.value) || []);
const rows = (...kinds: string[]) =>
  currentRows.value.filter((f) => kinds.includes(f.kind));
const one = (kind: string) => {
  const item = currentRows.value.find((f) => f.kind === kind);
  return item ? json(item.recordJson) : undefined;
};
const urlCards = computed(() =>
  targetUrls.value.map((url) => {
    const list = findingsByUrl.value.get(url) || [];
    const target = targetForUrl(url);
    let fingerprints=0,apis=0,endpoints=0,vulnerabilities=0,pendingVulnerabilities=0,sensitive=0,high=0;
    for(const finding of list){
      if(["fingerprint","wordpress","tech_stack"].includes(finding.kind)) fingerprints++;
      if(["api","route","js_file"].includes(finding.kind)) apis++;
      if(finding.kind.includes("endpoint")||finding.kind==="directory_find") endpoints++;
      if(finding.kind==="sensitive_info"&&validSensitiveRecord(finding)) sensitive++;
      if(finding.kind==="vulnerability"){
        const severity=effectiveSeverity(finding);
        if(severity!=="none"){
          vulnerabilities++;
          const validation=validationFor(finding);
          if(!validation||validation.verdict==="pending") pendingVulnerabilities++;
          if(["critical","high"].includes(severity)) high++;
        }
      }
    }
    return {
      url,
      company: companyForUrl(url),
      status: target?.status || "",
      valueScore: target?.valueScore || 0,
      scanMode: target?.scanMode || "",
      scanCount: target?.scanCount || 0,
      routingReason: target?.routingReason || "",
      total: list.length,
      fingerprints,apis,endpoints,vulnerabilities,pendingVulnerabilities,sensitive,high,
    };
  }),
);
const filteredUrlCards = computed(() =>
  urlCards.value.filter((card) => {
    if (resultTab.value === "vulnerabilities" && card.vulnerabilities === 0)
      return false;
    return (
      !normalizedSearch.value ||
      `${card.company} ${card.url}`
        .toLowerCase()
        .includes(normalizedSearch.value)
    );
  }),
);
const companyGroups = computed(() => {
  const groups = new Map<string, typeof filteredUrlCards.value>();
  for (const card of filteredUrlCards.value) {
    const items = groups.get(card.company) || [];
    items.push(card);
    groups.set(card.company, items);
  }
  return [...groups.entries()].map(([company, urls]) => ({ company, urls }));
});
const previewTargetRows = computed(() =>
  previewUrls.value.map((url) => {
    const target = targets.value.find(
      (item) => item.scanId === previewScan.value?.id && item.url === url,
    );
    return {
      url,
      company: target?.company?.trim() || "未提供公司",
      highValue:
        (target?.valueScore || 0) >= 80 ||
        target?.scanMode === "deep" ||
        String(target?.routingReason || "").includes("高价值"),
    };
  }),
);
function scanHighValueCount(scanId: string) {
  return targets.value.filter(
    (target) =>
      target.scanId === scanId &&
      (target.valueScore >= 80 ||
        target.scanMode === "deep" ||
        target.routingReason.includes("高价值")),
  ).length;
}
const currentCard = computed(() =>
  urlCards.value.find((c) => c.url === selectedUrl.value),
);
const currentTarget = computed(() => targetForUrl(selectedUrl.value));
const fingerprint = computed(() => one("fingerprint") || {});
const wordpress = computed(() => one("wordpress") || {});
const techStack = computed(() => one("tech_stack") || {});
const fingerprintCards = computed(() => [
  {
    key: "frontend",
    label: "前端技术",
    data: fingerprint.value.frontend || techStack.value.framework || {},
  },
  { key: "backend", label: "后端框架", data: fingerprint.value.backend || {} },
  {
    key: "server",
    label: "Web 服务器",
    data: fingerprint.value.server || { name: techStack.value.server },
  },
  { key: "waf", label: "WAF", data: fingerprint.value.waf || {} },
  { key: "cdn", label: "CDN", data: fingerprint.value.cdn || {} },
]);
const securityHeaders = computed(() =>
  rows("security_header").map((item) => ({
    item,
    data: json(item.recordJson),
  })),
);
const requestHeaderIntelligence = computed<Record<string, any>>(() =>
  one("request_header_intelligence") || {},
);
const apiRows = computed(() => rows("api"));
const realtimeEndpointRows = computed(() => rows("realtime_endpoint"));
const observedRequestHeaderRows = computed(() =>
  Array.isArray(requestHeaderIntelligence.value.observed)
    ? requestHeaderIntelligence.value.observed
    : [],
);
const declaredRequestHeaderRows = computed(() =>
  Array.isArray(requestHeaderIntelligence.value.declared)
    ? requestHeaderIntelligence.value.declared
    : [],
);
const possibleRequestHeaderRows = computed(() =>
  Array.isArray(requestHeaderIntelligence.value.possibleBrowserManaged)
    ? requestHeaderIntelligence.value.possibleBrowserManaged
    : [],
);
function headerDisplayValue(row: Record<string, any>) {
  const values = Array.isArray(row?.values) ? row.values : [];
  return (
    values
      .slice(0, 3)
      .map((item: any) =>
        String(item?.value || (item?.dynamic ? "<dynamic>" : "")),
      )
      .filter(Boolean)
      .join(" / ") || "—"
  );
}
function apiRequestHeaderNames(item: SentinelFinding) {
  const data = json(item.recordJson);
  const names = [
    ...Object.keys(
      data.requestHeaders && typeof data.requestHeaders === "object"
        ? data.requestHeaders
        : {},
    ),
    ...(Array.isArray(data.requestHeaderNames)
      ? data.requestHeaderNames.map(String)
      : []),
    ...(Array.isArray(data.declaredHeaders)
      ? data.declaredHeaders.map((row: any) => String(row?.name || ""))
      : []),
  ].filter(Boolean);
  return [...new Set(names)].slice(0, 16);
}
const runtimeFeatureRows = computed(() => rows("runtime_feature"));
const runtimeActionRows = computed(() => rows("runtime_action"));
const observedMutationRows = computed(() => rows("observed_mutation"));
const registrationRows = computed(() => rows("registration_endpoint"));
const routeRows = computed(() => rows("route").filter(validRouteRecord));
const jsRows = computed(() => rows("js_file"));
const runtimeRows = computed(() =>
  rows("runtime_signal").filter(
    (item) =>
      !["cryptojs", "jsencrypt", "sm_crypto", "web_crypto"].includes(
        String(json(item.recordJson).type || ""),
      ),
  ),
);
const sensitiveRows = computed(() =>
  rows("sensitive_info").filter(validSensitiveRecord),
);
const cryptoRows = computed(() => rows("crypto_signal"));
const endpointRows = computed(() =>
  rows(
    "endpoint",
    "endpoint_expanded",
    "directory_find",
    "rest_endpoint",
    "login_endpoint",
  ),
);
const vulnerabilityRows = computed(() => rows("vulnerability"));
const pocRows = computed(() => rows("poc_test"));
const isGreyboxScan = computed(() => selected.value?.scanType === "greybox");
const isCicdScan = computed(() => selected.value?.scanType === "cicd");
const sourceInventoryFinding = computed(() =>
  findings.value.find((item) => item.kind === "source_inventory"),
);
const sourceInventory = computed<Record<string, any>>(() =>
  sourceInventoryFinding.value
    ? json(sourceInventoryFinding.value.recordJson)
    : {},
);
const sourceFindingRows = computed(() =>
  findings.value.filter((item) => {
    if (item.kind === "dependency") {
      const data = json(item.recordJson);
      return Boolean(
        data.cve ||
        data.cwe ||
        data.cvss ||
        data.advisory ||
        data.vulnerable === true ||
        data.dependency_metadata?.fixed_version ||
        data.fixed_version,
      );
    }
    return (
      [
        "vulnerability",
        "code_smell",
        "security_hotspot",
        "secret",
        "sast",
        "dast",
        "iast",
      ].includes(item.kind) || item.kind.includes("vulnerability")
    );
  }),
);
const focusedSourceFindingRows = computed(() => {
  const selectedRow = sourceFindingRows.value.find((item) => item.id === selectedFindingId.value);
  return selectedRow ? [selectedRow] : sourceFindingRows.value.slice(0, 1);
});
const focusedVulnerabilityRows = computed(() => {
  const selectedRow = vulnerabilityRows.value.find((item) => item.id === selectedFindingId.value);
  return selectedRow ? [selectedRow] : vulnerabilityRows.value.slice(0, 1);
});
function sourceLocations(item: SentinelFinding) {
  const data = json(item.recordJson);
  const nested = Array.isArray(data.code_locations)
    ? data.code_locations
    : data.code_location
      ? [data.code_location]
      : [];
  if (nested.length) return nested;
  if (data.file)
    return [
      {
        file: data.file,
        start_line: data.start_line ?? data.startLine,
        end_line: data.end_line ?? data.endLine,
        snippet: data.snippet,
      },
    ];
  return [];
}
const sourceLocationRows = computed(() =>
  sourceFindingRows.value.flatMap((item) =>
    sourceLocations(item).map((location: any) => ({
      item,
      data: json(item.recordJson),
      location,
    })),
  ),
);
const sourceSeverityCounts = computed(() =>
  ["critical", "high", "medium", "low", "info"].map((severity) => ({
    severity,
    count: sourceFindingRows.value.filter(
      (item) => effectiveSeverity(item) === severity,
    ).length,
  })),
);
const sourceLanguageRows = computed<any[]>(() =>
  Array.isArray(sourceInventory.value.languages)
    ? sourceInventory.value.languages
    : [],
);
const sourceLanguages = computed(() =>
  sourceLanguageRows.value.map((item) => String(item.name)).filter(Boolean),
);
const sourceFrameworks = computed<any[]>(() =>
  Array.isArray(sourceInventory.value.frameworks)
    ? sourceInventory.value.frameworks
    : [],
);
const sourceManifests = computed<string[]>(() =>
  Array.isArray(sourceInventory.value.manifests)
    ? sourceInventory.value.manifests
    : [],
);
const sourceLineStats = computed(() => ({
  physical: Number(sourceInventory.value.lineStats?.physical || 0),
  code: Number(sourceInventory.value.lineStats?.code || 0),
  comments: Number(sourceInventory.value.lineStats?.comments || 0),
  blank: Number(sourceInventory.value.lineStats?.blank || 0),
  skippedLargeFiles: Number(
    sourceInventory.value.lineStats?.skippedLargeFiles || 0,
  ),
}));
const appsecVulnerabilities = computed(
  () => appsecResult.value.vulnerabilities || [],
);
function appsecSourcesFor(vulnerability: AppSecVulnerability) {
  return (appsecResult.value.sources || []).filter(
    (source) => source.vulnerabilityId === vulnerability.id,
  );
}
function sourceTypeLabel(value: string) {
  return (
    (
      {
        sast: "SAST",
        dast: "DAST",
        sca: "SCA",
        iast: "IAST",
        ai_validation: "AI 验证",
        scanner: "扫描器",
      } as Record<string, string>
    )[value] || value.toUpperCase()
  );
}
function authTypeLabel(value: string) {
  return (
    (
      {
        none: "匿名",
        cookie: "Cookie 会话",
        bearer: "Bearer Token",
        header: "自定义 Header",
      } as Record<string, string>
    )[value] || value
  );
}
function ciProviderLabel(value: string) {
  return (
    (
      {
        github: "GitHub Actions",
        gitlab: "GitLab CI",
        jenkins: "Jenkins",
        azure: "Azure Pipelines",
        other: "Other",
      } as Record<string, string>
    )[value] ||
    value ||
    "未记录"
  );
}
function gateStatusLabel(value: string) {
  return (
    (
      {
        passed: "通过",
        warning: "超限告警",
        blocked: "阻断发布",
        not_evaluated: "未评估",
      } as Record<string, string>
    )[value] || value
  );
}
function correlationParts(vulnerability: AppSecVulnerability) {
  const correlation = vulnerability.correlation || {};
  return [
    { key: "type", label: "漏洞类型 / CWE", ...correlation.type },
    { key: "url", label: "URL", ...correlation.url },
    { key: "parameter", label: "参数", ...correlation.parameter },
    { key: "dataFlow", label: "数据流", ...correlation.dataFlow },
  ];
}
const appsecSourceCounts = computed(() => {
  const counts: Record<string, number> = {};
  for (const source of appsecResult.value.sources || []) {
    counts[source.sourceType] = (counts[source.sourceType] || 0) + 1;
  }
  return counts;
});
const greyboxCorrelated = computed(() =>
  appsecVulnerabilities.value.filter((vulnerability) => {
    const types = new Set(
      appsecSourcesFor(vulnerability).map((source) => source.sourceType),
    );
    return types.has("sast") && types.has("dast");
  }),
);
const sourceIssueGroups = computed(() => {
  const groups = new Map<
    string,
    { name: string; count: number; high: number }
  >();
  for (const vulnerability of appsecVulnerabilities.value) {
    const name =
      vulnerability.vulnerabilityType || vulnerability.title || "未分类问题";
    const current = groups.get(name) || { name, count: 0, high: 0 };
    current.count += 1;
    if (["critical", "high"].includes(vulnerability.severity))
      current.high += 1;
    groups.set(name, current);
  }
  return [...groups.values()].sort(
    (left, right) => right.high - left.high || right.count - left.count,
  );
});
const cicdBlockingFindings = computed(() => {
  const context = appsecResult.value.context;
  const maxCritical = Number(context?.policy?.maxCritical ?? 0);
  const maxHigh = Number(context?.policy?.maxHigh ?? 5);
  const critical = appsecVulnerabilities.value.filter(
    (item) => item.severity === "critical",
  );
  const high = appsecVulnerabilities.value.filter(
    (item) => item.severity === "high",
  );
  return [
    ...(critical.length > maxCritical ? critical : []),
    ...(high.length > maxHigh ? high : []),
  ];
});
const sourceDependencies = computed(() =>
  sourceFindingRows.value.filter((item) => {
    const data = json(item.recordJson);
    return (
      data.dependency_metadata ||
      data.package ||
      data.package_name ||
      data.installed_version ||
      data.cve
    );
  }),
);
const sourceStats = computed(() => ({
  findings: sourceFindingRows.value.length,
  files: new Set(
    sourceLocationRows.value.map((row) => row.location.file).filter(Boolean),
  ).size,
  locations: sourceLocationRows.value.length,
  rules: new Set(
    sourceFindingRows.value
      .map(
        (item) =>
          json(item.recordJson).rule_id ||
          json(item.recordJson).ruleId ||
          item.recordKey,
      )
      .filter(Boolean),
  ).size,
  totalFiles: Number(sourceInventory.value.totalFiles || 0),
  codeFiles: Number(sourceInventory.value.codeFiles || 0),
}));
const comparison = computed(() => {
  const oldMap = new Map(
    previousFindings.value.map((item) => [
      `${item.targetUrl}|${item.kind}|${item.recordKey}`,
      item,
    ]),
  );
  const newMap = new Map(
    findings.value.map((item) => [
      `${item.targetUrl}|${item.kind}|${item.recordKey}`,
      item,
    ]),
  );
  let added = 0,
    removed = 0,
    changed = 0,
    unchanged = 0;
  for (const [key, item] of newMap) {
    const old = oldMap.get(key);
    if (!old) added++;
    else if (
      old.recordJson !== item.recordJson ||
      old.severity !== item.severity
    )
      changed++;
    else unchanged++;
  }
  for (const key of oldMap.keys()) if (!newMap.has(key)) removed++;
  return { added, removed, changed, unchanged, total: newMap.size };
});
const selectedValidationWorkItems = computed(() =>
  validationWorkItems.value.filter((item) => {
    const pending = !item.validationId || ["pending", "needs_more"].includes(item.verdict);
    if (validationFilter.value === "pending" && !pending) return false;
    if (
      !["all", "pending"].includes(validationFilter.value) &&
      item.verdict !== validationFilter.value
    )
      return false;
    return (
      !normalizedSearch.value ||
      `${item.projectName} ${item.taskName} ${item.url} ${item.scanId} ${item.title}`
        .toLowerCase()
        .includes(normalizedSearch.value)
    );
  }),
);
const validationWorkStats = computed(() => ({
  pending: validationWorkItems.value.filter(
    (item) => !item.validationId || ["pending", "needs_more"].includes(item.verdict),
  ).length,
  confirmed: validationWorkItems.value.filter((item) => item.verdict === "true_positive").length,
  rejected: validationWorkItems.value.filter((item) => item.verdict === "false_positive").length,
}));
const visibleFuseEntries = computed(() =>
  fuseEntries.value.filter(
    (item) =>
      (fuseFilter.value === "all" ||
        (fuseFilter.value === "archived" ? item.archived : !item.archived)) &&
      (fuseCategoryFilter.value === "all" ||
        fuseReasonCategory(item) === fuseCategoryFilter.value) &&
      (!normalizedSearch.value ||
        `${item.company} ${item.url} ${item.reason} ${item.sourceScanId}`
          .toLowerCase()
          .includes(normalizedSearch.value)),
  ),
);
const routeReasonItems = computed(() =>
  String(currentTarget.value?.routingReason || "")
    .split("；")
    .map((item) => item.trim())
    .filter(Boolean),
);
const recentTraceEvents = computed(() =>
  [...(liveTrace.value?.events || [])]
    .filter(
      (event) =>
        selectedUrl.value === "*" ||
        !event.targetUrl ||
        normalizedTraceTarget(event.targetUrl) ===
          normalizedTraceTarget(selectedUrl.value),
    )
    .slice(-30)
    .reverse(),
);
const latestTraceEvent = computed(() => recentTraceEvents.value[0]);
function traceEventLabel(value: string) {
  return (
    (
      {
        function_call: "工具调用",
        function_call_output: "工具结果",
        reasoning: "分析阶段",
        message: "Agent 消息",
      } as Record<string, string>
    )[value] || value
  );
}
function traceEventTitle(event: StrixTraceDetail["events"][number]) {
  if (event.name) {
    return event.eventType === "function_call_output"
      ? `${event.name} 返回结果`
      : `调用 ${event.name}`;
  }
  return traceEventLabel(event.eventType);
}
function currentTraceStep() {
  const event = latestTraceEvent.value;
  if (!event) return "等待 Strix 写入第一条结构化事件";
  if (event.eventType === "function_call")
    return `正在执行 ${event.name || "验证工具"}`;
  if (event.eventType === "function_call_output")
    return `${event.name || "工具"} 已返回，模型正在判断是否获得新证据`;
  if (event.eventType === "reasoning") return "正在分析已有响应并选择下一步";
  return "正在整理当前阶段结论";
}
function traceSession(value: string) {
  return value ? value.slice(0, 8) : "root";
}
function normalizedTraceTarget(value: string) {
  try {
    const parsed = new URL(value);
    return `${parsed.protocol}//${parsed.host}${parsed.pathname.replace(/\/+$/, "")}`;
  } catch {
    return String(value || "")
      .trim()
      .replace(/\/+$/, "")
      .toLowerCase();
  }
}
function jumpToResult(next: ResultTab) {
  resultTab.value = next;
  window.requestAnimationFrame(() =>
    document
      .querySelector(".result-subtabs")
      ?.scrollIntoView({ behavior: "smooth", block: "start" }),
  );
}
async function loadSelectedTrace(scanId: string, notify = false) {
  liveTraceBusy.value = true;
  try {
    const trace = await api.getStrixTrace(scanId);
    if (selected.value?.id === scanId) liveTrace.value = trace;
  } catch (error) {
    if (selected.value?.id === scanId) liveTrace.value = undefined;
    if (notify) emit("notify", "error", `无法读取 Strix 执行链：${String(error)}`);
  } finally {
    liveTraceBusy.value = false;
  }
}
const overviewBars = computed(() => [
  {
    label: tr("指纹", "Technology"),
    value: stats.value.fingerprintCount,
    color: "#4f7cff",
  },
  { label: "API", value: stats.value.apiCount, color: "#2f9dd7" },
  {
    label: tr("端点", "Endpoints"),
    value: stats.value.endpointCount,
    color: "#18a77b",
  },
  {
    label: tr("漏洞", "Findings"),
    value: stats.value.vulnerabilityCount,
    color: "#e65b65",
  },
  {
    label: tr("已验证", "Verified"),
    value: stats.value.validatedCount,
    color: "#7957d5",
  },
]);
const overviewMax = computed(() =>
  Math.max(1, ...overviewBars.value.map((i) => i.value)),
);
const taskStatus = computed(() =>
  [
    "draft",
    "queued",
    "scanning",
    "pausing",
    "paused",
    "recon_only",
    "completed",
    "partial",
    "failed",
  ].map((status) => ({
    status,
    count: scans.value.filter((s) => s.status === status).length,
  })),
);
const totalInputTokenUsage = computed(() =>
  tokenScans.value.reduce((sum, scan) => sum + scan.inputTokens, 0),
);
const totalOutputTokenUsage = computed(() =>
  tokenScans.value.reduce((sum, scan) => sum + scan.outputTokens, 0),
);
const totalTokenUsage = computed(() =>
  tokenScans.value.reduce((sum, scan) => sum + scanTokenTotal(scan), 0),
);
const totalCachedTokenUsage = computed(() =>
  tokenScans.value.reduce((sum, scan) => sum + scan.cachedTokens, 0),
);
const totalUncachedInputUsage = computed(() =>
  tokenScans.value.reduce((sum, scan) => sum + uncachedInput(scan), 0),
);
const cacheHitRate = computed(() =>
  totalInputTokenUsage.value
    ? Math.round((totalCachedTokenUsage.value / totalInputTokenUsage.value) * 100)
    : 0,
);
const tokensPerVulnerability = computed(() =>
  stats.value.vulnerabilityCount
    ? Math.round(totalTokenUsage.value / stats.value.vulnerabilityCount)
    : 0,
);
const zeroYieldScans = computed(() =>
  tokenScans.value.filter(
    (scan) =>
      scanTokenTotal(scan) > 0 &&
      ["completed", "partial", "recon_only", "failed"].includes(scan.status) &&
      !vulnerabilityScanIds.value.includes(scan.id),
  ),
);
const zeroYieldTokenUsage = computed(() =>
  zeroYieldScans.value.reduce((sum, scan) => sum + scanTokenTotal(scan), 0),
);
const attentionTaskCount = computed(() =>
  scans.value.filter((scan) => ["draft", "paused", "partial", "failed"].includes(scan.status)).length,
);
const highestCostScan = computed(() =>
  [...tokenScans.value].sort((left, right) => scanTokenTotal(right) - scanTokenTotal(left))[0],
);
const totalRequestUsage = computed(() =>
  tokenScans.value.reduce((sum, scan) => sum + scan.llmRequests, 0),
);
const tokenTypeRows = computed(() =>
  ["web", "code", "greybox", "cicd"].map((type) => {
    const rows = tokenScans.value.filter(
      (scan) => (scan.scanType || "web") === type,
    );
    return {
      type,
      input: rows.reduce((sum, scan) => sum + scan.inputTokens, 0),
      cached: rows.reduce((sum, scan) => sum + scan.cachedTokens, 0),
      uncachedInput: rows.reduce((sum, scan) => sum + uncachedInput(scan), 0),
      output: rows.reduce((sum, scan) => sum + scan.outputTokens, 0),
      requests: rows.reduce((sum, scan) => sum + scan.llmRequests, 0),
      total: rows.reduce((sum, scan) => sum + scanTokenTotal(scan), 0),
    };
  }),
);
const activeOpportunityStatuses = new Set(["queued", "ready", "in_progress"]);
const isVerifiableOpportunity = (item: SentinelOpportunity) =>
  item.score >= 65 && ["ready", "in_progress"].includes(item.status);
const visibleOpportunities = computed(() =>
  opportunities.value.filter((item) => {
    if (
      opportunityView.value === "ready" &&
      !isVerifiableOpportunity(item)
    )
      return false;
    if (
      opportunityView.value === "history" &&
      !["validated", "dismissed", "exhausted"].includes(item.status)
    )
      return false;
    if (
      opportunityView.value === "all" &&
      !activeOpportunityStatuses.has(item.status)
    )
      return false;
    if (!normalizedSearch.value) return true;
    return `${item.title} ${item.targetUrl} ${item.category} ${JSON.stringify(item.record)}`
      .toLowerCase()
      .includes(normalizedSearch.value);
  }),
);
const activeOpportunityClues = computed(() =>
  opportunities.value
    .filter(
      (item) =>
        activeOpportunityStatuses.has(item.status) &&
        !isVerifiableOpportunity(item),
    )
    .sort((left, right) => right.score - left.score)
    .slice(0, 5),
);
const selectedUrlOpportunities = computed(() =>
  detailOpportunities.value.filter(
    (item) => !selectedUrl.value || sameTargetUrl(item.targetUrl, selectedUrl.value),
  ),
);
const evidenceNextAction = computed(() => {
  const pending = vulnerabilityRows.value.filter((item) => !validationFor(item)).length;
  if (pending)
    return { tone: "risk", label: `${pending} 个漏洞等待人工定性`, action: "vulnerabilities" };
  const ready = selectedUrlOpportunities.value.filter(isVerifiableOpportunity).length;
  if (ready)
    return { tone: "opportunity", label: `${ready} 个高价值机会可以直接验证`, action: "opportunities" };
  if (["limited", "fuse_excluded"].includes(currentTarget.value?.status || ""))
    return { tone: "stopped", label: "该 URL 已停止，查看原因并决定是否恢复", action: "fuse" };
  if (endpointRows.value.length)
    return { tone: "endpoint", label: `${endpointRows.value.length} 个端点已响应，优先分析参数与鉴权`, action: "endpoints" };
  return { tone: "collect", label: "尚无高价值证据，检查 JS/API 情报后再决定是否续跑", action: "api" };
});
function followEvidenceNextAction() {
  if (evidenceNextAction.value.action === "fuse") tab.value = "fuse";
  else resultTab.value = evidenceNextAction.value.action as ResultTab;
}
const runningScanCount = computed(
  () => scans.value.filter((item) => ["queued", "scanning", "pausing"].includes(item.status)).length,
);
const opportunityCategoryLabel = (value: string) =>
  (
    {
      privilege_surface: "权限与管理",
      identity_surface: "身份与账户",
      file_surface: "文件处理",
      api_contract: "接口契约",
      business_transaction: "业务交易",
      administration: "配置与审计",
      data_query: "数据查询",
      api_surface: "接口测试面",
      product_match: "产品知识匹配",
      frontend_feature: "前端功能",
      fallback_discovery: "兜底发现",
    } as Record<string, string>
  )[value] || value;
const opportunityStatusLabel = (value: string) =>
  (
    {
      queued: "待调度",
      ready: "可直接验证",
      in_progress: "调查中",
      validated: "已验证",
      dismissed: "已忽略",
      exhausted: "无新增证据",
    } as Record<string, string>
  )[value] || value;
function opportunityEndpoint(item: SentinelOpportunity) {
  return String(item.record?.endpoint || item.record?.route || item.targetUrl || "");
}
function opportunityParameters(item: SentinelOpportunity) {
  return Array.isArray(item.record?.parameters)
    ? item.record.parameters.map(String).filter(Boolean)
    : [];
}
function opportunityKnowledge(item: SentinelOpportunity) {
  return Array.isArray(item.record?.knowledgeMatches)
    ? item.record.knowledgeMatches
    : [];
}
function opportunityKnowledgeTitles(item: SentinelOpportunity) {
  return opportunityKnowledge(item)
    .slice(0, 3)
    .map((value: any) => String(value?.title || "未命名知识"))
    .join(" / ");
}
function opportunityEvidenceCount(item: SentinelOpportunity) {
  return Math.max(
    Array.isArray(item.evidence) ? item.evidence.length : 0,
    Array.isArray(item.record?.evidenceRefs)
      ? item.record.evidenceRefs.length
      : 0,
  );
}
const evidenceChainRows = computed(() => {
  type EvidenceRow = {
    key: string;
    method: string;
    url: string;
    sources: string[];
    parameters: string[];
    statusCode: string;
    verified: boolean;
    opportunityScore: number;
    vulnerabilities: number;
  };
  const result = new Map<string, EvidenceRow>();
  const add = (item: SentinelFinding, verified: boolean) => {
    const data = json(item.recordJson);
    const method = String(data.method || "GET").toUpperCase();
    const path = String(data.url || data.path || data.endpoint || "").trim();
    if (!path) return;
    const url = endpointUrl(selectedUrl.value, path);
    const key = `${method}|${url}`;
    const current = result.get(key) || {
      key,
      method,
      url,
      sources: [],
      parameters: [],
      statusCode: "",
      verified: false,
      opportunityScore: 0,
      vulnerabilities: 0,
    };
    current.sources = [...new Set([...current.sources, String(data.source || kindLabel(item.kind))])];
    const parameters = Array.isArray(data.parameters)
      ? data.parameters.map((value: any) => String(value?.name || value)).filter(Boolean)
      : data.parameters && typeof data.parameters === "object"
        ? Object.keys(data.parameters)
        : [];
    current.parameters = [...new Set([...current.parameters, ...parameters])].slice(0, 12);
    current.statusCode = String(data.statusCode || current.statusCode || "");
    current.verified ||= verified || Boolean(data.statusCode);
    result.set(key, current);
  };
  apiRows.value.forEach((item) => add(item, false));
  registrationRows.value.forEach((item) => add(item, false));
  endpointRows.value.forEach((item) => add(item, true));
  for (const row of result.values()) {
    const normalized = row.url.toLowerCase().replace(/\/$/, "");
    row.opportunityScore = Math.max(
      0,
      ...selectedUrlOpportunities.value
        .filter((item) => {
          const endpoint = opportunityEndpoint(item).toLowerCase().replace(/\/$/, "");
          return endpoint && (normalized.endsWith(endpoint) || endpoint.endsWith(normalized));
        })
        .map((item) => item.score),
    );
    row.vulnerabilities = vulnerabilityRows.value.filter((item) => {
      const data = json(item.recordJson);
      const target = String(data.url || data.endpoint || data.path || "").toLowerCase().replace(/\/$/, "");
      return target && (normalized.endsWith(target) || target.endsWith(normalized));
    }).length;
  }
  return [...result.values()]
    .sort((a, b) => b.vulnerabilities - a.vulnerabilities || b.opportunityScore - a.opportunityScore || Number(b.verified) - Number(a.verified))
    .slice(0, 30);
});
async function setOpportunityStatus(item: SentinelOpportunity, status: string) {
  opportunityBusy.value = item.id;
  try {
    await api.updateSentinelOpportunityStatus(item.id, status);
    const replace = (rows: SentinelOpportunity[]) =>
      rows.map((row) => (row.id === item.id ? { ...row, status } : row));
    opportunities.value = replace(opportunities.value);
    detailOpportunities.value = replace(detailOpportunities.value);
    stats.value = await api.sentinelOverviewStats(projectFilter.value);
  } catch (error) {
    emit("notify", "error", `机会状态更新失败：${String(error)}`);
  } finally {
    opportunityBusy.value = 0;
  }
}
async function openOpportunity(item: SentinelOpportunity, markInProgress = false) {
  if (markInProgress && item.status !== "in_progress")
    await setOpportunityStatus(item, "in_progress");
  const scan = scans.value.find((value) => value.id === item.scanId);
  if (!scan) {
    emit("notify", "error", "对应任务不在当前项目筛选范围内");
    return;
  }
  await openScan(scan);
  selectedUrl.value = item.targetUrl;
  resultTab.value = "opportunities";
}
const transferProjectId = computed(
  () => projectFilter.value || selected.value?.projectId,
);
let liveTimer: number | undefined;
let liveSyncing = false;
let initialSyncTimer: number | undefined;
let initialSyncDone = sessionStorage.getItem("oviraptor-sentinel-initial-sync") === "done";

function fuseState(item: SentinelFuseEntry) {
  return fuseDetails[item.id] || emptyFuseDetail;
}
function ensureFuseState(item: SentinelFuseEntry) {
  return (
    fuseDetails[item.id] ||
    (fuseDetails[item.id] = {
      open: false,
      loading: false,
      loaded: false,
      tab: "summary",
      findings: [],
      validations: [],
    })
  );
}
function sameTargetUrl(left: string, right: string) {
  const normalize = (value: string) => {
    try {
      const url = new URL(value);
      return `${url.protocol}//${url.host}${url.pathname.replace(/\/$/, "")}${url.search}`;
    } catch {
      return value.replace(/\/$/, "");
    }
  };
  return normalize(left) === normalize(right);
}
function fuseRows(item: SentinelFuseEntry, ...kinds: string[]) {
  return fuseState(item).findings.filter(
    (row) => sameTargetUrl(row.targetUrl, item.url) && kinds.includes(row.kind),
  );
}
function fuseTarget(item: SentinelFuseEntry) {
  return targets.value.find(
    (target) =>
      target.scanId === item.sourceScanId &&
      sameTargetUrl(target.url, item.url),
  );
}
function fuseValidationRows(item: SentinelFuseEntry) {
  return fuseState(item).validations.filter((row) =>
    sameTargetUrl(row.url, item.url),
  );
}
async function toggleFuseDetail(item: SentinelFuseEntry) {
  const state = ensureFuseState(item);
  state.open = !state.open;
  if (!state.open || state.loaded || state.loading) return;
  state.loading = true;
  try {
    [state.findings, state.validations] = await Promise.all([
      api.listSentinelFindings(item.sourceScanId),
      api.listSentinelValidations(item.sourceScanId),
    ]);
    state.loaded = true;
  } catch (e) {
    state.open = false;
    emit("notify", "error", `完整情报加载失败：${String(e)}`);
  } finally {
    state.loading = false;
  }
}

async function load() {
  loading.value = true;
  try {
    [
      scans.value,
      targets.value,
      stats.value,
      investigationStats.value,
      vulnerabilityScanIds.value,
      opportunities.value,
    ] = await Promise.all([
      api.listSentinelScans(projectFilter.value, 300),
      api.listSentinelTargets(projectFilter.value),
      api.sentinelOverviewStats(projectFilter.value),
      api.investigationOverview(projectFilter.value),
      api.listSentinelVulnerabilityScanIds(projectFilter.value),
      api.listSentinelOpportunities(projectFilter.value, undefined, undefined, 800),
    ]);
    if (tab.value === "fuse") {
      fuseEntries.value = await api.listSentinelFuseZone(projectFilter.value);
    }
    if (tab.value === "validations")
      validationWorkItems.value = await api.listSentinelValidationWorkItems(projectFilter.value);
    if (tab.value === "validations" && !validationWorkEditor.value) {
      const first = selectedValidationWorkItems.value[0];
      if (first) editValidationWorkItem(first);
    }
    if (tab.value === "queue" && !previewScan.value && queueScans.value[0])
      await preview(queueScans.value[0]);
    if (selected.value) {
      const fresh = scans.value.find((s) => s.id === selected.value?.id);
      if (fresh) await openScan(fresh, false);
    } else if (tab.value === "results" && resultTaskScans.value[0])
      await openScan(resultTaskScans.value[0], false);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    loading.value = false;
  }
}
async function initialBackgroundSync() {
  if (!props.active || initialSyncDone) return;
  initialSyncDone = true;
  backgroundSyncing.value = true;
  try {
    const changed = await api.syncSentinelResults();
    if (changed > 0) await load();
    sessionStorage.setItem("oviraptor-sentinel-initial-sync", "done");
  } catch (e) {
    initialSyncDone = false;
    emit("notify", "error", `Strix 后台同步失败：${String(e)}`);
  } finally {
    backgroundSyncing.value = false;
  }
}
function scheduleInitialBackgroundSync() {
  if (initialSyncDone || !props.active) return;
  if (initialSyncTimer !== undefined) window.clearTimeout(initialSyncTimer);
  // Let the page paint and become interactive before walking Strix artifacts.
  initialSyncTimer = window.setTimeout(initialBackgroundSync, 1200);
}
async function liveSync() {
  if (
    liveSyncing ||
    !props.active ||
    document.hidden ||
    !scans.value.some((scan) => ["scanning", "pausing"].includes(scan.status))
  )
    return;
  liveSyncing = true;
  try {
    await api.syncSentinelResults();
    const [
      nextScans,
      nextTargets,
      nextStats,
      nextInvestigationStats,
      nextVulnerabilityScanIds,
      nextOpportunities,
    ] = await Promise.all([
      api.listSentinelScans(projectFilter.value, 300),
      api.listSentinelTargets(projectFilter.value),
      api.sentinelOverviewStats(projectFilter.value),
      api.investigationOverview(projectFilter.value),
      api.listSentinelVulnerabilityScanIds(projectFilter.value),
      api.listSentinelOpportunities(projectFilter.value, undefined, undefined, 800),
    ]);
    scans.value = nextScans;
    targets.value = nextTargets;
    stats.value = nextStats;
    investigationStats.value = nextInvestigationStats;
    vulnerabilityScanIds.value = nextVulnerabilityScanIds;
    opportunities.value = nextOpportunities;
    if (selected.value) {
      const fresh = nextScans.find((scan) => scan.id === selected.value?.id);
      if (fresh) selected.value = fresh;
      [
        checkpoints.value,
        findings.value,
        validations.value,
        detailOpportunities.value,
        scanAttempts.value,
      ] =
        await Promise.all([
          api.listSentinelCheckpoints(selected.value.id),
          api.listSentinelFindings(selected.value.id),
          api.listSentinelValidations(selected.value.id),
          api.listSentinelOpportunities(undefined, selected.value.id, undefined, 800),
          api.listSentinelScanAttempts(selected.value.id),
        ]);
      if (selected.value.scanType !== "web") {
        appsecResult.value = await api.listAppSecScanResult(selected.value.id);
      }
      await loadSelectedTrace(selected.value.id);
    }
  } catch {
    /* 后台轮询失败时保留上次结果，手动同步会显示具体错误。 */
  } finally {
    liveSyncing = false;
  }
}
async function openScan(scan: SentinelScan, jump = true) {
  selected.value = scan;
  if (jump) {
    tab.value = "results";
    resultTab.value = "summary";
  }
  detailBusy.value = true;
  try {
    [
      checkpoints.value,
      findings.value,
      validations.value,
      previousFindings.value,
      detailOpportunities.value,
      scanAttempts.value,
    ] = await Promise.all([
      api.listSentinelCheckpoints(scan.id),
      api.listSentinelFindings(scan.id),
      api.listSentinelValidations(scan.id),
      scan.previousScanId
        ? api.listSentinelFindings(scan.previousScanId).catch(() => [])
        : Promise.resolve([]),
      api.listSentinelOpportunities(undefined, scan.id, undefined, 800),
      api.listSentinelScanAttempts(scan.id),
    ]);
    const selectableUrls =
      resultTab.value === "vulnerabilities"
        ? urlCards.value.filter((card) => card.vulnerabilities > 0).map((card) => card.url)
        : targetUrls.value;
    selectedUrl.value = selectableUrls.includes(selectedUrl.value)
      ? selectedUrl.value
      : selectableUrls[0] || "";
    selectedFindingId.value =
      (scan.scanType === "web" ? vulnerabilityRows.value[0] : sourceFindingRows.value[0])?.id;
    appsecResult.value =
      scan.scanType === "web"
        ? { vulnerabilities: [], sources: [] }
        : await api.listAppSecScanResult(scan.id);
    if (scan.scanType === "web" && selectedUrl.value && selectedUrl.value !== "*") {
      await loadInvestigationGraph(scan.id, selectedUrl.value);
    } else {
      investigationGraph.value = undefined;
    }
    await loadSelectedTrace(scan.id);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    detailBusy.value = false;
  }
}

async function loadInvestigationGraph(scanId?: string, url?: string) {
  if (!scanId || !url || url === "*") {
    investigationGraph.value = undefined;
    return;
  }
  investigationBusy.value = true;
  try {
    investigationGraph.value = await api.getInvestigationGraph(scanId, url);
  } catch (error) {
    investigationGraph.value = undefined;
    emit("notify", "error", `调查图谱加载失败：${String(error)}`);
  } finally {
    investigationBusy.value = false;
  }
}

async function updateInvestigationStatus(item: InvestigationHypothesis, status: string) {
  investigationUpdatingId.value = item.id;
  try {
    await api.updateInvestigationHypothesis(item.id, status);
    await loadInvestigationGraph(item.scanId, item.targetUrl);
    emit("notify", "success", status === "in_progress" ? "已进入有界验证队列" : "调查假设状态已更新");
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    investigationUpdatingId.value = undefined;
  }
}
async function updateInvestigationApproval(item: InvestigationHypothesis, approved: boolean) {
  investigationUpdatingId.value = item.id;
  try {
    await api.setInvestigationMutationApproval(item.id, approved, 1, 30);
    await loadInvestigationGraph(item.scanId, item.targetUrl);
    emit(
      "notify",
      "success",
      approved
        ? "已按当前端点授权 1 次状态变更，30 分钟后自动失效"
        : "状态变更授权已立即撤销",
    );
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    investigationUpdatingId.value = undefined;
  }
}
async function preview(scan: SentinelScan) {
  previewScan.value = scan;
  const own = targets.value
    .filter((t) => t.scanId === scan.id)
    .map((t) => t.url);
  if (own.length) {
    previewUrls.value = [...new Set(own)];
    return;
  }
  try {
    const list = await api.listSentinelFindings(scan.id);
    previewUrls.value = [
      ...new Set(list.map((f) => f.targetUrl).filter((u) => u && u !== "*")),
    ];
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
function refreshScanReference(scanId: string) {
  const fresh = scans.value.find((item) => item.id === scanId);
  if (!fresh) return;
  if (selected.value?.id === scanId) selected.value = fresh;
  if (previewScan.value?.id === scanId) previewScan.value = fresh;
}
async function confirm(scan: SentinelScan) {
  try {
    await api.confirmSentinelScan(scan.id);
    emit("notify", "success", "任务已确认，前端深度解析与 Strix 扫描已启动");
    await load();
    await preview(scans.value.find((s) => s.id === scan.id) || scan);
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
async function pauseScan(scan: SentinelScan) {
  scanControlBusy.value = scan.id;
  try {
    await api.pauseSentinelScan(scan.id);
    emit(
      "notify",
      "success",
      "暂停请求已接收；当前 URL 的前端解析与 Strix 测试正在立即停止，后续 URL 不再启动",
    );
    await load();
    refreshScanReference(scan.id);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    scanControlBusy.value = "";
  }
}
async function resumeScan(scan: SentinelScan) {
  scanControlBusy.value = scan.id;
  try {
    await api.resumeSentinelScan(scan.id);
    emit("notify", "success", "任务已恢复，将从下一个未完成 URL 继续");
    await load();
    refreshScanReference(scan.id);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    scanControlBusy.value = "";
  }
}
async function executeRescan(scan: SentinelScan) {
  if (scan.scanType && scan.scanType !== "web") {
    const next = await api.rescanStrixWorkbenchScan(scan.id);
    await load();
    await openScan(scans.value.find((item) => item.id === next.id) || next);
    emit(
      "notify",
      "success",
      "已在当前工作台任务中重新执行；运行产物按尝试隔离，累计成本继续保留",
    );
    return;
  }
  const prepared = await api.rescanSentinelScan(scan.id);
  const next = await api.confirmSentinelScan(prepared.id);
  await load();
  const fresh = scans.value.find((item) => item.id === next.id) || next;
  await openScan(fresh);
  emit(
    "notify",
    "success",
    "已在当前任务中继续执行；复用前端证据，只重试未完成阶段，不再创建或确认新任务",
  );
}
async function rescan(scan: SentinelScan) {
  scanControlBusy.value = scan.id;
  try {
    if (!scan.scanType || scan.scanType === "web") {
      const boundSessions = await api.listSentinelScanAuthSessions(scan.id);
      const sessions = await Promise.all(
        boundSessions.map((session) =>
          session.status === "valid"
            ? api.validateBrowserAuthSession(session.id)
            : Promise.resolve(session),
        ),
      );
      if (sessions.some((session) => session.status !== "valid")) {
        authRecoveryScan.value = scan;
        authRecoverySessions.value = sessions;
        emit("notify", "info", "登录会话已失效：请在当前任务内重新登录，绿灯后即可续扫");
        return;
      }
    }
    await executeRescan(scan);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    scanControlBusy.value = "";
  }
}
async function reloadAuthRecoverySessions() {
  if (!authRecoveryScan.value) return;
  authRecoverySessions.value = await api.listSentinelScanAuthSessions(authRecoveryScan.value.id);
}
async function reopenAuthRecovery(session: BrowserAuthSession) {
  if (!authRecoveryScan.value) return;
  authRecoveryBusy.value = session.id;
  try {
    await api.openBrowserAuthSession({
      id: session.id,
      projectId: session.projectId,
      name: session.name,
      entryUrl: session.entryUrl,
    });
    await reloadAuthRecoverySessions();
    emit("notify", "info", "登录窗口已打开；完成验证码/SSO 并进入后台功能后，回来点击“我已登录，保存会话”");
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
  }
}
async function finishAuthRecovery(session: BrowserAuthSession) {
  authRecoveryBusy.value = session.id;
  try {
    const updated = await api.finishBrowserAuthSession(session.id);
    await reloadAuthRecoverySessions();
    emit(
      "notify",
      updated.status === "valid" ? "success" : "info",
      updated.status === "valid" ? "登录会话已更新，绿灯恢复" : updated.lastError || "会话仍需确认",
    );
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
  }
}
async function validateAuthRecovery(session: BrowserAuthSession) {
  authRecoveryBusy.value = session.id;
  try {
    const updated = await api.validateBrowserAuthSession(session.id);
    await reloadAuthRecoverySessions();
    emit(
      "notify",
      updated.status === "valid" ? "success" : "info",
      updated.status === "valid" ? "会话校验通过，可以继续原任务" : updated.lastError || "会话需要重新登录",
    );
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
  }
}
async function continueAfterAuthRecovery() {
  const scan = authRecoveryScan.value;
  if (!scan) return;
  authRecoveryBusy.value = "continue";
  scanControlBusy.value = scan.id;
  try {
    await reloadAuthRecoverySessions();
    if (authRecoverySessions.value.some((session) => session.status !== "valid")) {
      emit("notify", "info", "仍有绑定身份未恢复绿色状态，请先完成登录");
      return;
    }
    await executeRescan(scan);
    authRecoveryScan.value = undefined;
    authRecoverySessions.value = [];
  } catch (error) {
    emit("notify", "error", String(error));
  } finally {
    authRecoveryBusy.value = "";
    scanControlBusy.value = "";
  }
}
function editFuse(item: SentinelFuseEntry) {
  fuseEditor.value = item;
  Object.assign(fuseForm, {
    verdict: item.verdict || "pending",
    note: item.note || "",
    evidence: item.evidence || "",
    archived: item.archived,
  });
}
async function saveFuse(archive?: boolean) {
  if (!fuseEditor.value) return;
  fuseBusy.value = true;
  try {
    if (typeof archive === "boolean") fuseForm.archived = archive;
    await api.saveSentinelFuseReview({ id: fuseEditor.value.id, ...fuseForm });
    fuseEditor.value = undefined;
    await load();
    emit(
      "notify",
      "success",
      fuseForm.archived ? "停止记录已完成处置并归档" : "URL 处置记录已保存",
    );
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    fuseBusy.value = false;
  }
}
async function removeFuse() {
  if (!pendingFuseRemoval.value) return;
  fuseBusy.value = true;
  try {
    const retry = await api.removeSentinelFuseEntry(pendingFuseRemoval.value.id);
    pendingFuseRemoval.value = undefined;
    await load();
    emit("notify", "success", `URL 已移出熔断区并进入自动重试任务 ${retry.id}`);
  } catch (e) {
    emit("notify", "error", String(e));
  } finally {
    fuseBusy.value = false;
  }
}
function askRemove(scan: SentinelScan) {
  pendingDelete.value = scan;
}
async function remove() {
  const scan = pendingDelete.value;
  if (!scan) return;
  deleting.value = true;
  try {
    await api.deleteSentinelScan(scan.id);
    if (selected.value?.id === scan.id) {
      selected.value = undefined;
      findings.value = [];
      selectedUrl.value = "";
      liveTrace.value = undefined;
    }
    if (previewScan.value?.id === scan.id) previewScan.value = undefined;
    pendingDelete.value = undefined;
    emit("notify", "success", "Strix 任务及关联记录已删除");
    await load();
  } catch (e) {
    emit("notify", "error", `删除失败：${String(e)}`);
  } finally {
    deleting.value = false;
  }
}
async function exportProject() {
  if (!transferProjectId.value) {
    emit(
      "notify",
      "info",
      "请先在顶部选择要导出的项目，或打开该项目的一条扫描任务",
    );
    return;
  }
  try {
    emit(
      "notify",
      "success",
      `项目包已导出：${await api.exportSentinelProject(transferProjectId.value)}`,
    );
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
function editValidation(item: SentinelFinding, verdict?: string) {
  validationEditor.value = item;
  const old = validationFor(item);
  validationForm.verdict = verdict || old?.verdict || "needs_more";
  validationForm.severity = old?.severity || safeSeverity(item.severity);
  validationForm.note = old?.note || "";
  validationForm.evidence = old?.evidence || "";
}
async function saveValidation() {
  const item = validationEditor.value;
  if (!selected.value || !item) return;
  try {
    await api.saveSentinelValidation({
      scanId: selected.value.id,
      url: item.targetUrl,
      findingKey: findingKey(item),
      findingKind: item.kind,
      ...validationForm,
    });
    [validations.value, stats.value] = await Promise.all([
      api.listSentinelValidations(selected.value.id),
      api.sentinelOverviewStats(projectFilter.value),
    ]);
    if (tab.value === "validations")
      validationWorkItems.value = await api.listSentinelValidationWorkItems(projectFilter.value);
    validationEditor.value = undefined;
    emit(
      "notify",
      "success",
      `验证结论已保存：${verdictLabel(validationForm.verdict)}，风险等级已更新为 ${severityLabel(validationForm.verdict === "false_positive" ? "none" : validationForm.severity)}`,
    );
  } catch (e) {
    emit("notify", "error", String(e));
  }
}
function editValidationWorkItem(item: SentinelValidationWorkItem) {
  validationWorkEditor.value = item;
  Object.assign(validationWorkForm, {
    verdict: item.validationId ? item.verdict : "needs_more",
    severity: item.confirmedSeverity || safeSeverity(item.originalSeverity),
    note: item.note || "",
    evidence: item.evidence || "",
  });
}
async function saveValidationWorkItem() {
  const item = validationWorkEditor.value;
  if (!item) return;
  try {
    await api.saveSentinelValidation({
      scanId: item.scanId,
      url: item.url,
      findingKey: item.findingKey,
      findingKind: item.findingKind,
      ...validationWorkForm,
    });
    [validationWorkItems.value, stats.value] = await Promise.all([
      api.listSentinelValidationWorkItems(projectFilter.value),
      api.sentinelOverviewStats(projectFilter.value),
    ]);
    const fresh = validationWorkItems.value.find((row) => row.findingId === item.findingId);
    validationWorkEditor.value = fresh;
    emit("notify", "success", `漏洞结论已保存：${verdictLabel(validationWorkForm.verdict)}`);
  } catch (error) {
    emit("notify", "error", String(error));
  }
}
async function openValidationEvidence(item: SentinelValidationWorkItem) {
  const scan = scans.value.find((row) => row.id === item.scanId);
  if (!scan) {
    emit("notify", "error", "对应任务不在当前项目范围内");
    return;
  }
  await openScan(scan, false);
  selectedUrl.value = item.url;
  resultTab.value = "vulnerabilities";
  tab.value = "results";
}
watch(
  () => props.section,
  (value) => {
    tab.value = value;
    if (value === "results" && !selected.value && resultTaskScans.value[0])
      openScan(resultTaskScans.value[0], false);
  },
);
watch(
  () => props.resultView,
  (value) => {
    resultTab.value = value;
    if (props.section === "results") tab.value = "results";
    if (
      value === "vulnerabilities" &&
      (!selected.value || !vulnerabilityScanIds.value.includes(selected.value.id)) &&
      resultTaskScans.value[0]
    )
      openScan(resultTaskScans.value[0], false);
  },
);
watch(selectedUrl, (value) => {
  if (resultTab.value === "vulnerabilities")
    selectedFindingId.value = vulnerabilityRows.value[0]?.id;
  if (selected.value?.scanType === "web")
    loadInvestigationGraph(selected.value.id, value);
});
watch(resultTab, (value) => {
  if (value === "vulnerabilities")
    selectedFindingId.value = vulnerabilityRows.value[0]?.id;
});
watch(validationFilter, () => {
  const first = selectedValidationWorkItems.value[0];
  if (
    first &&
    !selectedValidationWorkItems.value.some(
      (item) => item.findingId === validationWorkEditor.value?.findingId,
    )
  )
    editValidationWorkItem(first);
});
watch(
  () => props.projectId,
  async (value) => {
    if (projectFilter.value === value) return;
    projectFilter.value = value;
    selected.value = undefined;
    previewScan.value = undefined;
    selectedUrl.value = "";
    await load();
  },
);
watch(tab, async (value) => {
  emit("section-change", value);
  try {
    if (value === "queue" && !previewScan.value && queueScans.value[0])
      await preview(queueScans.value[0]);
    if (value === "fuse") fuseEntries.value = await api.listSentinelFuseZone(projectFilter.value);
    if (value === "validations") {
      validationWorkItems.value = await api.listSentinelValidationWorkItems(projectFilter.value);
      const first = selectedValidationWorkItems.value[0];
      if (first && !validationWorkEditor.value) editValidationWorkItem(first);
    }
  } catch (error) {
    emit("notify", "error", String(error));
  }
});
watch(
  () => [stats.value.pendingVulnerabilityCount, stats.value.activeFuseCount] as const,
  ([vulnerabilities, fuse]) => emit("alerts-change", { fuse, vulnerabilities }),
  { immediate: true },
);
async function applySearch(value: string) {
  if (!value.trim()) {
    matchedScanIds.value = [];
    return;
  }
  tab.value = "results";
  try {
    matchedScanIds.value = await api.searchSentinelScanIds(value);
    const scan = visibleScans.value[0];
    if (scan && scan.id !== selected.value?.id) await openScan(scan, false);
  } catch (e) {
    emit("notify", "error", `Strix 查询失败：${String(e)}`);
  }
}
watch(() => props.search, applySearch);
watch(
  () => props.active,
  async (active) => {
    if (!active) return;
    // The board stays mounted while the user works in Asset. New drafts created
    // there are already in SQLite, but the in-memory scan list is stale. Reload
    // the lightweight database views whenever Strix becomes visible; artifact
    // synchronization remains a separate deferred/background operation.
    await load();
    scheduleInitialBackgroundSync();
  },
);
onMounted(async () => {
  await load();
  scheduleInitialBackgroundSync();
  if (props.search.trim()) await applySearch(props.search);
  liveTimer = window.setInterval(liveSync, 12000);
});
onUnmounted(() => {
  if (liveTimer !== undefined) window.clearInterval(liveTimer);
  if (initialSyncTimer !== undefined) window.clearTimeout(initialSyncTimer);
});
</script>

<template>
<div class="sentinel-page sentinel-v2">
    <div v-if="loading || backgroundSyncing" class="strix-load-state">
      <span class="loader-ring"></span>
      <div>
        <strong>{{
          loading
            ? tr("正在加载本地 Strix 数据", "Loading local Strix data")
            : tr(
                "正在后台解析新增 Strix 结果",
                "Parsing new Strix results in background",
              )
        }}</strong
        ><small>{{
          tr(
            "已保存的数据会先显示；后台同步不会阻塞页面操作。",
            "Saved data appears first; background sync does not block the page.",
          )
        }}</small>
      </div>
    </div>
    <SentinelAuthRecoveryPanel
      v-if="authRecoveryScan"
      :scan="authRecoveryScan"
      :sessions="authRecoverySessions"
      :busy="authRecoveryBusy"
      @reopen="reopenAuthRecovery"
      @finish="finishAuthRecovery"
      @validate="validateAuthRecovery"
      @continue="continueAfterAuthRecovery"
      @close="authRecoveryScan = undefined"
    />
    <template v-if="tab === 'overview'">
      <section class="panel investigation-hero">
        <div class="investigation-hero-copy">
          <span class="eyebrow">AUTONOMOUS INVESTIGATION DESK</span>
          <h2>先给结论与证据，再决定是否消耗 Token</h2>
          <p>
            页面渲染、功能入口触发、HTTP 捕获、参数还原、JS/指纹与本地知识匹配由确定性流程完成；
            只有形成高价值候选后才把有限上下文交给 Strix。
          </p>
        </div>
        <div class="investigation-hero-actions">
          <button class="button primary" @click="tab = 'workbench'">
            <Play :size="15" /> 新建自动调查
          </button>
          <button class="button ghost" @click="tab = 'queue'">
            <Activity :size="15" /> 任务与成本
          </button>
        </div>
      </section>
      <div class="investigation-layout">
        <section class="panel opportunity-inbox">
          <div class="panel-heading opportunity-heading">
            <div>
              <span class="eyebrow">OPPORTUNITY INBOX</span>
              <h3>高价值机会收件箱</h3>
              <p>每张卡片都必须说明价值、证据来源、接口参数和下一步；它不是漏洞结论。</p>
            </div>
            <div class="segmented opportunity-filter">
              <button :class="{ active: opportunityView === 'ready' }" @click="opportunityView = 'ready'">
                <span>可验证</span><b v-if="stats.readyOpportunityCount" class="opportunity-count-badge">{{ stats.readyOpportunityCount }}</b>
              </button>
              <button :class="{ active: opportunityView === 'all' }" @click="opportunityView = 'all'">
                <span>活跃</span><b v-if="stats.opportunityCount" class="opportunity-count-badge">{{ stats.opportunityCount }}</b>
              </button>
              <button :class="{ active: opportunityView === 'history' }" @click="opportunityView = 'history'">
                历史
              </button>
            </div>
          </div>
          <div class="opportunity-list">
            <article
              v-for="item in visibleOpportunities.slice(0, 24)"
              :key="item.id"
              class="opportunity-card"
              :class="[`score-${item.score >= 80 ? 'high' : item.score >= 65 ? 'medium' : 'low'}`, item.status]"
            >
              <div class="opportunity-score">
                <strong>{{ item.score }}</strong><small>价值分</small>
              </div>
              <div class="opportunity-content">
                <header>
                  <span>{{ opportunityCategoryLabel(item.category) }}</span>
                  <em :class="`opportunity-status ${item.status}`">{{ opportunityStatusLabel(item.status) }}</em>
                  <b v-if="opportunityEvidenceCount(item)" class="opportunity-evidence-badge">证据 {{ opportunityEvidenceCount(item) }}</b>
                  <small>{{ item.confidence || 'unknown' }} confidence</small>
                </header>
                <h4>{{ item.title }}</h4>
                <code class="opportunity-endpoint">{{ opportunityEndpoint(item) }}</code>
                <ul>
                  <li v-for="reason in item.why.slice(0, 3)" :key="reason">{{ reason }}</li>
                </ul>
                <div v-if="opportunityParameters(item).length" class="opportunity-params">
                  <span>参数</span><code v-for="parameter in opportunityParameters(item).slice(0, 12)" :key="parameter">{{ parameter }}</code>
                </div>
                <div v-if="opportunityKnowledge(item).length" class="opportunity-knowledge">
                  <Fingerprint :size="14" /> 已命中 {{ opportunityKnowledge(item).length }} 条本地知识：
                  {{ opportunityKnowledgeTitles(item) }}
                </div>
                <footer>
                  <span>{{ item.recommendedAction?.label || '查看完整证据后决定下一步' }}</span>
                  <div>
                    <button class="button primary small" :disabled="opportunityBusy === item.id" @click="openOpportunity(item, true)">
                      开始调查
                    </button>
                    <button class="button ghost small" @click="openOpportunity(item)">证据</button>
                    <button
                      v-if="activeOpportunityStatuses.has(item.status)"
                      class="button ghost small"
                      :disabled="opportunityBusy === item.id"
                      @click="setOpportunityStatus(item, 'dismissed')"
                    >忽略</button>
                  </div>
                </footer>
              </div>
            </article>
            <div v-if="!visibleOpportunities.length" class="empty-state opportunity-empty">
              <template v-if="opportunityView === 'ready' && activeOpportunityClues.length">
                <strong>当前没有达到可直接验证门禁的候选</strong>
                <span>仍保留 {{ activeOpportunityClues.length }} 条有效线索，没有把它们误标成可验证结果。</span>
                <div class="opportunity-clue-preview">
                  <button v-for="item in activeOpportunityClues" :key="item.id" @click="openOpportunity(item)">
                    <b>{{ item.score }}</b><span>{{ item.title }}</span><em>查看证据</em>
                  </button>
                </div>
                <button class="button ghost compact" @click="opportunityView = 'all'">查看全部活跃线索</button>
              </template>
              <template v-else>
                <strong>当前没有匹配的机会卡</strong>
                <span>新任务会先完成浏览器探索与前端侦察；没有 70 分以上候选时自动进入一次性目录/API 兜底发现。</span>
              </template>
            </div>
          </div>
        </section>
        <aside class="investigation-sidebar">
          <section class="panel investigation-pipeline">
            <span class="eyebrow">DECISION FUNNEL</span>
            <h3>自动调查漏斗</h3>
            <ol>
              <li class="done"><b>1</b><div><strong>渲染与功能触发</strong><small>导航、标签、菜单、详情控件</small></div></li>
              <li class="done"><b>2</b><div><strong>HTTP / 参数 / JS</strong><small>运行时请求与静态 AST 合并</small></div></li>
              <li><b>3</b><div><strong>指纹 + 本地知识 + PoC</strong><small>只跑命中的确定性验证</small></div></li>
              <li><b>4</b><div><strong>一次兜底发现</strong><small>拦截或无新增价值立即停止</small></div></li>
            </ol>
          </section>
          <section class="panel investigation-budget">
            <span class="eyebrow">COST & STOP POLICY</span>
            <h3>成本和停止条件</h3>
            <dl>
              <div><dt>活跃任务</dt><dd>{{ runningScanCount }}</dd></div>
              <div><dt>模型请求</dt><dd>{{ formatNumber(totalRequestUsage) }}</dd></div>
              <div><dt>总 Token</dt><dd :title="formatNumber(totalTokenUsage)">{{ formatCompactNumber(totalTokenUsage) }}</dd></div>
              <div><dt>可验证机会</dt><dd>{{ stats.readyOpportunityCount }}</dd></div>
              <div><dt>平均信息增益</dt><dd>{{ investigationStats.averageInformationGain }}/100</dd></div>
              <div><dt>允许模型目标</dt><dd>{{ investigationStats.tokenWorthyCount }}/{{ investigationStats.targetCount }}</dd></div>
              <div><dt>确定性事实</dt><dd>{{ investigationStats.factCount }}</dd></div>
              <div><dt>已晋升策略</dt><dd>{{ investigationStats.promotedStrategyCount }}</dd></div>
            </dl>
            <p>写请求仅捕获、不转发；单个 401/403 记为权限边界，确认 WAF、验证码、机器人挑战或持续限流才立即结束。</p>
          </section>
        </aside>
      </div>
      <div class="sentinel-kpis sentinel-kpis-v2">
        <article class="panel" :class="{ 'has-count-alert': stats.readyOpportunityCount > 0 }">
          <ShieldAlert :size="18" /><span>可验证机会</span
          ><strong>{{ stats.readyOpportunityCount }}</strong
          ><small>{{ stats.opportunityCount }} 个活跃候选</small
          ><b v-if="stats.readyOpportunityCount" class="kpi-count-alert">{{ stats.readyOpportunityCount }}</b>
        </article>
        <article class="panel" :class="{ 'has-count-alert': runningScanCount > 0 }">
          <Activity :size="18" /><span>正在调查</span
          ><strong>{{ runningScanCount }}</strong
          ><small>{{ stats.taskCount }} 个历史任务</small
          ><b v-if="runningScanCount" class="kpi-count-alert">{{ runningScanCount }}</b>
        </article>
        <article class="panel">
          <Network :size="18" /><span>接口与端点</span
          ><strong>{{ stats.apiCount + stats.endpointCount }}</strong
          ><small>运行时 + JS / AST + 发现</small>
        </article>
        <article class="panel">
          <Bug :size="18" /><span>{{ tr("漏洞", "Vulnerabilities") }}</span
          ><strong>{{ stats.vulnerabilityCount }}</strong
          ><small
            ><b class="risk-high">{{ stats.highRiskCount }}</b>
            {{ tr("个高危/严重", "high / critical") }}</small
          >
        </article>
        <article class="panel cumulative-token-kpi">
          <Cpu :size="18" /><span>累计 Token</span
          ><strong class="token-kpi-value" :title="formatNumber(totalTokenUsage)">{{ formatCompactNumber(totalTokenUsage) }}</strong
          ><small>{{ formatNumber(totalRequestUsage) }} 次模型请求</small>
        </article>
      </div>
      <div class="sentinel-overview-grid">
        <section class="panel sentinel-chart-card">
          <div class="panel-heading">
            <div>
              <span class="eyebrow">RESULT DISTRIBUTION</span>
              <h3>{{ tr("结构化结果分布", "Structured results") }}</h3>
              <p>
                {{
                  tr(
                    "长度代表记录数量，颜色代表数据类型，不代表风险。",
                    "Bar length is record count; color identifies the data type, not risk.",
                  )
                }}
              </p>
            </div>
          </div>
          <div class="sentinel-bar-chart">
            <div v-for="bar in overviewBars" :key="bar.label" class="bar-row">
              <span>{{ bar.label }}</span>
              <div>
                <i
                  :style="{
                    width: `${Math.max(3, (bar.value / overviewMax) * 100)}%`,
                    background: bar.color,
                  }"
                ></i>
              </div>
              <strong>{{ bar.value }}</strong>
            </div>
          </div>
        </section>
        <section class="panel sentinel-chart-card">
          <div class="panel-heading">
            <div>
              <span class="eyebrow">TASK STATUS</span>
              <h3>{{ tr("任务状态", "Task status") }}</h3>
            </div>
          </div>
          <div class="task-status-chart">
            <div v-for="item in taskStatus" :key="item.status">
              <span :class="`task-dot ${item.status}`"></span
              ><em>{{ statusLabel(item.status) }}</em
              ><strong>{{ item.count }}</strong>
            </div>
          </div>
        </section>
      </div>
      <section class="panel sentinel-token-overview">
        <div class="token-overview-heading">
          <div>
            <span class="eyebrow">TOKEN ACCOUNTING</span>
            <h3>{{ tr("模型 Token 用量", "Model token usage") }}</h3>
          </div>
          <div class="token-scope-switch segmented" role="tablist">
            <button :class="{ active: tokenScope === 'all' }" @click="tokenScope = 'all'">{{ tr("全部", "All") }}</button>
            <button :class="{ active: tokenScope === 'cloud' }" @click="tokenScope = 'cloud'">{{ tr("云端 AI", "Cloud AI") }}</button>
            <button :class="{ active: tokenScope === 'local' }" @click="tokenScope = 'local'">{{ tr("本地 LLM", "Local LLM") }}</button>
          </div>
          <p>
            {{
              tr(
                "缓存是输入的一部分；新增输入与输出分开计算。",
                "Cached tokens are part of input; uncached input and output are reported separately.",
              )
            }}
          </p>
        </div>
        <div class="token-summary-grid">
          <article>
            <span>{{ tr("输入总计", "Input total") }}</span>
            <strong>{{ formatNumber(totalInputTokenUsage) }}</strong>
          </article>
          <article class="cached">
            <span>{{ tr("其中缓存输入", "Cached input") }}</span>
            <strong>{{ formatNumber(totalCachedTokenUsage) }}</strong>
          </article>
          <article class="new-input">
            <span>{{ tr("新增输入", "Uncached input") }}</span>
            <strong>{{ formatNumber(totalUncachedInputUsage) }}</strong>
          </article>
          <article class="output">
            <span>{{ tr("输出", "Output") }}</span>
            <strong>{{ formatNumber(totalOutputTokenUsage) }}</strong>
          </article>
          <article class="total">
            <span>{{ tr("输入 + 输出总计", "Input + output") }}</span>
            <strong>{{ formatNumber(totalTokenUsage) }}</strong>
          </article>
          <article class="requests">
            <span>{{ tr("模型请求", "LLM requests") }}</span>
            <strong>{{ formatNumber(totalRequestUsage) }}</strong>
          </article>
        </div>
        <div class="cost-decision-grid">
          <article>
            <span>缓存命中率</span><strong>{{ cacheHitRate }}%</strong>
            <small>{{ cacheHitRate >= 50 ? "重复上下文复用正常" : "缓存复用偏低，检查提示词与任务续跑策略" }}</small>
          </article>
          <article>
            <span>每个漏洞消耗</span><strong>{{ stats.vulnerabilityCount ? formatNumber(tokensPerVulnerability) : "—" }}</strong>
            <small>{{ stats.vulnerabilityCount ? "Token / 结构化漏洞" : "当前没有可计算的漏洞产出" }}</small>
          </article>
          <article :class="{ warning: zeroYieldScans.length }">
            <span>零漏洞产出任务</span><strong>{{ zeroYieldScans.length }}</strong>
            <small>累计 {{ formatNumber(zeroYieldTokenUsage) }} Token，优先复盘熔断和路由</small>
          </article>
          <article v-if="highestCostScan" class="highest-cost">
            <span>最高成本任务</span><strong>{{ formatCompactNumber(scanTokenTotal(highestCostScan)) }}</strong>
            <small>{{ scanTitle(highestCostScan) }}</small>
            <button class="text-button" @click="openScan(highestCostScan)">查看任务证据</button>
          </article>
        </div>
        <div class="token-type-grid">
          <article v-for="item in tokenTypeRows" :key="item.type">
            <header>
              <span>{{ scanTypeLabel(item.type) }}</span>
              <strong>{{ formatNumber(item.total) }}</strong>
            </header>
            <dl>
              <div>
                <dt>{{ tr("输入", "Input") }}</dt>
                <dd>{{ formatNumber(item.input) }}</dd>
              </div>
              <div>
                <dt>{{ tr("缓存", "Cached") }}</dt>
                <dd>{{ formatNumber(item.cached) }}</dd>
              </div>
              <div>
                <dt>{{ tr("新增输入", "Uncached") }}</dt>
                <dd>{{ formatNumber(item.uncachedInput) }}</dd>
              </div>
              <div>
                <dt>{{ tr("输出", "Output") }}</dt>
                <dd>{{ formatNumber(item.output) }}</dd>
              </div>
              <div>
                <dt>{{ tr("请求", "Requests") }}</dt>
                <dd>{{ formatNumber(item.requests) }}</dd>
              </div>
            </dl>
          </article>
        </div>
      </section>
      <section class="panel sentinel-panel">
        <div class="panel-heading">
          <div>
            <span class="eyebrow">ALL TASKS</span>
            <h3>{{ tr("任务总览", "Task overview") }}</h3>
            <p class="task-overview-caption">
              按创建日期和任务类型组织，便于区分 Web、代码、灰盒与 CI/CD 扫描。
            </p>
          </div>
          <span class="project-scope-note">{{ props.projectId ? tr("跟随顶部当前项目", "Following current project") : tr("当前为全部项目汇总", "All-project summary") }}</span>
        </div>
        <div class="task-date-groups">
          <section
            v-for="group in taskGroups"
            :key="group.date"
            class="task-date-group"
          >
            <header>
              <strong>{{ group.date }}</strong
              ><span
                >{{
                  group.types.reduce(
                    (sum, bucket) => sum + bucket.scans.length,
                    0,
                  )
                }}
                个任务</span
              >
            </header>
            <div
              v-for="bucket in group.types"
              :key="bucket.type"
              class="task-type-group"
            >
              <div class="task-type-heading">
                <span class="scan-type-pill">{{
                  scanTypeLabel(bucket.type)
                }}</span
                ><small>{{ bucket.scans.length }} 个</small>
              </div>
              <div class="sentinel-task-grid">
                <div
                  v-for="scan in bucket.scans"
                  :key="scan.id"
                  class="sentinel-task-cell"
                >
                  <article
                    class="sentinel-task-card"
                    role="button"
                    tabindex="0"
                    @click="openScan(scan)"
                    @keydown.enter="openScan(scan)"
                  >
                    <header>
                      <span class="sentinel-status" :class="scan.status"
                        ><Activity :size="15" /></span
                      ><span class="scan-type-pill">{{
                        scanTypeLabel(scan.scanType)
                      }}</span
                      ><span v-if="scan.attemptCount" class="scan-attempt-pill"
                        >第 {{ scan.attemptCount }} 次执行</span
                      ><span class="llm-deployment-badge" :class="llmDeploymentClass(scan)" :title="scan.llmModel || undefined">
                        {{ llmDeploymentLabel(scan) }}
                      </span
                      ><span class="status-chip" :class="scan.status">{{
                        statusLabel(scan.status)
                      }}</span>
                    </header>
                    <h3>{{ scanTitle(scan) }}</h3>
                    <p>{{ scan.projectName }} · {{ scan.id }}</p>
                    <small class="task-date-line"
                      >创建于
                      {{ scan.createdAt || scan.updatedAt || "—" }}</small
                    ><small
                      v-if="scanSummary(scan)"
                      class="live-checkpoint"
                      >{{
                        scanSummary(scan)
                      }}</small
                    >
                    <div v-if="scan.totalTokens" class="task-token-usage">
                      <span>{{ tr("输入", "Input") }}</span
                      ><strong>{{ formatNumber(scan.inputTokens) }}</strong>
                      <dl>
                        <div>
                          <dt>{{ tr("缓存", "Cached") }}</dt>
                          <dd>{{ formatNumber(scan.cachedTokens) }}</dd>
                        </div>
                        <div>
                          <dt>{{ tr("新增输入", "Uncached") }}</dt>
                          <dd>{{ formatNumber(uncachedInput(scan)) }}</dd>
                        </div>
                        <div>
                          <dt>{{ tr("输出", "Output") }}</dt>
                          <dd>{{ formatNumber(scan.outputTokens) }}</dd>
                        </div>
                        <div>
                          <dt>{{ tr("总计", "Total") }}</dt>
                          <dd>{{ formatNumber(scanTokenTotal(scan)) }}</dd>
                        </div>
                      </dl>
                    </div>
                    <footer class="task-card-actions">
                      <button
                        class="button ghost compact"
                        @click.stop="openScan(scan)"
                      >
                        <Eye :size="13" /><span>{{
                          tr("查看结果", "Results")
                        }}</span></button
                      ><button
                        v-if="
                          scan.status === 'scanning' ||
                          scan.status === 'pausing'
                        "
                        class="button warning compact"
                        :disabled="scanControlBusy === scan.id"
                        @click.stop="pauseScan(scan)"
                      >
                        <Pause :size="13" /><span>{{
                          scan.status === "pausing"
                            ? tr("再次停止", "Retry stop")
                            : tr("立即暂停", "Pause now")
                        }}</span></button
                      ><button
                        v-else-if="scan.status === 'paused'"
                        class="button secondary compact"
                        :disabled="scanControlBusy === scan.id"
                        @click.stop="resumeScan(scan)"
                      >
                        <Play :size="13" /><span>{{
                          tr("继续扫描", "Resume")
                        }}</span></button
                      ><button
                        v-else-if="scan.status !== 'draft'"
                        class="button ghost compact"
                        :disabled="scanControlBusy === scan.id"
                        @click.stop="rescan(scan)"
                      >
                        <RefreshCw :size="13" /><span>{{
                          retryActionLabel(scan)
                        }}</span></button
                      ><button
                        class="button danger compact"
                        @click.stop="askRemove(scan)"
                      >
                        <Trash2 :size="13" /><span>{{
                          ["scanning", "pausing"].includes(scan.status)
                            ? tr("强制删除", "Force delete")
                            : tr("删除", "Delete")
                        }}</span>
                      </button>
                    </footer>
                  </article>
                </div>
              </div>
            </div>
          </section>
        </div>
        <div v-if="!taskGroups.length" class="empty-state">
          {{ tr("暂无任务", "No tasks") }}
        </div>
      </section>
    </template>

    <SentinelTaskCenter
      v-else-if="tab === 'queue'"
      :scans="queueScans"
      :preview="previewScan"
      :preview-targets="previewTargetRows"
      :attention-count="attentionTaskCount"
      :total-tokens="totalTokenUsage"
      :total-requests="totalRequestUsage"
      :zero-yield-count="zeroYieldScans.length"
      :zero-yield-tokens="zeroYieldTokenUsage"
      :cache-hit-rate="cacheHitRate"
      :control-busy="scanControlBusy"
      :high-value-count="scanHighValueCount"
      @preview="preview"
      @close="previewScan = undefined"
      @confirm="confirm"
      @pause="pauseScan"
      @resume="resumeScan"
      @retry="rescan"
      @remove="askRemove"
      @open="openScan"
    />

    <template v-else-if="tab === 'results'">
      <div class="sentinel-result-shell">
        <aside class="panel result-task-rail">
          <header>
            <span class="eyebrow">TASKS</span><strong>扫描任务</strong>
          </header>
          <button
            v-for="scan in resultTaskScans"
            :key="scan.id"
            :class="{ active: selected?.id === scan.id }"
            @click="openScan(scan, false)"
          >
            <span>{{ scan.projectName || "未命名" }}</span
            ><small
              >{{ statusLabel(scan.status) }} ·
              {{ scanSummary(scan) || "等待结果" }}</small
            ><em>{{ scan.id }}</em>
          </button>
        </aside>
        <aside
          v-if="selected?.scanType === 'web'"
          class="panel result-url-rail"
        >
          <header>
            <span class="eyebrow">COMPANY ASSETS</span
            ><strong>{{ filteredUrlCards.length }} 个目标</strong>
          </header>
          <section
            v-for="group in companyGroups"
            :key="group.company"
            class="company-url-group"
          >
            <h4>{{ group.company }}</h4>
            <button
              v-for="card in group.urls"
              :key="card.url"
              :class="{ active: selectedUrl === card.url }"
              @click="
                selectedUrl = card.url;
                if (resultTab !== 'vulnerabilities') resultTab = 'summary';
              "
            >
              <Globe2 :size="14" /><span>{{
                card.url === "*" ? "全部目标 / 全局发现" : card.url
              }}</span
              ><small
                ><i
                  v-if="card.scanMode"
                  class="route-mode"
                  :class="card.scanMode"
                  >{{ card.valueScore }} · {{ card.scanMode }}</i
                ><b v-if="card.high">{{ card.high }} 高危</b
                ><b v-if="card.sensitive" class="sensitive-count-badge">{{ card.sensitive }} 敏感</b
                >{{ statusLabel(card.status) }} · {{ card.scanCount }} 次扫描 ·
                {{ card.vulnerabilities }} 漏洞<span v-if="card.pendingVulnerabilities"> · {{ card.pendingVulnerabilities }} 待处理</span> · {{ card.total }} 条</small
              >
            </button>
          </section>
          <div
            v-if="selected && !filteredUrlCards.length"
            class="empty-state small"
          >
            当前搜索没有匹配的公司或 URL。
          </div>
        </aside>
        <aside v-else class="panel result-url-rail source-context-rail">
          <header>
            <span class="eyebrow">SOURCE CONTEXT</span
            ><strong>{{ sourceStats.codeFiles }} 个代码文件</strong>
          </header>
          <div class="source-context-copy">
            <span>源码路径</span><code>{{ selected?.sourcePath || "—" }}</code>
          </div>
          <div class="source-context-copy">
            <span>项目架构</span
            ><strong>{{ sourceInventory.architecture || "等待识别" }}</strong>
          </div>
          <div class="source-context-copy">
            <span>语言</span
            ><strong>{{ sourceLanguages.join("、") || "等待识别" }}</strong>
          </div>
          <div class="source-context-copy">
            <span>可审计发现</span
            ><strong
              >{{ sourceStats.findings }} 条 ·
              {{ sourceStats.rules }} 条规则</strong
            >
          </div>
        </aside>
        <main class="panel url-intelligence">
          <div v-if="!selected" class="empty-state">请先选择扫描任务。</div>
          <template v-else
            ><template v-if="selected.scanType !== 'web'"
              ><header class="url-intelligence-head source-intelligence-head">
                <div>
                  <span class="eyebrow"
                    >{{ scanTypeLabel(selected.scanType) }} · SOURCE
                    INTELLIGENCE</span
                  >
                  <h3>{{ selected.taskName || selected.projectName }}</h3>
                  <p>
                    {{ selected.sourcePath || "未提供源码路径" }} ·
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
                        ? "再次停止"
                        : "立即暂停"
                    }}</button
                  ><button
                    v-else-if="selected.status === 'paused'"
                    class="button secondary compact"
                    :disabled="scanControlBusy === selected.id"
                    @click="resumeScan(selected)"
                  >
                    <Play :size="14" />继续扫描</button
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
                    <Download :size="14" />导出项目包
                  </button>
                </div>
              </header>
              <div v-if="detailBusy" class="empty-state">正在解析源码结果…</div>
              <div v-else class="source-result-stack">
                <section class="source-scan-meta">
                  <div>
                    <span>扫描类型</span
                    ><strong>{{ scanTypeLabel(selected.scanType) }}</strong>
                  </div>
                  <div>
                    <span>扫描状态</span
                    ><strong>{{ statusLabel(selected.status) }}</strong>
                  </div>
                  <div>
                    <span>源码路径</span
                    ><code>{{ selected.sourcePath || "—" }}</code>
                  </div>
                  <div>
                    <span>扫描器 / 技能</span
                    ><code>{{ selected.skillNames || "默认规则集" }}</code>
                  </div>
                  <div v-if="isGreyboxScan">
                    <span>联测范围</span><strong>源码 + 运行期证据</strong>
                  </div>
                  <div v-if="isCicdScan">
                    <span>质量门禁</span><strong>变更范围 / 阻断发现</strong>
                  </div>
                </section>
                <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-architecture-block">
                  <div class="block-title">
                    <Layers3 :size="16" />
                    <div>
                      <strong>项目架构与技术栈</strong>
                      <small
                        >只根据源码扩展名和仓库清单识别，不使用漏洞记录里的
                        ecosystem 字段推测语言。</small
                      >
                    </div>
                  </div>
                  <template v-if="sourceInventoryFinding">
                    <div class="source-architecture-grid">
                      <article class="architecture-primary">
                        <span>应用架构</span>
                        <strong>{{
                          sourceInventory.architecture || "Source repository"
                        }}</strong>
                        <small
                          >{{ sourceStats.totalFiles }} 个仓库文件 ·
                          {{ sourceStats.codeFiles }} 个可识别代码文件</small
                        >
                      </article>
                      <article
                        v-for="framework in sourceFrameworks"
                        :key="`${framework.layer}-${framework.name}`"
                      >
                        <span>{{ framework.layer }}</span>
                        <strong>{{ framework.name }}</strong>
                        <small>清单特征：{{ framework.evidence }}</small>
                      </article>
                    </div>
                    <div class="source-loc-grid">
                      <article>
                        <span>物理行</span>
                        <strong>{{
                          formatNumber(sourceLineStats.physical)
                        }}</strong>
                      </article>
                      <article class="loc-code">
                        <span>有效代码</span>
                        <strong>{{
                          formatNumber(sourceLineStats.code)
                        }}</strong>
                      </article>
                      <article>
                        <span>注释行</span>
                        <strong>{{
                          formatNumber(sourceLineStats.comments)
                        }}</strong>
                      </article>
                      <article>
                        <span>空行</span>
                        <strong>{{
                          formatNumber(sourceLineStats.blank)
                        }}</strong>
                      </article>
                    </div>
                    <p class="source-loc-rule">
                      本地按可识别源码文件统计物理行；含代码的行计入有效代码，纯注释与空行分别计数。排除依赖、构建产物和版本库目录，单文件超过
                      5 MB 不读取。
                      <span v-if="sourceLineStats.skippedLargeFiles">
                        本次跳过
                        {{ sourceLineStats.skippedLargeFiles }} 个超大源码文件。
                      </span>
                    </p>
                    <div
                      v-if="sourceManifests.length"
                      class="source-manifest-list"
                    >
                      <span>识别证据</span>
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
                        <span>语言</span><span>文件数</span><span>有效代码</span
                        ><span>注释</span><span>空行</span><span>物理行</span
                        ><span>代码体积</span><span>文件占比</span>
                      </div>
                      <div
                        v-for="language in sourceLanguageRows"
                        :key="language.name"
                      >
                        <strong>{{ language.name }}</strong>
                        <span>{{ formatNumber(language.files) }}</span>
                        <span>{{ formatNumber(language.codeLines) }}</span>
                        <span>{{ formatNumber(language.commentLines) }}</span>
                        <span>{{ formatNumber(language.blankLines) }}</span>
                        <span>{{ formatNumber(language.lines) }}</span>
                        <span>{{ formatNumber(language.bytes) }} B</span>
                        <span
                          >{{ Number(language.percent || 0).toFixed(1) }}%</span
                        >
                      </div>
                    </div>
                  </template>
                  <div v-else class="source-inventory-warning">
                    没有可用的源码清单，通常表示原源码路径已不可读取。恢复该目录后重新打开任务即可本地补建，不会调用模型；也可以再次扫描生成新结果。
                  </div>
                </section>
                <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-token-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>模型与 Token 消耗</strong
                      ><small
                        >缓存输入属于输入总计；新增输入和模型输出分别展示。</small
                      >
                    </div>
                  </div>
                  <div class="source-token-grid">
                    <article>
                      <span>模型请求</span
                      ><strong>{{ formatNumber(selected.llmRequests) }}</strong>
                    </article>
                    <article>
                      <span>输入 Token</span
                      ><strong>{{ formatNumber(selected.inputTokens) }}</strong>
                    </article>
                    <article>
                      <span>输出 Token</span
                      ><strong>{{
                        formatNumber(selected.outputTokens)
                      }}</strong>
                    </article>
                    <article>
                      <span>缓存 Token</span
                      ><strong>{{
                        formatNumber(selected.cachedTokens)
                      }}</strong>
                    </article>
                    <article>
                      <span>新增输入 Token</span
                      ><strong>{{
                        formatNumber(uncachedInput(selected))
                      }}</strong>
                    </article>
                    <article>
                      <span>输入 + 输出总计</span
                      ><strong>{{ formatNumber(scanTokenTotal(selected)) }}</strong>
                    </article>
                  </div>
                </section>
                <section v-if="resultTab !== 'vulnerabilities' && scanAttempts.length" class="result-block attempt-ledger-block">
                  <div class="block-title">
                    <RefreshCw :size="16" />
                    <div><strong>执行尝试与增量成本</strong><small>同一任务 ID 下按尝试隔离；上方 Token 是任务累计值，这里只显示每次新增消耗。</small></div>
                  </div>
                  <div class="attempt-ledger">
                    <article v-for="attempt in scanAttempts" :key="attempt.attemptNumber" :class="[`attempt-${attempt.status}`, { current: attempt.attemptNumber === selected.attemptCount }]">
                      <header><span>第 {{attempt.attemptNumber}} 次</span><b>{{attemptStageLabel(attempt.stage)}}</b><em class="status-chip" :class="attempt.status">{{statusLabel(attempt.status)}}</em></header>
                      <p>{{attempt.checkpoint || '尚无阶段详情'}}</p>
                      <div class="attempt-cost"><span>请求 <b>{{formatNumber(attempt.llmRequests)}}</b></span><span>输入 <b>{{formatNumber(attempt.inputTokens)}}</b></span><span>缓存 <b>{{formatNumber(attempt.cachedTokens)}}</b></span><span>输出 <b>{{formatNumber(attempt.outputTokens)}}</b></span><span>本次总计 <b>{{formatNumber(attempt.totalTokens)}}</b></span></div>
                      <small>{{attemptTime(attempt)}}</small><code v-if="attempt.workDir" :title="attempt.workDir">{{attempt.workDir}}</code><mark v-if="attemptEndReason(attempt)">结束说明：{{attemptEndReason(attempt)}}</mark>
                    </article>
                  </div>
                </section>
                <section v-if="resultTab !== 'vulnerabilities'" class="result-block source-summary-block">
                  <div class="block-title">
                    <Code2 :size="16" />
                    <div>
                      <strong>代码扫描概况</strong
                      ><small
                        >参考
                        SAST、SCA、质量门禁和数据流分析结果标准化展示。</small
                      >
                    </div>
                  </div>
                  <div class="source-metric-grid">
                    <article>
                      <span>安全发现</span
                      ><strong>{{ sourceStats.findings }}</strong>
                    </article>
                    <article>
                      <span>受影响文件</span
                      ><strong>{{ sourceStats.files }}</strong>
                    </article>
                    <article>
                      <span>代码位置</span
                      ><strong>{{ sourceStats.locations }}</strong>
                    </article>
                    <article>
                      <span>命中规则</span
                      ><strong>{{ sourceStats.rules }}</strong>
                    </article>
                    <article>
                      <span>仓库代码文件</span
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
                      <strong>发现索引</strong
                      ><small
                        >先在索引选择一条发现，下方只展示当前发现的完整证据。</small
                      >
                    </div>
                  </div>
                  <div class="source-finding-index">
                    <div class="table-head">
                      <span>等级</span><span>标题</span
                      ><span>引擎 / Rule ID</span><span>CWE / CVE</span
                      ><span>文件</span><span>行号</span>
                    </div>
                    <div
                      v-for="item in sourceFindingRows"
                      :key="`index-${item.id}`"
                      :class="{ active: selectedFindingId === item.id }"
                      role="button"
                      tabindex="0"
                      @click="selectedFindingId = item.id"
                      @keydown.enter="selectedFindingId = item.id"
                    >
                      <span
                        :class="`severity-badge ${effectiveSeverity(item)}`"
                        >{{ severityLabel(effectiveSeverity(item)) }}</span
                      >
                      <strong>{{
                        item.title ||
                        json(item.recordJson).title ||
                        item.recordKey
                      }}</strong>
                      <code
                        >{{
                          json(item.recordJson).engine ||
                          json(item.recordJson).source ||
                          "Strix"
                        }}
                        ·
                        {{
                          json(item.recordJson).rule_id ||
                          json(item.recordJson).ruleId ||
                          item.recordKey
                        }}</code
                      >
                      <span
                        >{{ json(item.recordJson).cwe || "—" }} /
                        {{ json(item.recordJson).cve || "—" }}</span
                      >
                      <code>{{ sourceLocations(item)[0]?.file || "—" }}</code>
                      <span>{{
                        sourceLocations(item)[0]?.start_line ||
                        sourceLocations(item)[0]?.startLine ||
                        "—"
                      }}</span>
                    </div>
                    <div v-if="!sourceFindingRows.length" class="empty-inline">
                      没有可索引的安全发现。
                    </div>
                  </div>
                </section>
                <section class="result-block source-findings-block">
                  <div class="block-title">
                    <Bug :size="16" />
                    <div>
                      <strong>代码发现与审计证据</strong
                      ><small
                        >仅展示结构化安全问题与漏洞证据；Strix
                        扫描总结和质量门禁摘要不作为问题加载。</small
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
                      <small v-if="group.high">{{ group.high }} 个高风险</small>
                      <small v-else>无高风险项</small>
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
                            json(item.recordJson).title ||
                            item.recordKey
                          }}</strong
                          ><small
                            >{{
                              json(item.recordJson).rule_id ||
                              json(item.recordJson).ruleId ||
                              "未标注规则"
                            }}
                            · {{ json(item.recordJson).cwe || "无 CWE" }} ·
                            {{ json(item.recordJson).cve || "无 CVE" }} · 置信度
                            {{ json(item.recordJson).confidence || "—" }}</small
                          >
                        </div>
                        <span
                          v-if="json(item.recordJson).status"
                          class="finding-state"
                          >{{ json(item.recordJson).status }}</span
                        >
                      </header>
                      <div class="source-finding-grid">
                        <div>
                          <span>描述</span>
                          <p>
                            {{
                              json(item.recordJson).description ||
                              json(item.recordJson).message ||
                              "—"
                            }}
                          </p>
                        </div>
                        <div>
                          <span>技术分析 / 影响</span>
                          <p>
                            {{
                              json(item.recordJson).technical_analysis ||
                              json(item.recordJson).impact ||
                              json(item.recordJson).detail ||
                              "—"
                            }}
                          </p>
                        </div>
                        <div v-if="sourceLocations(item).length">
                          <span>文件与行号</span>
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
                            json(item.recordJson).data_flow ||
                            json(item.recordJson).taint_flow ||
                            json(item.recordJson).call_chain
                          "
                        >
                          <span>数据流 / 调用链</span>
                          <pre>{{
                            text(
                              json(item.recordJson).data_flow ||
                                json(item.recordJson).taint_flow ||
                                json(item.recordJson).call_chain,
                            )
                          }}</pre>
                        </div>
                        <div>
                          <span>修复建议</span>
                          <p>
                            {{
                              json(item.recordJson).recommendation ||
                              json(item.recordJson).remediation_steps ||
                              "—"
                            }}
                          </p>
                        </div>
                        <div
                          v-if="
                            json(item.recordJson).fix_before ||
                            json(item.recordJson).fix_after
                          "
                        >
                          <span>修复前 / 修复后</span>
                          <pre
                            >{{
                              json(item.recordJson).fix_before || "—"
                            }}\n\n→\n\n{{
                              json(item.recordJson).fix_after || "—"
                            }}</pre>
                        </div>
                        <div
                          v-if="
                            json(item.recordJson).dependency_metadata ||
                            json(item.recordJson).package
                          "
                        >
                          <span>依赖信息</span>
                          <pre>{{
                            JSON.stringify(
                              json(item.recordJson).dependency_metadata ||
                                json(item.recordJson).package,
                              null,
                              2,
                            )
                          }}</pre>
                        </div>
                        <div
                          v-if="
                            json(item.recordJson).evidence ||
                            json(item.recordJson).pocRequest
                          "
                        >
                          <span>证据 / 验证</span>
                          <pre>{{
                            text(
                              json(item.recordJson).evidence ||
                                json(item.recordJson).pocRequest,
                            )
                          }}</pre>
                        </div>
                        <div v-if="json(item.recordJson).assumptions">
                          <span>前提与限制</span>
                          <p>{{ text(json(item.recordJson).assumptions) }}</p>
                        </div>
                      </div>
                      <footer>
                        <button
                          class="button primary compact"
                          @click="editValidation(item)"
                        >
                          <ClipboardCheck :size="13" />{{
                            validationFor(item)
                              ? "修改验证结论"
                              : "开始人工验证"
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
                      当前源码任务没有结构化发现。
                    </div>
                  </div>
                </section>
                <section v-if="sourceDependencies.length" class="result-block">
                  <div class="block-title">
                    <Layers3 :size="16" />
                    <div>
                      <strong>依赖与供应链风险</strong
                      ><small
                        >展示包生态、已安装版本、修复版本和 CVE/CVSS。</small
                      >
                    </div>
                  </div>
                  <div class="source-dependency-list">
                    <article
                      v-for="item in sourceDependencies"
                      :key="`dep-${item.id}`"
                    >
                      <strong>{{
                        json(item.recordJson).package_name ||
                        json(item.recordJson).package ||
                        json(item.recordJson).dependency_metadata?.name ||
                        item.title
                      }}</strong
                      ><span
                        >{{
                          json(item.recordJson).dependency_metadata
                            ?.ecosystem ||
                          json(item.recordJson).package_ecosystem ||
                          "—"
                        }}
                        ·
                        {{
                          json(item.recordJson).dependency_metadata
                            ?.installed_version ||
                          json(item.recordJson).installed_version ||
                          "—"
                        }}</span
                      ><b
                        >{{ json(item.recordJson).cve || "无 CVE" }} · CVSS
                        {{ json(item.recordJson).cvss ?? "—" }}</b
                      ><em
                        >修复版本
                        {{
                          json(item.recordJson).dependency_metadata
                            ?.fixed_version ||
                          json(item.recordJson).fixed_version ||
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
                      <strong>灰盒运行期关联</strong
                      ><small
                        >统一漏洞关联源码位置、运行期端点、参数和验证来源。</small
                      >
                    </div>
                  </div>
                  <div class="greybox-overview">
                    <article>
                      <span>测试环境</span>
                      <strong>{{
                        appsecResult.context?.environment || "未记录"
                      }}</strong>
                    </article>
                    <article>
                      <span>认证上下文</span>
                      <strong>{{
                        authTypeLabel(appsecResult.context?.authType || "none")
                      }}</strong>
                      <small>{{
                        appsecResult.context?.authenticated
                          ? appsecResult.context?.authProfileName ||
                            "本次临时会话"
                          : "未提供认证会话"
                      }}</small>
                    </article>
                    <article>
                      <span>统一漏洞</span>
                      <strong>{{ appsecVulnerabilities.length }}</strong>
                    </article>
                    <article class="correlated-count">
                      <span>SAST + DAST 已关联</span>
                      <strong>{{ greyboxCorrelated.length }}</strong>
                    </article>
                    <article>
                      <span>来源记录</span>
                      <strong>{{ appsecResult.sources.length }}</strong>
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
                              vulnerability.vulnerabilityType || "未分类漏洞"
                            }}
                            · {{ vulnerability.status || "open" }}</small
                          >
                        </div>
                        <span class="correlation-score"
                          >{{ vulnerability.correlationScore }}% 关联度</span
                        >
                      </header>
                      <div class="appsec-link-grid">
                        <div>
                          <span>运行期端点</span>
                          <code
                            >{{ vulnerability.httpMethod || "—" }}
                            {{ vulnerability.url || "—" }}</code
                          >
                          <small
                            >参数：{{ vulnerability.parameter || "—" }}</small
                          >
                        </div>
                        <div>
                          <span>代码位置</span>
                          <code
                            >{{ vulnerability.file || "—"
                            }}<template v-if="vulnerability.startLine"
                              >:{{ vulnerability.startLine }}</template
                            ></code
                          >
                          <small>符号：{{ vulnerability.symbol || "—" }}</small>
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
                        同一条已验证记录同时包含源码位置与运行期请求证据。
                      </div>
                      <div v-else class="correlation-breakdown">
                        <span
                          v-for="part in correlationParts(vulnerability)"
                          :key="part.key"
                          :class="{ matched: part.matched }"
                        >
                          {{ part.label }}
                          <b>{{
                            part.matched ? `+${part.weight}` : "未匹配"
                          }}</b>
                        </span>
                      </div>
                    </article>
                    <div
                      v-if="!appsecVulnerabilities.length"
                      class="empty-inline"
                    >
                      当前任务没有可统一的结构化漏洞记录。
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
                      <strong>CI/CD 质量门禁</strong
                      ><small
                        >流水线只负责触发与门禁，问题来源仍保留 SAST、SCA
                        和验证证据。</small
                      >
                    </div>
                  </div>
                  <div class="cicd-context-grid">
                    <div>
                      <span>Provider</span
                      ><strong>{{
                        ciProviderLabel(appsecResult.context?.ciProvider || "")
                      }}</strong>
                    </div>
                    <div>
                      <span>仓库</span
                      ><code>{{
                        appsecResult.context?.repositoryUrl || "未记录"
                      }}</code>
                    </div>
                    <div>
                      <span>分支</span
                      ><code>{{
                        appsecResult.context?.branch || "未记录"
                      }}</code>
                    </div>
                    <div>
                      <span>Commit</span
                      ><code>{{
                        appsecResult.context?.commitSha || "未记录"
                      }}</code>
                    </div>
                    <div>
                      <span>Build / Pipeline</span
                      ><code>{{
                        appsecResult.context?.buildId || "未记录"
                      }}</code>
                    </div>
                    <div>
                      <span>环境</span
                      ><strong>{{
                        appsecResult.context?.environment || "未记录"
                      }}</strong>
                    </div>
                  </div>
                  <div
                    :class="`gate-status ${appsecResult.context?.gateStatus || 'not_evaluated'}`"
                  >
                    <div>
                      <span>发布门禁</span>
                      <strong>{{
                        gateStatusLabel(
                          appsecResult.context?.gateStatus || "not_evaluated",
                        )
                      }}</strong>
                      <small>{{
                        appsecResult.context?.gateReason ||
                        "历史任务没有门禁上下文"
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
                          / {{ appsecResult.context?.policy?.maxCritical ?? 0 }}
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
                          / {{ appsecResult.context?.policy?.maxHigh ?? 5 }}
                        </dd>
                      </div>
                      <div>
                        <dt>策略</dt>
                        <dd>
                          {{
                            appsecResult.context?.policy?.blockRelease
                              ? "超限阻断"
                              : "仅告警"
                          }}
                        </dd>
                      </div>
                    </dl>
                  </div>
                  <div class="cicd-blocking-table">
                    <div class="table-head">
                      <span>等级</span><span>问题</span><span>位置 / 端点</span
                      ><span>状态</span><span>生命周期</span><span>负责人</span>
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
                      <span>{{ vulnerability.owner || "未分配" }}</span>
                    </div>
                    <div
                      v-if="!cicdBlockingFindings.length"
                      class="empty-inline"
                    >
                      当前没有导致门禁超限的 Critical / High 问题。
                    </div>
                  </div>
                </section>
              </div></template
            ><template v-else
              ><header class="url-intelligence-head">
                <div>
                  <span class="eyebrow"
                    >{{ companyForUrl(selectedUrl) }} · URL INTELLIGENCE</span
                  >
                  <h3>
                    {{
                      selectedUrl === "*"
                        ? "全部目标 / 全局发现"
                        : selectedUrl || "未选择 URL"
                    }}
                  </h3>
                  <p>
                    {{ selected.projectName }} ·
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
                        ? "再次停止"
                        : "立即暂停"
                    }}</button
                  ><button
                    v-else-if="selected.status === 'paused'"
                    class="button secondary compact"
                    :disabled="scanControlBusy === selected.id"
                    @click="resumeScan(selected)"
                  >
                    <Play :size="14" />继续扫描</button
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
                    <Download :size="14" />导出整个项目
                  </button>
                </div>
              </header>
              <div v-if="selectedUrl" class="url-metric-strip">
                <span v-if="currentTarget?.scanMode" class="route"
                  ><Activity :size="14" /><b>{{ currentTarget.valueScore }}</b>
                  {{ routeModeLabel(currentTarget.scanMode) }}</span
                ><button
                  type="button"
                  title="查看指纹与配置"
                  @click="jumpToResult('fingerprint')"
                  ><Fingerprint :size="14" /><b>{{
                    currentCard?.fingerprints || 0
                  }}</b>
                  指纹</button
                ><button
                  type="button"
                  title="查看 JS、路由与 API"
                  @click="jumpToResult('api')"
                  ><Code2 :size="14" /><b>{{ currentCard?.apis || 0 }}</b>
                  API/JS</button
                ><button
                  type="button"
                  title="查看端点验证"
                  @click="jumpToResult('endpoints')"
                  ><Network :size="14" /><b>{{
                    currentCard?.endpoints || 0
                  }}</b>
                  端点</button
                ><button
                  type="button"
                  class="risk"
                  title="查看漏洞与人工验证"
                  @click="jumpToResult('vulnerabilities')"
                  ><Bug :size="14" /><b>{{
                    currentCard?.vulnerabilities || 0
                  }}</b>
                  漏洞</button
                ><span v-if="selected.totalTokens" class="url-token-metric"
                  ><Activity :size="14" /><b>{{
                    formatNumber(selected.inputTokens)
                  }}</b>
                  输入</span
                ><span v-if="selected.totalTokens" class="url-token-metric"
                  ><b>{{ formatNumber(selected.outputTokens) }}</b> 输出</span
                ><span class="url-token-metric"
                  ><RefreshCw :size="14" /><b>{{ selected.attemptCount || 0 }}</b>
                  次执行</span
                >
              </div>
              <button
                v-if="selectedUrl"
                type="button"
                :class="`evidence-next-action ${evidenceNextAction.tone}`"
                @click="followEvidenceNextAction"
              >
                <span>建议下一步</span><strong>{{ evidenceNextAction.label }}</strong><em>打开处理</em>
              </button>
              <nav class="result-subtabs">
                <button
                  v-for="item in [
                    { key: 'summary', label: '概要' },
                    { key: 'investigation', label: '调查图谱' },
                    { key: 'opportunities', label: '机会与下一步' },
                    { key: 'fingerprint', label: '指纹与配置' },
                    { key: 'api', label: 'JS / API' },
                    { key: 'endpoints', label: '端点验证' },
                    { key: 'vulnerabilities', label: '漏洞与验证' },
                  ]"
                  :key="item.key"
                  :class="{ active: resultTab === item.key }"
                  @click="resultTab = item.key as ResultTab"
                >
                  {{ item.label
                  }}<em v-if="item.key === 'investigation'">{{
                    investigationGraph?.metrics?.informationGain || 0
                  }}</em><em v-if="item.key === 'opportunities'">{{
                    selectedUrlOpportunities.length
                  }}</em><em v-if="item.key === 'api' && currentCard?.sensitive" class="sensitive-tab-count">{{
                    currentCard.sensitive
                  }}</em><em v-if="item.key === 'vulnerabilities'">{{
                    vulnerabilityRows.length
                  }}</em>
                </button>
              </nav>
              <div v-if="detailBusy" class="empty-state">正在解析结果…</div>
              <div v-else-if="!selectedUrl" class="empty-state">
                该任务没有 URL 结果。可先在任务中心查看待扫 URL。
              </div>

              <div
                v-else-if="resultTab === 'summary'"
                class="result-section-stack"
              >
                <section
                  v-if="selectedUrl !== '*' && selectedUrl"
                  class="result-block url-action-block"
                >
                  <div class="block-title">
                    <Globe2 :size="16" />
                    <div>
                      <button
                        class="url-title-link"
                        type="button"
                        @click="openTargetUrl(selectedUrl)"
                      >
                        {{ selectedUrl }} <ExternalLink :size="14" /></button
                      ><small
                        >{{ companyForUrl(selectedUrl) }} · 历史扫描
                        {{ currentTarget?.scanCount || 0 }} 次</small
                      >
                    </div>
                    <button
                      class="button ghost compact"
                      type="button"
                      @click="openTargetUrl(selectedUrl)"
                    >
                      <ExternalLink :size="14" />在浏览器打开
                    </button>
                  </div>
                </section>
                <section
                  v-if="currentTarget?.scanMode"
                  class="result-block adaptive-route-block"
                >
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>自适应扫描决策</strong
                      ><small
                        >{{ currentTarget.scanMode === 'manual_review'
                          ? '复杂前端只保留高价值线索，等待人工复核。'
                          : '本地前置分析决定候选价值，Strix 只验证高价值证据。' }}</small
                      >
                    </div>
                    <span class="route-mode" :class="currentTarget.scanMode">{{
                      routeModeLabel(currentTarget.scanMode)
                    }}</span>
                  </div>
                  <div class="adaptive-route-summary">
                    <article>
                      <span>前端价值</span
                      ><strong
                        >{{ currentTarget.valueScore
                        }}<small>/ 100</small></strong
                      >
                    </article>
                    <article>
                      <span>验证策略</span
                      ><strong>{{ routeModeLabel(currentTarget.scanMode) }}</strong>
                    </article>
                    <article>
                      <span>当前状态</span
                      ><strong>{{ statusLabel(currentTarget.status) }}</strong>
                    </article>
                  </div>
                  <div class="route-reason-list">
                    <span v-for="reason in routeReasonItems" :key="reason">{{
                      reason
                    }}</span
                    ><span v-if="!routeReasonItems.length">暂无分流依据</span>
                  </div>
                </section>
                <section
                  v-if="
                    liveTrace ||
                    liveTraceBusy ||
                    ['scanning', 'pausing'].includes(selected.status)
                  "
                  class="result-block strix-live-chain"
                >
                  <div class="block-title">
                    <Cpu :size="16" />
                    <div>
                      <strong>Strix 实时执行链</strong
                      ><small
                        >模型请求 → 工具/API 调用 → 返回结果 → 下一步判断；内容按原文保存在本机。</small
                      >
                    </div>
                    <span
                      v-if="['scanning', 'pausing'].includes(selected.status)"
                      class="live-chain-state"
                      ><Activity :size="12" /> LIVE</span
                    >
                  </div>
                  <div class="live-chain-current">
                    <span>当前步骤</span>
                    <strong>{{ currentTraceStep() }}</strong>
                    <small v-if="latestTraceEvent"
                      >Agent {{ traceSession(latestTraceEvent.sessionId) }}
                      <template v-if="latestTraceEvent.targetUrl">
                        · {{ latestTraceEvent.targetUrl }}</template
                      >
                      <template v-if="latestTraceEvent.callId">
                        · 调用 {{ latestTraceEvent.callId.slice(0, 12) }}</template
                      >
                      · {{ latestTraceEvent.createdAt }}</small
                    >
                  </div>
                  <div v-if="liveTrace" class="live-chain-metrics">
                    <article>
                      <span>模型请求</span
                      ><strong>{{ liveTrace.summary.llmRequests }}</strong>
                    </article>
                    <article>
                      <span>工具调用 / 返回</span
                      ><strong
                        >{{ liveTrace.summary.toolCallCount }} /
                        {{ liveTrace.summary.toolResultCount }}</strong
                      >
                    </article>
                    <article>
                      <span>Agent</span
                      ><strong>{{ liveTrace.summary.agentCount }}</strong>
                    </article>
                    <article>
                      <span>总 Token</span
                      ><strong>{{
                        formatNumber(liveTrace.summary.totalTokens)
                      }}</strong>
                    </article>
                  </div>
                  <div
                    v-if="liveTrace?.summary.tools.length"
                    class="live-chain-tools"
                  >
                    <span
                      v-for="tool in liveTrace.summary.tools"
                      :key="tool.name"
                      ><Wrench :size="11" /><b>{{ tool.name }}</b
                      ><em>{{ tool.calls }} 调用 / {{ tool.results }} 返回</em></span
                    >
                  </div>
                  <div v-if="recentTraceEvents.length" class="live-chain-events">
                    <article
                      v-for="(event, index) in recentTraceEvents"
                      :key="event.id"
                      :class="event.eventType"
                    >
                      <header>
                        <span>{{ traceEventLabel(event.eventType) }}</span>
                        <strong>{{ traceEventTitle(event) }}</strong>
                        <em
                          >Agent {{ traceSession(event.sessionId) }} ·
                          {{ event.status || event.role || "recorded" }}</em
                        >
                        <time>{{ event.createdAt }}</time>
                      </header>
                      <details v-if="event.detail" :open="index === 0">
                        <summary>
                          {{
                            event.eventType === "function_call"
                              ? "查看调用参数"
                              : event.eventType === "function_call_output"
                                ? "查看返回摘要"
                                : "查看阶段摘要"
                          }}
                          <span v-if="event.detailTruncated">· 限长预览</span>
                        </summary>
                        <pre>{{ event.detail }}</pre>
                      </details>
                    </article>
                  </div>
                  <div v-else class="empty-inline live-chain-empty">
                    {{
                      liveTraceBusy
                        ? "正在读取 Strix 结构化事件…"
                        : "尚无工具事件。Token 增长但没有新的工具结果会被判定为无进展，并自动停止当前 URL。"
                    }}
                  </div>
                  <p class="live-chain-note">
                    “前端证据片段”是 Web JavaScript
                    的限长局部内容，不是代码审计任务。静态框架页现在不会再向 Strix
                    下发这些片段。
                  </p>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Fingerprint :size="16" />
                    <div>
                      <strong>技术指纹</strong
                      ><small
                        >已将识别结果拆分为名称、版本、置信度和依据，不再显示对象
                        JSON。</small
                      >
                    </div>
                  </div>
                  <div
                    v-if="Object.keys(fingerprint).length"
                    class="fingerprint-grid"
                  >
                    <article v-for="card in fingerprintCards" :key="card.key">
                      <span>{{ card.label }}</span
                      ><strong>{{ displayName(card.data) }}</strong
                      ><small>{{ displayVersion(card.data) }}</small
                      ><em>{{ card.data?.confidence || "未知置信度" }}</em>
                      <p v-if="card.data?.evidence?.length">
                        依据：{{ card.data.evidence.join("、") }}
                      </p>
                    </article>
                  </div>
                  <div v-else class="empty-inline">未解析到技术指纹</div>
                </section>
                <section v-if="selected.previousScanId" class="result-block">
                  <div class="block-title">
                    <RefreshCw :size="16" />
                    <div>
                      <strong>与上一次扫描对比</strong
                      ><small
                        >基于
                        URL、结果类型和稳定记录键比较；内容变化单独计数。</small
                      >
                    </div>
                  </div>
                  <div class="comparison-grid">
                    <article>
                      <span>新增</span><strong>{{ comparison.added }}</strong>
                    </article>
                    <article>
                      <span>消失</span><strong>{{ comparison.removed }}</strong>
                    </article>
                    <article>
                      <span>发生变化</span
                      ><strong>{{ comparison.changed }}</strong>
                    </article>
                    <article>
                      <span>保持不变</span
                      ><strong>{{ comparison.unchanged }}</strong>
                    </article>
                  </div>
                </section>
                <section v-if="selected.totalTokens" class="result-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>Token 消耗</strong
                      ><small
                        >{{ scanTypeLabel(selected.scanType) }} ·
                        {{ selected.skillNames || "默认扫描策略" }}</small
                      >
                    </div>
                  </div>
                  <div class="comparison-grid token-grid">
                    <article>
                      <span>请求次数</span
                      ><strong>{{ formatNumber(selected.llmRequests) }}</strong>
                    </article>
                    <article>
                      <span>输入 Token</span
                      ><strong>{{ formatNumber(selected.inputTokens) }}</strong>
                    </article>
                    <article>
                      <span>输出 Token</span
                      ><strong>{{
                        formatNumber(selected.outputTokens)
                      }}</strong>
                    </article>
                    <article>
                      <span>缓存 Token</span
                      ><strong>{{
                        formatNumber(selected.cachedTokens)
                      }}</strong>
                    </article>
                    <article>
                      <span>新增输入 Token</span
                      ><strong>{{
                        formatNumber(uncachedInput(selected))
                      }}</strong>
                    </article>
                    <article>
                      <span>输入 + 输出总计</span
                      ><strong>{{ formatNumber(scanTokenTotal(selected)) }}</strong>
                    </article>
                  </div>
                </section>
                <section v-if="scanAttempts.length" class="result-block attempt-ledger-block">
                  <div class="block-title">
                    <RefreshCw :size="16" />
                    <div><strong>执行尝试与增量成本</strong><small>重新扫描会继续当前任务；每次尝试的阶段、停止原因和新增 Token 独立保留。</small></div>
                  </div>
                  <div class="attempt-ledger">
                    <article v-for="attempt in scanAttempts" :key="attempt.attemptNumber" :class="[`attempt-${attempt.status}`, { current: attempt.attemptNumber === selected.attemptCount }]">
                      <header><span>第 {{attempt.attemptNumber}} 次</span><b>{{attemptStageLabel(attempt.stage)}}</b><em class="status-chip" :class="attempt.status">{{statusLabel(attempt.status)}}</em></header>
                      <p>{{attempt.checkpoint || '尚无阶段详情'}}</p>
                      <div class="attempt-cost"><span>请求 <b>{{formatNumber(attempt.llmRequests)}}</b></span><span>输入 <b>{{formatNumber(attempt.inputTokens)}}</b></span><span>缓存 <b>{{formatNumber(attempt.cachedTokens)}}</b></span><span>输出 <b>{{formatNumber(attempt.outputTokens)}}</b></span><span>本次总计 <b>{{formatNumber(attempt.totalTokens)}}</b></span></div>
                      <small>{{attemptTime(attempt)}}</small><code v-if="attempt.workDir" :title="attempt.workDir">{{attempt.workDir}}</code><mark v-if="attemptEndReason(attempt)">结束说明：{{attemptEndReason(attempt)}}</mark>
                    </article>
                  </div>
                </section>
                <section v-if="evidenceChainRows.length" class="result-block evidence-chain-block">
                  <div class="block-title">
                    <Layers3 :size="16" />
                    <div><strong>接口证据链</strong><small>把前端解析、运行时请求、端点验证、机会评分与漏洞结果合并到同一行；不再需要在多个页签之间手工拼接。</small></div>
                  </div>
                  <div class="evidence-chain-table">
                    <div class="table-head"><span>方法</span><span>接口</span><span>来源 / 参数</span><span>验证</span><span>价值</span><span>漏洞</span></div>
                    <div v-for="row in evidenceChainRows" :key="row.key">
                      <b class="method-badge" :class="methodTone(row.method)">{{row.method}}</b>
                      <code :title="row.url">{{row.url}}</code>
                      <span><em>{{row.sources.join(' · ')}}</em><small v-if="row.parameters.length">参数：{{row.parameters.join('、')}}</small><small v-else>尚未还原参数</small></span>
                      <span><b v-if="row.verified" class="evidence-verified">HTTP {{row.statusCode || '已响应'}}</b><i v-else>仅候选</i></span>
                      <strong :class="{ valuable: row.opportunityScore >= 65 }">{{row.opportunityScore || '—'}}</strong>
                      <button :class="{ risk: row.vulnerabilities }" @click="resultTab='vulnerabilities'">{{row.vulnerabilities}}</button>
                    </div>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Shield :size="16" />
                    <div>
                      <strong>风险概况</strong
                      ><small
                        >统计采用人工验证后的等级；误报不再计入风险。</small
                      >
                    </div>
                  </div>
                  <div class="risk-overview">
                    <article>
                      <span>严重 / 高危</span
                      ><strong class="severity-critical">{{
                        vulnerabilityRows.filter((v) =>
                          ["critical", "high"].includes(effectiveSeverity(v)),
                        ).length
                      }}</strong>
                    </article>
                    <article>
                      <span>中危</span
                      ><strong class="severity-medium">{{
                        vulnerabilityRows.filter(
                          (v) => effectiveSeverity(v) === "medium",
                        ).length
                      }}</strong>
                    </article>
                    <article>
                      <span>低危 / 信息</span
                      ><strong>{{
                        vulnerabilityRows.filter((v) =>
                          ["low", "info"].includes(effectiveSeverity(v)),
                        ).length
                      }}</strong>
                    </article>
                    <article>
                      <span>误报 / 无风险</span
                      ><strong>{{
                        vulnerabilityRows.filter(
                          (v) => effectiveSeverity(v) === "none",
                        ).length
                      }}</strong>
                    </article>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div>
                      <strong>关键信息</strong
                      ><small>按 URL 汇总，已应用人工确认后的风险等级。</small>
                    </div>
                  </div>
                  <div class="key-finding-list">
                    <button
                      v-for="item in vulnerabilityRows.slice(0, 5)"
                      :key="item.id"
                      @click="resultTab = 'vulnerabilities'"
                    >
                      <span
                        :class="`severity-dot ${effectiveSeverity(item)}`"
                      ></span
                      ><strong>{{
                        item.title || json(item.recordJson).title
                      }}</strong
                      ><em>{{ severityLabel(effectiveSeverity(item)) }}</em>
                    </button>
                    <div v-if="!vulnerabilityRows.length" class="empty-inline">
                      当前 URL 暂无漏洞记录
                    </div>
                  </div>
                </section>
              </div>

              <div
                v-else-if="resultTab === 'investigation'"
                class="result-section-stack"
              >
                <InvestigationGraphPanel
                  :graph="investigationGraph"
                  :busy="investigationBusy"
                  :updating-id="investigationUpdatingId"
                  @status="updateInvestigationStatus"
                  @approval="updateInvestigationApproval"
                />
              </div>

              <div
                v-else-if="resultTab === 'opportunities'"
                class="result-section-stack"
              >
                <section class="result-block result-opportunity-panel">
                  <div class="block-title">
                    <ShieldAlert :size="16" />
                    <div>
                      <strong>为什么值得继续，以及下一步做什么</strong>
                      <small>机会卡来自运行时请求、路由、指纹与本地知识匹配；不会伪装成漏洞结论。</small>
                    </div>
                  </div>
                  <div class="opportunity-list detail-opportunity-list">
                    <article
                      v-for="item in selectedUrlOpportunities"
                      :key="item.id"
                      class="opportunity-card"
                      :class="`score-${item.score >= 80 ? 'high' : item.score >= 65 ? 'medium' : 'low'}`"
                    >
                      <div class="opportunity-score"><strong>{{ item.score }}</strong><small>价值分</small></div>
                      <div class="opportunity-content">
                        <header>
                          <span>{{ opportunityCategoryLabel(item.category) }}</span>
                          <em :class="`opportunity-status ${item.status}`">{{ opportunityStatusLabel(item.status) }}</em>
                          <small>{{ item.source }}</small>
                        </header>
                        <h4>{{ item.title }}</h4>
                        <code class="opportunity-endpoint">{{ opportunityEndpoint(item) }}</code>
                        <ul><li v-for="reason in item.why" :key="reason">{{ reason }}</li></ul>
                        <div v-if="opportunityParameters(item).length" class="opportunity-params">
                          <span>已还原参数</span><code v-for="parameter in opportunityParameters(item)" :key="parameter">{{ parameter }}</code>
                        </div>
                        <div v-if="opportunityKnowledge(item).length" class="opportunity-knowledge">
                          <Fingerprint :size="14" /> 本地知识：{{ opportunityKnowledgeTitles(item) }}
                        </div>
                        <section class="opportunity-next-step">
                          <strong>{{ item.recommendedAction?.label || '查看证据并选择验证方法' }}</strong>
                          <ol>
                            <li v-for="step in item.recommendedAction?.steps || []" :key="step">{{ step }}</li>
                          </ol>
                        </section>
                        <details>
                          <summary>原始机会记录 / 请求上下文</summary>
                          <pre>{{ JSON.stringify(item.record, null, 2) }}</pre>
                        </details>
                        <footer>
                          <span>{{ item.lastSeen }}</span>
                          <div>
                            <button class="button secondary small" @click="setOpportunityStatus(item, 'in_progress')">标记调查中</button>
                            <button class="button ghost small" @click="setOpportunityStatus(item, 'validated')">完成验证</button>
                            <button class="button ghost small" @click="setOpportunityStatus(item, 'exhausted')">无新增证据</button>
                          </div>
                        </footer>
                      </div>
                    </article>
                    <div v-if="!selectedUrlOpportunities.length" class="empty-state">
                      此 URL 暂无机会卡。旧任务需要重新执行前端侦察后才会生成自动探索与机会数据。
                    </div>
                  </div>
                </section>
              </div>

              <div
                v-else-if="resultTab === 'fingerprint'"
                class="result-section-stack"
              >
                <section class="result-block kind-fingerprint">
                  <div class="block-title">
                    <Server :size="16" />
                    <div>
                      <strong>技术栈详情</strong
                      ><small
                        >名称和证据已规范化；“未识别”表示没有足够证据，不等于不存在。</small
                      >
                    </div>
                  </div>
                  <div class="fingerprint-detail-list">
                    <article
                      v-for="card in fingerprintCards"
                      :key="card.key"
                      :class="`tone-${card.key}`"
                    >
                      <header>
                        <span>{{ card.label }}</span
                        ><strong
                          >{{ displayName(card.data) }}
                          <small>{{ displayVersion(card.data) }}</small></strong
                        ><em>{{ card.data?.confidence || "unknown" }}</em>
                      </header>
                      <div
                        v-if="card.data?.libraries?.length"
                        class="fingerprint-tags"
                      >
                        <span
                          v-for="library in card.data.libraries"
                          :key="library.name"
                          >{{ library.name }} {{ library.version || "" }}</span
                        >
                      </div>
                      <div
                        v-if="card.data?.buildTools?.length"
                        class="fingerprint-tags"
                      >
                        <span
                          v-for="tool in card.data.buildTools"
                          :key="tool"
                          >{{ tool }}</span
                        >
                      </div>
                      <p v-if="card.data?.evidence?.length">
                        识别依据：{{ card.data.evidence.join("；") }}
                      </p>
                    </article>
                    <article v-if="techStack.baseUrls?.length" class="tone-api">
                      <header>
                        <span>API 基础地址</span
                        ><strong>{{ techStack.baseUrls.length }} 个</strong>
                      </header>
                      <div class="fingerprint-tags">
                        <span v-for="url in techStack.baseUrls" :key="url">{{
                          url
                        }}</span>
                      </div>
                    </article>
                  </div>
                </section>
                <section
                  v-if="Object.keys(wordpress).length"
                  class="result-block"
                >
                  <div class="block-title">
                    <Fingerprint :size="16" />
                    <div>
                      <strong>WordPress</strong
                      ><small>版本、插件、主题与入口</small>
                    </div>
                  </div>
                  <div class="wordpress-summary">
                    <article>
                      <span>核心版本</span
                      ><strong>{{ text(wordpress.version) }}</strong>
                    </article>
                    <article>
                      <span>主题</span
                      ><strong
                        >{{ text(wordpress.theme?.name) }}
                        {{ text(wordpress.theme?.version) }}</strong
                      >
                    </article>
                    <article>
                      <span>插件</span
                      ><strong>{{ wordpress.plugins?.length || 0 }}</strong>
                    </article>
                    <article>
                      <span>REST / XML-RPC</span
                      ><strong
                        >{{
                          wordpress.restApiEnabled ? "REST 开启" : "REST 未知"
                        }}
                        ·
                        {{
                          wordpress.xmlrpcEnabled
                            ? "XML-RPC 开启"
                            : "XML-RPC 关闭"
                        }}</strong
                      >
                    </article>
                  </div>
                  <div class="plugin-list">
                    <span
                      v-for="plugin in wordpress.plugins || []"
                      :key="plugin.name"
                      ><b>{{ plugin.name }}</b
                      >{{ plugin.version || "未知版本" }}</span
                    >
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Shield :size="16" />
                    <div>
                      <strong>安全响应头</strong
                      ><small>橙色表示配置缺失，不直接判定为漏洞。</small>
                    </div>
                  </div>
                  <div class="security-header-table">
                    <div class="table-head">
                      <span>响应头</span><span>状态</span><span>当前值</span
                      ><span>修复建议</span>
                    </div>
                    <div v-for="row in securityHeaders" :key="row.item.id">
                      <strong>{{ row.item.title }}</strong
                      ><span
                        :class="
                          row.data.present ? 'config-ok' : 'config-missing'
                        "
                        >{{ row.data.present ? "已配置" : "缺失" }}</span
                      ><code>{{ row.data.value || "—" }}</code>
                      <p>{{ row.data.recommendation || "—" }}</p>
                    </div>
                    <div v-if="!securityHeaders.length" class="empty-inline">
                      没有安全头数据
                    </div>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Layers3 :size="16" />
                    <div><strong>Cookie / 外部服务 / 信息披露</strong></div>
                  </div>
                  <div class="compact-record-grid">
                    <article
                      v-for="item in rows(
                        'cookie',
                        'external_service',
                        'info_disclosure',
                        'open_port',
                      )"
                      :key="item.id"
                    >
                      <span>{{ kindLabel(item.kind) }}</span
                      ><strong>{{ item.title || item.recordKey }}</strong>
                      <pre>{{
                        JSON.stringify(json(item.recordJson), null, 2)
                      }}</pre>
                    </article>
                    <div
                      v-if="
                        !rows(
                          'cookie',
                          'external_service',
                          'info_disclosure',
                          'open_port',
                        ).length
                      "
                      class="empty-inline"
                    >
                      暂无记录
                    </div>
                  </div>
                </section>
              </div>

              <div v-else-if="resultTab === 'api'" class="result-section-stack">
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
                    当前是旧扫描记录或页面没有可触发控件；重新运行自动调查后会生成轨迹。
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
                    当前是旧扫描记录；重新运行自动调查后会从 CDP ExtraInfo 和业务 JS 生成请求头证据。
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
                <section class="result-block kind-api">
                  <div class="block-title">
                    <Code2 :size="16" />
                    <div>
                      <strong>API 列表</strong
                      ><small
                        >优先展示 AST
                        还原后的完整接口；动态变量保留为占位符，不再盲目拼接当前
                        URL。</small
                      >
                    </div>
                  </div>
                  <div class="api-table">
                    <div class="table-head">
                      <span>方法</span><span>接口</span><span>来源</span
                      ><span>参数 / 说明</span>
                    </div>
                    <div
                      v-for="item in apiRows"
                      :key="item.id"
                      :class="{ 'registration-api-row': isRegistrationApi(item) }"
                    >
                      <b
                        class="method-badge"
                        :class="methodTone(json(item.recordJson).method)"
                        >{{ json(item.recordJson).method || "UNKNOWN" }}</b
                      >
                      <div class="long-value-cell">
                        <code class="scroll-value" :title="apiUrl(item)">{{
                          apiUrl(item)
                        }}</code>
                        <strong
                          v-if="isRegistrationApi(item)"
                          class="registration-inline-badge"
                          >注册入口</strong
                        >
                        <button
                          class="icon-button compact"
                          title="复制完整接口"
                          @click="copyText(apiUrl(item))"
                        >
                          <ClipboardCopy :size="13" /></button
                      ><small
                          >{{ json(item.recordJson).confidence || "unknown" }} ·
                          {{
                            json(item.recordJson).extractionEngine || "legacy"
                          }}<template v-if="json(item.recordJson).reconstructionConfidence">
                            · 重组置信度 {{ Math.round(Number(json(item.recordJson).reconstructionConfidence) * 100) }}%</template
                          ></small
                        >
                        <strong
                          v-if="json(item.recordJson).candidateOnly"
                          class="registration-inline-badge reconstruction-candidate-badge"
                          >候选 · 待请求验证</strong
                        >
                      </div>
                      <div class="long-value-cell source">
                        <code
                          class="scroll-value"
                          :title="json(item.recordJson).source"
                          >{{ json(item.recordJson).source || "—" }}</code
                        >
                      </div>
                      <p>
                        <template v-if="json(item.recordJson).apiPrefix || json(item.recordJson).businessEndpoint">
                          前缀 {{ json(item.recordJson).apiPrefix || "/" }} · 业务路径
                          {{ json(item.recordJson).businessEndpoint || json(item.recordJson).path }}
                          <br />
                        </template>
                        {{ json(item.recordJson).note || (text(json(item.recordJson).parameters) ? `请求参数：${text(json(item.recordJson).parameters)}` : "") || json(item.recordJson).evidence }}
                        <template v-if="json(item.recordJson).responseKeys?.length">
                          · 响应字段：{{ text(json(item.recordJson).responseKeys) }}
                        </template>
                        <template v-if="apiRequestHeaderNames(item).length">
                          <br />请求头：{{ text(apiRequestHeaderNames(item)) }}
                          <b v-if="json(item.recordJson).extraRequestHeaderNames?.length" class="hidden-header-inline">含 {{ json(item.recordJson).extraRequestHeaderNames.length }} 个 ExtraInfo 补全头</b>
                        </template>
                        <template v-if="json(item.recordJson).initiator?.url">
                          <br />发起位置：{{ json(item.recordJson).initiator.functionName || 'anonymous' }} ·
                          {{ json(item.recordJson).initiator.url }}:{{ Number(json(item.recordJson).initiator.lineNumber || 0) + 1 }}
                        </template>
                      </p>
                    </div>
                    <div v-if="!apiRows.length" class="empty-inline">
                      没有发现可信 API；运行期 Hook 建议会显示在下方。
                    </div>
                  </div>
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
                        >只记录静态证据提示；是否启动浏览器 Hook 由 Strix
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

              <div
                v-else-if="resultTab === 'endpoints'"
                class="result-section-stack"
              >
                <section class="result-block">
                  <div class="block-title">
                    <Network :size="16" />
                    <div>
                      <strong>已验证端点</strong
                      ><small>状态码颜色只表示 HTTP 响应状态。</small>
                    </div>
                  </div>
                  <div class="endpoint-table">
                    <div class="table-head">
                      <span>状态</span><span>方法</span><span>完整 URL</span
                      ><span>来源</span><span>耗时 / 大小</span
                      ><span>说明</span>
                    </div>
                    <div v-for="item in endpointRows" :key="item.id">
                      <b
                        :class="`http-status ${statusTone(json(item.recordJson).statusCode)}`"
                        >{{ json(item.recordJson).statusCode || "—" }}</b
                      ><span class="method-badge">{{
                        json(item.recordJson).method || "GET"
                      }}</span
                      ><code>{{
                        endpointUrl(
                          selectedUrl,
                          json(item.recordJson).url ||
                            json(item.recordJson).path ||
                            "/",
                        )
                      }}</code
                      ><span>{{
                        json(item.recordJson).source || kindLabel(item.kind)
                      }}</span
                      ><span
                        >{{ json(item.recordJson).responseTime || "—" }} ms ·
                        {{ json(item.recordJson).bodyLength || "—" }} B</span
                      >
                      <p>
                        {{
                          json(item.recordJson).note ||
                          json(item.recordJson).detail ||
                          json(item.recordJson).bodySnippet ||
                          "—"
                        }}
                      </p>
                    </div>
                    <div v-if="!endpointRows.length" class="empty-inline">
                      没有端点验证数据
                    </div>
                  </div>
                </section>
              </div>

              <div v-else class="result-section-stack">
                <section class="result-block">
                  <div class="block-title">
                    <Bug :size="16" />
                    <div>
                      <strong>漏洞发现</strong
                      ><small
                        >原始等级和人工确认等级分开记录，页面统计采用确认后的等级。</small
                      >
                    </div>
                  </div>
                  <div class="vulnerability-master-detail">
                    <aside class="vulnerability-index-list">
                      <button
                        v-for="item in vulnerabilityRows"
                        :key="`vuln-index-${item.id}`"
                        :class="{ active: selectedFindingId === item.id }"
                        @click="selectedFindingId = item.id"
                      >
                        <span :class="`severity-badge ${effectiveSeverity(item)}`">{{ severityLabel(effectiveSeverity(item)) }}</span>
                        <div><strong>{{ item.title || json(item.recordJson).title || item.recordKey }}</strong><small>{{ json(item.recordJson).method || "GET" }} {{ json(item.recordJson).url || "/" }}</small></div>
                        <em v-if="validationFor(item)" :class="`validation-chip ${validationFor(item)?.verdict}`">{{ verdictLabel(validationFor(item)?.verdict || "") }}</em>
                      </button>
                      <div v-if="!vulnerabilityRows.length" class="empty-inline">当前 URL 没有漏洞记录</div>
                    </aside>
                  <div class="vulnerability-list focused">
                    <article
                      v-for="item in focusedVulnerabilityRows"
                      :key="item.id"
                      :class="`vuln-card severity-border-${effectiveSeverity(item)}`"
                    >
                      <header>
                        <span
                          :class="`severity-badge ${effectiveSeverity(item)}`"
                          >{{ severityLabel(effectiveSeverity(item)) }}</span
                        >
                        <div>
                          <strong>{{
                            item.title ||
                            json(item.recordJson).title ||
                            item.recordKey
                          }}</strong
                          ><small
                            >{{
                              json(item.recordJson).source === "strix"
                                ? "STRIX · "
                                : ""
                            }}{{
                              json(item.recordJson).type || "vulnerability"
                            }}
                            · 原始等级
                            {{ severityLabel(safeSeverity(item.severity)) }} ·
                            CVSS {{ json(item.recordJson).cvss ?? "—" }} ·
                            {{ json(item.recordJson).method || "GET" }}
                            {{ json(item.recordJson).url || "/" }}</small
                          ><small
                            v-if="
                              json(item.recordJson).cve ||
                              json(item.recordJson).cwe
                            "
                            >{{ json(item.recordJson).cve || "无 CVE" }} ·
                            {{ json(item.recordJson).cwe || "无 CWE" }} ·
                            修复工作量
                            {{
                              json(item.recordJson).fix_effort || "未知"
                            }}</small
                          >
                        </div>
                        <span
                          v-if="validationFor(item)"
                          :class="`validation-chip ${validationFor(item)?.verdict}`"
                          ><CheckCircle2 :size="13" />{{
                            verdictLabel(validationFor(item)?.verdict || "")
                          }}
                          · {{ severityLabel(effectiveSeverity(item)) }}</span
                        >
                      </header>
                      <div class="vuln-columns">
                        <div>
                          <span>漏洞描述</span>
                          <p>{{ json(item.recordJson).description || "—" }}</p>
                        </div>
                        <div>
                          <span>技术分析</span>
                          <p>
                            {{
                              json(item.recordJson).technical_analysis || "—"
                            }}
                          </p>
                        </div>
                        <div>
                          <span>证据</span>
                          <pre>{{ text(json(item.recordJson).evidence) }}</pre>
                        </div>
                        <div>
                          <span>影响</span>
                          <p>
                            {{
                              json(item.recordJson).impact ||
                              json(item.recordJson).detail ||
                              "—"
                            }}
                          </p>
                        </div>
                        <div>
                          <span>修复建议</span>
                          <p>
                            {{
                              json(item.recordJson).recommendation ||
                              json(item.recordJson).remediation_steps ||
                              "—"
                            }}
                          </p>
                        </div>
                        <div>
                          <span>PoC / 复现</span>
                          <pre>{{
                            json(item.recordJson).pocRequest ||
                            json(item.recordJson).poc_description ||
                            "—"
                          }}</pre>
                        </div>
                        <div v-if="json(item.recordJson).cvss_breakdown">
                          <span>CVSS 明细</span>
                          <pre>{{
                            JSON.stringify(
                              json(item.recordJson).cvss_breakdown,
                              null,
                              2,
                            )
                          }}</pre>
                        </div>
                        <div v-if="json(item.recordJson).code_locations">
                          <span>代码位置 / 修复差异</span>
                          <pre>{{
                            JSON.stringify(
                              json(item.recordJson).code_locations,
                              null,
                              2,
                            )
                          }}</pre>
                        </div>
                        <div v-if="json(item.recordJson).assumptions">
                          <span>前提与限制</span>
                          <p>{{ json(item.recordJson).assumptions }}</p>
                        </div>
                        <div v-if="json(item.recordJson).dependency_metadata">
                          <span>依赖信息</span>
                          <pre>{{
                            JSON.stringify(
                              json(item.recordJson).dependency_metadata,
                              null,
                              2,
                            )
                          }}</pre>
                        </div>
                      </div>
                      <footer>
                        <button
                          class="button primary compact"
                          @click="editValidation(item)"
                        >
                          <ClipboardCheck :size="13" />{{
                            validationFor(item)
                              ? "修改验证结论"
                              : "开始人工验证"
                          }}</button
                        ><span
                          v-if="validationFor(item)"
                          class="validation-saved-note"
                          >已保存：{{
                            verdictLabel(validationFor(item)?.verdict || "")
                          }}
                          / {{ severityLabel(effectiveSeverity(item)) }}</span
                        >
                      </footer>
                      <div
                        v-if="validationEditor?.id === item.id"
                        class="validation-editor inline-validation-editor"
                      >
                        <div class="block-title">
                          <ClipboardCheck :size="16" />
                          <div>
                            <strong
                              >人工验证：{{
                                item.title || item.recordKey
                              }}</strong
                            ><small>保存后立即更新当前卡片与统计</small>
                          </div>
                          <button
                            class="icon-button"
                            @click="validationEditor = undefined"
                          >
                            <X :size="15" />
                          </button>
                        </div>
                        <div class="verdict-picker">
                          <button
                            v-for="choice in [
                              { v: 'true_positive', l: '真实漏洞' },
                              { v: 'false_positive', l: '误报' },
                              { v: 'needs_more', l: '需要补证' },
                            ]"
                            :key="choice.v"
                            :class="{
                              active: validationForm.verdict === choice.v,
                            }"
                            @click="validationForm.verdict = choice.v"
                          >
                            {{ choice.l }}
                          </button>
                        </div>
                        <div class="validation-form-grid">
                          <label class="field"
                            ><span>确认后严重度</span
                            ><select v-model="validationForm.severity">
                              <option value="critical">严重</option>
                              <option value="high">高危</option>
                              <option value="medium">中危</option>
                              <option value="low">低危</option>
                              <option value="info">信息</option>
                            </select></label
                          ><label class="field"
                            ><span>验证备注</span
                            ><textarea
                              v-model="validationForm.note"
                              rows="3"
                              placeholder="复现过程、判断理由、限制条件"
                            ></textarea></label
                          ><label class="field span-two"
                            ><span>证据 / 请求响应 / 截图路径</span
                            ><textarea
                              v-model="validationForm.evidence"
                              rows="5"
                              placeholder="粘贴关键请求响应，或填写本地证据文件路径"
                            ></textarea>
                          </label>
                        </div>
                        <footer>
                          <button
                            class="button ghost"
                            @click="validationEditor = undefined"
                          >
                            取消</button
                          ><button
                            class="button primary"
                            @click="saveValidation"
                          >
                            <Save :size="14" />保存并更新风险
                          </button>
                        </footer>
                      </div>
                    </article>
                    <div v-if="!vulnerabilityRows.length" class="empty-inline">
                      当前 URL 没有漏洞记录
                    </div>
                  </div>
                  </div>
                </section>
                <section class="result-block">
                  <div class="block-title">
                    <Activity :size="16" />
                    <div><strong>PoC 测试记录</strong></div>
                  </div>
                  <div class="poc-list">
                    <article v-for="item in pocRows" :key="item.id">
                      <strong>{{
                        json(item.recordJson).name || item.title
                      }}</strong
                      ><span>{{
                        json(item.recordJson).result || "unknown"
                      }}</span
                      ><code
                        >{{ json(item.recordJson).method || "GET" }}
                        {{
                          endpointUrl(selectedUrl, json(item.recordJson).url)
                        }}</code
                      >
                      <p>
                        {{
                          json(item.recordJson).note ||
                          json(item.recordJson).responseSnippet ||
                          "—"
                        }}
                      </p>
                    </article>
                    <div v-if="!pocRows.length" class="empty-inline">
                      没有 PoC 测试记录
                    </div>
                  </div>
                </section>
              </div>
            </template></template
          >
        </main>
      </div>
    </template>

    <template v-else-if="tab === 'fuse'">
      <section class="panel fuse-zone-panel">
        <div class="panel-heading">
          <div>
            <span class="eyebrow">STOP &amp; DISPOSITION</span>
            <h3>停止与 URL 处置队列</h3>
            <p>
              这里处理“为什么停止、是否恢复”，不是判断漏洞真假。Strix 遇到拦截、成本失控或无进展时只停止当前 URL，其余队列继续执行。
            </p>
          </div>
          <div class="fuse-filters">
            <select v-model="fuseCategoryFilter" class="toolbar-select"><option value="all">全部停止原因</option><option value="budget">成本 / 无进展</option><option value="access">缺少访问条件</option><option value="blocked">遭到拦截</option><option value="failure">网络 / 执行异常</option><option value="low_value">价值不足</option></select>
            <select v-model="fuseFilter" class="toolbar-select"><option value="active">待处置</option><option value="archived">已归档</option><option value="all">全部</option></select>
          </div>
        </div>
        <div class="fuse-zone-stats">
          <article class="attention">
            <span>待决定是否恢复</span
            ><strong>{{
              fuseEntries.filter((item) => !item.archived && item.verdict === 'pending').length
            }}</strong>
          </article>
          <article>
            <span>补充条件后重试</span
            ><strong>{{ fuseEntries.filter((item) => !item.archived && item.verdict === 'needs_followup').length }}</strong>
          </article>
          <article>
            <span>已完成归档</span
            ><strong>{{
              fuseEntries.filter((item) => item.archived).length
            }}</strong>
          </article>
          <article>
            <span>当前显示</span
            ><strong>{{ visibleFuseEntries.length }}</strong>
          </article>
        </div>
        <div class="fuse-entry-list">
          <article
            v-for="item in visibleFuseEntries"
            :key="item.id"
            :class="{ archived: item.archived, attention: !item.archived && item.verdict === 'pending' }"
          >
            <header>
              <span class="fuse-icon"><ShieldAlert :size="16" /></span>
              <div>
                <strong>{{ item.company || "未提供公司" }}</strong>
                <div class="long-value-cell">
                  <code :title="item.url">{{ item.url }}</code
                  ><button
                    class="icon-button compact"
                    title="复制 URL"
                    @click="copyText(item.url)"
                  >
                    <ClipboardCopy :size="13" />
                  </button>
                </div>
              </div>
              <span :class="`fuse-verdict ${item.verdict}`">{{
                fuseVerdictLabel(item.verdict)
              }}</span>
            </header>
            <div class="fuse-disposition-callout">
              <span :class="`fuse-category category-${fuseReasonCategory(item)}`">{{ fuseCategoryLabel(fuseReasonCategory(item)) }}</span>
              <p><b>建议动作</b>{{ fuseRecommendedAction(item) }}</p>
            </div>
            <dl>
              <div class="fuse-reason-column">
                <dt>熔断原因</dt>
                <dd>
                  <div class="fuse-reason-parts">
                    <span
                      v-for="part in fuseReasonParts(item)"
                      :key="`${part.label}-${part.text}`"
                      :class="`reason-${part.tone}`"
                      ><b>{{ part.label }}</b
                      >{{ part.text }}</span
                    >
                  </div>
                </dd>
              </div>
              <div>
                <dt>来源任务</dt>
                <dd>
                  <code>{{ item.sourceScanId }}</code>
                </dd>
              </div>
              <div>
                <dt>更新时间</dt>
                <dd>{{ item.updatedAt }}</dd>
              </div>
            </dl>
            <div v-if="item.note || item.evidence" class="fuse-review-summary">
              <p>{{ item.note || "无备注" }}</p>
              <pre>{{ item.evidence || "无证据记录" }}</pre>
            </div>
            <footer>
              <button
                class="button secondary compact"
                @click="toggleFuseDetail(item)"
              >
                <Eye :size="13" />{{
                  fuseState(item).open ? "收起完整情报" : "查看完整情报"
                }}</button
              ><button class="button primary compact" @click="editFuse(item)">
                <ClipboardCheck :size="13" />{{
                  item.verdict === "pending" ? "记录处置决定" : "编辑处置记录"
                }}</button
              ><button
                v-if="!item.archived && item.verdict !== 'pending'"
                class="button ghost compact"
                @click="
                  editFuse(item);
                  saveFuse(true);
                "
              >
                <Archive :size="13" />完成并归档</button
              ><button
                class="button warning compact"
                @click="pendingFuseRemoval = item"
              >
                <RefreshCw :size="13" />恢复并自动重试
              </button>
            </footer>
            <section v-if="fuseState(item).open" class="fuse-intel-panel">
              <nav class="fuse-intel-tabs" aria-label="熔断目标情报分类">
                <button
                  v-for="entry in fuseDetailTabs"
                  :key="entry[0]"
                  :class="{ active: fuseState(item).tab === entry[0] }"
                  @click="fuseState(item).tab = entry[0]"
                >
                  {{ entry[1] }}
                </button>
              </nav>
              <div v-if="fuseState(item).loading" class="empty-inline">
                正在加载该 URL 的完整情报…
              </div>
              <template v-else>
                <div
                  v-if="fuseState(item).tab === 'summary'"
                  class="fuse-intel-summary"
                >
                  <dl>
                    <div>
                      <dt>URL</dt>
                      <dd class="scroll-value">{{ item.url }}</dd>
                    </div>
                    <div>
                      <dt>执行状态</dt>
                      <dd>
                        {{ statusLabel(fuseTarget(item)?.status || "limited") }}
                      </dd>
                    </div>
                    <div>
                      <dt>前端价值</dt>
                      <dd>{{ fuseTarget(item)?.valueScore ?? "—" }} / 100</dd>
                    </div>
                    <div>
                      <dt>扫描模式</dt>
                      <dd>
                        {{ (fuseTarget(item)?.scanMode || "—").toUpperCase() }}
                      </dd>
                    </div>
                    <div class="fuse-summary-reason">
                      <dt>熔断原因</dt>
                      <dd>
                        <div class="fuse-reason-parts compact">
                          <span
                            v-for="part in fuseReasonParts(item)"
                            :key="`summary-${part.label}-${part.text}`"
                            :class="`reason-${part.tone}`"
                            ><b>{{ part.label }}</b
                            >{{ part.text }}</span
                          >
                        </div>
                      </dd>
                    </div>
                    <div>
                      <dt>记录总数</dt>
                      <dd>
                        {{
                          fuseState(item).findings.filter((row) =>
                            sameTargetUrl(row.targetUrl, item.url),
                          ).length
                        }}
                      </dd>
                    </div>
                  </dl>
                  <div
                    v-for="row in fuseRows(
                      item,
                      'summary_target',
                      'risk_summary',
                    )"
                    :key="row.id"
                    class="fuse-intel-record"
                    :class="kindTone(row.kind)"
                  >
                    <strong>{{ row.title || kindLabel(row.kind) }}</strong>
                    <pre>{{
                      JSON.stringify(json(row.recordJson), null, 2)
                    }}</pre>
                  </div>
                </div>
                <div
                  v-else-if="fuseState(item).tab === 'fingerprint'"
                  class="fuse-intel-records"
                >
                  <div
                    v-for="row in fuseRows(
                      item,
                      'fingerprint',
                      'tech_stack',
                      'security_header',
                      'cookie',
                      'external_service',
                      'info_disclosure',
                      'open_port',
                    )"
                    :key="row.id"
                    class="fuse-intel-record"
                    :class="kindTone(row.kind)"
                  >
                    <header>
                      <span>{{ kindLabel(row.kind) }}</span
                      ><strong>{{ row.title || row.recordKey }}</strong>
                    </header>
                    <pre>{{
                      JSON.stringify(json(row.recordJson), null, 2)
                    }}</pre>
                  </div>
                  <div
                    v-if="
                      !fuseRows(
                        item,
                        'fingerprint',
                        'tech_stack',
                        'security_header',
                        'cookie',
                        'external_service',
                        'info_disclosure',
                        'open_port',
                      ).length
                    "
                    class="empty-inline"
                  >
                    没有指纹配置记录
                  </div>
                </div>
                <div
                  v-else-if="fuseState(item).tab === 'assets'"
                  class="fuse-intel-records"
                >
                  <div
                    v-for="row in fuseRows(
                      item,
                      'js_file',
                      'api',
                      'route',
                      'runtime_signal',
                      'crypto_signal',
                      'sensitive_info',
                    )"
                    :key="row.id"
                    class="fuse-intel-record"
                    :class="kindTone(row.kind)"
                  >
                    <header>
                      <span>{{ kindLabel(row.kind) }}</span
                      ><strong class="scroll-value">{{
                        row.title || row.recordKey
                      }}</strong>
                    </header>
                    <pre>{{
                      JSON.stringify(json(row.recordJson), null, 2)
                    }}</pre>
                  </div>
                  <div
                    v-if="
                      !fuseRows(
                        item,
                        'js_file',
                        'api',
                        'route',
                        'runtime_signal',
                        'crypto_signal',
                        'sensitive_info',
                      ).length
                    "
                    class="empty-inline"
                  >
                    没有 JS / API 情报
                  </div>
                </div>
                <div
                  v-else-if="fuseState(item).tab === 'endpoints'"
                  class="fuse-intel-records"
                >
                  <div
                    v-for="row in fuseRows(
                      item,
                      'endpoint',
                      'endpoint_expanded',
                      'directory_find',
                      'rest_endpoint',
                      'login_endpoint',
                      'parameter_json',
                      'parameter_xml',
                      'parameter_form',
                      'parameter_upload',
                      'parameter_path',
                      'parameter_query',
                    )"
                    :key="row.id"
                    class="fuse-intel-record"
                    :class="kindTone(row.kind)"
                  >
                    <header>
                      <span>{{ kindLabel(row.kind) }}</span
                      ><strong class="scroll-value">{{
                        row.title || row.recordKey
                      }}</strong>
                    </header>
                    <pre>{{
                      JSON.stringify(json(row.recordJson), null, 2)
                    }}</pre>
                  </div>
                  <div
                    v-if="
                      !fuseRows(
                        item,
                        'endpoint',
                        'endpoint_expanded',
                        'directory_find',
                        'rest_endpoint',
                        'login_endpoint',
                        'parameter_json',
                        'parameter_xml',
                        'parameter_form',
                        'parameter_upload',
                        'parameter_path',
                        'parameter_query',
                      ).length
                    "
                    class="empty-inline"
                  >
                    没有端点验证记录
                  </div>
                </div>
                <div v-else class="fuse-intel-records">
                  <div
                    v-for="row in fuseRows(
                      item,
                      'vulnerability',
                      'poc_test',
                      'risk_summary',
                    )"
                    :key="row.id"
                    class="fuse-intel-record"
                    :class="kindTone(row.kind)"
                  >
                    <header>
                      <span
                        >{{ kindLabel(row.kind) }} ·
                        {{ severityLabel(row.severity) }}</span
                      ><strong>{{ row.title || row.recordKey }}</strong>
                    </header>
                    <pre>{{
                      JSON.stringify(json(row.recordJson), null, 2)
                    }}</pre>
                  </div>
                  <div
                    v-for="row in fuseValidationRows(item)"
                    :key="`validation-${row.id}`"
                    class="fuse-intel-record validation"
                  >
                    <header>
                      <span>人工验证 · {{ verdictLabel(row.verdict) }}</span
                      ><strong>{{ row.findingKind }}</strong>
                    </header>
                    <p>{{ row.note || "无备注" }}</p>
                    <pre>{{ row.evidence || "无证据记录" }}</pre>
                  </div>
                  <div
                    v-if="
                      !fuseRows(
                        item,
                        'vulnerability',
                        'poc_test',
                        'risk_summary',
                      ).length && !fuseValidationRows(item).length
                    "
                    class="empty-inline"
                  >
                    没有漏洞或证明记录
                  </div>
                </div>
              </template>
            </section>
            <div v-if="fuseEditor?.id === item.id" class="fuse-review-editor">
              <label class="field"
                ><span>URL 处置状态</span
                ><select v-model="fuseForm.verdict">
                  <option value="pending">暂不决定</option>
                  <option value="manual_verified">已人工接管</option>
                  <option value="needs_followup">补充条件后重试</option>
                  <option value="not_reproducible">保持排除</option>
                </select></label
              >
              <label class="field"
                ><span>处置说明</span
                ><textarea
                  v-model="fuseForm.note"
                  rows="3"
                  placeholder="为什么停止、缺少什么条件、是否值得恢复"
                ></textarea>
              </label>
              <label class="field span-two"
                ><span>补充上下文 / 请求响应 / 证据路径</span
                ><textarea
                  v-model="fuseForm.evidence"
                  rows="5"
                  placeholder="记录关键请求响应或本地证据文件"
                ></textarea>
              </label>
              <footer>
                <button class="button ghost" @click="fuseEditor = undefined">
                  取消</button
                ><button
                  class="button primary"
                  :disabled="fuseBusy"
                  @click="saveFuse(false)"
                >
                  <Save :size="14" />保存处置</button
                ><button
                  class="button secondary"
                  :disabled="fuseBusy"
                  @click="saveFuse(true)"
                >
                  <Archive :size="14" />完成并归档
                </button>
              </footer>
            </div>
            <InlineConfirm
              v-if="pendingFuseRemoval?.id === item.id"
              title="恢复该 URL 并立即重试？"
              detail="会在原任务工作流中创建只包含该 URL 的续跑执行，复用已保存的前端证据；历史记录与累计成本都会保留。"
              :busy="fuseBusy"
              @cancel="pendingFuseRemoval = undefined"
              @confirm="removeFuse"
            />
          </article>
          <div v-if="!visibleFuseEntries.length" class="empty-state">
            当前没有匹配的熔断目标。
          </div>
        </div>
      </section>
    </template>

    <template v-else-if="tab === 'workbench'">
      <StrixTraceHub
        v-if="props.workbenchMode === 'traces'"
        @notify="(type, text) => emit('notify', type, text)"
      />
      <StrixWorkbench
        v-else
        :projects="props.projects"
        :project-id="props.projectId"
        :scans="scans"
        :initial-mode="props.workbenchMode as 'web' | 'code' | 'greybox' | 'cicd' | 'skills'"
        @notify="(type, text) => emit('notify', type, text)"
        @reload="load"
        @create-project="emit('create-project')"
        @open-scan="(scan) => openScan(scan)"
      />
    </template>

    <SentinelValidationWorkbench
      v-else-if="tab === 'validations'"
      :filter="validationFilter"
      :stats="validationWorkStats"
      :items="selectedValidationWorkItems"
      :editor="validationWorkEditor"
      :form="validationWorkForm"
      @update:filter="validationFilter = $event"
      @select="editValidationWorkItem"
      @evidence="openValidationEvidence"
      @save="saveValidationWorkItem"
    />

    <template v-else
      ><section class="panel sentinel-guide">
        <div class="guide-icon"><HelpCircle :size="23" /></div>
        <div>
          <h3>安全分析工作台</h3>
          <p>
            资产 URL 使用 Web 扫描；工作台提供代码审计、URL + 源码灰盒联测和
            CI/CD
            变更范围检查。所有结果统一进入任务、URL/源码目标、漏洞和人工验证页面。
          </p>
        </div>
      </section></template
    >
    <div
      v-if="pendingDelete"
      class="sentinel-confirm-backdrop"
      @click.self="pendingDelete = undefined"
    >
      <InlineConfirm
        class="sentinel-delete-confirm-modal"
        :title="
          `${['scanning', 'pausing'].includes(pendingDelete.status) ? '强制停止并删除' : '确认删除'}任务「${scanTitle(pendingDelete)}」？`
        "
        :detail="
          ['scanning', 'pausing'].includes(pendingDelete.status)
            ? '会先停止当前进程，再删除任务、结果和验证记录。'
            : '这是整任务删除：会同时删除该任务下全部真实 URL、公司归属、解析结果和人工验证记录；仅想去掉错误目标时请取消。'
        "
        :busy="deleting"
        @cancel="pendingDelete = undefined"
        @confirm="remove"
      />
    </div>
  </div>
</template>

<style src="../sentinel.css"></style>
