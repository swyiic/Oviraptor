# Oviraptor「平台 + 内置应用」改造分析（2026-10-16）

> 范围：`main` 与 `origin/feature/agent-first-app-platform-20261016`（下称 **AF 分支**）的只读分析。本文是唯一新增文件，没有改动代码，也没有 commit。
> 术语：用户叫「Nest」的东西，在代码里是 **sentinel**（前端 `src/features/sentinel`、插件 id `sentinel`、UI 标签「扫描/Nest」），底层是 Rust `agent_runtime/multi_agent/*`（含 `homogeneous/` 同质主管）。

## 1. main 现状架构（文字图）

```
Tauri 桌面进程（单体）
├─ 前端 Vue（src/）
│  ├─ App.vue / shell/        宿主外壳：工作空间、导航；Asset/Nest 入口按钮直接写在 App.vue 里
│  ├─ plugins/                「内置插件」注册表，仅编译期生效
│  │   ├─ catalog.ts          BUILTIN_PLUGINS = asset | sentinel | hackerone（冻结常量）
│  │   ├─ types.ts            BuiltinPluginId 是封闭的联合类型；页面只有导航元数据
│  │   ├─ preferences.ts      每个插件的启用/停用偏好（本地）
│  │   └─ runtime.ts          导航意图、链接候选
│  └─ features/
│      ├─ assets/（含 exposure/）  资产、归属、暴露面 UI，经 api.ts 调 Tauri invoke
│      ├─ sentinel/               Nest：调查对话、执行、fuse、overview
│      ├─ skills/                 技能目录 / 指令库 UI（SkillsWorkspace、catalogApi）
│      ├─ runtime/ conversations/ configuration/ environment/ hackerone/ workspaces/
└─ Rust（src-tauri/src/）
   ├─ commands/（约 780 个文件）     所有 Tauri 命令平铺在一个模块：assets*、exposure*、ownership*、
   │                                 agent_*、multi_agent_*、native_source_*、web_* ……
   ├─ application/model.rs           只是跨领域的 DTO（DashboardStats、AppSettings……），不是「应用」抽象
   ├─ agent_runtime/                 Nest 内核：multi_agent/{supervisor, specialist, scheduler, budget,
   │                                 directive, mailbox, homogeneous/{decompose, decision, apply, delivery…}}
   ├─ assets/、asset_logs、page_isolation   资产领域
   ├─ skill_packages/、skills_catalog/      技能包（SQLite、builtin 标记）与指令/知识目录
   ├─ scanning/、native_pipeline/、tool_supply/、jobs/、hackerone/
   └─ db*.rs                         一个共享 SQLite 库，asset、scan、agent 的表都在里面
```

**main 上的结论**：`src/plugins` 只是一份**编译期的导航与能力声明**（注释原文：“No URL, script path, dynamic manifest or installer”）。启停只影响 UI 可见性，组件是宿主直接 import 的。Asset 和 Nest 在三处深度耦合：
- 共享同一个 SQLite 与 `db_initialize*` 迁移；
- 共享平铺的 `commands/`，并通过 `lib.rs` 统一注册；
- 宿主 `App.vue` 直接引用它们的统计字段（`assetCount`、`scanCount`、`sentinelScanCount`）。

所以两者都还**不是可独立拆分的模块**。

## 2. AF 分支新增的应用化 / 技能化能力

源码层面的改动（不含 `docs/agent-first-20261016` 大量证据文件和 lock 文件）：

| 位置 | 内容 |
|---|---|
| `packages/app-sdk` | 应用清单校验（`manifest.mjs`）、打包（`packager.mjs`/`package-directory.mjs`）、worker 协议、page-client、handoff，以及 CLI `bin/pack-app.mjs` |
| `packages/native-app-sdk`（Rust crate） | 原生应用的 manifest/schema/driver，经 stdio 与宿主通信；`lib.rs` 里 `pub use native_app_sdk` |
| `packages/agent-host` | **新宿主**（Node）：`AppManager.install/setEnabled`、app-process、navigation、mcp-bridge（把应用工具暴露给 agent）、skills、upgrades、HTTP server |
| `packages/agent-ui` | 新宿主的 UI |
| `packages/runtime-acp` | 通过 ACP 接入 agent 运行时（`--runtime none / grok-acp`） |
| `packages/skills-library` | 声明式 Skill 库（SQLite，导入目录或内置 bundle；内置 36 个 skill、443 个文件） |
| `apps/assets` | **Asset 应用**：`app.package.json`（JS worker 版 v1）+ `native-manifest.mjs`（原生版 v2，入口 `bin/oviraptor-assets-service`）、独立 UI（`ui/Root.vue`）、legacy 契约审计 |
| `apps/scanning` | **扫描应用**：工具 `draft_create / draft_cancel / journal_read / legacy_saved_scans` |
| `examples/echo-app` | 示例应用 |
| `src-tauri/src/bin/oviraptor-{assets-service,scan-service,app-worker}.rs` | 原生应用的二进制入口 |
| `src-tauri/src/assets/{service,service_bootstrap,exposure_service,ownership_service,…}.rs` | 把资产领域从 commands 抽成服务层（约 9k 行，含测试） |
| `src-tauri/src/scanning/native_service_*`、`service_bootstrap.rs` | 扫描的原生服务化 |
| `src-tauri/src/legacy_app_bridge.rs`、`db/service_custody.rs` | 旧数据库到应用的桥接与数据托管 |
| `tools/build_app_packages.mjs` | 遍历 `apps/*`、`examples/*` 的 `app.package.json`，产出 `.agent-build/apps/<id>-<ver>` |
| `tools/verify_agent_first_build.mjs` | 在 repo 外启动独立分发包，验收：`/api/apps` 为空、能保存对话、`/api/skills/bundled` 装入 36 个 skill |
| `tools/test_skills_library_resources.cjs` | Skill 资源分页读取测试 |
| `src/features/assets/*`（小改） | `uiPorts.ts`、`formInteraction.ts`：把旧 UI 抽成端口，供应用 UI 复用 |

