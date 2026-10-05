# Oviraptor 架构边界

本项目采用“稳定门面 + 业务 Feature”的组织方式。调用方只依赖稳定入口，新增逻辑进入所属业务域，避免再次形成跨场景巨型文件。

## Rust / Tauri

`src-tauri/src/commands.rs` 只保留公共依赖、少量跨域基础函数和业务文件装配。各实现文件通过 `include!` 进入同一个 Rust 模块，因此 Tauri 命令名、`commands::command_name` 路径和既有私有函数关系保持不变。

| 文件 | 业务职责 |
| --- | --- |
| `commands/workspace_projects.rs` | 仪表盘、工作空间、应用设置、配置方案 |
| `commands/assets.rs` | 目标导入、资产查询、人工结论、运行与事件 |
| `commands/hackerone.rs` | HackerOne 项目、事件、Scope 同步 |
| `commands/environment.rs` | 本机依赖与沙箱适配器状态 |
| `commands/scan_lifecycle.rs` | 扫描创建、重试、尝试账本、Trace 查询 |
| `commands/trace_display_privacy.rs` | Trace 消息预览与详情返回前的只读隐私投影；提取正文前检查外层隐私标记，明确标记的私有推理不展示，不修改历史证据或运行时输入 |
| `commands/knowledge_learning.rs` | 知识、学习候选、Skill 沉淀与来源提炼 |
| `commands/rule_packs.rs` | 安全规则包管理与同步 |
| `commands/runtime_config.rs` | Skill 注入、预算、执行器解析与自适应配置 |
| `commands/frontend_recon.rs` | 前端证据压缩、路由评分、AI 证据包 |
| `commands/runtime_environment.rs` | LLM/FOFA 连接配置与运行环境变量 |
| `commands/scan_execution*.rs` | Web 执行编排、前端生产者、运行时配置、Token 与中止控制 |
| `commands/agent_contract.rs` | 运行时契约在 `commands` 内的兼容层：`AgentTargetOutcome`、停止语义、遗留检查点 |
| `commands/agent_runtime.rs` | `AgentExecutionPlan` 软硬预算、覆盖族与计划持久化 |
| `commands/agent_model_client.rs` | 模型网关适配层：把现有 Profile 解析结果接到 `agent_runtime::model` |
| `commands/agent_tools.rs` | 原生工具注册表与本地参数校验（作用域、身份隔离、契约与清理） |
| `commands/agent_native.rs` | 原生 Agent 循环、覆盖收口与结果入库 |
| `commands/agent_backend.rs` | Native 后端冻结计划、旧后端拒绝继续执行、单一终态聚合与执行计划查询 |
| `commands/asset_ownership.rs` | 资产归属域名判定与隔离规则 |
| `commands/exposure.rs` | 暴露面视图与运行记录 |
| `commands/src_assurance.rs` | SRC 专项能力清单与内置 OAST 接收器 |
| `commands/web_investigation_policy.rs` | Web 调查策略、契约/发现/验证轮次上限 |
| `commands/native_*.rs` | 内置采集、归并、分层、探测、Raw HTTP 等原生 Worker |
| `commands/source_inventory.rs` | 源码树统计与框架识别；只遍历选中目录内的普通文件，不跟随符号链接 |
| `commands/code_analysis.rs` | 代码/灰盒/CI 工作台扫描准备与 Native 分支派发 |
| `commands/scan_control.rs` | 启动、确认、暂停、恢复、删除和结果查询 |
| `commands/appsec_validation.rs` | 机会、漏洞关联、验证、熔断处置与导入导出 |
| `commands/investigation.rs` | 调查图谱、动作/API/假设协议、信息增益、增量基线、多身份差异与三层知识 |
| `commands/result_ingestion*.rs` | 历史 JSON 兼容读取、结果归并与前端结果入库 |
| `commands/artifact_import_status.rs` | 历史导入报告和状态；不承担列表与预览查询 |
| `commands/historical_import_views.rs` | 历史任务目录、任务预览和包预览的只读模型与查询；校验 canonical schema、声明类型及来源关系，不授予执行权限 |
| `commands/asset_export.rs` | 资产 CSV 导出 |
| `commands/tests.rs` | 跨业务行为回归测试 |

