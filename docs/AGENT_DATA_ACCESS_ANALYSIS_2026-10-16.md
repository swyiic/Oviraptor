# Agent 数据读取能力分析（2026-10-16）

分支：`feature/agent-first-app-platform-20261016`。§1–4 为分析；§5 与 P1/P2 轻量只读工具本轮已实现（见 checkpoint）。

## 1. 现状：agent-host 能调用哪些工具
- **工具链路**：`mcp-bridge.mjs` 只是一个 stdio 到本地 HTTP 的代理（地址 `127.0.0.1`，带 token，限定在当前 conversation）。`service.mjs` 处理 MCP 请求：`tools/list` 返回 `apps.capabilities(conversation.projectId)` 加上 skill 工具；`tools/call` 会重新解析工具，再调用 `apps.call(appId, tool, conversation.projectId, args, grantRevision)`。
- **门禁**（`apps.mjs` 的 `capabilities`/`call`）：应用必须已启用、不在变更中、worker 未停止、owner 没有处于 unresolved 状态；该项目的 `application_grants.permissions` 必须包含工具声明的 permission；`disclosure_runtime` 必须等于当前 runtime；调用前按 `inputSchema` 校验参数；grantRevision 一旦变化，返回 `host_tool_authority_changed`。
- **发现**（`discovery.mjs`）：注入一份不可信的能力摘要（应用 id 加工具名，有字节上限，超限会截断）。prompt 里还带了“Host-selected project”。
- **当前暴露给 agent 的只读数据工具**（manifest 里的 `tools`）：
  - assets：`saved_assets_stats`、`saved_assets_search`。权限都是 `inventory.read`，底层是 `assets::statistics::stats_on/search_on`，只查 active 视图，按 cursor 分页，默认每页 20 条。另外有 101 个 actions，但它们是给页面 UI 用的，不是 MCP tool。
  - scanning：只读的有 `scan_list`、`scan_draft_read`、`scan_status`（权限 `investigation.read`）。其余工具（draft/start/pause/resume/retry/cancel）都是写入或执行。
  - legacy 适配器（`legacy_app_bridge.rs`）：`saved_assets_*` 和 `saved_scans_list`，只读。

## 2. 项目空间和各表的只读接口缺口
| 需求 | 现状 |
|---|---|
| 列出项目 / 切换项目 | **没有**。项目由 conversation 固定（`conversation.projectId`），agent 不能列项目，也不能跨项目 |
| 资产表 | 有，但只能走 search 和 stats 两个固定视图，不能自定义字段或过滤 |
| 暴露面（exposure findings/runs） | **agent 不可用**。只有 Tauri command（`list_exposure_findings/runs`、`read_exposure_finding` 等） |
| 任务 / 日志 / 事件 | scanning 只能看任务状态；`list_runs`、`list_logs`、`list_asset_events` 只是 Tauri command |
| 表 schema 描述 | **没有**，agent 不知道有哪些表和字段 |
| 聚合统计 | 只有资产 stats；`dashboard_stats`、`sentinel_overview_stats`、`get_asset_ownership_summary` 没有暴露 |
| 跨应用读 | 没有。每个工具只属于自己的应用，并受该应用的 grant 约束 |

**src-tauri 里可以复用的只读 command / 函数**：`workspace_projects::list_projects`、`assets::list_assets(_at/_on_read_snapshot)`、`assets::statistics::{stats_on, search_on}`、`application_overview::dashboard_stats`、`list_targets`、`list_runs`、`list_logs`、`list_asset_events`、`get_asset_ownership_summary/profile`、`get_asset_rule_catalog`、`list_exposure_runs`、`list_exposure_findings`（只取摘要字段）、`sentinel_overview_stats`、`investigation_read.rs`。

## 3. 瓶颈判断
主要瓶颈是**没有工具**。底层查询能力很多，但 manifest 只开放了 2 个资产只读工具和 3 个扫描只读工具。次要原因有两个：
1. 发现层只给了工具名，没有 schema 或语义说明（摘要会截断），所以 agent 不知道还能查什么；
2. 门禁本身是合理的（按项目授权，`inventory.read` 和 `investigation.read` 需要用户授予）。如果项目没有授予 read 权限，`tools/list` 会直接为空，看起来就像“读不到”。

所以问题不是门禁挡住了已有能力，而是“能力没注册成 tool”，加上“发现信息太少”。另外，agent 按设计就不能列项目，这一点需要产品层面决定。

