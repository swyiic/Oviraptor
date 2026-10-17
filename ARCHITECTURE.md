# Oviraptor 架构（活文档）

> **地位**：仓库根目录的当前架构真源。  
> **分支锚点**：`feature/agent-first-app-platform-20261016`；**以包含本文件的提交为准**（勿冻结过期 hash）。  
> **语言**：中文；只描述可核对的边界，不把历史审计数字当作当前验收。  
> **配套**：应用开发见 [`docs/APP_DEVELOPER_GUIDE.md`](docs/APP_DEVELOPER_GUIDE.md)；包内细节以各包 README 为准。

本文吸收并取代「日常查阅」角色下分散的旧 `docs/architecture.md`、`docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md` 与日期戳 agent-first 规格。历史日期戳 / `NEST_*` 审计及上述旧文档**已从工作树移除**；快照见 git 标签 [`archive/docs-historical-20261017`](docs/HISTORY.md)（恢复说明见附录与 `docs/HISTORY.md`）。

---

## 1. 目的与产品边界

Oviraptor（窃蛋龙）是面向**授权资产管理**的跨平台桌面端：项目管理、资产采集/归属/暴露面、Native Agent 扫描编排、项目内 Chat 与 Skills。

| 在范围内 | 不在本文件或当前平台宣称范围内 |
| --- | --- |
| Tauri 2 / Rust 桌面壳 + Vue 3 UI + 私有 SQLite | 把 agent-first host 当作默认唯一产品入口 |
| `src-tauri/src/agent_runtime/` Native 控制面/执行面 | 未授权目标网络探测或付费模型「已验收」声明 |
| 可选 Node **agent-first** 宿主与可安装应用包 | 发布者签名、OS 级沙箱、自动升级回滚（尚未接受） |
| 内置域应用 `apps/assets`、`apps/scanning` 的**已迁移子集** | Scanning **完整执行引擎**已迁出桌面（尚未） |

两条并行轨：

1. **桌面轨（默认产品）**：`src/` + `src-tauri/`，完整业务与 Native 扫描仍在此。
2. **Agent-first 轨（opt-in）**：`packages/agent-host` + `packages/app-sdk` + `apps/*`；桌面可通过 `agent_host_sidecar` 拉起 Node 宿主，**不**替代 Nest/扫描内核。

新任务只走 Native Runtime；旧 Strix 运行时与结果格式不再接受（见根 [`README.md`](README.md)）。

---

## 2. 仓库地图

```text
Oviraptor/
├── ARCHITECTURE.md          ← 本文件（活架构）
├── README.md                ← 产品与本地开发入口
├── src/                     ← Vue 3 前端（features / shell）
├── src-tauri/               ← Tauri 2 / Rust 单体与 agent_runtime
├── packages/
│   ├── app-sdk/             ← 应用清单、打包、stdio JSON-RPC worker
│   ├── agent-host/          ← 独立 Node 宿主（项目/会话/授权/Apps）
│   ├── agent-ui/            ← 宿主 UI
│   ├── runtime-acp/         ← 官方 Grok CLI ACP 适配（非自研模型循环）
│   ├── native-app-sdk/      ← Native 侧应用相关 SDK
│   └── skills-library/      ← 随包 Skills 目录资源
├── apps/
│   ├── assets/              ← org.oviraptor.assets
│   └── scanning/            ← org.oviraptor.scanning
├── examples/echo-app/       ← 最小可安装示例
├── tools/                   ← 发布回执、模块边界等工具
└── docs/                    ← 活文档（历史审计已离树，见标签）
```

| 路径 | 角色 |
| --- | --- |
| `src/features/*` | 领域 UI：assets、sentinel（扫描）、conversations、skills、configuration、… |
| `src/shell/` | 默认导航、内置模块组合、跨 owner 展示协调 |
| `src-tauri/src/lib.rs` | 启动、AppState、命令注册、sidecar 生命周期 |
| `src-tauri/src/commands.rs` | 薄 owner 索引（当前含 14 个 `pub(crate) mod`，见 §4） |
| `src-tauri/src/scanning.rs` | 扫描准入/工作台/费用租约/与 Native 衔接（独立 root owner） |
| `src-tauri/src/agent_runtime/` | Native Agent 契约、reducer、multi_agent、模型与存储 |
| `src-tauri/src/agent_host_sidecar.rs` | 桌面拉起 Node agent-host；不触碰 Nest/资产内核 |
| `src-tauri/src/legacy_app_bridge.rs` + `bin/oviraptor-app-worker.rs` | 只读兼容适配器（显式 staged snapshot） |
| `packages/*`、`apps/*` | Agent-first 包边界；细节见各自 README |
| 历史审计（已离树） | 原 `docs/archive/` 与旧架构 stub / `progress.md`；恢复见标签 `archive/docs-historical-20261017` 与 [`docs/HISTORY.md`](docs/HISTORY.md) |
| 体积证据树（已外置） | 原 `docs/agent-first-20261016/`、`docs/asset-accuracy-20261015/`；内容已离仓，恢复细节见 [`docs/OFFLOAD_NOTE_20261017.md`](docs/OFFLOAD_NOTE_20261017.md) |