浏览器认证会话位于 `src-tauri/src/auth_session.rs`；数据库迁移、模型、任务 Worker 和 LLM Hook 分别保持在自己的顶层模块。

## Vue / TypeScript

`src/api.ts` 是兼容门面，只合并 Feature API。新增 Tauri 调用必须写入对应业务目录：

- `src/features/workspaces/api.ts`：工作空间、设置与配置方案；
- `src/features/assets/api.ts`：资产、任务、日志与导出；
- `src/features/hackerone/api.ts`：HackerOne；
- `src/features/sentinel/api.ts`：任务、证据、知识、验证与熔断；
- `src/features/runtime/api.ts`：运行环境与远程 Worker。

Nest 工作台的业务状态放在 `src/features/sentinel/workbench/`：`useBrowserAuthSessions.ts` 管身份隔离与捕获，`useFollowupRecovery.ts` 管补证提交找回与显式释放，`useClosureHandoff.ts` 管结案后独立任务交接，`useWorkbenchSkills.ts` 管 Skill 目录和编辑，`useWorkbenchTaskCreation.ts` 管任务创建、源码 USD 准入提示及迟到回执隔离，`useWorkbenchPresentation.ts` 管模式文案、Web 预设与源码目录选择。灰盒、CI/CD、Skills、任务侧栏、登录身份、补证/结案提示、预算输入及 Web 生效策略各在同业务目录的 `Workbench*` 子组件；`src/components/AgentWorkbench.vue` 约 386 行，只保留统一表单和模式路由。按业务职责继续渐进整理存量大文件；这一局部边界达标不代表整体 UI 或 Master Plan 已验收。

Sentinel 的纯展示和解释规则位于 `src/features/sentinel/presentation.ts`。它只负责确定性格式化、标签映射和记录降噪，不发请求、不修改状态。页面级异步动作继续由组件或后续 composable 编排。

调查图谱视图位于 `src/features/sentinel/components/InvestigationGraphPanel.vue`。它只消费 `InvestigationGraph`，展示页面→动作→API→假设因果链、增量指标、身份差异与验证契约；数据加载和状态更新仍由 `SentinelBoard.vue` 编排。

轨迹详情显示合同放在 `src/features/sentinel/traces/traceDetailContract.ts`，由 Board 与独立轨迹页共同核对回包任务身份和字段形状。Board 的只读轨迹状态由同目录 `useTaskTrace.ts` 管理，独立轨迹页保留 `useTraceSelection.ts` 的选择及轮询生命周期；不要把不同视图的生命周期强行合并，也不要将轨迹专用合同上移为全局工具。前端合同不能替代后端授权、脱敏或来源证明。

Board 的轨迹展示由 `components/results/SentinelTraceTimeline.vue` 负责：只消费已校验的详情与父级读取状态，维护目标过滤、最近 30 条记录及来源/统计范围文案，不请求接口、不启动轮询、不执行任务。历史事件是记录而不是当前活动证明；不能依据任务状态显示未经验证的 LIVE，或把工具返回事件解释为模型正在执行后续动作。统计属于任务级累计，不能冒充当前目标统计。

## Agent 后端边界

新任务只选择 Native；旧配置中的后端字段与历史 JSON 仅用于兼容读取。`src-tauri/src/commands/agent_backend.rs::agent_select_backend` 是统一选择入口，`agent_web_pipeline_runtime` 复用预算、代理与 Worker 解析。既有 attempt 的冻结矩阵和继续执行的父 attempt 仍须保持原后端身份；如果冻结为已停用后端，明确拒绝继续执行，不静默转换为 Native。该边界是代码约束，不代表完整 Master Plan 已验收。

