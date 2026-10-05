# 原模式、Root财务与结果消费回归审计（2026-10-04）

本批为REM-A01直接相关的REM-A11旧夹具迁移。源码增量仅test-only模块及literal登记，不修改生产授权、writer、SDK、预算/退出/删除合同。Goal仍active，Master14大项仍未完成。全部SDK请求走隔离localhost脚本provider，不是实际LLM质量验收。

## 先证明问题与作用域

基线aa80133c085e015822b5514f267762846a9049485146a376498dcae42e6ed739 / HEAD59be3d86。原14项重跑0/14，9.93秒，日志/tmp/oviraptor-remaining-native-execution-red.log。三项prepare旧种子缺出生mode，两个policy入口缺web_mode_receipt，其他旧回调缺当前原财务/消费依据。生产Single入口已经拒绝无Root，只有cfg(test)支持旧无run helper；未证明生产无Root绕过，不恢复旧producer换绿。

## 最小迁移及负向

1. 新Single helper使用实际ordinary creator/startup/HMAC/冻结计划/原Root，仅test-only ID生成器覆盖为agent-scan以保留既有断言；真正run_agent_target保留captured原身份与owned owner，再走真实consumer。
2. 匿名/WAF保留已付HTTP次数、SDK continuation、覆盖缺口/原因/引用、token、目标/快照/checkpoint和真实熔断。新链首2项0/2读到旧used_requests列0，改按原预算账单核对实际SDK/checkpoint，未恢复退役settle_usage。第二次0/2因新assert误用target_http_requests维名，改实际target_requests；这是新增测试查询错误，不是新生产缺陷。
3. 两新增负向：已付实际GET/SDK后跨目标或临时current attempt旋转，旧回调拒绝。全应用表物理rowid/值与费用保持、零新SDK/HTTP。旋转仅隔离行修改，不是完整启动/重启恢复验收。
4. Multi交付两旧fixture迁已有真实新creator/Root/Mapper/Web/Client SDK链，原ACK/terminal/lane/capability断言保持，新增重复finish费用/调用保持；真实已付后IGNORE/ABORT资源提交故障，execution_result不发布，费用不退款/重计。不是全finally/并发/恢复验收。
5. ExternalSurface只迁一个prepare正向：原Root/Mapper/External SDK、capture和一次GET；重复prepare全应用行保持、零新SDK/HTTP。初次0/1是新增查询误写specialist.phase，改原schema的state后相关通过，不计为生产缺陷修复。
6. 未批量改其他旧测试，无raw结果强写、mode或owner补造。Native JSON维持；认证身份仍需实际扫描策略与原归属验证。

## 验证

| 检查 | 实际结果 | 日志 |
|---|---|---|
| 首次原14基线 | 0过/14失败，9.93秒 | /tmp/oviraptor-remaining-native-execution-red.log |
| Single迁移/负向 | 4/4，11.45秒 | /tmp/oviraptor-native-e2e-original-r3.log |
| Multi交付迁移 | 2/2，8.96秒 | /tmp/oviraptor-child-delivery-original-r1.log |
| 最终相关逐名 | 90/90，125.28秒，选择=通过集合 | /tmp/oviraptor-original-positive-affected-r1.log |
| 原14独立复核 | 5过/9失败，23.00秒，无ignore | /tmp/oviraptor-original-positive-final-debt.log |
| 严格all-targets Clippy | exit0，18.83秒 | /tmp/oviraptor-original-positive-final-clippy.log |
| Native JSON exact | 1/1，0.27秒 | /tmp/oviraptor-original-positive-final-native-json.log |
| 退役登记exact初次 | 0/1，仅E2E文件全文hash变化 | /tmp/oviraptor-original-positive-final-literal.log |
| 退役最终 | 46/46，4.64秒，含exact与上下文负向 | /tmp/oviraptor-original-positive-literal-final.log |

不是全量门禁，不能与前批193/531或导入39拼成全项目全绿。90相关执行快照79b8cc35496f4c8039e849e97988a0a47a9d82e983850f3ad92316ba7fa44a48；后续仅一登记说明/hash变化，无Rust/生产修改，46退役在最终快照执行。2298个lib测试已编译，未全部执行；仅新增Rust局部fmt，不宣称全仓fmt。

