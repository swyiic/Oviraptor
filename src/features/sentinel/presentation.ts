import type { SentinelFinding, SentinelScan, SentinelScanAttempt } from "../../types";

type Translator = (zh: string, en: string) => string;
const mapped = (map: Record<string, string>, value: string, fallback = value) =>
  map[value] || fallback;

export function createSentinelLabels(tr: Translator) {
  const statusZh: Record<string, string> = {
    draft: "待确认", queued: "待扫描", frontend_recon: "前端解析", routed: "已分流",
    recon_only: "仅前端结果", manual_review: "复杂前端·人工复核", scanning: "扫描中",
    pausing: "正在停止", limited: "已熔断", fuse_excluded: "熔断区排除", deferred: "已延后",
    completed: "已完成", partial: "部分完成", failed: "失败", paused: "已暂停", imported: "已导入",
  };
  const statusEn: Record<string, string> = {
    draft: "Review", queued: "Queued", frontend_recon: "Frontend recon", routed: "Routed",
    recon_only: "Recon only", manual_review: "Manual review", scanning: "Scanning",
    pausing: "Pausing", limited: "Limited", fuse_excluded: "Fuse excluded", deferred: "Deferred",
    completed: "Completed", partial: "Partial", failed: "Failed", paused: "Paused", imported: "Imported",
  };
  const verdictZh: Record<string, string> = {
    true_positive: "真实漏洞", false_positive: "误报", needs_more: "需补证", pending: "未验证",
  };
  const verdictEn: Record<string, string> = {
    true_positive: "Confirmed", false_positive: "False positive", needs_more: "More evidence", pending: "Pending",
  };
  const severityZh: Record<string, string> = {
    critical: "严重", high: "高危", medium: "中危", low: "低危", info: "信息", none: "无风险", unknown: "未知",
  };
  const severityEn: Record<string, string> = {
    critical: "Critical", high: "High", medium: "Medium", low: "Low", info: "Info", none: "No risk", unknown: "Unknown",
  };
  return {
    statusLabel: (value: string) => tr(mapped(statusZh, value), mapped(statusEn, value)),
    retryActionLabel: (scan: SentinelScan) =>
      scan.status === "completed" ? tr("重新执行", "Run again") : tr("继续执行", "Continue"),
    verdictLabel: (value: string) => tr(mapped(verdictZh, value), mapped(verdictEn, value)),
    severityLabel: (value: string) =>
      tr(mapped(severityZh, value, value || "信息"), mapped(severityEn, value, value || "Info")),
    scanTypeLabel: (value: string) => mapped({
      web: "Web", code: tr("代码审计", "Code audit"), greybox: tr("灰盒", "Grey-box"), cicd: "CI/CD",
    }, value || "web"),
    llmDeploymentLabel: (scan: SentinelScan) => {
      if (scan.llmDeployment === "local") {
        return scan.llmFullPower
          ? tr("本地 LLM · 火力全开", "Local LLM · Full power")
          : tr("本地 LLM", "Local LLM");
      }
      return scan.llmDeployment === "cloud" ? tr("云端 AI", "Cloud AI") : tr("LLM 未记录", "LLM not recorded");
    },
  };
}

export const scanTitle = (scan: SentinelScan) => scan.taskName || scan.projectName || "未命名任务";
export const llmDeploymentClass = (scan: SentinelScan) =>
  scan.llmDeployment === "local" ? "local" : scan.llmDeployment === "cloud" ? "cloud" : "unknown";

export const routeModeLabel = (value: string) => mapped({
  quick: "快速验证", standard: "标准深挖", deep: "深度验证", skip: "仅前端解析",
  manual_review: "复杂前端·人工复核",
}, value);

export const fuseVerdictLabel = (value: string) => mapped({
  pending: "待处置", manual_verified: "已人工接管", needs_followup: "补充条件后重试",
  not_reproducible: "保持排除",
}, value);

