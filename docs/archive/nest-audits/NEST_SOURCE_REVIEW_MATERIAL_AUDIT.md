# Source Reviewer 材料绑定审计（2026-09-27）

## 当前状态更新：真实 v2 执行已接通

新建 v2 Source Coordinator 已执行独立 Reviewer 的真实 assignment/child/model receipt/mailbox ACK，并验证完整与增量聊天可见性；旧 v1 仍冻结为原 8 次预算，不自动升级。当前仅候选审查完成，正式不可变源码决策、Finding/CI 消费、整体覆盖和顶层 crash recovery 仍未完成。最新实现、真实失败、分阶段验证与后续验收要求见 `NEST_SOURCE_REVIEW_EXECUTION_AUDIT.md`；下文“没有新增模型调用”等仅描述材料准备增量的历史边界。

## 材料准备阶段边界（历史）

已接入生产源码根收口的审查材料准备，不是独立审查执行完成。生产结果仍为 `source_analysis_completed_unreviewed`，`independentReviewCompleted=false`，CI 继续显式保留未审查缺口。没有新增模型调用，没有放宽 v1 根计划的 8 次模型调用上限，没有添加 Web、主机或目标工具授权。

## 真实接线

1. `agent_runtime/multi_agent/source_rounds.rs::audit_completion` 返回带私有工具证据字段的 `SourceRoundAudit`。仅当真实逐轮模型/工具回执、请求、响应和事件校验通过后，收集成功的非 finish 工具结果；拒绝和 finish 不冒充验证事实。
2. `commands/native_source_completion.rs::source_assessment_completion` 仍证明四个阶段 assignment、child 终态、mailbox ACK 和费用账本，随后在同一事务调用 `source_review::capture`。
3. Analyzer 候选来自本 attempt 已接受且可复核的不可变结果回执，保留所有引擎来源。身份为 `analyzer_revision + logicalKey + revisionHash + revisionId`；不把导入哈希转换成图的数字修订。
4. 图候选来自当前 root 的有效修订快照。每个实际提交必须仍能找到；每个图候选必须匹配真实 SourceAnalyst 作者、完整 source_tools assignment、工具提交回执、冻结文件/hash、位置、内容、自然键和图 ID。v1 不接受无回执的 supersession。
5. 工具证据包含真实 root、作者 run、assignment、轮次和调用索引。候选绑定这些证据的 SHA-256 摘要；材料包含 scan/attempt、view/results/plan 摘要，确定性排序、脱敏后计算摘要。
6. 根 terminal event 保存 `reviewMaterial`。终态写入后，在同一事务重新证明整个闭环并比较完整 proof；触发器删除/改写候选、作者、材料等会使收口回滚，不重发模型调用。

## 本轮真实失败与修复

低负载集成 session 94886 exit 101：新增自然键复核错误地把 `contentHash` 加入候选身份数组，和现有 SourceBroker 不一致，合法候选收口返回 `source_review_material_candidate_binding_invalid`。这是实际集成回归，不是环境噪声。

修复将 v1 身份计算提取为 `native_pipeline/tools.rs::source_candidate_identity`，由提交与回执复核共用。保持旧数组字段/顺序、title/rationale trim 和可选字段语义，**没有迁移或改写已有图 ID/JSON**；文件内容 hash 仍独立验证，不偷偷升级身份算法。

新增生产执行回归独立构造旧身份数组，验证实际落库 ID/自然键；覆盖空白裁剪、缺省和显式可选参数。该测试不调用共享 helper 计算期望值，避免生产和验证同时改错仍然通过。

## 已收取的验证结果

