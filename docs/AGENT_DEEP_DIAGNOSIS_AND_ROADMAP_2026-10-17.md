# Agent 深度诊断与统一路线图（2026-10-17）

分支：`feature/agent-first-app-platform-20261016`  
权威 tip（写作时）：`131de90` — P1；**阶段 A 整包见后续 commit + `AGENT_PHASE_A_DELIVERY_2026-10-17.md`**  
范围：只读分析 + 本文档；**不改** Nest / assets 内核，不塞完整 Spur 渗透状态机。  
对照：Spur/Cairn harness 可迁移思想；Grok Build / 高质量助手行为；已落地 P0（`16463f6`）与 P1（`131de90`）。

---

## 1. 一句话总判

当前 agent 已是「**可用的本地 xAI 循环 + 薄质量护栏**」：流式、并行工具、OAuth、Responses+reasoning、Fact/假设分层都有了；离「**Grok Build 级问答手感 + Spur 级长程可靠**」还差一整层——**上下文工程（API messages 不压缩）、独立验收、数据面工具语义对齐、死码/漂移清理、以及一次体感跃迁的编排打包**，不是再叠十个小 prompt PR。

诚实边界：P0/P1 **不是玩具**（有单测、有真实循环、有 UI 档位），但也 **不是生产级长程 agent**（护栏多为「最多强制 1 次」的启发式；Reviewer 是注入一段 user hint，不是独立验收者；SQLite memory 压缩了，runtime `session.messages` 没有）。

---

## 2. 证据盘点（按路径）

### 2.1 Provider / 模型接口

| 现状 | 证据 |
|---|---|
| 默认模型 `grok-4.7`，base `https://api.x.ai/v1` | `packages/agent-host/src/provider.mjs` `PROVIDERS.xai` |
| 质量/速度档默认走 Responses（`POST /v1/responses`），`reasoning.effort` medium/low，`store:false`，带 `prompt_cache_key` | `OpenAiCompatibleRuntime.#responsesTurn`；`quality-prompt.mjs` `answerModeOptions` |
| 404/405/501 回退 Chat Completions | `#responsesTurn` |
| 非 xAI openai-compatible 仍走 Chat Completions | `prefersResponsesApi` |
| 流式：Responses 解析 `response.output_text.delta` / function_call；Chat 解析 `choices[].delta` | `readResponsesStream` / `readChatStream` |
| `parallel_tool_calls: true`；工具结果 16KB head+tail 截断 | `runToolCalls` / `truncateToolResult`；`MAX_TOOL_RESULT_BYTES = 16384` |
| maxSteps：quality 12 / speed 6；temp 0.3/0.4；maxTokens 4096/2048；timeout 300s/120s | `answerModeOptions` |
| **缺口**：`previous_response_id` 未主路径；reasoning 加密项仅弱保留；**401 不触发 refresh 重试** | `provider.mjs` 401→`host_provider_unauthorized`；见 OAuth |

### 2.2 Agent 循环 / 编排

