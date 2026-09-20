# Oviraptor 架构边界

本项目采用“稳定门面 + 业务 Feature”的组织方式。调用方只依赖稳定入口，新增逻辑进入所属业务域，避免再次形成跨场景巨型文件。

## Rust / Tauri

`src-tauri/src/commands.rs` 只保留公共依赖、少量跨域基础函数和业务文件装配。各实现文件通过 `include!` 进入同一个 Rust 模块，因此 Tauri 命令名、`commands::command_name` 路径和既有私有函数关系保持不变。

| 文件 | 业务职责 |
| --- | --- |
| `commands/workspace_projects.rs` | 仪表盘、工作空间、应用设置、配置方案 |
| `commands/assets.rs` | 目标导入、资产查询、人工结论、运行与事件 |
| `commands/hackerone.rs` | HackerOne 项目、事件、Scope 同步 |
| `commands/environment.rs` | 本机依赖检测、安装与 Strix 升级 |
| `commands/scan_lifecycle.rs` | 扫描创建、重试、尝试账本、Trace 查询 |
| `commands/knowledge_learning.rs` | 知识、学习候选、Skill 沉淀与来源提炼 |
| `commands/rule_packs.rs` | 安全规则包管理与同步 |
| `commands/runtime_config.rs` | Skill 注入、预算、执行器解析与自适应配置 |
| `commands/frontend_recon.rs` | 前端证据压缩、路由评分、AI 证据包 |
| `commands/runtime_environment.rs` | LLM/FOFA 连接配置与运行环境变量 |
| `commands/scan_execution.rs` | 前端生产者、Strix 执行、Token 与熔断控制 |
| `commands/agent_contract.rs` | 运行时契约在 `commands` 内的兼容层：`AgentTargetOutcome`、停止语义、遗留检查点 |
| `commands/agent_runtime.rs` | `AgentExecutionPlan` 软硬预算、覆盖族与计划持久化 |
| `commands/agent_model_client.rs` | 模型网关适配层：把现有 Profile 解析结果接到 `agent_runtime::model` |
| `commands/agent_tools.rs` | 原生工具注册表与本地参数校验（作用域、身份隔离、契约与清理） |
| `commands/agent_native.rs` | 原生 Agent 循环、覆盖收口与结果入库 |
| `commands/agent_backend.rs` | 后端选择、Strix 适配器、单一终态聚合与“执行计划”查询命令 |
| `commands/asset_ownership.rs` | 资产归属域名判定与隔离规则 |
| `commands/exposure.rs` | 暴露面视图与运行记录 |
| `commands/src_assurance.rs` | SRC 专项能力清单与内置 OAST 接收器 |
| `commands/web_investigation_policy.rs` | Web 调查策略、契约/发现/验证轮次上限 |
| `commands/strix_compatibility.rs` | Strix CLI 能力探测与兼容参数 |
| `commands/native_*.rs` | 内置采集、归并、分层、探测、Raw HTTP 等原生 Worker |
| `commands/code_analysis.rs` | 源码库存、代码/灰盒/CI 扫描准备 |
| `commands/scan_control.rs` | 启动、确认、暂停、恢复、删除和结果查询 |
| `commands/appsec_validation.rs` | 机会、漏洞关联、验证、熔断处置与导入导出 |
| `commands/investigation.rs` | 调查图谱、动作/API/假设协议、信息增益、增量基线、多身份差异与三层知识 |
| `commands/result_ingestion.rs` | Strix 产物解析、同步、修复与前端结果入库 |
| `commands/asset_export.rs` | 资产 CSV 导出 |
| `commands/tests.rs` | 跨业务行为回归测试 |

浏览器认证会话位于 `src-tauri/src/auth_session.rs`；数据库迁移、模型、任务 Worker 和 LLM Hook 分别保持在自己的顶层模块。

## Vue / TypeScript

`src/api.ts` 是兼容门面，只合并 Feature API。新增 Tauri 调用必须写入对应业务目录：

- `src/features/workspaces/api.ts`：工作空间、设置与配置方案；
- `src/features/assets/api.ts`：资产、任务、日志与导出；
- `src/features/hackerone/api.ts`：HackerOne；
- `src/features/sentinel/api.ts`：Strix、证据、知识、验证与熔断；
- `src/features/runtime/api.ts`：环境、升级与远程 Worker。

Sentinel 的纯展示和解释规则位于 `src/features/sentinel/presentation.ts`。它只负责确定性格式化、标签映射和记录降噪，不发请求、不修改状态。页面级异步动作继续由组件或后续 composable 编排。

