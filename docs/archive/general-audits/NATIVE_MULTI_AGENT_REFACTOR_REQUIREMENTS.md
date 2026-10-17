# Oviraptor（Nest）原生多智能体重构任务书

> 状态：历史合同，不要作为下一轮编码任务执行。
> 已完成并已从 `docs/` 删除的执行单：Stage 1A 持久化骨架、Stage 1A 审计修复。
> 当前唯一实施合同是 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`。下一轮只做该文件的 Stage 0。本文残留的 Stage 2 及之后要求，以总计划 Stage 6 起的更严格合同为准；总计划已取消“保留可执行 Strix adapter”。

> 编写日期：2026-09-20
> 适用范围：Oviraptor 1.1.59 当前未提交工作区
> 目标读者：负责下一阶段重构的 Qoder / 实现者
> 文档级别：实现与验收合同，不是产品愿景草稿

## 0. 使用方式与优先级

实现者开始前必须完整阅读：

1. `docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md`
2. `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`
3. `docs/NATIVE_AGENT_EXECUTOR_TASK.md`
4. `docs/NEST_AGENT_COLLABORATION_UI_REQUIREMENTS.md`
5. 本文件

发生冲突时按以下顺序处理：

1. Phase 2 可靠性与安全不变量优先；不得为了多 Agent 破坏续跑、冻结计划、范围、脱敏、证据绑定和唯一终态。
2. 本文件优先于旧架构文档中对 Phase 5 的简略描述。
3. 现有数据库历史兼容优先；只能做增量迁移，不能清空、覆盖或重新解释历史扫描。

当前工作区包含大量用户改动和未跟踪文件。禁止执行 `git reset --hard`、`git checkout -- .`、
`git restore .`、`git clean -fd`，不得覆盖与本任务无关的修改。修改前先运行并记录基线：

```bash
git status --short
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

## 1. 本轮目标

把当前原生单 Agent 演进成：

```text
1 个 Coordinator
  + 若干按结构化证据触发的专家 Agent
  + 1 个拥有漏洞确认权的 Evidence Reviewer
```

正确方向不是常驻几十个 Agent，也不是固定地把专家挨个执行一遍。Coordinator 是 chief of
staff，专家既是有界任务执行者，也是“还缺什么证据”的专业顾问，Reviewer 是结论门禁。Agent
可以围绕同一个 evidence revision 提交缺口、补充、反驳和依赖请求；Coordinator 根据预期信息
增益、成本、风险和重叠度动态选择下一步。所有协作必须经持久化任务切片、共享证据图、预算租约
和类型化消息完成，不能退化成无限自由聊天。

本轮最终目标同时包括：

- 通用运行时、数据库、UI 和业务状态不再以 Strix 命名或假设为中心。
- `StrixAgentBackend` 只作为可选兼容适配器存在，不参与 Native 多 Agent 内部调度。
- Nest 是用户可见的扫描工作台名称；`Native` / `Strix` 只允许出现在后端诊断和迁移设置中。
- `agentBackendPolicy=native` 时，完整多 Agent 路径不启动 Strix 进程、不读取 Strix 产物作为执行依据。

## 2. 明确不做什么

本轮不是：

- 为每个漏洞类型创建一个 Agent。
- 让 Coordinator 自己执行全部 HTTP、浏览器、上传和并发测试。
- 让子 Agent 通过自然语言聊天协商范围、预算或终态。
- 把本文的角色顺序误解成每个目标都必须经过的固定流水线。
- 让多个 Agent 同时对同一目标发请求以换取速度。
- 用多个相同单 Agent 循环并行扫描同一 URL。
- 先删除 Strix 兼容层，再补齐 Native 能力。
- 在多 Agent 之上新增第二套扫描状态机、第二套重试入口或第二个终态 reducer。
- 把响应差异、路径命中、模型判断或“看起来可疑”直接升级为 confirmed finding。
- 只保存高危结果而丢弃阴性结果、排除原因、覆盖缺口和低价值线索。

## 3. 当前实现基线与必须复用的能力

现有代码不是空白工程。重构必须复用以下真实能力，不得平行复制：

| 现有能力 | 唯一或主要实现位置 | 多 Agent 中的用途 |
| --- | --- | --- |
| frozen execution plan | `commands/agent_runtime.rs`、`agent_runtime/store.rs` | Coordinator 冻结根计划，子 Agent 只能领取切片 |
| attempt 继承 | `commands/agent_contract.rs` | 根 run 和子 run 续跑，不重复事实和预算 |
| scope 决策 | `agent_scope_assess` | 所有专家出站动作的唯一范围门禁 |
| Tool Broker 与 schema | `commands/agent_tools.rs` | 专家只能调用注册工具 |
| 原始证据/模型视图分离 | `agent_tools.rs`、`agent_runtime/secrets.rs` | 所有角色使用同一脱敏路径 |
| 内容寻址 artifact | `agent_runtime/store.rs` | Agent 之间只传 artifact id |
| event/snapshot replay | `agent_runtime/checkpoint.rs` | 崩溃恢复、租约回收和任务重派 |
| 单写终态 reducer | `agent_runtime/reducer.rs` | 只有 Coordinator 可关闭目标 |
| 父子 run、预算、租约 | `agent_runs` | 多 Agent 的持久化骨架 |
| 类型化 mailbox | `agent_messages` | Assignment、事实、复核结果通信 |
| 工具审计 | `tool_invocations` | 去重、预算、证据链和 Reviewer 检查 |

当前 `AgentRole`、`AgentMessageKind`、`parent_run_id`、`lease_expires_at` 和 mailbox 只是基础骨架，
不能以“表已经存在”视为 Phase 5 已完成。当前 Native 循环仍由一个 Agent 同时规划、执行、判断和
收口；本任务必须真正拆开权限与生命周期。

## 4. 全局不变量

以下规则在任何阶段都不得被打破：

