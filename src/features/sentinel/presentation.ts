import type { SentinelFinding, SentinelScan, SentinelScanAttempt } from "./types";
import { formatCompact, formatInteger, useI18n } from "../../i18n";

type Translator = (zh: string, en: string) => string;
const mapped = (map: Record<string, string>, value: string, fallback = value) =>
  map[value] || fallback;

// Label tables hold ["中文", "English"] pairs (or one language-neutral string)
// and resolve on every call, so a language switch applies without a reload.
// tr is only ever called inside functions, never while this module loads.
const { tr } = useI18n();
type LabelEntry = string | readonly [string, string];
const pairLabel = (map: Record<string, LabelEntry>, value: string, fallback = value, translate: Translator = tr) => {
  if (!Object.prototype.hasOwnProperty.call(map, value)) return fallback;
  const entry = map[value];
  return typeof entry === "string" ? entry : translate(entry[0], entry[1]);
};

export function createSentinelLabels(tr: Translator) {
  const statusLabels: Record<string, LabelEntry> = {
    draft: ["待确认", "Review"], queued: ["待扫描", "Queued"], frontend_recon: ["前端解析", "Frontend recon"], routed: ["已分流", "Routed"],
    recon_only: ["仅完成确定性侦察", "Recon only"], manual_review: ["复杂前端·人工复核", "Manual review"], scanning: ["扫描中", "Scanning"],
    pausing: ["正在停止", "Pausing"], limited: ["已停止", "Stopped"], fuse_excluded: ["已停止目标排除", "Excluded as stopped"], deferred: ["已延后", "Deferred"],
    completed: ["已完成", "Completed"], partial: ["待补充验证", "Needs validation"], failed: ["失败", "Failed"], paused: ["已暂停", "Paused"], imported: ["已导入", "Imported"],
    completed_with_gaps: ["已完成·存在覆盖缺口", "Completed with gaps"], protected_stop: ["受保护停止", "Protected stop"],
    resume_incompatible: ["续跑状态不兼容·需重新执行", "Resume incompatible · re-run required"],
    persistence_failure: ["本地记录失败·已停止以避免重复消耗", "Local record failed · stopped to avoid double spend"], cancelled: ["已取消", "Cancelled"],
  };
  const verdictLabels: Record<string, LabelEntry> = {
    true_positive: ["真实漏洞", "Confirmed"], false_positive: ["误报", "False positive"], needs_more: ["需补证", "More evidence"], pending: ["未验证", "Pending"],
  };
  const severityLabels: Record<string, LabelEntry> = {
    critical: ["严重", "Critical"], high: ["高危", "High"], medium: ["中危", "Medium"], low: ["低危", "Low"], info: ["信息", "Info"], none: ["无风险", "No risk"], unknown: ["未知", "Unknown"],
  };
  return {
    statusLabel: (value: string) => pairLabel(statusLabels, value, value, tr),
    retryActionLabel: (scan: SentinelScan) =>
      scan.status === "completed" ? tr("再次扫描", "Scan again") : tr("重试未完成阶段", "Retry incomplete stages"),
    verdictLabel: (value: string) => pairLabel(verdictLabels, value, value, tr),
    // An empty severity reads as "info"; an unknown one is shown as-is.
    severityLabel: (value: string) => pairLabel(severityLabels, value || "info", value, tr),
    scanTypeLabel: (value: string) => mapped({
      web: "Web", code: tr("代码审计", "Code audit"), greybox: tr("灰盒", "Grey-box"), cicd: "CI/CD",
    }, value || "web"),
    llmDeploymentLabel: (scan: SentinelScan) => {
      if (scan.llmDeployment === "local") {
        return scan.llmFullPower
          ? tr("本地模型 · 火力全开", "Local model · Full power")
          : tr("本地模型", "Local model");
      }
      return scan.llmDeployment === "cloud" ? tr("云端 AI", "Cloud AI") : tr("模型未记录", "Model not recorded");
    },
  };
}