| 现状 | 证据 |
|---|---|
| 主智能路径：`xai-api` → `OpenAiCompatibleRuntime.prompt` 多步循环 | `service.mjs` `#providerRuntime` / `#run` |
| 备选：`grok-acp`（官方 CLI，智能在 CLI 内；host 只拼 prompt + 归一事件） | `RUNTIME_IDS`；`packages/runtime-acp` |
| 每轮 host 拼：project + discovery + SQLite memory + ephemeral harness + 质量提醒 + 用户原文 | `buildTurnPrompt` + `#run` L376–399 |
| 质量档：数据题首轮 `tool_choice=required`；事实断言无工具 → DATA_TOOL_HINT（最多 1 次）；空结束 goal unmet → 最多 1 次强制 | `provider.mjs` L551–605；`agent-state.mjs` `shouldBlockEmptyCompletion` |
| 死胡同：相同失败签名 ≥2 → Reviewer + deep-think hint | `isDeadEnd` / `MAX_IDENTICAL_FAILURES=2` |
| Reviewer：每 4 步或死胡同注入 **同一会话的 user 消息**，非独立 agent | `REVIEWER_HINT` / `DEEP_THINK_HINT` |
| 计划 UI：tool 标题 `humanizeToolTitle` →「正在查询资产表」等 | `service.mjs` `#normalizePlanStep` |
| **缺口 A**：host `#run` 用 `emptyAgentState()` 拼 ephemeral harness，runtime 另有 session harness → **双份注入**；且 `setGoal(session.harness, userText)` 的 `userText` 实际是整段 turn prompt，goal 会被撑成大段 blob | `service.mjs` L389–390；`provider.mjs` L532–542 |
| **缺口 B**：`session.messages` **跨轮无限累积**，无滑动窗口；只有 SQLite `memory.mjs`（8KB）压缩 | `provider.mjs` sessions；`memory.mjs` `MAX_TOTAL_BYTES=8192` |
| **缺口 C**：`setAnswerMode` 热更新未同步 `forceToolOnDataQuestion` / `reasoningEffort` / `useResponsesApi` / `timeoutMs`（下一轮 `prompt` 的 `#applyMode` 会补，但仍易误导） | `service.mjs` L72–83 |

### 2.3 Tools / 数据面

| 现状 | 证据 |
|---|---|
| MCP：bridge stdio→loopback HTTP；`MAX_CONCURRENT=16`；响应硬顶 128KB（`MAX_RESPONSE=65536*2`）；工具调用 30s timeout；有界重试 | `mcp-bridge.mjs` |
| Host `apps.call`：grant + `disclosure_runtime` + `grantRevision`；失败 strategy hint | `service.mjs` `#mcpAppCall`；`STRATEGY_HINTS` |
| **Assets 已暴露工具**（`apps/assets/app.package.json`）：`inventory_stats/search/schema/query`、`project_overview`、`exposure_overview`、`inventory_import`、`legacy_snapshot_stats` | manifest `tools[]` |
| **Scanning 已暴露工具**：`task_status`、`journal_read`、`draft_create/cancel`、`legacy_saved_scans` | 同上 |
| Discovery：工具 purpose + required 参数，12KB 有序截断 | `discovery.mjs` `MAX_SUMMARY_BYTES=12288` |
| **漂移**：系统提示 / DATA_TOOL_HINT / humanize 仍写 `saved_assets_*`、`scan_list/status/draft_read`、`inventory_aggregate`；manifest **没有**这些名字；manifest 有的 `task_status`/`journal_read`/`inventory_stats` **不在** humanize 表 | `quality-prompt.mjs` L14–29、L68–81 vs manifests |
| 桌面仍有大量 Tauri 只读 command（logs/events/ownership/dashboard…）未注册为 MCP tool | 见既有 `docs/AGENT_DATA_ACCESS_ANALYSIS_2026-10-16.md` |

### 2.4 Memory / State

| 现状 | 证据 |
|---|---|
| 会话记忆：最近 4 轮原文 + 旧轮折成 `facts`/`hypotheses`/`compressedSummary`；总预算 8KB；写前 `scrubSecrets` | `memory.mjs` |
| 回合 harness：`confirmedFacts`≤12、`hypotheses`≤6、`openIntents`≤4 | `agent-state.mjs` |
| 记忆进 prompt 为 JSON 字面量 +「不可信、无授权」声明 | `formatMemoryPromptBlock` / `formatAgentStatePromptBlock` |
| **无**独立「事实图 / 报告层 / 原始会话」三层存储；无跨会话项目级记忆；无 Verifier 回写 | — |

### 2.5 UI / OAuth / Sidecar

