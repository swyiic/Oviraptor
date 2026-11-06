# 多 URL 子任务 + 每扫描独立 Agent 对话 —— 设计方案

> 状态：设计稿（未写任何实现代码）。目标是先确认方向，再分期落地。
> 结论先行：**你要的"多 URL 拆子任务"底层已经做好了**，主要缺的是"父任务 + 子任务"的**展示层**；**"每个扫描独立对话 + 下拉切换 + 总览会话"**需要新做，但能复用现成的关联表和切换范式。

---

## 0. 关键发现（当前真实现状）

### 0.1 多 URL 已经是"一个扫描 = 多个 per-URL 目标"

代码库**现在就是方案 B**（one scan, many targets），而且端到端打通：

- 新建扫描表单 `AgentWorkbench.vue` 的 Web 模式已经是**多行 URL 文本框**（`form.urls`，"每行一个"），`useWorkbenchTaskCreation.ts` 会按换行/逗号/分号/空白切分成数组，提示里就有"共 N 个 URL"。
- 后端 `create_sentinel_url_scan`（`scan_lifecycle.rs`）在一个事务里写 **1 行 `sentinel_scans`** + **N 行 `sentinel_targets`**（每个 URL 一行，`UNIQUE(project_id, scan_id, url)`，上限 **200 个 URL/扫描**）。`sentinel_scans` 上**没有 url 列**，URL 全在子表 `sentinel_targets`。
- 发现/机会/验证/调查图谱**全部按 `(scan_id, target_url)` 键**：`sentinel_findings`、`sentinel_opportunities`、`sentinel_validations`、`investigation_nodes/edges/...`。
- 原生多 agent 流水线**每个 URL 一个 coordinator/root `agent_run`**（`web_mode/root.rs` 强校验：root 的单个 `target_url` 必须属于本扫描的目标集），worker 再按 host/路径前缀切片。
- `SentinelBoard.vue` **已经在按 URL 出卡片**：`urlCards`、`findingsByUrl`、`targetUrls`、`targetCountByScan`。概览聚合也算好了：`sentinel_overview_stats` 的 `url_count`、漏洞计数；token/请求总额直接是 `sentinel_scans` 行上已经汇总好的列（`scan.totalTokens`、`scan.llmRequests`）。

**含义**：你说"当前各种展示数据都是展示一个 url 的"，其实底层数据和执行都是 per-URL 的，缺的是把它**明确组织成"父任务（总量/总漏洞）→ 点进去按 URL 看"**的展示。这块大头是 UI，不是数据模型重做。

### 0.2 对话其实有"两套系统"，不要混为一谈

| | System A：执行概览对话 | System B：人机聊天 |
|---|---|---|
| 组件 | `AgentDialog.vue` / `AgentWorkbench.vue` | `ConversationWorkspace.vue` / `ConversationPluginPanel.vue` |
| 本质 | 原生流水线（coordinator↔worker 协作、人工指令、评审门）的**只读时间线投影** | 真正的 **user↔model 聊天** |
| 存储 | `agent_collaboration_events`，键 `(scan_id, attempt_number, sequence)` | `conversation_messages`，键 `conversation_id` |
| 是否绑扫描 | **是**，天然 per-scan+attempt | **否**，按项目；只能通过 `conversation_links(kind='scan')` 做跳转引用 |
| 切换 UI | 已有"切换任务"下拉 + 线程 select | 左侧最近列表 + ⌘K 查找器，无紧凑下拉 |
| 人给 agent 发话 | **受控指令流程**（draft→confirm），非自由聊天 | 自由聊天（Enter 发送 + Steer 队列） |

- 新建扫描**目前不会**自动拉起聊天；唯一的"从扫描开聊"是手动"交给 Agent 分析"，而且**每次都新建一个会话、不复用**。
- 没有"总览会话"这种东西；最接近的概念是 `agent_dialog_selections` 里 `scope_key='all'` 这一行（"全部任务"范围）。

**含义**：你说的"像 Claude Code 那样、每个扫描一个对话、下拉切换、总会话看关键信息"——对话指的是 **System B 的真聊天**；需要把它**绑定到扫描**（复用 `conversation_links`）、**自动拉起**、加**切换器**和**总览**。System A（执行概览）保留为"这次扫描 agent 到底怎么跑的"时间线，两者并存、各司其职。

---

## 第一部分：多 URL → 父任务 + 子任务

### 1.1 方向选择：A vs B

- **方案 B（推荐，顺着现有架构）**：保持"一个 `sentinel_scans` = 多个 per-URL 目标"。把每个 URL 当作该扫描的一个**子任务视图**展示：父任务显示"URL 总数 / 总漏洞 / 总 token"，点进去按 URL 看各自的发现、调查图、执行轨迹。**数据和执行都现成**，主要是展示层 + 可选的 per-URL 生命周期。
- **方案 A（不推荐）**：每个 URL 建一个**独立子 `sentinel_scans`**，父扫描做跨子扫描聚合。现状几乎没有支撑：唯一的 `previous_scan_id` 是休眠且刻意松散的（删除时子任务被当作"独立任务"只解链），要新造父/子扫描类型、跨行聚合、并和现有 per-target 执行模型打架。成本高、逆着架构走。

