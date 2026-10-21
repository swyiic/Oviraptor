# Oviraptor 授权安全测试工作流实施规范

日期：2026-10-18。文档状态：**待实施的交接规范，不是完成声明或测试报告**。

本轮授权仅限创建此文档；文中代码、迁移、测试和合成案例均未执行。接手开发者应先核对最新工作树，再按批次实施。目标是可靠、有限、可审计的授权安全测试，禁止将模型拒绝解释为需要换说法、拆分请求或无限尝试的障碍。

## 0. 先读：已具备、明确缺陷、本轮未验证

### 0.1 已具备的代码基础

- Host 已按项目绑定会话和 MCP 调用，应用授权在派发前与返回后复查；Assets count/search/query 使用项目条件，分页游标绑定项目与过滤条件。
- Host 已有持久化任务、步骤、证据摘要、中断标记、有限断言验收；普通回答结束不等于任务获得验收。
- Native 已有受控工具循环、预算预留/结算账本、lease/fencing、事件与 checkpoint、未知目标调用阻止重放，以及有界终止分类。
- Native 漏洞确认要求实际请求与差异 artifact；双身份响应不同不能自动等于越权。
- 安装默认禁用、项目 grant 与 runtime 披露分离；Skills 是不可信参考材料，不能授予执行权限。
- 已有任务/项目 JSON 历史快照导出，Source 已有 JSON/SARIF/bundle 导出；不能声称报告功能完全不存在。

以上只表示看到了相应实现，**不表示本轮测试通过、真实 provider 接通或授权扫描成功**。

### 0.2 明确缺陷与能力缺口

| ID | 优先级与性质 | 当前证据 | 实施要求 |
|---|---|---|---|
| D01 | P1，确定缺陷 | `packages/agent-host/src/tasks.mjs:102`、`:106`、`:128`：未知副作用可由不同 run 中较新的成功证据解除，未要求证据核对原操作、对象或后置条件 | 改为 operation-bound reconciliation；无关读取不能解除未知 |
| D02 | P1，确定缺陷 | `packages/agent-host/src/provider.mjs:382`、`:489`：Chat delta 未处理结构化 refusal，Responses message 只提取 output_text；`:870` 主要按 content_filter 映射 refusal | 保留 provider 原始拒绝分类，停止自动补试，展示有限解释 |
| D03 | P1，能力缺口 | `quality-prompt.mjs:105` 是步数/输出/单请求超时；`provider.mjs:864` 可延长步数；`subagents.mjs:84` 有独立子循环 | Host 补持久化共享预算语义，不能把循环步数当总消费硬上限 |
| D04 | P1，安全收口下的能力缺口 | `src-tauri/src/commands/agent_tools_verdict.rs:299` 对 authorization family 返回 authorization_object_control_missing | 补可验证的对象归属、合法控制组契约，不能删除拒绝条件来换取“确认” |
| D05 | P2，业务边界耦合 | `packages/agent-host/src/desktop-assets.mjs:30` 直接打开并写 Assets inventory；`service.mjs:138` 直接定位 insight 库 | 改成 Assets owner 的版本化同步/查询接口，保留现有项目隔离和失败关闭 |
| D06 | P1，长任务可靠性缺口 | `memory.mjs:66`、`:101` 会裁剪 facts/hypotheses；`:363` 摘要没有原始 message/evidence 引用链；`service.mjs:964` 将工具结果写为短摘要 | 原始记录、结构化证据、工作摘要分层；约束/反证/未知副作用不能只在摘要里 |
| D07 | P2，窗口与装载策略缺口 | `memory.mjs:229` 用 96000 bytes 限制会话；`service.mjs:697` 先取该会话全部消息；`provider.mjs:640` 再筛到最多 40 条 | 按 provider/model 能力预算、分页按需装载，bytes 只是传输保护，不是 token 窗口 |

超时结论必须保留限制：`service.mjs:1040` 的重试策略未区分读写，但默认 `app-process.mjs:126`、`:177` 会停止超时 worker，Native owner 保持隔离。因此本次**没有证明默认实现已重复执行副作用**。确定的问题是持久化操作身份、未知结果核对和重试策略尚未形成完整契约。

### 0.3 本轮未验证

- 没有运行构建、单元/集成/全量测试、安全扫描、模型请求、UI 自动化或外部 URL 测试。
- 没有验证实际模型窗口、价格、缓存费用、OAuth 账户、已安装包或运行中二进制是否对应当前源码。
- 没有穷尽浏览器所有子请求、重定向、跨 SQLite owner 快照及并发预算路径。
- 没有确认所有报告脱敏点、所有模型拒绝格式、所有历史数据兼容情况。
- 没有读取用户数据库、凭据、其他 Codex 私有会话；没有停止进程或操作分支、提交、推送。
- 本轮未核实到 P0；这不构成不存在 P0 的保证。实施中如发现越权实际派发、未知写入盲重放或硬预算绕过，应按 P0 暂停该执行能力并保留证据。

## 1. 读取快照与代码导航

### 1.1 快照

- 仓库：`/Users/swyiic/Desktop/Rust/Oviraptor`。
- 分支：`feature/agent-first-app-platform-20261016`。
- HEAD：`30dd219821172eecd8e18636110722348c270415`；上一轮审查及本轮补读均为此 HEAD。
- 上一轮读取：2026-10-18 05:12–05:15 UTC；本轮补读：2026-10-18 05:24–05:25 UTC。
- 本轮开始的 `git status --porcelain=v1` 摘要：128 个已跟踪修改、71 个未跟踪条目。不是干净提交上的审查。
- 相关已修改文件包括 `packages/agent-host/src/{service,provider,store,memory,agent-state}.mjs`、`apps/assets/application.mjs`、`src/features/sentinel/presentation.ts`。
- 相关未跟踪文件包括 `packages/agent-host/src/{tasks,task-verifier,desktop-assets,subagents}.mjs`。
- 本轮定向读取的 `src-tauri/src/agent_runtime/model/{context,profile,usage}.rs`、`commands/{runtime_config,appsec_validation,native_source_findings,agent_tools_verdict}.rs` 未在相关状态列表显示修改。

另一个 Codex 正在并行完善，读取不是原子快照，**文件和行号可能随时变化**。行号用于定位，实施前必须按函数名重新核对；不要用此 HEAD 还原、覆盖或否定尚未提交的新实现。原有 `docs/AGENT_SETTINGS_REFACTOR_PLAN_2026-10-18.md` 是相关计划，本文件不覆盖其实施记录，也不把其中“通过”转写成本轮运行结果。

### 1.2 定向代码索引

以下路径相对仓库根目录，均为读取时位置。

