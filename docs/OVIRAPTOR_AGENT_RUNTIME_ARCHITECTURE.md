# Oviraptor 自研 Agent Runtime 架构与开发规范

## 1. 设计原则

Oviraptor 不应重新实现一个不可控的“大号自主 Agent”。正确边界是：

- Rust 编排器拥有任务状态、预算、身份、覆盖、停止和最终结论。
- 模型只在被分配的上下文中做规划、判断与总结。
- 模型不能直接获得 Cookie、API Key、文件系统、任意网络或任意 Shell。
- 所有动作必须经过 Tool Broker 的参数校验、策略决策、预算预留和结果审计。
- 多 Agent 通过持久化、类型化消息协作，不共享一段无限增长的聊天历史。
- 沙箱是最后一层防线；第一层安全边界必须是“不给任意能力”。
- Strix 作为迁移期后端保留，原生执行器达到功能等价后再逐步退出。

## 2. 总体架构

```text
Task/UI
  │
  ▼
Agent Runtime API
  │
  ├── Execution Planner ── AgentExecutionPlan / Coverage DAG / Budget Lease
  │
  ├── Deterministic Orchestrator
  │     ├── single-agent loop
  │     ├── multi-agent scheduler
  │     ├── retry/resume/cancel
  │     └── terminal-state reducer
  │
  ├── Model Gateway
  │     ├── cloud OpenAI-compatible
  │     ├── local OpenAI-compatible/oMLX
  │     ├── context compactor
  │     └── usage/rate/error classification
  │
  ├── Tool Broker + Policy Engine
  │     ├── evidence/query tools
  │     ├── browser/CDP tools
  │     ├── HTTP/replay/identity tools
  │     ├── discovery/verification tools
  │     └── optional sandboxed external tools
  │
  ├── Sandbox Runtime
  │     ├── NativeRestricted (default)
  │     ├── Container (optional Docker/Podman)
  │     └── RemoteWorker (future)
  │
  └── Event / Evidence Store
        ├── append-only events
        ├── checkpoints
        ├── typed agent mailbox
        ├── tool invocation audit
        └── immutable request/response artifacts
```

### 控制面与执行面

控制面只能由 Rust 编排器修改：任务状态、预算、Agent 生命周期、会话句柄、工具授权、覆盖账本、终态。

执行面负责运行经过授权的具体工具。执行面只能返回事实和 artifact 引用，不能直接把任务标记成完成或漏洞确认。

## 3. 代码目录

在 `src-tauri/src/agent_runtime/` 建立真正的 Rust module，逐步替换当前扁平 `include!`，但迁移期间不得一次性移动所有旧文件：

```text
agent_runtime/
  mod.rs
  contract.rs
  orchestrator.rs
  scheduler.rs
  reducer.rs
  checkpoint.rs
  budget.rs
  progress.rs
  model/
    mod.rs
    gateway.rs
    openai_compatible.rs
    context.rs
    usage.rs
  tools/
    mod.rs
    broker.rs
    registry.rs
    policy.rs
    evidence.rs
    http.rs
    browser.rs
    identity.rs
    discovery.rs
    finding.rs
  sandbox/
    mod.rs
    native_restricted.rs
    container.rs
    process.rs
  multi_agent/
    mod.rs
    mailbox.rs
    roles.rs
    leases.rs
  strix_adapter.rs
```

当前 `commands/agent_runtime.rs` 保留为迁移入口，最终只负责从现有扫描参数构造新 runtime 的 `AgentExecutionPlan`。

## 4. 核心契约

```rust
pub enum AgentBackendKind {
    Native,
    Strix,
}

pub enum AgentRole {
    Coordinator,
    EvidenceTriage,
    ContractVerifier,
    IdentityComparator,
    BusinessFlowAnalyst,
    AttackChainCorrelator,
    FinalReviewer,
}

pub enum SideEffectClass {
    ReadOnly,
    ControlledWrite,
    IrreversibleBlocked,
}

pub struct AgentExecutionPlan {
    pub scan_id: String,
    pub attempt_number: i64,
    pub target_url: String,
    pub mode: ScanMode,
    pub backend: AgentBackendKind,
    pub coverage: CoveragePlan,
    pub budget: BudgetEnvelope,
    pub identities: Vec<IdentityHandle>,
    pub allowed_origins: Vec<OriginScope>,
    pub stop_policy: StopPolicy,
}

pub trait ModelGateway: Send + Sync {
    fn complete(&self, request: ModelRequest, cancel: &CancelToken)
        -> Result<ModelResponse, ModelError>;
}

pub trait AgentTool: Send + Sync {
    fn descriptor(&self) -> ToolDescriptor;
    fn validate(&self, ctx: &ToolContext, input: &JsonValue)
        -> Result<ValidatedToolInput, ToolRejection>;
    fn execute(&self, ctx: &ToolContext, input: ValidatedToolInput)
        -> Result<ToolOutput, ToolError>;
}

pub trait SandboxRuntime: Send + Sync {
    fn start(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError>;
    fn execute(&self, handle: &SandboxHandle, call: SandboxedCall)
        -> Result<SandboxResult, SandboxError>;
    fn stop(&self, handle: SandboxHandle) -> Result<(), SandboxError>;
}
```

