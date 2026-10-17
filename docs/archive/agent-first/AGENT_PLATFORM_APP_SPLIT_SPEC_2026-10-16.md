# Oviraptor Agent 平台 / 应用拆分权威规格（2026-10-16）

> 基线分支：`feature/agent-first-app-platform-20261016`（下称 AF 分支）。前置分析：`docs/APP_PLATFORM_REFACTOR_ANALYSIS_2026-10-16.md`。
> 术语：Nest = 代码里的 `sentinel`（前端 `src/features/sentinel`）+ Rust `src-tauri/src/agent_runtime/multi_agent/`（含 `homogeneous/` 同质主管）。Asset = 资产/归属/暴露面（`src/features/assets`、`src-tauri/src/assets`、`apps/assets`）。
> 本规格只定义边界与增量，**不重写** AF 分支已有的 app-sdk / agent-host / skills-library / apps/assets。

## A. 目标架构

### A.1 三层

```
┌──────────────────────── Host：Agent 平台 ────────────────────────┐
│ packages/agent-host   对话编排、AppManager(install/enable/grant)、   │
│                       navigation/handoff、mcp-bridge、skills 访问   │
│ packages/agent-ui     聊天 UI、应用列表、启停、导航                   │
│ packages/runtime-acp  ACP 运行时（模型推理由运行时负责）               │
│ packages/skills-library  Skill 库（声明式，不执行）                   │
│ packages/app-sdk / native-app-sdk  应用 SDK 契约（manifest/worker）  │
│ src/ (Tauri 前端外壳，迁移期双轨)                                     │
└───────────▲──────────────── MCP 工具（限定名 app__tool）──────────┘
            │ manifest + worker/native 进程 + grant
┌───────────┴──────────── Apps：可执行应用包 ──────────────────────┐
│ apps/assets   (org.oviraptor.assets)   内置预装                     │
│ apps/scanning (org.oviraptor.scanning) = Nest 薄入口，内置预装       │
│ examples/echo-app、未来第三方应用        可安装                      │
│   └─ 内核在 Rust：src-tauri/src/{assets,scanning,agent_runtime/...}  │
└──────────────────────────────────────────────────────────────────┘
┌──────────────────────── Skills：只读教学包 ──────────────────────┐
│ skills-library：指令/参考资料，经 skills_list/skills_read 只读暴露    │
│ 可被应用 manifest 的 paired_skills 引用，但永不携带可执行能力          │
└──────────────────────────────────────────────────────────────────┘
```

### A.2 职责边界

| 层 | 能做 | 不能做 |
|---|---|---|
| Host | 聊天与会话编排；向运行时供给工具（已授权应用工具 + skills 工具）；安装/卸载/启停/升级应用；完整性校验；授权(grant)管理；导航/handoff；Skill 库导入/选择；内置预装 | 直接 import 应用内核代码；包含扫描/探测/漏洞/暴露面枚举实现；绕过 grant 调用应用工具；给 Skill 内容赋予权限 |
| App | 在自身进程内执行业务；通过 manifest 声明 pages/tools/actions/permissions；访问自身数据；声明 paired_skills | 越过 Host 直接与运行时/其他应用通信（跨应用只能 handoff）；在未授权时被调用；修改 Host 存储 |
| Skill | 提供指令、参考文档，教 agent 如何组合已有工具 | 执行代码、授予工具、授予项目访问、携带二进制或安装应用 |

## B. 为什么拆：给 Claude 的安全边界

动机：整仓交给 Claude 时，Nest/Asset 中的安全扫描与攻击面文字会触发安全边界。拆分后 Claude 只负责 agent 平台本身，不接触 Nest/Asset 内核。

### B.1 Host 工作区不得包含
漏洞利用、攻击载荷、扫描/探测细节、暴露面枚举实现、同质主管安全流水线实现细节、以上内容的示例或测试夹具。

