# Nest 自定义 Agent、协作与人工接管需求书

> 状态：未实现，但不是下一轮任务。角色配置目前只是不可执行草稿，没有协同台、HumanDirective 或自定义 Agent 运行实例。
> 这些要求已收进 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md` 的 Stage 11。在 Stage 0–10 通过人工复核之前不要实现本文。

> 编写日期：2026-09-20
> 适用范围：Oviraptor / Nest 原生多智能体运行时和桌面端
> 本文件补充 `NATIVE_MULTI_AGENT_REFACTOR_REQUIREMENTS.md`，两者共同构成实现与验收合同。

## 1. 架构决定

Nest 采用“固定内核 + 可配置专家模板 + 动态实例”的混合模式。

固定且不可删除的内核：

- Coordinator：唯一全局队列、预算、授权范围和目标终态控制者。
- Evidence Reviewer：唯一自动漏洞确认门禁。
- Tool Broker / Policy Engine：唯一工具、scope、身份、副作用和预算执行入口。
- Event / Evidence Store：唯一可恢复事实来源。

用户可以新增、复制、停用和调整专家模板，例如：

- 小王：信息搜集与接口整合。
- 小李：Web 安全验证。
- 小赵：深度调查和突破口分析。
- 小孙：组长/总检查；在 UI 中可以是 Coordinator 的显示身份。

显示名只用于 UI。权限必须绑定稳定的 `role_class + capability_bundle`，不能根据昵称授权。

Agent 模板数量可以增加，但同时活动实例仍遵守：

- 最多一个目标触碰 Agent。
- 最多一个只读分析 Agent。
- 最多一个 Evidence Reviewer。
- Coordinator 常驻但不直接触碰目标。

因此“角色数量”与“并发请求数”分离。可以定义十个专家，但只有证据满足触发条件且 lane 空闲时才
实例化其中一两个。

## 2. 一个模型 API 足够

首版只需要一个 `ModelGateway`、一个活动模型 Profile 和一组 API 凭证。所有角色复用该网关，但必须
拥有独立上下文、run、预算和工具视图：

```text
One API / One ModelGateway / One Profile
  ├── Coordinator context
  ├── 小王·侦察整合 context
  ├── 小李·Web 验证 context
  ├── 小赵·深度调查 context
  └── Evidence Reviewer context
```

禁止把“一个 API”实现成所有 Agent 共用一个聊天数组。每次模型请求必须在本地审计中关联：

```text
scan_id
attempt_number
run_id
agent_definition_id
agent_instance_id
role_class
assignment_id
coordination_thread_id
evidence_revision
```

云端模型可以在全局限流内并行不同 lane 的模型请求。本地模型默认并发 1；Agent 状态可以同时存在，
但模型回合通过公平队列串行执行。等待模型的 Agent 显示 `waiting_model_slot`，不会丢失任务。

后续可以为 Reviewer 或特殊角色配置第二 Profile，但它是可选增强，不是多 Agent 的必要条件。

## 3. 自定义 Agent 的安全模型

### 3.1 角色类别

用户新增 Agent 时必须先选择不可自造权限语义的 `role_class`：

```rust
enum AgentRoleClass {
    Coordinator,
    ReconAnalyst,
    TargetVerifier,
    DeepInvestigator,
    ImpactValidator,
    EvidenceReviewer,
}
```

- `ReconAnalyst`：只读分析和确定性侦察整合。
- `TargetVerifier`：可领取有界目标测试合同。
- `DeepInvestigator`：只读组合攻击链、找矛盾和提出补证需求。
- `ImpactValidator`：只验证已确认入口的有限影响，不等于无限制“后渗透”。
- `EvidenceReviewer`：只读证据复核，不能触碰目标。

Coordinator 和 EvidenceReviewer 默认内置。允许复制提示和显示配置，但不能降低其安全不变量。

### 3.2 AgentDefinition

每个自定义模板使用结构化 manifest，不允许只填一段无限权限 prompt：

```rust
struct AgentDefinition {
    id: String,
    schema_version: i64,
    display_name: String,
    description: String,
    role_class: AgentRoleClass,
    objective: String,
    instructions: String,
    activation_policy: ActivationPolicy,
    input_contract: InputContract,
    output_contract: OutputContract,
    capability_bundles: Vec<String>,
    sandbox_policy: SandboxPolicy,
    budget_policy: AgentBudgetPolicy,
    collaboration_policy: CollaborationPolicy,
    stop_policy: AgentStopPolicy,
    approval_policy: ApprovalPolicy,
    enabled: bool,
    built_in: bool,
}
```

### 3.3 激活条件

配置界面提供条件构建器，不要求用户写代码：

```text
ANY / ALL
  evidence kind exists
  coverage family is open
  identity count >= N
  endpoint trait matches
  hypothesis severity ceiling >= level
  another role requested prerequisite
  Reviewer returned insufficient_evidence
  no progress after N determined contracts
  capability is available
