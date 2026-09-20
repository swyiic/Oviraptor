# Oviraptor 原生 Agent Runtime 可靠性整改任务书

> 编写日期：2026-09-19  
> 适用版本：Oviraptor 1.1.59 当前未提交工作区  
> 使用方式：把本文件完整交给实现者。实现者必须先逐行阅读本文件以及本文指定的前置文档，再修改代码。不得只根据标题、摘要或聊天上下文自行推断需求。

## 0. 实现者开始前必须执行的动作

开始修改前，必须先阅读并理解：

1. `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`
2. `docs/NATIVE_AGENT_EXECUTOR_TASK.md`
3. 本文件 `docs/NATIVE_AGENT_RUNTIME_REMEDIATION_REQUIREMENTS.md`

然后检查这些实现文件：

- `src-tauri/src/commands/agent_backend.rs`
- `src-tauri/src/commands/agent_contract.rs`
- `src-tauri/src/commands/agent_native.rs`
- `src-tauri/src/commands/agent_runtime.rs`
- `src-tauri/src/commands/agent_tools.rs`
- `src-tauri/src/commands/agent_tests.rs`
- `src-tauri/src/commands/scan_lifecycle.rs`
- `src-tauri/src/commands/result_ingestion.rs`
- `src-tauri/src/agent_runtime/`
- `src-tauri/src/llm_hook.rs`

