# Agent：Grok OAuth / xAI 接入与速度·智能方案（2026-10-16）

## A) 当前模型/provider 配置（实读代码）
- `packages/agent-host/src/service.mjs`：运行时只有 `none` / `grok-acp`。`settings().modelProvider = { source:'legacy-desktop', availableHere:false }`——独立 host **没有** provider 配置；模型智能全在官方 Grok CLI（ACP）里。
- `packages/runtime-acp/index.mjs`：ACP v1 stdio 传输，只接受 CLI 广告的 `cached_token`；广告仅 `grok.com`/`oidc` 时报 `ACP_CACHED_AUTH_UNAVAILABLE`（authentication_required）；`xai.api_key` 广告被显式拒绝。宿主从不读写 OAuth token。
- 桌面端（`src-tauri/src/model_configuration.rs`、`conversations/reply.rs` 的 `OpenAiCompatibleGateway`、`ConfigDialog.vue`）：已有 **OpenAI 兼容** 多 profile（apiBase + apiKey + model），但属于旧桌面，不在 agent-host 内。
- UI `packages/agent-ui/src/panels.js`：设置页只有 runtime 下拉 + 「provider 留在桌面」说明。

## B) 现成 OAuth / xAI SDK？
- 仓库内无 xAI SDK、无 OAuth 客户端。唯一“OAuth”是 Grok CLI 自己的登录（grok.com / oidc），由 CLI 持有。
- xAI 官方现状（WebSearch/WebFetch docs.x.ai，2026-10）：推理 API `https://api.x.ai/v1`，**OpenAI 兼容**，认证 = `Authorization: Bearer <API key>`（console.x.ai 创建）。`/v1/me` 文档提到可接受 OAuth token，但**没有公开的第三方 OAuth 客户端注册/设备码流程**。

**结论**：不伪造 OAuth。两条官方路径并存：
1. `grok-acp`：用户用官方 Grok CLI 登录（这就是 Grok Build 的 OAuth），宿主只走 cached_token —— 已有。
2. 新增 `xai-api`：API key 安全存储 + OpenAI 兼容端点（默认 `https://api.x.ai/v1`、`grok-4.7`，与 Grok Build 同模型）。

## C) agent 循环现状
- 循环在 runtime 内（Grok CLI）；host 负责 prompt 组装（项目 + 发现摘要 + 记忆）、事件归一（`text_delta→message.delta`、tool_call→`runtime.plan/step`）、MCP 授权调用（`#mcpAppCall` 带 3 次有界重试）。
- 流式：✅ 已有（ACP chunk → SSE `/api/events` → UI `message.delta`，有 UI 测试）。
- 并行 tools/call：bridge `MAX_CONCURRENT=16` 已允许并发请求，但是否并发取决于 CLI；host 自有循环此前不存在。
- 超时：ACP prompt 120s、MCP 请求 30s、app 15s。上下文裁剪：记忆 8KB、发现摘要有上限；**工具结果无截断**（仅 bridge 128KB 硬上限，超限直接断开）。
- 失败处理：结构化错误码 + retryable，但没有给模型的“换策略”文案。

## D) 差距清单（按影响排序）与本轮处理
| # | 差距 | 处理 | 状态 |
|---|---|---|---|
| 1 | host 内无可直接配置的 Grok 模型 | `xai-api` 运行时：OpenAI 兼容流式循环 | ✅ |
| 2 | 工具结果可能超 128KB 导致 bridge 断开 | `truncateMcpResult` 16KB + “请缩小查询再读” | ✅ |
| 3 | 多工具串行 | `runToolCalls`：按依赖分波，独立并发（`parallel_tool_calls:true`） | ✅（xai-api 循环内；grok-acp 由 CLI 决定） |
| 4 | 规划弱 | `PLANNING_GUIDANCE` 注入每轮 prompt（plan→逐步执行→并行独立读） | ✅ |
| 5 | 失败后重复同样调用 | `strategyHint(code)`：bridge 错误 `data.hint` + 循环内 tool 结果 | ✅ |
| 6 | 流式 | 已有；新循环直接解析 SSE chunk → text_delta | ✅ |
| 7 | 上下文压缩（长会话 messages 累积） | 未做：需真 key 观察 token 用量 | 待做 |
| 8 | Responses API / 推理努力参数 / prompt cache | 未做 | 待做 |

## 必须真 key 才能 e2e 的
- xai-api 真实往返、`parallel_tool_calls` 实际行为、401/429 文案、首 token 延迟对比 Grok Build。
- grok-acp 的真实登录。所有单测用 mock fetch / 合成 runtime，无真实密钥。

## 安全
- key 存 `<dataDir>/secrets/model-provider.json`，目录 0700、文件 0600；`settings`/HTTP/事件/错误里只出现 `apiKeyConfigured`。
- baseUrl 仅 https（或 loopback http），禁止 URL 内凭据/query。
- 切换 provider 变更 `runtimeIdentity`（`xai-api:N`），已有披露授权按身份失效，需重新确认。