export const scanTitle = (scan: SentinelScan) => scan.taskName || scan.projectName || tr("未命名任务", "Untitled task");
export const llmDeploymentClass = (scan: SentinelScan) =>
  scan.llmDeployment === "local" ? "local" : scan.llmDeployment === "cloud" ? "cloud" : "unknown";

export const routeModeLabel = (value: string) => pairLabel({
  quick: ["快速扫描", "Quick scan"], standard: ["标准扫描", "Standard scan"], deep: ["深度扫描", "Deep scan"], skip: ["确定性侦察（未调用模型）", "Deterministic recon (no model calls)"],
  manual_review: ["复杂前端·自动收口", "Complex frontend · auto-closed"],
}, value);

export const fuseVerdictLabel = (value: string) => pairLabel({
  pending: ["待处置", "Awaiting action"], manual_verified: ["已人工接管", "Taken over manually"], needs_followup: ["补充条件后重试", "Retry with more conditions"],
  not_reproducible: ["保持排除", "Keep excluded"],
}, value);

export const kindLabel = (value: string) => pairLabel({
  fingerprint: ["指纹", "Fingerprint"], wordpress: "WordPress", tech_stack: ["技术栈", "Tech stack"], meta_tags: "Meta", links: ["链接", "Links"],
  security_header: ["安全头", "Security header"], cookie: "Cookie", open_port: ["开放端口", "Open port"], external_service: ["外部服务", "External service"],
  info_disclosure: ["信息泄露", "Information disclosure"], js_file: ["JS 文件", "JS file"], api: "API", realtime_endpoint: ["实时接口", "Realtime endpoint"],
  request_header_intelligence: ["请求头情报", "Request header intel"], route: ["路由", "Route"], runtime_signal: ["运行期信号", "Runtime signal"], runtime_diagnostics: ["运行时诊断", "Runtime diagnostics"],
  runtime_hook_plan: ["单一 Hook 建议", "Single hook suggestion"], crypto_signal: ["加密方式", "Crypto method"], sensitive_info: ["敏感信息", "Sensitive info"],
  env_var: ["环境变量", "Environment variable"], external_script: ["外部脚本", "External script"], endpoint: ["端点", "Endpoint"], endpoint_expanded: ["扩展端点", "Expanded endpoint"],
  directory_find: ["目录发现", "Directory discovery"], rest_endpoint: ["REST 端点", "REST endpoint"], login_endpoint: ["登录端点", "Login endpoint"],
  parameter_json: ["JSON 参数", "JSON parameter"], parameter_xml: ["XML 参数", "XML parameter"], parameter_form: ["表单参数", "Form parameter"],
  parameter_upload: ["上传参数", "Upload parameter"], parameter_path: ["路径参数", "Path parameter"], parameter_query: ["查询参数", "Query parameter"],
  vulnerability: ["漏洞", "Vulnerability"], poc_test: ["PoC 测试", "PoC test"], risk_summary: ["风险汇总", "Risk summary"], summary_target: ["目标汇总", "Target summary"],
  fixed_404: ["404 特征", "404 signature"],
}, value);

export function json(value: string) {
  try { return JSON.parse(value); } catch { return { raw: value }; }
}

export function text(value: unknown) {
  if (value === null || value === undefined || value === "") return "—";
  return typeof value === "object" ? JSON.stringify(value) : String(value);
}

export function safeSeverity(value: unknown) {
  const normalized = String(value || "").toLowerCase();
  return ["critical", "high", "medium", "low", "info", "none"].includes(normalized)
    ? normalized : "info";
}

export function endpointUrl(base: string, path: string) {
  if (!path) return base;
  try { return new URL(path, base).toString(); } catch { return `${base}${path}`; }
}

export const isHttpUrl = (value: string) => /^https?:\/\//i.test(value || "");

export function statusTone(code: unknown) {
  const value = Number(code);
  return value >= 500 ? "danger" : value >= 400 ? "warning" : value >= 300 ? "redirect"
    : value >= 200 ? "success" : "neutral";
}

// Numbers follow the selected language (77.4万 in Chinese, 77.4K in English).
export const formatNumber = (value: number) => formatInteger(value);
export const formatCompactNumber = (value: number) => formatCompact(value);

