# Oviraptor 原生 Agent Phase 2 完成任务书

> 适用代码基线：`1.1.59` 工作区未提交版本  
> 目标读者：下一轮实现者（Qwen / Codex / 人工开发）  
> 本文件是验收合同，不是方向建议。实现者必须先阅读现有架构、整改任务书和本文件，再改代码。

## 0. 当前审查结论

当前版本已经完成了有价值的基础建设：

- URL Web 任务具备原生模型客户端、七个工具、证据账本、身份隔离、范围判定和预算限制；
- Python 资产业务 worker 已迁移为 Rust，删除的旧 Python worker 没有残留路径引用；
- 前端生产构建通过；
- `cargo fmt --check` 通过；
- `cargo clippy --lib --all-targets -- -D warnings` 通过；
- Rust 单元与 Mock 端到端测试共 258 项全部通过。

但是当前版本还不能宣布“原生 Agent 已接管 Strix”。以下问题仍然存在：

1. `auto` 仍固定选择 Strix，原生 Agent 必须手工切换；
2. fresh 重跑仍可能读取上一轮 `agent_execution_plan`，导致新配置无法切换后端；
3. 启动依赖只按第一个 URL 判断，混合 native/Strix 目标可能在后续 URL 才失败；
4. `agent_runtime` 的恢复器、工具事务、租约、消息和 reducer 尚未真正成为原生循环的唯一事实来源；
5. 关键检查点、使用量和结果写入存在忽略数据库错误的路径；
6. 云模型取消只停止等待，后台阻塞线程仍可能继续完成网络请求；
7. UI、命令名、配置键和提示文案仍大量以 Strix 为中心；
8. `BoundedCompleted` 在扫描汇总中按普通完成计数，覆盖缺口容易被弱化；
9. 现有端到端测试主要基于 Mock，没有真实 Tauri + CDP + 本地模型联调门禁；
10. 新代码再次形成 3,000–6,000 行的超大文件，后续维护风险已经出现。

本阶段只处理上述事实，不继续叠加新的漏洞类型、页面或多 Agent 功能。

## 1. 总目标

完成一个真正由 Oviraptor 自己拥有的 URL Web Agent 闭环，使以下条件同时成立：

- `auto` 对符合条件的 URL Web 任务默认使用 native；
- 用户显式选择 `strix` 时才启动 Strix；
- native 任务没有 Strix、Docker 或 Python 也可以完整运行；
- fresh、resume、pause、cancel、崩溃恢复的语义互不混淆；
- 模型、工具、检查点、Token、终态只有一套权威记录；
- 任一持久化失败都不能继续假装成功；
- UI 能明确说明本轮由谁执行、做到哪里、为什么结束、还缺什么；
- 不允许为了切换 native 而暗中回退 Strix 或重复执行目标。

## 2. P0：修复 fresh 重跑仍复用旧后端

### 2.1 当前缺陷

`agent_select_backend` 会先读取 URL 的 `agent_execution_plan`，不区分当前 attempt 是 `fresh` 还是 `resume`。而 fresh 重跑只清理 `native_agent_state`，故上一轮后端会继续钉住下一轮。

结果：

- 第一次 Strix，随后把策略改为 native 并“重新执行”，仍可能继续 Strix；
- 第一次 native，随后显式改为 Strix 并 fresh 重跑，也可能继续 native；
- UI 展示的新策略与实际执行后端不一致。

### 2.2 必须修改

- 后端选择必须读取当前 attempt 的 `execution_mode`；
- `initial` / `fresh`：从当前配置重新选择并覆盖本 attempt 的计划；
- `resume`：只能继承父 attempt 的冻结计划；
- 正在运行的同一 attempt：只能读取已落库的冻结计划；
- 禁止通过删除全部历史记录来实现；历史计划必须保留在 attempt/run 维度。

建议将 checkpoint 主键从“scan + URL + stage 的当前快照”升级为至少能表达 attempt，或把冻结计划直接绑定 `agent_runs`，旧表只保留兼容投影。

### 2.3 必须新增的失败测试

