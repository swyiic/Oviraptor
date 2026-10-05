# Mapper / Identity 已存结果的运行期恢复审计（2026-09-26）

> 2026-09-28 源码入口补充：共享 `prepare_readonly_child` 原先未检查旧 fencing。实际源码 fixture 中，旧 RepoMapper 已完成且 lane 释放后，同一 attempt 的新 Coordinator fence 可创建 SourceAnalyst。回归 `source_specialists_rotated_fence_cannot_schedule_second_role_in_same_attempt` 先红后绿；现在在调度的 IMMEDIATE 事务内调用 `ensure_fresh_readonly_fence`，签发新预算、child 和能力前拒绝旧 generation 的只读专家。源码专家专项 11 项、fmt 和局部空白检查通过；`CARGO_BUILD_JOBS=1 cargo test --offline -j 1 --all-targets --all-features --quiet -- --test-threads=1` 主库 **1364/1364**、集成 **30/30**、exit 0，严格 `cargo clippy --offline -j 1 --all-targets --all-features -- -D warnings` exit 0。全量运行约 23 分钟，保持单编译任务和单测试线程。这是新 attempt 边界，不是旧回执迁移、终态恢复 UI 或授权 URL 验收。

## 交付范围

接续 `NEST_SPECIALIST_RECEIPT_AUDIT_2026-09-26.md`。此前模型 received 回执已经可靠落库，但交付失败后的 paused 子任务不能结算，编排入口也拒绝此前 completed/paused 的分析 assignment。

本增量处理 **当前活跃 attempt、仍可执行的 Coordinator、相同且有效的 fencing、尚未开始目标执行** 的 Mapper / Identity 初始化分析恢复。不是整任务任意断点续跑，不重发未知模型请求，不扩大 Web 目标权限。

## 实现与实际入口

### 1. 初始化分析的创建和启动原子化

`agent_runtime/multi_agent/scheduler.rs::prepare_readonly_child` 被 `multi_agent_prepare` 的 Mapper / Identity 路径使用。

- 新 assignment、child、预算预留、lane、capability 和 running 转换在同一个 IMMEDIATE 事务中提交。
- 开始写入失败时整体回滚，不留下“已分配但无法启动”的新子任务。
- 重入时核对确定性 assignment ID、dedup、role、lane、target、scan、attempt、epoch、fencing、trigger、revision、task slice 及 assignment/run 对应状态。
- 已存在的 running/paused/completed 分析子任务不重新调度、不重新预留、不重新授予权限。paused/completed 必须存在可校验的 received 回执。
- 输入重建时仅移除 Mapper / Identity 自己在 context 中生成的输出，避免第二次经过 Mapper 时把其上次输出错误当成新源证据；真实源证据变化仍由冻结请求 hash 拒绝。
- 已有 target-touching Executor 的 attempt 仍拒绝重入，不借此重跑目标请求。

### 2. paused → 本地完成，不经过 running

`specialist.rs::received_for_reconciliation` 只在调用方事务内读取、验证既有回执。检查当前归属、request hash、response hash、用量、model event 和 checkpoint，不创建模型派发许可。

`complete_readonly_assessment` 新增专用暂停恢复分支，要求：

1. role 为 Mapper 或 Identity，root/attempt 仍有效；
2. assignment 和 child 均 paused，原因为 `child_usage_reconciliation_required`，预算尚未结算；
3. lane 仍准确归属于该 assignment，所有执行 capability 已撤销；
4. received 回执有效、没有空结果/意外工具调用等拒绝标记，传入正文与用量等于保存结果。

满足条件后，在同一事务里结算原模型成本、直接结束 paused child、释放 lane、写结果 mailbox、投递并确认。任何一个步骤失败均回滚，保留原暂停及费用预留。成功的子任务使用 `readonly_receipt_reconciled` 终态码。

通用 scheduler 和普通结算入口没有开放 paused worker 执行；本地恢复不临时改回 running，也不重新签发 capability。重复/并发补齐只结算、投递一次。