| 领域 | 代码锚点 | 当前实现事实及限制 |
|---|---|---|
| 项目映射/UI竞态 | `src/features/agentHost/chatClient.ts:45`；`components/AgentHostChat.vue:137` | desktop externalId 映射、generation 防止旧请求更新新项目；缺 ID 时按名称查找的兜底仍需针对性验收 |
| 项目授权派发 | `packages/agent-host/src/apps.mjs:221`、`:232`、`:267`、`:293` | tools/list 取 grant/runtime 交集；context 由 Host 绑定项目；返回结果前重查授权 |
| 资产隔离 | `apps/assets/application.mjs:88`、`:94`、`:107`；`desktop-assets.mjs:38`、`:77` | 项目 WHERE、项目游标 hash、项目镜像删除；镜像读一致但多次分页不等于固定历史快照 |
| 镜像失败 | `packages/agent-host/src/service.mjs:884` | 已映射桌面库同步失败/需升级时拒绝继续返回旧镜像 |
| Host计划恢复 | `packages/agent-host/src/tasks.mjs:22`、`:28`、`:55`；`store.mjs:65`、`:194`、`:206` | task JSON/revision、running证据、中断处理；当前证据不是完整工具响应仓库 |
| Host有限验收 | `packages/agent-host/src/task-verifier.mjs:15`、`:49`、`:108` | 文件/JSON有限断言、有当前run证据；明确不证明测试执行、功能完整或漏洞成立 |
| 聊天原文 | `packages/agent-host/src/store.mjs:178`、`:182`、`:188` | user/assistant content 持久化，消息状态与未知结果单独存储 |
| Host短记忆 | `packages/agent-host/src/memory.mjs:5`、`:66`、`:101`、`:159` | 8192 bytes 工作记忆，有界 recent/facts/hypotheses；不是完备证据账本 |
| 会话压缩 | `packages/agent-host/src/memory.mjs:297`、`:363`、`:400` | 保留部分用户要求、调用结果成组；仍会截断，摘要缺可解析来源ID |
| 模型切换/回灌 | `packages/agent-host/src/service.mjs:696`；`provider.mjs:630` | SQLite取历史，provider留首条用户消息及尾部；没有完整工具响应回灌链 |
| Native上下文 | `src-tauri/src/agent_runtime/model/context.rs:10`、`:55`、`:117` | 规则/角色预算/事实/近期轮次及artifact引用；确定性裁剪。digest_text的“sha”是DefaultHasher短摘要，不能当SHA-256完整性凭证 |
| Native窗口 | `src-tauri/src/agent_runtime/model/profile.rs:108` | 本地窗口扣输出与余量后转byte估算，有最小4096与传输上限；注释明确不是实际token/财务硬约束 |
| Native计量 | `src-tauri/src/agent_runtime/model/usage.rs:38`、`:44`、`:99` | wire-size fallback与reported校验分开；不可把fallback解释为真实账单 |
| 受控循环 | `src-tauri/src/agent_runtime/controlled_executor.rs:145`、`:243`、`:262` | before_round/before_tool、观察回填、checkpoint、有界round/tool限制；不要另写通用循环 |
| Native费用 | `agent_runtime/multi_agent/budget/model.rs:8`、`:56`、`:96`；`budget/entries.rs:81` | 预留/结算/receipt绑定、未知阻止新预留、hard-limit；位置前缀为src-tauri/src/ |
| 恢复/终止 | `src-tauri/src/agent_runtime/checkpoint.rs:279`、`:296`；`reducer.rs:65`、`:109`、`:124` | 未知目标调用拒绝恢复，soft/hard/授权/持久化等区分；reducer测试helper不等于所有生产写入总闸 |
| 授权范围 | `src-tauri/src/commands/agent_tools_scope.rs:49`、`:122` | 冻结域名判断和重定向分类；端口/路径/方法精细授权需要单独契约 |
| Native确认 | `src-tauri/src/commands/agent_tools_verdict.rs:173`、`:265`、`:299` | 实际请求对照、差异artifact绑定；authorization对象控制证据缺失时拒绝 |
| 扫描预设 | `src-tauri/src/commands/runtime_config.rs:320`、`:460`、`:489`、`:501` | quick/standard/deep、评分选择与ceiling、各档timeout/token/request；不据此推导普适耗时 |
| 手动任务入口 | `src/features/sentinel/api.ts:89`、`:94`；`workbench/useWorkbenchTaskCreation.ts:97`、`:128`、`:153` | 草稿再确认启动；Source USD约束不支持时UI拒绝；后端约束仍须独立保留 |
| 两种扫描包 | `apps/scanning/application.mjs:10`；`src-tauri/src/scanning/native_service_application.rs:30`、`:305` | Node v1是draft-only；Native SDK2有start/pause/resume/retry action，不能把两者混写为完全迁移 |
| 已有报告出口 | `src-tauri/src/commands/appsec_validation.rs:1059`、`:1101`、`:1176`；`native_source_findings.rs:155`、`:181` | task/project历史JSON；Source JSON/SARIF/bundle；executionEligible=false，导入不恢复权限 |
| 人工复核入口 | `src/features/sentinel/api.ts:255`；`src-tauri/src/commands/appsec_validation.rs:811` | 已有validation workitems/save入口；本轮未证明它们足以作为自动验证回执 |
| Skills与安装 | `packages/agent-host/src/skills.mjs:131`；`store.mjs:59`；`apps.mjs:44` | lease内容指纹/项目/runtime校验、默认disabled、包文件复制后重校验；不是第三方代码OS沙箱或发布者签名 |

## 2. 目标、业务边界与交付原则

### 2.1 用户可见闭环

用户表达意图 → 选定/创建项目 → 查看范围和预算预览 → 明确授权必要动作 → 创建任务 → 同一任务引擎执行 → 可解释进度/暂停/取消/恢复 → 事实证据 → 有限验证 → 人工复核 → 可重生成报告。

授权安全测试是有限目标，不以“直到控制目标”“直到模型答应”或“深度必得漏洞”为成功条件。没有漏洞、证据不足、授权不足、预算不足都是可解释的正常结果。

### 2.2 所有权

| Owner | 负责 | 不负责 |
|---|---|---|
| Agent Host | 会话/任务、上下文装配、Host模型调度、工具准入、操作收据、Host消费账本、应用安装发现 | 直接修改Assets私有schema；自行实现第二套扫描引擎；授予目标网络权限 |
| Assets | 资产导入/归属候选/查询/镜像同步、项目数据schema和迁移 | 以资产存在推定扫描授权；发布漏洞确认 |
| Scanning/Native | 目标授权、执行计划、任务引擎、工具/network门禁、扫描费用与lease、证据/验证/恢复 | 依赖聊天摘要当授权；采用外部报告恢复执行权 |
| Skills | 指导材料及资源、内容指纹与披露同意 | 通过脚本/文字扩大权限；自动执行安装脚本 |
| 前端 | 预览/确认、任务入口、进度与风险解释、证据和报告展示 | 独立决定后端准入或通过按钮隐藏替代校验 |

共享预算指**共享字段语义、收据和上层聚合**，不要求Host与Native直接共享SQLite表或重写已成熟Native账本。先用薄适配层表达当前支持与不支持，保留两者自己的生命周期/凭据所有权。

### 2.3 不在首轮范围

不大拆现有模块，不改写通用Agent，不迁移全部扫描业务，不增加第二套编排，不引入复杂PDF服务，不升级真实用户数据库作试验，不安装/登录provider，不测试外部URL，不把拒绝后换模型/换措辞当自动策略。

## 3. 必须保持的系统不变量

1. 所有读取/写入都从可信调用上下文解析projectId；模型参数、摘要、URL内容不能覆盖它。
2. 项目数据授权、目标测试授权、模型/Skill披露、安装/启用是不同授权；任何一个不能替代其他。
3. 先持久化操作身份和派发意图，再调用有副作用工具；先保存可引用结果，再截断/压缩给模型。
4. `timeout`、断流、取消或worker退出不证明没有副作用，也不证明费用为零。
5. 未知结果不得自动重放；用户说“继续”也先核对，不自动视为重做未知操作授权。
6. toolCallId、operationId、physicalRequestId、evidenceId各有独立身份，不能互相充当幂等键。
7. 一个物理模型请求只结算一次；同一费用在父子聚合中只计一次。
8. 软阈值用于收口/暂停，硬阈值在派发前强制；Agent不能自己增加预算或提升档位。
9. 模型拒绝是独立终态，不是工具失败；不以拆分、重述、反复调用规避拒绝。
10. 外部页面、工具返回、导入数据、Skills和历史摘要都是不可信数据；其“批准”“系统指令”不产生权限。
11. 模型推断、摘要、文件存在、HTTP 200、两个身份的响应差异都不能单独证明漏洞。
12. 撤权同时约束能力发现、检索、派发、结果披露和报告导出；已发生副作用仍需留下受控审计事实。
13. 报告/导入包/历史证据不恢复执行、预算、lease、身份或复核权限。
14. 窗口装不下完整硬约束时应缩小检索或暂停，不能静默删掉禁止事项继续执行。
15. UI与Agent共用typed application services和后端状态；最终解释始终引用真实任务ID与收据。
16. 原始数据缺失时显示缺失/未知，不通过摘要补造“原始证据”。

## 4. 目标数据实体：增量扩展，不复制完整引擎