## 4. 分阶段方案（只读、遵守归属、不涉及 exposure 扫描实现）
**P0 排查（约 0.5 小时）**：在测试项目上确认 `assets` 和 `scanning` 的 read grant，以及 `disclosure_runtime` 都匹配。用 `tools/list` 确认 5 个只读工具都能出现。测试：现有 agent-host 测试，加一个 mcp `tools/list` 快照。

**P1 发现增强（约 1 小时）**：在 `discovery.mjs` 的摘要里给每个工具附上一句 description 和必填参数名（继续保持在字节上限内）；`tools/list` 本来就返回 inputSchema，确认各工具的 description 写得清楚。测试：给摘要字节上限和截断顺序加单元测试。

**P2 assets 只读工具（约 2–3 小时）**，在 `service_bootstrap.rs` 里注册为 `inventory.read` / `project` 范围，并在 `service.rs` 中分发：
- `inventory_schema`：返回资产相关表的白名单列和类型（静态描述，不直接查 sqlite_master）；
- `inventory_query`：基于现有的 `search_on` 和 filter 实现分页（limit 不超过 100，用 cursor），只允许查询白名单字段；
- `inventory_aggregate`：用 `stats_on` 加上 `get_asset_ownership_summary`，支持按 status、source、owner 等**白名单维度**做 group by；
- `project_overview`：调用 `dashboard_stats(project)`，并且只针对当前项目。
测试：在 `service_tests.rs` 测越权项目返回 error、deleted 视图被拒、limit 被截断；在 `legacy_app_bridge` 里加同构测试。

**P3 scanning 只读（约 1–2 小时）**：增加 `scan_journal_read`（task_journal 分页）和 `scan_runs_list`（复用 `list_runs`，只读 `investigation.read`）。日志内容需要脱敏或截断。

**P4 暴露面摘要（可选，约 2 小时）**：新增权限 `inventory.exposure.read`。只返回 `list_exposure_runs` 和 findings 的**计数、严重度、状态**等聚合结果，不返回 payload、请求内容、探测规则、凭据、源结果原文，也不暴露 run/start/cancel。

**P5 项目列表（产品决定）**：增加 host 级工具 `host__projects_list`，只返回当前用户有 grant 的项目的 id、名称和统计数字。默认关闭，跨项目查询仍然要求按项目授权。

## 安全边界
- 只注册读工具，不做通用 SQL；表、列、group by 维度都用白名单；所有查询都绑定 `conversation.projectId`（或 P5 里已授权的项目）。
- 复用现有的 grant、revision、owner、disclosure 门禁，不绕过 `apps.call`。
- 结果大小必须在 `MAX_RESPONSE`（128KB）以内，分页时强制 limit。
- 不返回 deleted 内容和 exposure 实现细节（探测 payload、规则、凭据、原始响应）。
- 新增的权限 id 必须写进 manifest，并通过 grants UI 由用户显式授予。

## 5. agent↔应用关联约束与耦合度

### 5.1 现状关联摘要

| 机制 | 位置 | 作用 |
|---|---|---|
| **bundled-apps** | `packages/agent-host/bundled-apps.json` + `bundled-apps.mjs` | 预装 Asset/Scanning；`builtin=true`、`uninstallable=false`；启动时 `ensureBundledApps` 安装并启用 |
| **shell-apps** | `shell-apps.json` + `shell-apps.mjs` | 壳层导航/视图 catalog；`hostAppId` 把壳模块绑到 host 应用；无 host 的条目（如 HackerOne）仅壳侧 |
| **manifest permissions/tools** | 各 `app.package.json` | 工具声明 `permission`；host 只承认 manifest 里的工具与权限 id |
| **application_grants** | host SQLite `application_grants` | 按 `(app_id, project_id)` 存 permissions[]、revision、`disclosure_runtime` |
| **paired_skills** | manifest `paired_skills` + `paired-skills.mjs` | 应用声明配套 Skill；安装预览/导入；发现摘要里给 before-call 提示；**不授予执行权** |
| **discovery 摘要** | `discovery.mjs` `buildDiscoverySummary` | 注入 prompt 的不可信能力摘要（应用 id、工具名、purpose、必填参数）；有字节上限，超限按「末应用末工具 → 应用 → Skills」有序截断；`authority: none` |
| **disclosure_runtime** | grant 列 + `capabilities(..., { runtimeIdentity })` | grant 的 `disclosureRuntime` 必须等于当前 `runtimeIdentity`，否则 capabilities 为空（工具对 agent 不可见） |
| **grantRevision** | grant.revision / tool.grantRevision | `tools/call` 带 expected revision；调用中/后 `#current` 校验；变更抛 `host_tool_authority_changed`（不可重试），客户端须刷新 `tools/list` |

