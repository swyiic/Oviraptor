# 源码专家调度合同与未复核 CI 审计

状态：源码只读专家的真实调度／模型回执／交付底座已实现，并通过 localhost 模型接口测试；**生产源码启动器尚未自动编排这些专家，源码独立 Reviewer 和可通过的 CI 复核资格尚未交付**。这不是整个 Master Plan 完成声明。

## 1. 本次实际修改

### 1.1 源码专家身份，不继承 Web 执行权

- 增加 `repo_mapper`、`source_analyst` 两种持久角色，接入现有 scheduler 和 specialist 调用日志。
- 两种角色仅允许 `read_only_analysis` 通道；允许能力为 `evidence.read`、`mailbox.read`、`mailbox.write`。不能取得目标通道、独立复核通道、shell、主机能力或 `review.write`。
- 当前阶段仅做冻结元数据和分析器候选的无工具初评。`SourceAnalyst` 这个角色名不代表已经拥有源码工具或深度代码审计能力，初评输入明确记录限制。

### 1.2 真实本轮材料与根任务合同

`agent_runtime/multi_agent/source.rs` 负责恢复并核验：

1. 当前 scan 和 attempt 均处于 scanning，未删除；源码范围合同仍对应该 scan 的类型和路径，canonical root 未被替换。
2. 已保存的来源快照、选中文件分析视图、不可变分析结果及其中的实际 revision/envelope；不读取 scan 级 current membership，不补造缺失回执。
3. 专用目标键为 `source:<analysis manifest digest>`，不能拿 Web URL 对应的协调器使用此合同。
4. 根任务必须是真实 Native/Multi Coordinator、独立根、无 assignment、未取消且可执行；其 plan hash 对应本轮 NativeSourcePlan，evidence hash 对应本轮 AnalysisResults。源码计划的 scan/attempt/type/tree/commit/base/diff 状态逐项核验。
5. assignment 的 task slice 固定绑定 root/scan/attempt/source/analysis/results/plan，以及具体候选接受凭据。固定初评 revision=1 是该只读任务的版本，不是从证据图 `MAX(revision)` 猜测候选修订。

调度、模型派发、模型结果记录、交付及重放都复核这些绑定。子任务取消、根任务取消、源码范围改变或材料完整性破坏后，不再派发模型。

### 1.3 真实模型与消息闭环

- 模型输入由实际选中文件元数据、分析器运行回执、候选 envelope 及覆盖缺口生成；输入中的候选被替换或遗漏，或者请求携带工具，均在模型派发前拒绝。
- 复用现有 `agent_specialist_calls` 的 at-most-once 派发、response/usage/event/checkpoint 回执；无法确认执行效果时，不自动重放。
- 源码初评交付前，必须存在可验证的 received 回执，且 summary、usage、source task 与其一致。不能只写一条“finished”消息冒充模型执行。
- assignment 完成、child 终态、预算结算、能力回收、lane 释放、mailbox 写入及接收/确认沿用现有原子交付路径。重放不再发请求、预留预算或重复发消息。
- 这些底层真实事件可以供后续团队时间线读取；**尚未完成源码启动器和 UI 的端到端接线，不以测试消息宣称生产聊天已上线**。

### 1.4 CI 不再把未复核扫描写成通过

此前 `native_source_scan.rs` 将 scan ID 传给要求 root-run ID 的 `ci::evaluate`；非空、无 gap 且零决定可以变成 Passed。

本次生产源码入口改用显式 `evaluate_unreviewed`：

- 不查无资格的历史决定，不传 scan ID 冒充 root。
- 分析完成后保留 `source_review_not_completed`，正常非空扫描为 `coverage_incomplete`，没有可计算的 confirmed/blocking 发现。
- 基础设施失败、显式 Diff 无可用范围、空集合等已有状态优先级保留；关闭“风险超阈值阻断”也不能把未复核变成通过。
- 无资格的 root-wide 决定读取器 `evaluate` 仅编译进阈值单元测试，生产代码不能调用它证明源码已复核。
- 这是报告真实性修正，不是在固定执行步数中止分析器；分析器原有执行流程继续完成。未来必须完成真实独立审查并提供精确本轮资格，才能解除这一缺口，不能直接删除 gap 来获得绿色 CI。

