# 资产搜集 · Agent 全接管设计（Asset-Collection Agent Takeover）

目标：让内置 Agent 通过**对话**全面接管工作空间的资产搜集与分析——读取/分析已搜资产、采证书与指纹、批量校验种子、（在 operator 设定的信封内）后台跑 Hunter、二次存活复测并回灌入库、异步派任务并验收——同时**不破坏授权墙、不让 agent 凭空花钱或扩范围**。

本文档既是现状盘点，也是落地 spec。所有 file:line 为撰写时锚点（基线：当前主线，含 agent-host 权限重构 + cited-roots 修复）。

---

## 0. 安全不变量（任何实现都必须守）
- Agent 永不自批**法律授权勾选**、永不自行**扩大已确认范围**、永不**凭空花钱**（Hunter 积分）。这三样是真墙。
- 放开的只是 scope 内的**执行摩擦**：读、免费发现/富化、证书指纹、二次存活复测、以及"operator 预设信封内"的 Hunter。
- 记忆/技法卡/jev/研究卡仍是**建议性**，永不喂任何 auth/scope/settlement/capability 闸。
- 所有改动默认行为不变（新工具按权限+manifest 显露；无授权=不显露）。

---

## 1. 现状：资产全流程（7 阶段）与落点
| 阶段 | Rust 命令 / 入口 | 表 | UI |
|---|---|---|---|
| 建空间 | `save_project` `commands/workspace_projects.rs:183` | `projects` | `ProjectDialog.vue` |
| 加资产+采集扫描 | `import_targets` `commands/assets.rs:64`；`start_job(pipeline=collect/full)` `jobs.rs:1659`（写 `assets` `:657`、`project_assets` `:733`） | `targets`/`assets`/`project_assets` | `QueryPanel.vue` |
| 增量 | `start_job` 的 `incremental` 旗（已完成种子跳过，无专门 diff）`jobs.rs`；种子 `discovery_seed_page.rs`/`discovery_pipeline.rs` | `asset_discovery_pipeline_queries` | `QueryPanel.vue:85` |
| 查询展示 | `list_assets` `commands/assets.rs:582` | `assets`+`project_assets` | `AssetWorkspace.vue` |
| 二次存活复测 | `reprobe_members_on` `commands/assets.rs:602` + `execute_reprobe_job` `jobs.rs:1065`，回写 `jobs.rs:1148` | `assets`(`last_alive`,`probe_outcome`) | `AssetWorkspace.vue:780` 按钮 → `AssetsPluginWorkspace.vue:104` |
| 证书+指纹 | `certificate_task_action` `assets/certificate_tasks.rs:559`；证书 SHA-256 **自动落种子** `:474` | `asset_certificate_tasks`/`_items`、`asset_discovery_seeds` | `CertificateTaskPanel.vue` |
| Hunter | `start_hunter_expansion` `assets/hunter_expansion.rs:942`；门 `allow_hunter` in `asset_discovery_pipelines` `discovery_pipeline.rs:6`；花费 `hunter_expansions*` | | `DiscoveryWorkbench.vue`/`HunterAccountSettings.vue` |

## 2. 现状：Agent 工具覆盖（apps.mjs 分发；manifest = app.package.json）
Agent 的有效授权来自 operator 的项目访问策略：**`full` 模式（operator 确认）快照所有已启用 app 的权限给 agent**（`access-policy.mjs:61-66,101-110`）；`auto`/`ask` 给 agent 零 app 权限。所以下面"可发起"均以 full 模式为前提。

**今天就能 agent 自驱（full 下）：**
- 读资产：`inventory_query/search/stats/schema`、`project_overview`（`inventory.read`）
- 免费发现/富化/清洗：`discovery_assist_start`（schema 仅 `{organization,includeIsolated}`，不含 Hunter）、`discovery_enrichment_start`、`discovery_cleanup_start`（批量校验/排除种子，可撤销）；`discovery_scope_propose`（展示，但**不能确认范围**）
- 证书+指纹：`certificate_task_start/status/pause/resume` + `scan_fingerprint_search/read`
- 暴露面：`exposure_task_start/resume/cancel` + 只读
- 扫描：`scan_from_assets`（建草稿）、`scan_start`（需 `investigation.execute`）、`scan_status/results/...`
- 验收：读 `*_status`/`journal_read`