---

## 3. 运行时拓扑

### 3.1 桌面（默认）

```text
Vue UI (src/)
    │  Tauri IPC
    ▼
Rust 单体 (src-tauri)
    ├── commands::* / scanning / conversations / skills_*
    ├── db::*  →  用户私有目录 SQLite（如 ~/oviraptor/oviraptor.sqlite3）
    ├── agent_runtime/     控制面：状态、预算、租约、终态 reducer
    │       └── 执行面：经授权的工具 / pipeline / multi_agent 调度
    └── agent_host_sidecar → 可选子进程：Node agent-host（loopback + session）
```

- 数据在应用私有目录；不要求系统预装 SQLite。
- 控制面只由 Rust 编排修改任务状态、预算、授权与终态；执行面返回事实与 artifact 引用，不能自行「宣布漏洞成立」。
- Sidecar 向 webview 暴露 `origin` / `session`；可用环境变量注入已有宿主，也可本地 spawn。**不**初始化应用域表，也**不**替代 `scanning` / Nest。

### 3.2 Agent-first（opt-in）

```text
agent-host (Node 24, packages/agent-host)
    ├── 自有 SQLite：项目 / 会话 / 应用清单 / grant revision
    ├── runtime-acp → 官方 `grok … agent stdio`（用户自备登录）
    ├── mcp-bridge  → 会话绑定的工具发现与派发
    └── app workers（stdio JSON-RPC）← app-sdk 协议
            ├── org.oviraptor.assets    → inventory.sqlite3
            └── org.oviraptor.scanning  → investigations.sqlite3
```

启动（仓库根）：

```sh
npm run build:agent-first
npm run build:app-packages
node .agent-build/agent-first/packages/agent-host/src/cli.mjs --data-dir /absolute/private/host-data
```

详见 [`packages/agent-host/README.md`](packages/agent-host/README.md)。

---

## 4. 模块归属（桌面轨）

原则：**DTO / IPC / 持久化不变量归 owner**；Shell 可聚合展示，但不能用 UI 禁用代替后端授权。

### 4.1 `commands.rs` 索引模块（14）

下列 14 个模块名是**撰写时快照**；增减以源码真源 [`src-tauri/src/commands.rs`](src-tauri/src/commands.rs) 中显式 `pub(crate) mod` 为准，勿只改本文数字。

当前快照：

`workspace_projects` · `application_overview` · `application_settings` · `workspace_config_profiles` · `assets` · `asset_rule_catalog` · `page_isolation` · `asset_export` · `tool_administrator` · `environment` · `model_profiles` · `ownership` · `exposure` · `hackerone`

扫描、Skills、Chat、FOFA 请求实现等另有独立 root owner / 模块，**不要**把它们算进上述 14 个索引里。

### 4.2 前端 features（与 Rust owner 对齐）

| 前端 | 主要 Rust / 域 |
| --- | --- |
| `src/shell/` | 导航、内置模块目录、跨 owner 协调 |
| `features/assets` | `commands::assets` / ownership / exposure / export / page_isolation |
| `features/sentinel` | `scanning` + Native runtime |
| `features/conversations` | `conversations` |
| `features/skills` | `skills_catalog` / `skill_packages` |
| `features/configuration` | 配置 profiles / 模型 profiles |
| `features/environment` · `runtime` | 环境准备、Worker 节点 |
| `features/hackerone` | HackerOne 缓存与导入 |
| `features/workspaces` | 项目身份与删除保护 |
| `features/agentHost` | 与 sidecar / agent-first UI 衔接 |

历史 IPC 方法数、注册表长度等**以当时审计文档为准**，可能已过时；改接线时对照 `lib.rs` 的 `generate_handler!` 与各 owner，勿照抄旧数字。更细的阶段性归属曾记于历史审计（现仅可通过标签 `archive/docs-historical-20261017` 检出，例如当时的 `BUSINESS_MODULE_BOUNDARIES` / `FRONTEND_MODULE_OWNERSHIP`）。

