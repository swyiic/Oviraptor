# 原 Multi 终态消费与旧关闭路径退役审计（2026-10-04，受限范围已实现）

Master 仍未完成，Goal active。本批补原 Multi 财务退出的活消费边界：拒绝可变终态、无原凭证、未闭合 Root、错配回调及读取后篡改；没有实现完整 Multi 物理删除，也没有证明整体功能、安装或真实模型推理质量。

## 原问题与作用域

原新 Web creator/HMAC/Root/C1/十维硬限额 → 实际 localhost Root SDK/Tick/费用 → 原 finish。在隔离 SQLite 中仅改变该 Root 的 terminal_reason，直接终态读取仍成功；实际外层结果消费也仍返回成功。两项功能红 0/2（1.29 秒），日志 `/tmp/oviraptor-multi-terminal-red.log`。模型回调没有选择 Root，原身份来自 Rust 捕获；临时篡改只用作负向。

旧 `record_runtime_terminal_facts_original` 直接返回可变 run 的终态；未退出 Multi 则绕过原退出事务走旧 runtime_adapter.close_run。外层 publication 失败还会转成 persistence_failure 后写目标/checkpoint/fuse 并计数。上述行为不能证明原付费退出，尤其不能成为后续删除依据。

作用域由原 db/scan/attempt/target/Root 捕获身份限定，交易内再次确认原 attempt 未被替换、Root 唯一且 Native coordinator。Multi 财务证明复用上一批不可变 receipt 与原完整来源，不采纳后继 C，不补造凭证，不把未知费用当结清。

## 最小修改

- 原 Multi 终态在一致读事务中核验原 receipt/schema、控制/物理 Root/C、十维费用、原退出与事件后才能返回 reduction。回调的状态/code/脱敏原因须与原退出一致，不允许结果消费自行关闭尚未退出的 Root。
- 外层消费先检查上述证明，失败直接停止，不生成失败投影、不计数。Single 仍走原 Single publication，额外纯读原 Single 财务合同防止 Multi 被改标签为 Single 逃逸。
- 原私有投影 IMMEDIATE 事务在写入前与提交前再核验原 Multi 证明及投影状态/code/原因。既有 authorizer 与原写表/精确写后核验保留；读与投影间的变化不能提交 checkpoint/target/fuse 或更新 tally。
- 删除最后一个生产调用后，`runtime_adapter::close_run`、`reducer::commit`、`store::settle_usage`、`store::mark_run_terminal` 及仅供它们的 imports 已限定 cfg(test)。不加 allow(dead_code)，不把旧测试辅助关闭器当活 API；纯 reducer、Native JSON 与原财务/预算事实保持。

## 实际生产调用与功能验证

新增 12 项最终 12/12（测试 17.36 秒，命令 17.93 秒）。包含原 paid 回放、直接读取与外层拒绝、凭证丢失且恢复精确不可变 DDL 后不补造、未闭合 Root、三种回调错配、读取后通过另一连接改变原终态、Single 标签伪装、真实 503 未知费用仍未结清且不能改成 completion、原 C 自然过期后只读消费且不授新 Tick。

另两项从实际 `run_agent_target` 领取原 invocation 开始，调用原 creator/Root/独立 Mapper/公开面匿名 GET/WebExecutor 工具循环与 Broker GET/只读 Client SDK及 Root 反馈，取得真正 OwnedAgentTargetOutcome。消费成功与重入全行/Native JSON/费用保持、计数一次，实际目标 GET 总数 2，重入零新 SDK/HTTP。负向在 actual owned return 后改变原终态，实际 owned consumer 停止、全部应用行与计数保持（runner log 记录拒绝）。正向释放真正 owned 后调用原 `finish_native_branch`，scan 实际 partial、十维 reserved/indeterminate 为0；随后删除仍准确拒绝原审计保留，全行保持。不是 SQL 设置状态或伪造 owned/branch；仍不是完整物理删除成功。

正向模型是 localhost 脚本，验证真实 SDK transport/原费用/角色链/消费合同，不验证真实模型脑子、15 角色或全六触发。没有 GUI/安装/外部 URL 验收。

## 检查结果与未通过清单

核心修后 2/2（1.31 秒），第一扩大 8/8（4.91 秒）。首次 owned 10 项 8过/2失败：夹具漏写生产公开面专家的无工具响应；修正后再 12 项10过/2失败，原 all-table 保持断言错误包含了正常投影应改的 target/checkpoint，改为保持其他完整物理行/财务/C，并保留所有拒绝时全表保持。不是生产功能红。严格检查首先暴露旧关闭器链四项 dead-code；限定测试构建后修过。

最终严格 all-target/all-feature Clippy -D warnings 0（10.48 秒）；编译2285项；新增逐名12/12；导入39/39（测试3.03秒，命令含编译37.49秒）；exact退役1/1（测试0.73秒，命令1.26秒）。日志 `/tmp/oviraptor-multi-terminal-final-r3-{gates.json,affected-names.json,compiled-test-list.txt,clippy.log,compile.log,affected.log,importer.log,literal.log}`。

