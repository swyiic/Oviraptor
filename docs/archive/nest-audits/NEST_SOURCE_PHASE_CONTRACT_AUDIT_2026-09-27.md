# 源码阶段权限与预算解耦审计

日期：2026-09-27。状态：总体覆盖 Reviewer 的实现前置增量，**不是总体覆盖执行链已完成**。

## 1. 本次实现

原实现中，源码角色准入、候选 Reviewer 开关和结案必需审查条件通过 `model_request_limit == 9` 推断阶段权限；裁决发布与结案投影分别通过 schema 3 推断发布能力。这些条件对现有版本成立，但新增覆盖阶段时容易产生分散修改和错误继承。

新增 `agent_runtime/multi_agent/source_plan_contract.rs::SourcePhaseContract`，统一解码被冻结的阶段版本与精确请求预算。生产注册、角色准入、候选审查启用、裁决发布、根结案和预算读取共用该合同。

| 冻结版本 | 请求上限 | 候选 Reviewer | 候选裁决发布 | 总体覆盖 Reviewer |
| --- | --- | --- | --- | --- |
| v1 | 8 | 否 | 否 | 未实现 |
| v2 | 9 | 是 | 否 | 未实现 |
| v3 | 9 | 是 | 是 | 未实现 |

版本、阶段字段、字段类型或上限不一致时拒绝，不用预算数字赋予阶段权限。注册在写入 root 前调用同一验证器；旧任务仍恢复原始版本，不迁移预算、不改写材料、回执或摘要。没有新增模型请求和工具权限。

保留给未来阶段的 `sourceCoveragePhaseVersion`、`sourceCoverageDecisionPhaseVersion` 在 v1–v3 中均拒绝，包括显式 null/false。这不是声称旧实现已经可以执行这些字段，而是禁止当前消费者静默忽略新增阶段声明。合法旧版本没有这些字段；历史外部 JSON 的只读导入流程不受这个 Native 执行合同取代。

## 2. 回归范围

新增四项纯合同测试：原始版本/字节不变、预算不能授予阶段、未知/混合/缺失/错误类型版本拒绝、未实现覆盖字段拒绝。

新增三项数据库/真实本机传输路径测试：

1. 未支持的注册版本在发布前拒绝，没有 root、assignment、租约或预算账本残留，随后正常 v3 可注册。
2. model claim 写入触发器注入覆盖字段时事务整体回滚，不留下被误认为已经派发的模型调用，移除故障后可正常领取。
3. 已完成候选审查的覆盖准备材料拒绝注入未来阶段；回滚后能重建完全相同的材料，原计划和模型调用数量不变。

现有调度准入回归同时增加覆盖阶段字段及 null 注入。源码整组回归覆盖此前的候选交付、保存响应重入、历史导入、CI、只读报告与工具边界；最终结果以收取的日志终态为准。

本次验证记录：

- 纯合同测试已通过：4 passed，0 failed。日志 `/tmp/oviraptor-source-phase-contract-unit.log`。
- 源码整组已收取 exit 0：244 passed、0 failed、0 ignored，448.32 秒；包含全部新增 7 项测试。命令 `nice -n 15 cargo test -j 1 --manifest-path src-tauri/Cargo.toml --lib source_ -- --test-threads=1`，日志 `/tmp/oviraptor-source-phase-contract-source.log`。
- 严格 Clippy 已收取 exit 0：`nice -n 15 cargo clippy -j 1 --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`，28.46 秒，日志 `/tmp/oviraptor-source-phase-contract-clippy.log`。
- `cargo fmt -- --check` exit 0，日志 `/tmp/oviraptor-source-phase-contract-fmt.log`；tracked diff 空白检查及新增合同/审计文档 no-index 空白检查通过。
- 本轮没有重跑 Rust 全量、UI、构建或浏览器回环；不得用上一轮全量结果声称此次改动后的所有门禁已重新通过。已收取全部 Cargo session 终态，最终进程检查未发现 Cargo、rustc 或库测试进程残留。

保持单 Cargo、单编译 job、单测试线程；`nice` 不等于硬 CPU 上限。未运行用户目标 URL、外部模型、主机操作、部署或新的 UI 功能。

## 3. 必须继续完成

下一步不是直接把版本改为 4：必须先实现独立 coverage subject 的输入/输出合同及确定性缺口约束，再贯通 assignment 路由、独立 child、真实模型回执、usage、mailbox+ACK、不可变裁决及恢复。新版本预算应显式冻结；v1–v3 仍拒绝未来阶段字段。

目前候选 Reviewer 的 assignment 查询仍假定该角色只有一个实例，scheduler/specialist 仍仅路由候选合同。须区分候选与覆盖 subject，且继续拒绝未知/重复 assignment；不能简单在 SQL 里过滤掉多余行而让其逃过总账验证。

覆盖准备摘要仍为 `prepared_not_reviewed`、`independentReviewCompleted=false`、`coverageSufficient=null`。本增量没有完成独立总体覆盖审查、源码自适应三档模式或整个 Master Plan。原完整通用门禁的证据见 `NEST_TOOLCHAIN_DIGEST_CPU_AUDIT_2026-09-27.md`，不冒充本轮修改后的全量结果。