模型返回的 `tool_call` 永远不能直接执行。调用链必须是：

```text
Model tool_call
  -> JSON Schema 校验
  -> Tool Broker 查找能力
  -> Policy Engine 判断范围/身份/副作用
  -> Budget Manager 预留预算
  -> Sandbox/Native tool 执行
  -> Artifact Store 保存原始证据
  -> Result Compactor 生成模型可读摘要
  -> Progress Engine 判断是否有新增价值
  -> Event Store 记录事件
```

## 5. 模型网关

### 统一模型接口

继续复用现有模型 Profile，不要求用户新增第二套配置。模型网关负责：

- OpenAI-compatible 请求和 tool calls。
- 云端、本地模型严格分流。
- API Key、Base URL、代理、模型名和输出上限解析。
- usage 统计和服务端不返回 usage 时的估算。
- streaming 日志，但数据库只写聚合事件，不能逐 token 写 SQLite。
- 认证失败、上下文超限、限流、临时 5xx、网络失败和取消的稳定分类。
- 本地模型串行，推理中不触发普通响应超时。
- 云端模型使用连接超时、响应超时和单次 provider retry。

### 上下文管理

禁止把全部扫描历史不断追加给模型。上下文分四层：

1. 固定系统规则和工具 schema hash。
2. 当前 Agent 的角色、预算租约和任务切片。
3. 结构化事实摘要：已确认、已排除、待验证、覆盖缺口。
4. 最近两轮工具输入输出摘要。

原始 HTTP、JS、截图和大 JSON 只保存在 artifact store，通过 opaque artifact id 按需查询。

达到上下文水位时，由 Rust `ContextCompactor` 生成确定性摘要，不再让模型花一次调用“总结聊天记录”。

## 6. Tool Broker

### 能力描述

每个工具必须声明：

- `name` 和 `schema_version`
- 严格输入 JSON Schema
- `side_effect_class`
- 需要的 capability
- 是否需要身份
- 支持的协议和方法
- 最大执行时间、最大输出字节、并发键
- artifact 类型
- 结果压缩器
- 进展字段提取器

### 首批工具

```text
evidence.search
evidence.get_artifact_summary
http.replay
http.compare
identity.compare
browser.perform_action
browser.capture_network_delta
discovery.targeted_paths
hypothesis.update
coverage.update
finding.propose
target.finish
```

后续外部扫描器不能直接暴露成 shell。每一个都必须包装成固定参数的 adapter，例如：

```text
nuclei.run_templates(template_ids, target_ids)
httpx.probe(target_ids, options_profile)
katana.crawl(target_id, depth_profile)
```

模型不得提供二进制路径、shell 字符串、输出路径或任意环境变量。

### 身份和 Secret

- 模型只能看到 `identity_handle`，不能看到 Cookie/JWT/API Key。
- Tool Broker 在执行时从任务级 vault 注入身份材料。
- A/B 分别建立独立 client、cookie jar 和浏览器 profile。
- 任何工具结果在进入模型前都要执行 secret redaction。
- Secret 不写入 prompt、普通日志、artifact 索引和 agent message。

## 7. 沙箱设计

### 默认：NativeRestricted

这是 macOS 内部应用最实际、最稳定的默认模式：

- Agent 没有 shell 工具。
- 所有网络通过 Rust Tool Broker。
- 所有 URL 都经过 origin allowlist 和 evidence relation 校验。
- 文件访问只允许 artifact id，不允许路径。
- 外部进程使用参数数组，不拼 shell 命令。
- 独立进程组、超时、取消、stdout/stderr 上限和资源采样。
- 每个 `scan_id/attempt/target` 有独立 scratch 目录。

这种 capability sandbox 比单纯依赖 macOS `sandbox-exec` 更可靠。不要把已经弱化或弃用的系统沙箱当唯一边界。

### 可选：Container

只有确实需要第三方 CLI、浏览器插件或不受信任解析器时使用：

- rootless Docker/Podman。
- 只读 rootfs。
- 非 root 用户。
- drop all capabilities。
- 禁止 privileged 和 host network。
- workspace evidence 只读挂载，scratch 单独可写。
- CPU、内存、进程数、文件数和执行时间限制。
- 默认禁止网络；网络调用仍经 Oviraptor 代理或显式目标 allowlist。
- 容器日志和退出原因回写事件系统。

Docker 不存在时必须自动使用 NativeRestricted，而不是让整个扫描失败。

### 未来：RemoteWorker

接口先预留，但当前不要实现。远程执行必须使用双向认证、一次性任务令牌、artifact hash 校验和按目标授权。

## 8. 确定性编排器

编排器不是另一个无限自主模型。它是 Rust 状态机：

```text
Prepared
 -> EvidenceTriage
 -> CoveragePlanning
 -> ContractExecution
 -> DifferentialVerification
 -> Correlation
 -> FinalReview
 -> Completed / BoundedCompleted / Limited / Failed / Cancelled
```