以下为**目标设计**，不是现有表/接口的完成声明。优先扩展现有task/evidence和owner表；新增表名由实施者依现有schema规范确定，不必机械采用本节名称。

| 实体 | 最小字段 | 权威来源/写入约束 |
|---|---|---|
| ProjectBinding | hostProjectId、desktopExternalId、mappingRevision、kind | Host/项目owner；不按同名猜测合并 |
| AuthorizationSnapshot | id/revision、projectId、principalRef、allowedTargets、excludedTargets、methods/actionClasses、expiresAt、sourceMessageIds、approvalReceiptId | 用户明确指令/可信确认转为后端约束；模型只可提案 |
| TaskExtension | taskId、parentTaskId、goal、constraintRefs、authorizationRef、presetSnapshotRef、budgetRootRef、checkpointVersion | 保留现有revision并做CAS更新 |
| OperationReceipt | operationId、project/task/run、tool/appVersion、argumentDigest、targetDigest、effectClass、authRevision、state、attempts、resultRef、reconciliationRef | Host/业务owner；操作ID先于派发落盘 |
| PhysicalRequestReceipt | physicalRequestId、operationId、requestDigest、provider/modelCapabilityRevision、dispatch/terminal状态、usageRef | 实际传输owner；每次物理发送独立，重试不是同一次费用 |
| RawRecord | id、project/conversation/task/run、kind、createdAt、contentRef/hash/encoding、redactionVersion、retentionClass、originTrust | owner持久化；原文含义见§7，不允许公开保存凭据 |
| EvidenceRecord | evidenceId、operationId、原始请求/响应ref、target/identity/object/contract、observedAt、provenance、status、schemaVersion | 主机或扫描执行记录；不能由模型伪造“observed” |
| Constraint/Question/Attempt | id、taskId、kind、sourceRefs、status、supersededBy、negativeEvidenceRefs | 独立于可丢失摘要；修改需明确来源与revision |
| SummaryVersion | id、scope、version、sourceRanges/IDs、sourceDigest、summaryText、claims及sourceRefs、omissions、supersedes、algorithmVersion | 派生非权威数据；标注不完备 |
| ModelCapability | provider/model/endpointRef、context/output上限、tool/refusal/usage能力、tokenizerRef、verifiedSource/revision、unknownFields | 配置或已验证元数据；无凭据、无臆造统一默认 |
| BudgetRoot/Entry | root、payer、task/operation/request归属、dimension、reserve/consume/release/indeterminate/reconcile、amount、unit、receiptRef、uniqueKey | 各消费owner事务写入；上层只聚合引用 |
| VerificationReceipt | id、method/version、scope、requestPair/objectControlRefs、evidenceSnapshot、verdict/reason、checkedAt | 验证器/受信owner，非模型一句confirmed |
| HumanReview | reviewerRef、project、findingRevision、decision、notes、evidenceRefs、reviewedAt | 明确人工动作；人工意见与机器验证分别展示 |
| ReportVersion | reportId/version、project、task/attempt列表、snapshotManifest、renderer/redaction版本、claimRefs、humanReviewRefs、contentHash | 报告owner生成，不可悄悄覆盖旧版本 |

ID均应稳定、不可复用；时间使用UTC可解析格式。hash应注明算法；完整性用SHA-256等项目已认可实现，不能把显示用DefaultHasher短hash叫作密码学签名。目标参数含秘密时只保存脱敏结构和受控secret引用，不能把secret拼进幂等键或日志。

## 5. API、事件与状态迁移契约

### 5.1 共用服务与工具表面

下列为逻辑接口建议。HTTP/Tauri/MCP只是适配器，必须调用同一owner服务，不要求所有接口都暴露给模型。

| 服务方法 | 核心输入 | 输出/规则 |
|---|---|---|
| previewIntent | URL/text、可信project binding、conversationId | intentCandidates、matchedProject、scopeDiff、预算/模块预览；不得发起网络扫描 |
| prepareTask | goal、project、授权/预设/预算提案、expectedRevision | draftId、previewDigest、所需确认项、不可用能力；不执行 |
| confirmTask | draftId、previewDigest、可信approvalReceipt、operationId | admitted taskId或明确拒绝；模型自填userConfirmed不算审批 |
| readTask/listTasks | context项目、taskId、cursor | 当前状态、预算、未知义务、证据摘要；列表不泄露其他项目 |
| pause/cancelTask | taskId、expectedRevision、operationId | accepted actionReceipt；接受不等于worker已退出 |
| resumeTask | taskId、checkpointRef、expectedRevision、operationId | 重新验证后的准入或blocked原因；不重放未知写入 |
| inspect/reconcileOperation | operationId、owner核对工具、预期状态契约 | not_executed/committed/partial/unknown及证据；不能由任意模型证据解除 |
| queryHistory/readEvidence | project/task/time/keyword/structured IDs、cursor、byte/token limit | 授权过滤后的片段、来源ID/版本、missing/truncated/revoked信息 |
| summarizeContext | source IDs/ranges、expected source head | 新SummaryVersion，不修改原文或授权 |
| createReport/readReport/exportReport | project、task attempts、snapshot policy、format | 固定证据快照、版本、脱敏清单、导出位置；无新扫描 |
| assets.syncSnapshot/query | stable project binding、source snapshot revision、operationId/cursor | Assets owner处理schema；Host只消费DTO |

每个模型工具必须有可发现描述、严格input/output schema、只读/副作用类别、scope说明、上限、operationId来源和错误码。Schema中声明权限不等于授予权限。未知工具/字段拒绝，不能自动替换成空参数执行。结果的`isError`与业务`unknown`不能丢失。

### 5.2 标准错误与终止

至少区分：`model_refusal`、`authentication_required`、`unsupported_capability`、`input_invalid`、`permission_denied`、`scope_changed`、`tool_failed_known`、`tool_outcome_unknown`、`provider_outcome_unknown`、`budget_soft_pause`、`budget_hard_stop`、`context_insufficient`、`persistence_failure`、`checkpoint_incompatible`、`cancelled`。

每个错误包含：稳定code、phase、可公开message、outcomeKnown、sideEffectsPossible、costKnown、retryPolicy、operation/requestRef；敏感详情留受控记录。只有被确认未派发、或owner提供满足契约的幂等/结果查询时才考虑自动重试。不能因HTTP状态或文本含“timeout”自行推定未执行。

### 5.3 状态迁移

任务逻辑状态映射到现有状态，避免一次重命名全部持久字段：

```text
draft -> awaiting_confirmation -> admitted -> running
running -> paused_budget | paused_user | blocked_authorization | blocked_unknown
running -> completed | bounded_completed | refused | failed | cancelling
cancelling -> cancelled_known | blocked_unknown
paused/blocked -> revalidate -> running（原预算剩余、范围和receipt有效时）
```

operation逻辑状态：

```text
prepared -> admitted -> dispatch_claimed -> in_flight
in_flight -> succeeded | failed_known | unknown
unknown -> reconciling -> committed | not_executed | partial | unknown
```

- 原worker停止只是生命周期事实，不自动把operation变成not_executed。
- not_executed核对必须指向原operation；符合原授权/预算时才准许后续attempt。
- partial先停止相关写操作，保留受影响对象与补救提案；补救也需独立授权和预算。
- 已完成任务的后续发现/复核使用新revision，不改写历史证据和报告。

### 5.4 事件

使用现有事件总线扩展：`task.prepared`、`authorization.changed`、`operation.admitted/dispatched/unknown/reconciled`、`budget.reserved/settled/paused`、`context.summary_created/retrieval_incomplete`、`evidence.recorded`、`verification.completed`、`report.created`。

事件带eventId、sequence、project/task/operation、revision、UTC时间、脱敏payload。UI可重连后按sequence补读或读当前状态；事件只作刷新提示，不能把收到事件当成执行完成/费用已结算。撤权后不再向旧页面/旧run披露受限payload。

## 6. 未知结果、持久化operationId与防重复副作用

### 6.1 实施切点

