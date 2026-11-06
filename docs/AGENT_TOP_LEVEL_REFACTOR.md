# Agent-at-top 重构设计（后端）+ UI 规划交接

状态：设计稿（2026-11-06，Opus）。后端由本设计驱动；UI 部分交 Sonnet 5.5 审核与实现（见 §6、§7）。

## 1. 目标

把应用的组织轴从「工作空间为中心」翻成「agent 为中心」：

- **agent 置顶**：可以直接开一个对话任务（不必先建工作空间），也可以挂到现有工作空间上干活。
- **库升为顶层全局资源**：记忆库、技法卡、工具库都不再埋在某个工作空间/配置页里，而是跨工作空间共享的顶层资产。
- **新增工具库**：沉淀 agent 自己写、验证过的可复用脚本，按目标指纹跨工作空间召回复用。

## 2. 现状（为什么不能硬翻）

当前以 `projectId` 为唯一组织键，耦合极深：

- host DB 约 16 张表以 `project_id INTEGER NOT NULL REFERENCES projects(id)` 为键：`conversations`、`application_grants`、`provider_usage`、`host_budget_limits/entries`、`tool_records`、`project_allocations`、`task_approvals`、`summary_versions`、`human_reviews`、`project_memory`、`project_access_policies`、`conversation_workspace_grants`、`conversation_material_grants` 等。
- 约 30 个前后端文件直接引用 `projectId`。工作区目录 `workspaces/<projectId>/agent`、额度、访问模式、能力集、预算全部按 project 切分。
- 前端 `selectedProjectId` 已可为 `undefined`（半支持「未选工作空间」），但**会话强依赖 project（NOT NULL 外键）**。

结论：把 `project_id` 改成 nullable 并在 30 个文件里到处兜底，是高风险大改。**本设计用「默认空间」规避**（§3 D1）。

## 3. 核心设计决策（含推荐）

### D1 — 自由对话 = 一个保留的「默认空间」，不做 nullable project_id（推荐）

保留一个固定工作空间行（如 `projects` 里一条 `kind='personal'`、id 固定的「直接对话 / 个人空间」）。不选工作空间时开的 agent 对话，全部挂到这个默认空间。

- 好处：`workspaces/<id>/agent`、access policy、budget、tool_records、memory 等**全部按 projectId 的现有管线零改动**继续工作。"agent 置顶"成为 IA/UX 翻转 + 一条默认行，而非 schema 重写。
- UI 上「直接对话」就是默认空间；用户随时可以把对话转挂到某个具名工作空间（改 `conversations.project_id`，带 workspace/记忆迁移策略，见 §5）。
- 代价：默认空间的数据（workspace/记忆）是"全局个人"语义，需在 UI 上和具名工作空间区分清楚。

### D2 — 库的全局化方式

| 库 | 现状 | 目标 | 做法 |
|----|------|------|------|
| 记忆 | `project_memory` 按 project | project + **全局**双作用域 | 加一个保留的「全局」scope（默认空间即可充当，或加 `scope` 列）。agent prompt 注入 = 全局记忆 + 当前空间记忆，合并展示。硬不变式不变：记忆永不进任何 auth/scope/settlement 闸。 |
| 技法卡 | `research_cards`（Tauri DB，本就不按 project） | **全局**（不变） | 已经是全局；只需 UI 从「配置中心」升到顶层库区。 |
| 工具库 | 无 | **全局**（新建） | 见 D3。 |

### D3 — 工具库落点与读写路径（推荐：Tauri DB 表 + host 只读注入，沿用 research_cards 那套）

- 存储：Tauri app DB 新表 `agent_tools`（跨 project，全局）。字段：`id, title, language(node|python), code, target_fingerprint, algo_tags(JSON), source_conversation_id, verified(bool), status(pending|approved|rejected), created_at`。
- **写入（agent 沉淀）**：agent 在某空间验证可用后，调一个工具 `sink_agent_tool`（host 工具 → 经 scan/cert bridge 落到 Tauri DB，或 host 直写一个 host 侧镜像；二选一，见开放决策 Q3），脚本正文 + 自评指纹入库，默认 `pending`。
- **写入（操作员策展）**：UI 也能从某空间工作区挑一个脚本"入库"，或编辑/审批。
- **召回（advisory 注入）**：新对话构建 prompt 时，host 读 `status='approved'` 且指纹/算法标签匹配当前目标线索的工具，作为 advisory 块注入（**和记忆、技法同一道墙**：只建议"可复用这个脚本"，给出库内相对标识，绝不自动执行、绝不进任何授权/范围/结算/能力闸）。
- **复用前强制**：agent 必须把候选脚本读一遍、在沙箱重跑验证后才用于新目标（防跨目标误用）。prompt 明确要求。
- 安全：工具库存的是 **agent 自己写、跑过的代码**，不是外来制品——不违反防投毒墙（`poison_strip` 针对外部材料）。但"入库/审批"仍走操作员可见审阅（像技法卡）。

