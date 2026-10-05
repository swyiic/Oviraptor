# Oviraptor 原生 Agent 执行器实施任务书

> 状态：URL Web 单 Agent 执行器已经落地，不要按本文重新实现。
> `agentBackendPolicy=auto` 对符合条件的 URL Web / 无源码 greybox 已选择 Native；带源码、Code、CI 仍走 Strix，这是总计划 Stage 4–5 要删的活路径，不是本文的返工项。
> 当前唯一实施合同是 `docs/NEST_STRIX_FREE_MULTI_AGENT_MASTER_PLAN.md`。下一轮只做 Stage 0。

## 1. 目标

把 Oviraptor 改为调查策略、预算、证据、工具和任务生命周期的真正所有者。Strix 降级为可选执行后端，不再决定扫描是否开始、何时停止、覆盖哪些能力或如何解释终态。

默认不增加用户配置负担：继续复用“设置与 API”里的现有云端/本地模型配置。新增后端策略默认为 `auto`：新 Web 任务优先使用原生 Agent；只有原生后端在首次模型或目标请求发生前明确返回 `unsupported_capability` 时才能切换 Strix，禁止扫描到一半再用 Strix 重跑造成双倍 Token。

## 2. 当前代码基线

必须保留并复用：

- `src-tauri/src/commands/agent_runtime.rs`：Oviraptor 持有的执行计划，包含软/硬预算、覆盖族、发现轮次和停止语义。
- `src-tauri/src/commands/investigation.rs`：调查图谱、假设、API、身份差异和渐进式基础调查决策。
- `src-tauri/src/commands/frontend_recon.rs`：确定性前端/CDP 证据包。
- `src-tauri/src/commands/native_frontend_recon.rs`：Rust 原生前端侦察。
- `src-tauri/src/commands/native_src_transport.rs`：原生 HTTP、Raw HTTP 和并发调度能力。
- `src-tauri/src/commands/scan_execution.rs::run_adaptive_strix_target`：现有 Strix 后端实现，只能作为适配器保留。
- `src-tauri/src/commands/scan_control.rs`：任务、重试、继续、暂停和删除入口。
- `src-tauri/src/commands/result_ingestion.rs`：统一结果与历史状态修复。

工作区已有大量未提交修改。禁止执行 `git reset --hard`、`git checkout -- .` 或覆盖与本任务无关的文件。

## 3. 必须新增的后端边界

在 `src-tauri/src/commands/` 增加以下文件，并在 `commands.rs` 中按依赖顺序 `include!`：

1. `agent_contract.rs`
2. `agent_model_client.rs`
3. `agent_tools.rs`
4. `agent_native.rs`
5. `agent_backend.rs`

最少需要这些类型：

```rust
enum AgentBackendKind { Native, Strix }

enum AgentTargetOutcome {
    Completed(AgentCompletion),
    BoundedCompleted(AgentCompletion),
    Incomplete(AgentStop),
    Limited(AgentStop),
    Failed(AgentStop),
    Cancelled,
}

struct AgentRunContext {
    scan_id: String,
    attempt_number: i64,
    target_url: String,
    target_dir: PathBuf,
    route: FrontendRoute,
    execution_plan: AgentExecutionPlan,
    evidence: JsonValue,
    capabilities: JsonValue,
    identities: Vec<AgentIdentity>,
}

trait AgentBackend {
    fn kind(&self) -> AgentBackendKind;
    fn execute(&self, context: &AgentRunContext) -> AgentTargetOutcome;
}
```

把 `StrixTargetOutcome` 改成后端无关的 `AgentTargetOutcome`。现有 `run_adaptive_strix_target` 应移动或包装到 `StrixAgentBackend`，但不得继续承担通用预算、状态解释和任务聚合职责。

## 4. 原生模型客户端

`agent_model_client.rs` 必须直接复用现有模型档案解析结果，严格区分本地和云端：

- 支持 OpenAI-compatible `chat/completions` 工具调用。
- 使用当前 profile 的模型名、API Base、API Key、代理和本地资源策略。
- 不从旧的全局环境变量重新读取另一套 Key。
- 本地模型：串行请求；模型请求正在返回时不触发响应超时；仍允许人工停止。
- 云端模型：保留连接超时、请求超时、429/5xx 分类和一次提供商级重试。
- 每次请求记录输入、缓存输入、输出和总 Token；服务端未返回 usage 时沿用现有估算逻辑。
- 工具 schema 不得在每一轮重复膨胀；保存一次稳定 schema hash。
- 上下文接近窗口时，压缩已完成分支，只保留系统规则、执行计划、未完成队列、已确认事实、最近两轮工具结果和覆盖账本。