> **推荐 B。** 下面按 B 设计。A 作为备选，只有在你明确需要"每个 URL 完全独立的扫描实体（独立预算账本、独立归档、独立删除）"时才值得。

### 1.2 "子任务"= per-URL 子视图（B 的落法）

概念映射（全部已有数据支撑）：

| 你的说法 | 落到现有模型 | 数据来源（已存在） |
|---|---|---|
| 父任务 | 1 行 `sentinel_scans` | `SentinelTaskCenter.vue` 已显示 |
| 子任务 | 1 行 `sentinel_targets`（per URL） | `listSentinelTargets`、`scanTargets` |
| 父显示"总数量" | URL 数 | `targetCountByScan` / `url_count` |
| 父显示"总漏洞" | 跨 URL 漏洞合计 | `listSentinelVulnerabilityCounts`、`sentinel_overview_stats` |
| 点进去按 URL 看 | per-URL 卡片 + 下钻 | `urlCards`、`findingsByUrl`、`targetUrls` |

### 1.3 要做的工作（B）

**展示层（主要）**

1. **父任务卡片**（`SentinelTaskCenter.vue`）明确展示聚合三件套：`目标 N 个 URL`、`漏洞合计 M`、`Token 合计`。现在"目标 N"已有，补"漏洞合计"和让三者作为父级摘要更醒目。
2. **父 → 子下钻**（`SentinelBoard.vue` 结果页）：把现有 `urlCards` 正式组织成"子任务列表"——每行一个 URL，显示该 URL 的状态、发现数、漏洞数、关键指纹；点一行进入该 URL 的详情（发现、调查图、per-URL 执行轨迹）。`urlCards`/`findingsByUrl` 机器已在，主要是信息架构和路由。
3. **per-URL 状态**：`sentinel_targets.status`（queued/…）已有；在子任务行显示每个 URL 的执行状态（排队/进行中/完成/失败），让"父=总，子=各自"一眼可见。

**可选增强（按需，决定范围）**

4. **per-URL 生命周期**（重要范围决策）：是否要能**单独**对某个 URL 重试/暂停/删除？当前是"整扫描"粒度（重试=加 attempt，删除=删整扫描）。若要 per-URL 控制，需要：per-target 的重试入口、per-target 预算结算、以及和 `native_scan_branches`（source/web 是按 attempt 不是按 URL）协调。**建议先只做只读的 per-URL 展示，生命周期后置**，确认展示满足需求再决定。
5. **创建表单**（`AgentWorkbench.vue`）：已支持多 URL，仅需在提交后给出"将生成 N 个子任务"的明确反馈（现在提示"共 N 个 URL"，可强化成子任务语义）。

### 1.4 B 要尊重的约束（避免踩雷）

- **200 URL/扫描上限**（`scan_lifecycle.rs`），超出报错——父任务要显示这个上限并优雅提示。
- **去重/归一**：`UNIQUE(project_id,scan_id,url)` 和 per-target `UNIQUE(scan_id,target_url,…)`，创建时就 dedup。
- **熔断区过滤**：URL 可能在创建/启动时被 `sentinel_fuse_zone` 丢掉（甚至全丢→"没有可扫描目标"），子任务列表要能表达"某 URL 被熔断跳过"。
- **预算是按 URL 扇出的**：每个 root run 有独立 token/请求预算，**总预算≈URL 数 × 单 URL 预算**。父任务要让用户明白多 URL = 多花钱（创建时按 URL 数预估）。

---

## 第二部分：每个扫描独立 Agent 对话 + 切换 + 总览

### 2.1 用哪套系统

- **保留 System A** 作为每个扫描的"执行概览"时间线（agent 之间怎么协作、人工指令、评审门）——它已经 per-scan、已有切换下拉。
- **用 System B 做"每个扫描的分析聊天"**（你说的"像你这样对话"）：把一个 `conversations` 会话**绑定到扫描**。桥梁 `conversation_links(kind='scan', target_id=scan_id)` 已存在。

### 2.2 要做的工作（System B）

1. **扫描 ↔ 会话 绑定 + 自动拉起**
   - 新建/启动扫描时自动创建一个会话并 `conversation_links(kind='scan')` 绑定到该 scan（触发点：`useWorkbenchTaskCreation` 成功后，或 `SentinelBoard.openScan`）。
   - 反向查找：新增"按 scan 查其会话"的查询（`conversations/store.rs` + `mod.rs` 命令），`App.vue analyzeProjectScan` / `scanAgentHandoff` 改为**复用**已绑定会话而不是每次 `createConversation`。