**今天 agent 做不到（缺口）：**
- **Hunter**：无 agent 工具，硬门在 `discovery-assist.mjs:131` + `apps.mjs:284-290`；`allowHunter/collectFirst` 只在 operator 的 `human_discovery_start`。
- **二次存活复测（reprobe）**：无任何 agent 工具（reprobe 仅 UI 按钮触发）。
- **建空间 / 增量采集触发**：无 agent 工具。
- **范围最终确认**：`human_scope_confirm` operator-UI-only（**故意保留**：范围确认=授权决定）。

## 3. DB 并发 / 锁表模型（结论）
`src-tauri/src/db.rs`：**WAL（`:551`）+ 每操作独立连接 + 无全局 `Mutex<Connection>` + `busy_timeout=30s`（`:50`）+ `with_write_retry` 5 次（`:112`）**。
- **读永不被写阻塞**（WAL reader 走快照，独立连接）——agent 派后台任务、用户继续查询/对话/只读操作：**完全安全**。
- **写之间串行**：抢单一 WAL 写锁，等+重试，不损坏。
- **长事务坑**：collect/reprobe 入库是**整份 CSV 一个 `BEGIN IMMEDIATE`→commit**（`jobs.rs:413→881`），持锁期间别的写排队（满 30s 再重试，极端 >1GB 可能冒 transient "database is locked"）；读不受影响。
- **同项目一次一个任务**：`require_idle_project`（`jobs.rs:1715`）拒"项目已有运行中任务"；**跨项目可并发**抢写锁。Hunter/discovery 是小事务分页写，交错友好。

**编排铁律（写进 agent 提示 + 工具实现）**：① 开任何写任务前先读一次 status（同项目有活就不开）；② 同项目串行写；③ "派任务后继续对话"里的其他操作优先只读；④ 依赖 `with_write_retry` 兜底并发写，不把 transient lock 当失败。

## 4. 要建的（全部完善清单）

### 4.1 reprobe agent 工具（最稳、优先）
- 新 agent 工具 `reprobe_assets`（ASSETS app，权限 `inventory.write` 或新 `inventory.reprobe`）：参数 `{ scope: {kind:'query', query:AssetQuery} | {kind:'selection', assetIds:[]} , expectedCount? }`。
- 后端复用 `reprobe_members_on`（`commands/assets.rs:602`）解析范围 → `start_job(pipeline='reprobe', reprobeScope)`（`jobs.rs:1801`）。**只对已授权现有资产重跑存活+回灌，不扩范围、不花钱。**
- Node 侧：scanning/assets 桥新增 `reprobe` 动作；`apps.mjs` 分发；manifest 加工具 schema。
- 守锁律：开前读 `list_runs`/status，同项目有活则拒并提示 agent 稍后；返回 run_id，agent 读 status 验收"正在回写最新存活状态"→完成。
- 测试：范围解析（query/selection）、同项目占用拒绝、回灌后 `last_alive` 更新、不扩范围。

### 4.2 Hunter 信封工具（operator 设信封，agent 信封内自驱）
- **Operator 一次性**：开 `allow_hunter=1` + 设**积分上限/本次预算**（存 `asset_discovery_pipelines` 或新列 `agent_hunter_budget`）。这是花钱/扩范围授权，保留给人。
- 新 agent 工具 `hunter_expansion_start`（ASSETS app，新权限 `inventory.hunter.run`，**仅 full 模式快照且 operator 已开 allow_hunter 时才授予**）：参数 `{ seedScope?, maxPoints<=budget }`。
- 后端门（必须全部满足才放行，否则 `host_tool_permission_denied`）：`allow_hunter=1` ∧ 凭据已存且 account-hash 匹配（`hunter_expansion.rs:251,258`）∧ `maxPoints<=operator 预算剩余` ∧ 项目 idle。复用 `start_hunter_expansion`（`:942`）。
- agent 验收：读 `discovery_seed_detail` / pipeline `hunter.{spentPoints,pages,added}`（`discovery_pipeline.rs:161-168`）；超预算后端自动停。
- **不放开**：`human_scope_confirm`（范围确认仍人工）；agent 不能改 `allow_hunter`、不能改预算。
- 测试：无 allow_hunter→拒；超预算→拒/截断；account-hash 不符→拒；正常→后台起、花费在账本、状态可验收。

### 4.3 建空间 + 增量触发（小补）
- `create_workspace` agent 工具（包 `save_project`，权限 `inventory.write`；去重按小写名，同 `:211`）。
- 让 `scan_from_assets`/发现流带 `incremental` 参数透传到 `start_job`。

### 4.4 异步编排 + 提示
- 工具已是"提交即返回 + 后台跑"模型（`start_job` spawn 线程 `jobs.rs:1588`）。补 agent 提示：派任务→读一次 status→继续对话；回头用 status/journal 验收；守第 3 节锁律。
- "结果回灌 DB"由各管线自身完成（collection/reprobe 入库、Hunter 分页、discovery `ingest_observations`）；agent 负责**触发 + 验收 + record_finding 汇总**，不手搓写库。