## 原14项当前具名状态

| 用例（commands::agent_tests::） | 当前结果 |
|---|---|
| bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure | 失败，继续保留回归债 |
| child_completion_executor_publishes_one_acknowledged_result_on_success | 通过，迁原入口/准备链 |
| child_completion_executor_result_is_not_published_before_resource_commit | 通过，迁原入口/准备链 |
| e2e_anonymous_spa_completes_without_strix | 通过，迁原入口/准备链 |
| e2e_confirmed_challenge_fuses_target | 通过，迁原入口/准备链 |
| e2e_missing_side_stops_at_insufficient_evidence | 失败，继续保留回归债 |
| e2e_no_finding_closes_normally | 失败，继续保留回归债 |
| e2e_personalized_permission_difference_does_not_confirm_idor | 失败，继续保留回归债 |
| e2e_two_equal_accounts_close_without_a_finding | 失败，继续保留回归债 |
| every_target_ends_in_exactly_one_terminal_state | 失败，继续保留回归债 |
| http_journal_execution_stop_survives_target_run_and_checkpoint_projection | 失败，继续保留回归债 |
| policy_native_coverage_only_is_ready_without_a_reviewer | 失败，继续保留回归债 |
| policy_native_routes_without_runtime_adapter | 失败，继续保留回归债 |
| public_surface_prepare_hands_real_observation_to_web_executor_without_replay | 通过，迁原入口/准备链 |

剩余9项必须迁实际出生mode/原身份/费用/消费依据。两旧policy及其saved-policy段当前在web_mode_receipt_missing早拒，旧正向期待不是可用的旧政策兼容。原四项具名债和其他结构/E2E债仍须核对，不能声称全项目只剩9项。没有skip/ignore或强改标签。

## 退役登记审查

已全文阅读agent_tests_e2e.rs、原未提交diff及本批两个函数增量。9个literal在test-only名称/注释/错误断言/旧policy输入。仅该entry reason/hash变化，category fixture、literalOccurrences9、pathMatch false保持；其余JSON entries对象相同。全文SHA从4222c98e954c39ae46eafdfca1027fa20712c1e88fdfb8cc4100c0743e23de7b到e61b08115db84c951d5a637372132ae73a6c4353738c3c1cd139ed2ffbec0853。新说明明确未解决旧policy不授活权限，未改scan scope、分类或门禁，不自动生成全allowlist。

## 未提交保护与交接

基线aa80133c085e015822b5514f267762846a9049485146a376498dcae42e6ed739，最终1321文件集合7b69582beb658c601060853cd06a46dbd2ae60da8f81c424ed69678b2918cd20；HEAD 59be3d86d25adda5b1f975759bf256ef3b94f32b。7路径6已有/1新，1314原范围外保持，无外来新增，diff--check0。

- src-tauri/src/commands/agent_tests.rs
- src-tauri/src/commands/agent_tests_child_completion.rs
- src-tauri/src/commands/agent_tests_e2e.rs
- src-tauri/src/commands/agent_tests_external_surface.rs
- src-tauri/src/commands/agent_tests_fresh_single_production.rs
- src-tauri/src/commands/agent_tests_identity_e2e.rs
- src-tauri/tests/backend_retirement_allowlist.json

四stage：native-e2e-original、child-delivery-original、public-handoff-original、original-positive-literal-review。/tmp对应-before.json、-prior-diffs、-reviewed-merge-diffs、-scope-final.json及-code-snapshot.json保留，所有最终增量已逐文件全文阅读。文档原字节在/tmp/oviraptor-original-positive-docs，写前SHA锁定，新前缀后原suffix全字节保持。

无真实数据库/资产/CAS操作，无安装/授权URL访问，无reset/批量覆盖/提交；上批真实盘点/备份范围不扩大为本批验收。InputParser原审批拒绝、未知完整原因保持，不绕过。继续Master原优先级，最后全量/安装打开/授权URL/真实模型质量仍待完成。