1. 一个 target attempt 只有一个 Coordinator。
2. 只有 Coordinator 能创建、暂停、取消、重派专家任务和写目标终态。
3. 只有 Evidence Reviewer 能输出 `confirmed` 或 `rejected`；其他角色只能提交 candidate。
4. Coordinator 不能越过 Reviewer 把 candidate 写成确认漏洞。
5. 同一时间最多一个会触碰目标的执行 Agent。
6. 同一时间最多一个只读分析 Agent和一个 Reviewer；三条通道总并发不超过 3。
7. 所有出站动作继续经过同一个 scope gate、Tool Broker、预算预留和审计事务。
8. 所有 Agent 都不能执行任意 shell、任意 Python、任意浏览器脚本或自行创建线程。
9. 子 Agent 不接收 Cookie、JWT、API Key 或原始业务秘密，只接收 opaque identity handle。
10. 原始响应、截图、JS 和文件内容不进入 mailbox；mailbox 只保存脱敏摘要和 artifact refs。
11. 合同、请求、工具调用、候选和复核决定必须有稳定 ID，不得依赖自然语言文本关联。
12. Fresh 与 ContinueIncomplete 语义继续严格分离。
13. 子 Agent 消耗必须计入根 attempt；父子预算不得超卖、重放或“失败退款”。
14. WAF、挑战、持续 429、用户暂停和取消会传播到所有子 run。
15. 任一数据库关键写入失败时停止后续副作用，不允许“动作成功但证据链丢失”。
16. 未覆盖、证据不足、能力缺失和 Reviewer 拒绝都不能显示为“安全”或“完整完成”。
17. 专家可以提出、补充或反驳 EvidenceGap，但只有 Coordinator 能接受提案、创建合同和改变全局队列。
18. 同一 evidence revision 上的协商必须有轮次和成本上限，不能让 Agent 在没有新事实时循环讨论。

## 5. 角色模型

### 5.1 Coordinator

唯一拥有全局队列、预算和目标终态的控制面角色。

职责：

- 固化授权范围、身份集合、禁止操作、扫描模式和根预算。
- 读取确定性侦察产物和共享证据图摘要。
- 根据触发谓词生成专家 `Assignment`，为其签发预算与能力租约。
- 使用合同键和证据图去重，禁止多个专家重复测试同一事实。
- 聚合候选漏洞链，但不自行确认漏洞。
- 决定继续、扩容、停止、等待身份、转人工或提交 Reviewer。
- 回收子 Agent 剩余租约；将全部子 run 状态归约成唯一目标终态。

禁止：

- 不直接调用 `http.replay`、`browser.perform_action`、上传、并发或第三方测试工具。
- 不直接生成 payload，不直接操作身份凭证。
- 不在没有 Reviewer 决定的情况下发布漏洞。

### 5.2 SPA/API Mapper

首个按条件运行的只读专家。前端分离、SPA、GraphQL、WebSocket 或存在 JS/网络证据时启动。

输入：确定性前端侦察、页面状态、脚本摘要、CDP 网络记录、source map 状态、身份句柄列表。

输出：结构化接口图谱和事实，不输出漏洞结论。必须严格区分：

- `observed`：浏览器或真实请求确实观察到。
- `source_derived`：从源码、source map 或请求构造器还原。
- `inferred`：字符串或路由模式推测，不能进入可执行队列，除非 Coordinator 创建有界验证合同。

至少识别：XHR/Fetch、GraphQL、WebSocket、请求构造器、拦截器、动态请求头、nonce、时间戳、
对象 ID、分页、API host、CDN、监控域名、身份提供方和业务域名。

### 5.3 External Surface Agent

匿名外部表面存在且无需登录时启动，只能使用 anonymous identity。

负责：公开页面、路由、API、GraphQL、WebSocket、文档、登录/注册/找回外围流程、错误信息、
调试端点、敏感配置引用、外部管理面和高置信隐藏接口。

不得：伪造内部账号、枚举大词表、绕过验证码、进行密码攻击或把版本线索直接报告为漏洞。

### 5.4 Identity & Session Agent

至少有一个已验证登录会话时启动。匿名、单账户和双账户是不同数据模型：

```text
AnonymousOnly
SingleIdentity { identity }
IdentitySet { identities: Vec<IdentityHandle>, comparable_pairs }
```

禁止为了 UI 一致强行生成“账号 A/B”。该 Agent 负责会话有效性、刷新、退出、设备绑定、
登录前后差异、Cookie/JWT/CSRF/动态签名来源和身份串线检查。HTTP 200 的业务失败必须与真正
认证成功分开。

### 5.5 Authorization Agent

满足任一条件才可启动：

- 有两个以上可比较身份；
- 存在对象标识且拥有合法控制组；
- 发现角色、租户、组织或管理功能差异。

水平/垂直越权候选必须绑定：控制请求、测试请求、两个身份或合法身份/匿名对照、对象变化、
响应差异、实际影响和反证检查。单纯 JSON 不同、字段数量不同或状态码不同都不构成结论。

### 5.6 Input & Parser Agent

存在可控参数、请求体、Header、路径、URL、文件名、XML、模板或序列化边界时启动。

统一处理 SQL/NoSQL/表达式异常、解析器差异、SSRF 候选、路径/URL/模板/序列化输入、类型与
边界值、Content-Type 和参数位置变化。模型只能从 Tool Broker 提供的受控变体模板中选择，
不能提交任意攻击字符串。

### 5.7 Upload Agent

只有观察到上传入口、文件参数或上传合同后才启动。使用无害标记文件，覆盖扩展名/MIME/魔数、
文件名与元数据、访问域名、权限、覆盖、跨用户读取、下载鉴权和解析面。

每个受控写入必须同时创建 `CleanupContract`。清理结果是终态前置条件：成功、明确不适用或
失败并转人工；不能静默忽略。没有清理能力时禁止执行写测试。

### 5.8 Business Logic Agent

仅在 Coordinator 已生成业务状态图后启动。负责状态机、重复提交、数值边界、正负对冲、优惠、
额度、积分、库存、步骤顺序、幂等和跨账户对象流转。

它不能凭字段名猜测金额后直接修改请求；每个动作必须引用 `BusinessTransition`、前置状态、预期
不变量、恢复或补偿动作以及最大尝试数。

### 5.9 Concurrency Agent

只有存在幂等键、余额、库存、领取、创建或重复消费的明确候选时启动。必须调用原生受控调度器，
默认并发 2，硬上限 3；不得让模型创建线程或进程。

它不是压力测试 Agent。必须记录基准请求、并发请求集合、最终状态查询和恢复检查。遇到 429、
WAF、状态不可恢复或目标异常立即停止。

### 5.10 Client-Side Agent