### 2.1 应用清单（manifest）格式

- **v1（JS worker）**，字段：`schemaVersion:1, id(反域名), version, hostApi(semver), entry(worker.mjs), dataSchemaVersion, pages[{id,title,path}], permissions[{id,description}], tools[{name,description,inputSchema}], lifecycle{start,stop}, files[{path,sha256}]`。
- **v2（原生）**：在 v1 基础上多了 `runtime{kind:'native',platform,arch}`、`actions`，`entry` 指向原生二进制。
- 有上限：例如 v1 最多 64 个工具、16 个页面、包体 32MB；v2 包体 256MB。
- 外层 `app.package.json = { manifest, ...打包参数 }`。

### 2.2 安装与加载

`AppManager.install(source)` 的流程：校验 → 按 manifest 计算 sha256 → 写入宿主 `applications` 表 → 默认停用。
- `setEnabled` 启动或停止 worker 进程；停用时清空 `application_grants`。
- 每次加载都会校验 digest，不一致就报 `host_app_integrity_changed`。
- 应用工具以限定名（`qualifiedToolName(appId, name)`）经 mcp-bridge 交给 agent。
- 应用间通过 `ui.handoff` 和 navigation 做受审的跨应用导航。

### 2.3 内置与可安装

- **main**：只有内置（编译期 `BUILTIN_PLUGINS`）。
- **AF 分支**：只有「可安装包」。`apps/*` 是第一方，但同样要打包再安装；独立分发版启动时 `/api/apps` 为空，没有任何预装应用。

另外有一点要注意：**App 和 Skill 是两套独立机制**。skills-library 的 README 写明 “App installation and Skill import remain separate operations”，Skill 只是指令和参考资料，不执行代码。所以「把应用当 skill 安装」这个说法，在 AF 分支的模型里应该落实为：**应用包（含工具和页面）+ 可选的配套 Skill（教 agent 怎么用）**，而不是把应用塞进 Skill 库。

## 3. 拆分现状与缺口

| 目标 | AF 分支进度 | 缺口 |
|---|---|---|
| Asset 作为应用 | ✅ 有 `apps/assets`（v1 和 v2 两版）、原生 service 二进制、服务层、独立 UI、legacy 桥 | v1 只有 4 个工具（inventory_import/stats/search、legacy_snapshot_stats），归属门禁、暴露面上传等主要靠 v2 actions 和 legacy 契约，覆盖完整度要按 `contracts/legacy-ipc-contracts.json` 逐项核对；数据仍通过 legacy bridge 读旧库，还没彻底迁到应用私有存储 |
| Nest 作为应用 | ❌ **基本没开始**。`apps/scanning` 只能做草稿、journal 和已保存扫描的只读访问；`agent_runtime/multi_agent`（含 homogeneous）在 AF 分支只有零星改动（budget clock 等），**没有抽成服务，也没有 manifest** | 缺 Nest 应用包，缺 multi_agent 的服务化入口（类似 `assets/service_bootstrap`），缺同质主管的工具和 actions 定义。另有两件事必须先定：AF 宿主自带 ACP agent 运行时，Nest 的主管/专家调度与宿主 agent 是什么关系；Nest 的模型预算/财务账本（budget、financial_continuation）如何在宿主下托管 |
| 内置分发 | ❌ 宿主没有「预装 / 内置应用」的机制 | 需要一份 bundled-apps 清单，首启时自动 install（必要时 enable），标记 `builtin`，并定义能否卸载、升级策略（参照 `skills.installBundled` 的做法） |
| 作为 skill 安装 | 部分。Skill 库已就绪 | 缺「应用 ↔ 配套 Skill」的关联字段（例如 manifest 增加 `skills`），也缺安装应用时一起导入 Skill 的流程 |
| 启停 / 导航 | ✅ 已有 `setEnabled`、navigation、handoff | Tauri 主程序这边还在用 main 的 `BUILTIN_PLUGINS`，没有接入 agent-host；两套宿主并存 |
| 宿主统一 | ❌ 现在是 Tauri 单体 + Node agent-host 两套 | 要决定 Tauri 内嵌 agent-host（sidecar），还是 Tauri 直接实现 host API |