修改前必须运行并记录基线：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
git status --short
```

当前工作区包含大量用户修改、已删除 Python worker 和尚未跟踪的 Rust 文件。禁止执行：

```text
git reset --hard
git checkout -- .
git clean -fd
git restore .
```

不得覆盖、撤销、移动与本任务无关的修改。不得因为文件较大而重新复制一份同功能模块。

## 1. 本轮唯一目标

本轮不是继续增加扫描能力，也不是美化 UI。唯一目标是使现有原生 Agent 满足以下可靠性契约：

1. “继续未完成阶段”确实继承上一执行 attempt 的可复用事实，不重复请求、不重复契约、不重复消耗。
2. “重新执行”创建完全独立的新 attempt，只清理当前结果面，不污染历史审计。
3. frozen execution plan 在一个逻辑执行链内不可被设置变化覆盖。
4. 所有 HTTP、浏览器导航和 CDP 网络增量都受同一套 allowed origins 约束。
5. 原始响应与模型输入严格分离，云端模型永远不能收到未脱敏 Cookie、Token、密码和业务敏感值。
6. A/B 差异不会把角色、租户、组织、权限字段错误降级为普通身份字段。
7. 覆盖率只能由可定位的真实证据证明，模型不能通过填写字符串伪造覆盖。
8. confirmed finding 必须绑定两条明确、不同且真实执行过的请求记录。
9. 升级前的 native checkpoint 可以迁移或明确归档，不能静默丢弃并重新烧 Token。
10. 所有工具参数执行前经过完整 JSON Schema 校验。

只有上述十项全部完成并通过验收，才能报告本任务完成。

## 2. 明确禁止的实现方式

以下方案即使能通过现有测试，也视为未完成：

- 不得通过放宽 `resumable()` 条件，让任意旧状态直接恢复。
- 不得删除 `attempt_number`、`evidence_hash` 或 `execution_plan_hash` 校验。
- 不得把新 attempt 伪装成旧 attempt 来绕过状态迁移。
- 不得在 resume 时重新读取当前全局配置并覆盖冻结计划。
- 不得只保存旧 backend 名称，却重新计算预算、范围、覆盖族或身份计划。
- 不得通过扩大 allowed origins 解决 CDP 第三方请求问题。
- 不得把浏览器捕获的所有域名自动加入授权范围。
- 不得仅在日志层脱敏；进入模型的 tool result 必须在生成时脱敏。
- 不得为了脱敏而删除全部响应信息；模型仍要看到结构、状态码、字段类型和有用差异。
- 不得继续使用 `ends_with("id")` 判断普通身份字段。
- 不得相信模型提供的 `family`、`controlRequest`、`testRequest` 文本就是证据。
- 不得用字符串包含关系替代 evidence id、request id 或 invocation id 关联。
- 不得新增第二套扫描状态机、第二套终态聚合器或第二套重试入口。
- 不得让 Native 已经消费模型 Token 或目标请求后自动回退 Strix。
- 不得修改任务级 session 隔离规则；session 不能跨扫描任务复用。
- 不得把缺证据、未覆盖、旧状态不兼容显示为“已完成”或“确认安全”。

## 3. 整改一：真正实现 attempt 继承和续跑

### 3.1 当前缺陷

当前“继续未完成阶段”会递增 `attempt_count`，生成新的 attempt；但 `NativeAgentState::resumable()` 要求旧 checkpoint 的 `attempt_number` 与当前 attempt 完全相等。结果是新 attempt 无法恢复旧状态，随后清除 checkpoint 并从头开始。

同时 `agent_build_run_context()` 每次都依据当前设置重新计算并覆盖 `agent_execution_plan`。只要设置、预算、范围或身份计划变化，plan hash 就会变化，旧状态再次无法恢复。

### 3.2 必须采用的模型

区分三个概念：

```text
scan                用户看到的长期任务
attempt             一次可审计的执行记录
continuation source 新 attempt 可以继承的上一个未完成 attempt
```

新 attempt 必须保留新的 `attempt_number`，同时显式记录：

```rust
parent_attempt_number: Option<i64>
resume_kind: Fresh | ContinueIncomplete
inherited_checkpoint_schema: Option<i64>
```

如果不希望修改数据库表，可以先将这三个字段写入 attempt metadata JSON；但最终读取必须是类型化代码，不能依赖 UI 文案判断。

### 3.3 Fresh 语义

重新执行时：

- 创建新 attempt。
- `parent_attempt_number = None`。
- 生成新的 execution plan。
- 清理当前结果面的 machine-generated 状态。
- 不继承旧请求预算、pending queue、contract attempts、contract outcomes、covered families 或 terminal reason。
- 保留不可变历史目录和历史 attempt 记录。
- 是否保留人工确认结论必须沿用现有产品约定，不在本轮擅自修改。

### 3.4 ContinueIncomplete 语义

继续未完成阶段时：

- 创建新 attempt，用于新的审计时间段。
- `parent_attempt_number` 指向上一未完成 attempt。
- 从父 attempt 读取完整冻结 execution plan。
- 创建子 attempt 的 plan 快照；内容与父 plan 完全相同，只允许更新当前 attempt 标识和运行时间字段。
- 继承已执行请求、已完成/耗尽契约、覆盖事实、预算使用、待处理队列、发现轮次和已经确认的结论引用。
- 清除父 attempt 的瞬态字段：`running`、旧错误横幅、旧进程 id、旧心跳时间、旧 terminal UI marker。
- 不继承会导致立即结束的旧 terminal reason；原始 terminal reason 仅作为历史事件保留。
- 已完成契约不能再次调用。
- 已花费预算必须计入逻辑扫描链总预算，不能通过点击继续重置硬预算。
- 如果父 checkpoint 不兼容，必须返回明确的 `resume_checkpoint_incompatible`，而不是静默 fresh start。

### 3.5 execution plan 冻结规则

执行计划至少必须冻结：

- backend
- scan mode
- target URL
- allowed origins
- identities 的任务级句柄
- coverage families
- per-contract attempt limits
- soft/hard token budget
- target request budget
- discovery round budget
- no-progress window
- stop policy
- local/cloud model provider class

继续任务不能从当前设置重新生成上述字段。如果用户希望应用新配置，必须使用“重新执行”。

### 3.6 失败行为

遇到以下情况不得自动 fresh：

- 找到父 checkpoint，但 schema 不兼容。
- evidence hash 不同。
- plan hash 不同。
- 父 attempt 没有可恢复状态。

必须终止该 URL 的自动续跑，并记录具体原因、父 attempt、当前 attempt 和建议操作。UI 展示“续跑状态不兼容，需要重新执行”，不能显示普通模型失败。

## 4. 整改二：checkpoint v1 到 v2 的迁移

### 4.1 当前缺陷

当前只接受 `schemaVersion == 2`，所有 v1 checkpoint 会被当成不存在。

### 4.2 必须实现的迁移器

新增纯函数，例如：

```rust
fn migrate_native_agent_state(value: &JsonValue) -> Result<NativeAgentState, StateMigrationError>
```

规则：

- v2：正常解析。
- v1：读取旧字段，缺失的新字段使用保守默认值。
- 未知高版本：拒绝降级读取，返回 `checkpoint_version_too_new`。
- 格式损坏：返回 `checkpoint_corrupted`。
- 迁移成功后，以 v2 写回当前 checkpoint，并追加一条 migration event。
- 原始 v1 JSON 必须保留在 attempt 历史 artifact 或事件中，便于审计。

v1 缺少字段时的默认值：

```text
observed_requests     = []
discovery_rounds      = 0
target_requests       = 从 budgetUsage 可推导则推导，否则 0
contract_attempts     = 已完成契约至少记为 1，否则 0
contract_outcomes     = 根据旧 completed/exhausted 集合映射
verdict_keys          = []
confirmed_findings    = 从旧持久化 finding 引用推导，否则 0
```

无法安全推导的字段必须保守处理，不得伪造已执行证据。迁移后如果缺少继续所必需的请求账本，应进入明确的 `resume_evidence_incomplete`，而不是假装已覆盖。

## 5. 整改三：统一范围约束

### 5.1 唯一范围判定器

所有出站目标必须调用同一个范围判定器。不得在 HTTP、浏览器、发现和身份比较工具中各自实现相似判断。

范围判定输入：

```rust
ScopeDecisionInput {
    requested_url,
    method,
    source,          // HTTPReplay, BrowserEntry, BrowserObserved, Discovery
    action_id,
    identity_key,
    execution_plan_hash,
}
```

返回：

```rust
ScopeDecision::Allow { normalized_origin, reason }
ScopeDecision::Ignore { reason }
ScopeDecision::Reject { code, reason }
```

### 5.2 必须校验的位置

- `replay_http` 发送请求前。
- `compare_identities` 每一侧请求前。
- `targeted_discovery` 生成候选和实际验证前。
- `browser_action` 把 `entry` 交给 CDP 前。
- CDP 返回每条 observed request 后、写入 runtime ledger 前。
- 任何由重定向产生的新 origin。

### 5.3 CDP 请求分类

对浏览器观察请求进行以下分类：

```text
authorized_business_api   进入正式 API、覆盖率和模型摘要
authorized_document       作为页面导航证据，不自动当 API
authorized_static         仅作资源引用，不进入正式 API
third_party_required      保存为第三方依赖摘要，不进入正式 API/覆盖率
telemetry_or_noise        丢弃或只计数
out_of_scope              审计记录后拒绝，不进入运行账本
```

CDN 上的 JS 可以作为证据来源，但不能因为 JS 来自 CDN 就把 CDN 的所有网络请求加入授权范围。

### 5.4 重定向规则

- 同 origin 重定向：允许。
- 已冻结 allowed origins 内的重定向：允许并记录。
- 身份提供商登录跳转：只允许浏览器显示和会话采集，不自动进行安全测试。
- 其他跨域重定向：停止该动作并返回 `redirect_out_of_scope`。

## 6. 整改四：模型输入脱敏与原始证据隔离

### 6.1 双视图数据模型

每次工具执行必须产生两个结果：

```rust
ToolExecutionResult {
    raw_artifact: RawArtifactRef,
    model_view: RedactedToolResult,
    progress_facts: ProgressFacts,
    audit: ToolAuditFacts,
}
```

原始数据只能写入 artifact store；`history.push(role=tool)` 只能使用 `model_view`。

### 6.2 必须脱敏的内容

至少覆盖：

- `Authorization`
- `Proxy-Authorization`
- `Cookie`、`Set-Cookie`
- access/refresh/id token
- JWT
- API Key、secret、password、passwd、pwd
- session、sid、csrf/xsrf token
- 手机号、邮箱、身份证号等已存在规则中的敏感值
- URL query 和表单/JSON body 中的同类字段
- 响应正文中被规则识别的敏感值

脱敏必须保留比较能力，例如：

```text
原值 -> <redacted:type:stable_hash_prefix>
```

同一运行中的相同秘密应有相同不可逆指纹，以便 A/B 或重放判断是否变化；不同运行不要求指纹相同。

### 6.3 模型可见信息

模型仍可看到：

- method、规范化 URL/path
- 状态码、内容类型、耗时、大小、是否截断
- 响应 header 名称，但不默认看到敏感 header 值
- JSON 字段路径和类型
- 经脱敏的短枚举、错误码、角色、租户/组织/权限字段
- body hash、结构 hash
- A/B 差异摘要
- artifact id

### 6.4 本地保存

原始请求/响应不能继续作为普通明文 JSON 随意落盘。最低要求：

- 文件权限仅当前用户可读写。
- 默认 UI 不直接展示 secret。
- artifact metadata 与 raw payload 分离。
- raw payload 使用任务级加密或系统钥匙串派生密钥；如果本轮无法完成加密，必须明确标记技术债并至少停止把 raw payload 写进普通 `rawBody` JSON。

不得通过“工具运行在内网”跳过模型输入脱敏，因为产品同时支持云端模型。

## 7. 整改五：严格 JSON Schema 校验

### 7.1 当前缺陷

现有参数校验只验证对象、未知字段和 required，不验证类型、enum、长度、数组大小、嵌套字段或数值范围。

### 7.2 必须实现

使用成熟 JSON Schema 校验器，或实现覆盖当前 schema 全部关键字的确定性校验器。至少支持：

- `type`
- `required`
- `additionalProperties: false`
- `properties`
- `items`
- `enum`
- `const`
- `minimum` / `maximum`
- `minLength` / `maxLength`
- `minItems` / `maxItems`
- 嵌套 object/array
- URI 或项目自定义 URL 格式检查

启动时预编译每个工具 schema；不得每次工具调用重新构造全部 schema。

校验失败时：

- 不执行工具。
- 不消耗目标请求预算。
- 记录稳定错误码和具体 JSON path。
- 将简洁修正提示返回模型。
- 连续相同参数错误计入无进展，防止无限重试。

## 8. 整改六：修复 A/B 身份与权限差异

### 8.1 字段分类不得使用模糊后缀

删除 `normalized.ends_with("id")` 作为 ownership 判断的逻辑。

字段至少划分为：

```text
volatile      时间戳、nonce、trace、随机数
secret        token、cookie、密钥
subject       当前账号自身标识，如 userId、username、avatar
authorization role、permission、tenant、organization、department、policy、scope
business      订单、项目、记录、对象归属等业务字段
unknown       未知但发生变化的字段
```

其中 authorization 字段变化必须进入 `materialAuthorizationDiffs`，不得进入 ignored 或 ownershipOnly。

建议使用精确词典和 token 化匹配：

```text
role, roleId, roles
permission, permissionId, permissions, perms
tenant, tenantId
organization, organizationId, org, orgId
department, departmentId, dept, deptId
policy, policyId
scope, scopes
group, groupId
privilege, authority, acl
```

`userId`、`accountId` 等 subject 字段也不能直接忽略；它们应被单独展示，用于确认响应是否确实属于对应账号。

### 8.2 A/B 比较条件

只有以下条件全部满足才允许输出权限差异结论：

- 两侧都有完整请求记录。
- method、规范化 host、path 和业务参数语义一致。
- A 与 B 使用不同任务级 identity handle。
- 两侧响应均记录 status、structure hash 和 artifact id。
- 差异不是仅由 volatile/secret 字段造成。

一侧缺失必须输出 `insufficient_evidence`，并给出缺失侧、缺失原因和建议动作。匿名任务不得展示账号 A/B。

## 9. 整改七：覆盖率必须绑定真实证据

### 9.1 当前缺陷

当前请求 trace 的 `family` 主要来自模型参数。模型可以对同一个无关接口填写不同 family，使多个覆盖族看起来被执行。

### 9.2 新覆盖证据结构

新增类型化记录：

```rust
CoverageEvidence {
    family: CoverageFamily,
    evidence_kind: ContractExecution | IdentityComparison | BrowserAction | DiscoveryProbe,
    contract_key: Option<String>,
    tool_invocation_ids: Vec<String>,
    request_record_ids: Vec<String>,
    verdict_id: Option<String>,
    result: Covered | Rejected | Exhausted | NotApplicable | InsufficientEvidence,
    reason_code: String,
}
```

### 9.3 family 的来源

family 必须由 Rust 根据以下事实推导：

- 当前执行的 evidence contract 类型。
- 工具类型和身份组合。
- 请求/响应事实。
- 本地覆盖规则。

模型可以建议 family，但不能直接写入最终 ledger。

例如：

- 单次匿名 GET 不能证明 authorization 已覆盖。
- 没有 A/B 或 anonymous/authenticated 对照，不能证明 identity authorization 已覆盖。
- 没有反射点或输入响应链，不能证明 XSS family 已覆盖。
- 仅访问登录页不能证明 session family 已覆盖。
- 目录发现无新增可以关闭当前 discovery round，但不能自动覆盖所有隐藏接口风险。

### 9.4 finish_target

`finish_target` 只能引用 runtime 已经生成的 coverage evidence id。模型提交不存在的 family/evidence id 时必须降级为 unsupported claim。

未覆盖、证据不足和不适用必须分别展示；任何一种都不能写成“未发现风险”。

## 10. 整改八：confirmed finding 绑定请求记录

### 10.1 禁止自由文本匹配

删除依靠 `controlRequest`/`testRequest` 文本中包含 path 的确认方式。

工具参数改为：

```json
{
  "controlRequestId": "request-record-id-1",
  "testRequestId": "request-record-id-2",
  "responseDifferenceArtifactId": "artifact-id",
  "impact": "...",
  "reproductionSteps": ["..."],
  "counterEvidence": "..."
}
```

### 10.2 confirmed 的硬条件

- 两个 request id 都存在于当前逻辑执行链的账本。
- 两个 id 不同。
- 两条请求与当前 contract/hypothesis 关联。
- 身份、参数或必要变量存在预期控制差异。
- 响应差异 artifact 存在并能关联两条响应。
- contract 至少执行过一次。
- 没有被 WAF、登录重定向、统一错误页或缓存响应误导。
- impact 和 reproduction steps 非空。

不满足时只能记录 `insufficient_evidence`，不能写入漏洞结论。

## 11. 整改九：终态和 UI 数据语义

本轮不要求重新设计 UI，但后端输出必须支持 UI 正确展示。

每个 target 当前只允许一个终态：

```text
completed
completed_with_gaps
paused
cancelled
protected_stop
resume_incompatible
failed
```

定义：

- `completed`：执行计划要求的覆盖全部具备真实证据，或有明确合法 not applicable。
- `completed_with_gaps`：流程正常收口，但仍有 uncovered/insufficient evidence。
- `paused`：用户主动暂停，可继续。
- `cancelled`：用户取消，不自动继续。
- `protected_stop`：WAF/CAPTCHA/持续 429 等明确保护信号。
- `resume_incompatible`：续跑状态无法安全迁移，必须重新执行。
- `failed`：程序、模型协议、数据库或工具执行异常。

普通 401/403、没有登录、没有发现漏洞、没有身份 B 都不属于 failed。

重试或继续时，当前 UI 状态面只展示新 attempt 的状态；旧错误、旧“前端完成”、旧“部分完成”等只能出现在历史记录中。

## 12. 整改十：单一事实来源和代码整理

当前同时存在：

- `commands/agent_runtime.rs` 中的执行计划结构。
- `agent_runtime/contract.rs` 中的公共执行计划结构。
- `NativeAgentState` legacy checkpoint。
- `agent_runs`、`agent_events`、`agent_snapshots` canonical runtime 表。

本轮必须明确唯一权威来源：

1. execution plan 的公共类型只能有一个。
2. 运行状态以 `agent_runs + agent_events + agent_snapshots` 为权威。
3. 如果 `NativeAgentState` 暂时保留，只能是 canonical snapshot 的序列化兼容层，不得自行维护另一套状态语义。
4. attempt 终态必须由一个 reducer 生成。
5. Native、Strix、resume、fresh 不得分别拼装不同终态文案。

不要求一次完成所有目录迁移，但不允许继续扩大重复类型和重复状态机。

## 13. 必须先写的失败测试

实现代码前先新增以下测试，并确认在旧实现上失败。禁止先改代码再补只会成功的测试。

### 13.1 续跑

1. attempt 1 执行两个请求、完成一个契约后中断。
2. 创建 attempt 2，模式为 ContinueIncomplete，父 attempt 为 1。
3. attempt 2 恢复两次已用请求、已完成契约和剩余队列。
4. 已完成契约不会再次执行。
5. attempt 2 当前状态不显示 attempt 1 的错误终态。

### 13.2 设置变化

1. attempt 1 开始后修改全局预算、backend 和 allowed origins 设置。
2. ContinueIncomplete 仍使用 attempt 1 的冻结计划。
3. Fresh 使用新设置生成新计划。

### 13.3 checkpoint 迁移

- v1 能迁移为 v2。
- v1 缺少请求账本时不会伪造覆盖。
- v3/未知高版本返回明确错误。
- 损坏 JSON 不会触发 fresh 自动重跑。

### 13.4 范围

- browser action 的外域 entry 被拒绝。
- CDP 捕获第三方 telemetry 不进入正式 API。
- authorized CDN JS 不导致 CDN API 自动授权。
- 跨域重定向被阻止。
- allowed origin 的业务 API 正常进入账本。

### 13.5 脱敏

- 请求 header、query、JSON body、form body 和响应正文中的秘密不会出现在 model history。
- 相同秘密的稳定指纹允许比较。
- raw artifact 与 model view 内容不同。
- 云端和本地模型均只接收 model view。

### 13.6 参数 schema

- 错误类型、非法 enum、超长字符串、数组超限、嵌套未知字段全部拒绝。
- 校验失败不增加 target request counter。
- 合法参数正常执行。

### 13.7 身份差异

- `roleId`、`tenantId`、`departmentId`、`permissionId` 变化是 material authorization diff。
- timestamp、nonce 不产生安全差异。
- token 变化不泄漏原值。
- userId 差异作为 subject alignment 展示，不被静默忽略。
- 一侧响应缺失返回 insufficient evidence。

### 13.8 覆盖率

- 同一个 GET 请求即使模型填写七个 family，也只能获得 Rust 可推导的覆盖。
- 没有 A/B 对照不能覆盖 authorization family。
- finish_target 引用不存在 evidence id 时拒绝该覆盖声明。
- uncovered 不能被终态聚合为 completed。

### 13.9 漏洞确认

- control/test 使用相同 request id 时拒绝。
- request id 不存在时拒绝。
- 两条请求属于其他 contract 时拒绝。
- 两条真实不同请求且差异 artifact 完整时允许 confirmed。

## 14. Mock 端到端验收

使用本地 mock 服务，禁止用真实生产目标做自动测试。至少覆盖：

1. 匿名 SPA：正常完成，不显示账号 A/B。
2. 单账号：会话有效，覆盖认证后 API，不伪造双账号差异。
3. 双账号平权：正常业务个性化差异被分类，不报越权。
4. 双账号权限差异：`roleId`/`tenantId` 差异被保留为高价值证据。
5. 一侧请求缺失：输出 insufficient evidence，不报漏洞。
6. 页面加载第三方埋点和 CDN：正式 API 列表只包含授权业务域。
7. WAF/CAPTCHA：protected_stop，其他 URL 不受污染。
8. attempt 中断后续跑：不重复已完成请求和契约。
9. checkpoint v1：迁移后续跑或明确要求 fresh，不静默重跑。
10. 工具返回含 token：模型请求捕获中不存在 token 原值。

每个端到端用例必须断言：

- 最终 target status
- attempt status
- 请求计数
- Token 计数
- 覆盖 evidence
- finding 数量
- 当前 UI 摘要所需字段
- 历史 attempt 不污染当前结果面

## 15. 实施顺序

严格按以下顺序，每阶段保持可编译：

1. 新增失败测试和测试 fixture。
2. 增加 attempt parent/resume metadata 和状态迁移器。
3. 修复 frozen execution plan 的读取与继承。
4. 实现唯一 scope decision 并接入 HTTP/CDP/发现工具。
5. 实现 raw artifact/model view 双视图和统一脱敏。
6. 替换工具参数校验器。
7. 重构 A/B 字段分类。
8. 引入 request record id、artifact id 和 coverage evidence id。
9. 收紧 confirmed finding。
10. 统一终态 reducer 和当前结果面清理。
11. 运行 mock 端到端测试。
12. 更新架构文档、版本日志和必要注释。

不要并行复制多套临时实现。每完成一步先删除被替代的旧分支，再进入下一步。

## 16. 完成前必须执行的命令

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run build
git diff --check
```