存在 DOM sink、postMessage、iframe/跨窗口通信、前端鉴权、本地存储敏感数据、路由污染、
第三方 SDK、source map、CSP 或跨域边界证据时启动。

默认属于只读分析通道；需要真实浏览器动作时，必须由 Coordinator 为该动作创建目标触碰合同，
并等待触碰通道空闲。

### 5.11 Deep Investigator

当初轮覆盖没有突破、不同专家结论矛盾，或 Reviewer 返回 `insufficient_evidence` 时按条件启动。
它读取共享证据图和其他专家的提案/反证，组合候选链、发现整体缺口，并向 Coordinator 提交最小
补证链。它默认只读，不能直接发目标请求、确认漏洞或命令其他 Agent。

### 5.12 Evidence Reviewer

独立只读角色，不能调用任何目标工具。它检查：

- 请求是否真实执行，artifact 与 tool invocation 是否存在。
- 控制组、身份隔离、对象变化和响应差异是否满足合同。
- 差异是否只是时间戳、广告、随机值、缓存、个性化、登录重定向、WAF 或统一错误页。
- 影响和严重度是否被夸大，是否存在反证。
- 清理是否完成，结果能否安全复现。
- 置信度、等级变化条件和仍缺失的证据。

输出只能是：

```text
confirmed
rejected
insufficient_evidence
```

Reviewer 不得补做目标请求。证据不足时只能返回缺失清单，由 Coordinator 决定是否新建合同。

## 6. 角色名称与兼容迁移

`AgentRole` 应扩展为稳定、产品无关的枚举：

```rust
Coordinator
SpaApiMapper
ExternalSurface
IdentitySession
Authorization
InputParser
Upload
BusinessLogic
Concurrency
ClientSide
DeepInvestigator
EvidenceReviewer
```

旧值按读取别名兼容，不得批量改写历史行：

| 旧值 | 新读取语义 |
| --- | --- |
| `evidence_triage` | `spa_api_mapper` |
| `contract_verifier` | `input_parser`（仅历史展示；新任务按真实触发角色） |
| `identity_comparator` | `authorization` |
| `business_flow_analyst` | `business_logic` |
| `final_reviewer` | `evidence_reviewer` |
| `attack_chain_correlator` | `deep_investigator` |

数据库继续保存稳定 snake_case，不保存 UI 中文或 prompt 名称。

## 7. 三条调度通道

Scheduler 必须实现显式 lane，不得只依赖线程池大小：

```rust
enum AgentLane {
    TargetTouching,   // 容量 1
    ReadOnlyAnalysis, // 容量 1
    Review,           // 容量 1
}
```

分类规则：

- `TargetTouching`：HTTP、浏览器动作、身份对照、上传、业务写入、并发验证和任何可能触发服务端逻辑的请求；GET 也算。
- `ReadOnlyAnalysis`：只读数据库、已保存 artifact、JS/source map 本地分析和证据图构建。
- `Review`：Evidence Reviewer，仅读取冻结证据包。

同一 target attempt 最多各运行一个。不同 target 是否可并行仍受应用级全局请求限速、同主机间隔和
模型并发限制约束；不得因为 lane 是 target-local 就突破全局限制。

Reviewer 只能复核冻结候选包。若证据仍在变化，旧复核结果必须以 candidate revision 标记过期，
不能被 Coordinator 继续发布。

## 8. 按证据触发与动态协商，而不是固定流水线

以下只是冷启动时的默认引导图，不是硬编码执行顺序：

```text
确定性侦察
  -> SPA/API Mapper（有前端/API 证据时）
  -> Coordinator 更新证据图和覆盖缺口
     -> 匿名公开面：External Surface
     -> 已验证身份：Identity & Session
     -> 双身份/对象对照：Authorization
     -> 可控输入：Input & Parser
     -> 上传入口：Upload
     -> 前端边界：Client-Side
     -> 业务状态图：Business Logic
     -> 幂等/余额/库存明确候选：Concurrency
  -> Candidate evidence bundle
  -> Evidence Reviewer
  -> Coordinator 发布结果或生成有界补证合同
```

每个角色必须有代码实现的确定性触发谓词；模型可以提出建议，但不能直接 spawn。建议至少包含：

```rust
struct AgentTriggerDecision {
    role: AgentRole,
    eligible: bool,
    reason_codes: Vec<String>,
    evidence_refs: Vec<String>,
    contract_keys: Vec<String>,
    missing_prerequisites: Vec<String>,
}
```

相同 `role + target + identity_set + contract_set + evidence_revision` 生成稳定 assignment dedup key。
重复建议只更新优先级或理由，不创建第二个子 run。

### 8.1 Coordinator 的确定性调度顺序

Coordinator 每次只在以下事件后重新调度：确定性侦察发布新 revision、子 Agent 提交确定结果、
Reviewer 返回决定、租约到期、预算/保护状态变化或用户恢复任务。不得在无新事实时反复询问模型
“下一步做什么”。

一次调度 tick 的固定顺序：

1. 恢复 event/snapshot，回收过期租约，传播取消和保护停止。
2. 读取根硬预算、当前预留和三条 lane 占用。
3. 将新消息幂等并入证据图，推进 evidence revision。
4. 关闭已经有确定结果的合同，生成仍缺失的 coverage obligations。
5. 运行角色触发谓词；过滤缺前置条件、已有 owner、已完成和已耗尽的合同。
6. 优先处理 Reviewer 明确要求且能够安全补齐的证据，其次处理高价值覆盖缺口。
7. 每条空闲 lane 最多签发一个 assignment；没有新增价值时不启动 Agent。
8. 所有必须覆盖项已确定、预算/能力耗尽或保护门禁触发时，进入 Reviewer/终态收口。

任务优先级必须由代码计算并可解释，至少考虑：影响上限、证据强度、覆盖增益、请求成本、身份
可用性、副作用风险和是否阻塞其他合同。模型可以建议分数原因，最终排序由 Rust 完成。

### 8.2 Evidence Gap Deliberation（证据缺口协商）

为了避免“Mapper 做完才轮到 External，再机械轮到 Identity”的死板流程，Coordinator 在每个新的
evidence revision 上发布一个脱敏、限长的 `SituationFrame`：

```rust
struct SituationFrame {
    revision: i64,
    target_summary: JsonValue,
    known_facts: Vec<String>,
    open_hypotheses: Vec<String>,
    coverage_gaps: Vec<String>,
    active_contracts: Vec<String>,
    available_identity_classes: Vec<String>,
    capability_state: Vec<String>,
    remaining_budget: BudgetSummary,
    protection_state: String,
}
```