1. attempt 1 = Strix，attempt 2 = fresh + native：第二轮必须 native；
2. attempt 1 = native，attempt 2 = fresh + Strix：第二轮必须 Strix；
3. attempt 1 = native，attempt 2 = resume，期间配置改成 Strix：第二轮仍必须 native；
4. resume 缺少父计划：必须 `resume_incompatible`，且模型请求和目标请求均为 0；
5. fresh 不得继承父轮次 Token、队列、终态、no-progress 或已耗尽契约。

## 3. P0：扫描启动前计算全部目标的后端矩阵

### 3.1 当前缺陷

扫描是否需要 Strix/Docker 目前主要依据第一个 URL 的后端判断。不同 URL 可能拥有不同历史计划，因此可能出现：

- 第一个目标 native，启动时跳过 Strix/Docker；
- 后续目标被旧计划选成 Strix；
- 运行到后续目标才发现 Strix 路径为空并失败。

### 3.2 必须修改

启动前生成不可变 `ScanBackendPlan`：

```text
scan_id
attempt_number
targets[]:
  url
  backend
  selection_reason
  plan_hash
requires_strix
requires_docker
requires_node
requires_browser
```

- 遍历本轮全部待执行 URL 后再解析依赖；
- 全 native 时不得探测 Strix、Docker、Python；
- 混合后端时必须完整准备两套依赖，或者在启动前明确拒绝并提示 fresh 重建，不得执行到一半才失败；
- 每个目标实际执行前校验其后端与 `ScanBackendPlan` 一致；
- 中途修改设置不能改变已冻结矩阵。

## 4. P0：让 agent_runtime 成为真正的唯一事实来源

### 4.1 当前缺陷

当前 `agent_runtime` 已有正确的表和 API，但生产路径尚未完整使用：

- `checkpoint::recover` 主要停留在测试；
- `store::begin_tool_invocation` / `finish_tool_invocation` 没有由原生工具循环统一调用；
- 原生循环仍通过 `AgentRunLedger` 手工插入部分事件和工具记录；
- `renew_lease`、消息队列和中断工具恢复没有进入实际编排；
- reducer 写一套终态，旧 `sentinel_targets` / tally 又写一套终态。

这会形成两套“看起来都正确”的状态，在崩溃、取消或数据库写入失败时出现分叉。

### 4.2 必须修改

原生循环每次工具调用必须采用以下事务顺序：

1. 校验 schema、scope、身份和授权；
2. `begin_tool_invocation`，落库 `running`；
3. 执行工具；
4. 原始响应写 Artifact Store；
5. `finish_tool_invocation` 写入确定状态和 artifact id；
6. append event；
7. 更新 snapshot；
8. 提交事务后才能把结果送回模型。

启动/恢复必须采用：

1. 加载 `agent_run`；
2. 加载 snapshot；
3. replay snapshot 之后的 event；
4. 将遗留 `running` 工具标记为 `interrupted`；
5. 仅把没有确定结果的契约放回队列；
6. 已消耗预算不得回退。

最终终态必须先由 reducer 产生，再投影到 `sentinel_targets` 和 `sentinel_scans`。旧表不再独立推导另一套结论。

## 5. P0：所有关键持久化改为 fail-closed

### 5.1 当前缺陷

下列路径会忽略数据库错误：

- `write_agent_checkpoint`；
- `NativeAgentState::persist`；
- 原生 Token 使用量累加；
- coverage/evidence 部分 `insert_finding`；
- 部分 runtime event / tool invocation 写入。

如果磁盘满、数据库锁超时或 schema 异常，扫描仍可能继续请求目标和模型，最后却无法恢复或无法解释成本。

### 5.2 必须修改

- 上述函数全部返回 `Result`；
- 工具结果、artifact、usage、checkpoint 必须在可验证的提交边界内完成；
- 写入失败后立即停止新的模型/目标请求；
- 终态为独立的 `persistence_failure`，不能伪装成模型失败或普通 partial；
- UI 明确显示“本地记录失败，已停止以避免重复消耗”；
- 数据库 busy 应有短暂、有界重试，但不能无限等待。

### 5.3 必须新增测试

