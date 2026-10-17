/** Human-readable (zh/en) explanation + next-step hint for agent-host error codes. Raw code kept for diagnostics. */
export type AgentHostErrorView = { title: [string, string]; hint: [string, string]; code: string };

const SWITCH: [string, string] = ["可在对话右上角切换到 DeepSeek（国内直连，无需 VPN）。", "Switch to DeepSeek from the model picker (no VPN needed)."];
const TABLE: Record<string, Omit<AgentHostErrorView, "code">> = {
  host_provider_network_unreachable: { title: ["无法连接模型服务（网络不可达）", "Model service unreachable"], hint: ["检查网络 / 代理设置。" + SWITCH[0], "Check network/proxy. " + SWITCH[1]] },
  host_provider_request_failed: { title: ["模型请求失败", "Model request failed"], hint: ["检查模型 ID 与 API key 是否匹配当前服务商，或切换运行时 / 模型后重试。", "Check the model id and API key match the provider, or switch runtime/model and retry."] },
  host_provider_unauthorized: { title: ["API key 无效或无权限", "API key rejected"], hint: ["到 设置 › Agent runtime › 高级 provider 重新填写当前服务商的 API key。", "Re-enter the API key in Settings › Agent runtime › Advanced provider."] },
  host_provider_api_key_required: { title: ["尚未配置该服务商的 API key", "API key not configured"], hint: ["到 设置 › Agent runtime › 高级 provider 填写 API key（DeepSeek 也可设置环境变量 DEEPSEEK_API_KEY）。", "Add the API key in Settings › Agent runtime › Advanced provider (or DEEPSEEK_API_KEY)."] },
  host_provider_api_key_invalid: { title: ["API key 格式不正确", "API key format invalid"], hint: ["请粘贴完整的 key，不要包含空格。", "Paste the full key without spaces."] },
  host_provider_model_invalid: { title: ["模型 ID 不合法", "Invalid model id"], hint: ["从右上角模型列表中选择。", "Pick a model from the picker."] },
  host_provider_base_url_invalid: { title: ["服务地址不合法", "Invalid base URL"], hint: ["需为 https 地址。", "Must be an https URL."] },
  host_reply_already_running: { title: ["上一条回复仍在进行", "A reply is still running"], hint: ["等待完成，或点击停止 / 纠偏。", "Wait, or stop / steer."] },
  host_runtime_busy: { title: ["运行时正忙", "Runtime busy"], hint: ["当前有回复在运行，稍后再试。", "A reply is running; try again shortly."] },
  host_reply_cancelled: { title: ["已停止", "Stopped"], hint: ["可以重新发送或编辑后再发。", "Resend or edit and send again."] },
  host_runtime_unavailable: { title: ["运行时不可用", "Runtime unavailable"], hint: ["到 设置 › Agent runtime 检查运行时配置。", "Check Settings › Agent runtime."] },
  host_reply_unavailable: { title: ["回复不可用", "Reply unavailable"], hint: ["请重试，或切换运行时。", "Retry or switch runtime."] },
};

export function describeAgentHostError(code: string | null | undefined): AgentHostErrorView {
  const raw = String(code || "").trim() || "unknown";
  const found = TABLE[raw];
  if (found) return { ...found, code: raw };
  if (/timeout|timed_out/i.test(raw)) return { title: ["模型响应超时", "Model timed out"], hint: ["网络较慢或服务繁忙，请重试。" + SWITCH[0], "Retry. " + SWITCH[1]], code: raw };
  if (/^agent_host_http_|fetch|Failed to fetch/i.test(raw)) return { title: ["无法连接本机 Agent host", "Cannot reach the local agent host"], hint: ["确认桌面应用的 sidecar 正在运行，然后重试。", "Make sure the desktop sidecar is running, then retry."], code: raw };
  return { title: ["出错了", "Something went wrong"], hint: ["请重试；若持续出现，请切换运行时或检查 设置 › Agent runtime。", "Retry; if it persists, switch runtime or check Settings › Agent runtime."], code: raw };
}