满足基础触发谓词的专家可以进行一次低成本“自荐”，提交 `GapProposal`：

```rust
struct GapProposal {
    id: String,
    revision: i64,
    proposer_role: AgentRole,
    gap_code: String,
    claim: String,
    supporting_fact_refs: Vec<String>,
    missing_evidence: Vec<String>,
    prerequisites: Vec<String>,
    proposed_contracts: Vec<String>,
    expected_information_gain: f32,
    impact_ceiling: String,
    estimated_cost: BudgetEstimate,
    side_effect_class: SideEffectClass,
    overlap_keys: Vec<String>,
    falsification_condition: String,
    stop_condition: String,
}
```

例子：

- Authorization 发现对象 ID，但指出缺少“对象归属基线”和第二身份，而不是直接开始越权测试。
- Identity & Session 看到该提案后，可补充“身份 B 无效，需要刷新会话”的 prerequisite。
- SPA/API Mapper 可指出对象接口只是 source-derived，尚未 observed，建议先触发对应页面动作。
- Reviewer 可指出当前 candidate 缺控制组、清理证据或实际影响，但不能自行发请求。
- Coordinator 将这些依赖合并成最小合同链，只执行能最大幅度减少不确定性的下一步。

其他专家不能自由修改提案，但可以提交一个结构化 `ProposalAssessment`：

```rust
struct ProposalAssessment {
    proposal_id: String,
    assessor_role: AgentRole,
    verdict: Support | Challenge | SupplyPrerequisite | Duplicate,
    reason_codes: Vec<String>,
    fact_refs: Vec<String>,
    supplied_prerequisites: Vec<String>,
    corrected_cost: Option<BudgetEstimate>,
    corrected_risk: Option<String>,
}
```

Coordinator 对提案做代码侧校验和仲裁：

```text
utility = information_gain
        + coverage_gain
        + impact_ceiling_weight
        + blocker_release_value
        - target_request_cost
        - token_cost
        - side_effect_risk
        - overlap_penalty
        - uncertainty_penalty
```

模型给出的数值必须被 Rust 归一化、钳制并由事实特征校正；不能让 Agent 通过自报高分获得权限。
Coordinator 可以：`accept`、`merge`、`defer`、`reject`、`request_assessment`。只有 `accept/merge`
会创建 Assignment 和合同 owner。

### 8.3 协商防失控规则

- 每个 evidence revision 最多两轮协商：提案轮和一次质询/补充轮。
- 每个角色每轮最多一个主提案、两个 assessment。
- 没有新 fact、artifact、review decision 或 capability change 时，不允许对同一 gap 重新提案。
- `Duplicate` 和 overlap keys 会合并提案，不能生成两个相同测试。
- 协商只消耗小额独立模型预算，不能挤占硬目标请求预算。
- Agent 不能点名命令另一 Agent；它只能声明 prerequisite，由 Coordinator 选择满足方式。
- 超过协商预算、所有 utility 低于阈值或只有不可满足前置条件时，直接转人工建议。
- 触发 WAF、持续 429、用户暂停/取消时立即停止新协商和新 assignment。

因此系统是动态重规划，不是静态 DAG；但每个真实动作仍由 Coordinator 签发、Tool Broker 执行和
Reviewer 复核。智能性位于“发现缺口、质询方案、选择下一步”，安全确定性位于“授权、执行、预算、
证据和终态”。

## 9. 共享结构化证据图

Agent 之间禁止共享自由增长的聊天历史。Canonical evidence graph 至少支持：

```text
Target
Identity
PageState
Endpoint
RequestRecord
ResponseShape
BusinessObject
BusinessState
BusinessTransition
Hypothesis
Contract
ToolInvocation
CandidateFinding
ReviewDecision
Finding
CoverageClaim
CleanupContract
EvidenceGap
GapProposal
ProposalAssessment
```

节点公共字段：

```rust
id
kind
schema_version
scan_id
attempt_number
target_id
producer_run_id
revision
confidence
provenance
artifact_refs
created_at
supersedes
```

边必须使用稳定类型，例如：

```text
OBSERVED_AT
SENT_AS
RECEIVED
AUTHENTICATED_AS
OWNS_OBJECT
DERIVED_FROM
SUPPORTS
CONTRADICTS
TESTS
CONTROL_FOR
VARIANT_OF
PRODUCED
REVIEWED_BY
COVERS
REQUIRES_CLEANUP
SUPERSEDES
```

所有结论从图上的 ID 关联推导。禁止用标题、URL 子串或 prompt 文本关联证据。

### 9.1 事实、假设和结论不可混用

- Fact：工具真实观察或本地确定性解析得到，不含模型推断。
- Hypothesis：待验证声明，必须列出前置事实、验证合同和停止条件。
- CandidateFinding：执行 Agent 提议，不能进入主漏洞视图。
- ReviewDecision：Reviewer 对某个 candidate revision 的决定。
- Finding：Coordinator 只能从 `confirmed` ReviewDecision 投影生成。

### 9.2 保留低价值和阴性信息

数据库必须保存：排除候选、证据不足、普通信息泄漏、组件线索、阴性结果、未覆盖区域和人工建议。
这些信息不能进入主漏洞计数，但可参与攻击链、去重和后续人工分析。

建议展示分层：

```text
主视图：confirmed 高危/严重问题
证据中心：全部执行证据、阴性结果、candidate 和 review
调查图谱：低价值线索、反证和潜在组合链
人工深入：缺能力、缺身份、预算停止和未完成合同
```

## 10. Assignment 与 mailbox 契约

子 Agent 只能接收结构化 `AgentAssignment`：

```rust
struct AgentAssignment {
    id: String,
    schema_version: i64,
    coordinator_run_id: String,
    child_run_id: String,
    role: AgentRole,
    lane: AgentLane,
    target_id: String,
    evidence_revision: i64,
    objective_code: String,
    fact_refs: Vec<String>,
    hypothesis_refs: Vec<String>,
    contract_keys: Vec<String>,
    identity_handles: Vec<String>,
    capability_lease: CapabilityLease,
    budget_lease: BudgetLease,
    stop_conditions: Vec<String>,
    deadline_at: String,
    dedup_key: String,
}
```

