# Agent 问答质量 P1（2026-10-17）

分支：`feature/agent-first-app-platform-20261016`。在 P0（系统提示 / answerMode / tool-hint）之上继续提质，并吸收微信文《Spur：安全领域的原子求解状态机》中**可迁移到通用问答 agent** 的 harness 思想（**不是** Nest/Spur 渗透状态机）。

## 本轮落地

### 1. Responses API + reasoning.effort
- xAI（`api.x.ai`）质量/速度档默认走 `POST /v1/responses`（流式保留）。
- 质量档：`reasoning.effort=medium`；速度档：`low`（文档：grok-4.7 推理不可关，只能降 effort）。
- `store:false`，自管历史；解析 SSE（`response.output_text.delta` / function_call / completed）。
- 非 xAI 的 openai-compatible 仍走 Chat Completions。
- Responses 404/405/501 时回退 Chat Completions。

### 2. 长会话压缩
- `memory.mjs`：滑动窗口保留最近 4 轮原文；更早轮次折叠为 `confirmedFacts` / `hypotheses` + `compressedSummary`。
- 工具成功 → 确认事实；失败 → 带范围的假设（「本次未成功 ≠ 不存在」）。

### 3. Spur/Cairn 可迁移 harness（`agent-state.mjs`）
| 思想 | 实现 |
|---|---|
| Fact vs 猜测 | `confirmedFacts` vs `hypotheses`；prompt 禁止把假设当事实 |
| 防偷懒 | 质量档数据题：目标未满足且无新证据 → 禁止空结束（最多 1 次强制） |
| 防遗忘 | 确认事实写入 memory；harness state 注入每轮 |
| 防死胡同 | 相同失败签名计数；触发 reviewer + deep-think |
| 局部 vs 全局 | 压缩：结构化事实摘要 + 最近原文窗口 |
| 失败写清范围 | strategy hint 与假设文案均带「本次/本工具」范围 |
| Reviewer 轻量 | 每 4 步或死胡同时注入方向审查 hint（不另起 multi-agent） |
| deep-think | 受阻时第一性原理提示（非渗透剧本） |

### 4. 工具编排
- 质量档数据问句：`tool_choice=required`（首轮无工具时）。
- 并行工具保留；空转轮次用空结束 guard / reviewer 打断。

### 5. 设置
- `settings.answerModeOptions` 增加 `reasoningEffort` / `useResponsesApi` / `forceToolOnDataQuestion`。
- UI 展示 reasoning 档位。

## 明确不做 / 诚实差距
- **未**实现完整 Spur/Cairn 渗透原子求解状态机；**未**改 Nest homogeneous / assets / scanning 内核。
- 未达 CyberGym 级评测；本轮是 harness 思想的轻量移植。
- Responses 流式事件形态若 xAI 微调字段名，解析器可能需跟进。
- `previous_response_id` 未作为主路径（隐私：`store:false`）。
- 完整 tauri debug bundle 视本机网络/时间；`build:agent-first` 必须绿。

## 验收试问
1. 「当前项目有多少资产？」— 应强制查工具；结论引用工具；计划步骤中文。
2. 「列出已装应用」— 同上；空结束会被挡一次。
3. 连续 8+ 轮后追问前轮数字 — memory 压缩后仍应记得确认事实。
4. 速度档闲聊「你好」— reasoning=low，可直接答。
5. 故意无权限场景 — 失败文案含范围，不说「不存在」。

## 测试
```bash
cd packages/agent-host
npx node@24 --test --test-concurrency=1 \
  test/agent-p1-harness.test.mjs \
  test/agent-quality.test.mjs \
  test/provider-agent-loop.test.mjs \
  test/session-memory.test.mjs
```
