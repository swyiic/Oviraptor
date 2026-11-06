import type { SentinelFuseEntry, SentinelTarget } from "../types";
import { useI18n } from "../../../i18n";

const { tr } = useI18n();

// Fuse copy and classification belong to the stop/disposition feature. These
// helpers only read saved evidence; they never decide whether a scan can resume.
export function fuseReasonParts(item: SentinelFuseEntry, target?: SentinelTarget) {
  const raw = target?.routingReason || item.reason || "";
  return String(raw)
    .split("；")
    .map((text) => text.trim())
    .filter(Boolean)
    .map((text) => {
      let tone = "general",
        label = tr("扫描信号", "Scan signal");
      if (/HTTP|入口可访问|入口受限|入口响应/.test(text)) {
        tone = "access";
        label = tr("入口", "Entry");
      } else if (/SourceMap/i.test(text)) {
        tone = "sourcemap";
        label = "SourceMap";
      } else if (/业务脚本|应用分包|JS/i.test(text)) {
        tone = "javascript";
        label = "JS";
      } else if (/识别到/.test(text)) {
        tone = "framework";
        label = tr("框架", "Framework");
      } else if (/API/.test(text)) {
        tone = "api";
        label = "API";
      } else if (/路由/.test(text)) {
        tone = "route";
        label = tr("路由", "Route");
      } else if (/敏感|鉴权|管理|上传|业务入口/.test(text)) {
        tone = "sensitive";
        label = tr("敏感业务", "Sensitive area");
      } else if (/熔断|模型调用|Token|无进展|计划\/待办/.test(text)) {
        tone = "fuse";
        label = tr("停止", "Stop");
      }
      return { text, tone, label };
    });
}

export function fuseReasonCategory(item: SentinelFuseEntry, target?: SentinelTarget) {
  const reason = `${item.reason} ${target?.routingReason || ""}`.toLowerCase();
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

export function fuseCategoryLabel(category: string) {
  return ({
    budget: tr("成本 / 无进展", "Cost / no progress"),
    access: tr("缺少访问条件", "Missing access"),
    blocked: tr("遭到拦截", "Blocked"),
    failure: tr("网络 / 执行异常", "Network / execution error"),
    low_value: tr("价值不足", "Low value"),
  } as Record<string, string>)[category] || category;
}

export function fuseRecommendedAction(item: SentinelFuseEntry, target?: SentinelTarget) {
  return ({
    budget: tr("先看已保存情报；有明确接口或参数再恢复，避免继续空烧 Token。", "Review the saved intelligence first; resume only once a clear API or parameter is known, to avoid burning more tokens."),
    access: tr("补充 Cookie、Token 或登录态后恢复重试。", "Add a cookie, token, or login session, then resume."),
    blocked: tr("保持停止；确认访问策略或降低频率后再恢复。", "Keep it stopped; confirm the access policy or lower the request rate before resuming."),
    failure: tr("确认网络、DNS 或证书状态，修复环境后直接重试。", "Check network, DNS, or certificate status; retry once the environment is fixed."),
    low_value: tr("快速人工复核现有 JS/API 证据；无新增价值即可归档。", "Quickly review the existing JS/API evidence manually; archive it if nothing new turns up."),
  } as Record<string, string>)[fuseReasonCategory(item, target)];
}