export function scanOperationError(error: unknown): string {
  const source = String(error);
  if (/(^|[\s:(])scan_agent_policy_history_unknown_requires_fresh_attempt(?=$|[\s:)])/u.test(source)) {
    return tr(
      "这条历史任务没有可验证的 Nest 策略快照，不能按当前设置直接续跑。请保留历史记录，并新建独立任务重新执行；系统不会自动重放旧操作。",
      "This historical task has no verifiable Nest policy snapshot, so it cannot be resumed with the current settings. Keep the history and create a separate task to run it again; old operations are never replayed automatically."
    );
  }
  return source;
}

export function humanizeScanCheckpoint(value: string, maxLength = 360) {
  const readable = scanOperationError(value || "");
  const source = String(readable || "")
    .replace(/\\:|\\\//g, (match) => match.slice(1))
    .replace(/[│╭╮╰╯─]{2,}/g, " ")
    .replace(/\s*Traceback[\s\S]*$/i, "")
    .replace(/\s*\[PYI-[^\]]+\][\s\S]*$/i, "")
    .replace(/\s*File "[^"]+", line \d+[\s\S]*$/i, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!source) return tr("没有记录可读的结束原因；请查看本次执行历史与运行轨迹。", "No readable end reason was recorded; check this attempt's run history and trace.");
  const marker = source.indexOf("报错细节："); // i18n-ignore: backend checkpoint marker
  const compact = marker >= 0
    ? (() => {
        const before = source.slice(0, marker).replace(/；+\s*$/u, "").trim();
        const after = source
          .slice(marker)
          .replace(/^；*报错细节：/u, "")
          .split(" | ")[0]
          .trim();
        return after ? `${before}；报错细节：${after}` : before; // i18n-ignore: rebuilds the backend checkpoint text
      })()
    : source.replace(/；{2,}/gu, "；");
  return compact.length > maxLength ? `${compact.slice(0, maxLength).trim()}…` : compact;
}