| 现状 | 证据 |
|---|---|
| agent-ui：runtime 三选一 + answerMode 质量/速度 + 高级 API key | `packages/agent-ui/src/panels.js` / `settings.js` |
| 桌面配置中心：device-code「用 Grok 登录」、answerMode、runtime 保存 | `AgentRuntimeSettingsPanel.vue` |
| OAuth：公开 Grok-CLI client_id `b1a00492-…`；device-code；scope 含 `api:access`；token 文件 0600 | `xai-oauth.mjs` `XAI_OAUTH`；`ProviderStore` |
| `ensureOAuthAccess()` **已实现但未被 send/server 调用** → 过期 token 易直接 401 | `service.mjs` L202–213；`server.mjs` 无引用 |
| Sidecar：Tauri 拉起 Node agent-host；注入 macOS 系统代理 | `src-tauri/src/agent_host_sidecar.rs`；`net-proxy.mjs` |
| 可观测：`runtime_metrics` 有 firstText/firstTool/total；**官方性能对比位显式 false** | `metrics.mjs` `officialPerformanceVerified: false` |

### 2.6 已落地 P0/P1（诚实评价）

| 交付 | SHA | 够什么 | 仍像玩具的地方 |
|---|---|---|---|
| P0 质量提示 / answerMode / tool-hint / head-tail / humanize / scrub | `16463f6` | 中文优先、事实题倾向查工具、UI 可见档位 | 启发式正则；强制最多 1 次；工具名已漂移 |
| P1 Responses+reasoning / memory 压缩 / harness Fact·死胡同·Reviewer | `131de90` | 吃到 xAI reasoning；长会话 SQLite 侧不立刻爆 | Reviewer=提示词；API messages 不压缩；无独立验收；ensureOAuth 死码 |

单测体量（本机路径）：`agent-p1-harness` 274 行、`agent-quality` 249、`provider-agent-loop` 119、`session-memory` 127——**单元/合成覆盖够写文档结论，不足以当 CyberGym/真人对比**。

---

## 3. 能力矩阵

| 目标能力 | Grok Build 理想 | Spur/Cairn 可迁移理想 | 当前实现 | 缺口 |
|---|---|---|---|---|
| 智能问答（先结论、中文、不编造） | 默认行为 + 强模型 | Fact≠假设写入黑板 | 系统提示 + 启发式 guard | **P1**（语义对齐工具名；减少假成功） |
| 工具使用（该查就查、并行、失败换策略） | 原生 tool loop | 结构化工具提交 + 失败范围 | 有；`tool_choice`/hint/strategyHint | **P1**（工具面窄+名称漂移） |
| 速度（首 token、流式、并行） | 流式 + 缓存 | 局部执行 vs 全局决策分离 | SSE 流式；并行工具；prompt_cache_key | **P2**（真机对比未做；messages 膨胀拖慢） |
| 长程可靠（多轮不丢、不死胡同） | 会话记忆 + 压缩 | 分层记忆；防偷懒/遗忘/死胡同 | SQLite 8KB 压缩 + 轻 harness | **P0**（API messages 无压缩；goal 污染） |
| 验收 / Verifier | 隐式自检 | 独立 Reviewer/Verifier | 注入 hint，无独立验收 | **P0**（数据题结束前硬验收一轮） |
| 可观测 | 延迟/工具轨迹可见 | 状态机可回放 | plan/step + metrics 表 | **P1**（缺「本轮用了哪些确认事实」面板） |
| 登录 / 凭证寿命 | 无感续期 | — | device-code 可用；refresh **未挂发送路径** | **P0** |
| 应用数据面 | 浏览器/邮件等生态 | 原子求解边界清晰 | 资产/扫描只读为主 | **P1**（扩展只读聚合，不碰内核） |
| 安全边界 | 权限明示 | 失败写清范围 | grant/revision/披露；范围文案有 | 保持；**不做**假 OAuth |

缺口等级：`P0`=下一整包必须体感；`P1`=同包或紧随；`P2`=可测后优化。