---

## 5. Native Agent Runtime（桌面）

路径：`src-tauri/src/agent_runtime/`。

**仍成立的设计边界**（与早期长文一致的部分）：

- Rust 拥有任务状态、预算、身份、停止与终态；模型只在分配上下文中规划/判断。
- 工具经 Broker / 策略 / 预算与审计；模型不直接持有任意 Cookie、密钥、文件系统或裸 Shell。
- 事件与证据可持久化；终态经 single-writer reducer / projection，不能用测试 helper 概括生产入口。

**当前树中可见的主要子树**（名称即路径，避免臆造 API）：

| 区域 | 作用（据目录与 README） |
| --- | --- |
| `contract.rs`、`reducer*`、`single_projection*` | 状态词汇、归约、单 Agent 终态发布 |
| `multi_agent/`（含 `scheduler/`、`budget/`、`mailbox/` 等） | 多智能体调度、租约、预算与协作消息 |
| `model/` | 模型网关与相关支持 |
| `store*`、`checkpoint*`、`evidence_graph*` | 事件/检查点/证据 |
| `execution_owner*`、`controlled_executor*`、`web_mode*` | 执行所有权与受控执行 |
| `role_config*` | 角色配置 |

产品侧已声明的角色方向（见根 README）：Coordinator、SPA/API Mapper、Web Executor、Evidence Reviewer 等；Deep Investigator 限制为只读角色。更多角色与跨平台发布验收仍在推进。

> **注意**：旧 `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md` 曾写「调度器、多 Agent 未实现」等——**已过时且已从树中删除**，以本文件 + 源码树 + README 为准。完整迁移计划类长文亦仅存于标签 `archive/docs-historical-20261017`。

---

## 6. Agent-first 应用平台（并行轨）

| 包 | 职责 | README |
| --- | --- | --- |
| `packages/app-sdk` | 清单校验、目录包、`pack-app`、stdio worker 协议；**无**资产/扫描业务 | [README](packages/app-sdk/README.md) |
| `packages/agent-host` | 独立宿主：安装/启用/授权/升级、MCP 桥、Skills、指标 | [README](packages/agent-host/README.md) |
| `packages/runtime-acp` | 官方 Grok ACP 传输与会话；不读 OAuth 文件、不自研模型环 | [README](packages/runtime-acp/README.md) |
| `packages/agent-ui` | 宿主界面 | [README](packages/agent-ui/README.md) |
| `examples/echo-app` | 首个独立打包示例 | 包内文件 |

要点（与实现一致、不做超额宣称）：

- 安装 ≠ 授权；启用后仍需显式项目权限 / 数据披露。关闭页面 ≠ 禁用应用。
- 工具名对外为 `appId::toolName`；派发携带 `hostBoundProjectId`、`grantRevision`、`operationId`。
- 页面在隔离 iframe：无 host API token；消息白名单，不转发为任意 API。
- Skills 是指导与资源，**不**授予应用权限；脚本保持惰性。
- 宿主 loopback + 每进程 session；拒绝随意 CORS / `Origin: null`。
- `bundled-apps.json` 预置 `org.oviraptor.assets` / `org.oviraptor.scanning`；`shell-apps.json` 描述壳层导航与能力标签（壳能力 ≠ worker 已实现工具全集）。

与遗留桌面 Chat / 模型配置：**尚未**一一替换；遗留 provider 配置不会静默变成 Grok 订阅凭据。

---

## 7. 内置域应用

### 7.1 Assets — [`apps/assets/README.md`](apps/assets/README.md)

- 应用 ID：`org.oviraptor.assets`；自有 `inventory.sqlite3`。
- 已实现工具：`inventory_import`、`inventory_stats`、`inventory_search`、`inventory_schema`、`inventory_query`、`project_overview`、`exposure_overview`。
- **不做**：目标网络 I/O、完整桌面规范身份/归属/暴露面工作流迁移、把缺失桥接报成空库存。
- **已完成：下架生产不可用 legacy 工具**：`legacy_snapshot_stats` **未**列入公开 manifest / describe 工具表；仅在宿主注入 `legacyBridge` 时注册，宿主托管桥接通前不对 Agent 暴露（避免把「不可用」误读成空库存）。

### 7.2 Scanning — [`apps/scanning/README.md`](apps/scanning/README.md)