export function scanInterruption(scan: SentinelScan) {
  if (scan.administrativeClosureRecorded) return {
    title: tr("已人工结案，原执行结果未结清", "Closed manually; original run not settled"),
    detail: tr("原始请求、预算占用和证据保留；结案不代表执行成功。", "The original request, budget usage and evidence are kept; closing does not mean the run succeeded."),
    action: tr("旧任务不能恢复、重试或删除；后续工作需新建独立任务并重新授权。", "The old task cannot be resumed, retried or deleted; further work needs a new, separately authorized task."),
    tone: "warning",
  };
  const effectiveStatus = scan.latestAttemptStatus || scan.status;
  if (!["partial", "failed", "limited", "paused", "cancelled"].includes(effectiveStatus)) return undefined;
  const raw = String(scan.latestAttemptStopReason || scan.latestAttemptCheckpoint || scan.currentCheckpoint || "");
  const lower = raw.toLowerCase();
  const result = {
    title: effectiveStatus === "paused"
      ? tr("本轮已由人工停止", "This attempt was stopped manually")
      : effectiveStatus === "partial"
        ? tr("本轮存在待补充验证", "This attempt has items that need validation")
        : tr("本轮执行未完成", "This attempt did not finish"),
    detail: humanizeScanCheckpoint(raw, 260),
    action: effectiveStatus === "paused"
      ? tr("继续时只处理尚未完成的 URL。", "Continuing only processes URLs that have not finished.")
      : tr("重试会创建同任务的新执行尝试，历史原因不会进入本轮结果。", "Retrying creates a new attempt of the same task; past reasons do not carry into its results."),
    tone: effectiveStatus === "failed" || effectiveStatus === "limited" ? "danger" : "warning",
  };
  if (scanOperationError(raw) !== raw) {
    return { ...result, title: tr("历史策略快照缺失", "Historical policy snapshot missing"), action: tr("保留历史记录，新建独立任务重新执行；不会自动重放旧操作。", "Keep the history and create a separate task to run it again; old operations are not replayed automatically.") };
  }
  if (effectiveStatus === "partial" && (lower.includes("调查队列已处理") || lower.includes("本轮未完整结束"))) { // i18n-ignore: backend checkpoint text
    return {
      ...result,
      title: tr("自动队列已结束，仍有待补充验证", "Automatic queue finished; some items still need validation"),
      action: tr("全部 URL 已处理；未取得目标请求与响应证据的项目保留为待补充验证，不会被误计为自动验证完成。", "All URLs were processed. Items without target request and response evidence stay marked as needing validation and are not counted as automatically validated."),
    };
  }
  if (lower.includes("prompt too long") || lower.includes("context window") || lower.includes("上下文超限") || lower.includes("memory guard")) { // i18n-ignore: backend reason text
    return { ...result, title: tr("模型上下文或内存达到上限", "Model context or memory limit reached"), action: tr("调整后的本地资源策略会在新 attempt 中生效；只重试未完成目标。", "The adjusted local resource policy applies in the new attempt; only unfinished targets are retried.") };
  }
  if (lower.includes("api key") || lower.includes("认证失败") || lower.includes("unauthorized")) { // i18n-ignore: backend reason text
    return { ...result, title: tr("模型认证未通过", "Model authentication failed"), action: tr("检查当前启用的模型 Profile；修复后只重试未完成目标。", "Check the active model profile; once fixed, only unfinished targets are retried.") };
  }
  if (lower.includes("cdp") || lower.includes("websocket is not defined") || lower.includes("前端探测未通过")) { // i18n-ignore: backend reason text
    return { ...result, title: tr("浏览器前端探测未完整结束", "Browser frontend probing did not finish"), action: tr("前端检查点已保留；新 attempt 会按证据版本决定复用或重新采集。", "Frontend checkpoints are kept; the new attempt reuses or recollects them based on the evidence version.") };
  }
  if (lower.includes("waf") || lower.includes("captcha") || lower.includes("机器人挑战") || lower.includes("429")) { // i18n-ignore: backend reason text
    return { ...result, title: tr("目标保护或限速触发停止", "Stopped by target protection or rate limiting"), action: tr("该目标不会被历史错误自动放行；确认环境后再处理。", "Past errors will not automatically clear this target; confirm the environment before acting.") };
  }
  if (lower.includes("会话") || lower.includes("session") || lower.includes("重新登录")) { // i18n-ignore: backend reason text
    return { ...result, title: tr("登录会话需要恢复", "Login session needs to be restored"), action: tr("重新登录并变为绿灯后，在同一任务内继续。", "Log in again; once the status turns green, continue in the same task.") };
  }
  if (lower.includes("超时") || lower.includes("no progress") || lower.includes("无进展")) { // i18n-ignore: backend reason text
    return { ...result, title: tr("执行阶段长时间没有有效进展", "No meaningful progress for a long time"), action: tr("新 attempt 不继承上一轮运行状态，只复用可验证证据。", "The new attempt does not inherit the previous run state; it only reuses verifiable evidence.") };
  }
  return result;
}

export function scanSummary(scan: SentinelScan) {
  const checkpoint = String(scan.latestAttemptStopReason || scan.latestAttemptCheckpoint || scan.currentCheckpoint || "").trim();
  if (!checkpoint.includes("学习候选生成失败：候选未通过质量门禁")) return humanizeScanCheckpoint(checkpoint); // i18n-ignore: backend checkpoint text
  return scan.status === "failed"
    ? tr("历史任务：扫描本体已失败；后台学习门禁曾覆盖原始摘要。学习结果不是失败原因，请查看运行日志。", "Historical task: the scan itself failed; the background learning gate overwrote the original summary. Learning is not the cause of the failure; check the run logs.")
    : tr("扫描已结束；本次证据不足以沉淀为学习候选，扫描结果不受影响。", "Scan finished; the evidence was not enough for a learning candidate. Scan results are unaffected.");
}