---

## 4. 根因（机制，不是症状）

1. **薄 prompt + 浅 harness，不是完整控制面**  
   行为主要靠 `QUALITY_SYSTEM_PROMPT` 与「最多强制 1 次」的 host 插入消息。没有「结束前必须通过验收谓词」的硬门；模型仍可在第二次尝试后空结束或软编造。

2. **上下文工程只做了一半**  
   `memory.mjs` 压缩的是 **SQLite 侧摘要**；真正打进模型的 `session.messages`（含整段 discovery/memory/harness 的 user 包）**从不裁剪**。长会话必然变慢、变糊、撞上下文——这是长程不可靠的主因，不是「再加一句防遗忘」。

3. **Harness 状态接线错位**  
   Host 每轮用空 state 拼 ephemeral goal；Runtime 用完整 turn prompt 当 `userText` 调 `setGoal` → goal/intent 被污染。黑板思想有了，**接线把黑板弄脏**。

4. **无独立验收者**  
   Reviewer/deep-think 与主循环共上下文、共诱因，不能当 Verifier。Spur 里「求解 vs 验收」分离的价值，这里还没落地。

5. **模型接口未吃满**  
   Responses + medium effort 已接；但缺：发送前 OAuth refresh、401 单次刷新重试、`previous_response_id`/服务端状态策略取舍说明、真机 first-token 基线。接口「接通」≠「运营可靠」。

6. **应用数据面工具语义漂移 + 覆盖不足**  
   Manifest 已演进到 `inventory_*` / `task_status`，提示词与 humanize 仍混旧名。模型被引导去调不存在的工具 → 空转或误判「读不到」。同时 logs/events/ownership 等桌面能力仍未工具化——智能上限被数据面卡住。

7. **双 runtime 叙事分裂**  
   质量护栏几乎只在 `xai-api`。用户若走 `grok-acp`，P0/P1 大多无效。产品未强制「问答主路径 = xai-api」。

---

## 5. 统一路线图（2 个阶段；默认下一阶段一次做完体感跃迁）

### 阶段 A —「一次体感跃迁」整包（建议立刻开工）

> **状态（2026-10-17）**：已整包落地，详见 `docs/AGENT_PHASE_A_DELIVERY_2026-10-17.md`。
> Checklist：✅ messages 压缩 ✅ setGoal 接线 ✅ 轻量 Verifier ✅ OAuth 挂 send ✅ 工具真名 ✅ 主路径 xai-api。


**目标**（用户能感到：更稳、更记得住、少胡说、登录不断）：  
1. **API messages 压缩**：与 memory 同构——保留最近 N 轮原文 + 折叠旧工具结果为 confirmed/hypothesis；硬顶字节；每 turn 重写 system。  
2. **Harness 接线修正**：只用用户原始问句 `setGoal`；host 不再注入空 ephemeral 双份；session harness 跨轮保留并写回 memory facts。  
3. **结束前验收（轻量 Verifier）**：质量档数据题，在 `end_turn` 前跑确定性检查（本轮是否调用只读数据工具 / 是否引用 confirmedFacts / 是否把 hypotheses 当事实）。失败则 **一次** 定向补查或改写，再允许结束。仍单 agent，不另起进程。  
4. **OAuth 寿命**：`send`/`#run` 前调用 `ensureOAuthAccess`；401 时 refresh 一次并重试同一 turn（有界）。  
5. **工具语义对齐**：提示词、DATA_TOOL_HINT、humanize、DATA_TOOL_NAME **只认当前 manifest 真名**；补 `task_status`/`journal_read`/`inventory_stats` 中文标题；删或降级旧别名。  
6. **主路径产品默认**：设置文案与默认引导「问答用 xAI Grok（OAuth）」；grok-acp 标为高级/CLI。

**非目标**：完整 Spur 状态机；Nest/assets 内核；假 OAuth app；浏览器/邮件生态；CyberGym 评测平台。