优先修`service.mjs`的MCP派发与`tasks.mjs`核对逻辑，不重写provider循环。把operation身份从`#mcpAppCall`内部临时UUID移到持久化准入层；task evidence记录operationRef，而不是只记录toolCallId和短summary。

派发事务顺序：

1. 从可信上下文解析project/task、当前grant、package/tool schema版本、目标scope。
2. 对规范化、脱敏后的业务参数生成argumentDigest/targetDigest；绑定effectClass与授权revision。
3. 查operation幂等记录：同key同digest返回已有receipt；同key不同digest拒绝冲突；unknown返回核对要求。
4. 同事务写operation prepared/admitted、预算reservation、dispatch claim；提交成功后才调用工具。
5. 保存owner结果/原文引用，事务写terminal receipt和预算结算，再发送模型观察与UI事件。
6. 任意“派发后、结果落盘前”故障保留unknown/reservation，不用重试掩盖持久化失败。

幂等作用域建议为`(projectId, ownerAppId, operationId)`；业务owner仍须检查参数digest。operationId要在UI提交不确定、Host重启、模型换run时可找回，不可每次生成新ID绕过旧义务。

### 6.2 解除未知的证据契约

核对回执必须包含原operationId、目标对象、原参数/意图digest、核对工具与版本、核对时间、授权与数据版本、明确后置条件以及原始结果ref。核对工具由owner声明，不能由模型任意指定“读一个文件就算核对”。

例如“给指定项目导入记录X”必须核对该项目内source/externalId和预期字段/版本；“启动扫描”必须查询同operation创建/启动的task和真实attempt，不能靠任务列表里存在任意任务；文件写入核对目标路径、预期内容hash和可接受版本，不能只检查目录存在。

`resolvesEvidenceRefs`最多作请求引用；真正清除unknown由受信核对器签发receipt后执行。只有时间较新、同run、成功状态、模型解释充分，均不足以解除未知。

### 6.3 重试规则

- 只读工具可有有限次数重试，但目标读取也可能消耗请求预算并触及范围门禁；每次物理请求都记账。
- 有副作用工具不因通用timeout重试。owner明确支持持久幂等且同operation/digest重放时，可查询已有结果或安全返回receipt。
- 模型再次提出相同或等价写操作时先关联未决operation，不能让新toolCallId等于新业务授权。
- Native checkpoint现有未知阻断保持不变；UI明确显示“待核对，未重放”。

## 7. 上下文与记忆分层、隔离及保留

### 7.1 四种scope

| Scope | 可保存内容 | 默认禁止 |
|---|---|---|
| 项目 | 已授权资产元数据、项目约束、跨任务可复用的已标来源事实 | 其他项目记录；把历史授权当永久许可 |
| 会话 | 原始用户/助手消息、用户修正、对话偏好、来源索引 | 以名称碰撞关联其他会话/项目；会话文字变系统授权 |
| 任务 | 原始目标、范围/排除项、预算、计划、决策、问题、反证、失败尝试、未知义务 | 在summary裁剪时删除硬约束；把子报告当父事实 |
| 工具操作 | 参数digest、目标、授权、派发/结果收据、原始工具输出和核对记录 | 缺operationId的隐式写入；跨操作借证据解除未知 |

授权、排除项、未知义务、费用预留、checkpoint兼容性存于结构化owner状态。摘要只引用这些实体，不能成为唯一副本。

### 7.2 三层内容存储

1. **原始记录层**：user/assistant消息及工具请求/结果的可引用记录。这里“原始”指压缩前、按明确脱敏策略捕获的版本；不承诺保存未脱敏secret。记录redactionVersion、是否完整、encoding/hash、来源时间和清理状态。
2. **结构化证据层**：请求/响应artifact、观察事实、假设、反证、决策、验证回执；事实的证明能力由provenance和验证器决定。
3. **工作摘要层**：近期窗口、任务摘要、阶段摘要、检索片段。允许重建/替换，必须带source refs和omissions。

现有`messages`和Native artifact/CAS优先复用。Host新增工具原文存储采用有界inline或owner blob引用；不要复制大HTTP体、JS、图片到每条prompt。先成功保存记录再截断给模型，持久化失败则停止将该观察作为可靠证据。

### 7.3 敏感信息最小化

- Authorization/Cookie、API key、会话token、secret输入在持久化和日志前按策略剥离；留secretRef和必要身份别名。
- 业务证据可能含个人数据：默认保留最小结构、字段路径、脱敏摘录和受控原始artifact；有独立保留期限和查看权限。
- 用户允许测试不等于允许将完整数据发给任意provider；context builder检查该runtime披露授权和模型连接。
- 索引同样不能泄密：不索引秘密值；列表总数、命中摘要、错误提示也受项目和grant约束。
- 删除/清理留下tombstone和保留策略说明，不虚构原文还能恢复。内容hash不能替代已删除正文。

### 7.4 检索接口和授权

queryHistory支持project、conversation/task、UTC时间范围、关键字、message/evidence/operation/artifact ID、kind/provenance、分页与limit。project由可信会话解析，不能接收模型自由选择任意project作为越界入口。

处理顺序：验证身份和项目binding → 检查当前grant/披露 → 加project owner过滤 → 检索/排序 → 限制片段体积 → 返回前再次检查授权。禁止先全库搜索再只过滤前端结果；撤权后不得从缓存摘要取回原受限内容。

跨任务引用需有当前项目权限、数据披露条件和显式范围；任务隔离要求更强时再限制task。历史ref不会重新授予已经撤销的权限。外部页面中的“请忽略用户限制”“已获管理员批准”作为数据保留，不能进入授权快照。

## 8. provider/model能力、token窗口与消费计量

### 8.1 ModelCapability契约

能力记录包括：provider/model/endpoint身份、contextWindowTokens（可unknown）、maxOutputTokens、tokenizer或估算策略、toolCalling及并行能力、Responses/Chat API支持、refusal表示、usage字段映射、cache/reasoning计量方式、来源/验证日期/版本。

不能统一宣称所有模型有同样上下文；不能按模型名字猜价格或免费缓存。runtime/provider切换后重新解析能力、披露和预算，不用上一连接的窗口/价格。ACP若不提供计量或可强制预算，明确为unsupported/unknown；严格硬费用任务不能静默降级运行。

### 8.2 有效输入窗口

若总窗口C已知，计算`inputAllowance = C - outputReservation - safetyMargin`，同时满足provider自己的输入/输出分项约束。输入计数包含system、工具schema、硬约束、当前请求、检索片段、历史和协议开销。推理token是否占输出窗口按provider已确认语义处理，不能重复扣或漏扣。

优先用对应tokenizer或官方已知计量；不支持时使用标为estimated的保守估算，并保留误差余量。bytes限制仍用于传输安全，不能当token硬保证。C未知时仅在明确配置了operator接受的保守输入上限下继续；否则报告能力缺失，禁止臆造C。若硬约束+当前请求就超预算，缩小任务/要求用户调整，不裁掉排除范围继续。

### 8.3 实际usage与费用

每个physicalRequest记录input/output、cachedInput、reasoning（若可得）、modelRequests、providerUsage原始受控ref，以及每项reported/estimated/unknown状态。cachedInput通常可能是input子集，reasoning可能是output子集，是否可加总必须由provider映射定义。

actual usage优先；缺usage不是0，估算不可伪装实账。Native现有usage_reported与一致性校验保留，Host要补同类语义。只有匹配physicalRequest、provider/model和dispatch receipt的terminal usage才能结算，矛盾别名/负数/不完整usage不能释放全部reservation。

费用使用明确currency、计价单位、整数minor/micro units、pricingRevision与来源；不能直接累计float USD。成本上下界与未知分开；缓存不能假定免费；未报告reasoning不能凭文本推算精确账单。成本不能可靠上界时，严格USD硬限模式拒绝准入或暂停等待配置；用户明确选择仅token/请求限额也必须如实显示不保证USD账单。

### 8.4 大结果、截断和断流