调查图谱视图位于 `src/features/sentinel/components/InvestigationGraphPanel.vue`。它只消费 `InvestigationGraph`，展示页面→动作→API→假设因果链、增量指标、身份差异与验证契约；数据加载和状态更新仍由 `SentinelBoard.vue` 编排。

## Agent 后端边界

`agentBackendPolicy`（`auto | native | strix`）保存在配置方案的 settings JSON 中，只在任务入口生效。迁移期 `auto` 与缺省值都选择 Strix；原生后端只能在配置里显式选 `native` 且目标为 URL 集合时启用（`auto` 改成原生默认属于 §17 Phase 7）。`src-tauri/src/commands/agent_backend.rs::agent_select_backend` 是唯一选择函数，Asset 创建任务与 Strix 工作台 URL 任务都调用它，并由 `agent_web_pipeline_runtime` 共用同一套预算、代理与 Worker 解析。已开始的 attempt 以 `sentinel_checkpoints.stage='agent_execution_plan'` 里记录的 `backend` 为准，改配置不会切换正在执行的 attempt。

原生 attempt 永不回退到 Strix 重跑（§19）：`run_agent_target` 选定后端后只调用该后端，避免重试时 Native 与 Strix 同时消耗 Token。

原生后端的结果只写统一表：请求响应证据 `stage='native-agent-evidence'`、覆盖账本 `stage='native-agent-coverage'`、漏洞 `stage='native-agent'` 且 `kind='vulnerability'`，运行中状态检查点 `stage='native_agent_state'`；不生成 Strix 产物文件。终态文案只能来自 `record_agent_target_outcome` 与 `AgentPipelineTally::finalize`。

## Agent Runtime（`src-tauri/src/agent_runtime/`）

| 文件 | 职责 |
| --- | --- |
| `contract.rs` | 规范枚举与类型：后端、角色、副作用类、`AgentExecutionPlan`、覆盖族、预算包、`TerminalState`、事件与消息种类 |
| `store.rs` | `agent_runs`/`agent_events`/`agent_messages`/`tool_invocations`/`agent_snapshots` 仓储、内容寻址 artifact、稳定哈希 |
| `reducer.rs` | 单写终态 reducer；已终态的 run 不接受第二次写入 |
| `checkpoint.rs` | 快照与事件重放、租约过期中断、预算不重复计费 |
| `secrets.rs` | Cookie/JWT/API Key 脱敏，写库与入 prompt 前必经 |
| `model/` | `ModelGateway`、OpenAI 兼容实现、Profile 输入类型、四层上下文与 usage |
| `strix_adapter.rs` | Strix 兼容适配器：只上报事实，不写旧状态列 |

Phase 0/1 只写入新表，`sentinel_targets`/`sentinel_scans` 的写法保持不变；Phase 3 才把编排器接到这张事件表上。

## Web 调查数据流

1. Chromium 运行时按身份捕获页面状态、可安全触发动作、实际请求/响应头和请求因果；AST/JSLuice/字符串证据补全尚未运行到的 API 与参数。
2. `result_ingestion.rs` 保留兼容 Findings/Opportunity，同时调用 `investigation.rs` 幂等重建当前 URL 的图谱。
3. 图谱计算 API/参数签名与上次身份基线的差异，产出 `investigation_metrics` 和本地 `modelGate`。
4. `frontend_recon.rs` 只把通过门禁的假设、验证契约和紧凑因果证据写入 `frontend-evidence.json`；完整原始结果仍只留本机。
5. `scan_execution.rs` 在启动 Strix 前再次执行门禁：复杂前端/重复目标无新证据即停止；普通 Web 首次只允许一次非递归兜底发现；确认 WAF 立即熔断。
6. 事实、策略、结果分别进入 `knowledge_facts`、`knowledge_strategies`、`knowledge_outcomes`；策略需要独立支持或已确认结果才晋升。

## 新代码规则

1. 新 Tauri 命令放入最接近的业务文件，并在 `src-tauri/src/lib.rs` 注册；不要把实现重新写回 `commands.rs`。
2. 新前端命令调用放入 Feature API；页面继续通过 `api` 门面访问，避免组件直接散落 `invoke`。
3. 纯格式化、分类和展示门禁进入 `presentation.ts`；带网络或数据库副作用的代码不得进入展示模块。
4. 独立业务视图优先新增组件；复用状态流程优先新增 composable。不要仅为降低行数机械拆分强耦合代码。
5. 新后端能力必须同时经 `agent_backend.rs` 的选择与聚合函数接入，禁止在任何入口直接调用 `run_adaptive_strix_target`。
6. 后端变更至少运行 `cargo test`，前端变更至少运行 `npm run build`；涉及前端侦察时同时运行 Python 与 Node Worker 测试/语法检查。
