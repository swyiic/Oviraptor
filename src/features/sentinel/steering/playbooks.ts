export type SteeringPlaybook = {
  command: string;
  label: { zh: string; en: string };
  hint: { zh: string; en: string };
  expand: (args: string) => string;
};

export const steeringPlaybooks: SteeringPlaybook[] = [
  {
    command: "deepdive",
    label: { zh: "深挖漏洞", en: "Deep-dive finding" }, // i18n-ignore
    hint: { zh: "/deepdive <findingId|url>", en: "/deepdive <findingId|url>" },
    expand: (args) =>
      `[playbook:deepdive]\n请对「${args.trim() || "当前优先级最高的 finding"}」启动深度验证：复现请求、沙箱内自写脚本迭代、必要时做有界 WAF 绕过；更新 agent_finding_exploits 并推送协作事件。`, // i18n-ignore
  },
  {
    command: "wafbypass",
    label: { zh: "WAF 绕过", en: "WAF bypass" }, // i18n-ignore
    hint: { zh: "/wafbypass <url>", en: "/wafbypass <url>" },
    expand: (args) =>
      `[playbook:wafbypass]\n对 ${args.trim() || "被拦截的请求 URL"} 做有界 WAF 效力测试：把状态码和响应片段交给当前云端模型分析，只执行模型提出的同主机下一发请求（最多 3 轮）。不要套用固定编码或请求头清单。遇 CAPTCHA/封禁即停止并标记 waf_blocked。`, // i18n-ignore
  },
  {
    command: "recon",
    label: { zh: "只读侦察", en: "Read-only recon" }, // i18n-ignore
    hint: { zh: "/recon [focus]", en: "/recon [focus]" },
    expand: (args) =>
      `[playbook:recon]\n只读侦察：${args.trim() || "当前目标"} — 仅用 inspect_evidence 与只读 HTTP，不调用高危工具；汇总待测面与缺口。`, // i18n-ignore
  },
];

export function expandSteeringInput(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("/")) return raw;
  const match = trimmed.match(/^\/([\w-]+)(?:\s+([\s\S]*))?$/);
  if (!match) return raw;
  const command = match[1].toLowerCase();
  const args = (match[2] ?? "").trim();
  const book = steeringPlaybooks.find((item) => item.command === command);
  return book ? book.expand(args) : raw;
}

export function steeringSlashTemplate(command: string): string {
  const book = steeringPlaybooks.find((item) => item.command === command);
  return book ? `/${book.command} ` : "";
}
