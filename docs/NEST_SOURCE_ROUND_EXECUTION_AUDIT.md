# 源码工具多轮执行与持久回执审计（2026-09-27）

状态：生产工具阶段已接入；独立 Source Reviewer、完整恢复和 Master Plan 尚未完成。本记录接续 `NEST_SOURCE_TOOL_AUTHORITY_AUDIT.md`，不把初评或工具分析当作独立复核。

## 1. 已接入的生产路径

`run_native_source_scan` → 源码 Coordinator → 两个无工具初评 → RepoMapper / SourceAnalyst 各自独立的 `source_tools` assignment → 多轮模型/本地工具 → 持久 `assignment.finish` 回执 → mailbox 消费/ack → 用量结算与 child 收口 → root 带缺口收口。

- 工具阶段使用独立 `SpecialistTransportContext`，不构造假 Web plan，不授予目标请求或主机权限。
- 冻结计划新增 `sourceToolsPhaseVersion=1`，历史计划不原地升级。
- 当前冻结模型请求预算仍是 8：两次初评 + 两个角色各最多三轮工具调用。新增 Reviewer 必须另行版本化预算合同，不得偷用 Web 请求额度。
- 当前根结果为 `source_analysis_completed_unreviewed`，保留 `independentReviewCompleted=false` 和 `source_review_not_completed`；不得报 CI 已复核通过。
- 多智能体阶段失败的报告缺口改为 `source_multi_agent_execution_incomplete`，不再把工具阶段失败误报成仅初评失败。

## 2. Journal 与不重放合同

`agent_source_model_rounds` 逐 assignment / round 保存请求、绑定、预留、响应、usage、事件序号和 hash；状态为 `executing / received / uncertain`。

`agent_source_tool_receipts` 逐 round / call index 保存工具调用 ID、参数、输出和事件证明；状态为 `planned / completed`。

1. 在 IMMEDIATE 事务中领取模型调用，网络 IO 在事务外执行。
2. `executing / uncertain` 不自动重发；未知费用不退款。
3. 后续 transcript 只能来自已持久化的上一轮 assistant 和工具回执。
4. 逐轮校验真实剩余额度、usage 合法性、工具集合、fence、lane、当前源码材料、checkpoint 和事件完整性。
5. 拒绝重复工具 ID、同动作不同 ID、未授权工具、敏感或过大参数、混合 finish 调用；超预算响应保存计费证据，但不能执行工具。
6. 迟到模型响应可以保存 usage，但不会恢复已撤销的执行权。
7. 已有工具回执重放只读取记录，不重复调用 Broker。
8. 普通文本回答不是完成证明；只有已完成的独占 `assignment.finish` 回执可用于结算。

Journal 恢复测试只证明同一有效 assignment 的持久回执可重读，不证明顶层 paused/终态任务能够自动恢复执行。

## 3. 本次真实回归发现与修复

### 3.1 工具阶段被误判为用户取消

生产工具阶段曾复用初评的 `specialist_model_cancel_token`，后者要求 `evidence.read`。工具 assignment 使用精确的源码工具集合，没有该初评权限，因此合法调用在派发时被取消。

修复：工具阶段改为 `source_tool_model_cancel_token`，调用工具阶段自己的完整授权验证，保留当前 attempt、root/child 状态、全部工具能力、lane、runtime、fence 和时限检查。没有给工具阶段补一个无关的初评权限来绕过检查。

### 3.2 Broker 拒绝之前的部分数据库写入

SourceBroker 可能先写候选，再因末尾的范围校验失败而返回 denial。直接将 denial 转成成功返回的 JSON 会让外层 journal 事务提交部分副作用。

修复：每个本地 Broker 调用使用同一数据库事务内的 SAVEPOINT。发生 denial 时先回滚该调用的全部数据库写入，再持久化拒绝回执；外层仍在写回回执后复核授权与 journal。SAVEPOINT 不用于网络、shell 或文件修改。

新增故障注入测试：仅在测试夹具中移除视图 UPDATE 的不可变保护，再让候选插入触发视图路径损坏，使 Broker 最终验证失败；候选和修订均不残留，视图恢复，denial 回执可重放而不再次执行。生产不可变保护没有放宽。

### 3.3 生产负面路径

新增真实 localhost 生产入口回归：工具模型 HTTP 500、只有文本而没有 finish、finish 被拒绝、邮箱写入失败、预算结算失败。要求失败 assignment 暂停，预留保留，无假完成消息，无模型自动重试；已收到的 usage/工具回执保持可核对。