### 任务队列

每个待执行项是 `InvestigationContract`，包含：

- 稳定 contract key
- 覆盖族
- method/origin/path
- 身份要求
- 输入来源和 evidence refs
- 最大尝试次数
- 副作用类型
- 控制组定义
- 成功/失败/无效判据
- cleanup/rollback
- 优先级和预计成本

模型可以建议合同优先级或补充参数，但不能凭空把字符串候选升级成可执行合同。

### 进展判断

只有这些变化算进展：

- 新正式端点
- 新参数或新响应结构
- 新身份差异
- 新状态转换
- 新验证结论
- 新覆盖族完成
- 新可复现漏洞证据

Token 增加、重复工具、重复状态码和模型重新描述旧信息不算进展。

软预算只在达到 `no_progress_window` 后收口；硬预算无条件停止。

## 9. 多 Agent 编排

### 不要默认多 Agent

多 Agent 只在任务可以拆成互不重复的合同，并且剩余预算足够时启用。简单页面使用单 Agent，避免协调成本超过测试本身。

建议角色：

- `Coordinator`：Rust 编排器，不是必须调用模型。
- `EvidenceTriage`：一次性压缩和排序证据。
- `ContractVerifier`：验证一组互不重复的 API/假设。
- `IdentityComparator`：专门处理匿名/A/B 的同接口差异。
- `BusinessFlowAnalyst`：处理状态机和业务流程。
- `AttackChainCorrelator`：仅在至少两个已验证证据节点时启用。
- `FinalReviewer`：检查反证、影响、置信度和结论一致性。

### Spawn 条件

只有全部满足才创建子 Agent：

- 至少两个独立合同分区。
- 分区之间不会操作同一个写状态。
- 父级预算能够给每个子级预留 Token/请求租约。
- 本地模型当前没有其他推理；本地默认仍串行。
- 目标没有 WAF/限流信号。

### 通信协议

Agent 不直接互相追加自然语言聊天。统一使用 mailbox envelope：

```rust
pub struct AgentMessage {
    pub id: String,
    pub run_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub kind: AgentMessageKind,
    pub correlation_id: String,
    pub dedup_key: String,
    pub payload: JsonValue,
    pub artifact_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

pub enum AgentMessageKind {
    Assignment,
    EvidenceProduced,
    HypothesisUpdated,
    CoverageUpdated,
    CapabilityUnavailable,
    BudgetReturned,
    Completed,
    Failed,
    Cancelled,
}
```

约束：

- payload 必须经过对应 schema 校验。
- 所有消息写库后再投递内存 channel。
- `dedup_key` 保证恢复后不会重复处理。
- Agent 只能写事件和消息，最终任务状态只能由 reducer 单写。
- 父 Agent 停止时递归取消所有子 Agent。
- 每个 Agent 有 lease 和 heartbeat；超时后回收预算并重新排队未确认合同。

## 10. 预算租约

总预算由 Oviraptor 持有。子 Agent 只能获得租约：

```text
Target hard budget
  - coordinator reserve
  - verifier lease A
  - verifier lease B
  - final review reserve
```

- Spawn 前原子预留。
- 每次模型调用前再次检查。
- 未使用预算在 Agent 结束时归还。
- 任何子 Agent 不能超过父级硬上限。
- 云端可有限并行，本地模型最多一个 in-flight 请求。
- 同一 host 的网络工具独立设置并发和节流，不由模型决定。

## 11. 事件、数据库与恢复

不要把高频事件全部塞进 `sentinel_checkpoints`。新增表：

### `agent_runs`

```text
id, scan_id, attempt_number, target_url, backend, role,
parent_run_id, status, plan_hash, evidence_hash,
soft_token_budget, hard_token_budget, used_tokens,
soft_request_budget, hard_request_budget, used_requests,
lease_expires_at, started_at, finished_at, terminal_reason
```

### `agent_events`

```text
id, run_id, sequence, event_type, payload_json, artifact_refs_json, created_at
UNIQUE(run_id, sequence)
```

### `agent_messages`

```text
id, run_id, from_agent, to_agent, kind, correlation_id,
dedup_key, payload_json, artifact_refs_json, delivered_at, acknowledged_at
UNIQUE(run_id, dedup_key)
```

### `tool_invocations`

```text
id, run_id, tool_name, tool_version, contract_key, identity_handle,
input_hash, policy_decision, status, progress_signature,
request_artifact_id, response_artifact_id, started_at, finished_at, error_class
```

### `agent_snapshots`

```text
run_id, last_sequence, schema_version, snapshot_json, updated_at
PRIMARY KEY(run_id)
```

SQLite 使用 WAL。状态 reducer 是单写者；模型调用和工具执行可以并行，但只能提交事件，不能并发直接修改最终状态。

恢复流程：

1. 读取 snapshot。
2. 重放 snapshot 之后的 events。
3. 将 `running` 但 lease 已过期的调用标记为 interrupted。
4. 只重新排队没有确定工具结果的合同。
5. 重新计算预算，不能重复计费或重复执行写操作。