export const kindLabel = (value: string) => mapped({
  fingerprint: "指纹", wordpress: "WordPress", tech_stack: "技术栈", meta_tags: "Meta", links: "链接",
  security_header: "安全头", cookie: "Cookie", open_port: "开放端口", external_service: "外部服务",
  info_disclosure: "信息泄露", js_file: "JS 文件", api: "API", realtime_endpoint: "实时接口",
  request_header_intelligence: "请求头情报", route: "路由", runtime_signal: "运行期信号",
  runtime_hook_plan: "单一 Hook 建议", crypto_signal: "加密方式", sensitive_info: "敏感信息",
  env_var: "环境变量", external_script: "外部脚本", endpoint: "端点", endpoint_expanded: "扩展端点",
  directory_find: "目录发现", rest_endpoint: "REST 端点", login_endpoint: "登录端点",
  parameter_json: "JSON 参数", parameter_xml: "XML 参数", parameter_form: "表单参数",
  parameter_upload: "上传参数", parameter_path: "路径参数", parameter_query: "查询参数",
  vulnerability: "漏洞", poc_test: "PoC 测试", risk_summary: "风险汇总", summary_target: "目标汇总",
  fixed_404: "404 特征",
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

export const formatNumber = (value: number) => new Intl.NumberFormat("zh-CN").format(value || 0);

export function formatCompactNumber(value: number) {
  const number = Math.max(0, Number(value) || 0);
  const compact = (scaled: number, unit: string) => `${scaled.toFixed(1).replace(/\.0$/, "")}${unit}`;
  if (number >= 100_000_000) return compact(number / 100_000_000, "亿");
  if (number >= 10_000) return compact(number / 10_000, "万");
  return formatNumber(number);
}

export function scanSummary(scan: SentinelScan) {
  const checkpoint = String(scan.currentCheckpoint || "").trim();
  if (!checkpoint.includes("学习候选生成失败：候选未通过质量门禁")) return checkpoint;
  return scan.status === "failed"
    ? "历史任务：扫描本体已失败；后台学习门禁曾覆盖原始摘要。学习结果不是失败原因，请查看运行日志。"
    : "扫描已结束；本次证据不足以沉淀为学习候选，扫描结果不受影响。";
}

export const uncachedInput = (scan: SentinelScan) => Math.max(0, scan.inputTokens - scan.cachedTokens);
export const scanTokenTotal = (scan: SentinelScan) => scan.totalTokens || scan.inputTokens + scan.outputTokens;

export const attemptStageLabel = (stage: string) => mapped({
  initializing: "初始化", preparing: "准备运行环境", frontend_recon: "前端与接口侦察",
  validation: "Strix 定向验证", evidence: "证据与结果归档", complete: "已结束", paused: "已暂停",
  stopped: "已停止", running: "执行中", unknown: "历史记录",
}, stage, stage || "未知阶段");

export function attemptTime(attempt: SentinelScanAttempt) {
  const start = String(attempt.startedAt || "").trim();
  const end = String(attempt.finishedAt || attempt.updatedAt || "").trim();
  return end && end !== start ? `${start} → ${end}` : start || "—";
}

export function attemptEndReason(attempt: SentinelScanAttempt) {
  const reason = String(attempt.stopReason || "").trim();
  return ["completed", "partial", "recon_only"].includes(attempt.status) &&
    reason.includes("学习候选生成失败：候选未通过质量门禁")
    ? "扫描本体已完成；本次证据未达到学习沉淀门禁，因此没有保存学习候选，不影响扫描结果。"
    : reason;
}

export const displayName = (value: any) => {
  const name = value?.framework || value?.name || value?.language || "";
  return !name || name === "Unknown" ? "未识别" : String(name);
};
export const displayVersion = (value: any) => value?.version ? `v${value.version}` : "版本未知";

export const sensitiveType = (value: string) => mapped({
  private_key: "私钥", aws_access_key: "AWS AccessKey", alibaba_access_key: "阿里云 AccessKey",
  tencent_access_key: "腾讯云 AccessKey", google_api_key: "Google API Key", google_oauth_token: "Google OAuth Token",
  github_token: "GitHub Token", gitlab_token: "GitLab Token", slack_token: "Slack Token",
  stripe_secret_key: "Stripe Secret Key", sendgrid_api_key: "SendGrid API Key", npm_token: "npm Token",
  jwt: "JWT 令牌", bearer_token: "Bearer Token", database_password: "数据库连接密码", webhook: "Webhook",
  cloud_access_key: "云平台密钥", password_assignment: "密码/Secret", wechat_appid: "微信 AppID",
  corp_id: "企业标识", email: "邮箱", cn_phone: "手机号", cn_id: "身份证号", mac_address: "MAC 地址",
  ip_address: "IP 地址", encryption_key: "加密密钥", credential_in_url: "URL 中的凭据",
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
    if (!isPrivate && !new RegExp(`(?:https?://|wss?://|(?:host|hostname|server|proxy|endpoint|listen|connect|remote|origin|address|ip|网关|服务器|地址)\\s*[:=])[^\\n]{0,80}${escapedIp}`, "i").test(context)) return false;
  }
  if (kind === "cn_phone" && !/(?:phone|mobile|telephone|tel|contact|手机号|手机|电话|联系方式)/i.test(context)) return false;
  if (kind === "mac_address" && !/(?:mac(?:address)?|device|hardware|网卡|设备)/i.test(context)) return false;
  if (kind === "email" && /(?:copyright|license|licensed|@preserve|contributors|package\.json|npmjs)/i.test(context)) return false;
  return true;
}

export const cryptoCategory = (value: string) => mapped({
  encoding: "编码", hash: "摘要 / KDF", symmetric: "对称加密", asymmetric: "非对称加密", china: "国密",
}, value);
export const methodTone = (value: unknown) => `method-${String(value || "unknown").toLowerCase()}`;
export const scriptTone = (value: unknown) => `script-${String(value || "application").toLowerCase()}`;
export const kindTone = (value: string) => `kind-${value.replace(/_/g, "-")}`;