**验收标准**：  
- 单测：messages 压缩、goal 仅含用户句、验收门、OAuth ensure 被调用（mock）、humanize 覆盖 manifest 工具名。  
- 手测剧本见 §7（至少 1–5、7 必过）。  
- `build:agent-first` / 既有 quality+p1 测试绿。

**预估触及文件**（业务面仍限 agent-host/ui/桌面设置）：  
- `packages/agent-host/src/provider.mjs`  
- `packages/agent-host/src/agent-state.mjs`  
- `packages/agent-host/src/memory.mjs`  
- `packages/agent-host/src/quality-prompt.mjs`  
- `packages/agent-host/src/service.mjs`  
- `packages/agent-host/src/server.mjs`（若需暴露诊断字段）  
- `packages/agent-ui/src/panels.js`、`settings.js`  
- `src/features/agentHost/components/AgentRuntimeSettingsPanel.vue`（文案）  
- 测试：`test/agent-p1-harness.test.mjs`、`agent-quality.test.mjs`、`provider-agent-loop.test.mjs`、`xai-oauth.test.mjs`（增补）  
- 本文档更新为「阶段 A 完成」勾选  

**风险**：Responses 事件字段若 xAI 变更需跟解析器；压缩过激可能丢指代（用 confirmedFacts 兜底）；401 重试需防双扣费（只重试未产出 text_delta 的失败）。

### 阶段 B —「数据面与可观测」整包（阶段 A 体感过关后再开）

**目标**：扩大 **只读** 工具（journal/runs 摘要、ownership 白名单聚合等——沿 `AGENT_DATA_ACCESS_ANALYSIS` P2–P4，经 manifest+grant，不碰 Nest 内核实现细节）；UI 展示「本轮确认事实 / 假设 / 缺口」；可选 `reasoning.effort=high` 手动档；真机 first-token 基线写入 metrics 文档。

**非目标**：写操作智能体化；跨项目自由查询默认开启；完整 penetration blackboard。

**验收**：授权项目上 §7 剧本 6–8；无 grant 时失败文案含范围；metrics 页能看到 api/reasoningEffort 字段（若已打点）。

**风险**：工具一多 discovery 截断更明显——需同步抬高/分层摘要，而不是只加工具。

---

## 6. 明确不做

| 不做 | 原因 |
|---|---|
| 把完整 Spur/Cairn **渗透**原子求解状态机塞进 Nest | 与 agent-first 问答宿主同质化冲突；用户已禁碰 Nest 协调器 |
| 假 OAuth / 伪造专用 client | 安全与合规；继续复用公开 Grok-CLI device-code |
| 改 assets / scanning **内核**实现与表结构 | 归属边界；只经 app manifest 工具扩展 |
| 为「看起来像 Grok Bot」堆浏览器/邮件工具 | 超出 Oviraptor 项目数据助手范围 |
| 再拆十个「只改一句 prompt」的碎片 PR | 体感跃迁需要阶段 A 整包 |

---

## 7. 给用户的试用 / 验收剧本（7 条）

前置：桌面「用 Grok 登录」成功；runtime = `xAI Grok`；answerMode = 质量；会话绑定已 grant `inventory.read`（及需要时 `investigation.read` / `inventory.exposure.read`）的项目；Asset/Scanning 应用已启用。

1. **「当前项目有多少资产？」**  
   期望：先出现「正在查询…」类步骤；结论引用 `inventory_query` / `project_overview` / `inventory_stats`；无空想整数。

2. **「列出已装应用和它们能读的工具」**  
   期望：依据 discovery/capabilities，不编造 app id；工具名与设置里一致。

3. **「你好」**  
   期望：短中文闲聊；不强制工具；速度可接受。

4. **连续追问**：「刚才那个数字按域名/来源拆一下」→「上一问的结论还成立吗？」  
   期望：引用确认事实或再查；不装作失忆后重编。