export function latestAttemptLabel(scan: SentinelScan, statusLabel: (value: string) => string) {
  if (scan.administrativeClosureRecorded) return tr("已人工结案 · 原执行结果未结清", "Closed manually · original run not settled");
  if (!scan.latestAttemptNumber) return tr("尚未执行", "Not run yet");
  const status = statusLabel(scan.latestAttemptStatus || scan.status);
  return tr(`最新第 ${scan.latestAttemptNumber} 次：${status}`, `Latest attempt ${scan.latestAttemptNumber}: ${status}`);
}

export const uncachedInput = (scan: SentinelScan) => Math.max(0, scan.inputTokens - scan.cachedTokens);
export const scanTokenTotal = (scan: SentinelScan) => scan.totalTokens || scan.inputTokens + scan.outputTokens;

export const attemptStageLabel = (stage: string) => pairLabel({
  initializing: ["初始化", "Initializing"], preparing: ["准备运行环境", "Preparing environment"], frontend_recon: ["前端与接口侦察", "Frontend and API recon"],
  validation: ["自动验证", "Automatic validation"], evidence: ["证据与结果归档", "Archiving evidence and results"], complete: ["已结束", "Finished"], paused: ["已暂停", "Paused"],
  stopped: ["已停止", "Stopped"], running: ["执行中", "Running"], unknown: ["历史记录未提供", "Not recorded in history"],
}, stage, stage ? stage : tr("历史记录未提供", "Not recorded in history"));

// label is a getter so each render reads it in the current language.
export const EXECUTION_STAGE_STEPS = [
  { key: "prepare", get label() { return tr("准备", "Prepare"); } },
  { key: "precollect", get label() { return tr("前置采集", "Pre-collection"); } },
  { key: "browser", get label() { return tr("浏览器采集", "Browser collection"); } },
  { key: "evidence", get label() { return tr("证据整理", "Evidence"); } },
  { key: "agent", get label() { return tr("Agent 执行", "Agent run"); } },
  { key: "finalize", get label() { return tr("结果整理", "Results"); } },
] as const;

export type ExecutionStageKey = (typeof EXECUTION_STAGE_STEPS)[number]["key"];

export function resolveExecutionStage(attempt: {
  stage?: string;
  checkpoint?: string;
  stopReason?: string;
  status?: string;
  llmRequests?: number;
}): { current: ExecutionStageKey; agentStarted: boolean; historyMissing: boolean } {
  const stage = String(attempt.stage || "").trim().toLowerCase();
  const text = `${attempt.checkpoint || ""} ${attempt.stopReason || ""}`.toLowerCase();
  const historyMissing = !stage && !String(attempt.checkpoint || "").trim() && !String(attempt.stopReason || "").trim();
  const llmRequests = Number(attempt.llmRequests || 0);
  const agentStarted = llmRequests > 0 || /agent|自动验证|validation|模型/.test(`${stage} ${text}`);

  let current: ExecutionStageKey = "prepare";
  if (["complete", "completed", "stopped", "paused"].includes(stage) || /结果整理|已结束|归档/.test(text)) {
    current = "finalize";
  } else if (["validation", "running"].includes(stage) || agentStarted) {
    current = "agent";
  } else if (["evidence"].includes(stage) || /证据整理|证据与结果/.test(text)) {
    current = "evidence";
  } else if (/浏览器采集|cdp|runtime probe|前端探测/.test(text) || stage === "frontend_recon") {
    current = /证据整理|部分侦察|证据已保存/.test(text) ? "evidence" : "browser";
  } else if (/前置采集|指纹|静态|frontend/.test(text)) {
    current = "precollect";
  } else if (["initializing", "preparing", "unknown"].includes(stage) || /准备|初始化/.test(text)) {
    current = "prepare";
  } else if (stage) {
    current = "precollect";
  }

  return { current, agentStarted, historyMissing };
}

export function executionStageStatusLabel(attempt: {
  stage?: string;
  checkpoint?: string;
  stopReason?: string;
  status?: string;
  llmRequests?: number;
}) {
  const resolved = resolveExecutionStage(attempt);
  if (resolved.historyMissing) return tr("历史记录未提供", "Not recorded in history");
  if (resolved.current !== "agent" && resolved.current !== "finalize" && !resolved.agentStarted) {
    return tr("Agent 未启动", "Agent not started");
  }
  return "";
}