### D4 — 权限/范围模型不变

- 默认空间有自己的 `project_access_policies` 行（mode: ask/auto/full），和任何空间一样。
- 两轴权限不变：picker 管 operator-env 锁；每目标法律授权独立、绝不自动 attest（见 [[permission-two-axis-model]]）。工具库/记忆/技法都是 advisory，不改这两轴。
- 沙箱边界不变：无网、只读授权源目录、写入限工作区、B4 单次高危审批不变。

### D5 — 迁移

- 幂等加默认空间行（`INSERT OR IGNORE`）+（如选 scope 列）给 `project_memory` 加 `scope` 列默认 'project'。
- 不动任何现有 project 的数据；现有会话继续挂原 project。
- 工具库是纯新增表，无迁移。

## 4. 后端分阶段（我来实现）

- **P0 默认空间**：加保留 project（`kind`/固定 id）+ 创建会话时未指定 project 则落默认空间；App 启动确保默认空间存在。host + Tauri 两侧。
- **P1 库全局化**：记忆加全局作用域 + prompt 合并注入；技法卡保持全局（仅 UI 搬家）。
- **P2 工具库**：`agent_tools` 表 + `sink/list/decide/delete_agent_tool` 命令 + 指纹匹配召回 + host advisory 注入 + 复用前重跑验证 prompt。
- **P3 收口**：会话转挂空间（改 project_id + 迁移策略）、额度/预算在默认空间下的默认值、文档与测试。

每阶段独立可验证、独立提交，延续当前 PR #58 分支的节奏。

## 5. 会话转挂空间（P3 细节）

默认空间的对话转到具名工作空间：改 `conversations.project_id`；工作区文件默认留在默认空间（可选择拷贝/移动已产脚本）；对话记忆跟随会话、项目记忆不跨搬。给 UI 一个明确的"挂到工作空间"动作 + 确认。

## 6. UI / IA 规划（交 Sonnet 5.5 审核与实现）

把左栏组织从「模块」翻成「agent 为家 + 库区置顶 + 工作空间可挂」：

- **顶部：Agent（对话）= 主页**。进来就能直接开对话任务（落默认空间），右上角一个「工作空间：无 / <具名>」挂载选择器。
- **库区（顶层，跨空间）**：记忆库、技法卡、工具库 三个入口，从「配置中心」升上来（配置中心只留真正的设置）。复用现有组件 `ProjectMemoryManager.vue`、`ResearchCardsManager.vue`，新增 `AgentToolsManager.vue`（工具库：列表/指纹标签/查看脚本/批准/删除/从工作区入库）。
- **工作空间区**：现有的资产/扫描/采集等仍按具名工作空间组织，作为"挂上去才出现"的上下文。
- 兼容：默认空间要和具名工作空间在视觉上清楚区分（"直接对话"徽标）。

5.5 的交付：左栏/路由重组、库区三面板的顶层化、工具库面板、挂载选择器、默认空间的视觉区分、以及 §6.1 的对话工具活动时间线。**后端契约（命令/字段）由本设计 §3/§7 固定，UI 按契约对接。**

### 6.1 — 对话工具活动：实时时间线排版（用户点名的目标样式）

agent 对话里工具执行的呈现，目标是用户给的参考样式（截图）：

- **可折叠工具组**：多步收起成「执行工具 N 次」，点开看每一步。
- **每步状态行**：类型 + 状态徽标（终端命令/脚本 已运行✓、运行中◌、已思考💭、失败✗）+ 一行命令/动作摘要（截断）。
- **运行中实时态**：整组顶部「正在执行中 · 128s」累计计时；单步「运行中 · 90 秒 · 已输出 40 行」实时刷新。
- **最新输出预览**：运行中的步骤展开有「最新输出预览」+「命令说明」（人话描述这步在干嘛，对应工具的 description/摘要）。
- **思考内嵌**：`已思考` 作为独立步骤插在工具之间，显示该步 thinking 摘要。
- **完成后**：整组收起成「执行工具 N 次」，保留每步可展开的回执（stdout/exitCode/产物链接）。