mailbox 允许的消息类别应扩展为：

```text
assignment
assignment_accepted
situation_published
gap_proposed
proposal_assessed
proposal_selected
proposal_deferred
prerequisite_requested
fact_submitted
hypothesis_proposed
candidate_submitted
coverage_submitted
review_requested
review_decided
capability_unavailable
budget_returned
heartbeat
completed
failed
cancelled
```

每条消息必须有 `schemaVersion`、`assignmentId`、`producerRunId`、`correlationId`、`dedupKey`、
`evidenceRevision` 和 artifact refs。`delivered_at` 不是处理完成；必须使用 `acknowledged_at`。
重放只能重新派发未确认消息，且 consumer 必须按 dedup key 幂等处理。

协商消息必须绑定 coordination round 和 evidence revision。Agent 只能读取同一 target、同一 attempt
内经过 Coordinator 发布的 SituationFrame、提案和 assessment；不能任意读取其他 Agent 的私有
prompt、完整上下文或未脱敏工具结果。

### 10.1 子 Agent 唯一输出信封

所有专家必须返回同一外层契约，不能各自直接写全局队列：

```rust
struct AgentOutputEnvelope {
    schema_version: i64,
    assignment_id: String,
    producer_run_id: String,
    evidence_revision_read: i64,
    facts: Vec<FactSubmission>,
    hypotheses: Vec<HypothesisSubmission>,
    candidates: Vec<CandidateSubmission>,
    coverage_claims: Vec<CoverageSubmission>,
    tool_invocation_ids: Vec<String>,
    artifact_refs: Vec<String>,
    suggested_next_steps: Vec<NextStepSuggestion>,
    capability_gaps: Vec<String>,
    unused_budget: BudgetDelta,
    completion: AssignmentCompletion,
}
```

`suggested_next_steps` 只是建议；Coordinator 必须重新运行前置条件、范围、去重、lane 和预算检查。
事实提交必须能由 tool invocation 或本地确定性解析 artifact 证明。coverage claim 必须沿用现有
`CoverageEvidence` 门禁，不能因为拆成多 Agent 就重新相信模型字符串。

## 11. 预算、租约与去重

### 11.1 根预算与子租约

`AgentExecutionPlan` 仍是根预算唯一来源。Coordinator 不能凭模型建议扩大全局硬预算，只能把剩余
预算切给子 Agent。

预算至少分开统计：

- 模型输入、缓存输入、输出和总 Token。
- 模型请求数。
- 目标请求数。
- 浏览器动作数。
- controlled write 数。
- 上传字节数。
- 并发验证次数。
- wall-clock deadline。

签发租约必须在一个数据库事务中完成：检查根剩余量、创建 assignment、创建 child run、预留额度、
写 Assignment message。任何一步失败都不启动子 Agent。

子 Agent 结束后只归还“未使用的预留额”；已发出的请求、已开始的模型调用和被取消的副作用不退款。
根计数必须等于 Coordinator 自用量加全部子 run 用量，允许预留未使用但不允许负数或超卖。

### 11.2 合同去重

目标动作的唯一去重键必须至少包含：

```text
scan attempt
canonical target
method/action
normalized parameter shape
identity handle or comparable pair
business object
contract purpose
side-effect class
```

同一个合同只能有一个 owner assignment。其他 Agent 可以订阅结果，但不能复制执行。owner 超时后，
Scheduler 先将运行中的 tool invocation 标记 interrupted，再把未产生确定结果的合同交给新 owner。

## 12. Tool Broker 权限矩阵

能力必须按角色白名单发放，而不是把整个工具列表交给所有 Agent：

| 角色 | 允许的能力 | 明确禁止 |
| --- | --- | --- |
| Coordinator | graph query、assignment、budget、finish | 所有目标请求与写动作 |
| SPA/API Mapper | evidence query、artifact summary、local JS/source-map analysis | HTTP replay、上传、任意浏览器动作 |
| External Surface | 匿名 browser/http、有界 discovery | 登录身份、写动作、大词表枚举 |
| Identity & Session | session inspect、refresh observation、identity compare | 跨任务 session、凭证导出 |
| Authorization | paired replay、object variant、response compare | 无控制组测试、任意对象枚举 |
| Input & Parser | broker-generated variants、replay、compare | 任意 payload、shell、文件系统 |
| Upload | harmless fixture upload、access check、cleanup | 可执行文件、不可清理上传 |
| Business Logic | state transition contracts、bounded replay | 无状态图写操作、批量交易 |
| Concurrency | native bounded race scheduler | 模型线程、压力测试、并发大于 3 |
| Client-Side | local analysis、受控 browser action | 任意 JS execution、范围外导航 |
| Deep Investigator | graph query、proposal/assessment、attack-chain synthesis | 所有目标工具、终态和漏洞确认 |
| Evidence Reviewer | graph/evidence/artifact summary read | 所有目标工具、candidate mutation |

工具注册表必须能根据 `AgentRole + CapabilityLease` 生成可见 schema。只在执行时拒绝还不够；无权工具
不应进入模型上下文。

## 13. Reviewer 发布门禁

每个 CandidateFinding 必须冻结成不可变 revision，并包含：

```rust
candidate_id
revision
hypothesis_id
contract_keys
control_request_ids
test_request_ids
identity_handles
business_object_ids
response_difference_artifact_ids
tool_invocation_ids
claimed_impact
counterevidence_refs
cleanup_contract_id
coverage_family
proposed_severity
```

Reviewer 的决定必须说明：

- verdict 与 reason codes。
- 支持证据和反证 ID。
- 缺失证据。
- 可复现性。
- confidence。
- final severity 与 severity change conditions。
- cleanup 状态。

硬门禁：

- `confirmed` 必须引用真实完成的 tool invocations 和 request records。
- 需要对照的类型必须有控制组；Authorization 必须有合法身份/对象对照。
- controlled write 必须有清理成功或明确的人工处置状态。
- candidate 更新后旧 ReviewDecision 自动失效。
- Reviewer 不允许基于模型声称存在但证据图中不存在的 ID 作决定。

## 14. 沙箱安排