## 12. Artifact Store

原始证据使用 content-addressed storage：

```text
artifacts/<sha256-prefix>/<sha256>
```

元数据包含：MIME、大小、来源工具、scan/attempt/target、身份句柄、redaction 状态和 hash。

- 数据库和 Agent message 只保存 artifact id。
- 相同内容自动去重。
- 大响应只向模型返回结构摘要和截断预览。
- UI 需要原始内容时按 artifact id 加载。
- Secret redaction 后的展示 artifact 与原始加密 artifact 分离。

## 13. Policy Engine

决策输入：

- 工具 descriptor
- contract
- target scope
- identity handle
- 当前预算和速率
- WAF/限流状态
- side effect class
- task capability manifest

输出只能是：

```text
Allow
Deny(reason)
RequireContract(reason)
RequireCleanup(reason)
StopTarget(reason)
```

规则至少包括：

- 普通 GET/HEAD/OPTIONS 和已采集请求的只读重放自动允许。
- POST 不等于危险，也不等于自动允许；根据合同和副作用判断。
- 删除、支付、外部消息、持久权限修改、DoS 永久禁止自动执行。
- 受控写必须有 cleanup、尝试上限和测试数据。
- 401/403 是边界证据，不等于 session 失效。
- 登录重定向且没有成功业务请求才可以使对应 session 失效。
- 明确 WAF/CAPTCHA/bot challenge/持续 429 停止主动请求。
- 非目标 CDN、遥测、广告、设备指纹只做关联服务记录。

## 14. Prompt Injection 与不可信内容

- 页面文本、JS 注释、HTTP 响应、文件内容和 artifact 都标记为 untrusted data。
- 目标内容不能修改工具 schema、预算、身份、范围或系统规则。
- 模型看到的是 JSON 字段，不把网页内容拼进 system prompt。
- 工具名、参数名和 shell 参数永远不能来自目标返回值直接执行。
- 模型建议的新 URL 必须重新通过 origin/evidence relation 校验。
- 模型输出漏洞前由 FinalReviewer 检查控制请求、测试请求、差异、影响和反证。

## 15. 终态 reducer

所有后端统一使用：

```text
Completed          所有计划合同已形成可用结果，覆盖账本已收口
BoundedCompleted   达到计划硬边界，但已有可用证据和明确未覆盖项
Incomplete         可恢复的外部/模型/能力问题，仍有未执行合同
Limited            WAF、挑战、限流或明确策略边界
Failed             配置、内部一致性或不可恢复执行错误
Cancelled          用户取消
```

“没有发现漏洞”可以是 Completed。`Incomplete` 不能因为“零漏洞”产生。UI 只显示 reducer 的最终状态，不拼接历史 attempt 的旧状态。

## 16. 可观测性

UI 展示用户能理解的信息：

- 当前阶段和 Agent 角色。
- 当前覆盖族、完成/未覆盖数量。
- 当前合同和最近新增证据。
- 模型调用、Token、工具调用和预算比例。
- 子 Agent 数、正在等待什么、为什么创建。
- 软预算扩容原因。
- 最终停止原因和人工深入建议。

原始 event JSON 放调试页，不放主界面。

## 17. 迁移步骤

### Phase 0：契约与事件系统

- 新建 module、数据库迁移、类型、reducer、snapshot/event replay。
- 不改变现有扫描行为。

### Phase 1：模型网关

- 从现有 Strix 配置解析中抽出统一 Profile。
- 完成云端/本地调用、工具调用解析、usage 和错误分类。
- 使用 mock provider 测试。

### Phase 2：Tool Broker

- 首先接入 evidence、HTTP replay、identity compare、finish。
- 所有工具都必须有 schema、policy、artifact 和 progress signature。

### Phase 3：单 Agent 原生闭环

- 标准模式跑通完整循环。
- 暂不启用多 Agent。
- 写入现有漏洞、证据、覆盖和任务 UI。

### Phase 4：Sandbox

- NativeRestricted 完成进程、网络、文件、Secret 边界。
- Container 作为可选增强，Docker 缺失自动降级。

### Phase 5：多 Agent

- 以 `docs/NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md` 和 `docs/NEST_AGENT_COLLABORATION_UI_REQUIREMENTS.md` 为实现与验收合同。
- 采用“一个 Coordinator + 按证据触发的专家 + 独立 Evidence Reviewer”，禁止无限 spawn。
- 运行时不是固定角色流水线：每个 evidence revision 发布 SituationFrame，专家提交证据缺口、依赖、收益/成本提案和结构化质疑，由 Coordinator 动态仲裁下一步。
- mailbox、assignment、共享证据图、budget/capability lease、heartbeat、合同唯一 owner、parent-child cancellation 必须同时落地。
- 调度固定为三条 lane：最多一个目标触碰 Agent、一个只读分析 Agent和一个 Reviewer；同一合同不得被两个 Agent 重复执行。

### Phase 6：标准/深度覆盖

- 身份差异、业务流、定向发现、攻击链与最终复核。
- 使用覆盖收益评估是否创建子 Agent。