禁止为本地模型伪造 API Key 校验。禁止把本地慢响应误报为认证失败。

## 5. 原生工具注册表

`agent_tools.rs` 至少提供以下工具，所有参数均使用严格 JSON Schema：

### `inspect_evidence`

- 读取 Oviraptor 已压缩证据，不允许任意读取整个工作目录。
- 支持按 API、动作、身份差异、敏感信息、路由候选查询。

### `replay_http`

- 复用实际采集的 method、URL、headers、cookies、body 和 content type。
- 默认只读；写请求必须具备现有 contract、尝试次数、清理步骤和恢复条件。
- 限制到任务目标或证据中明确记录的关联业务域名。
- 保存请求与响应原始字节、状态、内容类型、耗时和结构摘要。

### `compare_identities`

- A/B 身份分别加载各自任务级 session，不得混用 Cookie/Authorization。
- 对同一规范化 method + host + path 比较状态码、字段集合、对象归属、关键值和响应结构。
- 单账号或匿名任务不生成“账号 A/B”文案。
- 一侧未采集时返回 `insufficient_evidence`，不得推断越权。

### `targeted_discovery`

- 词表只能来自同源链接、表单、JS AST 调用点、已观察业务词、产品路由和本地知识库。
- 只验证同源或证据明确关联的业务域名。
- 图片、字体、遥测、埋点、广告、Sentry、设备指纹和静态资源不得进入正式 API。
- UNKNOWN method 不得执行；必须先从调用点、表单或实际运行时恢复 method。
- 每轮输出新增正式端点、不同响应和无增量签名。

### `browser_action`

- 只执行 Oviraptor 前端动作图中可定位的按钮、链接、表单和路由。
- 动作后采集网络增量，不重新抓取整个站点。
- CAPTCHA/WAF/机器人挑战立即返回保护信号。

### `record_hypothesis_result`

- 状态只能是 `confirmed`、`rejected`、`exhausted`、`insufficient_evidence`。
- confirmed 必须包含控制请求、测试请求、响应差异、影响和可复现步骤。

### `finish_target`

- 输出覆盖账本、未覆盖原因、确认问题、排除项、人工深入建议和停止原因。
- 未覆盖不能写成安全；没有漏洞也必须形成正常终态。

## 6. Agent 循环

在 `agent_native.rs` 实现真实循环：

1. 加载 `frontend-evidence.json`、任务级身份、能力清单和 `AgentExecutionPlan`。
2. 建立覆盖队列：信息泄露、错误处理、认证会话、授权、输入反射/XSS、隐藏接口、业务流程。
3. 优先执行已有高价值契约，再执行正式运行时 API，然后才使用定向发现轮次。
4. 每轮只允许模型选择一个或一组互不重复的工具动作。
5. 工具参数先经过本地校验，模型不能绕过目标范围、身份隔离、次数和清理要求。
6. 每轮计算进展签名：新端点、新响应结构、新身份差异、新参数、新验证结论、新覆盖族。仅 Token 增加或重复工具调用不算进展。
7. 达到软预算但仍有进展时继续；连续达到 `no_progress_window` 才收口。
8. 硬 Token、硬请求数、明确 WAF/CAPTCHA、持续 429、用户暂停或取消必须终止。
9. 普通 401/403 只记录权限边界，继续其他分支。
10. 模型必须显式调用 `finish_target`；若模型正常退出但没有调用，Oviraptor 根据本地覆盖账本生成终态，不能显示成莫名其妙的“部分完成”。

## 7. 状态与重试

新增检查点 `native_agent_state`，主键仍使用 `scan_id + url + stage`，内容至少包括：

- schemaVersion
- attemptNumber
- backend
- evidenceHash
- executionPlanHash
- completedContractKeys
- exhaustedContractKeys
- coveredFamilies
- pendingQueue
- progressSignature
- budgetUsage
- terminalReason

规则：

- “重新执行”：新 attempt，清空旧的运行中、错误、预算和终态标识；保留已确认漏洞和确定性前端证据，但重新生成执行计划。
- “继续未完成阶段”：只恢复 evidenceHash 和 planHash 都匹配的未完成队列；保留已完成契约，不重复消耗。
- session 只属于当前任务；新建任务不得复用其他任务历史 session。
- 同一目标同一 attempt 只能有一个后端；禁止 Native 和 Strix 同时运行。
- 每次状态变化都更新 `sentinel_scan_attempts` 和 `sentinel_targets`，最终只显示一个真实终态。

## 8. 结果入库

原生后端不要伪造 Strix 文件。直接写统一表：