- Coordinator、SPA/API Mapper、Evidence Reviewer：进程内，只读结构化数据库和脱敏 artifact 摘要。
- HTTP、身份对比、输入验证：`NativeRestricted`，只访问冻结 allowed origins。
- 浏览器/CDP：独立受限子进程，每身份独立 profile/session，任务结束清理。
- 上传、复杂文件解析、第三方工具：按需 Container；Docker 缺失时标记 capability unavailable。
- 源码构建、不可信 PoC：必须 Container，不得降级到宿主 shell。
- Concurrency：Rust 原生受控调度器，固定 2–3 并发，不交给模型管理。

所有沙箱返回事实和 artifact refs，不能写任务终态、覆盖完成或确认漏洞。

## 15. 数据库增量设计

不得用一列大 JSON 代替可查询的调度状态。可以保留 payload JSON，但关键生命周期字段必须是列。

### 15.1 `agent_runs` 增量

建议增加：

```text
root_run_id
assignment_id
lane
capability_lease_json
reserved_tokens
reserved_requests
heartbeat_at
cancel_requested_at
```

保留 `parent_run_id`、已有预算/usage 和 lease 字段。历史行通过读取默认值兼容，不强行回填角色语义。

### 15.2 新增 `agent_assignments`

至少包含：assignment id、coordinator/child run、role、lane、target、state、dedup key、trigger、任务切片、
证据 revision、合同集合、身份句柄集合、预算租约、能力租约、deadline、失败分类和时间戳。

唯一约束至少为 `(coordinator_run_id, dedup_key)`。

### 15.3 证据图表

建议使用 `agent_evidence_nodes` 与 `agent_evidence_edges`，payload 存类型专属字段；稳定公共字段用列。
旧 `sentinel_findings`、`sentinel_checkpoints`、调查图谱和现有结果表继续作为兼容投影。新执行只写一次
canonical graph，再由事务内 projector 更新旧 UI 表；禁止两个方向互相回写。

### 15.4 Review 表

`agent_review_decisions` 应唯一绑定 `(candidate_id, candidate_revision, reviewer_run_id)`，保存 verdict、
reason codes、证据/反证 refs、confidence、severity 和缺失证据。Finding 表只引用 confirmed review id。

### 15.5 协商表

新增 `agent_coordination_rounds` 与 `agent_gap_proposals`（assessment 可以独立表或 proposal 子表）。
必须保存 evidence revision、轮次状态、角色、gap code、事实引用、依赖、成本/风险估算、overlap keys、
Coordinator 决定及 reason codes。唯一约束至少包含：

```text
(coordinator_run_id, evidence_revision, proposer_role, gap_code, proposal_hash)
```

SituationFrame 按 revision 内容寻址；恢复时不得重新消耗已经完成的提案/质询模型调用。

### 15.6 原子提交边界

以下操作必须各自在单事务内完成：

1. assignment + child run + budget reservation + mailbox message。
2. tool invocation reservation + 合同 owner 锁定。
3. tool completion + artifact ref + evidence node/edge + usage delta + event。
4. candidate revision + review request。
5. review decision + candidate state。
6. Coordinator terminal reduction + 兼容结果面投影。
7. coordination round + SituationFrame + 可参与角色集合。
8. proposal/assessment + Coordinator decision + assignment（被接受时）。

### 15.7 代码边界

目标目录应逐步收敛为：

```text
src-tauri/src/agent_runtime/
  orchestrator.rs             # Coordinator tick、根终态、取消/暂停传播
  scheduler.rs                # lane、assignment、触发与重派
  budget.rs                   # 根预算、子租约和守恒检查
  evidence_graph/
    contract.rs               # node/edge/revision 类型
    store.rs                  # canonical graph repository
    projector.rs              # 向现有 Sentinel/Nest 结果面单向投影
  multi_agent/
    assignment.rs             # Assignment/OutputEnvelope
    mailbox.rs                # message、ack、replay、dedup
    roles.rs                  # 角色元数据和能力白名单
    triggers.rs               # 纯函数触发谓词
    deliberation.rs           # SituationFrame、gap proposal、assessment、仲裁
    reviewer.rs               # candidate bundle 与 review gate
  tools/                      # Tool Broker/registry/policy，所有角色共用
  sandbox/                    # NativeRestricted/Browser/Container
  strix_adapter.rs            # 迁移期兼容边界
```

迁移期间允许旧文件继续承载部分实现，但依赖方向必须是：

```text
Tauri commands / scan entry
  -> agent_backend selection
  -> Native orchestrator
  -> scheduler / role runner
  -> shared Tool Broker / store
```

`scan_execution.rs` 不得知道具体专家角色、mailbox 消息和 review 规则；它只启动/暂停/cancel 根 run
并消费聚合进度。`commands/agent_native.rs` 最终只保留 Native 后端适配和上下文构造，不继续承载
完整多 Agent 状态机。禁止为了快速迁移在 `commands/` 与 `agent_runtime/` 各保留一套类型。

## 16. 调度状态机

Assignment 状态固定为：

```text
prepared -> leased -> running -> waiting_review -> completed
                          |           |
                          v           v
                       paused      needs_evidence
                          |
                          v
                 failed / cancelled / lease_expired
```

规则：

- 只有 Scheduler 改 assignment 生命周期。
- 子 Agent 只能提交消息和 heartbeat，不能自行标记根任务完成。
- `lease_expired` 后先恢复审计账本，再决定重派；不能直接重复工具调用。
- `needs_evidence` 回到 Coordinator，由其生成新合同或转人工；Reviewer 不能自行调度执行 Agent。
- 父取消向全部后代传播；先停止新派发，再中断模型/浏览器/HTTP，最后写终态。
- 父暂停保留可复用事实和已关闭合同，不把运行中写操作自动重放。

Coordinator 自身另有协调状态：

```text
observing -> deliberating -> assigning -> awaiting_evidence
     ^              |              |              |
     |              v              v              v
     +--------- replanning <--- reviewing <--- evidence_changed
                                      |
                                      v
                                  reducing_terminal
```

这不是固定阶段进度条。任何新事实、反证、Reviewer 缺口或 capability change 都可以回到
`replanning`；但防失控规则和 dedup 保证不会在同一 revision 空转。

## 17. UI 与产品语义

用户可见模块名使用 Nest，运行过程展示：