```

模型可以建议启动哪个 Agent，但 Coordinator 必须用 Rust 规则重新校验条件。

### 3.4 输入与输出合同

用户从预置字段选择 Agent 能读取的证据类型，例如 Endpoint、RequestRecord、ResponseShape、
BusinessObject、Hypothesis 和 CandidateFinding。不能勾选“读取整个数据库”。

所有自定义 Agent 返回统一信封：新事实、新假设、candidate、coverage、证据缺口、建议下一步、
capability gap、artifact refs 和剩余预算。它不能直接写全局队列或确认漏洞。

## 4. 能力包与工具

角色多不意味着实现多套工具。所有工具只在中央 Tool Registry 注册一次；Agent 领取经过审计的
能力包。建议首批能力包：

| 能力包 | 典型工具 | 默认沙箱 |
| --- | --- | --- |
| `evidence-read` | graph query、artifact summary、coverage read | 进程内只读 |
| `frontend-map` | JS/source-map local analysis、CDP evidence read | 进程内/浏览器子进程 |
| `anonymous-http` | bounded replay、compare、targeted discovery | NativeRestricted |
| `identity-observe` | session state、refresh observation、identity compare | NativeRestricted/浏览器 |
| `authorization-verify` | paired replay、object variant、response compare | NativeRestricted |
| `input-variants` | Broker 生成的受控参数变体 | NativeRestricted |
| `upload-safe` | harmless fixture、access check、cleanup | Container 优先 |
| `business-state` | bounded transition、state compare、compensation | NativeRestricted + approval |
| `concurrency-bounded` | Rust race scheduler，并发 2–3 | NativeRestricted |
| `impact-validation` | 已证实入口的最小影响验证 | Container + 人工批准 |
| `review-only` | evidence/candidate/review read | 进程内只读 |

自定义 Agent 只能选择与 `role_class` 兼容的能力包。不能输入任意工具名，不能注册 shell、Python、
终端或自定义二进制。Tool Broker 执行时仍二次校验角色、租约、scope、身份和副作用。

### 4.1 ImpactValidator 的边界

“后渗透”不应实现成获得入口后自由操作系统。产品中应命名为“影响验证”，默认禁止：

- 持久化、提权、横向移动。
- 凭证导出、浏览真实用户数据、批量下载。
- 关闭防护、删除日志、植入长期文件。
- 任意 shell 和任意网络扫描。

它只能验证被明确批准的最小影响假设，使用无害标记和隔离容器，并有清理合同、时间上限和人工批准。
无法安全证明的影响进入人工深入建议。

## 5. 轻量级沙箱是否够用

够用，但沙箱必须按“工具调用风险”选择，不按“Agent 数量”一一创建。Agent 是逻辑实体，沙箱是
可复用执行资源：

```text
Agent assignment
  -> Tool Broker policy
  -> risk classification
  -> sandbox router
     ├── InProcessReadOnly
     ├── NativeRestricted
     ├── BrowserWorker
     └── ContainerPool