- SQLite busy；
- 磁盘写入失败；
- artifact 成功而 DB 失败；
- DB 成功而 artifact 失败；
- usage 写入失败时不得发起下一次模型请求。

## 6. P0：取消必须真正中止网络请求

### 6.1 当前缺陷

云模型请求由独立 blocking thread 执行。取消后主循环停止等待，但线程和 HTTP 请求可能继续到服务器响应或 300 秒超时，造成：

- UI 显示已停止，但云端仍可能继续计费；
- 多次停止/重试后残留后台线程；
- 旧响应虽然不会入库，但仍占网络和供应商并发。

### 6.2 必须修改

- 模型网关改为可取消的 async HTTP；
- 使用统一 cancellation token 取消 future 并释放 response/body；
- 本地 hook 与云端直连都必须验证对端连接断开；
- 禁止 detached blocking worker 作为长期方案；
- cancel 后必须等待请求句柄进入确定状态，再写 terminal event；
- 记录 `cancel_requested_at`、`transport_closed_at`，UI 可显示取消是否真正完成。

### 6.3 验收测试

使用一个会保持连接 60 秒的测试服务器：

- 发起请求后取消；
- 2 秒内客户端任务退出；
- 服务端观察到连接断开；
- 没有后台线程继续持有连接；
- Token/请求只按实际已经提交的请求记账，不因恢复重复计算。

## 7. P0：定义 auto 切换到 native 的发布门禁

### 7.1 新语义

完成本文件 P0 后：

- `native`：强制 native，不允许回退；
- `strix`：强制 Strix；
- `auto`：URL-only Web/无源码灰盒默认 native；源码、CI 或 native 明确不支持的能力才选择 Strix；
- 任一后端启动后失败，不得静默换另一个后端重复扫描。

### 7.2 能力判定

启动前检查：

- 模型端支持 tool calling；
- Base URL、模型名和凭据规则有效；
- Node/CDP helper 可用（仅需要浏览器时）；
- 目标证据包完整；
- 当前任务类型在 native capability manifest 中。

能力不足必须在请求目标前报清楚缺项。不要用“先试 native，失败再 Strix”实现 auto。

## 8. P1：统一模型配置，去除 Strix 命名绑架

### 8.1 数据模型

新增中立配置：

- `modelProfiles`；
- `activeModelProfileId`；
- `modelDeployment`；
- `modelApiBase` / `modelApiKey` / `localApiKey`；
- `agentBackendPolicy`。

将旧 `strixLlmProfiles` 等字段作为一次性迁移源。迁移后只写新字段，旧字段只读兼容，最终版本删除。

### 8.2 UI

- “Strix 模型配置”改为“模型与 Agent”；
- 模型连通测试必须包含 tool-calling 能力测试；
- native 时隐藏 Strix executable、Docker image 和 Strix 专属预算；
- Strix 时才显示第三方运行时设置；
- 云端与本地继续严格分开，禁止继承另一侧凭据。

## 9. P1：统一产品命名和入口

新增用户可见名称：

- “自动调查”——创建任务；
- “任务中心”——进度、成本、停止、续跑；
- “证据与结论”——请求、响应、对照和漏洞；
- “运行轨迹”——模型、工具、Token 和终态；
- “兼容后端”——只有设置页显示 Strix。

保留旧 Tauri command 名仅用于 ABI 兼容，新增中立 command 并让前端改用。日志中不能把 native 的前端探测、暂停、恢复或完成描述成“Strix 扫描”。

## 10. P1：修正 BoundedCompleted 和覆盖缺口展示

- tally 分开统计 `completed` 与 `completed_with_gaps`；
- 全部目标都是 bounded 时，扫描终态不得显示“无异常中断的完整完成”；
- UI 必须展示：已覆盖、未覆盖、不可适用、因预算未完成、建议人工深入；
- “没有漏洞”与“没有完成验证”必须视觉和数据上分开；
- 只有所有必需 family 已覆盖或被证据支持地标记 not-applicable，才是完整完成。

## 11. P1：真实端到端验收站点

新增本地 fixture 应用，至少包含：

