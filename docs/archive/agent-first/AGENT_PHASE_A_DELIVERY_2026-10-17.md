# Agent 阶段 A 交付说明（2026-10-17）

分支：`feature/agent-first-app-platform-20261016`  
对照：`docs/AGENT_DEEP_DIAGNOSIS_AND_ROADMAP_2026-10-17.md` §5 阶段 A  
范围：agent-host / agent-ui / 桌面设置文案；**未改** Nest / assets 内核；无完整 Spur；无假 OAuth；未 pop `stash@{0}`。

## 做了什么（6 点整包）

| # | 目标 | 落地 |
|---|---|---|
| 1 | API `session.messages` 压缩 | `compressSessionMessages`（`memory.mjs`）：保留最近 4 个 user turn + 折叠旧 tool 为 facts/hypotheses；硬顶 96KB；每 turn 重写 system；长会话中途软压缩 |
| 2 | harness `setGoal` 接线 | host 不再注入空 ephemeral goal；`runtime.prompt({ userText })` 只用用户原句 `setGoal`；session harness 跨轮保留，结束时 `pinFact` 回写 SQLite memory |
| 3 | 结束前轻量验收 | `verifyBeforeEnd`（`agent-state.mjs`）：质量档数据题在 `end_turn` 前检查只读工具 / confirmedFacts / 假设当事实；失败最多强制 1 次 |
| 4 | OAuth 挂 send | `#run` 前 `ensureOAuthAccess()`；`ensureOAuthAccess({ force })` 同步 runtime bearer；`#modelTurn` 在未产出 text 的 401 上 force refresh 一次并重试 |
| 5 | 工具真名对齐 | `QUALITY_SYSTEM_PROMPT` / `DATA_TOOL_HINT` / `DATA_TOOL_NAME` / `humanizeToolTitle` 对齐 manifest：`inventory_*`、`project_overview`、`exposure_overview`、`task_status`、`journal_read`、`legacy_*`、`draft_*` |
| 6 | 主路径默认 xai-api | settings `preferredRuntime: 'xai-api'`；UI/Vue 文案标「问答主路径」；grok-acp 标「高级」；不破坏 net-proxy |

额外：`setAnswerMode` 热更新同步 `forceToolOnDataQuestion` / `reasoningEffort` / `useResponsesApi` / `timeoutMs`。

## 关键文件

- `packages/agent-host/src/memory.mjs` — `compressSessionMessages`
- `packages/agent-host/src/agent-state.mjs` — `verifyBeforeEnd`
- `packages/agent-host/src/quality-prompt.mjs` — 真名 / humanize
- `packages/agent-host/src/provider.mjs` — goal / 压缩 / verifier / 401 `#modelTurn`
- `packages/agent-host/src/service.mjs` — OAuth before send、去双份 harness、pinFact、preferredRuntime
- `packages/agent-ui/src/panels.js`、`src/features/agentHost/components/AgentRuntimeSettingsPanel.vue`
- `packages/agent-host/test/agent-phase-a.test.mjs`

## 验收怎么试

### 自动化

```bash
node --test --test-concurrency=1 packages/agent-host/test/agent-phase-a.test.mjs \
  packages/agent-host/test/agent-p1-harness.test.mjs \
  packages/agent-host/test/agent-quality.test.mjs \
  packages/agent-host/test/provider-agent-loop.test.mjs \
  packages/agent-host/test/xai-oauth.test.mjs
# 或全量：npm run test:agent-first
```

覆盖：messages 压缩、goal 仅用户句、verifier、ensureOAuth 被调用、401 force refresh、humanize 真名、preferredRuntime。

### 手测（诊断 §7，至少 1–5、7）

前置：桌面「用 Grok 登录」；runtime = xAI Grok（主路径）；answerMode = 质量；项目已 grant `inventory.read`（及需要时 investigation）。

1. 「当前项目有多少资产？」→ 出现「正在查询/统计…」；结论引用 `inventory_*` / `project_overview`；无空想整数。
2. 「列出已装应用和它们能读的工具」→ 依据 discovery，不编造 app id。
3. 「你好」→ 短中文；不强制工具。
4. 连续追问拆数字 / 「上一问还成立吗？」→ 引用确认事实或再查。
5. 撤掉 `inventory.read` 再问资产数 → 说明缺权限；不说「没有资产」。
7. OAuth 将近过期再问 → 应 refresh 成功或明确请重新登录；无裸 401 堆栈。

## 非目标（仍不做）

完整 Spur 状态机；Nest/assets 内核；假 OAuth；浏览器/邮件生态。

## 诊断 checklist 勾选

见同日更新的 `AGENT_DEEP_DIAGNOSIS_AND_ROADMAP_2026-10-17.md` §5 阶段 A。

## Node 环境说明

本机当前默认 Node 为 v26.10.0。阶段 A 相关单测（phase-a / p1 / quality / provider-loop / session-memory / xai-oauth / net-proxy）全绿。

依赖 app worker 子进程的部分测（authority / bundled-apps / upgrades / runtime-settings disclosure 等）在 **未改代码的 tip `25195d1`** 上同样以 `host_app_process_exited` 失败，属环境/既有问题，非本整包引入。未 pop `stash@{0}`。