### Phase 7：Strix 退出

- `auto` 默认 Native。
- 连续多个版本观察 Native 成功率、发现率、Token/有效结论。
- Strix 变成可选兼容插件，最后才删除其强依赖。

## 18. 测试矩阵

必须使用本地 mock，不依赖生产网站：

- 匿名 SPA：首屏无 XHR，动作后出现业务 API。
- 单账号：会话有效、401/403 边界、Token 更新。
- 双账号：相同接口不同对象和字段；验证身份完全隔离。
- 动态验证码：登录由人工完成，后续自动使用任务级会话。
- WAF/挑战/持续 429：立即停止主动动作。
- 目录发现：只使用业务词，噪音资源不进入正式 API。
- 无漏洞：完整覆盖后 Completed。
- 模型慢响应：本地不误超时，云端按策略停止。
- 上下文超限：压缩后继续或明确 Incomplete。
- 暂停/恢复：无重复工具、无重复计费、无写操作重放。
- 进程崩溃：event replay 后恢复未完成合同。
- 多 Agent：合同不重复、预算不超卖、父级取消可传播。
- Prompt injection：网页要求修改范围或读取 Secret 时必须被拒绝。

每阶段执行：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
git diff --check
```

## 19. 给实现者的硬性限制

- 不得以新增 trait 但内部继续全部调用 Strix 作为完成。
- 不得把任意 shell 暴露给模型。
- 不得把 Cookie/JWT/API Key 写进 prompt 或 Agent message。
- 不得让多个 Agent 直接并发修改任务终态。
- 不得让重试同时启动 Native 和 Strix。
- 不得把全部 HTTP/JS/历史消息塞入上下文。
- 不得用 Docker 是否安装决定应用能否扫描。
- 不得把未覆盖、响应不同或模型判断直接当成漏洞。
- 不得在一个阶段结束时留下无法编译的中间状态。

## 20. 完成定义

架构完成至少满足：

1. `agentBackendPolicy=native` 时完全不启动 Strix。
2. 原生 Agent 能在 mock Web 上完成规划、工具调用、证据保存、覆盖收口和结果入库。
3. 单 Agent 稳定后，多 Agent 能按合同分区协作且不重复执行。
4. NativeRestricted 无 Docker 也能运行；Container 可选。
5. 暂停、取消、崩溃和继续都能通过事件与快照恢复。
6. 云端、本地模型严格分流，Token 和成本完全可追溯。
7. UI 不展示原始 JSON，而是展示合同、证据、覆盖、预算和终态。
8. Strix 只剩可选适配器，不再出现在通用业务状态、数据库契约和 UI 文案中。

## 21. 可靠性整改后的实际契约

本节记录原生 Runtime 可靠性整改完成后，代码里真实存在的契约与唯一实现位置。它与
`NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md` 的 §3–§12 一一对应；该文件是验收合同，
本节是实现事实。

### 21.1 attempt 继承与冻结计划

`NativeAgentState` 记录 `parentAttemptNumber`、`resumeKind`（`initial` / `fresh` /
`continue_incomplete`）和 `inheritedCheckpointSchema`。父 attempt 与执行模式来自
`sentinel_scan_attempts.execution_mode`，由 `agent_attempt_lineage` 单点解析。

- `Fresh` 不继承任何事实；`ContinueIncomplete` 继承父的冻结计划、已执行请求账本、
  已完成/耗尽契约、覆盖族、预算消耗、待处理队列与发现轮次，只清掉父的瞬态字段
  （运行中标记、旧错误、旧心跳、旧终态文案），原始终态原因保留在历史行里。
- 续跑不重新计算计划：`agent_frozen_plan` 只读父 attempt 的 `agent_execution_plan`
  快照并刷新 attempt 号；父快照缺失即拒绝续跑。
- checkpoint 不兼容时返回明确的 `NativeStateRejection`，终态是
  `resume_incompatible`，文案固定为“续跑状态不兼容，需要重新执行（…）”，
  绝不静默转成 fresh，也不显示成普通模型失败。

### 21.2 checkpoint 迁移

`migrate_native_agent_state` 负责 v1→v2：请求账本不猜测（缺失即
`resume_evidence_incomplete`），契约结果由旧集合映射，结论与确认漏洞按保守默认推导。
更高版本返回 `checkpoint_version_too_new`，损坏返回 `checkpoint_corrupted`。迁移成功后以
v2 写回、把原始 JSON 归档到 `native_agent_state_legacy` 并追加
`checkpoint_migrated` 事件。

### 21.3 唯一范围判定

`agent_scope_assess` 是所有 URL 的唯一判定入口：`replay_http`、`compare_identities`
两侧、`targeted_discovery` 候选与验证、`browser_action` 入口与 CDP 观测到的每一条请求、
以及重定向目标。分类为 `authorized_business_api` / `authorized_document` /
`authorized_static` / `third_party_required` / `telemetry_or_noise` / `out_of_scope`。
重定向规则：同 origin 放行；冻结范围内放行并记录；身份提供方跳转只用于浏览器展示与会话
采集，不自动测试；其余跨域返回 `redirect_out_of_scope`。

### 21.4 双视图与脱敏

`agent_execute_tool` 返回 `ToolExecutionResult { model_view, artifact_id, invocation_id }`，
只有 `model_view` 能进入对话历史。`agent_model_view` 用带 per-run salt 的
`<redacted:kind:fingerprint>` 替换凭证与个人值；同一次运行内同一秘密指纹一致，跨运行不可关联。
原始字节写在 `agent-http/<slot>.body`（有请求体时另有 `<slot>.request`），元数据
`<slot>.json` 只保存脱敏视图与文件指针。云端与本地模型走同一条脱敏路径。

技术债：payload 文件目前是 0600 权限 + 任务私有目录，尚未做任务级密钥或系统钥匙串派生密钥
加密；代码中以 `TECH DEBT (§6.4)` 标注。

### 21.5 参数 schema 校验

`compile_schema` + `CompiledSchema::validate` 是唯一的工具参数校验器，`agent_schema_registry`
在运行开始时一次性预编译。支持 `type`、`required`、`additionalProperties`、`properties`、
`items`、`enum`、`const`、`minimum/maximum`、`minLength/maxLength`、`minItems/maxItems`、
嵌套结构与 `format: url`。拒绝结果带稳定错误码与 JSON 路径（`$.coverage[0].reason`），
不执行工具、不消耗目标预算，重复相同参数错误计入无进展。

### 21.6 A/B 字段分类

`agent_field_class` 把差异字段分为 volatile / secret / subject / authorization / business /
unknown 六类，取消了 `ends_with("id")` 这种粗规则。`roleId`、`tenantId`、`departmentId`、
`permissionId` 等一律进入 `materialAuthorizationDiffs`，不会落入忽略桶或归属桶；
`userId`、`accountId` 进入 `subjectDiffs`，只用于确认归属。`agent_ab_pair_evidence` 校验
A/B 的硬条件（两条不同且完整的请求记录、同 method 与规范化 host+path、业务参数集合一致、
两个不同身份句柄、两侧都有 status/结构指纹/artifact id）。任一侧缺失即
`insufficient_evidence`。

### 21.7 覆盖证据账本

`CoverageEvidence { id, family, evidenceKind, contractKey, toolInvocationIds,
requestRecordIds, verdictId, result, reasonCode }` 由 `credit_coverage` 从已执行链路推导，
`finish_target` 只能引用其中真实存在的 id；越高的声明会被降级并出现在
`unsupportedCoverageClaims`。规则化的不足证据包括 `anonymous_only`、`no_identity_contrast`、
`login_page_only`、`no_reflection_point`、`discovery_round_closed`、`no_response_structure`。
已覆盖 / 证据不足 / 不适用 / 未覆盖在结果里是四个独立列表，任何一项都不写成“未发现风险”。

### 21.8 确认漏洞绑定

`record_hypothesis_result` 的确认结论必须引用 `controlRequestId`、`testRequestId` 与
`responseDifferenceArtifactId`。`agent_confirmed_finding_gate` 校验：两条 id 存在且不同、
属于本执行链账本、与该假设的契约或覆盖族关联、身份或参数确有对照、差异 artifact 存在且
关联的正是这两条响应、契约至少执行一次、两侧都不是登录重定向 / 缓存命中 / WAF 或统一拦截页、
影响与复现步骤非空。任一条件不满足即降级为 `insufficient_evidence`，并在结果里给出
`downgradedFrom` 与 `missingEvidence`。文本包含匹配已删除。

### 21.9 终态与单一事实来源

`agent_runtime::contract::TerminalState::to_sentinel_status` 是目标终态词汇的唯一来源：
`completed`、`completed_with_gaps`、`paused`、`cancelled`、`protected_stop`、
`resume_incompatible`、`persistence_failure`、`failed`。`AgentTargetOutcome::terminal_status` 只做委派，
`record_agent_target_outcome` 是唯一写库入口。普通 401/403、未登录、零漏洞、缺少身份 B
都不是 `failed`。执行计划公共类型只保留 `commands` 层的一份，Phase 0 的重复定义已删除；
`NativeAgentState` 是 canonical snapshot 的序列化兼容层，`agent_runs` / `agent_events` /
`agent_snapshots` 每次提交同步写入，两者计数必须一致（由测试断言）。

### 21.10 关键持久化 fail-closed

`write_agent_checkpoint`、`NativeAgentState::persist`、`persist_agent_usage` /
`persist_agent_evidence` / `persist_agent_coverage`、以及工具事务里的
`begin_tool` / `finish_tool` / `model_round` / `checkpoint` 全部返回 `Result`。
原生循环在任何提交边界拿到 `Err` 就调用 `persistence_stop`，立刻以
`persistence_failure` 结束本轮，不再发起下一次模型请求或目标请求；
`rejection_outcome` 同样把 `PersistenceFailed` 与该代码区分开。
数据库 busy 走 `with_db_retry` 的有界重试，其他错误立即失败。
`agent_terminal` 一类的收尾投影发生在尝试结束之后，没有后续消耗，
因此它的失败写进 runner 日志而不改变终态。UI 侧
`agent_terminal_status_label`、`presentation.ts` 与扫描汇总都单独显示
“本地记录失败，已停止以避免重复消耗”，且该状态不在续跑集合里。

### 21.11 可取消的模型传输

`agent_runtime::model::transport` 是唯一的模型 HTTP 出口。它在一个私有的
current-thread runtime 上驱动 `reqwest` 异步请求，`tokio::select!` 的另一支按
100 毫秒节奏查询取消谓词；取消命中时请求 future 被丢弃，响应体与其连接随之释放，
worker 线程被 `join`，因此 `send` 返回即代表传输已经落定，循环才允许写终止事件。
`CancelObservation` 携带 `cancel_requested_at`、`transport_closed_at`、
`settle_millis` 与发起方（本任务取消 / 下游对端离开）。网关把最近一次观测存在
`take_cancel_observation` 里，`note_transport_cancel` 以
`AgentEventKind::TransportCancelled` 落入 `agent_events`。云直连与 LLM Hook 的
HTTPS 转发共用这一出口，`reqwest::blocking` 的游离线程模型在模型路径上已不存在；
提供商重试等待 `wait_or_cancel` 也是可中断的。§6.3 的验收由三条测试覆盖：
挂起 60 秒的连接在 2 秒内落定、服务端观察到连接断开、被取消的轮次不产生
`model_round_completed`、不计费、也不会重发第二次请求。

### 21.12 验收 fixture 与持续限流

`src-tauri/src/commands/agent_acceptance.rs` 是 §11 的本地验收应用：一个进程内
HTTP 站点，15 个场景（匿名 SPA、单账号、两个平权账号、权限更高账号、登录失效、
普通 401/403、WAF 挑战、持续 429、隐藏 GET 接口、转义反射、可执行反射、越权正负例、
错误信息泄漏、source map、遥测与静态噪音）全部由请求行与账号 Cookie 确定性地回答。
它只被确定性 Mock 模型经真实工具与真实 HTTP 驱动，不伪造任何工具结果。
共享夹具 `agent_tests` 的 `agent_harness` / `model_round` / `retarget_model` /
`assert_agent_surface` 等只放开 `pub(super)` 可见性，不复制第二套 fixture。

目标侧的限流规则也在这里定死：单次 HTTP 429 只是推迟的活，同一 attempt 内连续两次
（`AGENT_PERSISTENT_RATE_LIMITS`）才置 `runtime.protected_stop`，以
`PERSISTENT_RATE_LIMIT` 收口并计入熔断；任何非 429 响应清零计数，
所以慢站点的偶发限流不会被当成拒绝服务。

### 21.13 覆盖缺口的独立统计与展示

`AgentPipelineTally` 把 `completed` 与 `completed_with_gaps` 分成两个计数器：
`BoundedCompleted` 只进后者。`finalize` 因此在“全部目标都带缺口收口”时写
`本轮执行完成但存在覆盖缺口：…带覆盖缺口完成 N…缺口未计入无异常完成`，
不再复用“无异常中断”的句子；混合场景的两种汇总也各自带上缺口计数。
`repair_associated_scan_state` 的 Web 汇总同样单独统计 `completed_with_gaps`，
不再把它并进“待补充验证”。结果面只有当每个必需 family 要么被证据覆盖、要么被
证据支持地标为 `not_applicable` 时才是 `Completed`；否则由 reducer 给
`BoundedCompleted`（无证据支撑则是未完成）。
UI 侧 `agentGapLabel` 区分“未覆盖 / 证据不足 / 因预算未完成 / 不适用”，
漏洞空态在账本未收口时改说“验证尚未收口……这不等于没有漏洞”，
并用 `.empty-inline.warning` 与“已完成覆盖账本且未发现漏洞”在视觉与数据上分开。

### 21.14 中立模型配置

模型设置的权威名字不再绑定第三方引擎：`modelProfiles` / `activeModelProfileId` 加扁平
`modelDeployment` / `modelApiBase` / `modelApiKey` / `localApiKey`，配合已有的
`agentBackendPolicy`。`db::migrate_neutral_model_settings` 在启动时一次性把
`strixLlmProfiles`（或更早的 `strixLlm/strixApiBase/strixApiKey` 三件套）提升为新键，
旧键保留不动供旧版本读取，但 Rust 与设置界面都只写新键；`ConfigDialog` 保存时会移除
已被提升的旧键，避免同一份 profile 出现两份互相矛盾的答案。
读取顺序固定为：新 profile 字段 → 新扁平键 → 旧 `strixLlm*` → 环境变量/CLI（仅云端一侧）。
本地部署只认 `localApiKey`，缺省为占位 `local`，永远不会借用云侧密钥或环境回落。
“测试模型”现在必须带一份工具声明发起一次请求（`tools` + `tool_choice`），
端点若按 `looks_like_unsupported_tools` 拒绝工具，直接报“不支持工具调用，无法用于原生 Agent”，
这正是 §7.2 能力门禁所需的信号。当 `agentBackendPolicy` 为 native 时，Strix 可执行文件、
运行目录与第三方引擎预算分区不再显示（镜像本就无 UI 字段）。

### 21.15 中立命名与入口

面向用户的入口不再以第三方引擎命名：导航“自动调查”、任务面板“任务中心”、
覆盖区“覆盖与完整性”、执行链“运行轨迹”、资产页“调查状态 / 发起自动调查”，
只有设置页与后端标签仍显示 Strix（它确实是兼容后端的名字）。
Tauri command 采取 ABI 兼容策略：旧名字（`test_strix_llm`、`list_strix_traces`、
`get_strix_trace`、`list_strix_skills`、`save_strix_skill`、
`start_strix_workbench_scan`、`rescan_strix_workbench_scan`）继续注册并原样转发到
`commands/agent_entries.rs` 里的中立入口（`test_model_profile`、`list_agent_traces`、
`get_agent_trace`、`list_agent_instructions`、`save_agent_instruction`、
`start_workbench_scan`、`rescan_workbench_scan`），前端只调用新名字。
后端选择与执行链相关的用户可见文案也已去引擎化：路由原因、暂停通知、
任务列表/日志/搜索线程错误、熔断区与 200 URL 上限提示、执行链阶段标签。
`native_run_is_never_described_as_a_strix_scan` 锁住这条约束：一次真实原生跑完后，
目标行的 `routing_reason`/`scan_mode` 与任务 `current_checkpoint` 里不得出现 Strix。

### 21.16 auto 的默认后端（Phase 2 §7 已生效）

`agent_select_backend` 现在只有一处判断：URL-only Web / 无源码灰盒的 `auto` 选择
native；源码、CI 或 native 表达不了的目标集合仍选兼容后端；显式 `strix` 永远 Strix。
任一后端开始执行后都不会因为失败悄悄换成另一个再扫一遍：选择先查本 attempt 的冻结
矩阵，再查继承链，最后才看当前配置。能力不足（模型名/端点未配置、任务类型不支持）
在发出目标请求之前就报出来：模型端不支持工具调用时，第一轮即以
`UNSUPPORTED_CAPABILITY` 结束，且发生在任何目标请求之前；设置页的连通性测试
（§21.14）会在启动前暴露同一问题。

（顺带修掉一个真实缺陷：LLM Hook 的 accept 没有清除 macOS 继承来的 `O_NONBLOCK`，
且 `read_request` 把一次 EAGAIN 当成错误，异步客户端分开写请求头与请求体时偶发 400。
现在 accept 后立即 `set_nonblocking(false)`，读取按 WouldBlock/TimedOut 重试并受
60 秒总期限约束。）

### 21.17 目录拆分（P2 §12 已执行）

巨型文件已按责任拆开，全部是**移动**而不是复制，没有任何第二套状态机、reducer 或工具实现：

- `commands/agent_tools.rs` 只留下工具运行期状态与常量；其余按职责拆成
  `agent_tools_specs|schema|scope|http|replay|identity_diff|discovery|browser|verdict|dispatch.rs`。
- `commands/agent_tests.rs` 只保留模块头与 include 清单，测试按契约区分文件：
  `agent_tests_fixtures|policy|harness|e2e|contracts|coverage|browser|validation|persistence|identity_e2e.rs`。
- `commands/tests.rs` 同上，拆成 `tests_runtime|code_rules|database_repair|investigation|frontend_recon|scan_lifecycle|results|model_config.rs`。
- `scan_execution.rs` 拆出 `scan_execution_routing|metrics|runtime|frontend.rs`；
  `result_ingestion.rs` 拆出 `result_ingestion_engines|runs|recon|artifacts|sync.rs`；
  `knowledge_learning.rs` 拆出 `knowledge_learning_quality|candidates|traces|skills.rs`。
- `src/sentinel.css` 按屏幕分区拆成 `src/styles/sentinel-base|workbench|trace|results|layout.css`，
  `sentinel.css` 变成保持层叠顺序的 import 索引。

`commands/*.rs` 本来就是 `include!` 进同一个 `mod commands`，因此这些分区文件之间仍然共用
模块内的私有项与 `use`；测试分区用 `mod` 内的 `include!` 达到同样效果，避免为了跨模块可见性
把夹具复制一份或全部改成 `pub`。仍然待拆：`SentinelBoard.vue`（需要按结果区拆组件与
composable，涉及 props/emits，必须配人工点验）、`db.rs` 与 `llm_hook.rs`（真实模块，拆分为
子模块需要逐项确定可见性）、`investigation.rs`。

### 21.18 验证入口

`cargo fmt --all -- --check`、`cargo test`、`cargo clippy --all-targets -- -D warnings`、
`npm run build`、`git diff --check`。§13 的失败测试与 §14 的 mock 端到端用例都在
`src-tauri/src/commands/agent_tests.rs` 与 `src-tauri/src/agent_runtime/tests.rs` 中，
共享断言函数 `assert_agent_surface` 负责每条用例的结果面检查。