### B.2 黑名单（Claude 禁止打开/修改，按 AF 分支实读）

```
apps/assets/**
apps/scanning/**
src/features/assets/**            (含 exposure/)
src/features/sentinel/**
src/features/hackerone/**
src-tauri/src/agent_runtime/**    (含 multi_agent/, homogeneous/, deleted_scan_audit/, evidence_graph/)
src-tauri/src/assets/**
src-tauri/src/scanning/** 与 src-tauri/src/scanning.rs
src-tauri/src/native_pipeline/**
src-tauri/src/tool_supply/** 与 tool_supply.rs
src-tauri/src/hackerone/**
src-tauri/src/recovery_finance/** 与 recovery_finance.rs
src-tauri/src/execution_service/**
src-tauri/src/page_isolation*  asset_log*  legacy_app_bridge.rs  db/service_custody.rs
src-tauri/src/commands/**         (平铺 ~780 文件，含 exposure*/ownership*/multi_agent_*/native_source_*/agent_*)
src-tauri/src/bin/oviraptor-assets-service.rs  oviraptor-scan-service.rs
src-tauri/src/db*.rs              (共享库含 scan/asset 表)
docs/NEST_*  docs/agent-first-20261016/**  以及任何 *exposure* / *scan* / *pentest* 文档
```

### B.3 白名单（Claude 可读写）

```
packages/agent-host/**
packages/agent-ui/**
packages/runtime-acp/**
packages/skills-library/**        (仅平台代码与非安全类 skill)
packages/app-sdk/**
packages/native-app-sdk/**        (仅 SDK 契约，不含具体应用)
examples/echo-app/**  以及新增假应用夹具 examples/fixture-*/**
tools/build_app_packages.mjs  tools/verify_agent_first_build.mjs  tools/test_skills_library_resources.cjs
src/plugins/**  src/shell/**  src/features/conversations/**  src/features/skills/**  src/App.vue (仅导航迁移)
docs/AGENT_PLATFORM_* docs/CLAUDE_AGENT_PLATFORM_*
```
测试中涉及应用时，**只使用假应用夹具**（echo-app 或新 fixture），不加载 apps/assets、apps/scanning。

## C. 基于 AF 分支的落地增量

1. **内置预装清单**
   - 文件：`packages/agent-host/bundled-apps.json`：`{"schemaVersion":1,"apps":[{"id":"org.oviraptor.assets","package":"<dist>/apps/org.oviraptor.assets-<ver>","enable":true,"builtin":true,"uninstallable":false},{"id":"org.oviraptor.scanning",...}]}`。
   - 首启：`AgentHost` 启动后读取清单 → 对未安装项调用 `AppManager.install` → `setEnabled(true)`；`applications` 表增加 `builtin` 列。已安装且版本更高则走 `upgrades.mjs`。
   - 状态落盘：`host_settings` 键 `bundled_apps_state`（每项 installed/enabled/error code）。
   - 可观测：`GET /api/apps/bundled` 返回每项状态；失败发出 `application.bundled_failed` 事件，不阻塞宿主启动；`verify_agent_first_build` 断言。
   - 用户可停用；`uninstallable:false` 的卸载返回 `host_app_builtin`。