- 工具原文先保存，再给模型有界结构摘要、total/returned/hasMore/cursor和artifactRef；截断片段不支持完整性结论。
- 超大输出不把全部正文塞回prompt；按字段/页/区段读取。某段原文未保存则标missing，不说可以回查。
- `length/max_output_tokens`不执行可能残缺的工具调用；仅记录完整且校验通过的观察。续写属于新请求，仍需预算，不能无界补写。
- 缺终止事件、response.failed、取消后的部分流不是完整回答；分别记录known text、unknown outcome/cost。收费收据晚到仍可结算，但不能因此重新授权派发。
- 401刷新只在连接匹配和已有有限策略下进行，不能据“未显示文字”断言未收费；每次物理发送独立留receipt。

## 9. 可追溯压缩与遗漏恢复

### 9.1 压缩流程

1. 原始消息/工具记录成功持久化，固定source head/range与hash。
2. 装配不可压掉的结构化硬约束、排除项、未知义务和预算状态；授权仍来自owner，不来自摘要。
3. 按任务阶段/轮次形成分块增量摘要，保留决策及理由、反证、待验证问题、失败尝试的参数差异和操作状态。
4. 每项summary claim携带source message/evidence/operation refs；摘要带version、algorithm/prompt版本、omissions、supersedes与生成时间。
5. 保留tool call与result协议单元；旧摘要再次压缩仍保留可检索锚点，不能只做“摘要的摘要”而丢来源。
6. 将当前预算内摘要与少量检索片段装入上下文；记录本次context manifest，便于解释模型看到了什么。

摘要生成可确定性或有限模型辅助；模型辅助也消耗预算且结果仍非权威。不要为了压缩强制额外付费请求。先实现确定性有来源摘要，再按实际收益评估其他方式。

### 9.2 不保证无损或百分百精度

压缩必然可能遗漏、误排序或丢细节。禁止以“无损记忆”“百分百精度”“永不遗忘”作为验收目标。重要安全结论、授权变化和重试判断必须回查原始证据/结构化状态；摘要独自不能证明漏洞。

“已确认”摘要标签只允许表示某条受信工具观察成立，不能升级为漏洞已验证。失败信息不得因缺少error关键词而变事实；事实分类读取结构化status/provenance，不再用正文正则决定真实性。用户修正保留原决策与supersededBy，不覆盖历史。

### 9.3 遗漏恢复

当用户说“前面禁止过”“那条反证”“第一次尝试”，先按当前project/task、时间/关键词检索，再按结构化ID回读原记录。返回来源与日期，让当前计划引用新加载的证据。

若命中不足：输出检索范围、已找到内容、缺失项和下一步可查询条件；不能生成貌似原话的内容。若原文未保存/已清理，返回`source_missing/source_purged`，摘要内容保持“历史摘要提及，原文无法核对”。旧grant已撤销则返回无披露权限，不能以“恢复记忆”为由泄露。

### 9.4 验收与度量

- 多轮后能找回早期“只读、不测试外部域”的约束；执行门禁来自结构化约束，即使summary遗漏仍有效。
- 多轮后找回早期反证，不能只检索支持模型当前猜测的片段。
- 两项目相同关键词/URL标题/对象ID不串项目；撤权后缓存与向量/文本索引也不返回旧内容。
- 压缩前后指定关键事实、反证、unknown操作集合一致；原文已清理时明确未知。
- 合成语料定义相关原始ID集合，测`recall@k = 找回相关ID/应找回相关ID`，同时记录precision、错误归因与样本量；指标只对该语料成立。
- 事实一致性使用带来源claim的人工/自动核对，标unsupported claim率；token节省记录同tokenizer下before/after与context overhead。未知tokenizer时只报告bytes变化。
- 不设置虚假百分百承诺；通过门槛需先有基线和样本，再由项目维护者冻结。不可妥协的门禁用例必须全部符合期望。

## 10. 用户贴URL时的交互规范

### 10.1 先解释意图，不先启动

结合当前会话判断：查看页面/解释链接、查询资产归属、导入项目资产、准备授权安全测试。**一个URL不等于扫描授权**。解析URL和查当前项目元数据可先做；访问URL、认证会话使用、探测、扫描分别经过适用权限和预算。

意图不明时给轻量动作：`查看链接`、`查询/导入资产`、`准备授权扫描`；不要堆完整设置弹窗，也不要默认点击扫描。已有明确上下文和授权时直接准备相应动作，不重复确认已授权的低风险操作。

### 10.2 项目选择和创建

- 优先复用当前project与可信externalId映射；展示“当前项目X、将处理目标Y”。同名项目不能自动合并。
- 若URL未匹配现有项目，给出候选/无匹配，并询问是否新建项目；预览项目名、目标、范围、预算、模块和后续动作。
- 单独确认建项目只创建项目；不能自动授权所有扫描。单独确认资产导入只授予该导入，不是主动测试许可。
- 用户一句明确包含“为本地合成目标创建项目，按这些范围/限额开始扫描”时，可用一次绑定完整预览的确认完成创建与扫描授权；不能机械要求每个内部步骤再确认。
- 预览后目标/预算/权限有变化，旧previewDigest失效；只重确认变化部分及其影响。

### 10.3 范围不自动扩张

scope至少可表达scheme、host规则、port、可选path prefix、method/action classes、排除项、身份别名、有效期。域名发现/关联资产/重定向/第三方登录依赖只形成候选，不自动加入范围。需要拓展时展示scope diff并请求新确认；保留Native原有冻结范围门禁。

DNS/重定向后的实际目标与私网规则、浏览器子请求、认证回调应在后端逐跳核对。首轮MVP可以仅支持明确的localhost合成target和窄HTTP读动作，不宣称所有精细scope已支持。

## 11. Agent代操作与UI共用执行路径

### 11.1 调用方式

Agent通过与UI共用的typed commands/application services执行，优先使用现有Tauri command、Native SDK action和应用worker协议的薄适配；不靠点击模拟，不绕过原始Native门禁。Node v1扫描包继续明确draft-only，不能伪装为SDK2完整执行。

前端负责：意图卡片、项目/范围/预算预览、确认回执UI、任务中心、进度/取消/恢复、证据与报告入口。组件名字可增量采用`IntentActionCard`、`TaskScopePreview`、`TaskProgress`、`UnknownOperationPanel`、`EvidenceReference`、`ReportPreview`；这些是目标职责，不是要求立即新建全部组件。

### 11.2 确认和权限

- 只读及低风险、已明确授权且未改变范围的操作直接执行；不要因为内部工具名不同反复请求授权。
- 高影响写入、认证身份变化、目标范围扩张、预设升档、预算增加需绑定明确用户确认。
- 后端铸造approvalReceipt，绑定project、principal、previewDigest、scope、action、预算、有效期和revision；模型传入`confirmed:true`不能伪造。
- 安装、启用、grant、披露、执行批准独立检查。撤权或package版本变化后旧工具列表、旧批准和旧结果缓存不能继续生效。
- Skills脚本不因“已安装Skill”自动执行；所需工具必须从真实可发现能力获取。

### 11.3 拒绝处理

Chat/Responses refusal标准化为独立结果，尽量保留不含敏感内容的原因分类。拒绝不触发工具补查、方向审查后重问、换模型或拆分请求。可说明已有结果、未执行项及可接受的有限任务边界；修改用户目标后另开明确任务，不以提示注入、绕过防护或无限尝试提高“成功率”。

## 12. 快速/标准/深度：手动执行预设，与Agent介入分离

### 12.1 当前事实与目标变化

当前`runtime_config.rs`已有三档、评分路由与ceiling，UI会传scanMode；聊天speed/quality是另一套选项。**不要将聊天quality等于深度扫描，也不要认为选择深度就允许所有模块。**

保留三档给用户手动选择，显示实际解析后的策略快照。Agent是同一任务引擎的有界计划/步骤选择能力，不是第四档，也不是另一个执行入口。Agent可在预设允许模块和预算内根据证据选择/跳过步骤，不能自行提升preset或解锁模块。

### 12.2 配置结构建议（示意，无生产默认值）