## 2. 验证证据

新增 7 项测试：

1. 两角色真实 assignment/child、response journal、完成结算及 acknowledged mailbox，幂等重放后总花费准确。
2. 目标/复核 lane、shell/review 能力、错误 revision 或修改任务合同均拒绝，不残留 assignment。
3. 无 response journal 的假完成、篡改 summary 或 source task 的完成被拒绝。
4. stop、换 attempt、源码路径、canonical envelope、选中文件、目标键、根计划/结果绑定、根/子取消和 single policy 都阻止新派发。
5. 通过真实生产 HTTP transport 调用 localhost 模型：两个角色各一次请求、各一次模型事件和已确认消息；不匹配输入不发网络请求，已完成结果重放不再次调用模型。
6. 生产分析入口保留未复核 gap，CI 不报 passed。
7. 单独构造完整且非空的 CI freeze，开启或关闭风险阻断都不能绕过未复核状态。

定向最终 session 54755 退出 0：7 passed；严格 Clippy 通过。

- `/tmp/oviraptor-source-specialists-focused-v4.log`
- `/tmp/oviraptor-source-specialists-clippy-v4.log`

首轮测试夹具失败记录保留：错误使用 ledger 的 `used_tokens` 列（实际为 `spent_tokens`）；篡改 envelope 被追加式触发器拒绝；直接覆盖只读视图文件被文件权限拒绝。后两者在隔离测试数据中显式模拟损坏后验证二次完整性检查，未放宽生产触发器或文件权限。

完整串行验证 session 6329 已退出 101：主库 1090 passed／1 failed，唯一失败是 REM-012 对 `contract.rs` 与 `tests_native_ci.rs` 的全文审核摘要未更新；后续 Clippy/fmt 未执行。人工复核后只更新两个审核条目的摘要与理由，4 项退役守卫已通过。后续传输解耦、子任务取消和完整 session 41545 终态统一见 `NEST_SPECIALIST_TRANSPORT_CANCELLATION_AUDIT.md`，不能将上一次 1084／30 当作本增量完整门禁。

## 3. 下一步必须实施，不能替换成“已完成”

1. 从生产 source branch 创建并冻结真实 Source Coordinator，将上述两个只读角色接入真实模型配置、预算、恢复和终态。后续已抽出与 Web 共用的最小模型 transport，源码 localhost 测试不再借用 Web execution plan；生产源码 Coordinator 编排仍未接入。
2. 为源码文件读取、候选构造与 `assignment.finish` 实现明确的角色/逐工具合同。现有 `agent_tools_source.rs` 的 Web 授权、`MAX(revision)` 和 SourceBroker 内存 finish 尚未替换。
3. 创建独立 source Reviewer assignment；冻结具体 candidate/revision/证据材料，复用真实 response journal，并原子提交 review decision、acknowledged mailbox 与预算。不能用根角色自审或测试假 Reviewer。
4. 生产 CI 只接收该本轮候选集合的有效 review delivery；处理缺失、冲突、重放、过期决定和零候选的完整审查回执。之后才有资格从未复核状态进入 Passed/Warning/Blocked。
5. 将生产源码专家及复核消息接入现有任务聊天和详情页，验证用户指令、不确定效果、停止恢复及最终归档。
6. 保留主计划其余未完范围：多引擎合并来源、灰盒/源码完整派发冻结、工具供应/沙箱、学习治理/资产、安装包与授权环境验收。

本轮未部署、未访问用户提供的公网 URL、未新增主机执行能力。主机审批边界仍按独立决策文档执行。