### 3. 生产链路的一次本地补交

`multi_agent_prepare` 调用 `deliver_readonly_assessment`：第一次原子交付失败后，先完成暂停/撤权，再最多尝试一次上述本地恢复。该函数不包含模型传输、工具调用或新调度。清理失败直接报告原始错误和清理错误，不猜测状态；恢复失败同时保留第一次错误和恢复原因。

这是有界的本地数据库恢复，不是循环重试 provider。最终仍失败时，上层原有终态收口继续处理；没有实现后台无限重试或终止后重新激活任务。

### 4. 观察者不能冒充模型派发者清理子任务

模型回执返回 `specialist_call_outcome_unknown_requires_reconciliation` 或冻结请求不匹配，说明当前调用者没有取得本次派发权。`failed_specialist_error` 不再因这两种观察结果暂停另一个调用者正在完成的子任务。

此修复的验证范围是 `multi_agent_prepare` 内的调度/模型/结果交付竞争。当时外层实际入口 `run_agent_target` 的并发调用仍可能进入整体失败收口。同日后续 `NEST_INVOCATION_OWNERSHIP_AUDIT_2026-09-26.md` 已增加流水线／目标活调用者锁和非拥有返回值隔离；全生命周期历史结果观察、崩溃恢复及投影收敛仍不能宣称完成。

## 新增测试

`src-tauri/src/commands/agent_tests_specialist_recovery.rs`，9 项：

1. `pipeline_reuses_completed_and_paused_analysis`：实际 Mapper 和 Identity 两条失败路径移除故障后重入；原先完成和暂停的角色都不重发模型、重授能力或重记成本；随后创建 Executor，但不实际请求目标。
2. `paused_delivery_rolls_back_every_partial_write`：8 类写入位置分别注入 ABORT / IGNORE；失败保持 paused、原费用预留和无部分消息；解除故障后重复完成仍只记一次。
3. `rejects_mismatched_or_unproven_receipts`：缺失回执/event/checkpoint、其他暂停原因、残留执行能力、丢失 lane、换代/过期、暂停/替换 attempt、终止 root、错误目标均拒绝。
4. `payload_and_usage_must_match_saved_result`：错误结果正文与错误用量不能用有效回执结算。
5. `observer_cannot_pause_dispatch_owner`：真实 loopback 模型请求停留在处理过程中，另一个编排调用观察到 executing；不撤销原角色能力，原调用继续完成；仅一个模型请求。
6. `production_delivery_recovers_locally_once`：通过条件触发器让原交付失败，真实生产 prepare 路径暂停后本地补交成功；禁止 paused → running；模型请求仍只有一次。
7. `concurrent_local_completion_settles_once`：两个连接同时补齐同一 paused 结果，两个调用返回成功但只结算/交付一次。
8. `bootstrap_creation_and_start_are_atomic`：assignment 或 run 的启动被 SQLite IGNORE 后，新建的 assignment/run/lane/capability/预算记录全部回滚。
9. `changed_source_evidence_does_not_resume_or_recall`：源证据改变后不能复用原结果，也不发第二次模型请求；保留暂停待处理。

这些测试使用本地 loopback，不访问用户提供的公网 URL。

## 验证