- session 4250 exit 0：`source_review_material` 四项测试通过。包括真实模型/工具阶段、作者/修订/自然键/来源/树/回执/事件篡改拒绝，以及终态触发器破坏回滚。日志 `/tmp/oviraptor-source-review-identity-low-load.log`。
- 实际工程 `http_endpoint::tests` 五项通过，exit 0；日志 `/tmp/oviraptor-http-endpoint-integration.log`。不再仅依赖独立 rustc 的夹具验证。
- session 19381 exit 0：`cargo clippy -j 1 --all-targets --all-features -- -D warnings`；先前 `clone_on_copy` 修正已得到验证。日志 `/tmp/oviraptor-review-material-clippy-low-load.log`。
- `cargo fmt --all -- --check` 和 `git diff --check` 通过。
- 更大范围源码回归 session 96167 已 exit 0：主库 172 项、历史导入器 2 项通过，日志 `/tmp/oviraptor-source-regression-low-load.log`。
- 完整 Rust 回归 session 41293 已收取 exit 0，单作业、单测试线程、nice 15：主库 1162 项、历史导入器 30 项通过，日志 `/tmp/oviraptor-full-after-fixture-lifecycle.log`。进程已结束；该二进制不含运行期间新增的 `source_review_contract`，不能把此结果写成最新合同代码的完整回归。

以上不是最新完整 Rust/UI/部署验收，也不证明 Master Plan 已完成。所有模型端点均为 localhost 测试夹具；未访问用户外部目标。

## 独立 Reviewer 仍需真正交付

### 后续输出合同增量（不是 Reviewer 执行交付）

新增 `source_review_contract.rs` 从实际审查材料构造逐候选的身份/摘要/证据引用合同；生产快照增加 `decisionContract`。严格响应解析拒绝多余或重复 JSON 字段、遗漏/重复/外来候选、过期摘要、伪造引用和已知作者自审。此解析器没有模型执行、租约、回执、写库或 CI 授权，不能仅凭一个不同的 reviewer 字符串证明独立审查。v1 输出仍为未审查。

低负载定向首轮 session 22710 exit 101（1 通过、5 失败），实际暴露了新增合同把导入器的 `sha256:<hex>` 修订身份误当成裸摘要校验的兼容错误。已修正为保留并校验既有命名空间，另修正新增测试的 Rust 借用冲突；没有修改历史 JSON/导入器身份算法。首轮日志 `/tmp/oviraptor-source-review-contract-low-load.log`。复测 session 17752 已 exit 0，`source_review_` 六项全部通过，日志 `/tmp/oviraptor-source-review-contract-low-load-rerun.log`。随后 session 72427 的全目标/全特性严格 Clippy（`-D warnings`）exit 0，日志 `/tmp/oviraptor-source-review-contract-clippy.log`；fmt 与 diff 空白检查通过。最新合同仍只有定向运行验证，不能由前序 1162 项全量代替最新完整回归。

### 尚缺的执行链

1. 新根的冻结版本显式包含审查阶段和预算，保留旧 v1 root 原合同，不静默增加第九次调用。
2. 由已证明材料构造只读 review assignment/不同作者 child-run，绑定精确摘要和带命名空间的候选修订。无候选不创建假候选 Reviewer；总体复核必须用单独的 CoverageClosureReview subject。
3. 源码 Reviewer 使用源码发布 runtime、输出上限、期限、取消和租约续期，不能因为角色名相同落入 Web transport 授权分支。
4. 严格逐候选输出合同：缺失、重复、外来或过期身份必须拒绝；理由、证据/反证引用、缺口、confidence 和严重度都需校验。材料不足只能明确不足，不能把模型返回一个 `confirmed` 字样当作证据证明。
5. 真实模型回执后才允许不可变 canonical decision；原子持久化决策、mailbox/ACK、费用结算和 child/root 收口；unknown 不能自动重发。
6. 当前材料/精确修订/独立作者/回执仍有效时，CI 和 Projector 才能消费决策；同一事实的图修订与导入修订不能混用。聊天/时间线读取真实事件，不伪造发言或执行状态。

这些要求是剩余交付，不是本增量已完成事项；整体目标继续有效。