export function attemptTime(attempt: SentinelScanAttempt) {
  const start = String(attempt.startedAt || "").trim();
  const end = String(attempt.finishedAt || attempt.updatedAt || "").trim();
  return end && end !== start ? `${start} → ${end}` : start || "—";
}

export function attemptEndReason(attempt: SentinelScanAttempt) {
  const reason = String(attempt.stopReason || "").trim();
  if (!reason) return "";
  return ["completed", "completed_with_gaps", "partial", "recon_only"].includes(attempt.status) &&
    reason.includes("学习候选生成失败：候选未通过质量门禁") // i18n-ignore: backend reason text
    ? tr("扫描本体已完成；本次证据未达到学习沉淀门禁，因此没有保存学习候选，不影响扫描结果。", "The scan itself completed; the evidence did not meet the learning gate, so no learning candidate was saved. Scan results are unaffected.")
    : humanizeScanCheckpoint(reason);
}

export const displayName = (value: any) => {
  const name = value?.framework || value?.name || value?.language || "";
  return !name || name === "Unknown" ? tr("未识别", "Unidentified") : String(name);
};
export const displayVersion = (value: any) => value?.version ? `v${value.version}` : tr("版本未知", "Unknown version");

export const sensitiveType = (value: string) => pairLabel({
  private_key: ["私钥", "Private key"], aws_access_key: "AWS AccessKey", alibaba_access_key: ["阿里云 AccessKey", "Alibaba Cloud AccessKey"],
  tencent_access_key: ["腾讯云 AccessKey", "Tencent Cloud AccessKey"], google_api_key: "Google API Key", google_oauth_token: "Google OAuth Token",
  github_token: "GitHub Token", gitlab_token: "GitLab Token", slack_token: "Slack Token",
  stripe_secret_key: "Stripe Secret Key", sendgrid_api_key: "SendGrid API Key", npm_token: "npm Token",
  jwt: ["JWT 令牌", "JWT"], bearer_token: "Bearer Token", database_password: ["数据库连接密码", "Database password"], webhook: "Webhook",
  cloud_access_key: ["云平台密钥", "Cloud access key"], password_assignment: ["密码/Secret", "Password/Secret"], wechat_appid: ["微信 AppID", "WeChat AppID"],
  corp_id: ["企业标识", "Corp ID"], email: ["邮箱", "Email"], cn_phone: ["手机号", "Mobile number"], cn_id: ["身份证号", "Chinese ID number"], mac_address: ["MAC 地址", "MAC address"],
  ip_address: ["IP 地址", "IP address"], encryption_key: ["加密密钥", "Encryption key"], credential_in_url: ["URL 中的凭据", "Credentials in URL"],
}, value);

