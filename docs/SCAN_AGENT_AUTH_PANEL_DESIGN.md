# 对话式扫描能力授权面板 + operator 长期授权（Scan Agent Auth Panel）

状态：设计 + 分阶段落地。本文件是 spec；实现进度见文末「落地进度」。

## 1. 背景与问题

Oviraptor 已把「扫描建任务」的 Workbench UI 入口移除，改为全部通过与内置 Agent
对话驱动。现状：

- Agent 用 scanning app 工具 `draft_create(title, targets)`（权限
  `investigation.draft.write`，full 模式已授予）在对话里建扫描 draft。
- 深度测试/利用工具按「已授权的能力类」条件下发
  （`src-tauri/src/commands/agent_tools_specs.rs` `agent_tool_specs_for`）：
  - `probe_sql_injection` ← `target_write`
  - `probe_command_execution` / `run_analysis_script` ← `code_exec`
  - `run_target_poc` ← `code_exec` + `egress`
  - `plan_ssrf_channel` ← `internal_scope`
  未授权的类连调度地板都到不了——工具根本不进 `tools/list`。
- 能力类授权**只能**由 Tauri 命令 `set_scan_capability_authorization`
  （`scan_capability_authorization.rs`，含 `legal_attestation` 硬校验）写入；
  该写入过去绑在已删除的 Workbench 建任务 UI。

于是现在缺一个「在对话里下达能力类授权」的入口，导致深度工具不下发
（表现为 SSRF 不探测、深挖跑不起来）。

## 2. 硬安全不变量（绝不可破）

1. **Agent 永不自写能力授权**：`scan_capability_authorizations` 表只能由
   operator 经受信任通道写入；Agent 的任何 MCP 工具都不写它。
2. **Agent 永不自批 `legal_attestation`**：attestation 是 operator 的法律签字，
   必须由 operator 在前端面板点击产生。Agent 读取的目标页面/响应是不可信数据，
   自批 = 可对任意目标自开 RCE/SSRF/写操作（提示注入面）。
3. **Agent 永不登记 standing authorization、永不扩目标模式。**
4. `authorize` / `set_scan_standing_authorization` 动作**绝不进 Agent 工具面**
   （`tools/list`）——与 `human_scope_confirm` 同构，只能由前端面板的用户点击进入。
5. `internal_scope` 硬守卫、DNS pin、SSRF floor、per-action 审批、预算、scope 冻结
   全部不动。
6. 无 standing authorization 时，行为回落到「面板需人点」，与当前一致。
7. 记忆 / 技法 / jev 仍是 advisory——永不喂入任何授权/范围/结算门。

两堵墙（沿用 memory `active-testing-authorization-chain`）：
(a) 工具是否下发由每个 scan attempt 的**不可变 operator 记录**决定；
(b) 每次高危动作仍有 per-action 审批。本设计只补 (a) 的写入入口，不触碰 (b)。

## 3. 合法能力类（canonical set）

以 `scan_capability_authorization.rs::HIGH_RISK_CLASS_STRINGS` 为准，
前端 `AgentWorkbench.vue::HIGH_RISK_CLASSES` 与之镜像：

```
code_exec, target_write, post_write, egress, internal_scope, persistence, exfil, waf_evasion
```

面板分组（展示语义，不改变底层 class id）：

- **高危能力类**：上面 8 个 class，可多选。
- **开源框架深度分析**：映射到 `target_write`
  （deep-dive `agent_finding_deep_dive` 走 `target_write`）。
- **自定义**：直接勾选任意合法 class。
- **合法授权确认**（`legalAttestation`）：一个勾选框，未勾则「授权」按钮禁用。

## 4. 目标流程

```
用户: "针对 xxx.com 做安全测试"
  → Agent: draft_create(title, [https://xxx.com])          // 已有工具
  → Host 钩子: 查 standing authorization 是否命中该目标
     ├─ 命中(未过期/未超预算): Host 自动为该 scan_id 写能力授权
     │    (attestedBy='operator_standing')，Agent 直接 scan_start
     └─ 未命中: Agent 调 request_scan_authorization(taskId, proposedClasses, reason)
          → 只发事件 scan.authorization.requested + 记录 Agent 建议，绝不写授权
          → 前端渲染对话内授权面板(多选 class + attestation 勾选框)
          → operator 点选 + 勾 attestation + 点「授权」
          → 前端走 Host 授权通道 scanAuthorizationConversationAction(action='authorize')
          → Host 校验 witness + investigation.execute + disclosureRuntime
          → 经 /scan-task 桥 internalAction='authorize_capability'
          → Rust set_scan_capability_authorization(scan_id, attempt, attestation, classes, 'operator')
  → Agent: scan_start → 深度工具按授权类下发
```

## 5. 组件设计

### 5.1 Host 授权处理器（最核心，step 1）

仿 `discovery-assist.mjs::discoveryConversationAction` 新增
`scanAuthorizationConversationAction(host, conversationId, projectId, input)`：

- `input.action==='authorize'`，带 `{ taskId, attemptNumber?, classes:[...],
  legalAttestation:true, witness }`。
- witness = `{ appRevision, grantRevision, policyRevision, runtimeIdentity }`，
  全部必须等于当前值，否则 `host_scan_authorization_stale` 409。
- 校验 scanning app enabled、`grant.permissions` 含 `investigation.execute`、
  `grant.disclosureRuntime===identity`，否则 403。