调用主路径：`mcp-bridge` → `AgentHost.mcp` → `apps.capabilities` / `apps.call` → worker。**没有任何应用间直调通道**；跨应用只能各自走 host 的 `apps.call(appId, tool, projectId, …)`。

### 5.2 耦合分级

**硬耦合（故意保留）**
- conversation ↔ `projectId`：agent 不能列/切项目；工具一律绑定当前对话项目。
- grant ↔ tool.permission：未授予则 tools/list 不含、直接 call → 403。
- builtin / uninstallable：bundled Asset/Scanning 不可卸载（`host_app_builtin`）。
- worker 进程与 package digest：篡改安装包在启动前拒绝。

**契约耦合（应通过 schema/权限，不应绑实现）**
- manifest `tools[].inputSchema` + `permission`：agent 只应依赖契约，不依赖应用内部 SQLite/UI。
- discovery 摘要与 `tools/list` description：语义发现，无权威。
- `paired_skills`：名义关联，执行仍走 Skill 独立授权。
- shell catalog ↔ hostAppId：导航耦合；业务调用仍经 host。

**应解耦但仍绑着的点（已知债）**
- Node 应用里的 `legacy_*` 工具依赖可选 `legacyBridge`（Trusted composition），缺省返回 unavailable——agent 可能误以为「读不到桌面数据」。
- native schemaVersion=2 的 actions（页面 UI）与 MCP tools 双轨；agent 看不到 actions，易与「完整资产能力」混淆。
- shell 模块与 host 应用同 id 叙事，但 Phase 4 进程隔离（Nest/Asset 不链进 Host）未做。
- 发现摘要字节上限导致大工具集时尾部工具被截断——依赖 `tools/list` 全量 schema 补齐。

### 5.3 目标边界

1. **agent 只依赖应用契约**：工具 schema + 声明权限 + discovery/`tools/list`；不依赖应用内部表名、UI action、Rust 内核路径。
2. **应用间不互调**：只经 host `apps.call` / MCP 限定名 `appId__tool`；禁止 A worker 直连 B 内部 API。
3. **未授权工具不可见**：`capabilities` 过滤；`tools/list` 与 discovery 一致。
4. **跨应用直调拒绝**：错误 appId、未安装、未启用、无 permission → `host_tool_permission_denied`（403，retryable=false）。
5. **authority 变更刷新**：grant/enable/disable/runtime disclosure 变化 → `host_tool_authority_changed` 或 list 变空；客户端必须重新 `tools/list`，不得沿用旧 grantRevision 重试业务。

### 5.4 连夜可落地的可测规则清单

| # | 规则 | 测法要点 |
|---|---|---|
| R1 | 未授予 permission → `tools/list` 不含该工具 | 只 grant `inventory.read`，写工具不出现 |
| R2 | 未授予时直接 `tools/call` → 403 / `host_tool_permission_denied`，不重试 | mcp-bridge / AgentHost.mcp 断言 `retryable=false`、调用次数=1 |
| R3 | grantRevision 变 → `host_tool_authority_changed`，客户端须刷新 | 调用中/后改 grant；结果 withheld |
| R4 | 应用未启用 → list 空；call 拒绝 | `setEnabled(false)` |
| R5 | uninstallable builtin 不可卸 | `uninstall(assets\|scanning)` → `host_app_builtin` |
| R6 | discovery 只暴露已装且已启用且已授权应用 | 摘要 JSON 不含 disabled / 未授权 appId |
| R7 | 应用 A 不能 call 应用 B 内部 API | `apps.call(A, B的工具名)` 因 manifest 无该工具而拒绝；MCP 名落到 B 时无 B grant 则 403 |
| R8 | disclosure_runtime 不匹配 → capabilities 空 | grant 写入旧 runtimeIdentity 后 list 空 |
| R9 | discovery 摘要 ≤ 字节上限；超限有序截断 | 单测塞满工具后仍 ≤ `MAX_SUMMARY_BYTES`，末工具先丢 |
| R10 | 只读数据工具结果压在既有 MAX（app `MAX_TOOL_RESULT_BYTES` / 业务自限）内 | `inventory_query` limit≤100 且超大时裁剪 items |

**实现状态（本轮）**：P1 发现增强、§5.4 耦合规则测试、assets 只读工具（`inventory_schema` / `inventory_query` / `project_overview`）、scanning 只读发现文案 — **已实现**（见 checkpoint）。