**最大障碍**：Nest 的多智能体内核（`agent_runtime/multi_agent` + `commands/` 里数百个 `agent_*`/`native_source_*`/`multi_agent_*` 文件）与共享 SQLite、预算账本、tool_supply、jobs 紧耦合，而且在 AF 分支里几乎没动。

## 4. 改造路线（每个阶段都有验收标准）

- **P0 基线**：基于 AF 分支新开工作分支，跑通以下命令：
  - `node tools/build_app_packages.mjs`
  - `node tools/verify_agent_first_build.mjs`
  - `apps/assets/test`
  - `tools/test_rust_business_boundaries.cjs`
  - `cargo test`

  **验收**：全部通过，并把结果记录到 docs。
- **P1 内置应用机制**：宿主增加 `bundledApps` 清单，首启时 install 并 enable `org.oviraptor.assets`、`org.oviraptor.scanning`，标记 builtin。用户可以停用；能否卸载按策略决定。

  **验收**：扩展 `verify_agent_first_build`，断言 `/api/apps` 含 2 个内置应用；停用再启用之后数据仍在。
- **P2 Asset 应用补全**：按 legacy IPC 契约逐项迁到 v2 actions 或 tools，数据从 legacy bridge 迁到应用私有存储（带一次性迁移）。

  **验收**：契约审计脚本显示 0 项未覆盖；在应用里完成资产导入、归属、暴露面的端到端流程。
- **P3 Nest 服务化**：仿照 `assets/service_bootstrap`，新增 `agent_runtime/service_bootstrap.rs` 和 `bin/oviraptor-nest-service.rs`。对外只暴露 `nest.start / status / directive / stop / journal` 等工具或动作；预算账本归应用私有。在 `test_rust_business_boundaries` 里加规则，禁止 commands 直连 multi_agent。

  **验收**：边界测试通过；无头启动一次同质主管任务（可以用 mock 模型）并拿到 journal。
- **P4 Nest 应用包**：新增 `apps/nest`（v2 manifest + 页面 + 配套 Skill），纳入 bundledApps。

  **验收**：从 `.agent-build` 安装 Nest，启用、导航、handoff（Asset → Nest 选目标）都能走通。
- **P5 应用 ↔ Skill 关联**：manifest 增加可选的 `skills` 字段，安装应用时预览并导入配套 Skill；Skill 也可以独立安装。

  **验收**：`test_skills_library_resources` 和新增的关联测试通过。
- **P6 宿主统一**：Tauri 以 sidecar 方式运行 agent-host，前端导航从 `BUILTIN_PLUGINS` 改为读 `/api/apps`；退役 `src/plugins/catalog.ts`。

  **验收**：桌面构建中，Asset 和 Nest 都作为应用出现，并且可以启停。

## 5. 风险与建议

1. **建议基于 AF 分支继续，而不是 main**。SDK、宿主、打包、完整性校验、启停、导航和 Asset 应用都已经在 AF 分支上，大约 1.8 万行源码加测试；回到 main 等于重做。前提是先把 AF 分支与 main 的最新提交 rebase/merge 一次。目前 AF 对 main 的 `src/` 改动很小（19 个文件），冲突主要会出现在 `src-tauri/src/commands` 和 `assets`。
2. **双宿主风险**：Tauri 单体和 Node agent-host 长期并存，会造成两套权限和数据。P6 之前要冻结 `src/plugins` 的新增功能。
3. **数据迁移风险**：legacy bridge 和 `service_custody` 是只读桥。真正迁移时要有备份和回滚方案。
4. **Nest 体量风险**：multi_agent 及其测试有数百个文件，并且已经积累大量审计文档（`docs/NEST_*`）。服务化要「外包一层」：先保留内部实现，只收口入口，不要先重写。
5. **概念澄清**：「当 skill 安装」应该理解为「应用包 + 配套 Skill」，不要把可执行能力放进 Skill 库，否则会破坏 skills-library「不执行代码」的安全前提。
6. AF 分支的 `docs/agent-first-20261016` 证据文件很多（约 1000 个文件、近 50 万行）。合并前建议精简，或改成制品存储。