原生 attempt 不回退到旧后端重跑：`run_agent_target` 对冻结的已停用后端明确拒绝，避免隐式重试消耗或改变历史事实。

原生后端的结果写统一表：请求响应证据 `stage='native-agent-evidence'`、覆盖账本 `stage='native-agent-coverage'`、漏洞 `stage='native-agent'` 且 `kind='vulnerability'`，运行中状态检查点 `stage='native_agent_state'`；不生成旧后端产物文件。终态文案只能来自 `record_agent_target_outcome` 与 `AgentPipelineTally::finalize`。

## Agent Runtime（`src-tauri/src/agent_runtime/`）

| 文件 | 职责 |
| --- | --- |
| `contract.rs` | 规范枚举与类型：后端、角色、副作用类、`AgentExecutionPlan`、覆盖族、预算包、`TerminalState`、事件与消息种类 |
| `store.rs` | `agent_runs`/`agent_events`/`agent_messages`/`tool_invocations`/`agent_snapshots` 仓储、内容寻址 artifact、稳定哈希 |
| `reducer.rs` | 单写终态 reducer；已终态的 run 不接受第二次写入 |
| `checkpoint.rs` | 快照与事件重放、租约过期中断、预算不重复计费 |
| `secrets.rs` | Cookie/JWT/API Key 脱敏，写库与入 prompt 前必经 |
| `model/` | `ModelGateway`、OpenAI 兼容实现、Profile 输入类型、四层上下文与 usage |

历史 JSON 的可读性和新任务执行权限分离；旧结果不得作为恢复旧执行器的理由。

## Web 调查数据流

1. Chromium 运行时按身份捕获页面状态、可安全触发动作、实际请求/响应头和请求因果；AST/JSLuice/字符串证据补全尚未运行到的 API 与参数。
2. `result_ingestion*.rs` 保留历史 Findings/Opportunity 兼容读取，同时调用调查模块归并当前 URL 的图谱。
3. 图谱计算 API/参数签名与上次身份基线的差异，产出 `investigation_metrics` 和本地 `modelGate`。
4. `frontend_recon.rs` 只把通过门禁的假设、验证契约和紧凑因果证据写入 `frontend-evidence.json`；完整原始结果仍只留本机。
5. `scan_execution*.rs` 和 Native 调度执行授权、预算、任务中止与证据边界；旧后端冻结计划拒绝继续执行。具体扫描是否满足覆盖要求必须以实际任务回执检验。
6. 事实、策略、结果分别进入 `knowledge_facts`、`knowledge_strategies`、`knowledge_outcomes`；策略需要独立支持或已确认结果才晋升。

## 新代码规则

1. 新 Tauri 命令放入最接近的业务文件，并在 `src-tauri/src/lib.rs` 注册；不要把实现重新写回 `commands.rs`。
2. 新前端命令调用放入 Feature API；页面继续通过 `api` 门面访问，避免组件直接散落 `invoke`。
3. 纯格式化、分类和展示门禁进入 `presentation.ts`；带网络或数据库副作用的代码不得进入展示模块。
4. 独立业务视图优先新增组件；复用状态流程优先新增 composable。不要仅为降低行数机械拆分强耦合代码。
5. 新后端能力必须经过统一选择、冻结计划和终态聚合函数；禁止从其他入口重新接入已停用后端。
6. 后端变更至少运行 `cargo test`，前端变更至少运行 `npm run build`；涉及前端侦察时同时运行 Python 与 Node Worker 测试/语法检查。
7. 前后端代码按业务边界分层，公共契约、序列化、错误语义只在真正跨业务复用时进入公共模块。新组件/模块以 200–300 行为体量目标；超过约 400 行先检查职责，超过 600 行优先拆解，不以机械分割代替清晰接口。测试按场景拆文件、共用装载夹具；清除过时重复用例，但不能删除唯一的授权、幂等、恢复或防越界回归来压缩体量。