```ts
type ExecutionPreset = {
  schemaVersion: number;
  id: 'quick' | 'standard' | 'deep';
  revision: number;
  limits: {
    wallTimeMs: number;
    modelRequests: number;
    modelTokens: { input: number; output: number; total?: number };
    targetRequests: number;
    concurrency: number;
    cost?: { currency: string; maxMinorUnits: string; pricingRevision: string };
  };
  softThresholds: { progressWindow: number; reserveForCloseout: number };
  allowedModules: string[];
  evidencePolicy: { requiredChecks: string[]; allowUnverifiedDraft: boolean };
};
type TaskExecutionChoice = {
  presetRef: { id: string; revision: number };
  agentAssistance: 'off' | 'plan-and-select';
  authorizationRef: string;
  budgetRootRef: string;
};
```

具体字段需映射现有Native计量维度；不支持的browser_actions/controlled_writes/upload_bytes不能填写“已强制”然后实际放行，现有denied_by_contract保持拒绝。价格/时间/并发数先从当前配置和合成测量取得，不在本规范拍脑袋设默认值。不得宣称普适“快速N分钟、深度必得漏洞”。

### 12.3 可解释执行与暂停

计划显示每步目标、所用模块、预计请求/费用的估算来源、证据要求、跳过理由。进度按真实operation/receipt更新，不以模型叙述填百分比。

软阈值到达：减少低价值分支、保留closeout预算、生成当前证据摘要；是否继续由冻结策略决定。硬阈值不能靠“还有进展”绕过。预算不足进入paused_budget，展示已用/预留/未知/可用余额、未完成覆盖和最小追加提案；没有用户批准不能自动补账或升档。

手动UI与Agent创建同种task/attempt，pause/resume/retry共用状态和owner。已有草稿→confirmSentinelScan模式保留；Agent不是再调用一套自管while循环。

## 13. Host与Native共享预算语义与恢复

### 13.1 账本边界

每个owner维护自己的强一致账本，上层任务引用budget roots。Host聊天、子任务、Native扫描等消费可归属同一用户工作流，但跨owner不是天然同一个数据库事务。

首轮选择以下可落地方式：上层有限总预算切成有回执的owner allocation；各owner原子预留自己的额度，未分配额度不准消费。若分配/回收跨owner结果不明，额度保留为pending/indeterminate，不能两边同时可用。先不实现任意双向实时调拨。

父子Agent共享Host root，每个physicalRequest与子task分项归属；子任务不得另开不受父预算约束的独立钱包。Native invoice归Native root，上层只展示/合计引用，不能再作为Host consume重扣一次。

### 13.2 原子准入与结算

`available = hardLimit - consumed - reserved - indeterminate`，维度单位必须一致。并发准入在事务中检查并写reservation，使用root版本/lease/fencing和唯一键。不能先读余额、调用provider、最后再扣。

actual terminal receipt匹配后结算consume并释放确定未用部分；缺失usage或未知派发保留相应义务。已知未发送才允许释放该部分。重试若产生新物理发送，需新receipt和额度，不能通过相同operationId免计费。

超出预留的真实收费仍应如实入账并阻止后续工作，不能截断账单来伪装未超预算。若provider不支持可验证最大收费，产品不得保证绝对USD封顶；严格模式在准入时拒绝不具上界的能力。

### 13.3 checkpoint与恢复

checkpoint包含schemaVersion、engine/package/tool版本、task/授权/预算ref及revision、最后事件sequence、未决operation/physicalRequest、计划与源证据ref。恢复先校验当前授权、目标、模型/工具能力、lease、预算剩余和未知义务。

- 预算暂停：用户明确批准追加，形成新allocation/revision，旧消耗与reservation继承；恢复不是新建“免费”root。
- 撤权暂停：等待所需明确授权，不能用历史summary恢复grant；不扩张原目标。
- 未知结果暂停：仅核对路径可用，禁止重放对应写入/目标请求；关联无关任务不必全局停摆。
- 版本不兼容：只读展示历史并给出迁移/新任务提案，不能自动重新签发旧receipt。
- 人工取消：停止新准入，等待已接受工作和晚到receipt；不能宣称取消后无副作用或零费用。

## 14. 越权对象归属与独立证据验证

### 14.1 最小可验证契约

先实现一种有限、合成、可解释的对象访问案例，不做通用“模型判断所有漏洞”。对象控制契约至少包含：objectRef/ID、owner identityRef、合法访问控制组、未授权identityRef、同一业务请求模板/method/参数位置、预期保护字段/结果、授权范围与验证器版本。

对象归属来源必须可独立核对，例如合成fixture的受信setup receipt、明确授权的管理查询或代码/配置证明加运行时绑定；模型自报“对象属于A”、URL有userId、两份响应结构不同都不够。

### 14.2 确认门槛

保留`agent_confirmed_finding_gate`现有请求归属、distinct、family/contract、差异artifact绑定、redirect/cache/WAF排除。新增object/control-group验证receipt绑定同一request pair与对象版本。

合法控制请求证明owner可以访问该对象；负控制证明无授权身份正常应被拒绝或不能获得受保护字段；测试请求证明非owner实际获得同一受保护对象/效果，并排除各自profile个性化、公开字段、缓存、登录页、WAF等干扰。

满足门槛才返回verified；缺证据保留candidate/insufficient_evidence，覆盖账本如实记缺口。不能仅将现有`return Err`删除或交给第二个模型投票。人工复核保留自己的决定，不改写原执行观察。

## 15. 报告闭环与增量交付

### 15.1 复用当前部分功能

现有task/project JSON导出有SQLite读取快照，标记historical_snapshot与executionEligible=false；Source导出有JSON/SARIF/bundle和review snapshot。保留其历史导入限制，不将其改造成可恢复任务的执行包。

本轮只确认这些出口与复核入口存在，没有核实完整Markdown/HTML安全报告、全部脱敏和PDF链路。因此先实现本地Markdown/HTML renderer，复用现有snapshot/evidence读取；不宣布报告功能从零开始，也不以复杂PDF引擎阻塞首版。

### 15.2 数据与状态

项目 → task/attempt → 原始事实与artifact → VerificationReceipt → HumanReview → ReportVersion。

报告区分：`draft`（含模型建议）、`verified`（明确验证方法/范围）、`unable_to_confirm`（缺何种证据）、`scope_limited`（授权/预算/能力不足）。报告整体状态与单finding状态分别标明，不能一个已验证finding让所有段落都变已验证。

证据快照固定task/attempt、事件head、evidence/artifact hash、验证/复核版本、scope、预算receipt和生成时间。单owner尽量用读取事务；跨owner用snapshot manifest列各自asOf/revision，不能伪称跨库原子快照。运行中导出也必须明确包含到哪个head。

### 15.3 最小报告内容

1. 项目、任务、目标、授权范围/排除项、时间、预设、引擎/验证器版本。
2. 实际执行模块、请求/计量的reported/estimated/unknown分类、暂停/终止原因。
3. 每项发现：结论状态、影响的已观察部分、对照步骤、请求/响应/对象控制证据ref、验证范围及限制。
4. 反证、未确认候选、未覆盖范围、未知副作用/费用和待人工问题。
5. 人工复核记录与修复建议；建议和推断标明，不把模型猜测的业务影响当事实。
6. 可复核附件索引、脱敏说明、source_missing/expired项、版本与生成快照。

### 15.4 导出与更新

Markdown默认脱敏；HTML转义所有不可信字段并限制Markdown渲染能力，不允许script/raw HTML注入、远程图片/字体自动请求或将凭据带入链接。导出路径由现有export owner控制，不能由模型任意写系统路径。

相同snapshot与renderer/redaction版本可重生成相同语义内容；新证据产生report新版本并列added/changed/retracted，不悄悄修改旧报告。重新生成不发送新目标请求、不付费重跑验证。PDF仅在现有依赖/平台能力评估后另批实施；当前根package.json未见专用PDF依赖，不据此断言整个仓库没有任何PDF能力。