- `cargo test --offline specialist_recovery -- --test-threads=1`：新增 9 项通过。
- `cargo test --offline --all-targets --all-features -- --test-threads=1`：主库 **757**、历史导入 **30** 项通过。
- `cargo clippy --offline --all-targets --all-features -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `npm run test:agent-dialog`：**38** 项通过。
- `npm run build`：通过；主 JS **777.36 kB**，分包性能警告仍在。
- `node tools/test_native_runtime.cjs`：通过；本地浏览器 **8** 次请求、匿名与身份对照完成、身份隔离通过。
- 日志前缀：`/tmp/oviraptor-20260926-readonly-recovery-`，包括 `new`、`full`、`clippy`、`fmt`、`ui`、`build`、`native`。以上不是发布包、真实 Tauri IPC/视觉验收或用户授权目标实测证明。

## 未完成事项与后续顺序

### 2026-09-28 补充：Coordinator fencing 换代边界

生产入口 `multi_agent_prepare` 在重取 Coordinator lease 后、改写 root 与预算账本 fencing 前，调用 `scheduler::ensure_fresh_readonly_fence`。如果同一扫描、attempt、目标的 Mapper、Identity 或源码只读 assignment 属于旧 root、epoch 或 token，本 attempt 明确返回 `readonly_fencing_changed_requires_fresh_attempt`。这不是对旧结果的丢弃或重新调度：旧回执、费用预留、消息和 assignment 保留供核对；新 attempt 有独立合同。无论旧 Mapper 已完成还是模型调用结果未知，都不得借新 Coordinator 继承旧 capability、把旧预算换绑成新预算或静默重发模型。

新增 `specialist_recovery_rotated_fence_requires_new_attempt_before_budget_rebinding` 从真实 `multi_agent_prepare` 分别构造完成 Mapper／暂停 Identity 与不确定 Mapper 两种情况，换代后断言明确错误、模型请求数不增加、旧账本 fence/预留/已用计数及 assignment 数不改变。`specialist_recovery_replacement_root_cannot_redo_completed_mapper_in_same_attempt` 证明新的 root 接管同一 attempt 不得再调用旧 Mapper 的模型或新建预算账本；原实现会重发，修复后先拒绝。专项 `specialist_recovery_` **11 项通过**；前一版 `cargo fmt --all -- --check`、严格 `cargo clippy --offline -j 1 --all-targets -- -D warnings` 和 `git diff --check` 通过，新 root 修复后的门禁须重跑。此前完整 Rust 进程在本增量编译后被外部中断，未取得 exit 0，不能当作全量验收。

2026-09-28 续核：初版跨 fence 护栏错误地把 `human_directive:*` 只读提案也当作 Mapper 初始化任务，使已废弃提案进入“未知结果、不重试”清理时拦住无关 Mapper。`ensure_fresh_readonly_fence` 现只针对非人工指令提案；提案仍由其专用回执和预算保留路径对账。真实目标重入回归 `directive_reentry_real_target_does_not_retry_unknown_proposal_or_block_mapper` 重新通过，指令专项 **104**、只读恢复专项 **11** 项通过，严格 Clippy、fmt、`git diff --check` 通过。修复前二进制的全量测试已观察到该失败，随后在约 950／1362 处终止以避免继续空耗 CPU，不能引用为通过证明。修复后的完整 `CARGO_BUILD_JOBS=1 cargo test --offline -j 1 --all-targets --all-features --quiet -- --test-threads=1` **1362／1362 主库、30／30 集成测试、exit 0**；用时约 23 分钟，运行时保持单线程和单编译任务。这仅是 Rust 门禁证明，不涵盖授权 URL 实测、Tauri IPC/视觉验收或 Master Plan 未交付项。

此边界仍**不实现**跨 fencing 的已收取结果安全迁移或终止后统一对账 UI；若将来需要在同 attempt 中复用旧完成结果，必须另外证明冻结请求一致、旧回执和 checkpoint 完整、原费用只结算一次，并提供不授旧权限的显式事务交接。不能通过放宽 assignment/模型回执的 fencing 相等校验实现。

1. 外层活调用竞争入口保护已由同日后续审计交付；重复调用观察 UI、历史 pending/tally 收敛及崩溃恢复仍待完成，见 `NEST_INVOCATION_OWNERSHIP_AUDIT_2026-09-26.md`。
2. root 已终止、lease 已过期或换代后的通用 specialist 只读对账入口与 UI；本增量不能替代此前人工指令专用对账 API。
3. Coordinator 仍在运行但用户需要显式恢复的统一 API/聊天动作；直接重入 prepare 的测试不是完整 UI 续跑验收。
4. Reviewer/Investigator 的通用业务恢复，以及 gap → 新合同 → 实际采集 → 新证据 revision → 独立再审闭环。业务交付原子性已在后续 `NEST_REVIEW_DELIVERY_ATOMICITY_AUDIT_2026-09-26.md` 中补齐；活跃根任务下 Reviewer received 的有限生产补交后续见 `NEST_REVIEW_RECEIPT_RECOVERY_AUDIT_2026-09-26.md`。不能据此消除 Investigator、终止后通用恢复和补证缺口。
5. 其余专家执行链、沙箱/工具供应、前端分包及视觉验收、部署与授权 URL 实测。

Master Plan 保持未完成。本轮未部署、未访问外部目标、未新增主机执行器、未改变 `web_only`，也未修改 Strix retirement allowlist。

### 2026-09-28 续核：历史人工建议回执的可见性

此前协同台只展示当前 attempt 的时间线。旧 attempt 的只读人工建议若已保存模型响应、处于 `receipt_pending`，在新 attempt 开始后会从当前聊天时间线消失。现 `native_scan_status_after` 同一读取快照内额外统计早期 attempt 中 `deferred`／`directive_task_ended_receipt_pending` 且 proposal 仍为 `received`、无结果消息的记录，返回 `historicalPendingReceipts`；聊天台只显示“旧尝试待核对记录”的数量、未核验警告及查看证据入口。它不投射旧响应为可信结论、不自动重试模型，也不借新 attempt 的 lease/fencing/预算结算旧账。旧 attempt 的专用 `reconcile_received` 仍拒绝跨 attempt 请求。

新增 `old_attempt_saved_receipt_remains_visible_as_read_only_debt` 先红（缺字段）、后绿，断言新 attempt 仍显示历史待核对计数、当前时间线不混入旧卡片、尝试旧对账无数据库写入。真实 Vue SFC 测试覆盖提示文案及不出现当前 attempt 补齐按钮。本增量门禁：该回归、`directive_reconciliation` 专项 **9/9**、`npm run test:agent-dialog` **100/100**、`npm run test:ui` **249/249**、`npm run build`、严格 all-targets/all-features Clippy、fmt、`git diff --check` 均通过。它只修复“看不见待核对债务”，**未**实现旧 attempt 的冻结来源校验、结算 API、专用操作界面或通用 specialist/Reviewer/Investigator 终态对账；Master Plan 仍未完成。

### 2026-09-28 续核：启动拒绝的终态分类

外层 `run_agent_target` 在 `multi_agent_prepare` 返回“必须新建 attempt”的拒绝后原先统一构造 `multi_agent_bootstrap_failed`。虽然底层没有重发请求，这会把换代／旧目标派发／旧外部捕获的不兼容恢复误报成普通启动失败，使目标计入失败而非人工重新执行。现只对三个明确的 Broker 错误码 `readonly_fencing_changed_requires_fresh_attempt`、`target_execution_recovery_requires_fresh_attempt`、`public_surface_recovery_requires_fresh_attempt` 构造 `resume_incompatible`；未知错误仍为普通失败，WAF／429 保护仍为熔断限制。原入口继续用已取得的 Coordinator lease 收口 root，不复用旧 assignment、模型回执、能力或预算。

新增 `bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure`：先观察到分类断言失败，再修复并验证三种拒绝的终态、目标状态、人工重新执行 tally 与文案，未知错误不误归类。`cargo fmt --all -- --check`、`CARGO_BUILD_JOBS=1 cargo clippy --all-targets -j 1 -- -D warnings` 通过；这三个文件目前均未跟踪，分别用 `git diff --no-index --check /dev/null <file>` 检查，输出无空白告警（因文件有差异，退出码 1）。`CARGO_BUILD_JOBS=1 cargo test -j 1 -- --test-threads=1` 主库 **1363/1363**、exit 0，另 `cargo test -j 1 --bin import-existing-results --features import-tools -- --test-threads=1` **30/30**、exit 0。全量主库单线程约 24 分钟。此分类修复不等于通用跨 fencing 收据迁移、终态对账 UI、安装包或授权环境验收，Master Plan 继续保持未完成。
