# Wave4 修复规格：桌面 Session（AgentHostChat）可用性

> 来源：2026-10-17（Asia/Shanghai）用户全新构建后试用截图反馈。
> 范围：仅桌面 Session/对话页的 agent-host 聊天路径（`AgentHostChat`）与 agent-host provider 配置；**不触碰扫描 / 资产业务逻辑**。
> 原则：小步、每项一个 commit、每个 commit 跑 `npm run test:agent-first` 与 `npm run build`（含 `vue-tsc`）。
> 上级文档：[`AGENT_UX_BACKLOG.md` › Wave 4](./AGENT_UX_BACKLOG.md#wave-4--桌面-session-模型切换--agenthostchat-操作面文档已立)

## 现象总览（截图）

| # | 现象 |
| --- | --- |
| 1 | 头部显示 `xAI Grok (API key) · openai/deepseek-v4-flash · 中国移动历史资产`：runtime 是 xAI，模型却是 DeepSeek id；每条消息都失败，助手气泡里直接显示 `host_provider_network_unreachable` / `host_provider_request_failed`，composer 上方还多出一行重复的 `host_provider_request_failed`。 |
| 2 | 布局无样式：巨大的「新对话」标题、runtime/模型是纯文本、原生 `<select>`、`+`、「旧版对话」按钮挤在左侧；composer textarea 无样式；模型切换器难看且位置不对。 |
| 3 | 错误显示原始错误码，没有中文说明与处理建议。 |
| 4 | 消息缺少复制、重新编辑（编辑后重发）、steer（运行中纠偏）。 |
| 5 | 自动标题、heartbeat（仍在运行）信号没有上屏。 |

---

## 问题 1（A）：模型与 runtime 不一致，DeepSeek 无法使用

### 根因

- `src/features/agentHost/settingsClient.ts` › `setProvider()`：请求体**写死** `provider: "xai"`，且不传 `baseUrl`。在「高级 provider」里把模型改成 `openai/deepseek-v4-flash` 后，host 存成 `{ provider: "xai", baseUrl: "https://api.x.ai/v1", model: "openai/deepseek-v4-flash" }`。
- `packages/agent-host/src/provider.mjs` › `validateProviderConfig()`：不校验 provider/model 是否匹配，任何模型 id 都接受，于是出现 xAI + DeepSeek 的非法组合；请求被发往 `api.x.ai`（国内无 VPN 不可达 → `host_provider_network_unreachable`；可达也会因模型不存在 → `host_provider_request_failed`）。
- `ProviderStore.save()`：只保存**一把** key；切换 provider 时若不重新输入 key 直接报 `host_provider_api_key_required`，无法在 xAI / DeepSeek 之间来回切。
- `src/features/agentHost/chatClient.ts` › `agentRuntimeHeader()`：只认 `xai-api`，标签写死 `xAI Grok`，即使 provider 实际是 openai-compatible 也显示 xAI。
- `openai/` 前缀来自 legacy `modelProfiles`（LiteLLM 风格 `provider/model`），DeepSeek 官方 API 只认 `deepseek-chat` / `deepseek-v4-flash` 这类裸 id。

### 要求行为

1. 选择 DeepSeek 模型（id 含 `deepseek`）⇒ host provider 自动成为 `openai-compatible`，`baseUrl = https://api.deepseek.com`，模型 id 去掉 `openai/` / `deepseek/` 前缀。走 `OpenAiCompatibleRuntime`（`/chat/completions`），**不需要 VPN**。
2. 选择 Grok 模型 ⇒ `provider: "xai"`，`baseUrl = https://api.x.ai/v1`。
3. **不允许** xAI + deepseek 组合：host 端 `validateProviderConfig` 统一归一化；已存的坏配置在读取时也归一化（老用户无需手工修文件）。
4. key 按 provider+baseUrl 分别保存；切换时复用该端点之前保存过的 key；DeepSeek 还可回退读取环境变量 `DEEPSEEK_API_KEY`。都没有时返回 `host_provider_api_key_required`（UI 显示中文提示，见问题 3）。
5. 头部标签按真实 provider 显示：`DeepSeek · deepseek-v4-flash` / `xAI Grok (API key) · grok-4.7`。

### 改动清单

| 文件 | 改动 |
| --- | --- |
| `packages/agent-host/src/provider.mjs` | 新增 `DEEPSEEK_BASE_URL`、`normalizeProviderModel(input)`；`validateProviderConfig` 先归一化；`ProviderStore` 增加 `keys`（按 `provider|baseUrl`）并在 `save()` 复用；`read()` 返回归一化配置；`DEEPSEEK_API_KEY` 回退。 |
| `src/features/agentHost/providerPresets.ts`（新） | 前端同款 `resolveProviderForModel(model)` + `SESSION_MODEL_PRESETS`（DeepSeek V4 Flash / DeepSeek Chat / Grok 4.7）。 |
| `src/features/agentHost/settingsClient.ts` | `setProvider` 接受 `provider`/`baseUrl`，缺省时由 `resolveProviderForModel` 推导，不再写死 `xai`。 |
| `src/features/agentHost/chatClient.ts` | `agentRuntimeHeader` 按 `apiProvider.provider/baseUrl` 输出 `DeepSeek` / `OpenAI 兼容` / `xAI Grok`；新增 `setProvider`、`setModel` 供头部切换器使用。 |
| `src/features/agentHost/types.ts` | 无需新增 runtime id（openai-compatible 仍跑在 `xai-api` runtime 槽位，即 provider-api runtime）。 |

### 验收

- [ ] 单测：`validateProviderConfig({ provider:'xai', model:'openai/deepseek-v4-flash' })` ⇒ `{ provider:'openai-compatible', baseUrl:'https://api.deepseek.com', model:'deepseek-v4-flash' }`。
- [ ] 单测：先存 xAI key，再存 DeepSeek key，再切回 xAI（不传 key）⇒ 成功且 bearer 为 xAI key。
- [ ] 单测：磁盘上已有 `{provider:'xai', model:'openai/deepseek-v4-flash'}` ⇒ `read()` 返回 deepseek 端点。
- [ ] 单测：`agentRuntimeHeader` 对 deepseek 配置不再包含 `xAI`。
- [ ] 手测：头部选 DeepSeek，无 VPN 发消息能得到回复。

## 问题 2（C）：头部 / composer 无样式，模型切换器错位

### 根因

- `src/features/conversations/ConversationWorkspace.vue` 的 `.conversation-header` / `.conversation-heading h2` / `.chat-icon-button` 等样式写在 `<style scoped>` 中；`AgentHostChat.vue` 是子组件，scoped 样式**不会**作用到子组件内部元素 ⇒ 浏览器默认样式（大号标题、原生 select、无边框 textarea）。
- `AgentHostChat.vue` 自身 scoped 样式只覆盖了气泡，头部与 composer 没有样式。
- 模型切换只存在于设置页 `AgentRuntimeSettingsPanel.vue`（自由输入框），Session 页没有就地切换器。

### 要求行为

- 紧凑工具栏（一行，≈48px）：左侧 = 对话标题（14px，省略号）+ 次级小字（项目名）；会话切换下拉 + 新建按钮；右侧 = 模型/runtime 下拉（样式化 `<select>`，显示当前 provider+模型，选项来自预设，切换即调用 `setProvider`）；「旧版对话」降级为小号文字链接。
- composer：圆角卡片、边框、自动高度 textarea、右下角发送/停止圆形按钮。

### 改动清单

| 文件 | 改动 |
| --- | --- |
| `src/features/agentHost/components/AgentHostChat.vue` | 重写 template 头部与 composer 结构，自带 scoped 样式（不依赖父组件）；新增模型下拉，切换后 emit `settingsChanged`。 |
| `src/features/conversations/ConversationWorkspace.vue` | 监听 `settingsChanged` 重新拉 settings（仅一行改动）。 |

### 验收

- [ ] `AgentHostChat.vue` 含 `data-testid="agent-host-model-picker"` 的 `<select>`，位于头部右侧容器 `.ahc-toolbar-right`。
- [ ] 「旧版对话」为 `.ahc-legacy-link`（文字链接样式）。
- [ ] 静态测试断言组件 scoped 样式内定义了 `.ahc-toolbar`、`.ahc-composer`。
- [ ] `npm run build` 通过。

## 问题 3（B）：原始错误码 + 重复错误行

### 根因

- `AgentHostChat.vue`：失败气泡渲染 `message.failureCode` 原文；`runtime.error` 事件把 `event.code` 写入 `error` ref，并在 composer 上方 `<p class="error">` 再显示一次 ⇒ 同一错误出现两次。

### 要求行为

- 新增 `describeAgentHostError(code)` ⇒ `{ title, hint }` 中文（英文 fallback 走 `tr`）。至少覆盖：`host_provider_network_unreachable`（网络不可达：检查网络/代理，或切换到 DeepSeek）、`host_provider_request_failed`（请求失败：检查模型 id 与 API key / 切换运行时）、`host_provider_unauthorized`（API key 无效）、`host_provider_api_key_required`（未配置 key：到 设置 › Agent runtime 填写）、`host_reply_already_running`、`host_runtime_busy`、超时、未知码（显示通用文案 + 小字原始码）。
- 失败气泡显示 title + hint，原始码仅作小字/tooltip。
- `runtime.error` 若对应当前运行消息（会刷新为 failed 气泡）则**不**再显示顶部错误行；顶部错误行只用于非消息级错误（加载项目失败、发送请求被拒等），且同样经过 `describeAgentHostError`。

### 改动清单

| 文件 | 改动 |
| --- | --- |
| `src/features/agentHost/errors.ts`（新） | `describeAgentHostError`。 |
| `AgentHostChat.vue` | 使用上面函数；移除 `runtime.error` → 顶部横幅的重复路径。 |

### 验收

- [ ] 单测：`describeAgentHostError('host_provider_network_unreachable').title` 含「网络」；未知码返回通用文案且保留原始码。
- [ ] 静态测试：`AgentHostChat.vue` 不再出现 `error.value = String(event.code`。

## 问题 4（D）：复制 / 编辑重发 / steer

### 根因

- `AgentHostChat.vue` 消息为纯 `<article><p>`，无操作按钮。agent-host 没有「截断历史」或「运行中注入消息」的 API（`server.mjs` 仅 send / cancel）。

### 要求行为（在现有 API 内实现，不做假 UI）

- **复制**：每条消息 hover 显示复制按钮（`navigator.clipboard.writeText`），复制成功短暂显示「已复制」。
- **编辑重发**（用户消息）：点击「编辑」把内容放回 composer 并聚焦；发送时作为新消息追加（host 无历史截断 API，**已知缺口**：旧消息保留在历史中）。
- **steer**：运行中 composer 不再禁用；输入后按钮变为「纠偏」，行为 = `cancel` 当前运行 → 等待 `message.completed`/取消落定 → 发送新消息（cancel+resend 语义；**已知缺口**：host 无真正的运行中注入，后续可加 `/api/runtime/steer`）。

### 验收

- [ ] 组件含 `data-testid="agent-host-copy"`、`agent-host-edit`、`agent-host-steer`。
- [ ] 运行中 textarea 未 `disabled`；steer 调用 `client.cancel` 后 `client.send`。

## 问题 5（E）：标题 + heartbeat 未上屏

### 根因

- host 已发 `conversation.updated`（`service.mjs` › `send()` 自动标题）与 `runtime.heartbeat`（`#run()` 每 `heartbeatMs`）事件，但 `AgentHostChat.vue` 的事件处理只处理 `message.delta` / `runtime.step` / `message.completed` / `runtime.error`。

### 要求行为

- `conversation.updated` ⇒ 更新 `conversations` 中对应标题（头部与下拉立刻变化）。
- `runtime.heartbeat` ⇒ 运行中显示「仍在运行 · 已用 Ns · 当前步骤」状态条；`message.completed` 后清除。

### 验收

- [ ] 静态测试：组件处理 `conversation.updated` 与 `runtime.heartbeat`，含 `data-testid="agent-host-heartbeat"`。

---

## 用户需要的配置

- DeepSeek key：桌面 设置 › Agent runtime（「高级 provider」）选 DeepSeek 模型并填入 API key 保存一次即可（存于 agent-host 数据目录 `secrets/model-provider.json`，权限 0600）；或在启动 sidecar 前设置环境变量 `DEEPSEEK_API_KEY`。
- runtime 需为「API（provider-api / `xai-api` 槽位）」；DeepSeek 与 xAI 共用这个槽位，由 provider 配置决定实际端点。

## 非目标

- 不改扫描 / 资产业务逻辑；不做真正的历史截断与运行中注入（记为缺口）；不移植旧版对话全部能力。