还必须运行本文件第 14 节的本地 mock 端到端用例。

## 17. 实现者最终回复格式

最终回复必须逐项给出，不得只说“已完成”“已优化”：

1. 修改的文件列表。
2. attempt 继承模型和数据库/metadata 变化。
3. frozen plan 如何保证不被设置变化覆盖。
4. v1 checkpoint 如何迁移。
5. 哪些出站路径接入了统一范围判定。
6. raw artifact 与 model view 如何分离。
7. JSON Schema 校验支持哪些关键字。
8. A/B 字段分类变化。
9. coverage evidence 和 request record id 如何关联。
10. confirmed finding 的硬条件。
11. 新增测试清单和测试结果。
12. 仍未完成的技术债、影响和后续建议。

任何一项没有完成，必须明确写“未完成”，不得用“基本完成”“已兼容”“后续可优化”掩盖。

## 18. 完成定义

只有同时满足以下条件，才允许将任务标记完成：

- ContinueIncomplete 在新 attempt 中真实继承父 attempt，不重复已执行工作。
- Fresh 与 ContinueIncomplete 语义在代码和测试中完全分离。
- 设置变化不会修改正在继续的冻结执行计划。
- v1 checkpoint 不会静默丢失。
- HTTP、浏览器 entry、CDP observed request 和重定向统一受 scope 控制。
- 未脱敏原始响应不会进入模型历史。
- `roleId`、`tenantId`、`departmentId`、`permissionId` 不会被当成普通 ownership 差异。
- coverage 由 evidence id 证明，而不是模型 family 字符串。
- finding 绑定两条不同 request id 和响应差异 artifact。
- 所有工具参数通过完整 schema 校验。
- Rust 测试、Clippy、前端构建、diff 检查和 mock 端到端全部通过。
- 当前结果面只有一个真实终态，历史错误不会污染最新 attempt。