## 16. Host与Assets边界修复

将`desktop-assets.mjs`直接操作inventory和Host insight私有schema的路径收敛到Assets owner，但不先大规模搬库。

第一步定义版本化DTO：sourceProjectBinding、sourceSnapshotId/revision、observations、identity规则、removed IDs、asOf、complete/truncated。同步有独立operationId和transaction；Assets owner决定schema迁移、唯一键、删除边界和返回版本。Host只决定是否调用及是否披露。

旧直接镜像适配器可短期保留在owner模块后面，明确标legacy，不在Host新增SQL分支。新接口上线后删除Host对私有表的写依赖；旧schema不兼容返回requires_upgrade，不能由模型读取静默迁移。保持当前“同步失败不返回陈旧镜像”的行为。

验收覆盖等量修改、删除、替换、跨项目相同externalId、分页cursor scope、旧版本升级、owner未安装/禁用/撤权。没有映射的app-only项目可读自己的inventory，不因可选桌面库缺失被误阻断。

## 17. 数据兼容、迁移与失败回滚

### 17.1 向前兼容

- 每个owner采用增量、有版本、事务性迁移；先在临时合成库验证。不由Host自动迁移Assets/Scanning私有表。
- 现有messages/task JSON保留；新增字段有明确legacy/unknown状态。旧短facts只能导入为advisory summary，不能补造原文、observed证据或授权。
- 旧operation缺ID时标legacy_unbound；可展示和人工核对，但不能凭新UUID自动重试。未知记录不能因为升级丢掉。
- 旧scanMode映射到有revision的preset snapshot，保留历史解析值；不能升级默认策略后悄悄扩大旧任务范围。
- 历史账本、checkpoint、导入报告不重新签发授权/lease/计费收据。缺source record返回missing，不能从摘要伪造回填。

### 17.2 失败与回滚

- 持久化失败在派发前：拒绝执行；派发后：unknown并阻断相关重放，不吞错误写“完成”。
- 迁移失败事务回滚，保留原库/版本；不部分启用新消费者。真实数据备份和迁移须另有明确维护授权，不在本轮文档工作执行。
- 功能flag可停新准入、保留只读审计和原owner恢复能力；不能通过flag清空unknown/reservation或退回无门禁旧路径。
- 旧二进制不理解新状态时拒绝写入或只读打开；不能将未知状态fallback成completed/observed。
- 事件发送失败可补读，不能回滚已经提交的真实副作用；报告生成失败不回滚扫描事实，允许同snapshot重生成。
- 不承诺外部副作用可事务回滚。补偿操作是新的受控操作，单独预览/授权/计费。

## 18. 按阻塞关系实施的最小批次

所有批次以最新源码差异为准，已经有等价实现的项目先核对并复用，不再重复大拆。每批先定义针对性验收，完成后才扩展范围。

| 批次/任务 | 优先级/依赖 | 主要切点与交付物 | 完成标准 |
|---|---|---|---|
| B0-01 基线对齐 | P1，首先 | 读AGENTS/本规范/当前diff；列当前已实现、待改和并行owner | 无覆盖他人改动；记录HEAD/dirty、所选有限文件和拟运行测试 |
| B1-01 操作收据 | P1，B0 | Host store/tasks/service增量operation记录、稳定ID、digest冲突 | 重启可定位原operation；先落盘后派发；同key不同参数拒绝 |
| B1-02 未知核对 | P1，B1-01 | owner核对契约、清除unknown唯一门禁、UI阻塞理由 | 无关新读取不能解除；unknown不盲重放；不影响无关只读工作 |
| B1-03 refusal分类 | P1，B0；可与B1-01独立实施 | Chat/Responses解析与标准终态/展示 | 拒绝不进入补试/工具执行；与tool/network错误区分；保留取消/断流语义 |
| B2-01 能力与计量 | P1，B1-01 | ModelCapability、usage映射及reported/estimated/unknown | 无虚构窗口价格；物理请求身份可关联usage；协议不支持显式拒绝 |
| B2-02 Host预算 | P1，B2-01 | root/child reservation、settle、indeterminate；对齐NativeDTO | 并发最后额度只准入一次；重启不退款；父子合计不重复计费 |
| B2-03 窗口准入 | P1，B2-01 | 发请求前context manifest和input/output余量计算 | 硬约束装不下不发送；大工具按页；不全量回灌历史 |
| B3-01 原文与索引 | P1，B1-01 | Host工具原文/证据引用、分层约束、项目过滤检索 | 保存后压缩；缺原文显示missing；撤权缓存同样失效 |
| B3-02 版本摘要 | P1，B3-01、B2-03 | SummaryVersion、source refs、增量分块与恢复 | 早期约束/反证可找回；工具协议成对；关键状态不依赖摘要 |
| B4-01 共用预览确认 | P1，B1、B2 | URL intent、typed prepare/confirm adapter、approval receipt | 单URL不启动；一次明确完整授权不被机械重复询问；变化使旧preview失效 |
| B4-02 执行预设 | P1，B4-01 | preset snapshot、agentAssistance独立、共用task引擎 | 手动/Agent状态一致；Agent不升档；预算不足暂停可解释 |
| B5-01 对象控制验证 | P1，B1、B2、B3-01 | 首个有限authorization object/control契约与验证receipt | 合法个性化不误报；缺控制组仍拒绝；合成越权有真实绑定证据 |
| B6-01 本地报告 | P2，B3-01；verified部分依B5 | 复用现有snapshot，Markdown/HTML renderer与ReportVersion | 区分draft/verified/unknown/scope；脱敏、引用、版本、可重生成 |
| B7-01 Assets owner接口 | P2，B1-01；不阻塞B1/B2 | 版本化sync/query DTO，替换Host私有SQL | 隔离/游标/同步失败回归通过；模型读不触发迁移 |
| B8-01 合成闭环验收 | P1，MVP所需B1–B6 | 一个localhost、mock provider、临时库的完整案例 | 覆盖预览授权→任务→暂停恢复→证据→复核→报告；无外部网络 |

首个可用里程碑：B1 + B2最小强约束 + B3原文/检索 + B4窄URL流程 + B6草稿报告。B5未完成时报告必须保留“越权无法确认”，不可为了赶MVP放松验证。B7后续收敛不阻塞首个安全闭环，但不再增加Host私有schema耦合。

## 19. 需要补的自动化测试与现有测试区别

### 19.1 已看到的测试代码（本轮没有运行）

- `packages/agent-host/test/task-recovery.test.mjs:74`：进程中断/持久任务；`:224`：旧run读取不能证明新写步骤。
- `packages/agent-host/test/provider-integrity-regression.test.mjs:46`：缺终止事件；`:54`：显式错误；`:146`：截断工具不执行。
- `packages/agent-host/test/workspace-authority-regression.test.mjs:201`：重试沿用operationId；fixture用Set模拟幂等，不能当所有真实工具exactly-once证明。
- `packages/agent-host/test/desktop-assets-sync.test.mjs:10`：等量修改/移除/替换；`desktop-assets-host-guard.test.mjs:49`：同步失败不读旧镜像。
- `packages/agent-host/test/context-integrity-regression.test.mjs:8`：协议成组压缩；`:21`：无关工具/旧记忆不证明当前资产数据。
- `src-tauri/src/agent_runtime/controlled_executor.rs:519`：unknown停止不重试的fixture。
- `src-tauri/src/agent_runtime/stage1a_tests/evidence_review.rs:198`：review confirmed不直接创建sentinel finding。

这些是代码存在的证据，**不是本轮通过清单**。接手者按批次选择匹配用例；不要默认跑`test:agent-first`或全量Rust测试。

### 19.2 新增/扩展验收用例

