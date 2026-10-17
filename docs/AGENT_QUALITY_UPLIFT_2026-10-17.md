# Agent 问答质量提升（2026-10-17）

分支：`feature/agent-first-app-platform-20261016`。目标：把内置 agent 的问答行为拉近「Grok Build / Grok Bot」管线水准——**保速**（流式、并行工具、截断）同时**显著提质**（先结论、先查证、少胡说）。

> 诚实说明：这是对齐行为与管线，**不是**声称已达到与 Grok Bot 完全同一智能。

## 差距盘点（最多 8 条 → 可测改动）

| # | 短板 | 可测改动 |
|---|---|---|
| 1 | 系统提示几乎只有 plan 英文策略，无中文优先/禁止编造/先查工具 | `quality-prompt.mjs` 注入 `QUALITY_SYSTEM_PROMPT` |
| 2 | 事实题未强制先读 inventory / project | 质量档 tool-hint guard（最多 1 次补查） |
| 3 | 无质量/速度档 | `answerMode=quality\|speed`（默认 quality，仍流式） |
| 4 | 工具结果截断只留头部 | head+tail 16KB 截断 |
| 5 | 计划步骤显示原始工具名 | `humanizeToolTitle` →「正在查询资产表」等 |
| 6 | 会话记忆可能写入密钥形态 | `scrubSecrets` 写入前脱敏 |
| 7 | 每轮 prompt 缺质量提醒与上海时区时间 | `buildTurnPrompt` + `Asia/Shanghai` |
| 8 | 设置 UI 看不到档位 | agent-ui / Vue 设置增加问答档位 |

**已保留且不破坏**：`parallel_tool_calls:true`、流式 SSE、OAuth device-code、net-proxy / macOS scutil 注入、16KB 工具截断预算、paired skills / discovery / grants。

## 改了什么（P0）

### P0-A 系统提示
- 新模块 `packages/agent-host/src/quality-prompt.mjs`
- 中文优先、先结论、禁止编造仓库/表数据、事实先用只读工具、少套话
- 注入：当前时间（Asia/Shanghai）、已装应用/工具/Skill 仍走 discovery 摘要、项目绑定信息

### P0-B 工具编排
- 并行工具默认开启（请求体 `parallel_tool_calls: true`）
- 质量档：若模型未调工具就断言具体资产/数量，host **最多强制 1 次** tool hint，避免死循环
- 速度档关闭该 guard
- 截断改为 head+tail，错误码 + strategy hint 仍返回模型

### P0-C 模型/速度
- 默认模型保持公开文档当前旗舰 **`grok-4.7`**
- `answerMode`：quality（temp 0.3 / maxSteps 12 / maxTokens 4096 / guard on）vs speed（更短更快）
- OAuth「用 Grok 登录」主路径未改

### P0-D 会话记忆
- 多轮仍注入 host session memory
- 写入前 scrub `sk-` / `xai-` / Bearer / api_key 等形态

### P0-E 测试
- 新增 `test/agent-quality.test.mjs`（系统提示、启发式、档位、guard、脱敏、UI 步骤文案、settings API）
- 更新 `provider-agent-loop.test.mjs` 截断与 system 断言

### P0-F UI
- agent-ui 设置：问答档位下拉
- 桌面 `AgentRuntimeSettingsPanel.vue`：质量/速度选择，与 runtime 一并保存
- 聊天 Plan progress：工具步骤显示「正在查询…」类中文标题

## 如何体感 / 验收试问

前置：登录 Grok（OAuth）或配置 xAI key，runtime 选 `xAI Grok`，会话绑定有 grant 的项目，Asset 应用已启用并授予 `inventory.read`。

1. **「当前项目有多少资产？」** — 应先出现计划步骤「正在查询资产表 / 项目概览」，再给数字；不应空想数字。
2. **「列出已装应用」** — 应依据 capability/discovery，不编造应用 id。
3. **「你好」** — 可直接闲聊，不强行调工具。
4. 切到 **速度档** 再问事实题 — 仍不应编造；可能更短，guard 关闭。
5. 连续追问「刚才那个数字按域名拆一下」 — 应带着上一轮工具摘要继续，而不是失忆重编。

## 剩余差距（诚实）

- 尚未切到 xAI Responses API / 推理 effort（仍走 Chat Completions 流式）
- 长会话 messages 压缩仍粗（记忆 8KB 有界）
- grok-acp 路径的智能仍主要在官方 CLI 内；本轮强化的是 host 拼装的 prompt + xai-api 循环
- 与 Grok Bot 完整工具生态（浏览器、邮件等）不对齐——Oviraptor 范围是项目/资产/扫描只读工具

## 验收命令

```bash
cd packages/agent-host
npx node@24 --test --test-concurrency=1 \
  test/agent-quality.test.mjs \
  test/provider-agent-loop.test.mjs \
  test/session-memory.test.mjs
```

说明：部分依赖 app worker 的集成测若出现 `host_app_process_exited`，属本机 worker 进程环境问题（与本轮质量改动无关；clean HEAD 同样复现）。质量相关单测应全绿。
