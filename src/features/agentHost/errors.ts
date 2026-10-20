/** Human-readable (zh/en) explanation + next-step hint for agent-host error codes. Raw code kept for diagnostics. */
export type AgentHostErrorView = { title: [string, string]; hint: [string, string]; code: string };

const SWITCH: [string, string] = ["可在对话右上角切换到 DeepSeek（国内直连，无需 VPN）。", "Switch to DeepSeek from the model picker (no VPN needed)."];
const TABLE: Record<string, Omit<AgentHostErrorView, "code">> = {
  host_tool_permission_denied: { title: ["当前项目尚未授予所需权限", "Project permission not granted"], hint: ["请审阅本轮的数据权限请求；文件权限与项目资产读取权限分别授予。", "Review the project's data permission request. Workspace file access is separate from asset data access."] },
  host_authorization_expired: { title: ["授权请求已失效", "Authorization request expired"], hint: ["这次确认没有生效。请刷新状态，审阅当前请求后再确认。", "This decision was not applied. Refresh and review the current request."] },
  host_provider_quota_exhausted: { title: ["模型服务额度不足", "Model quota exhausted"], hint: ["检查当前模型账户的余额或额度；无需重新授予文件权限。", "Check this provider account's balance or quota."] },
  host_provider_rate_limited: { title: ["模型服务限流", "Model rate limited"], hint: ["请稍后重试。现有授权仍在原范围内有效。", "Retry later. Existing grants remain valid within their scope."] },
  host_provider_request_invalid: { title: ["模型服务拒绝了请求格式", "Model rejected request format"], hint: ["检查模型、接口兼容性及历史工具消息格式；这不是文件授权失败。", "Check model/endpoint compatibility and tool-message format."] },
  host_provider_model_or_endpoint_missing: { title: ["模型或接口不存在", "Model or endpoint not found"], hint: ["检查配置中的模型 ID 与服务地址。", "Check the configured model ID and service URL."] },
  host_provider_service_unavailable: { title: ["模型服务暂时不可用", "Model service unavailable"], hint: ["服务端返回错误，稍后再试；不会自动重放工具操作。", "The provider returned a server error. Retry later."] },

  host_runtime_unconfigured: { title: ["尚未配置聊天模型", "Chat model not configured"], hint: ["请到配置中心的「聊天模型」选择服务商与模型，并完成登录或填写 API key。消息仍可保存；配置后再继续任务。", "Open Configuration Center › Chat models, select a provider and model, and sign in or add its API key. Messages can still be saved; continue the task after configuration."] },
  host_task_effect_pending: { title: ["上次操作仍在收尾", "An earlier operation is still finishing"], hint: ["请等操作返回后再继续。此时不会重放写操作，也不能提前确认结果。", "Wait for the operation to settle before continuing. Writes are not replayed and their outcome cannot be accepted yet."] },
  host_task_revision_conflict: { title: ["任务状态已更新", "Task state changed"], hint: ["刷新任务状态后再继续，避免用旧计划覆盖最新进度。", "Reload the task before continuing so an older plan cannot overwrite newer progress."] },
  host_workspace_execution_unsupported: { title: ["脚本隔离执行尚未接通", "Isolated script execution unavailable"], hint: ["可读取和保存工作区文件；运行脚本需要受支持的隔离运行环境。", "Workspace files can be read and saved. Running scripts requires a supported isolated runtime."] },
  host_provider_stream_incomplete: { title: ["模型回复中断", "Model response interrupted"], hint: ["回复未完整结束，请检查网络。涉及写入的任务先核实执行结果再重试。", "The response did not finish. Check the network and verify writes before retrying."] },
  host_desktop_asset_sync_failed: { title: ["资产数据尚未同步", "Asset synchronization failed"], hint: ["本轮未返回旧镜像作为最新资产，请检查本机资产服务后重试。", "The stale mirror was withheld. Check the local asset service before retrying."] },
  host_desktop_inventory_upgrade_required: { title: ["资产应用需要升级", "Assets application upgrade required"], hint: ["请在配置中心 → 已安装应用中点击「更新内置组件」，再重试。本轮没有迁移旧库，也没有把旧镜像作为最新数据返回。", "Explicitly upgrade Assets to 1.1.0 and retry. The old database was not migrated and its stale mirror was withheld."] },
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