```

- Coordinator、ReconAnalyst、DeepInvestigator、Reviewer 大部分工作进程内只读即可。
- 普通 HTTP、身份对照、参数变体和有限并发使用 NativeRestricted。
- CDP 使用独立 BrowserWorker，每个身份独立 session/profile。
- 文件上传、复杂解析、第三方工具、源码构建和影响验证使用按需容器池。
- Container 按 assignment/tool invocation 租用并回收，不为每个 Agent 常驻一个。
- Docker 不可用时只标记相关能力不可用；其他轻量 Agent 继续运行。

沙箱路由由工具描述符和副作用等级决定。用户可以选择更严格策略，但不能把要求 Container 的工具
降级到宿主机或 NativeRestricted。

## 6. Agent 之间的往返协作

协作围绕具体 EvidenceGap、Hypothesis 或 CandidateFinding 建立 thread，不使用一个无限增长的全局
群聊。消息包含用户可读摘要和机器字段：

```rust
struct CoordinationMessage {
    id: String,
    thread_id: String,
    evidence_revision: i64,
    from_agent_instance_id: String,
    to_role_class: Option<AgentRoleClass>,
    kind: Progress | EvidenceRequest | EvidenceResponse | Proposal |
          Challenge | Blocker | ReviewFeedback | HumanDirective | Decision,
    summary: String,
    reason_codes: Vec<String>,
    fact_refs: Vec<String>,
    contract_refs: Vec<String>,
    artifact_refs: Vec<String>,
    requires_coordinator_action: bool,
}
```

Agent 可以请求其他角色补齐 prerequisite、质疑提案、指出证据过期或合同重复，但不能直接命令另一个
Agent 发请求。Coordinator 将请求合并、去重和评分后决定是否生成新 Assignment。

### 6.1 示例

```text
小王·信息搜集：发现 /api/orders/{id}，但来源是 source map，尚未观察到真实请求。

小李·Web 验证：提出缺口 G-17。需要 observed request、账号 B 和对象归属基线，当前不能测试越权。

小赵·深度调查：补充 G-17。账号 B 的 session revision 已过期，而且 orderId 是否全局唯一尚未确认。

小孙·总控：合并两个提案。先派小王触发“订单详情”并采集请求，再派身份 Agent 刷新账号 B；
暂缓小李的越权合同。

新 revision：小王和身份 Agent 完成补证。

小孙·总控：前置条件满足，授权小李执行一次 A/B 对照。

小李·Web 验证：取得响应差异，但发现缓存标记，提交 candidate 而非 confirmed。