- 请求/响应证据：`sentinel_findings`，使用独立 stage `native-agent-evidence`。
- 漏洞：`sentinel_findings` 的统一 vulnerability contract。
- 覆盖账本：stage `native-agent-coverage`。
- 运行轨迹：检查点和 attempt ledger。
- 调查图谱：更新现有 hypothesis/api/identity/action 节点状态。

漏洞字段必须包含：标题、严重度、CWE/CVSS、置信度及依据、控制请求、测试请求、影响、反证检查、等级变化条件、修复建议、修复验证和结论修订历史。

只有 `confirmed` 可以进入漏洞结论；`insufficient_evidence` 和人工建议不能算漏洞或成功验证。

## 9. 后端选择和兼容

在现有配置 JSON 增加 `agentBackendPolicy`，允许 `auto | native | strix`，默认值 `auto`。UI 放在高级设置，不要求用户每次创建任务选择。

- `auto`：新任务使用 Native；只有首次推理和首次目标工具调用前的明确不支持才允许切 Strix。
- `native`：失败时保留原生错误，不自动双跑。
- `strix`：保留当前行为，便于回归和紧急兼容。
- 已开始的 attempt 把选定后端写进执行计划；配置改变不能切换正在执行的 attempt。

现有 Asset 创建任务和 Strix 页面创建任务必须走同一个后端选择函数，禁止保留两套入口逻辑。

## 10. 必须删除或迁移的重复逻辑

完成原生后端后检查并处理：

- `scan_execution.rs` 中通用预算/停止判断迁移到后端无关 orchestrator，Strix 适配器只负责进程与 artifact。
- 不得重新引入已删除的 `adaptive_target_limits`。
- 状态聚合只能有一个函数；禁止 Native、Strix、重试分别拼接三套“部分完成”文案。
- 后端无关的 WAF、401/403、无进展、软/硬预算语义只能定义一次。
- 旧版本历史修复 SQL 不得把 `baselineInvestigationAllowed=true` 的任务改成 `recon_only`。

## 11. UI 最低要求

任务详情增加“执行计划”折叠区，展示：

- 后端：原生 Agent / Strix
- 模式：快速 / 标准 / 深度
- 当前覆盖族与完成比例
- 软预算、硬上限和当前消耗
- 最近一次扩容原因
- 当前动作、最近新证据、无进展计数
- 最终停止原因

不要展示原始运行 JSON。详细请求响应继续使用现有 Burp 风格查看器。

## 12. 测试要求

必须新增单元/集成测试：

1. 标准模式首轮没有 API，但页面可交互时会执行基础覆盖而不是 `recon_only`。
2. Token 超过软预算但持续产生新端点时不会停止。
3. 连续无进展达到窗口后正常收口，不标记失败。
4. 硬 Token/请求上限必定停止。
5. 401/403 不触发熔断；WAF/CAPTCHA/持续 429 触发。
6. A/B 身份 Cookie、Authorization 和响应完全隔离。
7. 匿名任务不显示账号 A/B。
8. 遥测、图片、字体、静态资源和 UNKNOWN method 不进入正式 API。
9. 继续任务不重复已完成契约；重新执行清理旧终态。
10. `auto` 不会在 Native 已经消费 Token 后回退 Strix。
11. 本地模型慢响应不触发云端超时或 API Key 错误。
12. Native 结果能出现在现有证据、漏洞、覆盖和 Token UI 中。

完成后执行：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
git diff --check
```

另外建立一个本地 mock OpenAI-compatible 服务和 mock Web 站点，至少跑通：匿名 SPA、单账号、双账号、WAF、无发现收口五条端到端用例。测试不得访问真实生产站点。

## 13. 分阶段提交顺序

1. 后端无关 contract、状态和测试。
2. 模型客户端与 usage 统计。
3. 工具注册表和严格参数校验。
4. Native Agent 循环与检查点。
5. 统一 orchestrator 接入现有任务入口。
6. 结果入库和 UI。
7. Strix 适配器迁移、删除重复逻辑。
8. 全量测试与 mock 端到端验证。

每阶段都必须保持项目可编译。不要一次性重写整个 `scan_execution.rs`，也不要复制旧逻辑后留下两套状态机。

## 14. 完成定义

以下条件全部满足才算完成：

- `agentBackendPolicy=native` 时，不启动 Strix 进程也能完成一次标准 Web 调查。
- 执行计划、证据、Token、工具结果、覆盖账本和终态全部可追溯。
- 标准/深度模式不会因首轮缺少高分候选直接结束。
- 无增量会及时停止，有增量可以超过软预算继续，永远受硬上限约束。
- 重试、继续、会话隔离和最终状态不存在历史污染。
- 全部 Rust 测试、前端构建和五条 mock 端到端用例通过。