export function validRouteRecord(item: SentinelFinding) {
  const value = String(json(item.recordJson).path || "").trim();
  if (!value || value.length > 1000 || !value.startsWith("/") || value.startsWith("//")) return false;
  if (/^\/(?:m|path|d)(?:\/|$)/i.test(value) || /\s|[<>]/.test(value)) return false;
  if (/\.(?:avif|bmp|css|eot|gif|ico|jpe?g|js|map|mp3|mp4|pdf|png|svg|ttf|webp|woff2?)$/i.test(value.split(/[?#]/)[0])) return false;
  return !/^\/[mMlLhHvVcCsSqQtTaAzZ0-9.,+\-]+$/.test(value);
}

export function validSensitiveRecord(item: SentinelFinding) {
  const data = json(item.recordJson);
  const kind = String(data.type || "");
  const value = String(data.value || data.maskedValue || "");
  const context = String(data.context || "");
  if (kind === "ip_address") {
    const octets = value.split(".");
    if (octets.length !== 4 || octets.some((part: string) => !/^\d{1,3}$/.test(part) || Number(part) > 255)) return false;
    const index = context.indexOf(value);
    if (index >= 0 && (/[\d.]/.test(context[index - 1] || "") || /[\d.]/.test(context[index + value.length] || ""))) return false;
    if (/<path|viewbox|attrs:\s*\{?\s*d\s*:|svgpath|iconpath|fill\s*:|stroke\s*:/i.test(context)) return false;
    const escapedIp = value.replace(/\./g, "\\.");
    const versionTerms = "(?:@?version|\\bver\\b|release|jquery|bootstrap|easyui|layui|vue|react|angular|package\\.json|sourceMappingURL)";
    if (new RegExp(`${versionTerms}[^\\n]{0,100}${escapedIp}`, "i").test(context) || new RegExp(`${escapedIp}[^\\n]{0,60}${versionTerms}`, "i").test(context)) return false;
    const isPrivate = octets[0] === "10" || (octets[0] === "172" && Number(octets[1]) >= 16 && Number(octets[1]) <= 31) ||
      (octets[0] === "192" && octets[1] === "168") || octets[0] === "127" || (octets[0] === "169" && octets[1] === "254");
    if (!isPrivate && !new RegExp(`(?:https?://|wss?://|(?:host|hostname|server|proxy|endpoint|listen|connect|remote|origin|address|ip|网关|服务器|地址)\\s*[:=])[^\\n]{0,80}${escapedIp}`, "i").test(context)) return false; // i18n-ignore: context-matching pattern
  }
  if (kind === "cn_phone" && !/(?:phone|mobile|telephone|tel|contact|手机号|手机|电话|联系方式)/i.test(context)) return false;
  if (kind === "mac_address" && !/(?:mac(?:address)?|device|hardware|网卡|设备)/i.test(context)) return false;
  if (kind === "email" && /(?:copyright|license|licensed|@preserve|contributors|package\.json|npmjs)/i.test(context)) return false;
  return true;
}

export const cryptoCategory = (value: string) => pairLabel({
  encoding: ["编码", "Encoding"], hash: ["摘要 / KDF", "Hash / KDF"], symmetric: ["对称加密", "Symmetric encryption"], asymmetric: ["非对称加密", "Asymmetric encryption"], china: ["国密", "Chinese SM ciphers"],
}, value);
export const methodTone = (value: unknown) => `method-${String(value || "unknown").toLowerCase()}`;
export const scriptTone = (value: unknown) => `script-${String(value || "application").toLowerCase()}`;
export const kindTone = (value: string) => `kind-${value.replace(/_/g, "-")}`;

export function attemptBackendSummary(attempt: {
  backendPlanJson?: string;
  llmRequests?: number;
  status?: string;
  stage?: string;
}) {
  const raw = String(attempt.backendPlanJson || "").trim();
  if (!raw) {
    const ran = Number(attempt.llmRequests || 0) > 0;
    return {
      backend: ran ? tr("后端计划未记录（只读）", "Backend plan not recorded (read-only)") : tr("尚未进入后端执行", "Not yet handed to a backend"),
      environment: ran ? tr("计划未写入这次执行记录", "Plan not written to this attempt's record") : tr("执行环境未记录", "Execution environment not recorded"),
      taskStatus: attempt.status || "unknown",
    };
  }
  let plan: any = null;
  try { plan = JSON.parse(raw); } catch { plan = null; }
  const targetBackend = Array.isArray(plan?.targets)
    ? plan.targets.map((item: { backend?: string }) => item?.backend).find(Boolean)
    : "";
  const engine = String(plan?.engine || plan?.backend || targetBackend || plan?.kind || plan?.runner || "");
  const backend = engine === "native"
    ? "Native"
    : engine
      ? tr("未识别的执行计划（只读）", "Unrecognized execution plan (read-only)")
      : tr("后端计划已写入", "Backend plan recorded");
  const environment = String(plan?.environment || plan?.runtime || plan?.deployment || tr("执行环境未记录", "Execution environment not recorded"));
  return {
    backend,
    environment,
    taskStatus: String(plan?.status || attempt.status || "unknown"),
  };
}