2. **下拉切换器（Claude-Code 风格）**
   - 在聊天区顶部加一个紧凑下拉：列出"每个扫描任务一个会话"（读 `list_conversations`，用 `conversation_links` 过滤/标注所属扫描），当前项高亮，点击切换。
   - 复用 System A 的持久化范式（`agent_dialog_selections` 的 scope `all`/`project:<id>` + CAS 乐观并发）做"记住当前选中的会话"；或给 System B 加一张同形小表 `conversation_selections`。
   - 可直接复用 `ConversationFinder.vue`（⌘K 查找器，键盘导航）作为"大列表"的补充。
3. **总览会话（看关键信息）**
   - 新增一个只读"总览"伪会话项，排在切换器最前面。它聚合所有扫描会话的关键信息：每个扫描一行——状态、URL 数、漏洞合计、Token、最近一条消息/最新 `agent_collaboration_events` 摘要。点一行跳到对应扫描的会话。
   - 实现为跨 `sentinel_scans` + 各扫描最新摘要的只读聚合视图（像 `WorkbenchOverview.vue` 那样挂载），作为切换器里的一个特殊条目。

### 2.3 与多 URL 子任务的关系（两部分的接口）

一个关键设计问题：**对话是绑在"父扫描"还是"每个 URL 子任务"？**

- **推荐：对话绑在父扫描**（`conversation_links` 本来就是 scan 粒度）。一个扫描一个分析会话，用户在会话里可以让 agent 聚焦某个 URL（因为底层发现都带 `target_url`，agent 可按 URL 回答）。
- 子任务（URL）层面**不单独开聊天**，而是：在 per-URL 子视图里提供"就这个 URL 问 agent"的快捷入口，底层仍写进**父扫描的那一个会话**（带上 `target_url` 上下文）。这样会话数不随 URL 爆炸，总览也清爽。
- 若将来确实要 per-URL 独立会话，`conversation_links` 需要扩展出 URL 维度（新增一列或新 kind），属于后置增强。

### 2.4 要尊重的约束

- System B 的 agent 模式是**只读工具**（`conversations/agent.rs`），且"链接只是本地引用，不等于导入证据或执行授权"——per-scan 绑定不能破坏这个边界：绑定=方便定位，不自动给模型喂记录，除非用户按现有 `read_scans` 授权。
- `conversation_replies` 有"一个会话同时只能有一个进行中回复"的约束，切换器/总览要正确反映"某会话正在回复中"。
- 别和 System A 的"受控指令流程"混淆：对 System A（执行概览）里的"发指令给正在跑的 agent"仍走 draft→confirm 门；System B 是分析聊天，两者 UI 上要清楚区分。

---

## 需要你拍板的决策点

1. **多 URL：确认走 B（顺架构、主要是展示）**，还是坚持 A（每 URL 独立子扫描，成本高）？（推荐 B）
2. **per-URL 生命周期**：第一期只做**只读的 per-URL 子视图**，还是一步到位要"单独重试/删除某个 URL"？（推荐先只读）
3. **对话绑定粒度**：绑**父扫描**（推荐，一个扫描一个会话，URL 作为会话内上下文），还是每个 URL 一个独立会话？
4. **总览会话形态**：只读聚合列表（推荐），还是也要能在总览里直接发话？

---

## 粗略分期（确认方向后再细化）

- **第 1 期（多 URL 展示）**：父任务聚合卡片（URL 数/总漏洞/总 token）+ per-URL 子任务列表 + 下钻详情。纯前端为主，复用 `urlCards`/`findingsByUrl`/`targetCountByScan`/`sentinel_overview_stats`。
- **第 2 期（每扫描对话）**：扫描→会话绑定 + 自动拉起 + 复用式 handoff；切换下拉 + 选中持久化。后端加"按 scan 查会话"和绑定命令。
- **第 3 期（总览会话）**：跨扫描只读聚合总览，作为切换器首项。
- **可选（后置）**：per-URL 生命周期控制；per-URL 独立会话。

---

## 锚点文件（落地时）

- 多 URL：`db_schema.rs`（schema）、`scan_lifecycle.rs`（创建）、`scan_startup.rs`（启动/计划）、`web_mode/root.rs` + `web_mode.rs`（per-target coordinator）、`multi_agent/findings.rs`（发现按 target_url 键）、`appsec_validation.rs`（概览聚合）、`AgentWorkbench.vue` + `useWorkbenchTaskCreation.ts`（表单）、`SentinelBoard.vue` + `SentinelTaskCenter.vue`（per-URL 卡片 + 父聚合）、`api.ts` + `types.ts`。
- 对话：`conversations/schema.sql` + `conversations/mod.rs`/`store.rs`（per-scan 绑定 + 总览查询）、`useConversationWorkspace.ts` + `ConversationWorkspace.vue`（切换 + 自动拉起）、`scanAgentHandoff.ts` + `App.vue analyzeProjectScan`（复用而非新建）、`agent_dialog_selections` 范式（选中持久化）、`collaboration_events.rs` + `get_native_scan_status`（总览里引用执行摘要）。