| 用例ID | 类型与输入 | 期望 |
|---|---|---|
| T01 | operation派发后、结果落盘前模拟崩溃 | 重启unknown；原ID存在；执行次数不增加 |
| T02 | 未知写后新run读取无关文件/资产 | 不能解除unknown、不能以该证据验收原写 |
| T03 | 同operation重复/参数冲突 | 同参返回receipt或待核对；不同参冲突；不增加真实副作用 |
| T04 | 新toolCallId重复同未知业务操作 | 关联原义务，不能新UUID绕过 |
| T05 | Chat delta.refusal/Responses refusal part | refusal终态，无补试、无工具派发，UI分类正确 |
| T06 | refusal与tool fault/401/429/断流混合 | 分类独立；不把拒绝当重试策略；计量未知保留 |
| T07 | 两并发请求争最后一份额度 | 只一个成功预留；另一未派发；父子root一致 |
| T08 | 缺usage、矛盾usage、晚到receipt、重复settle | 无假零账单/退款；只结算一次；停止新准入 |
| T09 | Host→Native allocation确认丢失 | 总额度不在两owner同时可用；pending保留 |
| T10 | 不同模型窗口/未知能力/极小窗口 | 扣输出与余量；硬约束放不下拒发；不把bytes当tokens |
| T11 | 超大工具结果、截断参数、缺terminal SSE | 原文有ref或明确missing；残缺调用不执行；partial不算完成 |
| T12 | 30+轮合成对话含早期约束和反证 | 定向找回原ID；结构化门禁保持；summary不自证 |
| T13 | 双项目同关键词/同对象ID、撤权再检索 | 无跨项目命中/数量泄露；旧缓存不披露 |
| T14 | source已清理、summary还在 | source_purged；禁止补造原文；结论降为无法核对 |
| T15 | 网页/工具输出包含批准和覆盖指令 | 只作数据；不能变授权/预设升级/执行指令 |
| T16 | 单URL、仅建项目、明确创建+范围+扫描三种输入 | 前两者不启动扫描；完整授权一次确认后可走同引擎 |
| T17 | 预览后更改项目/目标/预算/撤权 | 旧确认拒绝；不会把异步结果写进新项目 |
| T18 | quick/standard/deep×Agent on/off | 同引擎，模块/额度一致；Agent不升档；状态可解释 |
| T19 | 两身份各自profile返回不同role | 不确认越权；保留候选/反证和具体reason |
| T20 | 合成owner对象被非owner读取 | 仅在绑定控制组/请求对照/对象证据齐全时verified |
| T21 | 报告含恶意HTML、secret、缺artifact、运行中新增证据 | 安全转义/脱敏；缺证据标明；旧snapshot不漂移 |
| T22 | 同snapshot重生成、增量新证据、导入报告 | 不重扫；版本可比较；导入不产生执行/复核授权 |
| T23 | 旧schema/旧task/旧preset升级和回滚 | 原文与unknown不丢；无自动扩大授权；失败保持只读可审计 |
| T24 | Assets等量变化/删除/映射缺失/owner升级 | 按项目同步正确；失败不旧读；无模型触发静默迁移 |

测试默认使用临时目录、合成SQLite、mock provider和localhost fixture。进程中断用测试自己创建的子进程，不能停止用户现有进程。真实provider/外部目标/真实数据库验收需要另行明确授权，不属于上述自动化默认范围。

## 20. 首个本地合成案例与人工验收

### 20.1 案例定义（待接手者实施，本轮未运行）

在临时测试环境创建localhost fixture：两个合成身份A/B、A拥有的对象O、公开profile端点，以及可切换的安全/故障版本。mock provider返回可预测的工具选择、一次refusal、一次usage缺失。所有网络只能命中该fixture，不访问外部域。

用户表达“为这个本地合成目标建立项目，限定这些路径/身份/请求与token额度，进行标准预设验证并生成报告”。UI/Agent解析成同一个prepare/confirm服务；一次完整确认后创建项目与任务。预设数值由测试fixture显式给定，不作为生产默认宣传。

### 20.2 人工验收步骤

1. 仅贴URL：看到动作选择，无扫描attempt或网络探测。
2. 明确完整授权：预览项目/范围/预算/模块；确认后拿到唯一taskId/operationId，任务中心可见。
3. 执行中看到真实工具/预算进度；达到合成预算软阈值暂停，不自行升档。
4. 模拟一个结果未知：页面显示核对要求；按继续不重复写；无关读取不能解除。
5. 经匹配operation的核对receipt后恢复剩余工作；旧消耗不清零。
6. 压缩多轮后回查早期排除项和反证，显示原始ID及时间；切项目不出现旧内容。
7. 安全profile个性化案例不误报；故障对象访问仅在控制组证据齐全时确认。
8. 人工复核后生成Markdown/HTML；报告区分已验证、未确认与范围不足，附证据快照和脱敏说明。
9. 新证据后产生新报告版本；旧版本可查；重生成不触发新扫描或模型请求。
10. 明确记录实际命令、fixture版本、测试结果、未完成项；不得用截图/模型总结替代执行证据。

## 21. 接手执行清单与完成定义

- [ ] 先复核当前HEAD、dirty、AGENTS、并行修改范围和本规范锚点，更新差异表。
- [ ] 选择一个批次/有限任务，不顺手重构无关设置、应用、数据库或UI。
- [ ] 先给D01/D02等确定缺陷补有意义的失败用例，再修对应门禁；低风险文案不机械补镜像实现测试。
- [ ] 任何新API/schema明确项目/授权/预算/operation来源、错误与unknown语义。
- [ ] 持久化先于派发/压缩；恢复不抹账、不盲重放；回滚不打开旧无门禁路径。
- [ ] 预算和窗口的reported/estimated/unknown在存储、API、UI、报告中一致。
- [ ] 所有摘要和报告结论可回查来源；原文缺失/撤权明确显示，不能虚构。
- [ ] 仅运行与改动相关、使用合成数据的针对性测试；通过后若无新风险不反复扩大测试。
- [ ] 汇报文件差异、实际运行命令/结果、未执行项、功能flag、兼容风险和下一批阻塞。
- [ ] 未获明确授权不操作真实账号/数据库/外部目标，不提交、合并、推送或干扰并行进程。

完成不是“Agent说完成”，而是所选批次的验收实际符合期望、变更可审查、未完成边界明确。安全功能的不可用应如实报告，不通过移除门禁换绿测。

## 22. 给接手Codex的可复制执行话术

```text
请在当前本地Oviraptor工作树按docs/AGENT_SECURITY_WORKFLOW_IMPLEMENTATION_2026-10-18.md增量实施。
先只读核对当前分支、HEAD、git status、AGENTS和本文锚点；另一个Codex可能并行修改，不还原、覆盖或清理他人改动。
本文是静态审查和目标规范，不是已运行验收；先列最新代码与本文的差异、已完成项和所选最小批次。
按依赖优先处理operation-bound未知结果核对、稳定operationId、防重复副作用及refusal分类，再补共享预算/原文索引/窗口与压缩。
不要重写通用Agent或再造扫描引擎。手动UI与Agent走同一typed application service和任务状态；Native已有scope、费用、lease、恢复门禁必须保留。
单URL不等于扫描授权；已有明确授权无需机械重复确认；范围变化、预算增加、高影响动作绑定可审查预览及可信确认。
模型拒绝独立停止，不通过拆分、改写、换模型或无限尝试规避。缺对象归属/控制组证据的越权候选保持无法确认。
先在临时库、mock provider和localhost合成案例实现一个闭环；不访问真实账号、真实用户数据库或外部目标，除非另有明确授权。
每批只改必要文件并跑针对性测试，不全量盲跑。报告实际命令与结果、变更、兼容风险、未完成和下一步；不宣称未执行的测试/扫描/模型功能已验证。
不擅自提交、合并、推送、切分支或停止现有进程。文档中是未来任务的命令不代表本轮已经执行。
若本文文件名有-v2等后缀，以上述本次交接的实际文件为准，不覆盖旧文档。
```

## 23. 本文交付说明

本文将上一轮全部审查发现与本轮定向补读纳入同一实施顺序。事实、待核实项与目标设计分别标注；所有测试、迁移和合成案例仍待后续实施者执行。本文不改变源码、配置、运行状态、安装包或任何用户数据，也不构成新的扫描授权。