另覆盖工具取消检查中的正常运行、child/root 取消、assignment 暂停、某项工具能力撤销、租约到期、lane 丢失、模型配置变化和截止时间。

### 3.4 工具阶段自己的租约续期

新增工具阶段 heartbeat，不放宽旧初评续期规则。仅在全部原权限仍然有效、临近到期且最后一轮 journal 仍为 executing 时续期 root、assignment、capabilities 和两个 run；复核更新行数和持久后置条件。过期、撤权、未知结果、runtime/期限变化和静默忽略写入均不能续期。续期不改变 fence、预留、用量或模型调用次数，也不延长冻结总时限。

取消探针在同一个事务中检查并续期，成功提交后才报告可继续。新增 11 场景回归已通过，包括各类租约过期、缺少活跃调用、四种忽略更新和续期写入期间撤权。

## 4. 验证记录

- session 12863 exit 101：初版测试 callback 错误类型不符，随后修复。
- session 23723 exit 0：当时 5 项 journal 测试通过，不覆盖后续生产接线。
- session 28478 exit 101：严格 Clippy 发现 journal 无生产调用；通过接入真实生产工具阶段解决，没有关闭 lint。
- session 52933 exit 0：接线后的严格 Clippy 通过，不覆盖之后的测试与修复。
- session 23936 exit 101：源码相关 156 通过 / 1 失败；真实入口暴露上述取消检查错误。
- session 69562 exit 0：修复后的真实生产入口测试通过；6 次 localhost 模型请求、4 个完成且已结算的 assignment、4 封 ack 消息、120 tokens / 6 model requests、零剩余预留。仍未独立复核。
- session 66409 exit 101：源码相关 158 通过 / 2 失败。其一暴露工具取消检查未核对完整能力集合，已补精确比对；其二是测试故障先被视图不可变保护拦截，已修正夹具以真正覆盖写后校验失败。
- session 23454 exit 0：上述修复后的工具派发负面路径 2 项、SAVEPOINT 回滚 1 项通过，不覆盖后续 heartbeat 增量。
- session 1247 exit 0：工具阶段 heartbeat 与已有负面路径共 3 项通过（10.37 秒），其中 heartbeat 测试覆盖 11 场景。
- session 23566 exit 0：七组实际 UI 组件 147/147、前端构建、localhost 浏览器回环通过。浏览器观测 8 请求、身份隔离成立；主 JS 830.15 kB 拆包警告仍存在。没有变更前端源码，不是完整安装包/IPC 验收。
- session 61232 exit 0：fmt、最终严格 Clippy（11.04 秒）、全目标全特性 Rust 主库 **1146** 项（599.87 秒）及历史导入器 **30** 项通过；另行 `git diff --check` 通过。这是多轮执行增量的最终结果，不覆盖之后的根收口证明修改。

日志位于 `/tmp/oviraptor-source-rounds-*`，生产修复定向日志为 `/tmp/oviraptor-source-tools-production-fixed.log`。临时日志可能被系统清理，计数和状态以本审计及对应运行输出为依据。

## 5. 不得遗漏的后续工作

1. 独立 Source Reviewer 的真实 assignment、模型回执、候选裁决、mailbox 交付及 CI 证据资格。必须保留作者与审查者独立，不能只删除未审查 gap。
2. 工具阶段专用续期已接线，定向和本增量全量回归通过；不能把它解释成整个 attempt 统一时限或顶层恢复。
3. 顶层阶段恢复：已完成阶段复用、paused 的原子本地结案、未知调用人工核对；禁止通过重新调度分配第二份预算。
4. 后续根收口已增加逐角色/阶段唯一性、完整回执复核和同事务终态写后验证，工具计数来自真实数据工具而非 finish 标记；见 `NEST_SOURCE_COMPLETION_PROOF_AUDIT.md`。该后续修改有独立验证记录，不能借用本文件 1146/30 的基线结果。
5. 配置化美元费用账本。显式美元上限继续拒绝未计价的源码模型派发，不假设免费。
6. Analyzer → 模型 → Reviewer 整个 attempt 共用的持久总时限。当前模型/工具期限不涵盖 root 创建前的分析器 `ProcessLimits::default()`。
7. Master Plan 其余人工指令、角色质询、真实聊天、沙箱/工具供应、知识治理、资产汇总、UI 与发布验收仍保留原范围。

本轮不启用 Host Agent、不部署、不访问外部目标，不改变历史 JSON 的授权含义。
