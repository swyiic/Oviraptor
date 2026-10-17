# Agent：Grok OAuth / xAI 接入与速度·智能方案（2026-10-16）

## A) 当前模型/provider 配置（实读代码）
- `packages/agent-host/src/service.mjs`：运行时只有 `none` / `grok-acp`。`settings().modelProvider = { source:'legacy-desktop', availableHere:false }`——独立 host **没有** provider 配置；模型智能全在官方 Grok CLI（ACP）里。
- `packages/runtime-acp/index.mjs`：ACP v1 stdio 传输，只接受 CLI 广告的 `cached_token`；广告仅 `grok.com`/`oidc` 时报 `ACP_CACHED_AUTH_UNAVAILABLE`（authentication_required）；`xai.api_key` 广告被显式拒绝。宿主从不读写 OAuth token。
- 桌面端（`src-tauri/src/model_configuration.rs`、`conversations/reply.rs` 的 `OpenAiCompatibleGateway`、`ConfigDialog.vue`）：已有 **OpenAI 兼容** 多 profile（apiBase + apiKey + model），但属于旧桌面，不在 agent-host 内。
- UI `packages/agent-ui/src/panels.js`：设置页只有 runtime 下拉 + 「provider 留在桌面」说明。

## B) Grok / xAI OAuth（2026-10-17 更新）
- **主路径**：复用公开 Grok-CLI OAuth 客户端（与 OpenCode `packages/opencode/src/plugin/xai.ts` 相同）
  - `CLIENT_ID=b1a00492-073a-47ea-816f-4c329264a828`
  - Device-code：`POST https://auth.x.ai/oauth2/device/code` → 打开 `verification_uri` / 展示 `user_code` → 轮询 `https://auth.x.ai/oauth2/token`
  - Scope：`openid profile email offline_access grok-cli:access api:access`；referrer=`oviraptor`
  - 成功后本机存 access+refresh（`<dataDir>/secrets/model-provider.json`，0600），Bearer `access_token` 调 `https://api.x.ai/v1`
  - 实现：`packages/agent-host/src/xai-oauth.mjs`；API：`POST /api/settings/provider/oauth/start|poll|logout`
- **备选**：PKCE + loopback helpers 已提供（`plan=generic`）；xAI 对非白名单 redirect 可能拒绝，故桌面主按钮走 device-code。
- **高级**：手动 API Key 仍可用，UI 折进「高级」。
- **另**：`grok-acp` 仍走官方 Grok CLI cached_token。

**诚实说明**：这不是自注册的「Oviraptor 专用 OAuth app」，而是复用 Grok-CLI 公开 client（OpenCode 同款）。无伪造假 OAuth。

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