- 当前阶段、Coordinator 状态和当前活动专家。
- 当前证据缺口、专家提案、支持/质疑关系，以及 Coordinator 为什么选择或暂缓。
- 为什么启动该专家、它拿到的任务切片。
- 三条 lane 是否占用；不得伪装成无限并行。
- 根预算、已预留、已消耗、已归还。
- 覆盖：已覆盖、证据不足、不适用、未覆盖。
- candidate 数量、Reviewer 的 confirmed/rejected/insufficient 数量。
- 停止原因和人工深入建议。

主视图不显示原始 event JSON。调试页可以展示 run tree、assignment、message、tool invocation 和
artifact id，但必须脱敏。

Native 路径中的通用 UI 不得出现“Strix 正在运行”“Strix 已完成”等文案。后端诊断页可以显示
`Native` 或 `Strix compatibility backend`，历史 Strix 任务保留其真实来源。

## 18. 分阶段实施顺序

### Stage 0：Phase 2 门禁

- 先确认可靠性整改全部测试通过。
- 修复 Clippy `-D warnings`，不能带警告进入多 Agent 重构。
- 冻结现有 mock E2E 作为回归基线。

### Stage 1：契约和持久化，不改变执行行为

- 扩展 AgentRole、message、assignment、lane、review 和 evidence graph 类型。
- 增量数据库迁移和 repository 测试。
- 加入 feature policy：`single | shadow | multi`，默认 `single`。
- `shadow` 只计算触发和任务计划，禁止产生额外目标请求或模型调用。

### Stage 2：Coordinator 与串行 Scheduler

- 将当前循环中的全局计划、预算、终态和队列所有权移入 Coordinator。
- 初期 Scheduler 仍串行运行一个兼容 Native 专家，但必须通过 assignment 和 lease。
- 证明结果与现有单 Agent 等价且没有重复计费。

### Stage 2.5：动态证据缺口协商

- 落地 SituationFrame、GapProposal、ProposalAssessment 和 coordination round。
- 先在 `shadow` 下运行；只产生提案与仲裁记录，不创建额外目标合同。
- 证明同一 revision 不空转、提案能合并、低收益建议不消耗执行预算。
- 实施阶段仍按 Stage 顺序推进；运行时不得把 Stage 顺序硬编码成专家流水线。

### Stage 3：SPA/API Mapper

- 接入共享证据图。
- Mapper 只读已保存证据，输出 observed/source-derived/inferred 图谱。
- `shadow` 对比新旧接口图谱，不能主动补请求。

### Stage 4：Evidence Reviewer

- 所有新 Native candidate 必须经 Reviewer。
- 先在 shadow 中对现有 finding 给出决定，观察误拒绝/误确认。
- 切到强制门禁后，执行 Agent 不再直接写 confirmed finding。

### Stage 4.5：Deep Investigator 与往返补证

- 在初轮无突破、证据矛盾或 Reviewer insufficient 时按条件启动。
- Deep Investigator 通过 EvidenceGap 请求其他角色补齐前置事实，不能直接命令或执行。
- Coordinator 能把“侦察补证 -> 验证 -> 深入分析 -> 再补证”组织成有限循环。
- 接入 `docs/NEST_AGENT_COLLABORATION_UI_REQUIREMENTS.md` 定义的自定义 Agent、协作线程和人工干预。

### Stage 5：Identity & Session、Authorization

- 落实 AnonymousOnly/SingleIdentity/IdentitySet。
- 身份隔离、A/B 控制组和对象对照全部用 request ids。
- 双身份不足时返回 insufficient，不伪造 A/B。

### Stage 6：External Surface、Input & Parser、Client-Side

- 使用明确触发谓词和角色能力白名单。
- 受控变体模板替代任意 payload。
- 保持 TargetTouching lane 容量为 1。

### Stage 7：Upload

- 先完成 CleanupContract 和无害 fixture，再开放上传测试。
- Docker 缺失只影响需要容器的能力，不使整个 Nest 失败。

### Stage 8：Business Logic、Concurrency

- 先落地业务状态图，再允许业务写合同。
- Concurrency 始终走 Rust 原生调度器，默认 2、硬上限 3。

### Stage 9：Native 默认与 Strix 退出

- `auto` 切换 Native 前必须观测成功率、完整覆盖率、误报/拒绝率、Token/有效结论和恢复成功率。
- 通用类型、表和 UI 去除 Strix 命名依赖。
- Strix 降为可选 adapter/plugin；不得删除历史产物读取能力。

每个 Stage 都必须可独立编译、测试和回滚 feature policy；不得提交半套并行状态机。

## 19. 必须新增的测试

### 19.1 调度与并发

- 同一 target 永远只有一个 Coordinator。
- TargetTouching 同时最多一个；ReadOnly 和 Reviewer 各最多一个。
- 相同 dedup key 不产生第二个 assignment/run/tool invocation。
- lane 租约过期后只重派未产生确定结果的合同。
- 父暂停、取消和保护停止传播到全部子 run。
- 不同专家可对同一 evidence revision 提交缺口、补充或反驳，由 Coordinator 动态选择下一步。
- 同一 revision 最多两轮协商，无新事实时不会重新提案或再次调用模型。
- 高重叠提案被 merge/duplicate，不生成重复 assignment。
- Agent 自报高 utility 不会绕过代码侧前置条件、预算或风险钳制。

### 19.2 预算

- 多子 Agent 并发申请不能超卖根预算。
- 预留、使用、归还和根累计严格守恒。
- 崩溃恢复不重复扣 Token 或请求。
- 已开始后取消的动作不退款。
- child 硬上限和 root 硬上限都能独立停止。

### 19.3 证据与 Reviewer

- 执行 Agent 提交 candidate 后不能直接出现在 confirmed 主视图。
- Reviewer 无真实 request/invocation/artifact id 时只能 insufficient/rejected。
- candidate revision 变化使旧 review 失效。
- 时间戳、广告、随机字段、缓存和统一 WAF 页被识别为反证。
- rejected、insufficient、阴性证据和低价值线索仍被保存。
- confirmed review 才能投影为 Finding，且投影可幂等重放。

### 19.4 身份

- 匿名 SPA 不显示账号 A/B。
- 单账号不会启动 Authorization，除非存在合法匿名控制组和对象合同。
- 双账号 Cookie/Header/Storage 完全隔离。
- HTTP 200 业务失败不被当成有效登录。
- session refresh 后旧 request record 仍能追溯到当时身份版本。

### 19.5 各专家触发