- 应用 ID：`org.oviraptor.scanning`；自有 `investigations.sqlite3`。
- 已实现工具：`draft_create`、`draft_cancel`、`task_status`、`journal_read`。
- **已完成：下架生产不可用 legacy 工具**：`legacy_saved_scans` **未**列入公开 manifest / describe 工具表；仅在宿主注入 `legacyBridge` 时注册，宿主托管桥接通前不对 Agent 暴露。
- `draft_create` 明确返回 `executionStarted:false` / `executionAvailable:false` / `authorization:not_granted`——**存草稿 ≠ 开扫**。
- `thin-service.mjs`：Phase-1 非敏感工具表面；**不**改写 Nest/multi_agent 内核。
- 完整执行、费用/租约/重放、取消实引擎状态：仍在桌面 `scanning` + Native。

### 7.3 Legacy bridge 与 app-sdk 共享助手

- Rust：`legacy_app_bridge` + `oviraptor-app-worker`，只读、显式 `--data-dir` 下的 `legacy-input.sqlite3` staged 快照。
- 已安装 Node worker **无**子进程权限，不能拉起该二进制；`createApplication(..., {legacyBridge})` 是受信嵌入缝，宿主托管传输仍为后续工作。
- **已完成：app-sdk 共享助手**：`apps/assets` 与 `apps/scanning` 的 SQLite / legacy stub 去重进 `packages/app-sdk`（`sqlite-data.mjs`、`legacy-bridge-stub.mjs`），随 Node runtime 一起 vendor；应用侧经 `sdk/` 引用，不再各维护一份副本。

---

## 8. 安全与信任边界

| 层 | 当前事实 |
| --- | --- |
| 应用包 | 哈希完整性 + 受信本地目录安装；**不是**发布者签名 |
| Node 权限 | 文件系统标志约束包/数据路径；**不是**网络沙箱或敌意原生代码隔离 |
| Grant | 声明权限 ≠ 授予；撤销后须扣留结果字节 |
| 页面 | 无 host token；不能把 iframe 消息升级成任意宿主 API |
| ACP / Grok | 不读/不复制 OAuth；官方 CLI 自有登录；无 `--always-approve` 类绕过 |
| TLS / 桌面 | 既有桌面认证与会话逻辑仍在 Rust 侧；agent-first 另有 session 头 |
| Sidecar | 最小接线；不把 Nest 业务内核暴露给 Node 包 |

非声称：完整 OS 分发签名验收、升级自动回滚、任意第三方包市场。

---

## 9. 数据与证据规则

| 算证据 | 不算「产品已迁完 / 已付费验收」 |
| --- | --- |
| `cargo` / `npm run build` / `node --test` 在**声明源码**上的退出码与日志 | 模拟 ACP 或 app 草稿通过 |
| `tools/release/build_receipt.mjs` 等本地构建观察（`sourceBinding: not_verified`） | 用旧 DMG 追认当前工作树 |
| 包测试：真实临时 SQLite、stdio worker、项目隔离 | Node 测试代替 connected native bridge |
| agent-first 验收清单中的明文命令与计数 | `docs/**/evidence` 大包/截图树本身 |

臃肿证据已离仓（原 `docs/agent-first-20261016/`、`docs/asset-accuracy-20261015/`）；恢复路径与清单见 [`docs/OFFLOAD_NOTE_20261017.md`](docs/OFFLOAD_NOTE_20261017.md)。架构正文只保留指针。发布核对流程见根 README。

---

## 10. 验证矩阵（入口）

在仓库根（按变更面选择，勿把历史 suite 总数抄为当前门禁）：

| 面 | 典型命令 |
| --- | --- |
| 前端生产构建 | `npm run build` |
| Rust 检查 | `cd src-tauri && cargo check` |
| Agent-first 构建 | `npm run build:agent-first`、`npm run build:app-packages` |
| Agent-first 测试 | `npm run test:agent-first`（及包内 `node --test`） |
| App SDK / 示例 | `node --test --test-concurrency=1 packages/app-sdk/test/*.test.mjs examples/echo-app/test/*.test.mjs` |
| Assets + Scanning | `node --test apps/assets/test/*.test.mjs apps/scanning/test/*.test.mjs` |
| 模块边界等 | `npm run test:module-boundaries` 等（见 `package.json` scripts） |
| 发布观察 | `node tools/release/build_receipt.mjs .` 等 |

验收叙述与「不得把 mock 当成真实 Grok/扫描」的历史稿见标签 `archive/docs-historical-20261017` 内当时的 agent-first acceptance 文档；当前以本文件 §9–§11、包测试与源码为准。

---

## 11. 迁移状态（诚实账本）