## 5. 示例链如何落（判断归属→证书→种子→Hunter→验收）
1. "公司是否属于移动集团" → `inventory_query`+`project_overview` 读 + 推理（+研究卡佐证）。
2. "分析证书、确认后补种子" → `certificate_task_start` 采证书，SHA-256 自动落 `asset_discovery_seeds`；人工隔离复核的证书走 agent 提议、operator 确认。
3. "批量校验种子、排除不对的、留对的、展示" → `discovery_cleanup_start`（免费可撤销）+ `discovery_scope_propose` 面板展示。
4. "用 Hunter 后台再搜、看日志" → **4.2 信封工具** `hunter_expansion_start`（operator 已开 allow_hunter+预算）；agent 后台起、读账本/日志验收。
5. "验收是否顺利" → 读各 `*_status`/journal；超预算/失败如实报。
6. "派任务后继续对话" → 全后台异步，读永不锁；守同项目串行。

## 6. 实现顺序与验收
1. reprobe 工具（4.1）→ 测试 → 合。
2. Hunter 信封（4.2）→ 测试（重点测闸）→ 合。
3. create_workspace+incremental（4.3）+ 提示（4.4）。
硬要求：真实现+真测试，诚实报告每项"已实现+测过/仅设计/未做"，不写桩；不破坏现有 agent-host 测试；不碰第 0 节不变量。

## 7. 落地结论（2026-11-07，诚实记录）
- **4.1 reprobe_assets — 已实现 + 已测。** Rust 桥 `reprobe` 动作(项目绑定服务端强制)、
  Node 翻译/透传、清单工具(权限 inventory.write)。测试：Rust reprobe_tests ×4 + Node
  reprobe-tool ×5。详见提交 16856e6。
- **4.2 Hunter 信封 — 已实现 + 已测(门穷尽)；真实付费整链未现场验证。** 新模块
  `agent_hunter`(信封表 + operator-only Tauri 命令 + 纯 hunter_gate + 账户哈希绑定)、
  桥 `hunter_start`、capped 后台起、Node 预门、清单工具(权限 inventory.hunter.run)。
  测试：Rust agent_hunter ×10(门 7 + 信封回灌/账户绑定/预算边界 3)+ Node hunter-tool ×3。
  **诚实边界**：对 api.hunter 的真实付费分页+记账+超额自停端到端运行需 operator 付费
  账户与额度，本会话不真花钱跑；capped 起复用既有经测 run_existing + 预算 stopped 闭包，
  门与上限有单测但整链付费未执行。详见提交 62a04da。
- **4.4 异步编排 + 提示 — 已实现。** quality-prompt.mjs 新增第 29–31 条：提交即返回/只读
  一次 status 验收/不轮询；同项目一次一个写任务、读不被写阻塞、transient lock 自动重试
  不当失败；reprobe_assets 与 hunter_expansion_start 的用法与边界。
- **4.3 create_workspace + incremental 透传 — 仅设计，未实现（诚实说明原因）。**
  - *create_workspace*：Agent 桥(certificate-tasks/discovery-assist)按设计是**项目绑定**的——
    每次调用都携带当前会话映射的桌面 projectId，Rust 校验 `result.projectId===desktopId`
    (certificate-tasks.mjs:42)。而 create_workspace 要**新建**一个与当前会话无绑定的桌面项目，
    其返回 id 必然不等于当前 desktopId，直接违反这条绑定不变量；要落地需要另开一条"无项目绑定"
    的桥路径，触碰项目绑定安全墙。对一个价值有限的"小补"不值当，宁可仅设计不强塞半成品。
    host↔desktop 的项目绑定(externalId)本就是 operator 建空间/开会话时建立的，Agent 既不能建
    host 项目也不能设 externalId，故 Agent 新建的桌面项目在本会话内也无法被 Agent 使用。
  - *incremental 透传*：Agent 当前的资产交接面没有 incremental 语义——`scan_from_assets` 产出
    的是扫描草稿(非 start_job)，reprobe 走 start_job 但 incremental 对复测无意义(已固定 None)；
    真正带 incremental 的采集(collect/full)触发走 operator 的 human_discovery_start(collectFirst
    为 operator-only)。给 Agent 加 incremental 需新开一个 Agent 采集触发工具，与"付费首采/采集
    为 operator 决定"的边界重叠，故仅设计、未实现。