2. **Nest 薄服务入口**：`apps/scanning` 已有 v1 manifest（draft_create/draft_cancel/journal_read/legacy_saved_scans）与 native-manifest。由**非 Claude 的维护者**仿 `apps/assets` 补 v2 manifest 的「建任务 / 读状态 / 读日志」三个表面，入口转发现有 Rust API（`src-tauri/src/bin/oviraptor-scan-service.rs` + 新增 `agent_runtime` 的 service_bootstrap），不重写多智能体内核。manifest 中工具描述只写业务语义，不写探测细节。
3. **Host 接入新宿主**：Tauri 以 sidecar 运行 agent-host；前端导航由 `src/plugins/catalog.ts` 的 `BUILTIN_PLUGINS`（被 `src/App.vue`、`src/shell/moduleCatalog.ts`、`src/features/conversations/ConversationPluginPanel.vue`、`src/plugins/preferences.ts` 引用）改为读取 `/api/apps`。双轨期间加开关 `OVIRAPTOR_APP_HOST=agent-host|legacy`。完成定义：上述 4 处不再引用 `BUILTIN_PLUGINS`，`catalog.ts` 删除，桌面构建中 Asset/Nest 作为应用出现且可启停。
4. **应用 ↔ Skill**：manifest 可选字段 `paired_skills: [{ name, source: "bundled"|"package-relative path", required:false }]`。app-sdk 校验：只能指向 Skill 目录（markdown/参考资料），禁止可执行引用。安装应用时返回 Skill 预览，用户确认后由 skills-library 导入；卸载应用不删 Skill。
5. **Claude 工作区形态：选 (b) 同仓白名单**。理由：现有 monorepo，宿主与 SDK 共用构建脚本，拆子仓成本高。约束：Claude 任务提示必须列出 B.2/B.3；改动前后用 `git diff --name-only` 自检，不在白名单的文件改动即失败。后续若需更强隔离再升级到 (a)。

## D. 分阶段与验收

**Phase 0 基线**
- [ ] AF 分支 merge/rebase 最新 main，无冲突遗留
- [ ] `node tools/build_app_packages.mjs` 通过
- [ ] `node tools/verify_agent_first_build.mjs` 通过
- [ ] `packages/agent-host/test`、`packages/skills-library` 测试通过，结果记入 docs

**Phase 1 内置预装 + 导航走应用宿主**
- [ ] bundled-apps.json 生效，首启 `/api/apps` 含 assets + scanning 且 enabled
- [ ] 失败项在 `/api/apps/bundled` 可见，宿主仍可聊天
- [ ] 停用→启用后应用数据保留
- [ ] C.3 完成定义达成（或开关默认 agent-host）

**Phase 2 Nest 薄服务入口（非 Claude）**
- [ ] scanning v2 manifest 暴露建任务/读状态/读日志
- [ ] `tools/test_rust_business_boundaries.cjs` 增加规则：commands 不直连 multi_agent
- [ ] mock 模型下无头跑通一次任务并读到 journal

**Phase 3 Claude 在白名单打磨 agent**（测试全部用假应用夹具）
- [ ] 工具发现：每轮 prompt 前注入「已安装且已授权应用 + 工具摘要 + 已选 Skill 摘要」；`tools/list_changed` 后下一轮可见新工具
- [ ] 按 Skill 调用：存在 paired skill 时 agent 先 `skills_read` 再调用对应应用工具（夹具剧本断言调用顺序）
- [ ] 失败恢复：mcp-bridge / AgentHost.mcp 返回结构化错误（`host_tool_permission_denied`、`host_grant_revision_conflict`、`host_tool_authority_changed`、超时），区分可重试与不可重试；可重试的做有界重试（≤2 次，带退避）
- [ ] 不越权：未授权/已停用应用的工具不出现在 tools/list，直接调用返回 403 且不重试，而是提示用户授权
- [ ] 多步计划：多工具任务产生可见的步骤计划事件（`runtime.plan`）并逐步回报
- [ ] 会话记忆：长对话的摘要/固定事实在会话重载后仍可用（host 侧存储，不依赖运行时缓存）

**Phase 4（可选）进程隔离**
- [ ] Nest/Asset 仅以原生服务进程运行，Host 进程不链接其 crate 模块
- [ ] Host 构建产物中不含黑名单源码

## E. 明确不做
- 不要求 Claude 修改 Nest 同质主管、多智能体内核、暴露面/扫描实现。
- 不把应用可执行逻辑塞进 Skill。
- 不要求平权多智能体；Nest 维持主管模式，Host agent 与 Nest 主管是「调用方—应用」关系。