5. **无权限场景**：临时撤掉 `inventory.read` 再问资产数。  
   期望：说明缺权限/范围；**不**说「项目没有资产」。

6. **扫描侧**：「最近任务状态 / journal 摘要」（需 investigation grant）。  
   期望：走 `task_status` / `journal_read`；步骤标题中文；失败写范围。

7. **登录寿命**：OAuth 登录后闲置至 access 将近过期（或 mock 过期）再问 1。  
   期望：阶段 A 后应 refresh 成功或明确「请重新登录」；不应裸 401 堆栈。

8. **速度档对照**：切速度档问「你好」与简单事实题。  
   期望：reasoning=low、更短；仍不编造；事实题仍宜查工具（guard 关闭时至少不瞎报数）。

---

## 8. 附录

### 8.1 与 Spur/Cairn 思想对照（可迁移 ⊂ 问答 harness）

| Spur/Cairn 思想 | 当前 | 阶段 A 目标 |
|---|---|---|
| 黑板 Fact / Intent | `confirmedFacts` / `openIntents` / `hypotheses` 有；接线脏 | 纠正 setGoal；单一 blackboard |
| reason ↔ explore | 隐式多步 loop | 验收门区分「该查 / 该答」 |
| 分层记忆 | 仅 SQLite 摘要 + 最近 4 轮 | + API messages 同构压缩 |
| 防偷懒 | 空结束/无工具强制各 ≤1 | 结束前确定性验收 |
| 防遗忘 | memory 注入 | messages 压缩保留 facts |
| 防死胡同 | 相同失败计数 + hint | 保留；换参/换工具提示强化 |
| Reviewer / Verifier | 只有 Reviewer 文案 | 加轻量 Verifier 谓词 |
| 失败范围 | strategyHint + 假设文案 | 保持 |
| 结构化工具提交 | JSON args + schema | 保持；对齐真名 |
| 局部执行 vs 全局决策 | 并行工具 vs host 策略 | 压缩=全局摘要，近轮=局部 |
| deep-think | 文案注入 | 仅死胡同时 |
| 原子求解边界 | **不**引入渗透原子 | 明确边界：问答验收原子即可 |

### 8.2 与已推 SHA 对照

| SHA | 内容 |
|---|---|
| `8da4ca5` | xAI API provider、流式循环、并行工具、截断、strategy hint |
| `7b21866` | Grok device-code OAuth + 侧栏应用恢复 |
| `5af3fdb` | 代理 env + macOS scutil 注入 sidecar |
| `16463f6` | **P0** quality-prompt / answerMode / tool-hint / UI 档位 |
| `131de90` | **P1** Responses + reasoning + agent-state harness + memory 压缩 |
| （本文档提交） | 深度诊断 + 阶段 A/B 路线图；无业务代码 |

### 8.3 相关文档

- `docs/AGENT_QUALITY_UPLIFT_2026-10-17.md`（P0）  
- `docs/AGENT_QUALITY_UPLIFT_P1_2026-10-17.md`（P1）  
- `docs/AGENT_GROK_OAUTH_AND_SPEED_PLAN_2026-10-16.md`  
- `docs/AGENT_DATA_ACCESS_ANALYSIS_2026-10-16.md`  
- `docs/AGENT_PROCESS_ISOLATION_2026-10-17.md`  
- `docs/AGENT_FIRST_ARCHITECTURE_2026-10-16.md`

---

## 9. 给执行者的下一动作建议

**建议立刻开工阶段 A 整包**（不要再拆碎 prompt PR）。  
开工前冻结：runtime 主路径 = `xai-api`；工具真名以 `apps/*/app.package.json` 为准；验收以 §7 剧本 + host 单测为准。  
阶段 B 等阶段 A 真人体感过关后再开，避免数据面扩张掩盖上下文/验收漏洞。