1. 匿名 SPA；
2. 一个账号；
3. 两个平权账号；
4. 两个权限不同账号；
5. 登录失效；
6. 普通 401/403；
7. 明确 WAF/机器人挑战；
8. 持续 429；
9. 隐藏 GET 接口；
10. 参数反射但无 XSS；
11. 可确认的反射 XSS；
12. 水平越权阳性与阴性；
13. 错误信息泄漏；
14. source map 暴露；
15. 第三方遥测与静态资源噪音。

使用确定性 Mock 模型驱动真实工具与真实 HTTP/CDP，不允许直接伪造工具结果。验收内容包括：

- 请求确实发出；
- A/B cookie/header 不串线；
- 噪音不进入正式 API；
- confirmed 绑定真实 control/test request id；
- WAF 才熔断，普通 401/403 不熔断；
- pause/resume 不重复请求；
- fresh 清理运行态但保留历史和已确认漏洞；
- UI 最终状态与数据库 reducer 一致。

## 12. P2：代码拆分

禁止继续扩大以下文件：

- `src-tauri/src/commands/agent_tools.rs`；
- `src-tauri/src/commands/agent_tests.rs`；
- `src/components/SentinelBoard.vue`；
- `src/sentinel.css`。

建议结构：

```text
src-tauri/src/agent_runtime/
  orchestrator.rs
  lifecycle.rs
  tool_broker.rs
  policy.rs
  tools/
    inspect.rs
    replay.rs
    identity.rs
    discovery.rs
    browser.rs
    verdict.rs
    finish.rs
  tests/
    lifecycle.rs
    scope.rs
    identity.rs
    persistence.rs
    e2e.rs

src/features/sentinel/
  components/
    TaskCenter.vue
    ExecutionPlan.vue
    EvidenceViewer.vue
    CoverageLedger.vue
    RuntimeTrace.vue
  composables/
    useScanLifecycle.ts
    useAgentExecution.ts
```

拆分只能移动权责清晰的模块，不能复制旧函数再包一层。

## 13. 本阶段明确不做

- 不实现默认多 Agent；
- 不增加新的攻击型工具；
- 不扩大目录爆破字典；
- 不增加远程 worker；
- 不重写资产和暴露面页面；
- 不删除 Strix 兼容后端；
- 不用“更多 prompt”掩盖状态机和持久化缺陷。

## 14. 实施顺序

必须严格按以下顺序：

1. 为第 2–6 节先写失败测试；
2. 修 fresh/backend matrix；
3. 接通 runtime 工具事务、恢复与 reducer；
4. 改造 fail-closed 持久化；
5. 改造真正可取消的模型传输；
6. 通过真实 fixture E2E；
7. 把 `auto` 切到 native；
8. 迁移中立模型字段和 UI 文案；
9. 拆分超大文件；
10. 最后更新版本号、README、架构图和发布日志。

不要先改 UI 宣称 native 已完成，再补后端。

## 15. 完成前必须执行

```bash
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --lib --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

另外必须执行：

- fixture E2E 全矩阵；
- native 模式在没有 Strix、Docker、Python 的干净环境运行；
- Strix 显式模式兼容测试；
- initial / fresh / resume / pause / cancel / crash-recovery 状态矩阵；
- 本地慢模型和云模型取消测试；
- SQLite busy / artifact 写失败测试；
- 两账号身份隔离测试；
- 任务详情、成本、覆盖账本和运行轨迹的 UI 人工检查。

## 16. 完成定义

以下条件全部满足才算 Phase 2 完成：

- `auto` 的 URL Web 任务默认 native；
- fresh 可以按当前设置切换后端，resume 永远继承父计划；
- 多 URL 后端依赖在启动前确定，不会运行到中途才缺 Strix/Docker；
- native 的工具、恢复、预算和终态全部由 `agent_runtime` 权威记录；
- 关键写入失败立即停止，不产生假完成；
- 取消会真正断开本地和云端模型请求；
- 未覆盖项不会被普通“已完成”吞掉；
- native 任务 UI 不再以 Strix 命名；
- 真实 fixture E2E 通过；
- 没有隐式后端回退、重复扫描或重复计费；
- 旧任务仍可读，旧配置可迁移，旧 Strix 后端可显式运行。