- `classes ⊆` 合法集合；非空 classes 必须 `legalAttestation===true`，否则 400。
- 通过后经 `host.apps.scanningTasks.authorizeCapability(...)`
  → `/scan-task` 桥 `internalAction='authorize_capability'`
  → Rust `set_scan_capability_authorization(scan_id, attempt, attestation, classes, 'operator')`。
- 写后二次重校 witness（authority_changed 防 TOCTOU），再 `host.emit`
  `scan.authorization.updated`。
- **`authorize` 动作绝不进 Agent 的 `tools/list`**；它只能由前端面板经
  server 路由 `POST /scan-authorization/:conversationId` 进入
  （与 discovery 的 `confirm` 同构）。
- `input.action==='status'`（或缺省）：只读返回当前授权态 + witness，供面板渲染。

### 5.2 Agent 工具 `request_scan_authorization`（step 2）

- 加入 scanning manifest（`apps/scanning/app.package.json`），权限
  `investigation.draft.write`；bump 版本 + 同步 bundled-apps。
- 入参 `{ taskId, proposedClasses:[...], reason }`。
- 由 Host 在 `ScanningTasks.call` 本地处理（**不**走 `/scan-task` 桥、**不**写授权）：
  - 校验 `proposedClasses ⊆` 合法能力类；
  - 校验 `taskId` 格式（本会话绑定的任务或 draft id）；
  - `onAuthorizationRequested({conversationId, taskId, proposedClasses, reason})`
    → service `emit('scan.authorization.requested', ...)`；
  - 返回回执 `{ requested:true, authorizationPending:true, authorized:false }`。
- 工具描述明确：这只呼出面板 + 记录建议，不授权、不执行、不自批 attestation。

### 5.3 长期授权 standing authorization（step 3）

- Rust 新表 `scan_standing_authorizations`：
  `target_pattern`（精确域名或 `*.pattern`）、`authorized_classes`（JSON）、
  `legal_attestation`、`budget`（可选，剩余可用次数）、`expires_at`（可选）、
  `attested_by`、`created_at`。
- operator-only Tauri 命令
  `set_scan_standing_authorization({ targetPattern, classes, legalAttestation:true,
  budget?, expiresAt? })`（`lib.rs` 注册）+ 匹配函数
  `standing_authorization_match(host) -> Option<classes>`。
- Agent 建 draft 后 Host 钩子查命中（未过期/未超预算）→ 自动为该 scan_id 写
  能力授权（`attestedBy='operator_standing'`，复用
  `set_scan_capability_authorization`），Agent 可直接 `scan_start`。
- **该命令绝不进 Agent 工具面**；Agent 永不登记、永不扩 pattern。

### 5.4 前端面板（step 4）

- 复用 `AgentWorkbench.vue:155-205` 的能力类多选 + attestation 逻辑做一个
  **聊天内授权面板**组件，由 `scan.authorization.requested` 事件驱动渲染。
- 提交走 Host 授权通道（5.1）；未勾 attestation 则「授权」按钮禁用。
- operator 配置页登记 standing authorization（5.3）。

### 5.5 提示（step 5）

- `packages/agent-host/src/quality-prompt.mjs` 教 Agent：
  建 draft → 若未命中长期授权则 `request_scan_authorization` 呼面板 →
  operator 点选后再 `scan_start`；不自批、不催点。

## 6. taskId ↔ scan_id 说明（已知边界）

`scan_capability_authorizations.scan_id` 外键指向 `sentinel_scans(id)`，是原生扫描
任务；`draft_create` 建的是 Node 侧草稿（`investigations.sqlite3`）。因此：

- **operator 授权通道（5.1）在原生 scan_id 上落库**——`set_scan_capability_authorization`
  自身会拒绝不存在的 scan（"任务不存在"），这是授权真正生效的那一步，带强校验。
- **Agent 的 `request_scan_authorization`（5.2）是纯 advisory**：只校验 class 合法 +
  taskId 格式 + 发事件，零权限、零副作用。更强的任务归属校验发生在 operator 通道
  （它跑在原生桥上，project-scoped `owned` 守卫 + scan 存在性校验）。
- 这把「Agent 零权限呼叫」与「operator 带校验落库」清晰分成两堵墙，正是不变量 1/2/4
  要求的隔离。

## 7. 测试（真测试，不写桩）

- **Rust**：standing authorization 匹配（命中/未命中/过期/超预算）；
  `set_scan_capability_authorization` 仍要求 attestation（保留现有测试）；
  `authorize_capability` 桥动作 project-scoped + 写入 + attestation 必需；
  Agent 路径无法写授权（负路径——无 agent 可达 action 写该表）。
- **Host**：授权处理器 witness 校验、attestation 必需、operator-only
  （`authorize` 不在 `tools/list`）；`request_scan_authorization` 只发事件不授权
  （桥未被调用）。
- **Node**：`request_scan_authorization` 只呼面板不授权；未授权时深度工具不下发
  （`agent_tools_specs` 过滤——Rust 侧既有 gate）。

## 落地进度

见 PR 描述与各文件注释。核心完成标准：
step 1（host 授权处理器）+ step 2（request_scan_authorization）+ 对应测试落地并全绿。
step 3/4/5 按优先级推进，未完成的在 PR 中诚实标注「仅设计 / 未做」。
