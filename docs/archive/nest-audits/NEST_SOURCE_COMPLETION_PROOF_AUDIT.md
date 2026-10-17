# 源码多智能体根收口证明审计（2026-09-27）

状态：生产收口已改为同事务证据核验；本增量最终全量回归已通过（session 58086 exit 0）。不是独立 Source Reviewer 或 Master Plan 的完成声明。随后源码轨迹展示修复单独记录于 `NEST_SOURCE_TRACE_PROJECTION_AUDIT.md`，不能借用本次结果代替其验收。

## 1. 修复的问题

原 `source_assessment_completion` 只检查四个 completed/settled assignment、两个工具阶段、零未决预算及撤销后的 capabilities。角色重复、早期工具回执缺失、邮箱内容与模型响应不符等问题，不能仅靠计数排除。证明读取与 `finish_coordinator_run` 还在不同事务中。

新生产路径使用 `native_source_completion.rs` 的 `finish_native_source_coordinator`。保留 `sourceToolsPhaseVersion=1` 和八次模型请求上限，不升级旧任务、不借用 Web 请求额度、不增加主机操作。

## 2. 收口合同

1. 同一 IMMEDIATE 事务内验证活跃 attempt、Coordinator lease、当前模型配置和发布的 source runtime，恢复真实冻结源码任务。
2. 精确要求 RepoMapper / SourceAnalyst 各有一份初评和一份 source_tools assignment。逐项验证 task slice、revision、root/child/parent/assignment、目标、角色、lane、epoch/fence、完成时间、结算状态及取消状态。
3. 初评核对真实 specialist receipt、响应摘要、usage、模型事件和 checkpoint。
4. 工具阶段逐轮核对 journal、绑定/hash、完整 transcript 链、授予工具集合、全部工具回执/事件、末轮独占 finish、累计 usage 和 checkpoint。不能只验证末轮 finish。
5. 每份结果必须具有唯一、正确路由、已交付且已 ACK 一次的结果消息，payload 与回执推导内容一致。其他非结果类控制消息不参与四份结果计数，不应因正常聊天而阻断收口。
6. 子任务持久用量必须等于回执用量；根预算已结算、无残余预留/可执行 capability；费用总和、模型请求数、冻结预算和实际子任务数一致。
7. 写入根任务带缺口终态、关闭相关人工指令、追加真实 `terminal_reduced` 收口事件，然后在同一事务再次核验完整证明、根终态、冻结计划、runtime 与源码材料。任一失败回滚本次根收口，不回滚此前已完成且真实发生的模型调用与其费用。

共享的 `finish_coordinator_run_in_transaction` 仅提取原终态写入逻辑，原 Web 调用仍使用原事务包装入口。此次不修改 Web Reviewer 的资格合同。

## 3. 计数与结果真实性

`verifiedToolResults` 来自逐轮校验通过且非拒绝的实际数据工具回执；不计 `assignment.finish` 控制标记，不计 Broker denial，也不等于确认漏洞数。

真实成功用例：六次 localhost 模型请求、四个 assignment、四封结果 ACK、120 tokens、两个成功 inventory 结果。四条工具回执中另外两条是 finish，因此数据工具计数为 2，不是 4。

不把子任务累计用量再写入根任务 `used_tokens/used_requests`。根 Coordinator 自身没有调用模型，现有 Native trace 会累加所有 run；复制累计值会造成双倍计费。汇总保存在原根预算账本、报告字段和收口事件中，测试同时核对 trace 仍为 120 tokens / 6 次请求。

仍然返回 `source_analysis_completed_unreviewed`，`independentReviewCompleted=false`，保留 `source_review_not_completed`。不发布确认 finding，不让 CI 因本增量获得复核通过资格。

## 4. 验证记录

- 前序 session 61232 已终态 exit 0：fmt、严格 Clippy（11.04 秒）、全目标全特性 Rust 主库 **1146** 项（599.87 秒）及历史导入器 **30** 项通过。这是本次收口修改之前的基线。
- session 4109 exit 101：第一版真实入口回归发现后置 runtime 核验调用了自行开启事务的包装函数，嵌套事务导致 `source_runtime_verification_unavailable`。已改为在现有事务中使用 `verify_source_runtime_in`，没有削弱核验。
- session 98564 exit 0：源码派发相关 **9** 项通过。其中成功入口含 **16** 种持久状态情况（正常及 15 类破坏），新增收口故障测试当时含 **9** 类终态写入故障。
- session 82332 已收取 exit 0：严格 Clippy 通过（17.87 秒）。本结果不覆盖后续增加的两类故障断言。
- 最终 session 58086 已收取 exit 0：fmt、严格 Clippy（8.63 秒）、全目标全特性 Rust 主库 **1147** 项（622.07 秒）、历史导入器 **30** 项（2.69 秒）通过。此次输入包含 root 预算/材料绑定故障，以及避免 trace 重复计费的断言。本次编译完成后开始的轨迹展示修复不在此结果范围内。
- session 10887 已收取 exit 0：七组 UI **147** 项、前端构建、本地浏览器回环（8 请求）通过；主 JS 830.15 kB 拆包警告保留，不代表安装包、IPC 或外部目标验收。

最终通过的故障矩阵涵盖：忽略终态写入、邮箱 ACK/内容损坏、预算漂移、child 用量变化、root 取消、attempt 暂停、模型事件缺失、runtime 配置变化、根预算同时漂移和根材料绑定变化。

日志：`/tmp/oviraptor-source-closure-{production,dispatch,clippy,clippy-final,fmt-final,full-final,ui,build,browser}.log`。临时日志可能被清理，本文件不得把未收取的结果写成通过。

## 5. 不得视为已完成的事项

- 独立 Source Reviewer：实际 assignment、模型响应、独立作者核验、候选 revision 绑定、不可变决策、mailbox、结果发布和 CI 资格仍未接入生产。
- 源码完整顶层恢复、未知调用核对与 paused 阶段原子恢复；此次仅核验已有持久结果，不复活或重发任务。
- 美元费用账本、整 attempt 跨 analyzer/model/review 的统一持久期限。
- Master Plan 中的人工引导、角色质询、聊天端到端、隔离沙箱/工具供应、知识治理、资产汇总、UI 和发布验收。

后续接入 Reviewer 必须版本化冻结计划、模型请求预算及阶段证明集合，不能把当前四份阶段证明直接解释为已经独立审查。整体目标保持未完成。