扩大实际选择所有直接结果消费调用者文件、上一批普通 Web Single 的148项与旧辅助函数合同，261 项全部运行，无忽略；结果246过/15失败（测试243.65秒，命令244.27秒），严格Clippy0、编译2285。上一批148项实际全部通过，失败没有与上一批531已通过集合重叠。选择器最初误把 include 索引当测试文件，运行前拒绝；随后按实际编译名称逐项选择，没有删除失败项换绿。

15项中本轮新增 owned 正向已走过真正 branch 和十维结清，仅因删除错误包含中文说明而精确字符串断言失败；修为校验准确原因前缀并保持全行/SDK断言，已在最终12/12重新通过。**没有把修后的点检拼成261全绿**。另外14项仍未通过，未逐一证明全部在本批前已失败，暂列诊断债，不能擅自称全部旧失败：

- `commands::agent_tests::bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure`
- `commands::agent_tests::child_completion_executor_publishes_one_acknowledged_result_on_success`
- `commands::agent_tests::child_completion_executor_result_is_not_published_before_resource_commit`
- `commands::agent_tests::e2e_anonymous_spa_completes_without_strix`
- `commands::agent_tests::e2e_confirmed_challenge_fuses_target`
- `commands::agent_tests::e2e_missing_side_stops_at_insufficient_evidence`
- `commands::agent_tests::e2e_no_finding_closes_normally`
- `commands::agent_tests::e2e_personalized_permission_difference_does_not_confirm_idor`
- `commands::agent_tests::e2e_two_equal_accounts_close_without_a_finding`
- `commands::agent_tests::every_target_ends_in_exactly_one_terminal_state`
- `commands::agent_tests::http_journal_execution_stop_survives_target_run_and_checkpoint_projection`
- `commands::agent_tests::policy_native_coverage_only_is_ready_without_a_reviewer`
- `commands::agent_tests::policy_native_routes_without_runtime_adapter`
- `commands::agent_tests::public_surface_prepare_hands_real_observation_to_web_executor_without_replay`

其中 child completion两项及公开面 prepare一项缺原 Web mode declaration；两个 policy用例返回 web_mode_receipt_missing；其余停在目标/计数/原终态未投影。必须逐项追踪原出生、退出、SDK/费用与消费前提，再迁移正向或修复实际代码问题，不放宽原证明。名单与原输出存 `/tmp/oviraptor-multi-terminal-unresolved-names.json`、`/tmp/oviraptor-multi-terminal-final-r2-{affected-names.json,additional-files.json,gates.json,affected.log}`。r2 additional-files 保留该批完整直接调用者与新增旧辅助测试文件清单；不可声称已完成完整 Rust/UI/Native JSON 黄金或安装门禁。

## 工作树保护

9代码路径7已有/2新；本批前分别备份原全文和 Git diff，逐文件完整增量 diff 已复核，1294 原范围外代码文件保持。1303文件集合 SHA `e8098f17ba412de6bbcbd357a83439e9b6f667ea23c769b8e72b13120101ebc7`，HEAD `59be3d86d25adda5b1f975759bf256ef3b94f32b` 未变，diff--check0。既有七路径为 commands的terminal_report/legacy_consumer/legacy_writer/agent_tests和runtime的reducer/runtime_adapter/store；两新路径为commands/agent_tests_multi_terminal_original.rs（364行）与agent_tests_multi_terminal_owned.rs（215行）。原所有内容保护，不 reset、不批量覆盖、不自动提交。

保护 `/tmp/oviraptor-multi-terminal-original-{baseline.json,before.json,prior-diffs,scope-final.json,code-snapshot.json,reviewed-merge-diffs,reviewed-files.json}`；文档另存 `/tmp/oviraptor-multi-terminal-docs-before/`。本批仅临时 SQLite、localhost 和测试子进程，无真实数据库/CAS/资产/安装/授权外部URL/提交。生产不移除任何防篡改 trigger；只有隔离故障测试删除原 receipt 后恢复精确 DDL，以证明不接受损坏/不补造。

## 继续顺序与风险

REM-A01及其他13项全部未完成。下一步在本批真正 owned/branch 闭合链基础上保全 Multi 的原 worker/request/费用/退出/事件/快照/source、物理rowid和所有原校验依赖，处理 Tick/timeline RESTRICT 与不可变财务保留，完成无活授权的独立审计和原子删除/崩溃/丢回复/重试；同时修复直接相关正向结果入库前提，14项扩大失败进入REM-A11，原四项具名债和其他E2E债仍保留，不声称全项目只剩14项。

本次原财务退出只读证明不是完整 worker/request/子进程/父监督退出或完整 source 存档；历史 paid Root 无原 receipt不会被自动接受，须先精确盘点/备份及明确迁移合同，不能重签旧终态、改标签、忽略未知费用或删除真实资产。动态预算/grant/精确对账/显式续跑、六触发/全15角色/GeneralReAct/Broker/真正并行、用户聊天/逐路日志/整体UI、非Web与数据知识继续；最后完整门禁、同源安装 App 打开、授权URL匿名只读、实际模型质量。

InputParser原自动审批拒绝保持，完整原因不可见，不重试绕过。Goal active，不暂停、不标complete。