| 主题 | 状态 |
| --- | --- |
| Strix 退出 | 产品路径：**新任务不接受**旧 Strix；历史 Master Plan / progress 细节见标签 `archive/docs-historical-20261017` |
| 业务模块 owner 拆分 | 桌面轨已有明确 commands 索引与 features；历史验证数字勿直接复用 |
| Agent-first 宿主 | **可用、opt-in**；未替换全部遗留 Chat/配置 |
| App SDK + Echo | **可用**；另含共享 `sqlite-data` / `legacy-bridge-stub` 助手（见 §7.3） |
| Assets 应用包 | **薄壳已落地**（导入/统计/搜索）；完整桌面 Assets 未迁完；`legacy_snapshot_stats` 已从 Agent 公开表面下架 |
| Scanning 应用包 | **草稿/日志薄壳已落地**；**执行仍在桌面 Native**；`legacy_saved_scans` 已从 Agent 公开表面下架 |
| Legacy bridge | Rust 只读适配存在；**未**接入宿主托管运输 |
| Desktop sidecar | **已接线**（`agent_host_sidecar`） |
| Skills（宿主） | 预览/导入/目录/会话级同意；与桌面 Skills 数据不静默互通 |
| 默认应用切换 | **阻塞**：完整配置迁移 + Scanning 执行等价验收之前，桌面仍为完整路径 |
| 文档治理 | 活文档：本文件 + 开发指南 + 包 README；历史 `NEST_*`/日期戳已离树，见标签 `archive/docs-historical-20261017` |
| **解耦卫生（近期已完成）** | （1）下架生产不可用 legacy 工具；（2）app-sdk 共享 sqlite/legacy helpers；（3）新增中立 `src-tauri/src/scanning_contract.rs` 端口（`scope_contract` / `financial_continuation` / `recovery_approval_signature` / `fresh_signature` / `mapper_loop_bounds` / `evidence_bridge_const` / `log_events`），实现仍在 `scanning`，`agent_runtime` 经 contract 调用而不再点名 `crate::scanning::…`；（4）`tools/test_agent_runtime_import_ratchet.cjs` 将 `agent_runtime`→`crate::scanning` 边计数基线从 **73 降到 27**（类型/常量薄表面 + `log_events` 行为端口已抽；`crate::commands::assets` 仍为 0） |
| **解耦后续** | 剩余 **27** 处均为行为边（见 `docs/behavioral-decouple-plan.md`：homogeneous_execution、evidence_bridge、publication_replay 等）；完整共享 crate 抽取未宣称完成 |


---

## 12. 附录：历史文档与归档

历史日期戳审计、`NEST_*` 交付稿，以及旧 `docs/architecture.md`、`docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`、`docs/progress.md`，**已从工作树删除**。完整快照保留在 annotated 标签 **`archive/docs-historical-20261017`**（指向删除前的 HEAD）。简述见 [`docs/HISTORY.md`](docs/HISTORY.md)。

恢复示例：

```bash
git checkout archive/docs-historical-20261017 -- docs/archive \
  docs/architecture.md docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md docs/progress.md
```

| 类别 | 处置 |
| --- | --- |
| `docs/archive/`（NEST / agent-first / asset-exposure / general-audits / task-prompts / integration 等） | **已离树**；标签可恢复 |
| 旧架构 stub + `progress.md` | **已离树**；标签可恢复；日常以本文件为准 |
| 体积证据树 | **已离仓**；见 [`docs/OFFLOAD_NOTE_20261017.md`](docs/OFFLOAD_NOTE_20261017.md) |
| 包 README | **保留**为包级真源（`packages/*/README.md`、`apps/*/README.md`） |

### TODO（已知未知，不编造）

- [x] 历史审计从工作树移除；快照标签 `archive/docs-historical-20261017`；体积证据仍外置（见 OFFLOAD_NOTE）。
- [ ] 当前 IPC 注册精确个数、前端 API 方法数：以 `lib.rs` / 生成清单现场核对，本文不固化可能过期的整数。
- [ ] Scanning 执行迁入 app/service 的协议、费用/租约对接、宿主 legacy 运输：仍为后续交付，无完成日期承诺。
- [ ] Agent-first 与桌面配置/Chat 的正式切默认条件：依赖独立验收（历史 acceptance 稿见上述标签）。
- [ ] `shell-apps.json` 中的壳层能力标签与 worker 实装工具之间的逐项矩阵：宜在 APP 指南或包 README 增补，本文不臆造对应表。

---

*修订时请同步：以包含本文件的提交为准（勿写死过期 hash）、§11 状态表、包 README 链接是否仍有效。*