小赵·深度调查：建议补一条 cache-bypass 控制请求；总控接受后再交 Reviewer。
```

每个 evidence revision 最多一轮提案和一轮补充/质疑。没有新 fact、artifact、ReviewDecision 或
capability change 时，不允许继续互相请求。

## 7. 用户可见协作与思维边界

用户可以看到真实协作进程：

- Agent 状态、当前 Assignment、合同进度、Token、请求数和最近心跳。
- EvidenceRequest、EvidenceResponse、Proposal、Challenge、Blocker 和 Coordinator Decision。
- 工具开始/结束、脱敏输入摘要、状态和 artifact 引用。
- Reviewer 的 verdict、反证、缺失证据、置信度和等级变化条件。

页面不展示或持久化：

- 私有 chain-of-thought、隐藏推理草稿和逐 token 内容。
- 原始 Cookie、JWT、API Key、密码或未脱敏正文。
- Agent 自报但无法由合同/覆盖账本证明的百分比。
- 未经过 Tool Broker 的命令或 payload。

每次模型回合如需向用户解释，必须另行输出短 `decision_summary`：观察到什么、缺什么、建议什么、
为什么现在值得做。它不是私有思维链。

## 8. 人工在环

人工可以：

- 补充 URL、参数、对象 ID、页面路径、业务规则和可能入口。
- 上传或引用脱敏请求、截图、API 文档和账号说明。
- 请求某个自定义 Agent 进行“评估”；这只创建提案请求，不直接执行工具。
- 调整 EvidenceGap 优先级。
- 提供或刷新身份；凭证只进入专用安全捕获流程。
- 暂停、恢复、取消单个 Assignment 或整个 target。
- 批准/拒绝 controlled write、上传、业务状态变化、并发或影响验证。
- 将未完成 gap 转为人工接管。

人工不能：

- 在普通对话框扩大 scope。范围变更必须创建新的 plan revision 并明确确认。
- 用“直接测”绕过 Tool Broker、预算、清理合同和并发上限。
- 原地修改 immutable fact；修正必须写 `HumanCorrection` 和 supersedes 边。
- 把 candidate 直接改成自动 confirmed。可记录独立的 `human_confirmed` 来源。
- 把 Cookie/Token 粘贴进普通协作线程。

自然语言“给总控留言”必须先解析为 `HumanDirective` 草案，向用户显示目标、意图、关联证据、
是否影响 scope、预计成本和需要的批准，确认后才能进入 Coordinator 队列。

## 9. Role Studio：自定义 Agent 页面

Nest 内新增“Agent 工作室”，不增加新的顶层产品模块。推荐向导：

1. 名称与职责：显示名、描述、role class。
2. 何时启动：条件构建器和 prerequisite。
3. 可读信息：输入证据类型与最大上下文。
4. 能做什么：选择能力包；不直接勾选底层危险工具。
5. 运行边界：预算、超时、最大合同、停止条件。
6. 协作方式：可向哪些 role class 请求什么 prerequisite。
7. 审批和沙箱：由风险自动生成，用户只能加强。
8. Mock 演练：使用本地夹具验证触发、输出 schema、越权工具拒绝和预算停止。

示例配置：

```json
{
  "displayName": "小李",
  "roleClass": "target_verifier",
  "objective": "验证公开 Web 与 API 候选，输出可复核 candidate",
  "activationPolicy": {
    "all": ["observed_endpoint_exists", "open_verification_contract_exists"]
  },
  "inputContract": ["Endpoint", "RequestRecord", "ResponseShape", "Hypothesis"],
  "capabilityBundles": ["anonymous-http", "authorization-verify", "input-variants"],
  "budgetPolicy": {
    "maxModelRequests": 6,
    "maxTargetRequests": 12,
    "maxWallSeconds": 600
  },
  "sandboxPolicy": "auto_strict",
  "collaborationPolicy": {
    "mayRequest": ["recon_analyst", "deep_investigator"],
    "maxRequestsPerRevision": 1
  },
  "stopPolicy": ["waf", "persistent_429", "no_progress", "budget_exhausted"]
}
```

保存前必须静态校验 role/capability/sandbox 兼容性。自定义 instructions 中要求忽略 scope、读取 Secret、
使用 shell 或绕过 Reviewer 的内容必须拒绝。

## 10. 协同台页面

Nest 子导航建议：

```text
总览 · 新建扫描 · 任务 · 协同台 · 证据图谱 · 发现 · Agent 工作室 · 设置
```

协同台由四区组成：

1. 顶部任务条：目标、attempt、运行状态、保护状态、总预算、暂停/人工接管。
2. 左侧团队栏：Agent 卡片、当前任务、lane、进度、usage、等待原因。
3. 中间协作线程：按 gap/hypothesis/candidate 显示往返、补证与 Coordinator 决定。
4. 右侧人工干预：补充线索、请求评估、提供身份、批准动作、调整优先级。

默认只展示当前 thread、需要人工动作和最近 Coordinator 决定。原始 event JSON、长响应和底层日志进入
详情抽屉，不淹没协作视图。

每个 Agent 卡片至少显示：role、显示名、状态、Assignment 摘要、lane、已完成/总合同、Token、目标
请求数、等待原因和心跳。进度由合同决定结果计算，不接受 Agent 自报百分比。

## 11. 数据库和桌面 API

建议增加：

```text
agent_definitions
agent_definition_versions
agent_instances
agent_coordination_threads
agent_coordination_messages
agent_human_directives
agent_capability_bundles
```

AgentDefinition 修改后创建版本；已经运行的 instance 始终引用冻结版本，不能被设置页热修改。
CoordinationMessage append-only，修正通过 supersedes id。Secret 只存安全句柄。

建议的后端命令：

```text
list_agent_definitions
save_agent_definition
validate_agent_definition
simulate_agent_definition
set_agent_definition_enabled
get_agent_team_state
list_coordination_threads
get_coordination_thread
submit_human_directive
request_agent_assessment
pause_agent_assignment
resume_agent_assignment
cancel_agent_assignment
approve_agent_contract
```

Tauri 实时事件只发送 ID、状态和短摘要。前端刷新或应用重启后必须能从数据库完全恢复，不能把 UI
内存当事实来源。

## 12. 必须新增的测试

- 多角色共享一个 mock API endpoint，但上下文、工具 schema、usage 和错误互不污染。
- 本地模型并发 1 时，多个 Agent 公平等待并恢复。
- 自定义 ReconAnalyst 选择目标触碰能力包时保存失败。
- 要求 Container 的能力不能降级到 NativeRestricted。
- Agent 数量增加不增加目标触碰并发上限。
- 验证 Agent 的 prerequisite 能转给信息搜集 Agent，补证后继续原合同。
- DeepInvestigator 能提出补证链，但无新证据时不会循环重扫。
- 页面刷新后恢复 Agent 卡片、线程、未读人工动作和预算。
- UI 消息无 raw model output、Secret 和虚构进度。
- HumanDirective 不能绕过 scope、approval、cleanup、Reviewer 或预算。
- AgentDefinition 版本冻结；修改模板不改变运行中实例。
- 自定义 Agent 的恶意 instructions 无法扩权或获得隐藏工具。

## 13. 实施顺序

1. 先实现 AgentDefinition、版本冻结、role class、能力包和静态校验。
2. 把内置专家迁移为同一 manifest 模型，但保留内核角色不可删除属性。
3. 实现统一单 API 的独立 context 和公平模型队列。
4. 实现 coordination thread/message repository，先只读展示现有事件。
5. 接入 EvidenceRequest -> Coordinator -> 新 Assignment 的往返补证。
6. 增加 DeepInvestigator、Reviewer insufficient 回补和防空转门禁。
7. 实现 Agent 工作室与 mock 演练。
8. 实现协同台和 HumanDirective；最后开放高影响动作审批。

## 14. 完成定义

1. 固定内核无法被自定义 Agent 替换或绕过。
2. 用户可以通过 Role Studio 安全创建、验证、版本化和停用专家模板。
3. 一个 API/Profile 能承载全部角色，且上下文、工具、预算和 usage 隔离。
4. 工具集中注册，Agent 只领取兼容能力包；角色增加不复制工具实现。
5. 沙箱按工具风险路由和复用；轻量任务不启容器，高风险任务不能降级。
6. Agent 能围绕具体 gap 往返补证，不是固定顺序也不是无限自由聊天。
7. 用户能看到真实状态、协作摘要、补证请求、阻塞点和 Coordinator 决定。
8. 页面不展示私有思维链、Secret、未脱敏正文或虚构进度。
9. 人工能补线索、请求评估、提供身份、批准动作、暂停和接管，但不能绕过安全门禁。
10. UI 刷新、应用重启和 attempt 续跑后，定义版本、实例、线程与人工动作完整恢复且不重复执行。