基础（经 5.5 实测核正）：真正承载对话工具活动实时态的是 `AgentHostChat.vue` 里的 `steps`/`heartbeat`/`inner` + `task.evidence`（`AgentTaskEvidence` 已有 `startedAt`/`finishedAt`/`status`/`toolName`/`summary` 分步、随 `task.updated` 实时推）——**不是** `ConversationToolActivity.vue`（旧非-agent-host 路径的完成后静态列表）或 `AgentTraceHub.vue`（sentinel 扫描轨迹，与对话无关）。5.5 已在此基础上做出 composable `toolTimeline.ts` + `AgentToolTimelineList.vue`（1efb9d4）：折叠工具组、分步状态、整组/单步计时、思考内嵌、完成折叠。缺口（输出预览、思考时序）见 §7，由后端补后前端从近似换精确。

## 7. 前后端契约（UI 依赖的接口，后端我保证）

- 工具库（Tauri 命令，沿用 research_cards 风格，camelCase 字段）：`list_agent_tools(status?)`、`ingest_agent_tool(title, language, code, targetFingerprint?, algoTags?, sourceConversationId?)`（直接收 code → pending；"从工作区入库"由前端先经 host 读到脚本内容再传 code，Rust 不跨读 host 工作区）、`decide_agent_tool(id, approve)`、`delete_agent_tool(id)`。字段：id/title/language(node|python)/code/targetFingerprint/algoTags(JSON 数组)/sourceConversationId/verified/status/createdAt。**不跑 poison_strip**（自写代码，非外来材料）。
- 记忆：现有 host chat client `projectMemory` 扩一个全局作用域读取；或新增 `globalMemory()`。
- 默认空间：host 暴露"确保默认空间存在 / 其 id"的接口，供 UI 开直接对话。
- 会话挂载：`attach_conversation_to_workspace(conversationId, projectId)`。
- 工具活动时间线（§6.1，字段已按 5.5 实测核正）：主数据源 = `task.evidence`(`AgentTaskEvidence`，已有 `startedAt`/`finishedAt`/`status`/`toolName`/`summary`，随 `task.updated` 实时推) + 事件 `runtime.step`(仅 title)/`runtime.heartbeat`(整 run 一份，当前步 elapsedMs+stepTitle)/`runtime.inner`(单份快照)。**后端待补的两处缺口**（P1 顺带）：①运行中工具的单步 `outputLines`/`previewText`(最新输出预览，脱敏截断)——现无；②`runtime.inner` 加单调 `seq` 或快照时间戳，让「已思考」精确插在两次工具调用之间（现 UI 用"内容变化即追加"近似）。UI 只消费，补后换精确。

## 8. 开放决策（请你拍板，其余我按推荐做）

- **Q1**：默认空间用「固定保留 project 行」(推荐，低风险) 还是坚持真·nullable project_id（干净但大改）？
- **Q2**：记忆全局作用域用「默认空间即全局」还是给 `project_memory` 加独立 `scope` 列（更清晰，推荐后者）？
- **Q3**：工具库写入路径——agent 运行时自动"沉淀"入库（需 host→Tauri 写通道），还是只允许操作员从工作区手动入库（更可控，推荐先做这个，自动沉淀作为后续）？
- **Q4**：工具库召回的"目标指纹"用什么线索（算法族标签 + 域名/应用指纹 + 用户点名）？先做标签+用户点名匹配，指纹自动化后续。

## 9. 分工

- 后端（Opus，我）：P0–P3 全部后端 + 契约落定 + 测试。
- UI（Sonnet 5.5，独立会话）：§6/§7 的前端重组与新面板，按契约对接。
- 节奏：延续 PR #58 分支；每阶段独立提交；UI 会话基于定稿的契约启动。

相关记忆：[[agent-host-workspace-file-capability]]、[[agent-cross-session-memory]]、[[permission-two-axis-model]]、[[oviraptor-single-agent-philosophy]]、[[pentest-harness-design-docs]]。