- 没有上传入口不创建 Upload。
- 没有明确幂等/库存候选不创建 Concurrency。
- 没有业务状态图不创建 Business Logic 写合同。
- source-derived/inferred endpoint 不自动当 observed endpoint。
- Reviewer 缺证据不会自行发请求。
- 初轮无突破时 Deep Investigator 能指出具体缺口，但不会在无新事实时要求重复扫描。

### 19.6 恶意输入与边界

- 页面 prompt injection 不能改变角色、范围、预算、tool capability 或 Reviewer verdict。
- mailbox 不能携带原始 Cookie/JWT/API Key。
- artifact id 越权、跨 scan、跨 attempt 引用被拒绝。
- 子 Agent 构造范围外 URL、重定向或浏览器动作被统一 scope gate 拒绝。
- 角色尝试调用未授权工具时 schema 对模型不可见，执行层仍二次拒绝。

### 19.7 Mock E2E

至少覆盖：

1. 匿名 SPA：Mapper -> External -> Reviewer，无漏洞完成。
2. 单账号：Identity 启动，Authorization 不启动。
3. 双账号越权：Identity -> Authorization -> Reviewer confirmed。
4. 双账号个性化差异：Reviewer rejected。
5. 输入异常有差异但无影响：insufficient。
6. 上传成功且清理成功：可复核；清理失败：转人工且不完整。
7. 业务重复提交：有状态图后验证。
8. 幂等候选：并发 2，检查最终状态；无压力行为。
9. WAF/429：停止目标触碰 Agent，保留只读分析和审计。
10. 中途崩溃：恢复后不重复请求、合同、消息、review 或计费。

## 20. 质量和性能门禁

每阶段都必须运行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

另外必须记录：

- 单 Agent 基线与多 Agent 的目标请求数、模型请求数和 Token。
- 重复合同数，目标必须为 0。
- 预算超卖数，目标必须为 0。
- 无 Reviewer 的 confirmed finding 数，目标必须为 0。
- secret 泄漏到 message/event/model view 的测试命中数，目标必须为 0。
- 恢复后的重复 tool invocation 数，目标必须为 0。
- `shadow` 与当前结果的差异及解释。

不能用“Agent 更多”或“测试通过”代替这些指标。

## 21. 明确禁止的伪完成方式

- 只扩展 `AgentRole` 枚举和 prompt，但所有工作仍在一个循环里完成。
- 子 Agent 最后仍无条件调用 `run_native_agent` 的完整扫描。
- Coordinator 仍直接拥有所有目标工具。
- 让每个专家复制一份 Tool Broker、scope 或预算实现。
- 用 `tokio::spawn` 数量表示调度完成，却没有 lane、租约和去重。
- 用 mailbox 传长文本、原始响应或聊天历史。
- 让 Reviewer 与执行 Agent 使用同一 run、同一 prompt 或同一写权限。
- 通过隐藏低危/阴性数据制造“只保留严重漏洞”的表面效果。
- 多 Agent 失败后自动回退 Strix 并合并两套结果。
- 为通过测试降低证据门禁、放宽范围或扩大并发。
- 在代码中继续增加通用 `strix_*` 状态、设置、数据库列或 UI 名称。

## 22. 完成定义

只有同时满足以下条件，才允许声明多 Agent 重构完成：

1. Native target attempt 有且只有一个 Coordinator。
2. 至少 SPA/API Mapper、Identity & Session、Authorization 和 Evidence Reviewer 是真正独立 run。
3. 专家由确定性证据谓词按需触发，不会一次性全部启动。
4. 专家能围绕 EvidenceGap 提案、补充和质疑；Coordinator 基于结构化收益/成本动态重规划，而非固定逐个执行。
5. TargetTouching / ReadOnlyAnalysis / Review 三条 lane 的容量约束有数据库与并发测试证明。
6. 合同有唯一 owner，跨 Agent 不重复请求。
7. 子预算租约不会超卖，暂停/崩溃/重放不重复计费。
8. Agent 仅通过证据图、assignment 和类型化 mailbox 协作。
9. 执行 Agent 无法直接发布 confirmed finding。
10. Reviewer 无目标工具，并对 candidate revision 给出可审计决定。
11. confirmed finding 绑定真实请求、工具调用、差异 artifact、身份/对象和 Reviewer decision。
12. 匿名、单身份、双身份数据模型和 UI 明确分离。
13. Upload 有强制清理合同；Concurrency 默认 2、硬上限 3。
14. 所有 Agent 无 shell，所有动作经统一 Tool Broker、scope、预算和审计。
15. Fresh/ContinueIncomplete、冻结计划、Secret 脱敏和唯一终态没有回归。
16. `agentBackendPolicy=native` 全程不启动 Strix。
17. 通用业务状态和 Nest UI 不依赖 Strix 命名；Strix 仅剩兼容适配器与历史来源标识。
18. 全量 Rust 测试、Clippy、前端构建、diff 检查及本文 mock E2E 全部通过。
19. 自定义 Agent 经过 role/capability/sandbox 校验，不能通过 prompt 自行扩权。
20. Deep Investigator 能驱动有界往返补证；人工可以观察协作和安全干预而不绕过范围、预算或 Reviewer。

## 23. 实现者最终回复格式

最终回复必须逐项报告：

1. 修改文件与新增数据库迁移。
2. Coordinator、Scheduler、lane 和 assignment 的真实实现位置。
3. 已实现角色、触发谓词、输入、输出和能力白名单。
4. SituationFrame、GapProposal、assessment、仲裁评分和防空转机制。
5. 证据图节点、边和旧结果面投影方式。
6. mailbox schema、幂等和 ack/replay 语义。
7. 根预算与子租约的守恒证明。
8. 合同 owner、去重键和租约过期重派方式。
9. Reviewer 门禁和 confirmed finding 的证据链。
10. 身份模型、隔离和 Authorization 前置条件。
11. Upload 清理与 Concurrency 上限。
12. 暂停、取消、崩溃和 ContinueIncomplete 如何恢复整棵 run tree。
13. Strix 剩余依赖清单，以及哪些仍是兼容需要。
14. 新增测试、完整命令结果和指标对比。
15. 尚未完成的项目、影响与下一阶段建议。
16. 自定义 Agent manifest、能力包、沙箱路由、协同台和人工干预的实现状态。

任何一项未完成必须明确写“未完成”，不得用“基本完成”“已兼容”或“后续优化”替代。
